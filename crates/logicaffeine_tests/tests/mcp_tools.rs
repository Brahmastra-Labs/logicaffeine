//! `tools/call` behavior for `largo mcp`, in-process against the pure handler.
//!
//! Payload assertions ride the `structuredContent` field (2025-06-18 MCP);
//! every tool also mirrors the same object as pretty JSON in a text content
//! block for clients that predate structured content.

use logicaffeine_cli::mcp::McpServer;
use serde_json::{json, Value};

const SOCRATES: &str = "\
## Theorem: Socrates
Given: Socrates is a man.
Given: Every man is mortal.
Prove: Socrates is mortal.
Proof: Auto.
";

const WEAK: &str = "\
## Theory Weak

Axiom pseudo_reflexivity: for all a b, Cong(a, b, b, a).

Theorem reflexivity: prove for all a b, Cong(a, b, a, b).
";

fn initialized() -> McpServer {
    let mut server = McpServer::new();
    server
        .handle_line(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                           "clientInfo": {"name": "t", "version": "0"}}
            })
            .to_string(),
        )
        .expect("initialize response");
    server
        .handle_line(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string());
    server
}

/// Call a tool; return the full `result` object (content + structuredContent
/// + isError).
fn call(server: &mut McpServer, tool: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0", "id": 42, "method": "tools/call",
        "params": {"name": tool, "arguments": arguments}
    })
    .to_string();
    let out = server.handle_line(&line).expect("tools/call response");
    let response: Value = serde_json::from_str(&out).expect("valid JSON response");
    assert!(
        response.get("error").is_none(),
        "tools/call must not fail at the protocol layer: {response}"
    );
    response["result"].clone()
}

/// The structured payload of a tool result.
fn payload(result: &Value) -> &Value {
    &result["structuredContent"]
}

#[test]
fn check_reports_structured_diagnostics_for_bad_source() {
    let mut server = initialized();
    // `item 0` is THE canonical agent mistake (zero-based habit) and a
    // check-time diagnostic with a stable code. (Undefined variables are
    // currently runtime-only — the known binder-indexing gap.)
    let result = call(
        &mut server,
        "logos_check",
        json!({"source": "# Main\n\n## Main\n\nLet x be item 0 of xs.\n"}),
    );
    assert_eq!(
        result["isError"], false,
        "a completed check is not a tool error: {result}"
    );

    let p = payload(&result);
    assert_eq!(p["ok"], false, "{result}");
    let diagnostics = p["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "{result}");
    let d = &diagnostics[0];
    assert_eq!(d["code"], "zero-index", "{result}");
    assert_eq!(
        d["severity"], "info",
        "the LSP decision table calls zero-index an idiom note: {result}"
    );
    assert!(
        d["explanation"].as_str().is_some_and(|e| !e.is_empty()),
        "socratic prose rides along: {result}"
    );
    assert_eq!(d["span"]["start_line"], 5, "{result}");

    // The text block mirrors the same JSON for structured-content-blind clients.
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("zero-index"), "{result}");
}

#[test]
fn check_passes_hello_world() {
    let mut server = initialized();
    let result = call(
        &mut server,
        "logos_check",
        json!({"source": "# Main\n\n## Main\n\nShow \"Hello, world!\".\n"}),
    );
    assert_eq!(result["isError"], false, "{result}");
    let p = payload(&result);
    assert_eq!(p["ok"], true, "{result}");
    assert_eq!(p["diagnostics"].as_array().unwrap().len(), 0, "{result}");
}

#[test]
fn check_accepts_file_param_and_errors_on_missing_file() {
    let mut server = initialized();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("snippet.lg");
    std::fs::write(&path, "# Main\n\n## Main\n\nLet x be item 0 of xs.\n").unwrap();

    let result = call(
        &mut server,
        "logos_check",
        json!({"file": path.to_str().unwrap()}),
    );
    assert_eq!(result["isError"], false, "{result}");
    let p = payload(&result);
    assert_eq!(p["ok"], false, "{result}");
    assert_eq!(p["diagnostics"][0]["code"], "zero-index", "{result}");
    assert_eq!(
        p["diagnostics"][0]["file"],
        path.to_str().unwrap(),
        "the diagnostic names the file it came from: {result}"
    );

    let result = call(
        &mut server,
        "logos_check",
        json!({"file": "/no/such/file.lg"}),
    );
    assert_eq!(
        result["isError"], true,
        "a missing file is a tool error: {result}"
    );
}

