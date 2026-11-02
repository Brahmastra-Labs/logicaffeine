//! `largo dap` — the LOGOS debugger served over the Debug Adapter Protocol.
//!
//! DAP is JSON over stdio with `Content-Length` framing (like LSP). The entire
//! protocol lives in the pure [`DapServer::handle`] transition — one request
//! object in, a response plus any events back — so every conformance test drives
//! it in-process (`dap_lock`); [`cmd_dap`] is only the stdio shell around it.
//!
//! The server is a thin serializer over
//! [`logicaffeine_compile::debug_session::DebugSession`] — the SAME facade the
//! Studio drawer renders (session_parity_lock proves they agree), so an editor
//! stepping through LOGOS over DAP and the in-browser drawer observe one truth.
//! The headline is real reverse execution: `supportsStepBack` is advertised and
//! `stepBack`/`reverseContinue` are honest time-travel (the debugger keeps the
//! whole snapshot history), not a re-run illusion.

use serde_json::{json, Value};

use logicaffeine_compile::debug_session::DebugSession;

/// The DAP server state machine: an optional live session and a monotone output
/// sequence counter (DAP stamps every message the adapter sends with a `seq`).
pub struct DapServer {
    session: Option<DebugSession>,
    out_seq: i64,
    /// Set once `disconnect`/`terminate` is seen — the stdio shell then stops.
    done: bool,
}

impl Default for DapServer {
    fn default() -> Self {
        Self::new()
    }
}

impl DapServer {
    /// A fresh server with no session yet (created on `launch`).
    pub fn new() -> DapServer {
        DapServer {
            session: None,
            out_seq: 0,
            done: false,
        }
    }

    /// Whether the peer has asked to end the session.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// The next outbound `seq` (DAP requires every adapter message to carry one).
    fn next_seq(&mut self) -> i64 {
        self.out_seq += 1;
        self.out_seq
    }

    /// Build a response to `request` (echoing its `seq` as `request_seq`).
    fn response(&mut self, request: &Value, success: bool, body: Value) -> Value {
        let request_seq = request.get("seq").and_then(Value::as_i64).unwrap_or(0);
        let command = request.get("command").and_then(Value::as_str).unwrap_or("");
        json!({
            "seq": self.next_seq(),
            "type": "response",
            "request_seq": request_seq,
            "success": success,
            "command": command,
            "body": body,
        })
    }

    /// Build a response carrying an error `message` (DAP `success: false`).
    fn error(&mut self, request: &Value, message: &str) -> Value {
        let request_seq = request.get("seq").and_then(Value::as_i64).unwrap_or(0);
        let command = request.get("command").and_then(Value::as_str).unwrap_or("");
        json!({
            "seq": self.next_seq(),
            "type": "response",
            "request_seq": request_seq,
            "success": false,
            "command": command,
            "message": message,
        })
    }

    /// Build an event.
    fn event(&mut self, name: &str, body: Value) -> Value {
        json!({
            "seq": self.next_seq(),
            "type": "event",
            "event": name,
            "body": body,
        })
    }

    /// A `stopped` event (after a step / breakpoint) or a `terminated` event when
    /// the program has run to its end — whichever the session's state calls for.
    fn stop_or_terminate(&mut self, reason: &str) -> Value {
        let running = self
            .session
            .as_ref()
            .map(DebugSession::is_running)
            .unwrap_or(false);
        if running {
            self.event(
                "stopped",
                json!({ "reason": reason, "threadId": 1, "allThreadsStopped": true }),
            )
        } else {
            self.event("terminated", json!({}))
        }
    }

