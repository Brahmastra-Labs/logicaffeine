//! Lane V_vernacular — the T12-011 diagnosis, run as a test rather than argued in prose.
//!
//! `AUDIT-T12-011` recorded `the_quantifier_and_function_term_intersection_is_not_empty` as
//! `status: Native` while it FAILS, and asked whether the failing assertion is a stale guard or a
//! live soundness hole. This file answers both halves with assertions, so the ruling can be made
//! against evidence:
//!
//! 1. **The gate is SOUND.** `crates/logicaffeine_tests/tests/kernel_V_vernacular_extra.rs` pins that a
//!    FALSE quantified statement is refused on Surface C and a TRUE one is admitted. Surface C
//!    GAINED the binder form its T12-011 guard asserts it lacks; it did not gain a hole.
//! 2. **The tail of T12-011 is currently UNREACHED**, because guard 2 panics before it. Every
//!    assertion after that guard is reproduced here verbatim, so a ruling that flips the guard is
//!    not a ruling into the dark: if these pass, nothing else is hiding behind it.
//!
//! Nothing here EDITS the corpus. The corpus case is the specification; this is the measurement.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_proof::development::prove_development;
use logicaffeine_proof::formula::parse_formula;
use logicaffeine_proof::{ProofExpr, ProofTerm};

/// Every theorem of a Surface B development, with its kernel verdict. Same helper as the tier file.
fn surface_b(body: &str) -> Vec<(String, bool)> {
    prove_development(body)
        .unwrap_or_else(|e| panic!("the development must parse before it is proved: {e:?}"))
        .into_iter()
        .map(|(_, r)| (r.name, r.verified))
        .collect()
}

/// THE TAIL OF T12-011, reached. Guard 2 of
/// `the_quantifier_and_function_term_intersection_is_not_empty` panics before any of this executes,
/// so these assertions have never run in the campaign. They are the case's actual CONTENT — the
/// refutation of "the intersection `quantifier AND function term` is empty" — and if they hold, the
/// only thing wrong with T12-011 is a guard that our own T12-009 fix falsified.
#[test]
fn the_unreached_tail_of_t12_011_holds() {
    // Surface B has BOTH. This is the refutation.
    let both = parse_formula("for all n, Eq(Add(n, 0), n)")
        .expect("Surface B must parse a quantifier over a function term");
    assert!(
        matches!(&both, ProofExpr::ForAll { variable, body }
            if variable == "n"
            && matches!(body.as_ref(),
                ProofExpr::Identity(ProofTerm::Function(f, args), ProofTerm::Variable(v))
                if f == "Add"
                   && v == "n"
                   && matches!(&args[0], ProofTerm::Variable(a) if a == "n"))),
        "the parse must carry a BINDER over a FUNCTION TERM: {both:?}"
    );
    assert_eq!(
        surface_b("Theorem right_identity: prove for all n, Eq(Add(n, 0), n)."),
        vec![("right_identity".to_string(), true)],
        "`∀ n, n + 0 = n` must certify on the vernacular, from no axioms"
    );
    for (name, decl) in [
        ("add_comm", "Theorem add_comm: prove for all a b, Eq(Add(a, b), Add(b, a))."),
        ("mul_one", "Theorem mul_one: prove for all n, Eq(Mul(n, 1), n)."),
        (
            "distributivity",
            "Theorem distributivity: prove for all a b c, Eq(Mul(a, Add(b, c)), Add(Mul(a, b), Mul(a, c))).",
        ),
        ("sub_add", "Theorem sub_add: prove for all n, Eq(Sub(Add(n, 1), 1), n)."),
    ] {
        assert_eq!(
            surface_b(decl),
            vec![(name.to_string(), true)],
            "`{name}` must certify — the quantified-arithmetic fragment is general"
        );
    }
    // THE RESIDUE, pinned so the win is not overstated.
    assert_eq!(
        surface_b("Theorem named_zero: prove for all n, Eq(Add(n, Zero), n)."),
        vec![("named_zero".to_string(), false)],
        "`Zero` spelled as a name is an opaque CONSTANT, not the numeral"
    );
}
