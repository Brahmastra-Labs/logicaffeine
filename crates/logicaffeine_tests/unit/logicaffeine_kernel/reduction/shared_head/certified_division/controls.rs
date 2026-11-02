//! The evaluator's certified division on Lean's original `Nat.div` and `Nat.mod`
//! (`fixtures/lean_scale/LG_Div.lean`, pinned Lean 4.14.0 and lean4export).
use super::*;
#[path = "../../../lean_scale_export.rs"]
mod lean_scale_export;
use lean_scale_export::{environment_without, fixture, on_a_large_stack, spelled};

fn nat(n: &str) -> Term {
    Term::Lit(Literal::Nat(crate::BigInt::parse_decimal(n).unwrap()))
}
fn call(f: &str, x: Term, y: Term) -> Term {
    app(app(g(&spelled(f)), x), y)
}

/// No unrolling of the well-founded recursion reaches 2^128 / 2^64 on this allowance.
const FUEL: u64 = 1 << 16;

/// Lean's unary division program receives its operands as `PSigma.mk.{u,v} x y`, a
/// universe-polymorphic constant: the evaluator reads the operands out of that pair, observes both,
/// and answers with the certified quotient or remainder, never by unrolling.
#[test]
fn leans_division_is_computed_through_its_universe_polymorphic_pair() {
    on_a_large_stack(|| {
        let (ctx, _) =
            crate::interface::lean::import(&environment_without(&fixture("LG_Div"), "LG_DV."))
                .expect("Lean's own division imports");
        let two_64 = "18446744073709551616";
        let two_128 = "340282366920938463463374607431768211456";
        for (f, x, y, answer, wrong) in [
            ("Nat.div", "17", "5", "3", "4"),
            ("Nat.mod", "17", "5", "2", "3"),
            ("Nat.div", "17", "0", "0", "1"),
            ("Nat.mod", "17", "0", "17", "16"),
            ("Nat.div", two_128, two_64, two_64, "18446744073709551617"),
            (
                "Nat.mod",
                "340282366920938463463374607431768223801",
                "1000000007",
                "279644622",
                "279644623",
            ),
        ] {
            let lhs = call(f, nat(x), nat(y));
            assert!(
                crate::reduction::constructor_equivalent(&ctx, &lhs, &nat(answer), FUEL),
                "{f} {x} {y} = {answer}"
            );
            assert!(
                crate::reduction::constructors_disagree(&ctx, &lhs, &nat(wrong), FUEL),
                "{f} {x} {y} ≠ {wrong}"
            );
            assert!(
                !crate::reduction::constructors_disagree(&ctx, &lhs, &nat(answer), FUEL),
                "{f} {x} {y} = {answer} is never refuted"
            );
        }
    });
}
