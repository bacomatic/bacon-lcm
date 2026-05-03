// mcp-server/src/server.rs
use std::sync::Arc;
use tokio::sync::Mutex;

use rmcp::{
    ServerHandler,
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::{
        CallToolResult, Content, Implementation, ProtocolVersion,
        ServerCapabilities, ServerInfo,
    },
    schemars,
    tool_handler, tool_router,
};

use bacon_lcm_core::{
    ContextItem, LcmConfig, LcmSession, MessageRole, SummaryLevel,
    providers::{create_embedder, create_summarizer, create_token_counter},
    storage::StorageLayer,
};

use crate::error::lcm_err_to_mcp;

// ── Argument structs ──────────────────────────────────────────────────────────

/// Arguments for `lcm_store`.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LcmStoreArgs {
    /// Message role: "user" | "assistant" | "system" | "tool"
    pub role: String,
    /// The message content.
    pub content: String,
}

/// Arguments for `lcm_describe`.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LcmDescribeArgs {
    /// UUID of the summary node to describe.
    pub summary_id: String,
}

/// Arguments for `lcm_expand`.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LcmExpandArgs {
    /// UUID of the summary node to expand back to its original messages.
    pub summary_id: String,
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn role_from_str(role: &str) -> Result<MessageRole, rmcp::ErrorData> {
    match role {
        "user"      => Ok(MessageRole::User),
        "assistant" => Ok(MessageRole::Assistant),
        "system"    => Ok(MessageRole::System),
        "tool"      => Ok(MessageRole::Tool),
        other => Err(rmcp::ErrorData::invalid_params(
            format!("unknown role '{}'; expected user|assistant|system|tool", other),
            None,
        )),
    }
}

fn parse_summary_id(s: &str) -> Result<uuid::Uuid, CallToolResult> {
    uuid::Uuid::parse_str(s).map_err(|e| {
        CallToolResult::error(vec![Content::text(format!(
            "invalid summary_id '{}': {}",
            s, e
        ))])
    })
}

// ── LcmServer ─────────────────────────────────────────────────────────────────

/// MCP server exposing bacon-lcm session management as MCP tools.
#[derive(Clone)]
pub struct LcmServer {
    pub(crate) session: Arc<Mutex<Option<LcmSession>>>,
    tool_router: ToolRouter<LcmServer>,
}

#[tool_router]
impl LcmServer {
    pub fn new() -> Self {
        Self {
            session: Arc::new(Mutex::new(None)),
            tool_router: Self::tool_router(),
        }
    }

    // ── lcm_session_new ───────────────────────────────────────────────────────

    #[rmcp::tool(description = "Create a new LCM session. \
        Replaces any existing session. \
        Returns the new session_id (UUID string).")]
    async fn lcm_session_new(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        let new_session = build_default_session().await.map_err(lcm_err_to_mcp)?;
        let session_id = new_session.session().id.to_string();
        {
            let mut guard = self.session.lock().await;
            *guard = Some(new_session);
        }
        Ok(CallToolResult::success(vec![Content::text(session_id)]))
    }

    // ── lcm_session_info ──────────────────────────────────────────────────────

    #[rmcp::tool(description = "Return statistics for the active LCM session as JSON. \
        Fields: session_id, message_count, token_count, summary_count, is_compacting.")]
    async fn lcm_session_info(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut guard = self.get_or_create_session().await?;
        let session = guard.as_mut().expect("guaranteed by get_or_create_session");
        let info = session.get_session_info().await.map_err(lcm_err_to_mcp)?;
        let json = serde_json::json!({
            "session_id":    info.session.id.to_string(),
            "message_count": info.message_count,
            "token_count":   info.token_count,
            "summary_count": info.summary_count,
            "is_compacting": info.is_compacting,
        });
        Ok(CallToolResult::success(vec![Content::text(json.to_string())]))
    }

    // ── lcm_store ─────────────────────────────────────────────────────────────

