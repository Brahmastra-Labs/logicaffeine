//! Core reasoning primitives that real mathematics leans on, proved + kernel-certified
//! through the general prover. Each is a first-order workhorse: transitive chaining,
//! substitution of equals, contraposition.

use logicaffeine_proof::verify::prove_certify_check;
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn k(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn v(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn p(n: &str, a: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate {
        name: n.to_string(),
        args: a,
        world: None,
    }
}
fn and(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(l), Box::new(r))
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}
fn not_(e: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(e))
}
fn forall(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll {
        variable: var.to_string(),
        body: Box::new(body),
    }
}
fn eq(l: ProofTerm, r: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(l, r)
}
fn iff(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Iff(Box::new(l), Box::new(r))
}

/// Reflexivity of equality: ⊢ `A = A`.
#[test]
fn equality_reflexivity() {
    let r = prove_certify_check(&[], &eq(k("A"), k("A")));
    assert!(r.verified, "A = A should hold: {:?}", r.verification_error);
}

/// Symmetry of equality: `A = B` ⊢ `B = A`.
#[test]
fn equality_symmetry() {
    let r = prove_certify_check(&[eq(k("A"), k("B"))], &eq(k("B"), k("A")));
    assert!(r.verified, "A = B ⊢ B = A: {:?}", r.verification_error);
}

/// Transitivity of equality: `A = B`, `B = C` ⊢ `A = C`.
#[test]
fn equality_transitivity() {
    let r = prove_certify_check(
        &[eq(k("A"), k("B")), eq(k("B"), k("C"))],
        &eq(k("A"), k("C")),
    );
    assert!(
        r.verified,
        "A = B, B = C ⊢ A = C: {:?}",
        r.verification_error
    );
}

/// Proof by cases (∨-elimination): `P ∨ Q`, `P → R`, `Q → R` ⊢ `R`.
#[test]
fn proof_by_cases() {
    let pp = p("sunny", vec![k("A")]);
    let qq = p("rainy", vec![k("A")]);
    let rr = p("outside", vec![k("A")]);
    let premises = [
        ProofExpr::Or(Box::new(pp.clone()), Box::new(qq.clone())),
        implies(pp.clone(), rr.clone()),
        implies(qq.clone(), rr.clone()),
    ];
    let r = prove_certify_check(&premises, &rr);
    assert!(
        r.verified,
        "case analysis should conclude R: {:?}",
        r.verification_error
    );
}

/// Ex falso quodlibet: `P`, `¬P` ⊢ `Q` (anything follows from a contradiction).
#[test]
fn contradiction_ex_falso() {
    let premises = [p("hot", vec![k("A")]), not_(p("hot", vec![k("A")]))];
    let goal = p("blue", vec![k("B")]);
    let r = prove_certify_check(&premises, &goal);
    assert!(
        r.verified,
        "a contradiction should prove anything: {:?}",
        r.verification_error
    );
}

/// Biconditional elimination (forward): `P ↔ Q`, `P` ⊢ `Q`.
#[test]
fn biconditional_forward() {
    let premises = [
        iff(p("rains", vec![k("A")]), p("wet", vec![k("A")])),
        p("rains", vec![k("A")]),
    ];
    let goal = p("wet", vec![k("A")]);
    let r = prove_certify_check(&premises, &goal);
    assert!(r.verified, "P ↔ Q, P ⊢ Q: {:?}", r.verification_error);
}

/// Implication introduction (→I): prove `P → Q` by assuming `P` and deriving `Q`.
/// `⊢ P → P` — assume P, conclude P by assumption. (The prover has MP but, per the
/// roadmap, was missing →I — you can use an implication but not construct one.)
#[test]
fn implication_introduction_reflexive() {
    let goal = implies(p("rains", vec![k("A")]), p("rains", vec![k("A")]));
    let r = prove_certify_check(&[], &goal);
    assert!(r.verified, "⊢ P → P: {:?}", r.verification_error);
}

/// →I discharging the antecedent into the context: `Q ⊢ P → Q`.
#[test]
fn implication_introduction_weakening() {
    let q = p("wet", vec![k("A")]);
    let goal = implies(p("rains", vec![k("A")]), q.clone());
    let r = prove_certify_check(&[q], &goal);
    assert!(r.verified, "Q ⊢ P → Q: {:?}", r.verification_error);
}

/// Universal introduction (∀I): prove `∀x. φ(x)` by proving φ for an arbitrary x.
/// `⊢ ∀x. (loved(x) → loved(x))` — generalize over x, then →I.
#[test]
fn universal_introduction() {
    let goal = forall(
        "x",
        implies(p("loved", vec![v("x")]), p("loved", vec![v("x")])),
    );
    let r = prove_certify_check(&[], &goal);
    assert!(
        r.verified,
        "∀x.(loved(x) → loved(x)): {:?}",
        r.verification_error
    );
}

