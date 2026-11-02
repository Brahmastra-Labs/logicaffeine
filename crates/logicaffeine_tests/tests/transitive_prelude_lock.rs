//! PIT-OF-SUCCESS — a stdlib function that transitively calls ANOTHER prelude module's `.lg`
//! function must resolve on every execution tier.
//!
//! Regression guard for the `apply_prelude` module-selection fixpoint. Before it, a program that
//! referenced only `fsumReal` or `geometricMeanReal` failed with `Unknown function: absReal` /
//! `Unknown function: ln`: the loader spliced statistics.lg (the module the program named) but not
//! math.lg (the module statistics.lg's bodies call). `builtin_backed` names like `sqrt`/`pow` kept
//! resolving, which is why `pstdevReal` (→ `sqrt`) worked and masked the class — only the `.lg`-
//! backed transitive dependencies (`absReal`, `ln`, `exp`) went missing. The loader now selects
//! modules to a fixpoint, and `loader::assembled_prelude_is_transitively_call_closed` locks the
//! structural invariant; this file locks the observable behavior end-to-end across all tiers.

mod common;
use common::assert_all_tiers;

/// `fsumReal` (Neumaier compensated summation) calls math.lg's `absReal` — a `.lg` function, NOT a
/// builtin. The whole sum `1.5 + 2.5 + 3.0` is `7.0`; every tier must agree on `7`.
#[test]
fn fsum_pulls_math_absreal_on_all_tiers() {
    assert_all_tiers("## Main\n    Show fsumReal([1.5, 2.5, 3.0]).\n", "7");
}

/// `geometricMeanReal` works in the log domain — it calls math.lg's `ln` and `exp` (both `.lg`
/// functions). The geometric mean of `1, 2, 4` is `(1·2·4)^(1/3) = 8^(1/3) = 2`; every tier `2`.
#[test]
fn geometric_mean_pulls_math_ln_exp_on_all_tiers() {
    assert_all_tiers(
        "## Main\n    Show geometricMeanReal([1.0, 2.0, 4.0]).\n",
        "2",
    );
}
