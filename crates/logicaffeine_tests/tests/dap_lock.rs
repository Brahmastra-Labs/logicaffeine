//! W11 — `dap_lock`: the DAP server's conformance transcript.
//!
//! `largo dap` — the LOGOS
//! debugger over the Debug Adapter Protocol. The whole protocol is the pure
//! [`DapServer::handle`] transition, so this drives a full session in-process:
//! initialize → launch → threads → stackTrace → scopes → variables → stepping →
//! stepBack (the time-travel headline) → evaluate → disconnect. It pins the
//! message shapes an editor depends on, and — the point of the whole wave — that
//! the DAP answers are the SAME facade truth the Studio drawer renders.

use logicaffeine_cli::dap::DapServer;
use serde_json::{json, Value};

const PROGRAM: &str = "## Main\n\
    \x20   Let x be 10.\n\
    \x20   Let y be x plus 5.\n\
    \x20   Let z be y times 2.\n\
    \x20   Show z.\n";

/// Build a DAP request object with a monotone client `seq`.
fn request(seq: i64, command: &str, arguments: Value) -> Value {
    json!({ "seq": seq, "type": "request", "command": command, "arguments": arguments })
}

/// The lone response in `outs` (asserts exactly one response is present).
fn response(outs: &[Value]) -> &Value {
    let responses: Vec<&Value> = outs
        .iter()
        .filter(|m| m.get("type").and_then(Value::as_str) == Some("response"))
        .collect();
    assert_eq!(
        responses.len(),
        1,
        "expected exactly one response, got {outs:?}"
    );
    responses[0]
}

/// The events in `outs`, in send order.
fn events<'a>(outs: &'a [Value]) -> Vec<&'a Value> {
    outs.iter()
        .filter(|m| m.get("type").and_then(Value::as_str) == Some("event"))
        .collect()
}

/// `initialize` advertises reverse execution and answers with an `initialized`
/// event — the capabilities an editor reads before it will offer step-back.
#[test]
fn initialize_advertises_time_travel() {
    let mut s = DapServer::new();
    let outs = s.handle(&request(1, "initialize", json!({})));
    let resp = response(&outs);
    assert_eq!(resp["success"], json!(true));
    assert_eq!(resp["command"], json!("initialize"));
    assert_eq!(resp["request_seq"], json!(1));
    assert_eq!(
        resp["body"]["supportsStepBack"],
        json!(true),
        "reverse execution is advertised"
    );
    assert_eq!(
        resp["body"]["supportsConfigurationDoneRequest"],
        json!(true)
    );
    // Exactly the `initialized` event follows.
    let evs = events(&outs);
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0]["event"], json!("initialized"));
}

/// `launch` compiles the program and stops at entry; a bad launch is a clean
/// `success: false`, never a panic.
#[test]
fn launch_stops_at_entry() {
    let mut s = DapServer::new();
    let outs = s.handle(&request(2, "launch", json!({ "source": PROGRAM })));
    assert_eq!(response(&outs)["success"], json!(true));
    let evs = events(&outs);
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0]["event"], json!("stopped"));
    assert_eq!(evs[0]["body"]["reason"], json!("entry"));
    assert_eq!(evs[0]["body"]["threadId"], json!(1));

    // A launch with neither source nor program fails cleanly.
    let mut s2 = DapServer::new();
    let bad = s2.handle(&request(1, "launch", json!({})));
    assert_eq!(response(&bad)["success"], json!(false));
}

/// The full editor round: threads → stackTrace → scopes → variables. Every seq
/// is stamped and monotone; the frame carries the single main thread's state.
#[test]
fn threads_stack_scopes_variables_round() {
    let mut s = DapServer::new();
    s.handle(&request(1, "initialize", json!({})));
    s.handle(&request(2, "launch", json!({ "source": PROGRAM })));
    // Step once so `x` is bound and visible in the frame.
    s.handle(&request(3, "stepIn", json!({})));
    s.handle(&request(4, "stepIn", json!({})));

    let threads = response(&s.handle(&request(5, "threads", json!({})))).clone();
    let thread_list = threads["body"]["threads"].as_array().unwrap();
    assert_eq!(thread_list.len(), 1, "single-task session: one thread");
    assert_eq!(thread_list[0]["id"], json!(1));

    let stack = response(&s.handle(&request(6, "stackTrace", json!({ "threadId": 1 })))).clone();
    let frames = stack["body"]["stackFrames"].as_array().unwrap();
    assert!(!frames.is_empty(), "there is at least a Main frame");
    let frame_id = frames[0]["id"].as_i64().unwrap();
    assert_eq!(frames[0]["name"], json!("Main"));

    let scopes = response(&s.handle(&request(7, "scopes", json!({ "frameId": frame_id })))).clone();
    let scope_list = scopes["body"]["scopes"].as_array().unwrap();
    assert_eq!(scope_list[0]["name"], json!("Locals"));
    let vref = scope_list[0]["variablesReference"].as_i64().unwrap();
    assert_eq!(
        vref,
        frame_id + 1,
        "variablesReference is frameId+1 (0 is reserved)"
    );

    let vars = response(&s.handle(&request(
        8,
        "variables",
        json!({ "variablesReference": vref }),
    )))
    .clone();
    let var_list = vars["body"]["variables"].as_array().unwrap();
    // At least one variable is x = 10 (the debug-info pass names Main locals).
    let has_x = var_list.iter().any(|v| {
        v["name"] == json!("x")
            && v["value"]
                .as_str()
                .map(|s| s.contains("10"))
                .unwrap_or(false)
    });
    assert!(
        has_x,
        "x = 10 is visible through the DAP variables call: {var_list:?}"
    );
}

