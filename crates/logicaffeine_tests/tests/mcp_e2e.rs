//! End-to-end MCP sessions against the real `largo mcp` binary over pipes.
//!
//! The in-process suites (`mcp_protocol.rs`, `mcp_tools.rs`) own protocol and
//! tool semantics; these tests own what only the real process can prove —
//! stdout purity, the sandbox subprocess actually executing and actually
//! dying on timeout, seed reproducibility across whole sessions, and clean
//! EOF shutdown.

#[path = "cli_common/mod.rs"]
mod common;

use std::io::Write;
use std::process::Stdio;
use std::time::{Duration, Instant};

use common::*;
use serde_json::{json, Value};

/// Run one scripted MCP session: write every line, close stdin, collect the
/// responses. Returns `(responses, exit_ok, elapsed)`.
fn session(lines: &[String]) -> (Vec<Value>, bool, Duration) {
    let start = Instant::now();
    let mut child = largo()
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("largo mcp should spawn");

    {
        let mut stdin = child.stdin.take().expect("piped stdin");
        for line in lines {
            stdin.write_all(line.as_bytes()).unwrap();
            stdin.write_all(b"\n").unwrap();
        }
        // Dropping stdin sends EOF: the server drains its queue and exits.
    }

    let output = child.wait_with_output().expect("collect server output");
    let elapsed = start.elapsed();

    let responses: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("every stdout line must be JSON-RPC ({e}): {line}"))
        })
        .collect();
    (responses, output.status.success(), elapsed)
}

fn initialize_line(id: u64) -> String {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "e2e", "version": "0"}}
    })
    .to_string()
}

fn initialized_line() -> String {
    json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string()
}

fn call_line(id: u64, tool: &str, arguments: Value) -> String {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": {"name": tool, "arguments": arguments}
    })
    .to_string()
}

/// The response carrying `id`, or panic.
fn by_id(responses: &[Value], id: u64) -> &Value {
    responses
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("no response with id {id}: {responses:?}"))
}

const HELLO: &str = "# Main\n\n## Main\n\nShow \"Hello, world!\".\n";

/// A program whose print interleaving is the scheduler's decision.
const RACY: &str = "\
# Main

## To worker:
    Show \"worker\".

## Main

Launch a task to worker.
Show \"main\".
";

const INFINITE: &str = "\
# Main

## Main

Let mutable x be 1.
While x is greater than 0:
    Set x to x plus 1.
Show x.
";

#[test]
fn full_session_runs_hello_world_with_pure_jsonrpc_stdout() {
    let (responses, exit_ok, _) = session(&[
        initialize_line(1),
        initialized_line(),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}).to_string(),
        call_line(3, "logos_run", json!({"source": HELLO})),
    ]);

    assert!(exit_ok, "clean exit after EOF");
    // Purity is already proven by session() parsing every line; the ids seal
    // request/response pairing.
    assert!(by_id(&responses, 1)["result"].is_object());
    assert!(by_id(&responses, 2)["result"]["tools"].is_array());

    let run = &by_id(&responses, 3)["result"];
    assert_eq!(run["isError"], false, "{run}");
    let p = &run["structuredContent"];
    assert_eq!(p["stdout_lines"], json!(["Hello, world!"]), "{run}");
    assert_eq!(p["exit_code"], 0, "{run}");
    assert_eq!(p["timed_out"], false, "{run}");
    assert!(
        p["determinacy"].as_str().is_some_and(|d| !d.is_empty()),
        "the payload names the program's determinacy class: {run}"
    );
}

#[test]
fn seeded_runs_are_byte_identical_across_sessions() {
    let run_once = |seed: u64| -> Value {
        let (responses, _, _) = session(&[
            initialize_line(1),
            initialized_line(),
            call_line(2, "logos_run", json!({"source": RACY, "seed": seed})),
        ]);
        let result = by_id(&responses, 2)["result"].clone();
        assert_eq!(result["isError"], false, "{result}");
        result["structuredContent"].clone()
    };

    let first = run_once(42);
    let second = run_once(42);
    assert_eq!(
        first, second,
        "same seed, whole payload byte-identical — reproducibility is the contract"
    );

    // Both prints arrive regardless of interleaving.
    let lines = first["stdout_lines"].as_array().unwrap();
    assert_eq!(lines.len(), 2, "{first}");
}

