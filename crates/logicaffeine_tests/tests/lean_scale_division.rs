//! Lean's `Nat.div` and `Nat.mod` on literals — a CHECKED acceleration in both kernels.
//!
//! Lean's kernel computes `Nat.div` and `Nat.mod` on literals natively (GMP). In the export they are
//! `Nat.div._unary` and `Nat.modCore._unary`: definitions by well-founded recursion
//! (`WellFounded.fix`), whose unrolling goes through `Acc.rec` on a proof no kernel reduces cheaply,
//! and whose recursion subtracts one divisor per step — so `2^128 / 2^64` is out of reach of any
//! reduction. Every acceptance below is decided by the checked acceleration or not at all.
//!
//! The acceleration trusts no name and no axiom. A well-founded program on pairs of naturals is
//! computed natively only after the kernel that uses it proves, by its own conversion, the
//! program's one-step unfolding (δ, proof irrelevance on the accessibility proof, ι), and that the
//! unfolding is Lean's recurrence over already-checked vocabulary — the guard `0 < y ∧ y ≤ x` over
//! the standard `And` and `Nat.le`, the checked subtraction and addition, and `dite`:
//! * modulus: `f x y = if 0 < y ∧ y ≤ x then f (x - y) y else x`;
//! * quotient: `f x y = if 0 < y ∧ y ≤ x then f (x - y) y + 1 else 0`.
//!
//! Fixture: `fixtures/lean_scale/LG_Div.lean` (Lean's own `%` and `/`, by zero, at 2^64 and 2^128,
//! through `Fin` and `UInt8`), exported with the pinned Lean 4.14.0 and the pinned lean4export;
//! `fixtures/lean_scale/LG_DivImpostor.lean` holds two programs shaped like `Nat.modCore` that must
//! never be accelerated (one answers `x + 1`, one recurses through an axiom).
#![allow(dead_code)]

#[path = "harvest_support/checker.rs"]
mod checker;
#[path = "lean_scale_support/witness.rs"]
mod witness;

use checker::{check_in, Root};
use logicaffeine_kernel::interface::lean::reference::Reference;
use logicaffeine_kernel::{defeq_for_test, Term};
use witness::{
    against, both_accept, both_refuse, kernel_spelling, map_literal, with_last_argument,
};

const STEM: &str = "LG_Div";

const ROOTS: [&str; 10] = [
    "LG_DV.modSmall",
    "LG_DV.modWrap",
    "LG_DV.divBasic",
    "LG_DV.divByZero",
    "LG_DV.modByZero",
    "LG_DV.mod64",
    "LG_DV.div128",
    "LG_DV.mod128",
    "LG_DV.finDiffer",
    "LG_DV.uint8Wrap",
];

/// Lean's `#print axioms` for each root, in `ROOTS` order.
const AXIOMS: [&[&str]; 10] = [&[], &[], &[], &[], &[], &[], &[], &[], &[], &["propext"]];

fn fixture() -> String {
    witness::witness_with_axioms(STEM, &ROOTS, &AXIOMS).0
}

