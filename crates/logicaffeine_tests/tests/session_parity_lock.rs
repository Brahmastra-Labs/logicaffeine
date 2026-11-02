//! W11 — `session_parity_lock`: the DebugSession facade is a faithful LENS.
//!
//! The debugger puts one facade —
//! [`DebugSession`] — under both the `largo dap` server and the Studio drawer.
//! For that to be safe, the facade must never be a second source of truth: every
//! DAP-shaped answer it gives must be exactly what the underlying [`Debugger`]
//! reports. This lock steps a facade and a bare debugger in LOCKSTEP over the
//! same program and proves they agree at every op — pc, time-travel step, output
//! — and that the facade's `variables` are precisely the debugger's frame
//! registers (reordered into DAP's innermost-first convention). It then proves
//! the time-travel headline (`stepBack`) and the `evaluate` seam.

use logicaffeine_compile::debug::Debugger;
use logicaffeine_compile::debug_session::{DapThread, DebugSession};

const PROGRAM: &str = "## Main\n\
    \x20   Let x be 10.\n\
    \x20   Let y be x plus 5.\n\
    \x20   Let z be y times 2.\n\
    \x20   Show z.\n";

/// The named registers of the debugger's innermost frame, as (name, kind, value)
/// — the ground truth the facade's `variables(0)` must reproduce.
fn debugger_innermost_named(dbg: &Debugger) -> Vec<(String, String, String)> {
    let snap = dbg.snapshot();
    match snap.frames.last() {
        None => Vec::new(),
        Some(f) => f
            .registers
            .iter()
            .map(|r| {
                (
                    r.name.clone().unwrap_or_else(|| format!("R{}", r.index)),
                    r.kind.clone(),
                    r.value.clone(),
                )
            })
            .collect(),
    }
}

/// Launch parity: a fresh session and a fresh debugger agree on where they
/// stopped and what has run (nothing yet).
#[test]
fn launch_agrees_with_the_debugger() {
    let session = DebugSession::launch(PROGRAM).expect("launches");
    let dbg = Debugger::from_source(PROGRAM).expect("compiles");
    assert_eq!(session.pc(), dbg.snapshot().pc, "same starting pc");
    assert_eq!(
        session.step(),
        dbg.snapshot().step,
        "no ops executed on either"
    );
    assert_eq!(session.step(), 0, "and that is step zero");
    assert_eq!(
        session.threads(),
        vec![DapThread {
            id: 1,
            name: "main".to_string()
        }]
    );
}

/// Lockstep parity: stepping the facade and a bare debugger op-for-op keeps their
/// pc, step cursor, running flag, output, AND frame variables identical — the
/// whole way to termination. If the facade ever drifted, this catches the exact op.
#[test]
fn stepping_stays_in_lockstep_with_the_debugger() {
    let mut session = DebugSession::launch(PROGRAM).expect("launches");
    let mut dbg = Debugger::from_source(PROGRAM).expect("compiles");

    // A generous bound: the program is a handful of ops; the cap only guards a
    // runaway. We assert termination happened well within it.
    let mut steps = 0;
    while session.is_running() && steps < 1000 {
        session.step_in();
        dbg.step();
        steps += 1;

        assert_eq!(
            session.pc(),
            dbg.snapshot().pc,
            "pc diverged at step {steps}"
        );
        assert_eq!(
            session.step(),
            dbg.snapshot().step,
            "cursor diverged at step {steps}"
        );
        assert_eq!(
            session.is_running(),
            dbg.is_running(),
            "running flag diverged at step {steps}"
        );
        assert_eq!(
            session.output(),
            dbg.snapshot().output,
            "output diverged at step {steps}"
        );

        // The DAP `variables(0)` must BE the debugger's innermost frame registers.
        let facade: Vec<(String, String, String)> = session
            .variables(0)
            .into_iter()
            .map(|v| (v.name, v.kind, v.value))
            .collect();
        assert_eq!(
            facade,
            debugger_innermost_named(&dbg),
            "variables diverged at step {steps}"
        );
    }

    assert!(!session.is_running(), "the program terminates");
    assert!(
        steps < 1000,
        "and terminates promptly, not by hitting the cap"
    );
    // (10 + 5) * 2 = 30 — the real work, seen through the facade's output channel.
    assert_eq!(session.output().join("").trim_end(), "30");
}

/// Time-travel parity: `stepBack` is not a re-run illusion — after stepping
/// forward K ops and back one, the facade sits at step K-1, exactly where a
/// debugger that did the same lands. This is the `supportsStepBack` headline.
#[test]
fn step_back_is_true_time_travel() {
    let mut session = DebugSession::launch(PROGRAM).expect("launches");
    let mut dbg = Debugger::from_source(PROGRAM).expect("compiles");

    for _ in 0..3 {
        session.step_in();
        dbg.step();
    }
    let before = session.step();
    assert_eq!(before, dbg.snapshot().step, "forward parity holds");

    session.step_back();
    dbg.step_back();
    assert_eq!(
        session.step(),
        before - 1,
        "the facade actually went back one op"
    );
    assert_eq!(
        session.step(),
        dbg.snapshot().step,
        "and the debugger agrees"
    );
    assert_eq!(session.pc(), dbg.snapshot().pc, "at the identical pc");
}

/// The `evaluate` seam routes through the Wave-10 frame evaluator (the
/// tree-walker oracle): a watch expression over the paused frame's live
/// variables returns program-true values. Here, after all three lets have run,
/// `x plus z` = 10 + 30 = 40.
#[test]
fn evaluate_reads_the_live_frame_through_the_oracle() {
    let mut session = DebugSession::launch(PROGRAM).expect("launches");
    // Run to completion so x, y, z are all bound in the frame.
    let mut steps = 0;
    while session.is_running() && steps < 1000 {
        session.step_in();
        steps += 1;
    }
    // A bare variable read.
    assert_eq!(
        session.evaluate("x").expect("x is in scope").trim_end(),
        "10"
    );
    // A compound expression over two live bindings.
    assert_eq!(
        session
            .evaluate("x plus z")
            .expect("both in scope")
            .trim_end(),
        "40"
    );
}
