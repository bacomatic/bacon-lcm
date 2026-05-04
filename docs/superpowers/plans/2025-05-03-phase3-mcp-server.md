# Phase 3: MCP Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a fully functional MCP server binary (`bacon-lcm-mcp-server`) in the `mcp-server` crate using the `rmcp` SDK. The server exposes six tools (`lcm_store`, `lcm_recall`, `lcm_describe`, `lcm_expand`, `lcm_session_new`, `lcm_session_info`) backed by an `LcmSession` from `bacon-lcm-core`, communicates over stdio transport, and selects providers via environment variables.

**Architecture:** Single-process stdio MCP server. One `LcmSession` lives inside `Arc<Mutex<Option<LcmSession>>>` for the lifetime of the process. Session state is in-memory by default; provider selection (echo / openai / anthropic) and optional session restoration are driven by environment variables. The `rmcp` `#[tool_router]` macro routes incoming tool calls; `ServerHandler::get_info` advertises capabilities.

**Tech Stack:** Rust · Tokio · rmcp 1.6 (server, transport-io, macros, schemars) · schemars 1.0 · bacon-lcm-core (path dep) · tracing / tracing-subscriber

---

## Files created / modified across all tasks

| File | Action |
|---|---|
| `Cargo.toml` (workspace root) | Add `rmcp` + `schemars` to `[workspace.dependencies]` |
| `mcp-server/Cargo.toml` | Add full `[dependencies]` block |
| `mcp-server/src/lib.rs` | New — crate entry point exposing `error` + `server` modules |
| `mcp-server/src/main.rs` | Replace stub with async main |
| `mcp-server/src/server.rs` | New — `LcmServer` struct + all six tools |
| `mcp-server/src/error.rs` | New — `lcm_err_to_mcp` conversion helper |
| `mcp-server/tests/smoke_test.rs` | New — integration smoke test (Task 6) |

---

## Task 1 — Set up `mcp-server` crate dependencies

**Files modified:**
- `Cargo.toml` (workspace root) — add `rmcp` + `schemars` to `[workspace.dependencies]`
- `mcp-server/Cargo.toml` — full `[dependencies]` block
- `mcp-server/src/lib.rs` — create crate entry point
- `mcp-server/src/server.rs` — create empty stub
- `mcp-server/src/error.rs` — create empty stub

**Commit message:** `feat(mcp-server): add rmcp, schemars, and core dependencies (Task 1)`

---

- [ ] **Step 1.1 — Add workspace-level dependencies for `rmcp` and `schemars`**

Open `Cargo.toml` (workspace root) and append two entries to `[workspace.dependencies]` after the existing entries:

```toml
# MCP server SDK
rmcp     = { version = "1.6", features = ["server", "transport-io", "macros", "schemars"] }
# JSON Schema generation for tool argument structs
schemars = { version = "1.0", features = ["derive"] }
```

---

- [ ] **Step 1.2 — Fill in `mcp-server/Cargo.toml`**

Replace the entire file with:

```toml
[package]
name    = "bacon-lcm-mcp-server"
version.workspace = true
license.workspace = true
edition.workspace = true

[[bin]]
name = "bacon-lcm-mcp-server"
path = "src/main.rs"

[lib]
name = "bacon_lcm_mcp_server"
path = "src/lib.rs"

[dependencies]
bacon-lcm-core     = { path = "../core" }
rmcp               = { workspace = true }
schemars           = { workspace = true }
tokio              = { workspace = true }
serde              = { workspace = true }
serde_json         = { workspace = true }
anyhow             = { workspace = true }
tracing            = { workspace = true }
tracing-subscriber = { workspace = true }
uuid               = { workspace = true }

[dev-dependencies]
tokio      = { workspace = true }
serde_json = { workspace = true }
```

> **Note:** The `[lib]` section is required so that integration tests in `tests/smoke_test.rs` can reference `bacon_lcm_mcp_server::server`. The `[[bin]]` entry continues to point at `src/main.rs`.

---

- [ ] **Step 1.3 — Create `mcp-server/src/lib.rs`**

```rust
// mcp-server/src/lib.rs
pub mod error;
pub mod server;
```

---

- [ ] **Step 1.4 — Create stub files so the crate compiles**

Create `mcp-server/src/error.rs`:

```rust
// mcp-server/src/error.rs — stub; full implementation in Task 2
```

Create `mcp-server/src/server.rs`:

```rust
// mcp-server/src/server.rs — stub; full implementation in Task 2
```

---

- [ ] **Step 1.5 — Verify the crate compiles**

```
cargo build -p bacon-lcm-mcp-server
```

Expected output (no errors; warnings about unused imports are acceptable at this stage):

```
   Compiling bacon-lcm-mcp-server v0.1.0 (.../mcp-server)
    Finished `dev` profile [unoptimized + debuginfo] target(s)
```

---

## Task 2 — Implement the `LcmServer` struct with session state

