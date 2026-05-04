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

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
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
