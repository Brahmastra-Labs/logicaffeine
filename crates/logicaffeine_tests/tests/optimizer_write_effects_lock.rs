//! Every optimizer pass must agree on what a statement MAY write.
//!
//! The run-path lattice had four private answers to "which names does this block assign or mutate?" (CSE,
//! LICM and its clients, deforestation, SCCP), each with its own blind spots: a loop body was CSE'd with the
//! pre-loop availability set, an `Inspect` arm's `Set` was invisible to CSE, a `Swap` was invisible to LICM,
//! and a call to a function that `Set`s a global was a write in the propagator but not in CSE or LICM. Each
//! blind spot is a silent wrong answer. These are the minimal programs, each asserted against the LITERAL
//! expected output on the tree-walker (the oracle), the raw VM, the AOT-lattice-optimized VM
//! (`compile_to_program_optimized`, what `largo build` optimizes), and the CLI `largo run --interpret` path
//! (the run-path lattice) — the two optimizer pipelines are different pass lists and both must be sound.

#[path = "common/thread_cpu.rs"]
mod thread_cpu;

use logicaffeine_compile::compile::{
    compile_to_program_optimized, program_outcome, tw_outcome, vm_outcome,
};

fn assert_prints(label: &str, src: &str, expected: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "{label}: tree-walker must run cleanly");
    assert_eq!(tw.output.trim(), expected, "{label}: tree-walker output");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "{label}: VM must run cleanly");
    assert_eq!(
        vm.output.trim(),
        expected,
        "{label}: VM output (raw compile)"
    );
    let program = compile_to_program_optimized(src).unwrap_or_else(|e| {
        panic!("{label}: the optimizer pipeline must compile the program: {e:?}")
    });
    let opt = program_outcome(&program);
    assert_eq!(opt.error, None, "{label}: optimized VM must run cleanly");
    assert_eq!(
        opt.output.trim(),
        expected,
        "{label}: optimized VM output (the AOT lattice on the VM)"
    );
    let cli = logicaffeine_compile::ui_bridge::interpret_for_ui_sync_with_args(src, &[]);
    assert_eq!(
        cli.error, None,
        "{label}: the CLI `--interpret` path must run cleanly"
    );
    assert_eq!(
        cli.lines.join("\n").trim(),
        expected,
        "{label}: CLI `--interpret` output (the run-path lattice)"
    );
}

/// A function that `Set`s a global is a write at every call site: the read after the call recomputes.
#[test]
fn a_call_to_a_global_mutating_function_invalidates_a_cached_read_of_that_global() {
    let src = "## To bump:\n    Set counter to counter + 1.\n## Main\n    Let mutable counter be 1.\n    Let a be counter * 100.\n    bump().\n    Let b be counter * 100.\n    Show a.\n    Show b.\n";
    assert_prints("call between reads", src, "100\n200");
}

/// The call sits inside a `Let` value rather than a statement of its own.
#[test]
fn a_global_mutating_call_inside_a_let_value_invalidates_a_cached_read_of_that_global() {
    let src = "## To bumpAndGet -> Int:\n    Set counter to counter + 1.\n    Return counter.\n## Main\n    Let mutable counter be 1.\n    Let a be counter * 100.\n    Let c be bumpAndGet().\n    Let b be counter * 100.\n    Show a.\n    Show c.\n    Show b.\n";
    assert_prints("call inside a Let", src, "100\n2\n200");
}

/// The mutation reaches the global through a second function (transitive), and the reads are in a loop.
#[test]
fn a_transitively_global_mutating_call_in_a_loop_keeps_every_read_fresh() {
    let src = "## To inner:\n    Set counter to counter + 1.\n## To outer:\n    inner().\n## Main\n    Let mutable counter be 0.\n    Let mutable i be 0.\n    While i is less than 3:\n        Let c be counter * 10.\n        Show c.\n        outer().\n        Set i to i + 1.\n";
    assert_prints("transitive call in loop", src, "0\n10\n20");
}

