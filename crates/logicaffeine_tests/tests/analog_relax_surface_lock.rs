//! ★ RELAXATION SURFACE LOCK ★ — the four-fabric thesis reaches the LANGUAGE: `Solve` and
//! `Minimize` are statements, their digital semantics are exact stdlib solves on every tier,
//! and one program's partition spans all four faces of the machine.
//!
//! What this file locks:
//!   1. `Solve M times x equals v for x.` — parses, desugars to the exact stdlib solve, and
//!      returns the true solution on tw AND VM (the digital correctness story; the settling
//!      fabric is the certificate-gated accelerator for the same equation, locked in R1).
//!   2. `Minimize the quadratic Q with linear b over x.` — the exact minimizer (`Q·x = −b`),
//!      verified against hand math.
//!   3. SHAPE ERRORS REFUSE with the form named (a mismatched unknown, a missing product).
//!   4. THE FOUR FABRICS — one program partitions into Dataflow + Dynamics + Relaxation +
//!      Digital, the Wave-4 composition demo.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. The digital path IS the semantics; accelerators
//! only ever match it inside their own certificates.

use logicaffeine_compile::codegen_ana::AnalogTarget;
use logicaffeine_compile::codegen_mx::{fabrics, Fabric};
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// `[[4,1],[1,3]]·x = [1,2]` ⇒ x = [1/11, 7/11].
const SOLVE: &str = "## Main\n    Let mutable m be a new Seq of Seq of Float.\n    Let mutable r1 be a new Seq of Float.\n    Push 4.0 to r1.\n    Push 1.0 to r1.\n    Push r1 to m.\n    Let mutable r2 be a new Seq of Float.\n    Push 1.0 to r2.\n    Push 3.0 to r2.\n    Push r2 to m.\n    Let mutable v be a new Seq of Float.\n    Push 1.0 to v.\n    Push 2.0 to v.\n    Solve m times x equals v for x.\n    Show item 1 of x.\n    Show item 2 of x.\n";

/// Minimize ½xᵀQx + bᵀx with Q=[[2,0],[0,4]], b=[−2,−8] ⇒ x = [1, 2].
const MINIMIZE: &str = "## Main\n    Let mutable q be a new Seq of Seq of Float.\n    Let mutable r1 be a new Seq of Float.\n    Push 2.0 to r1.\n    Push 0.0 to r1.\n    Push r1 to q.\n    Let mutable r2 be a new Seq of Float.\n    Push 0.0 to r2.\n    Push 4.0 to r2.\n    Push r2 to q.\n    Let mutable b be a new Seq of Float.\n    Push 0.0 - 2.0 to b.\n    Push 0.0 - 8.0 to b.\n    Minimize the quadratic q with linear b over x.\n    Show item 1 of x.\n    Show item 2 of x.\n";

#[test]
fn solve_returns_the_true_solution_on_both_tiers() {
    for (name, out) in [("tw", tw_outcome(SOLVE)), ("vm", vm_outcome(SOLVE))] {
        assert_eq!(out.error, None, "{name}: {out:?}");
        let nums: Vec<f64> = out
            .output
            .split_whitespace()
            .map(|t| t.parse().unwrap())
            .collect();
        assert_eq!(nums.len(), 2, "{name}: {out:?}");
        assert!(
            (nums[0] - 1.0 / 11.0).abs() < 1e-9,
            "{name}: x₁ = {}",
            nums[0]
        );
        assert!(
            (nums[1] - 7.0 / 11.0).abs() < 1e-9,
            "{name}: x₂ = {}",
            nums[1]
        );
    }
}

#[test]
fn minimize_returns_the_exact_minimizer() {
    for (name, out) in [("tw", tw_outcome(MINIMIZE)), ("vm", vm_outcome(MINIMIZE))] {
        assert_eq!(out.error, None, "{name}: {out:?}");
        let nums: Vec<f64> = out
            .output
            .split_whitespace()
            .map(|t| t.parse().unwrap())
            .collect();
        assert!(
            (nums[0] - 1.0).abs() < 1e-9 && (nums[1] - 2.0).abs() < 1e-9,
            "{name}: {nums:?}"
        );
    }
}

#[test]
fn shape_errors_refuse_with_the_form_named() {
    let mismatched = SOLVE.replace("for x.", "for y.");
    let out = tw_outcome(&mismatched);
    let msg = out.error.expect("a mismatched unknown must refuse");
    assert!(msg.contains("SAME name"), "{msg}");

    let no_product = SOLVE.replace("Solve m times x equals v for x.", "Solve m equals v for x.");
    let out = tw_outcome(&no_product);
    let msg = out.error.expect("a product-less Solve must refuse");
    assert!(msg.contains("M times x"), "{msg}");
}

#[test]
fn one_program_spans_all_four_fabrics() {
    let program = "## To blend (a: Float, b: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Return a * 0.25 + b * 0.5.\n\n## To settle (m: Seq of Seq of Float, v: Seq of Float) -> Seq of Float:\n    Solve m times x equals v for x.\n    Return x.\n\n## To relax (q: Seq of Seq of Float, b: Seq of Float) -> Seq of Float:\n    Minimize the quadratic q with linear b over x.\n    Return x.\n\n## To host (s: Text) -> Text:\n    Return s.\n\n## Main\n    Show blend(0.5, 0.25).\n";
    let tags = fabrics(program, &AnalogTarget::ideal());
    let of = |n: &str| tags.iter().find(|(m, _)| m == n).map(|(_, f)| *f);
    assert_eq!(of("blend"), Some(Fabric::Dataflow), "{tags:?}");
    assert_eq!(of("settle"), Some(Fabric::Dynamics), "{tags:?}");
    assert_eq!(of("relax"), Some(Fabric::Relaxation), "{tags:?}");
    assert_eq!(of("host"), Some(Fabric::Digital), "{tags:?}");
}
