//! Invariants for `eval_wprog_config_faulted` — the fault-model diagnostic that runs the whole-program
//! interpreter under an injected hardware fault so a synthesis divergence can be characterized in pure Rust.
//! Two properties make it trustworthy, and this lock holds both:
//!
//!   1. Anti-drift: with an empty fault set it is byte-identical to the golden `eval_wprog_config`, across
//!      arithmetic / branching / looping / recursion / heap-collection programs over a range of inputs. If the
//!      faulted interpreter ever diverges from ground truth, every result it reports is meaningless.
//!   2. Live injection: a fault that folds every access of a state memory onto a tiny window changes the result
//!      of a program that uses that memory — so the hook is proven wired in, not silently modelling nothing.

use logicaffeine_compile::codegen_sva::fabric_rtl::program_cfg_fabric;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config, eval_wprog_config_faulted, serialize_program, WFault,
};

/// Small programs exercising every terminator + cell kind the localizer must model faithfully.
fn corpus() -> Vec<(&'static str, &'static str, Vec<i64>)> {
    vec![
        // Straight-line arithmetic.
        ("arith", "## To f (x: Int) -> Int:\n    Return x * x + 7.\n", vec![-3, 0, 5, 41]),
        // Branch.
        (
            "branch",
            "## To f (x: Int) -> Int:\n    If x is greater than 10:\n        Return x - 10.\n    Return x + 100.\n",
            vec![0, 5, 11, 50],
        ),
        // Loop (block walk + register file across reconfiguration).
        (
            "loop",
            "## To f (n: Int) -> Int:\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is less than or equal to n:\n        Set acc to acc + i.\n        Set i to i + 1.\n    Return acc.\n",
            vec![0, 1, 6, 20],
        ),
        // Recursion (the frame stack).
        (
            "recursion",
            "## To f (n: Int) -> Int:\n    If n is less than 1:\n        Return 0.\n    Return n + f(n - 1).\n",
            vec![0, 3, 8, 15],
        ),
        // Heap collection (the state plane).
        (
            "heap",
            "## To f (x: Int) -> Int:\n    Let mutable xs be [10, x, 30].\n    Set item 1 of xs to 99.\n    Return item 1 of xs + item 2 of xs.\n",
            vec![-5, 0, 7, 22],
        ),
        // Division / functional unit.
        (
            "division",
            "## To f (x: Int) -> Int:\n    Return (x * 7) / 3.\n",
            vec![0, 4, 9, 100],
        ),
    ]
}

#[test]
fn faulted_with_no_fault_is_byte_identical_to_the_golden_vm() {
    for (name, src, inputs) in corpus() {
        let cfg = serialize_program(&program_cfg_fabric(src, "f"));
        for &x in &inputs {
            let golden = eval_wprog_config(&cfg, &[x]);
            let faulted = eval_wprog_config_faulted(&cfg, &[x], &[], &[]).0;
            assert_eq!(
                golden, faulted,
                "ANTI-DRIFT: the fault localizer's no-fault path diverged from the golden VM on `{name}` @ x={x} \
                 (golden={golden}, faulted={faulted}) — the diagnostic is no longer trustworthy",
            );
        }
    }
}

#[test]
fn a_fault_actually_perturbs_a_program_that_uses_that_memory() {
    // Folding every heap access onto a 1-word window (`HeapDecodeBits(0)` -> addr & 0 = 0) must change the
    // result of a program that reads/writes distinct heap slots.
    let heap_src = "## To f (x: Int) -> Int:\n    Let mutable xs be [10, x, 30].\n    Set item 1 of xs to 99.\n    Return item 1 of xs + item 2 of xs.\n";
    let cfg = serialize_program(&program_cfg_fabric(heap_src, "f"));
    let golden = eval_wprog_config(&cfg, &[7]);
    let faulted = eval_wprog_config_faulted(&cfg, &[7], &[], &[WFault::HeapDecodeBits(0)]).0;
    assert_ne!(
        golden, faulted,
        "INJECTION DEAD: HeapDecodeBits(0) failed to perturb a heap-using program (golden={golden}) — the fault \
         hook is not wired into heap addressing, so the localizer models nothing",
    );

    // Cell-value plane: folding earlier-cell references onto a 1-entry window MUST perturb a multi-cell block.
    let arith_src = "## To f (x: Int) -> Int:\n    Return x * x + x - 3.\n";
    let cfg2 = serialize_program(&program_cfg_fabric(arith_src, "f"));
    let g2 = eval_wprog_config(&cfg2, &[9]);
    let f2 = eval_wprog_config_faulted(&cfg2, &[9], &[], &[WFault::VDecodeBits(0)]).0;
    assert_ne!(
        g2, f2,
        "INJECTION DEAD: VDecodeBits(0) failed to perturb a multi-cell arithmetic block (golden={g2}) — the \
         cell-value fault hook is not wired in",
    );
}

#[test]
fn compound_faults_compose_both_decodes() {
    // A two-memory fault set applies both decodes (fold semantics), not just the first.
    let src = "## To f (x: Int) -> Int:\n    Let mutable xs be [10, x, 30].\n    Set item 1 of xs to 99.\n    Return (item 1 of xs) * (item 2 of xs) + x - 1.\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let both = eval_wprog_config_faulted(
        &cfg,
        &[7],
        &[],
        &[WFault::HeapDecodeBits(0), WFault::VDecodeBits(0)],
    )
    .0;
    let heap_only = eval_wprog_config_faulted(&cfg, &[7], &[], &[WFault::HeapDecodeBits(0)]).0;
    // Composing a second independent decode must be able to change the outcome vs one alone.
    assert_ne!(
        both, heap_only,
        "compound fault set did not compose the second decode (both={both}, heap_only={heap_only})",
    );
}
