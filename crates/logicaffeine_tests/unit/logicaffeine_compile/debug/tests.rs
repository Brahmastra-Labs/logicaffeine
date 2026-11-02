use super::*;

const PROG: &str = "## Main\n\nLet x be 6.\nLet y be 7.\nShow x + y.";

fn run_to_done(dbg: &mut Debugger) {
    for _ in 0..10_000 {
        if dbg.snapshot().state != "paused" {
            break;
        }
        dbg.step();
    }
}

/// The pc of the first op whose disassembly starts with `prefix` (self-calibrating
/// so the tests survive a change in the exact bytecode layout).
fn pc_of(dbg: &Debugger, prefix: &str) -> usize {
    dbg.disassembly()
        .iter()
        .find(|l| l.text.starts_with(prefix))
        .unwrap_or_else(|| panic!("no `{prefix}` op in the disassembly"))
        .pc
}

#[test]
fn arms_at_entry() {
    let dbg = Debugger::from_source(PROG).expect("compiles");
    let s = dbg.snapshot();
    assert_eq!(s.step, 0, "history cursor at the start");
    assert_eq!(s.pc, 0, "stopped before the first op");
    assert_eq!(s.state, "paused");
    assert!(s.output.is_empty());
    assert!(s.total_ops > 0, "the program has instructions");
    assert!(!s.frames.is_empty(), "at least the Main frame");
}

#[test]
fn stepping_to_done_matches_a_normal_run() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    run_to_done(&mut dbg);
    let s = dbg.snapshot();
    assert_eq!(s.state, "done", "reaches completion");
    // The debugger's output is exactly the interpreter's output for the same source.
    let interp = crate::ui_bridge::interpret_for_ui_sync_with_args(PROG, &[]);
    assert_eq!(interp.error, None);
    assert_eq!(
        s.output, interp.lines,
        "stepped output == single-shot run output"
    );
}

#[test]
fn disassembly_is_faithful_to_the_source() {
    let dbg = Debugger::from_source(PROG).expect("compiles");
    let texts: Vec<&str> = dbg.disassembly().iter().map(|l| l.text.as_str()).collect();
    assert!(
        texts[0].starts_with("LoadConst"),
        "first op loads a literal"
    );
    assert_eq!(texts.last(), Some(&"Halt"), "program ends in Halt");
    assert!(
        texts.iter().any(|t| t.starts_with("Add")),
        "the `x + y` add is present"
    );
    assert!(
        texts.iter().any(|t| t.starts_with("Show")),
        "the `Show` is present"
    );
}

#[test]
fn step_into_advances_one_op() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    assert!(!dbg.snapshot().op_text.is_empty(), "paused on a real op");
    dbg.step();
    let s = dbg.snapshot();
    assert_eq!(s.step, 1, "history cursor advanced one op");
    assert_eq!(s.pc, 1, "straight-line pc advanced");
}

#[test]
fn registers_carry_their_values() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    // Run up to (not through) the Show — by then x, y, and x+y have been computed.
    let show = pc_of(&dbg, "Show");
    dbg.set_breakpoint(show);
    dbg.resume();
    let s = dbg.snapshot();
    assert_eq!(s.pc, show);
    let main = &s.frames[0];
    let values: Vec<&str> = main.registers.iter().map(|r| r.value.as_str()).collect();
    assert!(
        values.contains(&"6"),
        "x's value is live in a register: {values:?}"
    );
    assert!(
        values.contains(&"7"),
        "y's value is live in a register: {values:?}"
    );
    assert!(values.contains(&"13"), "x + y was computed: {values:?}");
    // The op that wrote the sum marked its register changed on the prior step.
    assert!(
        main.registers.iter().any(|r| r.changed),
        "the most recent write is flagged changed"
    );
}

#[test]
fn registers_carry_their_type() {
    // A learning aid: every live register reports the type of the value it holds,
    // so a student watching `x = 6` also sees `x : Int`.
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    let show = pc_of(&dbg, "Show");
    dbg.set_breakpoint(show);
    dbg.resume();
    let s = dbg.snapshot();
    let typed: Vec<(&str, &str)> = s.frames[0]
        .registers
        .iter()
        .map(|r| (r.kind.as_str(), r.value.as_str()))
        .collect();
    assert!(typed.contains(&("Int", "6")), "x:6 is typed Int: {typed:?}");
    assert!(
        typed.contains(&("Int", "13")),
        "x+y:13 is typed Int: {typed:?}"
    );
    assert!(
        s.frames[0].registers.iter().all(|r| !r.kind.is_empty()),
        "no live register is left untyped: {typed:?}"
    );
}

