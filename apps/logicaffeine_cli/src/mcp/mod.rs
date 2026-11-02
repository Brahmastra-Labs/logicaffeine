//! `largo mcp` — the LOGOS toolchain served over the Model Context Protocol.
//!
//! MCP is JSON-RPC 2.0 over stdio, one newline-delimited JSON object per
//! line. The entire protocol lives in the pure [`McpServer::handle_line`]
//! transition — one input line in, zero or one output line back — so every
//! conformance test drives it in-process; [`cmd_mcp`] is only the stdio
//! shell around it. Stdout carries nothing but JSON-RPC; logs go to stderr.
//!
//! Register with an agent client as:
//!
//! ```text
//! claude mcp add logos -- largo mcp
//! ```

mod protocol;
mod resources;
mod sandbox;
mod tools;

use serde_json::{json, Value};

use protocol::{Incoming, INVALID_PARAMS, METHOD_NOT_FOUND, NOT_INITIALIZED};

/// The MCP server state machine.
pub struct McpServer {
    initialized: bool,
}

impl McpServer {
    /// A fresh, un-initialized server.
    pub fn new() -> Self {
        McpServer { initialized: false }
    }

    /// Handle one JSON-RPC line. Returns the response line, or `None` for
    /// notifications (which never get responses).
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let incoming = match protocol::parse_line(line) {
            Ok(incoming) => incoming,
            Err(response) => return Some(response),
        };
        let Incoming { id, method, params } = incoming;

        // Notifications are consumed silently — including
        // `notifications/initialized`, which carries no state we don't
        // already have (the `initialize` request itself opens the session).
        let id = id?;

        let response = match method.as_str() {
            "initialize" => {
                self.initialized = true;
                let requested = params.get("protocolVersion").and_then(Value::as_str);
                protocol::ok(
                    &id,
                    json!({
                        "protocolVersion": protocol::negotiate_version(requested),
                        "capabilities": {
                            "tools": {},
                            "resources": {},
                            "prompts": {},
                        },
                        "serverInfo": {
                            "name": "logos",
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                    }),
                )
            }
            "ping" => protocol::ok(&id, json!({})),
            _ if !self.initialized => protocol::error(
                &id,
                NOT_INITIALIZED,
                "server is not initialized: send initialize first",
            ),
            "tools/list" => protocol::ok(&id, tools::list()),
            "tools/call" => tools::call(&id, &params),
            "resources/list" => protocol::ok(&id, resources::list()),
            "resources/read" => resources::read(&id, &params),
            "prompts/list" => protocol::ok(&id, prompts_list()),
            "prompts/get" => prompts_get(&id, &params),
            other => protocol::error(&id, METHOD_NOT_FOUND, &format!("unknown method '{other}'")),
        };
        Some(response)
    }
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

/// The one prompt the server ships: how to write LOGOS.
const WRITE_LOGOS_PROMPT: &str = "\
You are writing LOGOS, the English programming language. Before writing any \
code, read the resource logos://quickguide — it teaches the whole language \
on one page. The essentials: programs live in `## Main` blocks; every \
statement is an English sentence ending with a period; `Let x be 5.` \
declares, `Set x to 6.` mutates (requires `Let mutable`), `Show x.` prints; \
indices are 1-based; value comparison is `is equal to` / `equals`, never \
bare `is`. Check your work with the logos_check tool (structured \
diagnostics with teaching explanations), then execute with logos_run.";

/// The `prompts/list` result.
fn prompts_list() -> Value {
    json!({"prompts": [{
        "name": "write-logos",
        "description": "Orient an agent that is about to write LOGOS code",
        "arguments": [],
    }]})
}

/// The `prompts/get` response line.
fn prompts_get(id: &Value, params: &Value) -> String {
    match params.get("name").and_then(Value::as_str) {
        Some("write-logos") => protocol::ok(
            id,
            json!({
                "description": "Orient an agent that is about to write LOGOS code",
                "messages": [{
                    "role": "user",
                    "content": {"type": "text", "text": WRITE_LOGOS_PROMPT},
                }],
            }),
        ),
        Some(other) => protocol::error(id, INVALID_PARAMS, &format!("unknown prompt '{other}'")),
        None => protocol::error(id, INVALID_PARAMS, "prompts/get needs a 'name'"),
    }
}

/// Handle `largo mcp`: serve MCP over stdio until EOF.
///
/// With `--worker <engine>` (hidden), run the sandbox worker instead — the
/// self-invocation target of the `logos_run` tool, never called by users.
pub fn cmd_mcp(
    worker: Option<String>,
    worker_args: Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(engine) = worker {
        return sandbox::worker(&engine, &worker_args);
    }

    use std::io::{BufRead, Write};

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut server = McpServer::new();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = server.handle_line(&line) {
            let mut out = stdout.lock();
            out.write_all(response.as_bytes())?;
            out.write_all(b"\n")?;
            out.flush()?;
        }
    }
    Ok(())
}
