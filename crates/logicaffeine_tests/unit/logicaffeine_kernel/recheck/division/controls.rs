//! The independent kernel's own division certificate, on Lean's original `Nat.div` and
//! `Nat.modCore` (`fixtures/lean_scale/LG_Div.lean`, pinned Lean 4.14.0 and lean4export) and on the
//! two programs shaped like `Nat.modCore` that must never be computed as `%`
//! (`fixtures/lean_scale/LG_DivImpostor.lean`).
use super::*;
#[path = "../../lean_scale_export.rs"]
mod lean_scale_export;
use lean_scale_export::{environment_without, fixture, on_a_large_stack, spelled};

fn nat_lit(n: &str) -> Term {
    Term::Lit(Literal::Nat(crate::BigInt::parse_decimal(n).unwrap()))
}
fn call(f: &str, x: Term, y: Term) -> Term {
    Term::App(
        Box::new(Term::App(Box::new(Term::Global(spelled(f))), Box::new(x))),
        Box::new(y),
    )
}
fn eq_nat(a: Term, b: Term) -> Term {
    [Term::Global("Nat".into()), a, b]
        .into_iter()
        .fold(Term::Global(spelled("Eq")), |f, x| {
            Term::App(Box::new(f), Box::new(x))
        })
}
fn refl_nat(a: Term) -> Term {
    [Term::Global("Nat".into()), a]
        .into_iter()
        .fold(Term::Global(spelled("Eq.refl")), |f, x| {
            Term::App(Box::new(f), Box::new(x))
        })
}
/// `(refl lhs : lhs = rhs)`.
fn claim(lhs: Term, rhs: Term) -> Term {
    Term::Let {
        name: "claim".into(),
        ty: Box::new(eq_nat(lhs.clone(), rhs)),
        value: Box::new(refl_nat(lhs)),
        body: Box::new(Term::Var("claim".into())),
    }
}

struct Restore(usize, usize, bool);
impl Drop for Restore {
    fn drop(&mut self) {
        WHNF_DEPTH.with(|x| x.set(self.0));
        WHNF_FUEL.with(|x| x.set(self.1));
        WHNF_EXHAUSTED.with(|x| x.set(self.2));
    }
}

/// `f` on one reduction allowance, as `reduce` spends it inside a check.
fn on_one_allowance<R>(f: impl FnOnce() -> R) -> R {
    let _restore = Restore(
        WHNF_DEPTH.with(|x| x.replace(1)),
        WHNF_FUEL.with(|x| x.replace(MAX_RECHECK_STEPS)),
        WHNF_EXHAUSTED.with(|x| x.replace(false)),
    );
    f()
}

/// The certificate this kernel earns for `lean`.
fn certified(ctx: &Context, lean: &str) -> Option<Operation> {
    on_one_allowance(|| {
        let answer = certify(ctx, &spelled(lean)).map(|(operation, _, _)| operation);
        assert!(
            !WHNF_EXHAUSTED.with(|x| x.get()) || answer.is_none(),
            "{lean}: a certificate is never earned on a spent allowance"
        );
        answer
    })
}

#[test]
fn leans_quotient_and_remainder_programs_are_certified_by_this_kernels_own_recurrence() {
    on_a_large_stack(|| {
        let (ctx, _) =
            crate::interface::lean::import(&environment_without(&fixture("LG_Div"), "LG_DV."))
                .expect("Lean's own division imports");
        assert_eq!(certified(&ctx, "Nat.div._unary"), Some(Operation::Quotient));
        assert_eq!(
            certified(&ctx, "Nat.modCore._unary"),
            Some(Operation::Remainder)
        );
        assert_eq!(
            certified(&ctx, "Nat.sub"),
            None,
            "a binary program is not a division program"
        );
    });
}