#[test]
fn variable_timeline_tracks_each_variable_over_time() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    dbg.resume();
    let tl = dbg.variable_timeline();
    let names: Vec<&str> = tl.vars.iter().map(|v| v.name.as_str()).collect();
    assert!(names.contains(&"x"), "x is a traced variable: {names:?}");
    assert!(names.contains(&"y"), "y is a traced variable: {names:?}");

    // Every trace spans the full (windowed) history, so the oscilloscope is aligned.
    for v in &tl.vars {
        assert_eq!(
            v.points.len(),
            tl.steps,
            "trace {} spans the timeline",
            v.name
        );
    }
    let x = tl.vars.iter().find(|v| v.name == "x").unwrap();
    assert_eq!(x.kind, "Int", "x is typed on its trace");
    // x takes the value 6 at some edge and is holding 6 once the program has run.
    assert!(
        x.points.iter().any(|p| p.value == "6" && p.changed),
        "x has a 6-edge"
    );
    assert_eq!(x.points.last().unwrap().value, "6", "x ends at 6");
    assert_eq!(tl.cursor, dbg.snapshot().step, "playhead is at the cursor");
}

#[test]
fn snapshot_carries_fol_semantics() {
    // Each executing op exposes its first-order-logic meaning.
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    let mut fols = Vec::new();
    while dbg.is_running() {
        let s = dbg.snapshot();
        if !s.fol.is_empty() {
            fols.push(s.fol.clone());
        }
        dbg.step();
    }
    assert!(
        fols.iter().any(|f| f == "R0 = 6"),
        "literal load reads as R0 = 6: {fols:?}"
    );
    assert!(
        fols.iter().any(|f| f == "x = R0"),
        "the copy into x reads as x = R0: {fols:?}"
    );
    assert!(
        fols.iter().any(|f| f.contains("x + y")),
        "the addition reads over named vars: {fols:?}"
    );
}

#[test]
fn fol_renders_a_comparison_as_a_biconditional() {
    // `is x < y` compiles to a comparison op whose FOL is a biconditional.
    let src = "## Main\n\nLet x be 6.\nLet y be 7.\nLet t be x < y.\nShow t.";
    let mut dbg = Debugger::from_source(src).expect("compiles");
    let mut fols = Vec::new();
    while dbg.is_running() {
        let s = dbg.snapshot();
        if !s.fol.is_empty() {
            fols.push(s.fol.clone());
        }
        dbg.step();
    }
    assert!(
        fols.iter()
            .any(|f| f.contains("\u{27fa}") && f.contains("x < y")),
        "the comparison reads as a biconditional: {fols:?}"
    );
}

#[test]
fn socratic_prompt_asks_the_learner_to_predict() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    let mut qs = Vec::new();
    while dbg.is_running() {
        let s = dbg.snapshot();
        if !s.socratic.is_empty() {
            qs.push(s.socratic.clone());
        }
        dbg.step();
    }
    assert!(
        qs.iter().any(|q| q.ends_with('?')
            && q.contains("sum")
            && q.contains("(6)")
            && q.contains("(7)")),
        "the addition asks the learner to predict the sum from the live operands: {qs:?}"
    );
    assert!(
        qs.iter().all(|q| q.contains('?')),
        "every Socratic prompt is a question: {qs:?}"
    );
}

#[test]
fn socratic_poses_a_yes_no_question_for_a_comparison() {
    let src = "## Main\n\nLet x be 6.\nLet y be 7.\nLet t be x < y.\nShow t.";
    let mut dbg = Debugger::from_source(src).expect("compiles");
    let mut qs = Vec::new();
    while dbg.is_running() {
        let s = dbg.snapshot();
        if !s.socratic.is_empty() {
            qs.push(s.socratic.clone());
        }
        dbg.step();
    }
    assert!(
        qs.iter()
            .any(|q| q.starts_with("Is ") && q.contains("less than") && q.ends_with('?')),
        "the comparison is posed as a yes/no question: {qs:?}"
    );
}

#[test]
fn socratic_done_state_invites_reflection() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    dbg.resume();
    let s = dbg.snapshot();
    assert!(
        s.socratic.contains("expected"),
        "the finished prompt invites reflection: {}",
        s.socratic
    );
}

#[test]
fn live_proof_is_true_now_and_proven_for_every_run() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    dbg.resume();
    let r = dbg.assert_at_cursor("x < y");
    assert!(r.parsed, "parsed the comparison");
    assert_eq!(r.now, Some(true), "x=6 < y=7 holds now: {}", r.now_detail);
    assert_eq!(
        r.verdict,
        ProofVerdict::ProvenTrue,
        "proven for every run: {}",
        r.verdict_detail
    );
    assert!(
        r.verdict_detail.contains("[6, 6]"),
        "cites the proven ranges: {}",
        r.verdict_detail
    );
}

