// mcp-server/src/server.rs
use std::sync::Arc;
use tokio::sync::Mutex;

use rmcp::{
    ServerHandler,
    handler::server::router::tool::ToolRouter,
    model::{ServerCapabilities, ServerInfo},
    tool_handler, tool_router,
};

use bacon_lcm_core::{
    LcmConfig, LcmSession,
    providers::{create_embedder, create_summarizer, create_token_counter},
    storage::StorageLayer,
};

use crate::error::lcm_err_to_mcp;

/// MCP server exposing bacon-lcm session management as MCP tools.
#[derive(Clone)]
pub struct LcmServer {
    pub(crate) session: Arc<Mutex<Option<LcmSession>>>,
    tool_router: ToolRouter<LcmServer>,
}

// Empty tool_router for now — tools are added in later tasks.
#[tool_router]
impl LcmServer {
    pub fn new() -> Self {
        Self {
            session: Arc::new(Mutex::new(None)),
            tool_router: Self::tool_router(),
        }
    }
}

impl Default for LcmServer {
    fn default() -> Self {
        Self::new()
    }
}

impl LcmServer {
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

#[tool_handler(router = self.tool_router)]
impl ServerHandler for LcmServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "bacon-lcm-mcp-server",
                env!("CARGO_PKG_VERSION"),
            ))
    }
}

/// Build the default LCM session from environment variables.
pub async fn build_default_session() -> bacon_lcm_core::LcmResult<LcmSession> {
    let provider = std::env::var("LCM_SUMMARIZER_PROVIDER").unwrap_or_else(|_| "echo".to_string());
    let model = std::env::var("LCM_SUMMARIZER_MODEL").unwrap_or_else(|_| "echo".to_string());
    let api_key = std::env::var("LCM_SUMMARIZER_API_KEY").ok();

    let token_counter = create_token_counter("naive", None)
        .map_err(bacon_lcm_core::LcmError::Provider)?;

    let summarizer = create_summarizer(&provider, model, None, api_key, None, None)
        .map_err(bacon_lcm_core::LcmError::Provider)?;

    let embedder = create_embedder("null", None, None, None, None)
        .map_err(bacon_lcm_core::LcmError::Provider)?;

    let config = LcmConfig::defaults();
    let storage = StorageLayer::memory();

    LcmSession::new(token_counter, summarizer, embedder, config, storage).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_lcm_server_constructs_without_panic() {
        let server = LcmServer::new();
        let guard = server.session.lock().await;
        assert!(guard.is_none());
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
        assert!(session.is_ok(), "failed: {:?}", session.err());
    }
}
