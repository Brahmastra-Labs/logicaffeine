//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ASSUME RANGE LOCK — `Assume x is between A and B.` is a REAL, ENFORCED input contract on every
//! executing tier, and a STATICALLY RECOVERABLE range declaration for the analog lane.
//!
//! The statement is deliberately a NEW HEAD over the PROVEN body: it parses to the same
//! `RuntimeAssert { hard: true }` that `Require that <cond>.` produces, with the condition built from
//! the existing `is between A and B` desugar (`A <= x and x <= B`). Zero new AST variants, zero new
//! VM ops — every tier inherits enforcement through machinery that already carries it (TW arm, VM
//! `Op::FailWith`, AOT-Rust `assert!`, the C arm this phase adds, WASM via the VM bytecode).
//!
//! What this file locks:
//!   1. In range, `Assume` is inert and byte-identical to the `Require that` spelling (TW = VM = C =
//!      AOT-Rust, all against the tree-walker oracle).
//!   2. A violation FAILS on every executing tier — an assumption is a contract, not a comment. This
//!      also locks closed the old C-tier gap where `RuntimeAssert` was silently dropped as
//!      `/* unsupported stmt */` (a hard `Require that` did not enforce in C).
//!   3. The declared ranges are statically recoverable (`codegen_ana::assumed_ranges`) — the analog
//!      lane's gain-scaling input, with negative bounds exact.
//!   4. The analog refusal gate consumes them: float dataflow WITH declared ranges clears
//!      `UnboundedInput`; without, it still refuses.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE.
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::codegen_ana::assumed_ranges;
use logicaffeine_compile::compile::{
    compile_to_analog, compile_to_c, compile_to_wasm, tw_outcome, vm_outcome,
};

// ============================================================
//  Corpus
// ============================================================

const IN_RANGE: &str =
    "## Main\n    Let x be 0.5.\n    Assume x is between -1.0 and 1.0.\n    Show x * 2.0.\n";

/// The identical program with the established `Require that` spelling — `Assume` must mean exactly this.
const IN_RANGE_REQUIRE: &str =
    "## Main\n    Let x be 0.5.\n    Require that x is between -1.0 and 1.0.\n    Show x * 2.0.\n";

const VIOLATION: &str =
    "## Main\n    Let x be 2.5.\n    Assume x is between -1.0 and 1.0.\n    Show x.\n";

/// A hard `Require that` violation — pins the C tier enforcing the EXISTING surface too (the arm
/// this phase adds closes the silent-drop gap for both spellings at once).
const REQUIRE_VIOLATION: &str =
    "## Main\n    Let n be 7.\n    Require that n is between 1 and 5.\n    Show n.\n";

/// Function-shaped: the analog region unit. The range names the float parameter.
const FN_SHAPED: &str = "## To scale (x: Float) -> Float:\n    Assume x is between -1.0 and 1.0.\n    Return x * 0.5.\n\n## Main\n    Show scale(0.5).\n";

/// Float dataflow with NO declared ranges — the analog gate must still refuse `UnboundedInput`.
const FN_NO_RANGES: &str =
    "## To scale (x: Float) -> Float:\n    Return x * 0.5.\n\n## Main\n    Show scale(0.5).\n";

// ============================================================
//  Helpers
// ============================================================