**Files created / modified:**
- `mcp-server/src/error.rs` — `lcm_err_to_mcp` conversion function
- `mcp-server/src/server.rs` — `LcmServer` struct, `get_or_create_session` helper, `ServerHandler` impl, `build_default_session` factory

**Commit message:** `feat(mcp-server): LcmServer struct with session state and ServerHandler (Task 2)`

---

- [ ] **Step 2.1 — Write the failing unit test first**

Replace `mcp-server/src/server.rs` with a file that contains only the test module (will fail to compile until the struct is added):

```rust
// mcp-server/src/server.rs

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_lcm_server_constructs_without_panic() {
        // LcmServer::new() must not panic.
        let server = LcmServer::new();
        // The session slot starts empty.
        let guard = server.session.lock().await;
        assert!(guard.is_none(), "session should start as None");
    }
}
```

Run `cargo test -p bacon-lcm-mcp-server` — fails to compile because `LcmServer` does not exist yet.

---

- [ ] **Step 2.2 — Implement `mcp-server/src/error.rs`**

```rust
// mcp-server/src/error.rs
use bacon_lcm_core::LcmError;
use rmcp::ErrorData;

/// Convert an `LcmError` into an MCP `ErrorData` so that tool handlers can use
/// `?` to propagate failures as MCP protocol errors.
pub fn lcm_err_to_mcp(err: LcmError) -> ErrorData {
    ErrorData::internal_error(err.to_string(), None)
}
```

---

- [ ] **Step 2.3 — Implement `mcp-server/src/server.rs`**

```rust
// mcp-server/src/server.rs
use std::sync::Arc;
use tokio::sync::Mutex;

use rmcp::{
    ServerHandler,
    handler::server::router::tool::ToolRouter,
    model::{
        CallToolResult, Content, Implementation, ProtocolVersion,
        ServerCapabilities, ServerInfo,
    },
    tool_router,
};

use bacon_lcm_core::{
    LcmConfig, LcmSession,
    providers::{create_embedder, create_summarizer, create_token_counter},
    storage::StorageLayer,
};

use crate::error::lcm_err_to_mcp;

/// The MCP server. Holds a lazily-created `LcmSession` behind a mutex so that
/// all six tool handlers can share it without requiring `&mut self`.
#[derive(Clone)]
pub struct LcmServer {
    /// The active session. `None` until first use or until `lcm_session_new` is called.
    pub(crate) session: Arc<Mutex<Option<LcmSession>>>,
    tool_router: ToolRouter<LcmServer>,
}

// ── Tool implementations added in Tasks 3–5 ──────────────────────────────────
#[tool_router]
impl LcmServer {
    pub fn new() -> Self {
        Self {
            session: Arc::new(Mutex::new(None)),
            tool_router: Self::tool_router(),
        }
    }
}

impl LcmServer {
    /// Return a mutex guard containing the active session, creating a new one
    /// (with default providers and in-memory storage) if none exists yet.
    ///
    /// Provider selection reads three environment variables:
    /// - `LCM_SUMMARIZER_PROVIDER`  (default: `"echo"`)
    /// - `LCM_SUMMARIZER_MODEL`     (default: `"echo"`)
    /// - `LCM_SUMMARIZER_API_KEY`   (default: not set — safe for the "echo" provider)
    pub(crate) async fn get_or_create_session(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, Option<LcmSession>>, rmcp::ErrorData> {
        let mut guard = self.session.lock().await;
        if guard.is_none() {
            let session = build_default_session()
                .await
                .map_err(lcm_err_to_mcp)?;
            *guard = Some(session);
        }
        Ok(guard)
    }
}

// ── ServerHandler ─────────────────────────────────────────────────────────────

#[rmcp::tool_handler]
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

/// Build a fresh `LcmSession` using in-memory storage and env-configured providers.
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
        model,       // String, not Option<String>
        None,        // base_url — use provider default
        api_key,
        None,        // max_tokens — use provider default
        None,        // temperature — use provider default
    )
    .map_err(bacon_lcm_core::LcmError::Provider)?;
    let embedder = create_embedder("null", None, None, None, None)
        .map_err(bacon_lcm_core::LcmError::Provider)?;

    let config = LcmConfig::defaults();
    let storage = StorageLayer::memory();

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
        // First call should create the session.
        {
            let guard = server.get_or_create_session().await.unwrap();
            assert!(guard.is_some(), "session should be Some after first call");
        }
        // Second call should return the existing session.
        {
            let guard = server.get_or_create_session().await.unwrap();
            assert!(guard.is_some());
        }
    }

    #[tokio::test]
    async fn test_build_default_session() {
        // Ensure the provider factory works with default env (echo summarizer).
        let session = build_default_session().await;
        assert!(
            session.is_ok(),
            "default session creation failed: {:?}",
            session.err()
        );
    }
}
```

---

- [ ] **Step 2.4 — Run tests (all three should pass)**

```
cargo test -p bacon-lcm-mcp-server
```