#[test]
fn quotients_and_remainders_beyond_any_unrolling_are_decided_by_the_independent_kernel() {
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
            ("Nat.mod", "5", "5", "0", "1"),
            ("Nat.div", two_128, two_64, two_64, "18446744073709551617"),
            ("Nat.mod", "18446744073709551621", "7", "0", "1"),
            (
                "Nat.mod",
                "340282366920938463463374607431768223801",
                "1000000007",
                "279644622",
                "279644623",
            ),
        ] {
            let lhs = call(f, nat_lit(x), nat_lit(y));
            crate::recheck(&ctx, &claim(lhs.clone(), nat_lit(answer)))
                .unwrap_or_else(|e| panic!("{f} {x} {y} = {answer}: {e:?}"));
            assert!(
                matches!(
                    crate::recheck(&ctx, &claim(lhs, nat_lit(wrong))),
                    Err(ReCheckError::Ill(_))
                ),
                "{f} {x} {y} = {wrong} is refused as ill-typed, never left undecided"
            );
        }
    });
}

#[test]
fn programs_shaped_like_the_modulus_are_never_certified() {
    on_a_large_stack(|| {
        let (ctx, _) = crate::interface::lean::import(&fixture("LG_DivImpostor"))
            .expect("Lean accepts both definitions");
        assert_eq!(
            certified(&ctx, "LG_DI.badMod._unary"),
            None,
            "badMod answers x + 1 where the modulus answers x"
        );
        assert_eq!(
            certified(&ctx, "LG_DI.axMod._unary"),
            None,
            "axMod subtracts through an axiom"
        );
        assert!(
            on_one_allowance(|| arithmetic::certifies_difference(&ctx, &spelled("Nat.sub"))),
            "the subtraction badMod uses is certified"
        );
        assert!(
            crate::recheck(
                &ctx,
                &claim(
                    call("LG_DI.badMod", nat_lit("5"), nat_lit("5")),
                    nat_lit("0")
                )
            )
            .is_err(),
            "badMod 5 5 is never computed as 5 % 5"
        );
    });
}

/// The first kernel's certificate store is never this kernel's evidence: a division certificate
/// forged into the first kernel's issuer for `badMod` leaves this kernel computing nothing.
#[test]
fn a_certificate_in_the_first_kernels_store_is_never_this_kernels_evidence() {
    on_a_large_stack(|| {
        let (ctx, _) = crate::interface::lean::import(&fixture("LG_DivImpostor"))
            .expect("Lean accepts both definitions");
        let program = spelled("LG_DI.badMod._unary");
        let forged = ctx.cached_division_recurrence(&program, || {
            Some(["mod".to_owned(), spelled("PSigma.mk")])
        });
        assert_eq!(
            forged,
            Some(["mod".to_owned(), spelled("PSigma.mk")]),
            "the first kernel's store holds the forgery"
        );
        assert_eq!(certified(&ctx, "LG_DI.badMod._unary"), None);
        assert!(
            crate::recheck(
                &ctx,
                &claim(
                    call("LG_DI.badMod", nat_lit("5"), nat_lit("5")),
                    nat_lit("0")
                )
            )
            .is_err(),
            "badMod 5 5 is still never computed as 5 % 5 by this kernel"
        );
    });
}

/// A refusal is a conclusion too: a program shaped like a division that is none is certified once
/// per registry generation, never again at each observation — Lean's `Nat.gcd._unary` has the
/// signature, and an omega proof observes it at numerals over and over.
#[test]
fn a_refused_program_is_certified_once_per_registry_generation() {
    on_a_large_stack(|| {
        let (ctx, _) = crate::interface::lean::import(&fixture("LG_DivImpostor"))
            .expect("Lean accepts both definitions");
        let certifications = || CERTIFICATIONS.with(|n| n.get());
        let wrong = claim(
            call("LG_DI.badMod", nat_lit("5"), nat_lit("5")),
            nat_lit("0"),
        );
        let before = certifications();
        assert!(
            crate::recheck(&ctx, &wrong).is_err(),
            "badMod 5 5 is never computed as 5 % 5"
        );
        assert_eq!(
            certifications(),
            before + 1,
            "badMod is certified once, however often it is observed"
        );
        assert!(crate::recheck(&ctx, &wrong).is_err());
        assert!(crate::recheck(
            &ctx,
            &claim(
                call("LG_DI.badMod", nat_lit("7"), nat_lit("3")),
                nat_lit("1")
            )
        )
        .is_err());
        assert_eq!(
            certifications(),
            before + 1,
            "the refusal stands while the registries do"
        );
    });
}