#[test]
fn live_proof_statically_refutes_a_false_predicate() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    dbg.resume();
    let r = dbg.assert_at_cursor("x > y");
    assert_eq!(r.now, Some(false), "x=6 > y=7 is false now");
    assert_eq!(
        r.verdict,
        ProofVerdict::ProvenFalse,
        "refuted for every run: {}",
        r.verdict_detail
    );
}

#[test]
fn live_proof_proves_a_constant_equality_and_a_bound() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    dbg.resume();
    assert_eq!(
        dbg.assert_at_cursor("x == 6").verdict,
        ProofVerdict::ProvenTrue,
        "x is provably 6"
    );
    assert_eq!(
        dbg.assert_at_cursor("x >= 0").verdict,
        ProofVerdict::ProvenTrue,
        "x is provably non-negative"
    );
    assert_eq!(
        dbg.assert_at_cursor("y <= 100").verdict,
        ProofVerdict::ProvenTrue,
        "y is provably under 100"
    );
}

#[test]
fn live_proof_rejects_unparseable_input() {
    let dbg = Debugger::from_source(PROG).expect("compiles");
    let r = dbg.assert_at_cursor("hello world");
    assert!(!r.parsed, "garbage is not a comparison");
    assert_eq!(r.verdict, ProofVerdict::Unknown);
}

#[test]
fn proven_invariants_prove_constant_values_and_types() {
    // The Oracle statically proves x ≡ 6 and y ≡ 7 (singleton ranges) and Int type —
    // facts that hold on EVERY run, available before stepping.
    let dbg = Debugger::from_source(PROG).expect("compiles");
    let proven = dbg.proven_invariants();
    let names: Vec<&str> = proven.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"x"), "x has proven facts: {names:?}");
    let x = proven.iter().find(|p| p.name == "x").unwrap();
    assert!(
        x.facts.iter().any(|f| f.contains("[6, 6]")),
        "x proven \u{2208} [6,6]: {:?}",
        x.facts
    );
    assert!(
        x.facts.iter().any(|f| f.contains("Int")),
        "x proven Int: {:?}",
        x.facts
    );
    let y = proven.iter().find(|p| p.name == "y").unwrap();
    assert!(
        y.facts.iter().any(|f| f.contains("[7, 7]")),
        "y proven \u{2208} [7,7]: {:?}",
        y.facts
    );
}

#[test]
fn proven_facts_are_available_without_stepping() {
    // Proven facts come from compile-time analysis, not execution — present at cursor 0.
    let dbg = Debugger::from_source(PROG).expect("compiles");
    assert_eq!(dbg.snapshot().step, 0, "fresh debugger, nothing executed");
    assert!(
        !dbg.proven_invariants().is_empty(),
        "proven facts ready before any step"
    );
}

#[test]
fn observed_invariants_report_a_constant() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    dbg.resume();
    let ins = dbg.observed_invariants();
    let x = ins.iter().find(|i| i.name == "x").expect("x has insights");
    assert!(
        x.facts
            .iter()
            .any(|f| f.contains("constant") && f.contains('6')),
        "x is observed constant 6: {:?}",
        x.facts
    );
}

#[test]
fn observed_invariants_detect_a_monotonic_range() {
    let src = "## Main\n\nLet n be 1.\nSet n to 2.\nSet n to 3.\nShow n.";
    let mut dbg = Debugger::from_source(src).expect("compiles");
    dbg.resume();
    let ins = dbg.observed_invariants();
    let n = ins.iter().find(|i| i.name == "n").expect("n has insights");
    assert!(
        n.facts
            .iter()
            .any(|f| f.contains("range") && f.contains('1') && f.contains('3')),
        "n ranges over [1,3]: {:?}",
        n.facts
    );
    assert!(
        n.facts.iter().any(|f| f.contains("increase")),
        "n only increases: {:?}",
        n.facts
    );
}