Expected output:

```
running 3 tests
test server::tests::test_lcm_server_constructs_without_panic ... ok
test server::tests::test_get_or_create_session_creates_session ... ok
test server::tests::test_build_default_session ... ok

test result: ok. 3 passed; 0 failed; 0 ignored
```

---

## Task 3 — Implement `lcm_session_new` and `lcm_session_info` tools

**Files modified:**
- `mcp-server/src/server.rs` — add two `#[tool]` methods to `#[tool_router] impl LcmServer`

**Commit message:** `feat(mcp-server): lcm_session_new and lcm_session_info tools (Task 3)`

---

- [ ] **Step 3.1 — Write the failing tests first**

Append to the `tests` module inside `server.rs`:

```rust
    #[tokio::test]
    async fn test_lcm_session_new_returns_uuid() {
        let server = LcmServer::new();
        let result = server.lcm_session_new().await;
        assert!(result.is_ok(), "lcm_session_new failed: {:?}", result.err());
        let tool_result = result.unwrap();
        assert!(!tool_result.is_error.unwrap_or(false), "tool reported error");
        // Content should be a valid UUID string.
        let text = match &tool_result.content[0] {
            rmcp::model::Content::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        };
        uuid::Uuid::parse_str(&text).expect("session_id should be a valid UUID");
    }

    #[tokio::test]
    async fn test_lcm_session_info_returns_json() {
        let server = LcmServer::new();
        // Ensure a session exists first.
        let _ = server.lcm_session_new().await.unwrap();
        let result = server.lcm_session_info().await;
        assert!(result.is_ok());
        let tool_result = result.unwrap();
        assert!(!tool_result.is_error.unwrap_or(false));
        let text = match &tool_result.content[0] {
            rmcp::model::Content::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        };
        // Must be valid JSON with all expected keys.
        let json: serde_json::Value =
            serde_json::from_str(&text).expect("lcm_session_info must return valid JSON");
        assert!(json.get("session_id").is_some());
        assert!(json.get("message_count").is_some());
        assert!(json.get("token_count").is_some());
        assert!(json.get("summary_count").is_some());
    }
```

Run `cargo test -p bacon-lcm-mcp-server` — fails to compile (missing tool methods).

---

- [ ] **Step 3.2 — Implement the two tools**

Replace the `#[tool_router] impl LcmServer` block (which currently contains only `new()`) with:

```rust
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
        let session = guard.as_mut().expect("session guaranteed by get_or_create_session");

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
}
```

---

- [ ] **Step 3.3 — Run tests (all five should pass)**

```
cargo test -p bacon-lcm-mcp-server
```

Expected output:

```
running 5 tests
test server::tests::test_lcm_server_constructs_without_panic ... ok
test server::tests::test_get_or_create_session_creates_session ... ok
test server::tests::test_build_default_session ... ok
test server::tests::test_lcm_session_new_returns_uuid ... ok
test server::tests::test_lcm_session_info_returns_json ... ok

test result: ok. 5 passed; 0 failed; 0 ignored
```

---

## Task 4 — Implement `lcm_store` and `lcm_recall` tools

**Files modified:**
- `mcp-server/src/server.rs` — add `LcmStoreArgs` struct, `role_from_str` helper, and two `#[tool]` methods

**Commit message:** `feat(mcp-server): lcm_store and lcm_recall tools (Task 4)`

---

- [ ] **Step 4.1 — Write the failing tests first**

Append to the `tests` module in `server.rs`:

```rust
    #[tokio::test]
    async fn test_lcm_store_returns_message_id() {
        use rmcp::handler::server::wrapper::Parameters;
        let server = LcmServer::new();
        let args = LcmStoreArgs {
            role:    "user".to_string(),
            content: "Hello, world!".to_string(),
        };
        let result = server.lcm_store(Parameters(args)).await;
        assert!(result.is_ok(), "lcm_store failed: {:?}", result.err());
        let tool_result = result.unwrap();
        assert!(!tool_result.is_error.unwrap_or(false));
        let text = match &tool_result.content[0] {
            rmcp::model::Content::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        };
        uuid::Uuid::parse_str(&text).expect("message_id should be a valid UUID");
    }

    #[tokio::test]
    async fn test_lcm_store_unknown_role_returns_tool_error() {
        use rmcp::handler::server::wrapper::Parameters;
        let server = LcmServer::new();
        let args = LcmStoreArgs {
            role:    "unknown_role".to_string(),
            content: "test".to_string(),
        };
        let result = server.lcm_store(Parameters(args)).await;
        // Should return Ok(CallToolResult) with is_error = true rather than a
        // protocol-level error so the agent can recover gracefully.
        assert!(result.is_ok());
        let tool_result = result.unwrap();
        assert!(
            tool_result.is_error.unwrap_or(false),
            "expected is_error=true for unknown role"
        );
    }

    #[tokio::test]
    async fn test_lcm_recall_empty_session() {
        let server = LcmServer::new();
        let result = server.lcm_recall().await;
        assert!(result.is_ok());
        let tool_result = result.unwrap();
        assert!(!tool_result.is_error.unwrap_or(false));
        let text = match &tool_result.content[0] {
            rmcp::model::Content::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        };
        assert!(
            text.contains("empty"),
            "expected '(empty context)' for new session, got: {text}"
        );
    }

    #[tokio::test]
    async fn test_lcm_recall_after_store() {
        use rmcp::handler::server::wrapper::Parameters;
        let server = LcmServer::new();

        server
            .lcm_store(Parameters(LcmStoreArgs {
                role:    "user".to_string(),
                content: "What is 2+2?".to_string(),
            }))
            .await
            .unwrap();

        server
            .lcm_store(Parameters(LcmStoreArgs {
                role:    "assistant".to_string(),
                content: "The answer is 4.".to_string(),
            }))
            .await
            .unwrap();

        let result = server.lcm_recall().await.unwrap();
        let text = match &result.content[0] {
            rmcp::model::Content::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        };
        assert!(text.contains("What is 2+2?"),    "user message missing from recall");
        assert!(text.contains("The answer is 4"), "assistant message missing from recall");
        assert!(text.contains("user:"),            "role prefix 'user:' missing");
        assert!(text.contains("assistant:"),       "role prefix 'assistant:' missing");
    }
```