/// Compile through the AOT-C backend and gcc, run, and report `(ran_ok, stdout)`.
/// `None` = did not reach execution (compile failure) — always an assertion failure here.
fn c_run(program: &str) -> Option<(bool, String)> {
    let c = compile_to_c(program).ok()?;
    let dir =
        std::env::temp_dir().join(format!("logos_assume_{}_{:x}", std::process::id(), c.len()));
    std::fs::create_dir_all(&dir).ok()?;
    let src = dir.join("out.c");
    let bin = dir.join("out");
    std::fs::write(&src, &c).ok()?;
    let compiled = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&src)
        .arg("-lm")
        .output()
        .ok()?;
    if !compiled.status.success() {
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    }
    let run = std::process::Command::new(&bin).output().ok()?;
    let out = (
        run.status.success(),
        String::from_utf8_lossy(&run.stdout).trim().to_string(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    Some(out)
}

// ============================================================
//  1. In range: inert, and exactly the `Require that` meaning
// ============================================================

#[test]
fn assume_is_inert_in_range_and_means_require_that() {
    let oracle = tw_outcome(IN_RANGE);
    assert_eq!(
        oracle.error, None,
        "in-range Assume must not fail on the tree-walker"
    );
    assert!(!oracle.output.trim().is_empty(), "probe must Show a result");

    let require = tw_outcome(IN_RANGE_REQUIRE);
    assert_eq!(require.error, None);
    assert_eq!(
        oracle.output, require.output,
        "`Assume` must mean exactly `Require that`"
    );

    let vm = vm_outcome(IN_RANGE);
    assert_eq!(vm.error, None, "in-range Assume must not fail on the VM");
    assert_eq!(
        vm.output.trim(),
        oracle.output.trim(),
        "VM must match the oracle byte-for-byte"
    );
}

// ============================================================
//  2. Violation: fails on every executing tier
// ============================================================

#[test]
fn assume_violation_fails_on_tw_and_vm() {
    let tw = tw_outcome(VIOLATION);
    let tw_err = tw
        .error
        .expect("violated Assume must fail on the tree-walker");
    assert!(
        tw_err.contains("Assertion failed"),
        "TW failure must be the assertion failure: {tw_err}"
    );

    let vm = vm_outcome(VIOLATION);
    let vm_err = vm.error.expect("violated Assume must fail on the VM");
    assert!(
        vm_err.contains("Assertion failed"),
        "VM failure must be the assertion failure: {vm_err}"
    );
}

#[test]
fn assume_enforces_on_the_c_tier() {
    if !common::require_tool("gcc") {
        return;
    }
    let oracle = tw_outcome(IN_RANGE);
    let (ok, out) = c_run(IN_RANGE).expect("in-range program must compile through the C tier");
    assert!(ok, "in-range Assume must not fail at C runtime");
    assert_eq!(out, oracle.output.trim(), "C output must match the oracle");

    let (ok, _) =
        c_run(VIOLATION).expect("violation program must still COMPILE through the C tier");
    assert!(
        !ok,
        "violated Assume must fail at C runtime — a dropped contract is a silent miscompile"
    );

    let (ok, _) =
        c_run(REQUIRE_VIOLATION).expect("Require-that program must compile through the C tier");
    assert!(!ok, "violated hard `Require that` must fail at C runtime (the old silent-drop gap stays closed)");
}

#[test]
fn assume_executes_on_aot_rust() {
    let oracle = tw_outcome(IN_RANGE);
    let r = common::run_logos(IN_RANGE);
    assert!(
        r.success,
        "in-range Assume must run on AOT-Rust: {}",
        r.stderr
    );
    assert_eq!(
        r.stdout.trim(),
        oracle.output.trim(),
        "AOT-Rust output must match the oracle"
    );

    let v = common::run_logos(VIOLATION);
    assert!(!v.success, "violated Assume must fail on AOT-Rust");
}

#[test]
fn assume_lowers_on_wasm() {
    assert!(
        compile_to_wasm(IN_RANGE).is_ok(),
        "Assume must lower on the direct-WASM tier"
    );
}

// ============================================================
//  3. Static recovery — the analog lane's input
// ============================================================

#[test]
fn assume_ranges_are_statically_recoverable() {
    let ranges = assumed_ranges(FN_SHAPED);
    assert_eq!(ranges.len(), 1, "exactly one declared range: {ranges:?}");
    assert_eq!(ranges[0].var, "x");
    assert_eq!(
        (ranges[0].lo, ranges[0].hi),
        (-1.0, 1.0),
        "negative bounds must recover exactly"
    );

    let main_level = assumed_ranges(IN_RANGE);
    assert_eq!(main_level.len(), 1);
    assert_eq!(main_level[0].var, "x");
    assert_eq!((main_level[0].lo, main_level[0].hi), (-1.0, 1.0));

    assert!(
        assumed_ranges(FN_NO_RANGES).is_empty(),
        "no Assume ⇒ no recovered ranges"
    );

    // The `Require that` spelling recovers identically — the range is the RuntimeAssert shape,
    // not the head keyword.
    let req = assumed_ranges(IN_RANGE_REQUIRE);
    assert_eq!(req.len(), 1);
    assert_eq!((req[0].lo, req[0].hi), (-1.0, 1.0));
}

// ============================================================
//  4. The analog gate consumes the ranges
// ============================================================

#[test]
fn analog_gate_reads_declared_ranges() {
    // PROMOTED: the lowering landed — with the contract declared, the analog backend now
    // produces a full artifact (netlist + schedule + certificate + budget + SPICE).
    let art =
        compile_to_analog(FN_SHAPED).expect("with Assume present the analog pipeline compiles");
    assert_eq!(art.fn_name, "scale");
    assert!(!art.netlist.cells.is_empty());
    assert!(
        art.budget.recheck(),
        "the artifact's budget must replay from its own terms"
    );
    assert!(
        art.spice.contains(".options"),
        "the SPICE netlist pins its simulator options"
    );

    let without = compile_to_analog(FN_NO_RANGES).expect_err("no ranges ⇒ refusal");
    let msg = format!("{without:?}");
    assert!(
        msg.contains("declared range"),
        "without Assume the gate must refuse UnboundedInput: {msg}"
    );
}