#[test]
fn provenance_explains_why_a_value_exists() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    dbg.resume();
    let snap = dbg.snapshot();
    // The slot holding x + y = 13.
    let sum = snap
        .frames
        .last()
        .unwrap()
        .registers
        .iter()
        .find(|r| r.value == "13")
        .unwrap()
        .index;
    let node = dbg.provenance(sum).expect("13 has a provenance");
    assert_eq!(node.value, "13");
    assert!(
        node.narration.contains("add") || node.op_text.to_lowercase().contains("add"),
        "the sum was produced by an add: {} / {}",
        node.op_text,
        node.narration
    );
    // Its inputs trace back to the two literals 6 and 7.
    let input_vals: Vec<&str> = node.inputs.iter().map(|n| n.value.as_str()).collect();
    assert!(input_vals.contains(&"6"), "one input is 6: {input_vals:?}");
    assert!(input_vals.contains(&"7"), "one input is 7: {input_vals:?}");
}

#[test]
fn provenance_bottoms_out_at_a_constant_load() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    dbg.resume();
    let snap = dbg.snapshot();
    let x = snap
        .frames
        .last()
        .unwrap()
        .registers
        .iter()
        .find(|r| r.name.as_deref() == Some("x"))
        .unwrap()
        .index;
    let node = dbg.provenance(x).expect("x has a provenance");
    assert_eq!(node.value, "6");
    assert!(
        node.inputs.is_empty(),
        "a literal load consumes no registers"
    );
    assert!(
        node.narration.contains("load") || node.op_text.to_lowercase().contains("load"),
        "x came from a load: {} / {}",
        node.op_text,
        node.narration
    );
}

#[test]
fn provenance_chains_through_a_dependent_assignment() {
    // z depends on the sum, which depends on x and y — a two-level lineage.
    let src = "## Main\n\nLet x be 6.\nLet y be 7.\nLet z be x + y.\nLet w be z + x.\nShow w.";
    let mut dbg = Debugger::from_source(src).expect("compiles");
    dbg.resume();
    let snap = dbg.snapshot();
    let w = snap
        .frames
        .last()
        .unwrap()
        .registers
        .iter()
        .find(|r| r.name.as_deref() == Some("w"))
        .unwrap()
        .index;
    let node = dbg.provenance(w).expect("w has a provenance");
    assert_eq!(node.value, "19", "w = (6+7) + 6");
    // One input is z = 13, and z itself decomposes into 6 and 7.
    let z = node
        .inputs
        .iter()
        .find(|n| n.value == "13")
        .expect("w reads z=13");
    let z_inputs: Vec<&str> = z.inputs.iter().map(|n| n.value.as_str()).collect();
    assert!(
        z_inputs.contains(&"6") && z_inputs.contains(&"7"),
        "z traces to 6 and 7: {z_inputs:?}"
    );
}

#[test]
fn variable_names_are_resolved() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    let show = pc_of(&dbg, "Show");
    dbg.set_breakpoint(show);
    dbg.resume();
    let s = dbg.snapshot();
    let named: Vec<(&str, &str)> = s.frames[0]
        .registers
        .iter()
        .filter_map(|r| r.name.as_deref().map(|n| (n, r.value.as_str())))
        .collect();
    assert!(
        named.contains(&("x", "6")),
        "x = 6 shown by name: {named:?}"
    );
    assert!(
        named.contains(&("y", "7")),
        "y = 7 shown by name: {named:?}"
    );
}

#[test]
fn production_compile_carries_no_debug_names() {
    // The debugger's compile path captures variable names…
    let (dbg_prog, _proven) = compile_source(PROG).expect("debug compile");
    assert!(
        !dbg_prog.reg_names.is_empty(),
        "debug path records reg_names"
    );
    // …but the production compile path records NOTHING — the debug info is gated,
    // so it strips out of shipped builds at zero cost.
    let prod_prog = crate::ui_bridge::with_parsed_program(PROG, |parsed, interner| {
        let (stmts, types, _policies) = parsed?;
        crate::vm::Compiler::compile_with_types(stmts, interner, Some(types))
    })
    .expect("production compile");
    assert!(
        prod_prog.reg_names.is_empty(),
        "production path captures no debug names"
    );
}

#[test]
fn breakpoint_halts_continue() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    let show = pc_of(&dbg, "Show");
    dbg.set_breakpoint(show);
    dbg.resume();
    let s = dbg.snapshot();
    assert_eq!(s.pc, show, "Continue stopped at the breakpoint");
    assert!(s.at_breakpoint);
    assert_eq!(s.state, "paused");
    assert!(s.output.is_empty(), "the Show has not run yet");
    // One more step emits the output.
    dbg.step();
    assert!(
        !dbg.snapshot().output.is_empty(),
        "stepping the Show emits a line"
    );
}