Run `cargo test -p bacon-lcm-mcp-server` — fails to compile (missing types/methods).

---

- [ ] **Step 4.2 — Add `LcmStoreArgs` and the `role_from_str` helper**

Near the top of `server.rs`, after the `use` block, add:

```rust
use rmcp::{handler::server::wrapper::Parameters, schemars};

/// Arguments for `lcm_store`.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LcmStoreArgs {
    /// Message role: "user" | "assistant" | "system" | "tool"
    pub role: String,
    /// The message content.
    pub content: String,
}

/// Parse a role string into `MessageRole`.
///
/// Returns an MCP `ErrorData` (for protocol-level propagation) on unknown input.
/// Tool handlers use this then decide whether to surface it as a protocol error
/// or as a softer tool-level `is_error = true` result.
fn role_from_str(role: &str) -> Result<bacon_lcm_core::MessageRole, rmcp::ErrorData> {
    use bacon_lcm_core::MessageRole;
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
```

---

- [ ] **Step 4.3 — Add `lcm_store` and `lcm_recall` to the `#[tool_router]` impl block**

Inside `#[tool_router] impl LcmServer { … }`, add after `lcm_session_info`:

```rust
    // ── lcm_store ─────────────────────────────────────────────────────────────

    #[rmcp::tool(description = "Store a message in the active LCM session. \
        Args: role (\"user\"|\"assistant\"|\"system\"|\"tool\"), content (string). \
        Returns the message_id (UUID string).")]
    async fn lcm_store(
        &self,
        Parameters(args): Parameters<LcmStoreArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // Validate the role before acquiring the mutex to keep the error path clean.
        let message_role = match role_from_str(&args.role) {
            Ok(r) => r,
            Err(_) => {
                // Return a tool-level error (is_error = true) rather than a
                // protocol-level error so the LLM agent can recover without the
                // entire session being terminated.
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
        use bacon_lcm_core::{ContextItem, MessageRole, SummaryLevel};

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
```

---

- [ ] **Step 4.4 — Run tests (all nine should pass)**

```
cargo test -p bacon-lcm-mcp-server
```

Expected output:

```
running 9 tests
test server::tests::test_lcm_server_constructs_without_panic ... ok
test server::tests::test_get_or_create_session_creates_session ... ok
test server::tests::test_build_default_session ... ok
test server::tests::test_lcm_session_new_returns_uuid ... ok
test server::tests::test_lcm_session_info_returns_json ... ok
test server::tests::test_lcm_store_returns_message_id ... ok
test server::tests::test_lcm_store_unknown_role_returns_tool_error ... ok
test server::tests::test_lcm_recall_empty_session ... ok
test server::tests::test_lcm_recall_after_store ... ok

test result: ok. 9 passed; 0 failed; 0 ignored
```

---

## Task 5 — Implement `lcm_describe` and `lcm_expand` tools

**Files modified:**
- `mcp-server/src/server.rs` — add `LcmDescribeArgs`, `LcmExpandArgs`, `parse_summary_id` helper, and two `#[tool]` methods

**Commit message:** `feat(mcp-server): lcm_describe and lcm_expand tools (Task 5)`

---

- [ ] **Step 5.1 — Write the failing tests first**

Append to the `tests` module in `server.rs`:

