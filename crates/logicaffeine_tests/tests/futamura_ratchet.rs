//! The Futamura / Jones ratchet — a single monotonic scoreboard.
//!
//! Each floor reflects what is PROVEN today and may only RISE. When a phase lands it
//! raises its floor and the exhaustive locks pin it there; if a later change regresses
//! coverage below a floor, THIS test goes red. That is the "ratchet to perfection":
//! Jones optimality can be added but never silently lost.
//!
//! Raise these constants (never lower them) as the campaign advances.

mod pe_support;

use logicaffeine_compile::compile::count_dispatch;
use pe_support::*;

/// How many of the three Futamura projections currently produce Jones-optimal
/// (zero-dispatch) residuals over the smoke corpus.
///   1 = P1 today.  Phase 3 raises this to 3 (the P2 compiler + the P3 cogen).
const PROJECTIONS_AT_JONES_ZERO: usize = 1;

/// Representative programs spanning the folded surface (straight-line, recursion, loops).
fn smoke_corpus() -> Vec<(&'static str, &'static str)> {
    vec![
        ("arith", "## Main\nShow 2 + 3 * 4."),
        (
            "recursion",
            "## To fact (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * fact(n - 1).\n\n## Main\nShow fact(5).",
        ),
        (
            "loop",
            "## Main\nLet mutable s be 0.\nRepeat for i from 1 to 5:\n    Set s to s + i.\nShow s.",
        ),
    ]
}

/// P1 residuals are Jones-optimal today; this floor can only rise as P2/P3 join.
#[test]
fn projections_at_jones_zero_never_regresses() {
    let mut p1_ok = true;
    for (name, prog) in smoke_corpus() {
        match decompile(prog) {
            Ok(residual) => {
                let d = count_dispatch(&residual);
                assert_eq!(
                    d, 0,
                    "[{name}] P1 residual regressed to {d} dispatch unit(s):\n{residual}"
                );
            }
            Err(e) => {
                p1_ok = false;
                eprintln!("[{name}] P1 projection failed: {e}");
            }
        }
    }
    // Phase 3 will add P2 (generated compiler) and P3 (generated cogen) checks here and
    // raise PROJECTIONS_AT_JONES_ZERO to 3.
    let achieved = usize::from(p1_ok);
    assert!(
        achieved >= PROJECTIONS_AT_JONES_ZERO,
        "projection Jones-coverage regressed below floor {PROJECTIONS_AT_JONES_ZERO} (achieved {achieved})"
    );
}

/// Liveness floor for the hardened oracle: dispatch hidden in a container is caught and
/// clean code is not over-flagged. The exhaustive adversarial corpus lives in
/// `phase_pe_jones_adversarial.rs`; this guards against the oracle being weakened back to
/// the old `_ => {}` counter.
#[test]
fn oracle_hardening_never_regresses() {
    assert!(
        count_dispatch("## Main\nLet xs be [coreEval(a, b, c)].\nShow \"ok\".") > 0,
        "oracle regressed: dispatch nested in a List slipped past count_dispatch"
    );
    assert_eq!(
        count_dispatch("## Main\nShow 2 + 3 * 4."),
        0,
        "oracle regressed: clean straight-line code was flagged as overhead"
    );
}