#[test]
fn time_travel_steps_backwards() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    dbg.step();
    dbg.step();
    dbg.step();
    let three = dbg.snapshot();
    assert_eq!(three.step, 3);
    let pc_at_three = three.pc;
    dbg.step_back();
    assert_eq!(dbg.snapshot().step, 2, "rewound one op");
    // Forward again reproduces the same state (deterministic replay).
    dbg.step();
    let again = dbg.snapshot();
    assert_eq!(again.step, 3);
    assert_eq!(again.pc, pc_at_three, "replay is deterministic");
}

#[test]
fn restart_rewinds_to_entry() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    run_to_done(&mut dbg);
    assert_eq!(dbg.snapshot().state, "done");
    dbg.restart();
    let s = dbg.snapshot();
    assert_eq!(s.step, 0);
    assert_eq!(s.pc, 0);
    assert_eq!(s.state, "paused");
    assert!(s.output.is_empty());
}

// ── Audit: loops, calls, recursion, errors, time-travel ───────────────────

const FUNC: &str =
    "## To double (x: Int) -> Int:\n    Return x + x.\n\n## Main\nLet result be double(5).\nShow result.";
const WHILE: &str = "## Main\nLet mutable sum be 0.\nLet mutable i be 1.\nWhile i is at most 5:\n    Set sum to sum + i.\n    Set i to i + 1.\nShow sum.";

/// Step a program op-by-op to completion and return the final snapshot.
fn drive_to_end(src: &str) -> DebugSnapshot {
    let mut dbg = Debugger::from_source(src)
        .unwrap_or_else(|e| panic!("compile failed: {e}\n--- src ---\n{src}"));
    let mut guard = 0;
    while dbg.is_running() && guard < 500_000 {
        dbg.step();
        guard += 1;
    }
    dbg.snapshot()
}

/// THE soundness invariant: stepping any program op-by-op (rebuilding VM state
/// each step) must produce output BYTE-IDENTICAL to a normal interpreter run.
/// This exercises every save/restore path — `iter_stack` (loops), call frames
/// (functions/recursion), globals, and lists.
#[test]
fn corpus_stepped_output_matches_interpreter() {
    let corpus: &[(&str, &str)] = &[
        ("while_loop", WHILE),
        ("repeat_range", "## Main\nLet mutable total be 0.\nRepeat for i from 1 to 5:\n    Set total to total + i.\nShow total."),
        ("for_in", "## Main\nLet mutable sum be 0.\nRepeat for x in [10, 20, 30]:\n    Set sum to sum + x.\nShow sum."),
        ("conditional", "## Main\nLet x be 3.\nIf x is greater than 5:\n    Show \"big\".\nOtherwise:\n    Show \"small\"."),
        ("function", FUNC),
        ("recursion", "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(6)."),
        ("list", "## Main\nLet xs be [1, 2, 3].\nPush 4 to xs.\nShow length of xs."),
    ];
    for (name, src) in corpus {
        let oracle = crate::ui_bridge::interpret_for_ui_sync_with_args(src, &[]);
        assert_eq!(
            oracle.error, None,
            "{name}: the interpreter itself errored: {:?}",
            oracle.error
        );
        let snap = drive_to_end(src);
        assert_eq!(
            snap.state, "done",
            "{name}: debugger did not finish (state={}, out={:?})",
            snap.state, snap.output
        );
        assert_eq!(
            snap.output, oracle.lines,
            "{name}: stepped output diverged from the interpreter"
        );
    }
}

#[test]
fn stepping_a_while_loop_runs_all_iterations() {
    let snap = drive_to_end(WHILE);
    assert_eq!(snap.state, "done");
    assert_eq!(
        snap.output,
        vec!["15".to_string()],
        "op-by-op stepping summed 1..=5"
    );
}

#[test]
fn call_stack_descends_into_the_function() {
    let mut dbg = Debugger::from_source(FUNC).expect("compiles");
    let mut entered = false;
    let mut guard = 0;
    while dbg.is_running() && guard < 10_000 {
        let s = dbg.snapshot();
        if s.frames.len() >= 2 {
            entered = true;
            assert!(
                s.frames.last().unwrap().function.is_some(),
                "inner frame is a function"
            );
            break;
        }
        dbg.step();
        guard += 1;
    }
    assert!(entered, "stepping descends into the called function");
}

#[test]
fn step_over_runs_the_call_without_descending() {
    let mut dbg = Debugger::from_source(FUNC).expect("compiles");
    let call_pc = pc_of(&dbg, "Call");
    let mut guard = 0;
    while dbg.snapshot().pc != call_pc && dbg.is_running() && guard < 1000 {
        dbg.step();
        guard += 1;
    }
    assert_eq!(dbg.snapshot().pc, call_pc, "reached the Call op");
    let depth = dbg.snapshot().frames.len();
    dbg.step_over();
    let s = dbg.snapshot();
    assert_eq!(
        s.frames.len(),
        depth,
        "step-over did not leave us inside the callee"
    );
    assert!(
        s.pc > call_pc || s.state == "done",
        "advanced past the call"
    );
}