#[test]
fn check_line_col_is_char_counted_on_multibyte_source() {
    let mut server = initialized();
    // The é sits before the error on the same line: char columns and byte
    // columns diverge, and the diagnostic must count chars.
    let source = "# Main\n\n## Main\n\nLet a be \"é\" zz.\n";
    let result = call(&mut server, "logos_check", json!({"source": source}));
    let p = payload(&result);
    assert_eq!(p["ok"], false, "{result}");
    let d = &p["diagnostics"][0];

    let start = d["span"]["start"].as_u64().unwrap() as usize;
    let start_line = d["span"]["start_line"].as_u64().unwrap() as usize;
    let start_col = d["span"]["start_col"].as_u64().unwrap() as usize;
    assert_eq!(start_line, 5, "{result}");

    // Independent char count from the line start to the span start.
    let line_start = source[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let char_col = source[line_start..start].chars().count() + 1;
    assert_eq!(
        start_col, char_col,
        "column must be char-counted, not byte-counted: {result}"
    );
    let byte_col = start - line_start + 1;
    assert_ne!(
        byte_col, char_col,
        "fixture must actually place a multibyte char before the error"
    );
}

#[test]
fn logic_compiles_english_to_fol_in_all_modes() {
    let mut server = initialized();

    let result = call(
        &mut server,
        "logos_logic",
        json!({"source": "Every man is mortal."}),
    );
    assert_eq!(result["isError"], false, "{result}");
    let fol = payload(&result)["fol"].as_str().unwrap();
    assert!(fol.contains('∀'), "default mode is unicode FOL: {fol}");

    let result = call(
        &mut server,
        "logos_logic",
        json!({"source": "Every woman loves a man.", "mode": "all_scopes"}),
    );
    let readings = payload(&result)["readings"].as_array().unwrap();
    assert!(
        readings.len() >= 2,
        "scope ambiguity must enumerate at least two readings: {result}"
    );

    let result = call(
        &mut server,
        "logos_logic",
        json!({"source": "A man walks.\nHe whistles.", "mode": "discourse"}),
    );
    assert_eq!(result["isError"], false, "{result}");
    let fol = payload(&result)["fol"].as_str().unwrap();
    assert!(
        fol.to_lowercase().contains("walk") && fol.to_lowercase().contains("whistle"),
        "discourse resolves the pronoun into one formula: {fol}"
    );
}

#[test]
fn prove_verifies_socrates_and_flags_underivable() {
    let mut server = initialized();

    let result = call(&mut server, "logos_prove", json!({"source": SOCRATES}));
    assert_eq!(result["isError"], false, "{result}");
    let p = payload(&result);
    assert_eq!(p["all_verified"], true, "{result}");
    assert_eq!(p["theorems"][0]["name"], "Socrates", "{result}");
    assert_eq!(p["theorems"][0]["verified"], true, "{result}");

    let result = call(&mut server, "logos_prove", json!({"source": WEAK}));
    let p = payload(&result);
    assert_eq!(p["all_verified"], false, "{result}");
    let theorems = p["theorems"].as_array().unwrap();
    assert!(
        theorems
            .iter()
            .any(|t| t["name"] == "reflexivity" && t["verified"] == false),
        "the underivable theorem is reported by name: {result}"
    );
}

#[test]
fn emit_produces_rust_and_wasm() {
    let mut server = initialized();
    let hello = "# Main\n\n## Main\n\nShow \"Hello, world!\".\n";

    let result = call(
        &mut server,
        "logos_emit",
        json!({"source": hello, "target": "rust"}),
    );
    assert_eq!(result["isError"], false, "{result}");
    let p = payload(&result);
    assert_eq!(p["target"], "rust", "{result}");
    assert!(
        p["code"].as_str().is_some_and(|c| c.contains("fn main")),
        "{result}"
    );

    let result = call(
        &mut server,
        "logos_emit",
        json!({"source": hello, "target": "wasm"}),
    );
    assert_eq!(result["isError"], false, "{result}");
    let p = payload(&result);
    assert_eq!(p["target"], "wasm", "{result}");
    let b64 = p["wasm_base64"].as_str().unwrap();
    assert!(
        b64.starts_with("AGFz"),
        "base64 must decode to bytes starting \\0asm: {b64}"
    );
    assert!(p["bytes"].as_u64().unwrap() > 8, "{result}");
}

#[test]
fn opts_reports_fired_optimizations() {
    let mut server = initialized();
    let source = "# Main\n\n## Main\n\nLet mutable total be 0.\nLet mutable i be 1.\n\
                  While i is less than 1000:\n    Set total to total plus i.\n\
                  \x20   Set i to i plus 1.\nShow total.\n";
    let result = call(&mut server, "logos_opts", json!({"source": source}));
    assert_eq!(result["isError"], false, "{result}");
    let p = payload(&result);
    let aot: Vec<&str> = p["aot"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        aot.contains(&"closedform"),
        "the closed-form pass must fire on an arithmetic-series loop: {result}"
    );
    assert!(p["run"].is_array() && p["vm"].is_array(), "{result}");
}
