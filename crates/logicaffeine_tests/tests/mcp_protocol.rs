//! Protocol conformance for `largo mcp` — the MCP server's JSON-RPC core.
//!
//! Every test drives the pure handler (`McpServer::handle_line`) in-process:
//! one input line in, zero or one output line back. No process spawning —
//! the stdio shell is a 20-line wrapper tested end-to-end in `mcp_e2e.rs`.

use logicaffeine_cli::mcp::McpServer;
use serde_json::{json, Value};

/// Serialize a request line.
fn req(id: Value, method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

/// Serialize a notification line (no id).
fn notification(method: &str) -> String {
    json!({"jsonrpc": "2.0", "method": method}).to_string()
}

/// Handle one line and parse the response.
fn roundtrip(server: &mut McpServer, line: &str) -> Value {
    let out = server
        .handle_line(line)
        .unwrap_or_else(|| panic!("expected a response for: {line}"));
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("response is not JSON ({e}): {out}"))
}

/// A server that has completed the initialize handshake.
fn initialized() -> McpServer {
    let mut server = McpServer::new();
    let response = roundtrip(
        &mut server,
        &req(
            json!(1),
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test-harness", "version": "0.0.0"}
            }),
        ),
    );
    assert!(
        response.get("result").is_some(),
        "initialize must succeed: {response}"
    );
    assert!(
        server
            .handle_line(&notification("notifications/initialized"))
            .is_none(),
        "notifications never get responses"
    );
    server
}

#[test]
fn initialize_echoes_supported_version_and_advertises_capabilities() {
    let mut server = McpServer::new();
    let response = roundtrip(
        &mut server,
        &req(
            json!(1),
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "t", "version": "0"}
            }),
        ),
    );
    let result = &response["result"];
    assert_eq!(result["protocolVersion"], "2025-06-18", "{response}");
    assert!(result["capabilities"]["tools"].is_object(), "{response}");
    assert!(
        result["capabilities"]["resources"].is_object(),
        "{response}"
    );
    assert!(result["capabilities"]["prompts"].is_object(), "{response}");
    assert_eq!(result["serverInfo"]["name"], "logos", "{response}");
    assert!(
        result["serverInfo"]["version"]
            .as_str()
            .is_some_and(|v| !v.is_empty()),
        "{response}"
    );
}

#[test]
fn initialize_negotiates_unknown_version_down() {
    let mut server = McpServer::new();
    let response = roundtrip(
        &mut server,
        &req(
            json!(1),
            "initialize",
            json!({"protocolVersion": "1999-01-01", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}),
        ),
    );
    // Not an error: the server offers its latest supported version instead.
    assert_eq!(
        response["result"]["protocolVersion"], "2025-06-18",
        "{response}"
    );
}

#[test]
fn requests_before_initialize_are_rejected() {
    let mut server = McpServer::new();
    let response = roundtrip(&mut server, &req(json!(1), "tools/list", json!({})));
    assert!(
        response.get("error").is_some(),
        "tools/list before initialize must error: {response}"
    );

    let mut ready = initialized();
    let response = roundtrip(&mut ready, &req(json!(2), "tools/list", json!({})));
    assert!(
        response.get("result").is_some(),
        "tools/list after handshake must succeed: {response}"
    );
}

#[test]
fn initialized_notification_gets_no_response() {
    let mut server = initialized();
    // A second stray notification is also silent.
    assert!(server
        .handle_line(&notification("notifications/initialized"))
        .is_none());
}

#[test]
fn unknown_method_and_garbage_and_bad_params_map_to_jsonrpc_errors() {
    let mut server = initialized();

    let response = roundtrip(&mut server, &req(json!(7), "no/such/method", json!({})));
    assert_eq!(response["error"]["code"], -32601, "{response}");

    let response = roundtrip(&mut server, "this is not json {");
    assert_eq!(response["error"]["code"], -32700, "{response}");
    assert!(
        response["id"].is_null(),
        "parse errors carry id null: {response}"
    );

    // tools/call without a tool name is invalid params.
    let response = roundtrip(&mut server, &req(json!(8), "tools/call", json!({})));
    assert_eq!(response["error"]["code"], -32602, "{response}");
}

#[test]
fn ping_works_pre_and_post_init() {
    let mut fresh = McpServer::new();
    let response = roundtrip(&mut fresh, &req(json!("p1"), "ping", json!({})));
    assert!(
        response.get("result").is_some(),
        "ping pre-init: {response}"
    );

    let mut ready = initialized();
    let response = roundtrip(&mut ready, &req(json!("p2"), "ping", json!({})));
    assert!(
        response.get("result").is_some(),
        "ping post-init: {response}"
    );
}

#[test]
fn tools_list_names_exactly_the_v1_tools_with_object_schemas() {
    let mut server = initialized();
    let response = roundtrip(&mut server, &req(json!(3), "tools/list", json!({})));
    let tools = response["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools/list must return a tools array: {response}"));

    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "logos_check",
            "logos_emit",
            "logos_logic",
            "logos_opts",
            "logos_prove",
            "logos_run"
        ],
        "the v1 tool surface is pinned"
    );

    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        assert!(
            tool["description"].as_str().is_some_and(|d| !d.is_empty()),
            "{name} needs a description"
        );
        assert_eq!(
            tool["inputSchema"]["type"], "object",
            "{name} schema must be an object schema: {tool}"
        );
        assert!(
            tool["inputSchema"]["properties"].is_object(),
            "{name} schema needs properties: {tool}"
        );
    }
}

#[test]
fn resources_list_and_read_serve_the_quickguide() {
    let mut server = initialized();
    let response = roundtrip(&mut server, &req(json!(4), "resources/list", json!({})));
    let resources = response["result"]["resources"].as_array().unwrap();
    assert!(
        resources.iter().any(|r| r["uri"] == "logos://quickguide"),
        "quickguide must be listed: {response}"
    );

    let response = roundtrip(
        &mut server,
        &req(
            json!(5),
            "resources/read",
            json!({"uri": "logos://quickguide"}),
        ),
    );
    let contents = &response["result"]["contents"][0];
    assert_eq!(contents["uri"], "logos://quickguide", "{response}");
    assert_eq!(contents["mimeType"], "text/markdown", "{response}");
    assert!(
        contents["text"]
            .as_str()
            .is_some_and(|t| t.contains("LOGOS")),
        "quickguide text must be embedded: {response}"
    );

    let response = roundtrip(
        &mut server,
        &req(
            json!(6),
            "resources/read",
            json!({"uri": "logos://no-such-resource"}),
        ),
    );
    assert!(
        response.get("error").is_some(),
        "unknown URI must error: {response}"
    );
}

#[test]
fn response_id_echoes_request_id_forms() {
    let mut server = initialized();

    let response = roundtrip(&mut server, &req(json!("string-id"), "ping", json!({})));
    assert_eq!(response["id"], "string-id");

    let response = roundtrip(&mut server, &req(json!(0), "ping", json!({})));
    assert_eq!(response["id"], 0);
}