#[test]
fn step_out_returns_to_the_caller() {
    let mut dbg = Debugger::from_source(FUNC).expect("compiles");
    let mut guard = 0;
    while dbg.is_running() && dbg.snapshot().frames.len() < 2 && guard < 1000 {
        dbg.step();
        guard += 1;
    }
    assert_eq!(dbg.snapshot().frames.len(), 2, "inside the function");
    dbg.step_out();
    assert!(
        dbg.snapshot().frames.len() <= 1,
        "step-out returns to the caller"
    );
}

#[test]
fn runtime_error_surfaces_without_panicking() {
    let mut dbg = Debugger::from_source("## Main\nLet x be 1 / 0.\nShow x.").expect("compiles");
    let mut guard = 0;
    while dbg.is_running() && guard < 100 {
        dbg.step();
        guard += 1;
    }
    let s = dbg.snapshot();
    assert_eq!(
        s.state, "error",
        "division by zero surfaces as an error, not a panic"
    );
    assert!(s.error.is_some(), "the error carries a message");
}

/// The Studio's Output column follows the debugger through `output()` and `error()`: at
/// every step, forward and back, they say what the snapshot says.
#[test]
fn output_and_error_read_what_the_snapshot_shows() {
    let agree = |dbg: &Debugger, name: &str| {
        let s = dbg.snapshot();
        assert_eq!(
            dbg.output(),
            s.output.as_slice(),
            "{name} at step {}: output",
            s.step
        );
        assert_eq!(
            dbg.error(),
            s.error.as_deref(),
            "{name} at step {}: error",
            s.step
        );
    };
    let programs: &[(&str, &str)] = &[
        ("while_loop", WHILE),
        ("function", FUNC),
        (
            "shows",
            "## Main\nShow 1.\nShow \"two\".\nRepeat for i from 3 to 4:\n    Show i.",
        ),
        ("error", "## Main\nShow 1.\nLet x be 1 / 0.\nShow x."),
    ];
    for (name, src) in programs {
        let mut dbg = Debugger::from_source(src).expect("compiles");
        agree(&dbg, name);
        let mut guard = 0;
        while dbg.is_running() && guard < 10_000 {
            dbg.step();
            agree(&dbg, name);
            guard += 1;
        }
        assert!(
            !dbg.output().is_empty(),
            "{name}: the program printed something"
        );
        while dbg.snapshot().step > 0 {
            dbg.step_back();
            agree(&dbg, name);
        }
        assert!(
            dbg.output().is_empty() && dbg.error().is_none(),
            "{name}: rewound to the start"
        );
    }
}

#[test]
fn entering_a_function_does_not_flag_spurious_changes() {
    let mut dbg = Debugger::from_source(FUNC).expect("compiles");
    let mut guard = 0;
    while dbg.is_running() && dbg.snapshot().frames.len() < 2 && guard < 1000 {
        dbg.step();
        guard += 1;
    }
    let s = dbg.snapshot();
    assert_eq!(s.frames.len(), 2, "entered the function");
    assert!(
        s.frames
            .last()
            .unwrap()
            .registers
            .iter()
            .all(|r| !r.changed),
        "crossing into a new frame must not flag stale registers as changed"
    );
}

#[test]
fn time_travel_across_a_call_restores_the_frame() {
    let mut dbg = Debugger::from_source(FUNC).expect("compiles");
    let mut guard = 0;
    while dbg.is_running() && dbg.snapshot().frames.len() < 2 && guard < 1000 {
        dbg.step();
        guard += 1;
    }
    let inside = dbg.snapshot();
    assert_eq!(inside.frames.len(), 2, "inside the function");
    let (pc, step) = (inside.pc, inside.step);
    dbg.step();
    dbg.step_back();
    let back = dbg.snapshot();
    assert_eq!(back.step, step, "rewound to the in-function step");
    assert_eq!(back.pc, pc, "pc restored exactly");
    assert_eq!(
        back.frames.len(),
        2,
        "still inside the function after the rewind"
    );
}