    #[rmcp::tool(description = "Store a message in the active LCM session. \
        Args: role (\"user\"|\"assistant\"|\"system\"|\"tool\"), content (string). \
        Returns the message_id (UUID string).")]
    async fn lcm_store(
        &self,
        Parameters(args): Parameters<LcmStoreArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let message_role = match role_from_str(&args.role) {
            Ok(r) => r,
            Err(_) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "unknown role '{}'; expected user|assistant|system|tool",
                    args.role
                ))]));
            }
        };

        let mut guard = self.get_or_create_session().await?;
        let session = guard.as_mut().expect("guaranteed by get_or_create_session");

        let message_id = session
            .add_message(message_role, args.content)
            .await
            .map_err(lcm_err_to_mcp)?;

        Ok(CallToolResult::success(vec![Content::text(message_id.to_string())]))
    }

    // ── lcm_recall ────────────────────────────────────────────────────────────

    #[rmcp::tool(description = "Retrieve the active context window for the current LCM session. \
        Returns a human-readable string where each line is prefixed by its role \
        (e.g. \"user: …\") or summary level (e.g. \"[Summary L0]: …\").")]
    async fn lcm_recall(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut guard = self.get_or_create_session().await?;
        let session = guard.as_mut().expect("guaranteed by get_or_create_session");

        let items = session.get_context().await.map_err(lcm_err_to_mcp)?;

        if items.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text("(empty context)")]));
        }

        let mut lines = Vec::with_capacity(items.len());
        for item in &items {
            let line = match item {
                ContextItem::Message(msg) => {
                    let role_str = match msg.role {
                        MessageRole::User      => "user",
                        MessageRole::Assistant => "assistant",
                        MessageRole::System    => "system",
                        MessageRole::Tool      => "tool",
                    };
                    format!("{}: {}", role_str, msg.content)
                }
                ContextItem::Summary(node) => {
                    let level_num = match node.level {
                        SummaryLevel::Leaf      => 0,
                        SummaryLevel::Condensed => 1,
                        SummaryLevel::Emergency => 2,
                    };
                    format!("[Summary L{}]: {}", level_num, node.content)
                }
            };
            lines.push(line);
        }

        Ok(CallToolResult::success(vec![Content::text(lines.join("\n"))]))
    }

    // ── lcm_describe ──────────────────────────────────────────────────────────

    #[rmcp::tool(description = "Inspect a summary node. \
        Args: summary_id (UUID string). \
        Returns JSON with: id, level (\"Leaf\"|\"Condensed\"|\"Emergency\"), \
        token_count, content_preview (first 120 chars), \
        reachable_message_count, lineage_length.")]
    async fn lcm_describe(
        &self,
        Parameters(args): Parameters<LcmDescribeArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let summary_id = match parse_summary_id(&args.summary_id) {
            Ok(id) => id,
            Err(err_result) => return Ok(err_result),
        };

        let mut guard = self.get_or_create_session().await?;
        let session = guard.as_mut().expect("guaranteed by get_or_create_session");

        let describe_result = match session.describe(summary_id).await {
            Ok(r) => r,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(e.to_string())]));
            }
        };

        let level_str = match describe_result.summary.level {
            SummaryLevel::Leaf      => "Leaf",
            SummaryLevel::Condensed => "Condensed",
            SummaryLevel::Emergency => "Emergency",
        };

        let content_preview = describe_result
            .summary
            .content
            .chars()
            .take(120)
            .collect::<String>();

        let json = serde_json::json!({
            "id":                      describe_result.summary.id.to_string(),
            "level":                   level_str,
            "token_count":             describe_result.summary.token_count,
            "content_preview":         content_preview,
            "reachable_message_count": describe_result.reachable_message_count,
            "lineage_length":          describe_result.lineage.len(),
        });

        Ok(CallToolResult::success(vec![Content::text(json.to_string())]))
    }

    // ── lcm_expand ────────────────────────────────────────────────────────────

    #[rmcp::tool(description = "Expand a summary node back to its original verbatim messages. \
        Args: summary_id (UUID string). \
        Returns a newline-separated list of messages formatted as \
        \"<role>: <content>\" (one message per line).")]
    async fn lcm_expand(
        &self,
        Parameters(args): Parameters<LcmExpandArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let summary_id = match parse_summary_id(&args.summary_id) {
            Ok(id) => id,
            Err(err_result) => return Ok(err_result),
        };

        let mut guard = self.get_or_create_session().await?;
        let session = guard.as_mut().expect("guaranteed by get_or_create_session");

        // Verify the summary exists (describe returns SummaryNotFound for unknown IDs).
        if let Err(e) = session.describe(summary_id).await {
            return Ok(CallToolResult::error(vec![Content::text(e.to_string())]));
        }

        let messages = match session.expand(summary_id).await {
            Ok(msgs) => msgs,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(e.to_string())]));
            }
        };

        if messages.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "(no messages found for this summary)",
            )]));
        }

        let lines: Vec<String> = messages
            .iter()
            .map(|msg| {
                let role_str = match msg.role {
                    MessageRole::User      => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::System    => "system",
                    MessageRole::Tool      => "tool",
                };
                format!("{}: {}", role_str, msg.content)
            })
            .collect();

        Ok(CallToolResult::success(vec![Content::text(lines.join("\n"))]))
    }
}