#[test]
fn leans_division_and_modulus_import_at_their_statements_in_both_kernels() {
    let export = fixture();
    let reference = Reference::read(&export).expect("the reference translator reads the export");
    let (ctx, report) = witness::imported(&export, &ROOTS)
        .unwrap_or_else(|e| panic!("the production reader must accept Lean's own division: {e}"));
    let mut failures = Vec::new();
    for (lean, axioms) in ROOTS.into_iter().zip(AXIOMS) {
        if let Err(why) = check_in(
            &export,
            &ctx,
            &report,
            &reference,
            &Root {
                lean,
                kind: "theorem",
                axioms,
            },
        ) {
            failures.push(format!("{lean}: {why}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Each `rfl` root with its answer moved by one: `4 % 5 = 3`, `5 % 5 = 1`, `17 / 5 = 4`,
/// `17 / 0 = 1`, `17 % 0 = 16`, and at 2^64 and 2^128.
const FALSIFIED: [(&str, u64, u64); 5] = [
    ("LG_DV.modSmall", 4, 3),
    ("LG_DV.modWrap", 0, 1),
    ("LG_DV.divBasic", 3, 4),
    ("LG_DV.divByZero", 0, 1),
    ("LG_DV.modByZero", 17, 16),
];

#[test]
fn a_false_quotient_or_remainder_is_refused_by_both_kernels() {
    let export = fixture();
    let reference = Reference::read(&export).expect("the reference translator reads the export");
    let (ctx, _) = witness::imported(&export, &ROOTS).expect("the import");
    for (lean, from, to) in FALSIFIED {
        let name = kernel_spelling(lean);
        let statement = reference
            .declaration(&name)
            .expect("reference translation")
            .ty
            .clone();
        let (_, proof) = ctx.retained(&name).expect("the proof is retained");
        both_accept(&ctx, &against(proof, &statement))
            .unwrap_or_else(|e| panic!("{lean} at its statement: {e}"));
        let altered = with_last_argument(&statement, |a| map_literal(a, from, to));
        assert!(
            !defeq_for_test(&ctx, &statement, &altered),
            "{lean}: the altered answer must differ: {altered}"
        );
        assert!(
            both_refuse(&ctx, &against(proof, &altered)),
            "{lean}: both kernels refuse the proof at {altered}"
        );
    }
}

/// The 2^64 and 2^128 roots with their answers moved by one: each is refused, never decided by
/// running out of budget into an acceptance.
#[test]
fn a_false_answer_at_two_to_the_sixty_four_and_one_twenty_eight_is_refused() {
    let export = fixture();
    let reference = Reference::read(&export).expect("the reference translator reads the export");
    let (ctx, _) = witness::imported(&export, &ROOTS).expect("the import");
    for (lean, change) in [
        ("LG_DV.mod64", (0u64, 1u64)),
        ("LG_DV.mod128", (279_644_622, 279_644_623)),
    ] {
        let name = kernel_spelling(lean);
        let statement = reference
            .declaration(&name)
            .expect("reference translation")
            .ty
            .clone();
        let (_, proof) = ctx.retained(&name).expect("the proof is retained");
        let altered = with_last_argument(&statement, |a| map_literal(a, change.0, change.1));
        assert!(
            both_refuse(&ctx, &against(proof, &altered)),
            "{lean}: both kernels refuse the proof at {altered}"
        );
    }
    // `2^128 / 2^64 = 2^64` becomes `= 2^64 + 1`: the literal is past `u64`, so rebuild it.
    let name = kernel_spelling("LG_DV.div128");
    let statement = reference
        .declaration(&name)
        .expect("reference translation")
        .ty
        .clone();
    let (_, proof) = ctx.retained(&name).expect("the proof is retained");
    let altered = with_last_argument(&statement, |a| bump_big_literal(a));
    assert!(
        !defeq_for_test(&ctx, &statement, &altered),
        "div128: the altered answer must differ: {altered}"
    );
    assert!(
        both_refuse(&ctx, &against(proof, &altered)),
        "div128: both kernels refuse the proof at {altered}"
    );
}

/// The answer `2^64` becomes `2^64 + 1` wherever it is spelled. The reference reads the answer as
/// `OfNat.ofNat Nat 2^64 (instOfNatNat 2^64)`, so both occurrences move, as `map_literal` moves a
/// `u64` answer.
fn bump_big_literal(t: &Term) -> Term {
    use logicaffeine_kernel::{BigInt, Literal};
    fn go(t: &Term, from: &BigInt, moved: &mut usize) -> Term {
        match t {
            Term::Lit(Literal::Nat(n)) if n == from => {
                *moved += 1;
                Term::Lit(Literal::Nat(n.add(&BigInt::from_u64(1))))
            }
            Term::App(f, a) => {
                Term::App(Box::new(go(f, from, moved)), Box::new(go(a, from, moved)))
            }
            other => other.clone(),
        }
    }
    let two_64 = BigInt::parse_decimal("18446744073709551616").unwrap();
    let mut moved = 0;
    let bumped = go(t, &two_64, &mut moved);
    assert_eq!(
        moved, 2,
        "the answer spells 2^64 as the numeral and as its instance's index: {t}"
    );
    bumped
}

const IMPOSTORS: [&str; 2] = ["LG_DI.badMod", "LG_DI.axMod"];

/// Programs shaped like `Nat.modCore` that are not Lean's modulus import and check as ordinary
/// definitions, and are never computed as `%`: `badMod 5 5` is `badMod 0 5 = 1` by its own
/// recurrence, never `0`; `axMod` recurses through an axiom.
#[test]
fn a_program_shaped_like_the_modulus_is_not_accelerated() {
    let (export, _) = witness::witness_with_axioms(
        "LG_DivImpostor",
        &IMPOSTORS,
        &[&[], &["LG_DI.opaqueSub", "LG_DI.opaqueSub_lt"]],
    );
    let (ctx, _) = witness::imported(&export, &[]).expect("Lean's kernel accepts both definitions");
    let call = |f: &str, x: u64, y: u64| {
        let lit = |n: u64| {
            Term::Lit(logicaffeine_kernel::Literal::Nat(
                logicaffeine_kernel::BigInt::from_u64(n),
            ))
        };
        Term::App(
            Box::new(Term::App(
                Box::new(Term::Global(kernel_spelling(f))),
                Box::new(lit(x)),
            )),
            Box::new(lit(y)),
        )
    };
    let lit = |n: u64| {
        Term::Lit(logicaffeine_kernel::Literal::Nat(
            logicaffeine_kernel::BigInt::from_u64(n),
        ))
    };
    assert!(
        !defeq_for_test(&ctx, &call("LG_DI.badMod", 5, 5), &lit(0)),
        "badMod is not the modulus"
    );
    assert!(
        !defeq_for_test(&ctx, &call("LG_DI.axMod", 5, 5), &lit(0)),
        "axMod is not the modulus"
    );
}
