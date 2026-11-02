//! The register VM reclaims a top-level statement's scratch registers when the statement ends.
//!
//! MEASURED before this lock: a straight-line `Main` of `Set item i of X to v.` cost EXACTLY two registers
//! per statement and never returned them — 100 statements → 207 registers, 1,000 → 2,007, 4,000 → 8,007.
//! The allocator's own documentation says scratch is "dead at every statement boundary by the allocator's
//! recycling discipline", but the only recycler was `exit_block`, which fires when an `If`/`While` body
//! closes; top-level `Main` never enters one. So any straight-line program hit the 16,384-register frame
//! at about 8,000 statements — and a single real-width Qwen3.8 tensor fill is 5,120. The VM is the oracle
//! every other tier is checked against, so an oracle that cannot hold a real-width fill cannot check
//! anything at the scale that matters.
//!
//! The same leak held in every frame and block: a function body or a loop body cost three registers per
//! statement (100 → 303, 4,000 → 12,003), so the reclaim lives in `compile_stmt` itself, at every statement
//! boundary, not only in `Main`'s top-level loop.
//!
//! Two properties, both about behavior rather than a counter:
//!   * the register cost of straight-line code is independent of its length, in Main, a function, a loop;
//!   * programs past the old ceiling run, and print exactly what the tree-walking interpreter — the
//!     reference semantics — prints.
//!
//! ★ MUTATION PROBES (both run 2026-09-23):
//!   * drop the reclaim (`compile_stmt` never calls `reclaim_scratch`) → all six red: the counts grow and the
//!     long programs die with `out of registers`.
//!   * reclaim too far (floor = the statement's start, ignoring live bindings) → the three size tests stay
//!     green but every answer test goes red (`Cannot index into Int`; VM ≠ interpreter): a binding created by
//!     a statement was handed out again as scratch. The answer tests are what make "reclaim" mean "safely".
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test vm_register_recycling_lock`

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

fn register_count(src: &str) -> usize {
    frame_sizes(src).0
}

/// `(Main's frame size, every function's frame size)`.
fn frame_sizes(src: &str) -> (usize, Vec<usize>) {
    logicaffeine_compile::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _p) = parsed.expect("parses");
        let prog =
            logicaffeine_compile::vm::Compiler::compile_with_types(stmts, interner, Some(types))
                .expect("compiles");
        (
            prog.register_count,
            prog.functions.iter().map(|f| f.register_count).collect(),
        )
    })
}

/// `n` straight-line accumulations, each with its own scratch, indented for a body at `indent`.
fn accumulations(n: usize, indent: &str) -> String {
    (1..=n)
        .map(|i| format!("{indent}Set total to total + {i} * {}.\n", i % 5 + 1))
        .collect()
}

fn in_a_function(n: usize) -> String {
    format!(
        "## To accumulate (seed: Int) -> Int:\n    Let mutable total be seed.\n{}    Return total.\n\n## Main\nShow accumulate(7).\n",
        accumulations(n, "    ")
    )
}

fn in_a_loop(n: usize) -> String {
    format!(
        "## Main\nLet mutable total be 0.\nLet mutable k be 0.\nWhile k is less than 3:\n{}    Set k to k + 1.\nShow total.\n",
        accumulations(n, "    ")
    )
}

fn fills(n: usize) -> String {
    let mut s = format!("## Main\nLet X be a 1 by {n} tensor of Real.\n");
    for i in 1..=n {
        s += &format!("Set item {i} of X to {:?}.\n", (i % 7) as f64 * 0.25);
    }
    s += "Show item 1 of X.\n";
    s
}

/// ── STRAIGHT-LINE CODE COSTS A CONSTANT NUMBER OF REGISTERS.
#[test]
fn register_cost_does_not_grow_with_program_length() {
    let (short, long) = (register_count(&fills(100)), register_count(&fills(4000)));
    println!("100 statements -> {short} registers, 4000 statements -> {long} registers");
    assert_eq!(
        long, short,
        "40x more straight-line statements must not need more registers ({short} -> {long})"
    );
}

/// ── NAMES SURVIVE THE RECLAIM. A binding made by one statement is read by later ones, across a span long
/// enough that any register it lived in would have been reused many times over if it were treated as
/// scratch.
#[test]
fn a_binding_outlives_the_scratch_around_it() {
    let mut s = String::from("## Main\nLet a be 11.\nLet mutable total be 0.\n");
    for i in 0..3000 {
        s += &format!("Let t{i} be a * {i}.\nSet total to total + t{i} - a * {i}.\n");
    }
    s += "Show a.\nShow total.\nShow t2999.\n";
    let (vm, tw) = (vm_outcome(&s), tw_outcome(&s));
    assert!(vm.error.is_none(), "the VM runs it: {:?}", vm.error);
    // `tw_outcome` omits the final newline `vm_outcome` keeps; the differential locks compare trimmed output.
    assert_eq!(
        vm.output.trim(),
        tw.output.trim(),
        "the VM and the interpreter disagree"
    );
    assert_eq!(
        vm.output.split_whitespace().collect::<Vec<_>>(),
        vec!["11", "0", &(11 * 2999).to_string()]
    );
}

/// ── THE PROGRAM THAT FOUND THIS. Qwen3.8's real `d = 5120`, every element of X and W filled, the norm, and
/// every element of the result printed. The VM used to refuse it (`out of registers`); it must now run and
/// print what the reference interpreter prints, element for element.
#[test]
fn a_real_width_rmsnorm_runs_on_the_vm_and_matches_the_interpreter() {
    let d = 5120usize;
    let mut s = format!("## Main\nLet X be a 1 by {d} tensor of Real.\n");
    for i in 1..=d {
        s += &format!(
            "Set item {i} of X to {:?}.\n",
            ((i * 37) % 101) as f64 / 50.0 - 1.0
        );
    }
    s += &format!("Let W be a 1 by {d} tensor of Real.\n");
    for i in 1..=d {
        s += &format!(
            "Set item {i} of W to {:?}.\n",
            0.5 + ((i * 13) % 17) as f64 / 34.0
        );
    }
    s += &format!("Let Y be rmsnorm of X with weight W.\nLet mutable i be 1.\nWhile i is at most {d}:\n    Show round(item i of Y * 1000000).\n    Set i to i + 1.\n");
    let vm = vm_outcome(&s);
    assert!(
        vm.error.is_none(),
        "the VM runs a real-width fill: {:?}",
        vm.error
    );
    let tw = tw_outcome(&s);
    assert!(tw.error.is_none(), "{:?}", tw.error);
    assert_eq!(
        vm.output.split_whitespace().count(),
        d,
        "every element is printed"
    );
    assert_eq!(
        vm.output.trim(),
        tw.output.trim(),
        "the VM and the interpreter disagree on a real-width rmsnorm"
    );
}

/// ── THE DISCIPLINE HOLDS IN EVERY FRAME, NOT ONLY MAIN. A function body is its own frame; a straight-line
/// body there must cost the same few registers whether it is 100 statements long or 4000.
#[test]
fn a_function_body_costs_a_constant_number_of_registers() {
    let (short, long) = (
        frame_sizes(&in_a_function(100)).1,
        frame_sizes(&in_a_function(4000)).1,
    );
    println!("function frame: 100 statements -> {short:?}, 4000 statements -> {long:?}");
    assert_eq!(
        long, short,
        "a 40x longer function body must not need a larger frame"
    );
}

/// ── AND IN EVERY BLOCK. A loop body is recycled when the loop closes, but the statements INSIDE it each
/// hold their scratch until then — so a long body alone can exhaust the frame.
#[test]
fn a_loop_body_costs_a_constant_number_of_registers() {
    let (short, long) = (
        register_count(&in_a_loop(100)),
        register_count(&in_a_loop(4000)),
    );
    println!("loop body: 100 statements -> {short} registers, 4000 statements -> {long} registers");
    assert_eq!(
        long, short,
        "a 40x longer loop body must not need more registers"
    );
}

/// ── AND THE ANSWER IS UNCHANGED. Past the old ~8,000-statement ceiling, inside a function and inside a
/// loop, the VM prints exactly what the reference interpreter prints.
#[test]
fn long_function_and_loop_bodies_run_on_the_vm_and_match_the_interpreter() {
    for (what, src) in [("function", in_a_function(9000)), ("loop", in_a_loop(9000))] {
        let (vm, tw) = (vm_outcome(&src), tw_outcome(&src));
        assert!(
            vm.error.is_none(),
            "{what}: the VM runs a 9000-statement body: {:?}",
            vm.error
        );
        assert!(tw.error.is_none(), "{what}: {:?}", tw.error);
        assert_eq!(
            vm.output.trim(),
            tw.output.trim(),
            "{what}: the VM and the interpreter disagree"
        );
    }
}