/// `stepBack` is honest time-travel: after stepping forward it returns to the
/// prior op, and the frame reflects the earlier state. The response is followed
/// by a `stopped` event (the editor re-reads state on it).
#[test]
fn step_back_reverses_execution() {
    let mut s = DapServer::new();
    s.handle(&request(1, "initialize", json!({})));
    s.handle(&request(2, "launch", json!({ "source": PROGRAM })));

    // Line number on a frame is the debugger's step cursor — a proxy for "where".
    let line_after = |s: &mut DapServer, seq| {
        let st = response(&s.handle(&request(seq, "stackTrace", json!({ "threadId": 1 })))).clone();
        st["body"]["stackFrames"][0]["line"].as_i64().unwrap()
    };

    for seq in 3..=5 {
        s.handle(&request(seq, "stepIn", json!({})));
    }
    let forward_line = line_after(&mut s, 6);

    let back = s.handle(&request(7, "stepBack", json!({})));
    assert_eq!(response(&back)["success"], json!(true));
    let evs = events(&back);
    assert_eq!(evs.len(), 1, "a stopped/terminated event follows a step");
    assert_eq!(evs[0]["event"], json!("stopped"));

    let back_line = line_after(&mut s, 8);
    assert_eq!(
        back_line,
        forward_line - 1,
        "stepBack actually went back one op"
    );
}

/// `evaluate` runs a watch expression against the live frame through the
/// Wave-10 frame evaluator: after all lets, `x plus z` = 10 + 30 = 40.
#[test]
fn evaluate_reads_the_live_frame() {
    let mut s = DapServer::new();
    s.handle(&request(1, "initialize", json!({})));
    s.handle(&request(2, "launch", json!({ "source": PROGRAM })));
    // Run to completion.
    for seq in 3..=20 {
        s.handle(&request(seq, "stepIn", json!({})));
    }
    let ev = response(&s.handle(&request(
        21,
        "evaluate",
        json!({ "expression": "x plus z" }),
    )))
    .clone();
    assert_eq!(ev["success"], json!(true));
    assert_eq!(ev["body"]["result"], json!("40"));

    // A nonsense expression fails cleanly (success:false), never a panic.
    let bad = s.handle(&request(
        22,
        "evaluate",
        json!({ "expression": "wobble zibble" }),
    ));
    assert_eq!(response(&bad)["success"], json!(false));
}

/// `disconnect` acknowledges and marks the session done (the stdio shell then
/// stops); an unknown command is a clean `success: false`.
#[test]
fn disconnect_and_unknown_commands_are_graceful() {
    let mut s = DapServer::new();
    s.handle(&request(1, "initialize", json!({})));
    let unknown = s.handle(&request(2, "frobnicate", json!({})));
    assert_eq!(response(&unknown)["success"], json!(false));
    assert!(!s.is_done(), "an unknown command does not end the session");

    let bye = s.handle(&request(3, "disconnect", json!({})));
    assert_eq!(response(&bye)["success"], json!(true));
    assert!(s.is_done(), "disconnect ends the session");
}

/// Every adapter message carries a monotone `seq` — a hard DAP requirement.
#[test]
fn every_message_has_a_monotone_seq() {
    let mut s = DapServer::new();
    let mut last = 0;
    for (seq, cmd, args) in [
        (1, "initialize", json!({})),
        (2, "launch", json!({ "source": PROGRAM })),
        (3, "stepIn", json!({})),
        (4, "threads", json!({})),
    ] {
        for msg in s.handle(&request(seq, cmd, args)) {
            let m = msg["seq"].as_i64().unwrap();
            assert!(m > last, "seq must strictly increase: {m} after {last}");
            last = m;
        }
    }
}