impl Default for LcmServer {
    fn default() -> Self {
        Self::new()
    }
}

impl LcmServer {
    /// Replace the active session (used by main.rs for session restore).
    pub async fn set_session(&self, session: LcmSession) {
        let mut guard = self.session.lock().await;
        *guard = Some(session);
    }

    /// Get (or lazily create) the LCM session.
    pub(crate) async fn get_or_create_session(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, Option<LcmSession>>, rmcp::ErrorData> {
        let mut guard = self.session.lock().await;
        if guard.is_none() {
            let session = build_default_session().await.map_err(lcm_err_to_mcp)?;
            *guard = Some(session);
        }
        Ok(guard)
    }
}

// ── ServerHandler ─────────────────────────────────────────────────────────────

#[tool_handler(router = self.tool_router)]
impl ServerHandler for LcmServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_server_info(Implementation::from_build_env())
        .with_protocol_version(ProtocolVersion::V_2024_11_05)
        .with_instructions(
            "bacon-lcm MCP server — lossless context memory for LLM agents. \
             Tools: lcm_store, lcm_recall, lcm_describe, lcm_expand, \
             lcm_session_new, lcm_session_info."
                .to_string(),
        )
    }
}

// ── Provider / session factory ────────────────────────────────────────────────

/// Build the default LCM session from environment variables.
///
/// If `DATABASE_URL` is set, uses the Postgres `StorageLayer`; otherwise
/// falls back to the in-memory layer (useful for local dev/testing).
pub async fn build_default_session() -> bacon_lcm_core::LcmResult<LcmSession> {
    let provider = std::env::var("LCM_SUMMARIZER_PROVIDER")
        .unwrap_or_else(|_| "echo".to_string());
    let model = std::env::var("LCM_SUMMARIZER_MODEL")
        .unwrap_or_else(|_| "echo".to_string());
    let api_key = std::env::var("LCM_SUMMARIZER_API_KEY").ok();

    let token_counter = create_token_counter("naive", None)
        .map_err(bacon_lcm_core::LcmError::Provider)?;
    let summarizer = create_summarizer(
        &provider,
        model,
        None,
        api_key,
        None,
        None,
    )
    .map_err(bacon_lcm_core::LcmError::Provider)?;
    let embedder = create_embedder("null", None, None, None, None)
        .map_err(bacon_lcm_core::LcmError::Provider)?;

    let config = LcmConfig::defaults();

    let storage = if let Ok(db_url) = std::env::var("DATABASE_URL") {
        tracing::info!("DATABASE_URL set — using Postgres storage layer");
        let pool = sqlx::PgPool::connect(&db_url)
            .await
            .map_err(|e| bacon_lcm_core::LcmError::Storage(
                bacon_lcm_core::StorageError::ConnectionFailed(e),
            ))?;
        bacon_lcm_daemon::storage::postgres_layer(pool)
    } else {
        tracing::debug!("DATABASE_URL not set — using in-memory storage layer");
        StorageLayer::memory()
    };

    LcmSession::new(token_counter, summarizer, embedder, config, storage).await
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_lcm_server_constructs_without_panic() {
        let server = LcmServer::new();
        let guard = server.session.lock().await;
        assert!(guard.is_none(), "session should start as None");
    }

    #[tokio::test]
    async fn test_get_or_create_session_creates_session() {
        let server = LcmServer::new();
        {
            let guard = server.get_or_create_session().await.unwrap();
            assert!(guard.is_some());
        }
        {
            let guard = server.get_or_create_session().await.unwrap();
            assert!(guard.is_some());
        }
    }

    #[tokio::test]
    async fn test_build_default_session() {
        let session = build_default_session().await;
        assert!(session.is_ok(), "default session creation failed: {:?}", session.err());
    }

    fn extract_text(result: &rmcp::model::CallToolResult) -> String {
        match &result.content[0].raw {
            rmcp::model::RawContent::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        }
    }

    #[tokio::test]
    async fn test_lcm_session_new_returns_uuid() {
        let server = LcmServer::new();
        let result = server.lcm_session_new().await;
        assert!(result.is_ok());
        let tool_result = result.unwrap();
        assert!(!tool_result.is_error.unwrap_or(false));
        let text = extract_text(&tool_result);
        uuid::Uuid::parse_str(&text).expect("session_id should be a valid UUID");
    }

    #[tokio::test]
    async fn test_lcm_session_info_returns_json() {
        let server = LcmServer::new();
        let _ = server.lcm_session_new().await.unwrap();
        let result = server.lcm_session_info().await;
        assert!(result.is_ok());
        let tool_result = result.unwrap();
        assert!(!tool_result.is_error.unwrap_or(false));
        let text = extract_text(&tool_result);
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(json.get("session_id").is_some());
        assert!(json.get("message_count").is_some());
        assert!(json.get("token_count").is_some());
        assert!(json.get("summary_count").is_some());
    }

    #[tokio::test]
    async fn test_lcm_store_returns_message_id() {
        let server = LcmServer::new();
        let result = server.lcm_store(Parameters(LcmStoreArgs {
            role: "user".to_string(),
            content: "Hello!".to_string(),
        })).await;
        assert!(result.is_ok());
        let tool_result = result.unwrap();
        assert!(!tool_result.is_error.unwrap_or(false));
        let text = extract_text(&tool_result);
        uuid::Uuid::parse_str(&text).expect("message_id should be a valid UUID");
    }

    #[tokio::test]
    async fn test_lcm_store_unknown_role_returns_tool_error() {
        let server = LcmServer::new();
        let result = server.lcm_store(Parameters(LcmStoreArgs {
            role: "unknown_role".to_string(),
            content: "test".to_string(),
        })).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn test_lcm_recall_empty_session() {
        let server = LcmServer::new();
        let result = server.lcm_recall().await.unwrap();
        assert!(!result.is_error.unwrap_or(false));
        let text = extract_text(&result);
        assert!(text.contains("empty"));
    }

    #[tokio::test]
    async fn test_lcm_recall_after_store() {
        let server = LcmServer::new();
        server.lcm_store(Parameters(LcmStoreArgs {
            role: "user".to_string(),
            content: "What is 2+2?".to_string(),
        })).await.unwrap();
        server.lcm_store(Parameters(LcmStoreArgs {
            role: "assistant".to_string(),
            content: "The answer is 4.".to_string(),
        })).await.unwrap();
        let result = server.lcm_recall().await.unwrap();
        let text = extract_text(&result);
        assert!(text.contains("What is 2+2?"));
        assert!(text.contains("The answer is 4"));
        assert!(text.contains("user:"));
        assert!(text.contains("assistant:"));
    }

    #[tokio::test]
    async fn test_lcm_describe_unknown_id_returns_tool_error() {
        let server = LcmServer::new();
        let fake_id = uuid::Uuid::new_v4().to_string();
        let result = server
            .lcm_describe(Parameters(LcmDescribeArgs { summary_id: fake_id }))
            .await;
        assert!(result.is_ok(), "lcm_describe should return Ok even on not-found");
        assert!(result.unwrap().is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn test_lcm_describe_invalid_uuid_returns_tool_error() {
        let server = LcmServer::new();
        let result = server
            .lcm_describe(Parameters(LcmDescribeArgs {
                summary_id: "not-a-uuid".to_string(),
            }))
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn test_lcm_expand_unknown_id_returns_tool_error() {
        let server = LcmServer::new();
        let fake_id = uuid::Uuid::new_v4().to_string();
        let result = server
            .lcm_expand(Parameters(LcmExpandArgs { summary_id: fake_id }))
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn test_lcm_expand_invalid_uuid_returns_tool_error() {
        let server = LcmServer::new();
        let result = server
            .lcm_expand(Parameters(LcmExpandArgs {
                summary_id: "bad-uuid".to_string(),
            }))
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn test_build_default_session_memory_when_no_db_url() {
        // When DATABASE_URL is not set the session must still build successfully.
        std::env::remove_var("DATABASE_URL");
        let session = build_default_session().await;
        assert!(
            session.is_ok(),
            "build_default_session failed without DATABASE_URL: {:?}",
            session.err()
        );
    }
}