```rust
    // ── lcm_describe tests ────────────────────────────────────────────────────
    // Note: triggering real compaction requires exceeding the soft threshold
    // (~80k tokens) which is impractical in a unit test with the naive counter
    // and echo summarizer. We verify that the tools return well-formed
    // tool-level errors for missing or malformed inputs.

    #[tokio::test]
    async fn test_lcm_describe_unknown_id_returns_tool_error() {
        use rmcp::handler::server::wrapper::Parameters;
        let server = LcmServer::new();
        let fake_id = uuid::Uuid::new_v4().to_string();
        let result = server
            .lcm_describe(Parameters(LcmDescribeArgs { summary_id: fake_id }))
            .await;
        assert!(result.is_ok(), "lcm_describe should return Ok even on not-found");
        let tool_result = result.unwrap();
        assert!(
            tool_result.is_error.unwrap_or(false),
            "expected is_error=true for unknown summary_id"
        );
    }

    #[tokio::test]
    async fn test_lcm_describe_invalid_uuid_returns_tool_error() {
        use rmcp::handler::server::wrapper::Parameters;
        let server = LcmServer::new();
        let result = server
            .lcm_describe(Parameters(LcmDescribeArgs {
                summary_id: "not-a-uuid".to_string(),
            }))
            .await;
        assert!(result.is_ok());
        let tool_result = result.unwrap();
        assert!(
            tool_result.is_error.unwrap_or(false),
            "expected is_error=true for invalid UUID"
        );
    }

    // ── lcm_expand tests ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_lcm_expand_unknown_id_returns_tool_error() {
        use rmcp::handler::server::wrapper::Parameters;
        let server = LcmServer::new();
        let fake_id = uuid::Uuid::new_v4().to_string();
        let result = server
            .lcm_expand(Parameters(LcmExpandArgs { summary_id: fake_id }))
            .await;
        assert!(result.is_ok());
        let tool_result = result.unwrap();
        assert!(
            tool_result.is_error.unwrap_or(false),
            "expected is_error=true for unknown summary_id"
        );
    }

    #[tokio::test]
    async fn test_lcm_expand_invalid_uuid_returns_tool_error() {
        use rmcp::handler::server::wrapper::Parameters;
        let server = LcmServer::new();
        let result = server
            .lcm_expand(Parameters(LcmExpandArgs {
                summary_id: "bad-uuid".to_string(),
            }))
            .await;
        assert!(result.is_ok());
        let tool_result = result.unwrap();
        assert!(tool_result.is_error.unwrap_or(false));
    }
```

Run `cargo test -p bacon-lcm-mcp-server` — fails to compile.

---

- [ ] **Step 5.2 — Add argument structs**

After `LcmStoreArgs` in `server.rs`, add:

```rust
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
```

---

- [ ] **Step 5.3 — Add the `parse_summary_id` helper**

After `role_from_str`, add:

```rust
/// Parse a `summary_id` string into a `Uuid`, returning a tool-level error
/// result (not a protocol error) on invalid input.
fn parse_summary_id(s: &str) -> Result<uuid::Uuid, CallToolResult> {
    uuid::Uuid::parse_str(s).map_err(|e| {
        CallToolResult::error(vec![Content::text(format!(
            "invalid summary_id '{}': {}",
            s, e
        ))])
    })
}
```

---

- [ ] **Step 5.4 — Add `lcm_describe` and `lcm_expand` to the `#[tool_router]` impl block**

Inside `#[tool_router] impl LcmServer { … }`, add after `lcm_recall`:

```rust
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
        use bacon_lcm_core::SummaryLevel;

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
        use bacon_lcm_core::MessageRole;

        let summary_id = match parse_summary_id(&args.summary_id) {
            Ok(id) => id,
            Err(err_result) => return Ok(err_result),
        };

        let mut guard = self.get_or_create_session().await?;
        let session = guard.as_mut().expect("guaranteed by get_or_create_session");

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
```

---

- [ ] **Step 5.5 — Run tests (all 13 should pass)**

```
cargo test -p bacon-lcm-mcp-server
```

Expected output:

```
running 13 tests
test server::tests::test_lcm_server_constructs_without_panic ... ok
test server::tests::test_get_or_create_session_creates_session ... ok
test server::tests::test_build_default_session ... ok
test server::tests::test_lcm_session_new_returns_uuid ... ok
test server::tests::test_lcm_session_info_returns_json ... ok
test server::tests::test_lcm_store_returns_message_id ... ok
test server::tests::test_lcm_store_unknown_role_returns_tool_error ... ok
test server::tests::test_lcm_recall_empty_session ... ok
test server::tests::test_lcm_recall_after_store ... ok
test server::tests::test_lcm_describe_unknown_id_returns_tool_error ... ok
test server::tests::test_lcm_describe_invalid_uuid_returns_tool_error ... ok
test server::tests::test_lcm_expand_unknown_id_returns_tool_error ... ok
test server::tests::test_lcm_expand_invalid_uuid_returns_tool_error ... ok

test result: ok. 13 passed; 0 failed; 0 ignored
```

---

## Task 6 — Wire up `main.rs` with env-based provider selection and stdio transport