/// LICM: a `Let` reading `x` is NOT loop-invariant when the body swaps `x` — `Swap` writes both names.
#[test]
fn a_swap_in_the_loop_body_keeps_a_read_of_either_name_inside_the_loop() {
    let src = "## Main\n    Let mutable x be 3.\n    Let mutable y be 5.\n    Repeat for i from 1 to 3:\n        Let s be x * 10.\n        Show s.\n        Swap x and y.\n";
    assert_prints("swap in loop", src, "30\n50\n30");
}

/// LICM: a `Let` reading a global is NOT loop-invariant when the body calls a function that `Set`s it.
#[test]
fn a_read_of_a_callee_mutated_global_is_not_hoisted_out_of_the_calling_loop() {
    let src = "## To bump:\n    Set counter to counter + 1.\n## Main\n    Let mutable counter be 0.\n    Repeat for i from 1 to 3:\n        Let c be counter * 10.\n        Show c.\n        bump().\n";
    assert_prints("global read across call in loop", src, "0\n10\n20");
}

/// A closure that `Set`s a captured outer variable: every tier agrees on what that means. Today all
/// three leave the outer variable untouched (the closure works on its own copy) — a language-design
/// question for the cross-tier semantics wave, pinned here only as agreement, not as the answer.
#[test]
fn every_tier_agrees_on_a_closure_assigning_a_captured_variable() {
    let src = "## Main\n    Let mutable counter be 1.\n    Let bump be (e: Int) ->:\n        Set counter to counter + e.\n    Let a be counter * 100.\n    bump(1).\n    Let b be counter * 100.\n    Show a.\n    Show b.\n";
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker must run cleanly");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "VM must run cleanly");
    assert_eq!(
        vm.output.trim(),
        tw.output.trim(),
        "raw VM must agree with the tree-walker"
    );
    let program =
        compile_to_program_optimized(src).expect("the optimizer pipeline must compile the program");
    let opt = program_outcome(&program);
    assert_eq!(opt.error, None, "optimized VM must run cleanly");
    assert_eq!(
        opt.output.trim(),
        tw.output.trim(),
        "optimized VM must agree with the tree-walker"
    );
}

/// The compile stays LINEAR in the recursive inliner's unfold depth. `abstract_interp`'s loop walker
/// used to run a full fixpoint (two ascent passes, four narrowing probes, a recording walk) at every
/// nesting LEVEL, so a nest N deep cost ~4^N body walks — and the inliner builds exactly that nest by
/// unfolding a self-call inside a loop. n-queens measured 0.15 / 0.32 / 1.33 / 5.9 / 25.7 seconds at
/// depths 3 to 7, and codegen's Oracle was 99% of it (6.03 s of a 6.06 s compile at depth 6). Bounded
/// nesting made it 0.15 / 0.22 / 0.40 / 0.61 / 0.86. This pins the SHAPE rather than any wall-clock
/// number: each extra level may cost meaningfully more than the last, but never multiply.
#[test]
fn the_compile_stays_linear_in_the_unfold_depth() {
    use logicaffeine_compile::compile::compile_to_rust;
    let body = include_str!("../../../benchmarks/programs/nqueens/main.lg");
    let mut timings: Vec<(u32, std::time::Duration)> = Vec::new();
    for depth in [4u32, 5, 6, 7] {
        std::env::set_var("LOGOS_RECURSE_DEPTH", depth.to_string());
        let t = thread_cpu::thread_cpu_time();
        compile_to_rust(body).expect("n-queens compiles at every unfold depth");
        timings.push((depth, thread_cpu::thread_cpu_time() - t));
    }
    std::env::remove_var("LOGOS_RECURSE_DEPTH");
    // Exponential growth shows up as a ratio that stays large as the depth climbs; the pre-fix curve
    // ran 4.2x, 4.4x, 4.4x per level. A generous ceiling still separates the two regimes by a mile.
    for pair in timings.windows(2) {
        let (d0, t0) = pair[0];
        let (d1, t1) = pair[1];
        let ratio = t1.as_secs_f64() / t0.as_secs_f64().max(0.000_001);
        assert!(
            ratio < 3.0,
            "compile CPU time multiplied {ratio:.1}x from unfold depth {d0} to {d1} ({t0:?} -> {t1:?}) — \
             the loop walker is compounding per nesting level again, not merely growing"
        );
    }
}