#[test]
fn narration_explains_each_step_in_english() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    let mut narrations = Vec::new();
    let mut guard = 0;
    while dbg.is_running() && guard < 1000 {
        let s = dbg.snapshot();
        if !s.narration.is_empty() {
            narrations.push(s.narration.clone());
        }
        dbg.step();
        guard += 1;
    }
    let all = narrations.join(" | ");
    assert!(
        all.contains("add"),
        "an add step is narrated in English: {all}"
    );
    assert!(all.contains("print"), "the print is narrated: {all}");
    assert!(
        narrations
            .iter()
            .any(|n| n.contains("x(6)") && n.contains("y(7)")),
        "the add narration names the variables and their live values: {narrations:?}"
    );
}

#[test]
fn op_io_in_snapshot_targets_the_operands() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    let add = pc_of(&dbg, "Add");
    let mut guard = 0;
    while dbg.snapshot().pc != add && dbg.is_running() && guard < 100 {
        dbg.step();
        guard += 1;
    }
    let s = dbg.snapshot();
    assert_eq!(s.pc, add);
    assert!(
        s.op_writes.is_some(),
        "the Add writes a destination register"
    );
    assert_eq!(
        s.op_reads.len(),
        2,
        "the Add reads its two operands (for the datapath)"
    );
}

#[test]
fn seek_scrubs_anywhere_in_history() {
    let mut dbg = Debugger::from_source(WHILE).expect("compiles");
    while dbg.is_running() {
        dbg.step();
    }
    let end = dbg.snapshot();
    assert_eq!(end.state, "done");
    let total = end.total_steps;
    assert!(total > 3, "the loop took several ops");
    // Scrub to the very start.
    dbg.seek(0);
    let s0 = dbg.snapshot();
    assert_eq!(s0.step, 0);
    assert_eq!(s0.state, "paused");
    assert!(s0.output.is_empty(), "no output has happened at the entry");
    // A middle step reads "paused" (per-frame outcome), not the program's "done".
    dbg.seek(total / 2);
    assert_eq!(dbg.snapshot().step, total / 2);
    assert_eq!(dbg.snapshot().state, "paused");
    // Scrub back to the end → final output restored.
    dbg.seek(total);
    let again = dbg.snapshot();
    assert_eq!(again.step, total);
    assert_eq!(again.state, "done");
    assert_eq!(
        again.output, end.output,
        "scrubbing to the end restores the final output"
    );
}

#[test]
fn reverse_continue_runs_back_to_a_breakpoint() {
    let mut dbg = Debugger::from_source(WHILE).expect("compiles");
    let show = pc_of(&dbg, "Show");
    while dbg.is_running() {
        dbg.step();
    }
    assert_eq!(dbg.snapshot().state, "done");
    dbg.set_breakpoint(show);
    dbg.reverse_resume();
    let s = dbg.snapshot();
    assert_eq!(s.pc, show, "reverse-continue landed on the breakpoint");
    assert!(s.at_breakpoint);
    assert!(
        s.output.is_empty(),
        "rewound to the moment before the Show ran"
    );
}

#[test]
fn restart_rewinds_but_keeps_explored_history() {
    let mut dbg = Debugger::from_source(WHILE).expect("compiles");
    while dbg.is_running() {
        dbg.step();
    }
    let total = dbg.snapshot().total_steps;
    dbg.restart();
    let s = dbg.snapshot();
    assert_eq!(s.step, 0, "back at the entry");
    assert_eq!(s.state, "paused");
    assert_eq!(
        s.total_steps, total,
        "explored history is retained, so re-stepping is instant"
    );
}

#[test]
fn heap_view_lists_a_distinct_object() {
    let src = "## Main\nLet xs be [1, 2, 3].\nShow length of xs.";
    let mut dbg = Debugger::from_source(src).expect("compiles");
    while dbg.is_running() {
        dbg.step();
    }
    let s = dbg.snapshot();
    assert!(
        s.heap
            .iter()
            .any(|o| o.kind == "list" && o.referenced_by.contains(&"xs".to_string())),
        "the list `xs` shows up as a heap object: {:?}",
        s.heap
    );
}

#[test]
fn heap_view_shows_storage_layout() {
    // A list of ints is stored as a PACKED `Vec<i64>`, not boxed values — the
    // memory layout the debugger teaches.
    let src = "## Main\nLet xs be [1, 2, 3].\nShow length of xs.";
    let mut dbg = Debugger::from_source(src).expect("compiles");
    while dbg.is_running() {
        dbg.step();
    }
    let s = dbg.snapshot();
    let list = s
        .heap
        .iter()
        .find(|o| o.kind == "list")
        .expect("a list on the heap");
    assert_eq!(
        list.storage, "packed Vec<i64>",
        "an int list is densely packed: {list:?}"
    );
}

