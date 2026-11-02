//! Rung 0a registration-order lock — a definiendum is a δ-unfoldable kernel
//! DEFINITION in the prepared context, whatever order the symbol collector
//! hands its atoms to the kernel.
//!
//! `prepare_ctx_with_defs` declares the atoms it collects out of a `HashMap`,
//! whose iteration order is fresh on every proof thread. A definiendum that is
//! collected from an UNEXPANDED goal (the `check_derivation_with_defs` entry)
//! must never be re-declared as an opaque predicate: the kernel hides a
//! definition whose body is still pending from `get_global`, so a probe-then-
//! declare pass would displace the definition with an axiom, and δ could no
//! longer reconcile the definiens with the goal. Every call below runs on a
//! proof thread of its own, with hash seeds of its own, so 32 calls sample 32
//! independent registration orders; with eight atoms in the definiens a
//! registration that depends on the order fails 8 rounds in 9, and cannot
//! survive all 32 (p < 2⁻¹⁰⁰). Even rounds use a definiendum that sorts BEFORE
//! its atoms and odd rounds one that sorts AFTER them, so a registration that
//! merely sorted the names would still be caught on one side.

use logicaffeine_proof::verify::{check_derivation_with_defs, Definition};
use logicaffeine_proof::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

const ROUNDS: usize = 32;

fn konst(name: &str) -> ProofTerm {
    ProofTerm::Constant(name.to_string())
}

fn pred(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.to_string(),
        args,
        world: None,
    }
}

/// `a₁ ∧ (a₂ ∧ (… ∧ aₙ))`, right-nested.
fn conjunction(atoms: &[ProofExpr]) -> ProofExpr {
    let (last, rest) = atoms.split_last().expect("at least one atom");
    rest.iter().rev().fold(last.clone(), |acc, a| {
        ProofExpr::And(Box::new(a.clone()), Box::new(acc))
    })
}

/// The ∧-introduction chain proving [`conjunction`] from its atoms as premises.
fn derivation(atoms: &[ProofExpr]) -> DerivationTree {
    let (last, rest) = atoms.split_last().expect("at least one atom");
    rest.iter().rev().fold(
        DerivationTree::leaf(last.clone(), InferenceRule::PremiseMatch),
        |acc, a| {
            DerivationTree::new(
                ProofExpr::And(Box::new(a.clone()), Box::new(acc.conclusion.clone())),
                InferenceRule::ConjunctionIntro,
                vec![
                    DerivationTree::leaf(a.clone(), InferenceRule::PremiseMatch),
                    acc,
                ],
            )
        },
    )
}

/// `name(x) :↔ atom₁(x) ∧ … ∧ atom₈(x)` over the given atom stem.
fn eight_atom_definition(name: &str, stem: &str) -> (Definition, Vec<String>) {
    let atoms: Vec<String> = (1..=8).map(|i| format!("{stem}{i}")).collect();
    let body: Vec<ProofExpr> = atoms.iter().map(|a| pred(a, vec![konst("x")])).collect();
    let def = Definition {
        name: name.to_string(),
        params: vec!["x".to_string()],
        definiens: conjunction(&body),
    };
    (def, atoms)
}

#[test]
fn definiendum_is_a_kernel_definition_in_every_registration_order() {
    for round in 0..ROUNDS {
        let (name, stem) = if round % 2 == 0 {
            ("aardvark", "zebra")
        } else {
            ("zymurgy", "aardwolf")
        };
        let (def, atoms) = eight_atom_definition(name, stem);
        let premises: Vec<ProofExpr> = atoms.iter().map(|a| pred(a, vec![konst("A")])).collect();
        let goal = pred(name, vec![konst("A")]);
        let tree = derivation(&premises);

        let result = check_derivation_with_defs(&premises, &goal, &[def], tree);

        assert!(
            result.kernel_ctx.is_definition(name),
            "round {round}: `{name}` is not a kernel definition — an opaque declaration displaced it"
        );
        assert!(
            result.kernel_ctx.get_definition_body(name).is_some(),
            "round {round}: `{name}` has no δ-unfoldable body"
        );
        assert!(
            !result
                .kernel_ctx
                .pending_obligations()
                .iter()
                .any(|(who, _)| *who == name),
            "round {round}: `{name}` left pending after preparation: {:?}",
            result.kernel_ctx.pending_obligations()
        );
        assert!(
            result.verified,
            "round {round}: `{name}(A)` should δ-reconcile with its definiens: {:?}",
            result.verification_error
        );
    }
}
