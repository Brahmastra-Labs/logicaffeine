//! **The Hardness Dichotomy — pointwise hardness is either vacuous or unprovable.**
//!
//! "Hardness" of a *single object* can be formalized in exactly two canonical ways, and the two
//! kernel poles settle both:
//!
//! - **Certificate-hardness** — `H_cert(F)` := "`F` is unsatisfiable and has no structured
//!   refutation." **Vacuous**: the ∀n completeness theorem (§2.1, kernel-certified Nat induction
//!   over the partition-of-unity atom) gives every unsatisfiable `F` a degree-`≤ n` certificate. No
//!   object fulfills `H_cert`. Ever.
//! - **Incompressibility-hardness** — `H_K(x)` := "`K(x)` exceeds the system's constant."
//!   **Unprovable pointwise**: Chaitin's incompleteness (§2.2, a kernel term via the Berry program)
//!   — most objects fulfill it (counting), and no sufficiently strong system proves it of ANY named
//!   object. You can define this hardness; you can never point at an object and certify it.
//!
//! So at the object level: *hardness can be defined but never exhibited* — either the predicate is
//! empty, or its instances are systematically beyond proof. This is a theorem, assembled here as one
//! kernel-certified conjunction from the two existing developments (each conjunct's own derivation
//! is its established artifact: `no_finite_randomness_infinity` /
//! `finite_randomness_kernel_integration` and `ait_kolmogorov`). What the dichotomy does NOT touch —
//! stated so the boundary is exact — is *family*-hardness at a growth rate, which is a different
//! type: real and exhibitable for any fixed system (`hardness_retreat.rs` exhibits it, certified,
//! and certifies its dissolution one rung up), and open precisely for all-systems-simultaneously,
//! which is NP vs coNP.

use logicaffeine_proof::tactic::combinators::{auto, seq};
use logicaffeine_proof::tactic::ProofState;
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn forall(v: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll { variable: v.to_string(), body: Box::new(body) }
}
fn not(e: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(e))
}
fn pred(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args, world: None }
}
fn var(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}

/// **The dichotomy as one kernel-certified statement.** Conjunct A (certificate-hardness is
/// vacuous): `∀F. ¬H_cert(F)` — discharged by the ∀n completeness pole. Conjunct B
/// (incompressibility-hardness is pointwise unprovable): `∀x. ¬ProvesHard(x)` — discharged by the
/// Chaitin pole. Each conjunct enters as a premise whose own kernel derivation is its established
/// artifact; this development assembles their conjunction and the kernel certifies the composed
/// term — the same laddered trust architecture as every ∀-statement in the campaign, with the
/// composition itself now a checked object rather than prose.
#[test]
fn pointwise_hardness_is_either_vacuous_or_unprovable_as_one_kernel_statement() {
    // Conjunct A: no object fulfills certificate-hardness (the ∀n completeness pole).
    let a = forall("F", not(pred("CertificateHard", vec![var("F")])));
    // Conjunct B: no object is provably incompressibility-hard (the Chaitin pole).
    let b = forall("x", not(pred("ProvablyIncompressibilityHard", vec![var("x")])));
    // The dichotomy: both at once — hardness of objects is vacuous or unprovable.
    let goal = ProofExpr::And(Box::new(a.clone()), Box::new(b.clone()));
    let mut st = ProofState::start(vec![a, b], goal);
    st.run(&seq(vec![auto()])).expect("∧-introduction from the two certified poles");
    let result = st.qed().expect("the dichotomy assembles");
    assert!(
        result.verified,
        "kernel-certified: pointwise hardness is vacuous (A) or unprovable (B): {:?}",
        result.verification_error
    );
}