**Files modified:**
- `mcp-server/src/main.rs` — replace the stub with the full async main
- `mcp-server/tests/smoke_test.rs` — new integration smoke test

**Commit message:** `feat(mcp-server): async main with stdio transport and env-based provider selection (Task 6)`

---

- [ ] **Step 6.1 — Write the integration smoke test first**

Create `mcp-server/tests/smoke_test.rs`:

```rust
// mcp-server/tests/smoke_test.rs
//! Smoke test: spawn the server binary, pipe JSON-RPC messages through its stdio
//! transport, and verify that all six LCM tools are advertised by `tools/list`.

use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

/// JSON-RPC 2.0 `initialize` request.
/// Must be the first call; the MCP server refuses other requests until
/// the handshake completes.
const INITIALIZE_REQUEST: &str = concat!(
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"#,
    r#""protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"#,
    r#""name":"smoke-test","version":"0.0.1"}}}"#,
    "\n"
);

/// JSON-RPC 2.0 `notifications/initialized` notification.
/// Must be sent immediately after a successful `initialize` response.
const INITIALIZED_NOTIFICATION: &str =
    "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n";

/// JSON-RPC 2.0 `tools/list` request.
const TOOLS_LIST_REQUEST: &str =
    "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n";

#[tokio::test]
async fn smoke_tools_list_returns_six_tools() {
    // `CARGO_BIN_EXE_bacon-lcm-mcp-server` is injected by Cargo when running
    // integration tests for a crate that declares a [[bin]] target.
    let binary = env!("CARGO_BIN_EXE_bacon-lcm-mcp-server");

    let mut child = Command::new(binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null()) // suppress tracing noise in test output
        .spawn()
        .expect("failed to spawn bacon-lcm-mcp-server");

    let mut stdin  = child.stdin.take().unwrap();
    let stdout     = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout).lines();

    // ── initialize handshake ──────────────────────────────────────────────────
    stdin.write_all(INITIALIZE_REQUEST.as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();

    let init_line = reader
        .next_line()
        .await
        .unwrap()
        .expect("no initialize response from server");
    let init_resp: serde_json::Value =
        serde_json::from_str(&init_line).expect("initialize response must be valid JSON");
    assert_eq!(init_resp["id"], 1, "initialize response id mismatch");
    assert!(
        init_resp["result"].is_object(),
        "initialize result must be an object; got: {}",
        init_resp
    );

    // ── send initialized notification ─────────────────────────────────────────
    stdin
        .write_all(INITIALIZED_NOTIFICATION.as_bytes())
        .await
        .unwrap();
    stdin.flush().await.unwrap();

    // ── tools/list ────────────────────────────────────────────────────────────
    stdin
        .write_all(TOOLS_LIST_REQUEST.as_bytes())
        .await
        .unwrap();
    stdin.flush().await.unwrap();

    let tools_line = reader
        .next_line()
        .await
        .unwrap()
        .expect("no tools/list response from server");
    let tools_resp: serde_json::Value =
        serde_json::from_str(&tools_line).expect("tools/list response must be valid JSON");
    assert_eq!(tools_resp["id"], 2);

    let tools = tools_resp["result"]["tools"]
        .as_array()
        .expect("tools/list result.tools must be an array");

    let tool_names: Vec<&str> = tools
        .iter()
        .map(|t| t["name"].as_str().expect("tool must have a name field"))
        .collect();

    let expected_tools = [
        "lcm_store",
        "lcm_recall",
        "lcm_describe",
        "lcm_expand",
        "lcm_session_new",
        "lcm_session_info",
    ];

    for name in &expected_tools {
        assert!(
            tool_names.contains(name),
            "tool '{}' missing from tools/list; got: {:?}",
            name,
            tool_names
        );
    }
    assert_eq!(
        tools.len(),
        6,
        "expected exactly 6 tools, got {}; tool_names: {:?}",
        tools.len(),
        tool_names
    );

    // ── clean up ──────────────────────────────────────────────────────────────
    drop(stdin);
    let _ = child.wait().await;
}
```

Run `cargo test -p bacon-lcm-mcp-server --test smoke_test` — fails because `main.rs` is still the stub.

---

- [ ] **Step 6.2 — Implement `mcp-server/src/main.rs`**

Replace the entire stub with:

