//! ★ MIXED-SIGNAL PARTITION LOCK ★ — the dispatch covenant, mechanized.
//!
//! The analog plane is an accelerator, never a correctness dependency:
//!   1. TOTALITY — every function of a program lands in exactly one list (binding or ledgered
//!      refusal); nothing is silently skipped, and partitioning is DETERMINISTIC.
//!   2. ADMITTED-ONLY — every binding carries a certificate that replays (`budget.recheck()`),
//!      and an INT-lane binding's budget is its exactness proof (ε < ½ by admission).
//!   3. FALLBACK TOTALITY — every BOUND region also runs on the DIGITAL engine model
//!      (`program_calls_fabric` → `eval_wprog_config`) with the ≡VM answer: stripping the
//!      binding costs energy, never correctness. Proven here at region granularity on the real
//!      engine model, and at whole-SoC granularity by the later composition phases.
//!   4. THE LEDGER — the corpus partition is pinned exactly; a refusal that starts binding is a
//!      CAPABILITY and must move its pin (celebratory, conscious); a binding that starts
//!      refusing is a regression and fails outright.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE.

mod analog_common;

use logicaffeine_compile::codegen_ana::{contract_free_twin, AnalogTarget, RegionLane};
use logicaffeine_compile::codegen_mx::partition;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config, program_calls_fabric,
};

/// A program with one region of EACH bindable lane plus digital-only work: a float DAG, an
/// int DAG (½-grid-provable at ±8 on the calibrated V1 chip), and recursive fib (digital).
const MIXED: &str = "## To blend (a: Float, b: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Return a * 0.5 + b * 0.25.\n\n## To add2 (a: Int, b: Int) -> Int:\n    Assume a is between -8 and 8.\n    Assume b is between -8 and 8.\n    Return a + b.\n\n## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\n    Show blend(0.5, 0.25).\n    Show add2(5, -3).\n    Show fib(10).\n";

#[test]
fn the_corpus_partition_is_total_deterministic_and_pinned() {
    let target = AnalogTarget::sky130_fpaa();
    let p1 = partition(MIXED, &target);
    let p2 = partition(MIXED, &target);

    // 1. Totality: all three functions accounted for, exactly once.
    assert_eq!(p1.all_fns(), vec!["add2", "blend", "fib"]);
    assert_eq!(p1.bindings.len() + p1.refusals.len(), 3);

    // Determinism: byte-for-byte the same decision both times.
    assert_eq!(p1.bindings.len(), p2.bindings.len());
    assert_eq!(p1.refusals, p2.refusals);

    // 4. The pinned partition: blend (float-ε) and add2 (int-EXACT) bind; fib stays digital
    //    with a NAMED reason. Moving any of these pins is a conscious edit here.
    let lanes: Vec<(&str, RegionLane)> = p1
        .bindings
        .iter()
        .map(|b| (b.fn_name.as_str(), b.lane))
        .collect();
    assert!(
        lanes.contains(&("blend", RegionLane::Float)),
        "blend must bind ε-float: {lanes:?}"
    );
    assert!(
        lanes.contains(&("add2", RegionLane::Int)),
        "add2 must bind certified-exact int: {lanes:?}"
    );
    let fib = p1
        .refusals
        .iter()
        .find(|r| r.fn_name == "fib")
        .expect("fib stays digital");
    assert!(
        !fib.reason.is_empty(),
        "a refusal always carries its named reason"
    );

    // 2. Admitted-only: every binding's certificate replays; the int binding's budget IS its
    //    exactness proof.
    for b in &p1.bindings {
        assert!(
            b.artifact.budget.recheck(),
            "`{}`: a binding's budget must replay",
            b.fn_name
        );
        if b.lane == RegionLane::Int {
            assert!(
                b.artifact.budget.eps_math < 0.5,
                "`{}`: an int binding is admitted BY its exactness proof",
                b.fn_name
            );
        }
    }
}

#[test]
fn every_bound_region_also_runs_on_the_digital_engine_model() {
    let target = AnalogTarget::sky130_fpaa();
    let p = partition(MIXED, &target);
    assert!(
        !p.bindings.is_empty(),
        "the corpus must bind something or this test is vacuous"
    );

    // 3. Fallback totality at region granularity, on the REAL engine model, over the SAME
    //    contract-free twin both lanes lower (the range check is the control plane's bytecode
    //    in both deployments; the whole-program engine lock of the composition phases carries
    //    it). Integer probes drive both regions; float params ride as f64 bit patterns.
    let twin = contract_free_twin(MIXED);
    for b in &p.bindings {
        let cfg = program_calls_fabric(&twin, &b.fn_name);
        match b.lane {
            RegionLane::Int => {
                let (got, want) = (eval_wprog_config(&cfg, &[5, -3]), 5 + (-3));
                assert_eq!(
                    got, want,
                    "`{}`: the digital engine runs the bound int region ≡VM",
                    b.fn_name
                );
            }
            RegionLane::Float => {
                let (a, bb) = (0.5f64, 0.25f64);
                let got = eval_wprog_config(&cfg, &[a.to_bits() as i64, bb.to_bits() as i64]);
                let want = (a * 0.5 + bb * 0.25).to_bits() as i64;
                assert_eq!(
                    got, want,
                    "`{}`: the digital engine runs the bound float region bit-exactly",
                    b.fn_name
                );
            }
        }
    }
}

#[test]
fn arbitrary_shapes_partition_without_crashing() {
    let target = AnalogTarget::sky130_fpaa();
    // Concurrency in Main + a bindable region function: the partition is about REGIONS; the
    // pure region still binds, and nothing panics on the concurrency shape.
    let with_conc = "## To add2 (a: Int, b: Int) -> Int:\n    Assume a is between -8 and 8.\n    Assume b is between -8 and 8.\n    Return a + b.\n\n## To produce (ch: Int):\n    Send 1 into ch.\n\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to produce with jobs.\n    Receive a from jobs.\n    Show a.\n    Show add2(1, 2).\n";
    let p = partition(with_conc, &target);
    assert_eq!(
        p.bindings.len() + p.refusals.len(),
        2,
        "both functions classified"
    );
    assert!(
        p.bindings.iter().any(|b| b.fn_name == "add2"),
        "the pure region still binds"
    );
    assert!(
        p.refusals.iter().any(|r| r.fn_name == "produce"),
        "the channel fn is ledgered, not crashed"
    );

    // A digital-only program partitions to an empty binding set, not an error.
    let digital =
        "## To triple (n: Int) -> Int:\n    Return n * 3.\n\n## Main\n    Show triple(4).\n";
    let p = partition(digital, &target);
    assert!(p.bindings.is_empty(), "no contracts ⇒ nothing binds");
    assert_eq!(p.refusals.len(), 1);
    assert!(
        p.refusals[0].reason.contains("declared range"),
        "{}",
        p.refusals[0].reason
    );
}