#[test]
fn heap_view_reveals_aliasing() {
    // `Let b be a` aliases the SAME list — the classic beginner trap. The heap view
    // must show ONE allocation referenced by both, not two copies.
    let src = "## Main\nLet a be [1, 2, 3].\nLet b be a.\nShow a.";
    let mut dbg = Debugger::from_source(src).expect("compiles");
    while dbg.is_running() {
        dbg.step();
    }
    let s = dbg.snapshot();
    let lists: Vec<&HeapObject> = s.heap.iter().filter(|o| o.kind == "list").collect();
    assert_eq!(
        lists.len(),
        1,
        "a and b share ONE list allocation, not two: {:?}",
        s.heap
    );
    let list = lists[0];
    assert!(
        list.referenced_by.contains(&"a".to_string()),
        "`a` references it: {list:?}"
    );
    assert!(
        list.referenced_by.contains(&"b".to_string()),
        "`b` references it: {list:?}"
    );
    assert!(list.shared, "aliasing is flagged");
}

#[test]
fn heap_reference_count_is_the_programs_own() {
    // `a` and `b` name one list. The debugger keeps a copy of the VM for every step of
    // its rewind history; none of those copies are the program's references.
    let src = "## Main\nLet a be [1, 2, 3].\nLet b be a.\nShow a.";
    let mut dbg = Debugger::from_source(src).expect("compiles");
    while dbg.is_running() {
        dbg.step();
    }
    let list_rc = |dbg: &Debugger| {
        let s = dbg.snapshot();
        let list = s
            .heap
            .iter()
            .find(|o| o.kind == "list")
            .cloned()
            .expect("a list on the heap");
        (list.rc, list.referenced_by.len())
    };
    let (rc, roots) = list_rc(&dbg);
    assert!(rc >= 2, "a and b both hold the list: rc {rc}");
    assert_eq!(rc, roots, "every handle on it is a root in Main");
    for _ in 0..4 {
        while dbg.snapshot().step > 0 {
            dbg.step_back();
        }
        while dbg.is_running() {
            dbg.step();
        }
    }
    assert_eq!(
        list_rc(&dbg).0,
        rc,
        "rewinding and replaying the run must not change the count"
    );
}

#[test]
fn function_frames_name_their_variables() {
    let mut dbg = Debugger::from_source(FUNC).expect("compiles");
    let mut guard = 0;
    while dbg.is_running() && dbg.snapshot().frames.len() < 2 && guard < 1000 {
        dbg.step();
        guard += 1;
    }
    let s = dbg.snapshot();
    assert_eq!(s.frames.len(), 2, "inside double");
    let names: Vec<Option<String>> = s.frames[1]
        .registers
        .iter()
        .map(|r| r.name.clone())
        .collect();
    assert!(
        names.contains(&Some("x".to_string())),
        "the parameter x is named inside double: {names:?}"
    );
}

#[test]
fn the_call_stack_names_each_function() {
    let mut dbg = Debugger::from_source(FUNC).expect("compiles");
    let mut guard = 0;
    while dbg.is_running() && dbg.snapshot().frames.len() < 2 && guard < 1000 {
        dbg.step();
        guard += 1;
    }
    let s = dbg.snapshot();
    assert_eq!(s.frames[0].function, None, "the bottom frame is Main");
    assert_eq!(
        s.frames[1].function.as_deref(),
        Some("double"),
        "the callee is named, not numbered"
    );
}

#[test]
fn production_compile_carries_no_function_debug_info() {
    let (dbg_prog, _proven) = compile_source(FUNC).expect("debug compile");
    assert_eq!(
        dbg_prog.fn_debug.len(),
        dbg_prog.functions.len(),
        "the debug path describes every function"
    );
    let prod_prog = crate::ui_bridge::with_parsed_program(FUNC, |parsed, interner| {
        let (stmts, types, _policies) = parsed?;
        crate::vm::Compiler::compile_with_types(stmts, interner, Some(types))
    })
    .expect("production compile");
    assert!(
        prod_prog.fn_debug.is_empty(),
        "production captures no function debug info"
    );
}

#[test]
fn stack_frames_carry_their_base_address() {
    let mut dbg = Debugger::from_source(FUNC).expect("compiles");
    let mut guard = 0;
    while dbg.is_running() && dbg.snapshot().frames.len() < 2 && guard < 1000 {
        dbg.step();
        guard += 1;
    }
    let s = dbg.snapshot();
    assert_eq!(s.frames.len(), 2, "inside the function");
    assert_eq!(
        s.frames[0].base, 0,
        "the Main frame starts at stack address 0"
    );
    assert!(
        s.frames[1].base > 0,
        "the callee frame is stacked above Main (higher address)"
    );
}
