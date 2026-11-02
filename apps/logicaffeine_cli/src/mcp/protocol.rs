//! JSON-RPC 2.0 envelope for the MCP server: request parsing, response
//! rendering, error codes, and protocol-version negotiation.

use serde_json::{json, Value};

/// MCP protocol revisions this server speaks, newest first. Negotiation
/// echoes the client's requested version when it is in this list and offers
/// the newest otherwise.
pub const SUPPORTED_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// JSON-RPC: malformed JSON.
pub const PARSE_ERROR: i64 = -32700;
/// JSON-RPC: structurally invalid request object.
pub const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC: unknown method.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC: bad method parameters.
pub const INVALID_PARAMS: i64 = -32602;
/// MCP: request received before the initialize handshake.
pub const NOT_INITIALIZED: i64 = -32002;

/// One parsed incoming message.
pub struct Incoming {
    /// `None` = notification (no `id` member): never answered.
    pub id: Option<Value>,
    pub method: String,
    pub params: Value,
}

/// Parse one line. `Err` carries the ready-to-send parse-error response.
pub fn parse_line(line: &str) -> Result<Incoming, String> {
    let value: Value = serde_json::from_str(line)
        .map_err(|e| error(&Value::Null, PARSE_ERROR, &format!("malformed JSON: {e}")))?;
    let id = value.get("id").cloned();
    let method = match value.get("method").and_then(Value::as_str) {
        Some(m) if !m.is_empty() => m.to_string(),
        _ => {
            return Err(error(
                id.as_ref().unwrap_or(&Value::Null),
                INVALID_REQUEST,
                "request object has no method",
            ))
        }
    };
    let params = value.get("params").cloned().unwrap_or_else(|| json!({}));
    Ok(Incoming { id, method, params })
}

/// A success response line.
pub fn ok(id: &Value, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

/// An error response line.
pub fn error(id: &Value, code: i64, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}

/// The version the server answers an `initialize` carrying `requested`.
pub fn negotiate_version(requested: Option<&str>) -> &'static str {
    requested
        .and_then(|r| SUPPORTED_VERSIONS.iter().find(|v| **v == r))
        .copied()
        .unwrap_or(SUPPORTED_VERSIONS[0])
}