    /// Handle one DAP request, returning the response followed by any events (in
    /// send order). Unknown commands get a `success: false` response, never a panic.
    pub fn handle(&mut self, request: &Value) -> Vec<Value> {
        let command = request.get("command").and_then(Value::as_str).unwrap_or("");
        match command {
            "initialize" => {
                let resp = self.response(
                    request,
                    true,
                    json!({
                        "supportsStepBack": true,
                        "supportsConfigurationDoneRequest": true,
                        "supportsEvaluateForHovers": true,
                        "supportsTerminateRequest": true,
                    }),
                );
                let initialized = self.event("initialized", json!({}));
                vec![resp, initialized]
            }

            "launch" => {
                let args = request.get("arguments").cloned().unwrap_or(Value::Null);
                let source = launch_source(&args);
                match source {
                    Err(msg) => vec![self.error(request, &msg)],
                    Ok(src) => match DebugSession::launch(&src) {
                        Err(e) => vec![self.error(request, &e)],
                        Ok(session) => {
                            self.session = Some(session);
                            let resp = self.response(request, true, json!({}));
                            let stopped = self.event(
                                "stopped",
                                json!({ "reason": "entry", "threadId": 1, "allThreadsStopped": true }),
                            );
                            vec![resp, stopped]
                        }
                    },
                }
            }

            "setBreakpoints" => {
                // Op-address breakpoints: DAP source `line`s are echoed back as
                // verified so a client can set them; `continue` honours whatever
                // the facade has armed. (Line→pc mapping is the editor bridge's
                // job, tracked with the VSCode contribution.)
                let lines = request
                    .get("arguments")
                    .and_then(|a| a.get("breakpoints"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let verified: Vec<Value> = lines
                    .iter()
                    .map(|bp| {
                        let line = bp.get("line").and_then(Value::as_i64).unwrap_or(0);
                        json!({ "verified": true, "line": line })
                    })
                    .collect();
                vec![self.response(request, true, json!({ "breakpoints": verified }))]
            }

            "configurationDone" => vec![self.response(request, true, json!({}))],

            "threads" => {
                let threads = match &self.session {
                    None => Vec::new(),
                    Some(s) => s
                        .threads()
                        .into_iter()
                        .map(|t| json!({ "id": t.id, "name": t.name }))
                        .collect(),
                };
                vec![self.response(request, true, json!({ "threads": threads }))]
            }

            "stackTrace" => {
                let frames = match &self.session {
                    None => Vec::new(),
                    Some(s) => s
                        .stack_trace()
                        .into_iter()
                        .map(|f| json!({ "id": f.id, "name": f.name, "line": f.line, "column": 0 }))
                        .collect::<Vec<_>>(),
                };
                let total = frames.len();
                vec![self.response(
                    request,
                    true,
                    json!({ "stackFrames": frames, "totalFrames": total }),
                )]
            }

            "scopes" => {
                // One Locals scope per frame; its variablesReference is frameId+1
                // (reference 0 is reserved by DAP for "no children").
                let frame_id = request
                    .get("arguments")
                    .and_then(|a| a.get("frameId"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                vec![self.response(
                    request,
                    true,
                    json!({ "scopes": [{
                        "name": "Locals",
                        "variablesReference": frame_id + 1,
                        "expensive": false,
                    }] }),
                )]
            }

            "variables" => {
                let vref = request
                    .get("arguments")
                    .and_then(|a| a.get("variablesReference"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let frame_id = vref - 1;
                let vars = match (&self.session, frame_id >= 0) {
                    (Some(s), true) => s
                        .variables(frame_id)
                        .into_iter()
                        .map(|v| {
                            json!({
                                "name": v.name,
                                "value": v.value,
                                "type": v.kind,
                                "variablesReference": 0,
                            })
                        })
                        .collect::<Vec<_>>(),
                    _ => Vec::new(),
                };
                vec![self.response(request, true, json!({ "variables": vars }))]
            }

            "next" | "stepIn" | "stepOut" | "continue" | "stepBack" | "reverseContinue" => {
                if self.session.is_none() {
                    return vec![self.error(request, "no active session — send launch first")];
                }
                {
                    let s = self.session.as_mut().unwrap();
                    match command {
                        "next" => s.next(),
                        "stepIn" => s.step_in(),
                        "stepOut" => s.step_out(),
                        "continue" => s.continue_(),
                        "stepBack" => s.step_back(),
                        "reverseContinue" => s.reverse_continue(),
                        _ => unreachable!("outer match guarantees a stepping command"),
                    }
                }
                let body = if command == "continue" {
                    json!({ "allThreadsContinued": true })
                } else {
                    json!({})
                };
                let resp = self.response(request, true, body);
                let evt = self.stop_or_terminate("step");
                vec![resp, evt]
            }

            "evaluate" => {
                let expr = request
                    .get("arguments")
                    .and_then(|a| a.get("expression"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match &self.session {
                    None => vec![self.error(request, "no active session")],
                    Some(s) => match s.evaluate(expr) {
                        Ok(result) => vec![self.response(
                            request,
                            true,
                            json!({ "result": result.trim_end(), "variablesReference": 0 }),
                        )],
                        Err(e) => vec![self.error(request, &e)],
                    },
                }
            }

            "disconnect" | "terminate" => {
                self.done = true;
                vec![self.response(request, true, json!({}))]
            }

            other => vec![self.error(request, &format!("unsupported DAP command '{other}'"))],
        }
    }
}

/// Resolve the LOGOS source for a `launch` request: an inline `source` string,
/// or a `program` file path to read. One of the two must be present.
fn launch_source(args: &Value) -> Result<String, String> {
    if let Some(src) = args.get("source").and_then(Value::as_str) {
        return Ok(src.to_string());
    }
    if let Some(path) = args.get("program").and_then(Value::as_str) {
        return std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read program '{path}': {e}"));
    }
    Err("launch needs a `source` (inline LOGOS) or a `program` (file path)".to_string())
}

/// `largo dap` — speak DAP over stdio with `Content-Length` framing until the
/// peer disconnects. Stdout carries nothing but framed DAP messages.
pub fn cmd_dap() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut server = DapServer::new();
    let mut reader = std::io::BufReader::new(stdin.lock());

    while let Some(request) = read_message(&mut reader)? {
        let outs = server.handle(&request);
        let mut out = stdout.lock();
        for msg in outs {
            write_message(&mut out, &msg)?;
        }
        out.flush()?;
        if server.is_done() {
            break;
        }
    }
    Ok(())
}

/// Read one `Content-Length`-framed JSON message from `reader`, or `None` at EOF.
fn read_message<R: std::io::BufRead>(reader: &mut R) -> std::io::Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None); // EOF
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // end of headers
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            content_length = rest.trim().parse().ok();
        }
    }
    let len = match content_length {
        Some(l) => l,
        None => return Ok(None),
    };
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    match serde_json::from_slice(&buf) {
        Ok(v) => Ok(Some(v)),
        Err(_) => Ok(Some(Value::Null)),
    }
}

/// Write one `Content-Length`-framed JSON message.
fn write_message<W: std::io::Write>(out: &mut W, msg: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(msg)?;
    write!(out, "Content-Length: {}\r\n\r\n", body.len())?;
    out.write_all(&body)?;
    Ok(())
}
