//! ★ LOCK — a hot loop is specialized on the flags it never changes. ★
//!
//! A branch whose condition the loop never writes goes the same way on every iteration of an entry: a `Bool` flag read
//! from outside the loop, or an `Int` the loop never writes compared against a constant. The JIT compiles the loop
//! against the value it saw: the branch folds, the side it never takes is dropped, and the region carries a value guard
//! on the flag. A dropped side may hold what the JIT cannot lower (here, a `Show`), so a loop that stayed on bytecode
//! now runs native. An entry that finds the flag changed runs on bytecode, and its output is the same.
//!
//! Every case prints exactly what the tree-walker prints.

#![cfg(all(target_arch = "x86_64", not(target_os = "windows")))]

use logicaffeine_compile::compile::tw_outcome_with_args;
use logicaffeine_compile::vm::ScalarValue;
use logicaffeine_jit::{optimization_trace_jit, JitBackendOptimizationTrace};

const NATIVES: &str = "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n";

fn run(body: &str, argument: &str) -> JitBackendOptimizationTrace {
    let source = format!("{NATIVES}## Main\n{body}");
    let args = ["logos".to_string(), argument.to_string()];
    let traced = optimization_trace_jit(&source, &args);
    let tree = tw_outcome_with_args(&source, &args);
    assert_eq!(traced.outcome.error, tree.error, "the JIT and the tree-walker disagree on the error:\n{source}");
    assert_eq!(
        traced.outcome.output.trim(),
        tree.output.trim(),
        "the JIT and the tree-walker disagree on the output for {argument}:\n{source}"
    );
    traced
}

/// The values the run's specialized loops were specialized on.
fn specialized_on(traced: &JitBackendOptimizationTrace) -> Vec<ScalarValue> {
    traced.value_specializations.iter().flat_map(|s| s.guards.iter().map(|g| g.1)).collect()
}

const FLAG: &str = "\
Let verbose be parseInt(item 2 of args()) equals 1.
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 3000:
    Set acc to acc + i * 3.
    If verbose:
        Show i.
    Set i to i + 1.
Show acc.
";

const MODE: &str = "\
Let mode be parseInt(item 2 of args()).
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 3000:
    If mode equals 2:
        Show i.
    Set acc to acc + i.
    Set i to i + 1.
Show acc.
";

/// Both sides of the branch present: the optimizer unswitches the loop on the invariant before the JIT sees it, so the
/// JIT compiles the quiet copy as it stands.
const MODE_BOTH_SIDES: &str = "\
Let mode be parseInt(item 2 of args()).
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 3000:
    If mode equals 2:
        Show i.
    Otherwise:
        Set acc to acc + i.
    Set i to i + 1.
Show acc.
";

const THRESHOLD: &str = "\
Let level be parseInt(item 2 of args()).
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 3000:
    If level is greater than 5:
        Show i.
    Set acc to acc + i * level.
    Set i to i + 1.
Show acc.
";

const FLIPS_ON_THE_LAST_PASS: &str = "\
Let last be parseInt(item 2 of args()).
Let mutable total be 0.
Let mutable pass be 0.
While pass is at most last:
    Let loud be pass equals last.
    Let mutable i be 0.
    While i is less than 300:
        Set total to total + i.
        If loud:
            Show i.
        Set i to i + 1.
    Set pass to pass + 1.
Show total.
";

const WRITTEN_IN_THE_LOOP: &str = "\
Let mutable verbose be parseInt(item 2 of args()) equals 1.
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 3000:
    Set acc to acc + i.
    If verbose:
        Show i.
    Set verbose to i equals 2999.
    Set i to i + 1.
Show acc.
";

#[test]
fn a_flag_that_is_off_drops_its_branch_and_the_loop_runs_native() {
    let traced = run(FLAG, "0");
    assert!(traced.region_successes >= 1, "the loop runs native once its Show is dropped: {traced:?}");
    assert_eq!(specialized_on(&traced), vec![ScalarValue::Bool(false)], "{traced:?}");
}

#[test]
fn a_flag_that_is_on_keeps_its_branch() {
    let traced = run(FLAG, "1");
    assert!(
        !specialized_on(&traced).contains(&ScalarValue::Bool(false)),
        "the branch is taken, so it is not dropped: {traced:?}"
    );
}

#[test]
fn an_int_compared_with_a_constant_decides_its_branch() {
    let traced = run(MODE, "1");
    assert!(traced.region_successes >= 1, "mode 1 never shows, so the loop runs native: {traced:?}");
    assert!(specialized_on(&traced).contains(&ScalarValue::Int(1)), "specialized on mode = 1: {traced:?}");
    run(MODE, "2");
}

#[test]
fn a_branch_with_both_sides_on_an_invariant_runs_native_either_way() {
    let traced = run(MODE_BOTH_SIDES, "1");
    assert!(traced.region_successes >= 1, "the quiet copy of the unswitched loop runs native: {traced:?}");
    run(MODE_BOTH_SIDES, "2");
}

#[test]
fn an_ordering_against_a_constant_decides_its_branch() {
    let traced = run(THRESHOLD, "3");
    assert!(traced.region_successes >= 1, "level 3 never shows, so the loop runs native: {traced:?}");
    assert!(specialized_on(&traced).contains(&ScalarValue::Int(3)), "specialized on level = 3: {traced:?}");
    run(THRESHOLD, "9");
}

#[test]
fn a_flag_that_turns_on_in_a_later_entry_runs_its_branch_on_bytecode() {
    let traced = run(FLIPS_ON_THE_LAST_PASS, "10");
    let [spec] = traced.value_specializations.as_slice() else {
        panic!("the inner loop was specialized once: {traced:?}");
    };
    assert_eq!(spec.guards.iter().map(|g| g.1).collect::<Vec<_>>(), vec![ScalarValue::Bool(false)]);
    assert!(spec.entries >= 10, "every quiet pass ran the specialized loop: {spec:?}");
    assert!(spec.misses >= 1, "the loud pass found the flag on: {spec:?}");
}

const LOUD_NOW_AND_THEN: &str = "\
Let every be parseInt(item 2 of args()).
Let mutable total be 0.
Let mutable pass be 0.
While pass is less than 40:
    Let loud be pass % every equals every - 1.
    Let mutable i be 0.
    While i is less than 300:
        Set total to total + i.
        If loud:
            Show i.
        Set i to i + 1.
    Set pass to pass + 1.
Show total.
";

#[test]
fn a_flag_on_now_and_then_keeps_the_quiet_passes_native() {
    // Every tenth pass is loud, and its loop cannot run native: the Show it reaches is not lowered. Each loud pass is one
    // miss for its whole entry, not one per iteration, so the specialized loop is never given up and every quiet pass
    // runs it.
    let traced = run(LOUD_NOW_AND_THEN, "10");
    let [spec] = traced.value_specializations.as_slice() else {
        panic!("the inner loop was specialized once: {traced:?}");
    };
    assert_eq!(spec.misses, 4, "one miss for each loud pass: {spec:?}");
    assert_eq!(spec.generic_entries, 0, "a loud pass reaches the Show, so no generic region can run it: {spec:?}");
    assert!(spec.entries >= 35, "the quiet passes ran the specialized loop: {spec:?}");
    assert!(!spec.replaced, "a rare loud pass does not cost the quiet ones their native loop: {spec:?}");
}

#[test]
fn a_flag_written_in_the_loop_is_not_specialized() {
    for argument in ["0", "1"] {
        let traced = run(WRITTEN_IN_THE_LOOP, argument);
        assert!(traced.value_specializations.is_empty(), "the loop writes its flag: {traced:?}");
    }
}