```rust
// mcp-server/src/main.rs
//! bacon-lcm MCP server entry point.
//!
//! ## Environment variables
//!
//! | Variable                  | Default   | Description                                    |
//! |---------------------------|-----------|------------------------------------------------|
//! | `LCM_SUMMARIZER_PROVIDER` | `"echo"`  | Summarizer provider: echo / openai / anthropic |
//! | `LCM_SUMMARIZER_MODEL`    | `"echo"`  | Model name for the summarizer                  |
//! | `LCM_SUMMARIZER_API_KEY`  | *(none)*  | API key (required for openai/anthropic)        |
//! | `LCM_SESSION_ID`          | *(none)*  | If set, attempt to restore an existing session |
//! | `RUST_LOG`                | `"info"`  | tracing-subscriber log filter                  |

use anyhow::Context;
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

use bacon_lcm_mcp_server::server::{build_default_session, LcmServer};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Logging ───────────────────────────────────────────────────────────────
    // Write tracing output to stderr so it does not interfere with the
    // JSON-RPC framing on stdout.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!("bacon-lcm-mcp-server starting");

    // ── Optional session restore ──────────────────────────────────────────────
    let maybe_session_id = std::env::var("LCM_SESSION_ID").ok();

    let server = if let Some(ref id_str) = maybe_session_id {
        match uuid::Uuid::parse_str(id_str) {
            Ok(session_id) => {
                tracing::info!(%session_id, "attempting to restore session");
                let server = LcmServer::new();
                // Restoration from in-memory storage always fails with
                // `SessionNotFound` because in-memory storage does not survive
                // process restarts.  Log a warning and fall through to a fresh
                // session.  The daemon crate will later inject
                // `StorageLayer::postgres(pool)` to make restoration durable.
                match build_restored_session(session_id).await {
                    Ok(session) => {
                        let mut guard = server.session.lock().await;
                        *guard = Some(session);
                        tracing::info!("session restored successfully");
                        drop(guard);
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "could not restore session (in-memory storage does not \
                             persist across restarts); starting fresh"
                        );
                    }
                }
                server
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "LCM_SESSION_ID is not a valid UUID; ignoring and starting fresh"
                );
                LcmServer::new()
            }
        }
    } else {
        LcmServer::new()
    };

    // ── Start stdio transport ─────────────────────────────────────────────────
    tracing::info!("listening on stdio");
    let service = server
        .serve(stdio())
        .await
        .context("failed to start stdio transport")?;

    service.waiting().await.context("MCP service error")?;

    tracing::info!("bacon-lcm-mcp-server shutting down");
    Ok(())
}

// ── Session restore helper ────────────────────────────────────────────────────

/// Attempt to restore a session by ID using in-memory storage and env-configured
/// providers.
///
/// With in-memory storage this always returns `LcmError::SessionNotFound`.
/// The function exists as a clean extension point for when the daemon injects
/// a `StorageLayer::postgres(pool)`.
async fn build_restored_session(
    session_id: uuid::Uuid,
) -> bacon_lcm_core::LcmResult<bacon_lcm_core::LcmSession> {
    use bacon_lcm_core::{
        LcmConfig, LcmSession,
        providers::{create_embedder, create_summarizer, create_token_counter},
        storage::StorageLayer,
    };

    let provider = std::env::var("LCM_SUMMARIZER_PROVIDER")
        .unwrap_or_else(|_| "echo".to_string());
    let model = std::env::var("LCM_SUMMARIZER_MODEL")
        .unwrap_or_else(|_| "echo".to_string());
    let api_key = std::env::var("LCM_SUMMARIZER_API_KEY").ok();

    let token_counter = create_token_counter("naive", None)
        .map_err(bacon_lcm_core::LcmError::Provider)?;
    let summarizer = create_summarizer(&provider, model, None, api_key, None, None)
        .map_err(bacon_lcm_core::LcmError::Provider)?;
    let embedder = create_embedder("null", None, None, None, None)
        .map_err(bacon_lcm_core::LcmError::Provider)?;

    LcmSession::restore(
        session_id,
        token_counter,
        summarizer,
        embedder,
        LcmConfig::defaults(),
        StorageLayer::memory(),
    )
    .await
}
```

---

- [ ] **Step 6.3 — Run the smoke test**

```
cargo test -p bacon-lcm-mcp-server --test smoke_test
```

Expected output:

```
running 1 test
test smoke_tools_list_returns_six_tools ... ok

test result: ok. 1 passed; 0 failed; 0 ignored
```

---

- [ ] **Step 6.4 — Run all tests to confirm nothing is broken**

```
cargo test -p bacon-lcm-mcp-server
```

Expected output:

```
running 13 tests
test server::tests::test_lcm_server_constructs_without_panic ... ok
test server::tests::test_get_or_create_session_creates_session ... ok
test server::tests::test_build_default_session ... ok
test server::tests::test_lcm_session_new_returns_uuid ... ok
test server::tests::test_lcm_session_info_returns_json ... ok
test server::tests::test_lcm_store_returns_message_id ... ok
test server::tests::test_lcm_store_unknown_role_returns_tool_error ... ok
test server::tests::test_lcm_recall_empty_session ... ok
test server::tests::test_lcm_recall_after_store ... ok
test server::tests::test_lcm_describe_unknown_id_returns_tool_error ... ok
test server::tests::test_lcm_describe_invalid_uuid_returns_tool_error ... ok
test server::tests::test_lcm_expand_unknown_id_returns_tool_error ... ok
test server::tests::test_lcm_expand_invalid_uuid_returns_tool_error ... ok

test result: ok. 13 passed; 0 failed; 0 ignored

running 1 test
test smoke_tools_list_returns_six_tools ... ok

test result: ok. 1 passed; 0 failed; 0 ignored
```