#[test]
fn timeout_kills_the_infinite_loop_and_the_server_answers_on() {
    let start = Instant::now();
    let (responses, exit_ok, _) = session(&[
        initialize_line(1),
        initialized_line(),
        call_line(
            2,
            "logos_run",
            json!({"source": INFINITE, "timeout_ms": 500}),
        ),
        json!({"jsonrpc": "2.0", "id": 3, "method": "ping", "params": {}}).to_string(),
    ]);
    let elapsed = start.elapsed();

    let run = &by_id(&responses, 2)["result"];
    assert_eq!(
        run["isError"], true,
        "a timed-out run is a tool error: {run}"
    );
    let message = run["structuredContent"]["error"].as_str().unwrap();
    assert!(
        message.contains("wall-clock") || message.contains("exceeded"),
        "the error names the limit: {message}"
    );

    // The server is not wedged: it answered the follow-up ping and exited
    // cleanly, and the whole session stayed in the same order of magnitude
    // as the 500ms budget.
    assert!(by_id(&responses, 3)["result"].is_object(), "{responses:?}");
    assert!(exit_ok);
    assert!(
        elapsed < Duration::from_secs(20),
        "the infinite loop must die at the deadline, not at the heat death: {elapsed:?}"
    );
}

#[test]
fn step_budget_stops_the_infinite_loop_deterministically() {
    let (responses, _, _) = session(&[
        initialize_line(1),
        initialized_line(),
        call_line(
            2,
            "logos_run",
            json!({"source": INFINITE, "step_budget": 100_000, "timeout_ms": 30_000}),
        ),
    ]);
    let run = &by_id(&responses, 2)["result"];
    let p = &run["structuredContent"];
    assert_eq!(
        p["exit_code"], 122,
        "budget exhaustion is the distinct exit 122, not a timeout: {run}"
    );
    assert_eq!(p["timed_out"], false, "{run}");
    assert!(
        p["error"]
            .as_str()
            .is_some_and(|e| e.contains("step budget")),
        "the error names the budget: {run}"
    );
}

#[test]
fn eof_exits_promptly_with_zero() {
    let (_, exit_ok, elapsed) = session(&[initialize_line(1)]);
    assert!(exit_ok, "EOF is a clean shutdown");
    assert!(
        elapsed < Duration::from_secs(10),
        "shutdown must be prompt: {elapsed:?}"
    );
}

/// G12 — the engine labels are STRICT: `tw` runs the tree-walker, `vm` runs
/// the bytecode VM, the payload says WHICH engine ran, and both engines
/// produce the same answer for a tier-stable program.
#[test]
fn strict_engine_labels_carry_engine_identity() {
    let (responses, _, _) = session(&[
        initialize_line(1),
        initialized_line(),
        call_line(2, "logos_run", json!({"source": HELLO, "engine": "tw"})),
        call_line(3, "logos_run", json!({"source": HELLO, "engine": "vm"})),
    ]);

    let tw = &by_id(&responses, 2)["result"];
    assert_eq!(tw["isError"], false, "{tw}");
    let p = &tw["structuredContent"];
    assert_eq!(p["strict"], true, "strict mode is declared: {tw}");
    assert_eq!(p["engine_requested"], "tw", "{tw}");
    assert_eq!(
        p["engine_ran"], "tree-walker",
        "tw label ⇒ the tree-walker ran: {tw}"
    );
    assert_eq!(p["refused"], false, "{tw}");
    assert_eq!(p["stdout_lines"], json!(["Hello, world!"]), "{tw}");

    let vm = &by_id(&responses, 3)["result"];
    assert_eq!(vm["isError"], false, "{vm}");
    let p = &vm["structuredContent"];
    assert_eq!(p["strict"], true, "{vm}");
    assert_eq!(p["engine_requested"], "vm", "{vm}");
    assert_eq!(
        p["engine_ran"], "vm",
        "vm label ⇒ the bytecode VM ran: {vm}"
    );
    assert_eq!(p["refused"], false, "{vm}");
    assert_eq!(p["stdout_lines"], json!(["Hello, world!"]), "{vm}");
}

/// G12 — requesting `vm` on a program the bytecode VM cannot run (concurrency
/// needs the scheduler driver) returns the TYPED refusal: no engine ran, the
/// error says why, and the tree-walker's output is NOT smuggled in.
#[test]
fn requesting_vm_on_an_unsupported_program_is_a_typed_refusal_not_tw_output() {
    let (responses, _, _) = session(&[
        initialize_line(1),
        initialized_line(),
        call_line(2, "logos_run", json!({"source": RACY, "engine": "vm"})),
    ]);

    let run = &by_id(&responses, 2)["result"];
    assert_eq!(
        run["isError"], false,
        "the tool did its job — the refusal is the payload: {run}"
    );
    let p = &run["structuredContent"];
    assert_eq!(p["strict"], true, "{run}");
    assert_eq!(p["engine_requested"], "vm", "{run}");
    assert!(
        p["engine_ran"].is_null(),
        "NOTHING ran — no silent substitution: {run}"
    );
    assert_eq!(p["refused"], true, "the refusal is machine-readable: {run}");
    assert_eq!(
        p["stdout_lines"],
        json!([]),
        "the tree-walker's answer must NOT appear under the vm label: {run}"
    );
    assert!(
        p["error"].as_str().is_some_and(|e| e.contains("scheduler")),
        "the refusal says WHY: {run}"
    );
}