/// Soundness control for ∀I: a universal must NOT follow from a single instance.
/// `man(Socrates) ⊬ ∀x. man(x)` — the kernel rejects generalizing a bound instance
/// (the body proof would have type `man(Socrates)`, not `man(x)`).
#[test]
fn universal_introduction_rejects_unsound_generalization() {
    let premises = [p("man", vec![k("Socrates")])];
    let goal = forall("x", p("man", vec![v("x")]));
    let r = prove_certify_check(&premises, &goal);
    assert!(!r.verified, "∀x.man(x) must NOT follow from man(Socrates)");
}

/// Biconditional introduction (↔I): prove `P ↔ Q` by proving both directions.
/// `⊢ P ↔ P` — both directions are `P → P`, each by →I.
#[test]
fn biconditional_introduction_reflexive() {
    let pp = p("rains", vec![k("A")]);
    let goal = iff(pp.clone(), pp);
    let r = prove_certify_check(&[], &goal);
    assert!(r.verified, "⊢ P ↔ P: {:?}", r.verification_error);
}

/// ↔I from the two implication directions: `P → Q`, `Q → P` ⊢ `P ↔ Q`.
#[test]
fn biconditional_introduction_from_directions() {
    let pp = p("rains", vec![k("A")]);
    let qq = p("wet", vec![k("A")]);
    let premises = [
        implies(pp.clone(), qq.clone()),
        implies(qq.clone(), pp.clone()),
    ];
    let goal = iff(pp, qq);
    let r = prove_certify_check(&premises, &goal);
    assert!(r.verified, "P→Q, Q→P ⊢ P↔Q: {:?}", r.verification_error);
}

/// Double-negation introduction (constructive): `P ⊢ ¬¬P`. The engine produced this
/// rule but the certifier had no arm for it (a certifier-gap bug). `¬X ≡ X→False`,
/// so `¬¬P = (P→False)→False`, proved by `λ(hnp:¬P). hnp p`.
#[test]
fn double_negation_introduction() {
    let pp = p("rains", vec![k("A")]);
    let goal = not_(not_(pp.clone()));
    let r = prove_certify_check(&[pp], &goal);
    assert!(r.verified, "P ⊢ ¬¬P: {:?}", r.verification_error);
}

/// Classical reductio (proof by contradiction): for a positive goal G, assume ¬G,
/// derive ⊥, conclude G via the `dne` axiom. `¬¬P ⊢ P` — double-negation
/// elimination, classically valid (assume ¬P, contradict ¬¬P, conclude P).
#[test]
fn classical_double_negation_elimination() {
    let pp = p("rains", vec![k("A")]);
    let premises = [not_(not_(pp.clone()))];
    let goal = pp;
    let r = prove_certify_check(&premises, &goal);
    assert!(
        r.verified,
        "¬¬P ⊢ P (classical): {:?}",
        r.verification_error
    );
}

/// Transitive chaining — the workhorse of order/equality reasoning:
/// `∀x∀y∀z. (less(x,y) ∧ less(y,z)) → less(x,z)`, `less(A,B)`, `less(B,C)` ⊢ `less(A,C)`.
#[test]
fn transitivity_chain() {
    let trans = forall(
        "x",
        forall(
            "y",
            forall(
                "z",
                implies(
                    and(
                        p("less", vec![v("x"), v("y")]),
                        p("less", vec![v("y"), v("z")]),
                    ),
                    p("less", vec![v("x"), v("z")]),
                ),
            ),
        ),
    );
    let premises = [
        trans,
        p("less", vec![k("A"), k("B")]),
        p("less", vec![k("B"), k("C")]),
    ];
    let goal = p("less", vec![k("A"), k("C")]);

    let r = prove_certify_check(&premises, &goal);
    assert!(
        r.verified,
        "transitivity should chain to less(A, C): {:?}",
        r.verification_error
    );
}

/// Substitution of equals (Leibniz): `A = B`, `big(A)` ⊢ `big(B)`.
#[test]
fn equality_substitution() {
    let premises = [ProofExpr::Identity(k("A"), k("B")), p("big", vec![k("A")])];
    let goal = p("big", vec![k("B")]);

    let r = prove_certify_check(&premises, &goal);
    assert!(
        r.verified,
        "substitution of equals should give big(B): {:?}",
        r.verification_error
    );
}

/// Contraposition / modus tollens: `P → Q`, `¬Q` ⊢ `¬P`.
#[test]
fn modus_tollens() {
    let premises = [
        implies(p("rains", vec![k("A")]), p("wet", vec![k("A")])),
        not_(p("wet", vec![k("A")])),
    ];
    let goal = not_(p("rains", vec![k("A")]));

    let r = prove_certify_check(&premises, &goal);
    assert!(
        r.verified,
        "modus tollens should give ¬rains(A): {:?}",
        r.verification_error
    );
}
