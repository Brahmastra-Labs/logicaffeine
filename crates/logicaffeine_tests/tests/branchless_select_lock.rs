//! `Opt::BranchlessSelect`: `If c: Set x to a. Otherwise: Set x to b.` as a select between both arms.
//!
//! When both arms are pure arithmetic that cannot trap, both are computed and `c` picks one: the compiled loop
//! carries a conditional move where it carried a branch on `c`, and a branch on a data-dependent `c` (collatz's
//! parity) mispredicts. An arm whose exactness rests on a guard (`3 * k + 1` exact while `k` stays inside its leaf
//! bound) keeps the guard, tested on its own, before either arm: when every guard holds, the select runs; when one
//! fails, the original branch runs with its exact arithmetic. Folding the guard into `c` gave the branch back to
//! the compiler, and collatz ran slower than before; standing alone it ran 1.26x faster, level with rustc. An arm
//! that can trap (a division by a variable, an item read) is never computed on a pass that would not compute it.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{build_c_release, compile_logos, compile_to_rust, run_all_tiers};
use logicaffeine_compile::compile::{
    compile_to_c, run_strict_tw_with_args, run_strict_vm_with_args,
};
use logicaffeine_compile::optimization::decorate_source;

/// The marker of a selected `If`: the two arm values bound before the select.
const SELECT: &str = "__select_then";

/// One collatz step on the argument.
const STEP: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let mutable k be parseInt(item 2 of arguments).
If k % 2 equals 0:
    Set k to k / 2.
Otherwise:
    Set k to 3 * k + 1.
Show k.
"#;

#[test]
fn a_two_armed_assignment_compiles_to_a_select() {
    let rust = compile_to_rust(STEP).unwrap();
    assert!(
        rust.contains(SELECT),
        "the parity step selects between both arms. Got:\n{rust}"
    );
    assert!(
        rust.contains("logos_mul_exact"),
        "the exact arithmetic stays for a value past the arm's guard. Got:\n{rust}"
    );
}

#[test]
fn turning_the_optimization_off_keeps_the_branch() {
    let rust = compile_to_rust(&decorate_source(STEP, &["branchlessselect"])).unwrap();
    assert!(!rust.contains(SELECT), "Got:\n{rust}");
}

#[test]
fn every_tier_takes_the_same_step_at_every_value() {
    let aot = compile_logos(STEP);
    assert!(aot.success, "AOT build failed:\n{}", aot.stderr);
    let dir = tempfile::tempdir().unwrap();
    let (c_src, c_bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&c_src, compile_to_c(STEP).unwrap()).unwrap();
    let built = build_c_release(&c_src, &c_bin).unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    for (n, want) in [
        ("0", "0"),
        ("7", "22"),
        ("8", "4"),
        ("-7", "-20"),
        ("-8", "-4"),
        // Past the leaf bound 2^61 that keeps `3 * k + 1` inside a machine word, still odd: the guard fails and
        // the exact branch computes a value that fits.
        ("2305843009213693953", "6917529027641081860"),
        ("2305843009213693954", "1152921504606846977"),
    ] {
        let argv = vec!["prog".to_string(), n.to_string()];
        let tw = run_strict_tw_with_args(STEP, &argv);
        let vm = run_strict_vm_with_args(STEP, &argv);
        assert_eq!(
            (tw.output.trim(), tw.error.as_deref()),
            (want, None),
            "tree-walker, k = {n}"
        );
        assert_eq!(
            (vm.output.trim(), vm.error.as_deref()),
            (want, None),
            "vm, k = {n}"
        );
        for (tier, bin) in [
            ("aot-rust", aot.binary_path.as_path()),
            ("c", c_bin.as_path()),
        ] {
            let out = std::process::Command::new(bin).arg(n).output().unwrap();
            assert!(
                out.status.success(),
                "{tier}, k = {n}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&out.stdout).trim(),
                want,
                "{tier}, k = {n}"
            );
        }
    }
}

#[test]
fn an_arm_that_can_divide_by_zero_is_not_computed_ahead() {
    let source = "## Main
Let mutable total be 0.
Let mutable i be 0.
While i is less than 100:
    Let d be i % 7.
    Let mutable q be 0.
    If d equals 0:
        Set q to 0.
    Otherwise:
        Set q to 1000 / d.
    Set total to total + q.
    Set i to i + 1.
Show total.
";
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains(SELECT),
        "`1000 / d` divides by zero on the pass that takes the other arm. Got:\n{rust}"
    );
    let agreed = run_all_tiers(source);
    assert!(agreed.error.is_none(), "{:?}", agreed.error);
    common::assert_c_output(source, agreed.output.trim());
    common::assert_compiled_equals_interpreted(source);
}

#[test]
fn an_arm_that_reads_an_item_is_not_computed_ahead() {
    let source = "## Main
Let xs be [10, 20, 30].
Let mutable total be 0.
Let mutable i be 0.
While i is less than 6:
    Let mutable v be 0.
    If i is at most 3 and i is at least 1:
        Set v to item i of xs.
    Otherwise:
        Set v to 0 - i.
    Set total to total + v.
    Set i to i + 1.
Show total.
";
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains(SELECT),
        "`item i of xs` reads past the list on the passes the guard excludes. Got:\n{rust}"
    );
    let agreed = run_all_tiers(source);
    assert_eq!(
        (agreed.output.trim(), agreed.error.as_deref()),
        ("51", None)
    );
    common::assert_c_output(source, "51");
    common::assert_compiled_equals_interpreted(source);
}

#[test]
fn a_branch_that_does_more_than_one_assignment_keeps_its_branch() {
    let source = "## Main
Let mutable a be 0.
Let mutable b be 0.
Let mutable i be 0.
While i is less than 10:
    If i % 2 equals 0:
        Set a to a + i.
    Otherwise:
        Set b to b + i.
    Set i to i + 1.
Show a * 100 + b.
";
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains(SELECT),
        "the arms assign different names. Got:\n{rust}"
    );
    let agreed = run_all_tiers(source);
    assert_eq!(
        (agreed.output.trim(), agreed.error.as_deref()),
        ("2025", None)
    );
}

#[test]
fn the_collatz_benchmark_selects_its_step() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/programs/collatz/main.lg");
    let rust = compile_to_rust(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(
        rust.contains(SELECT),
        "collatz's parity step is a select. Got:\n{rust}"
    );
}

/// Two arms over the same leaves (`checksum + flips`, `checksum - flips`: fannkuch's alternating checksum) need the
/// same guard, so it is tested once rather than twice joined.
#[test]
fn arms_over_the_same_leaves_test_their_guard_once() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable checksum be parseInt("40").
Let flips be parseInt("7").
Let permCount be parseInt("3").
If permCount % 2 equals 0:
    Set checksum to checksum + flips.
Otherwise:
    Set checksum to checksum - flips.
Show checksum.
"#;
    let rust = compile_to_rust(source).unwrap();
    let guard_line = rust
        .lines()
        .find(|l| {
            l.trim_start().starts_with("if ") && rust.contains(SELECT) && l.contains("checksum")
        })
        .unwrap_or_else(|| panic!("the select keeps a guard over its leaves. Got:\n{rust}"));
    assert!(
        guard_line.matches("checksum <=").count() == 1
            && guard_line.matches("flips <=").count() == 1,
        "each leaf tested once, not the same guard joined to itself: {guard_line}"
    );
    let agreed = run_all_tiers(source);
    assert!(agreed.error.is_none(), "{:?}", agreed.error);
    assert_eq!(agreed.output.trim(), "33");
    common::assert_compiled_equals_interpreted(source);
}