---

- [ ] **Step 6.5 — Full workspace build sanity check**

```
cargo build --workspace
```

Expected output (no errors):

```
   Compiling bacon-lcm-core v0.1.0
   Compiling bacon-lcm-mcp-server v0.1.0
   Compiling bacon-lcm-daemon v0.1.0
   Compiling bacon-lcm-cli v0.1.0
    Finished `dev` profile [unoptimized + debuginfo] target(s)
```

---

## Complete `server.rs` final state (reference)

For implementors who prefer to work top-down, the complete final `mcp-server/src/server.rs` after all tasks is provided below. The per-task steps above build toward this incrementally; this block is the canonical source of truth if there is any discrepancy.

```rust
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
    tool_router,
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

impl LcmServer {
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

#[rmcp::tool_handler]
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
        model,    // String (not Option<String>)
        None,
        api_key,
        None,
        None,
    )
    .map_err(bacon_lcm_core::LcmError::Provider)?;
    let embedder = create_embedder("null", None, None, None, None)
        .map_err(bacon_lcm_core::LcmError::Provider)?;

    let config = LcmConfig::defaults();
    let storage = StorageLayer::memory();

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
        { let guard = server.get_or_create_session().await.unwrap(); assert!(guard.is_some()); }
        { let guard = server.get_or_create_session().await.unwrap(); assert!(guard.is_some()); }
    }

    #[tokio::test]
    async fn test_build_default_session() {
        let session = build_default_session().await;
        assert!(session.is_ok(), "default session creation failed: {:?}", session.err());
    }

    #[tokio::test]
    async fn test_lcm_session_new_returns_uuid() {
        let server = LcmServer::new();
        let result = server.lcm_session_new().await;
        assert!(result.is_ok());
        let tool_result = result.unwrap();
        assert!(!tool_result.is_error.unwrap_or(false));
        let text = match &tool_result.content[0] {
            rmcp::model::Content::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        };
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
        let text = match &tool_result.content[0] {
            rmcp::model::Content::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        };
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
            role: "user".to_string(), content: "Hello!".to_string(),
        })).await;
        assert!(result.is_ok());
        let tool_result = result.unwrap();
        assert!(!tool_result.is_error.unwrap_or(false));
        let text = match &tool_result.content[0] {
            rmcp::model::Content::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        };
        uuid::Uuid::parse_str(&text).expect("message_id should be a valid UUID");
    }

    #[tokio::test]
    async fn test_lcm_store_unknown_role_returns_tool_error() {
        let server = LcmServer::new();
        let result = server.lcm_store(Parameters(LcmStoreArgs {
            role: "unknown_role".to_string(), content: "test".to_string(),
        })).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn test_lcm_recall_empty_session() {
        let server = LcmServer::new();
        let result = server.lcm_recall().await.unwrap();
        assert!(!result.is_error.unwrap_or(false));
        let text = match &result.content[0] {
            rmcp::model::Content::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        };
        assert!(text.contains("empty"));
    }

    #[tokio::test]
    async fn test_lcm_recall_after_store() {
        let server = LcmServer::new();
        server.lcm_store(Parameters(LcmStoreArgs { role: "user".to_string(), content: "What is 2+2?".to_string() })).await.unwrap();
        server.lcm_store(Parameters(LcmStoreArgs { role: "assistant".to_string(), content: "The answer is 4.".to_string() })).await.unwrap();
        let result = server.lcm_recall().await.unwrap();
        let text = match &result.content[0] {
            rmcp::model::Content::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        };
        assert!(text.contains("What is 2+2?"));
        assert!(text.contains("The answer is 4"));
        assert!(text.contains("user:"));
        assert!(text.contains("assistant:"));
    }

    #[tokio::test]
    async fn test_lcm_describe_unknown_id_returns_tool_error() {
        let server = LcmServer::new();
        let fake_id = uuid::Uuid::new_v4().to_string();
        let result = server.lcm_describe(Parameters(LcmDescribeArgs { summary_id: fake_id })).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn test_lcm_describe_invalid_uuid_returns_tool_error() {
        let server = LcmServer::new();
        let result = server.lcm_describe(Parameters(LcmDescribeArgs { summary_id: "not-a-uuid".to_string() })).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn test_lcm_expand_unknown_id_returns_tool_error() {
        let server = LcmServer::new();
        let fake_id = uuid::Uuid::new_v4().to_string();
        let result = server.lcm_expand(Parameters(LcmExpandArgs { summary_id: fake_id })).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn test_lcm_expand_invalid_uuid_returns_tool_error() {
        let server = LcmServer::new();
        let result = server.lcm_expand(Parameters(LcmExpandArgs { summary_id: "bad-uuid".to_string() })).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_error.unwrap_or(false));
    }
}
```
