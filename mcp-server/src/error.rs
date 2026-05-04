// mcp-server/src/error.rs
use bacon_lcm_core::LcmError;
use rmcp::ErrorData;

/// Convert an `LcmError` into an MCP `ErrorData`.
pub fn lcm_err_to_mcp(err: LcmError) -> ErrorData {
    ErrorData::internal_error(err.to_string(), None)
}
