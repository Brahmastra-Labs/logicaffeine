//! Rung 0a, Stride 2 — kernel registration of user definitions + δ reconciliation.
//!
//! These tests bypass proof *search* (the engine treats a definiendum opaquely
//! until Stride 3's expand-for-search). They drive [`check_derivation_with_defs`]
//! with a hand-built derivation of the *definiens*, and assert that the kernel
//! reconciles it against a goal stated with the *definiendum* via δ-unfolding —
//! proving the definition is a real, δ-unfoldable kernel node, not an inlining.

use logicaffeine_proof::verify::{
    check_derivation_with_defs, dependency_graph, prove_certify_check_with_defs, Definition,
};
use logicaffeine_proof::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

/// An `Entity` constant. By the proof layer's naming convention a constant is a
/// *capitalized* proper name (a bare lowercase name is read as a variable and
/// left unregistered) — so subjects here are `"A"`, `"B"`, `"C"`, while the
/// definition's bound parameters stay lowercase `"x"`, `"y"`, `"z"`.
fn konst(name: &str) -> ProofTerm {
    ProofTerm::Constant(name.to_string())
}

fn pred(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args, world: None }
}

fn and(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(l), Box::new(r))
}

/// `glorp(x) :↔ shiny(x) ∧ round(x)`. A derivation of `shiny(a) ∧ round(a)`
/// must certify against the goal `glorp(a)` — the definition δ-unfolds at the root.
#[test]
fn definition_unfolds_at_kernel_root() {
    // Definiens parameters are Constants (mirroring the real LogicExpr→ProofExpr
    // lowering), so this also exercises the Global→Var rewrite in registration.
    let def = Definition {
        name: "glorp".to_string(),
        params: vec!["x".to_string()],
        definiens: and(
            pred("shiny", vec![konst("x")]),
            pred("round", vec![konst("x")]),
        ),
    };

    let shiny_a = pred("shiny", vec![konst("A")]);
    let round_a = pred("round", vec![konst("A")]);
    let glorp_a = pred("glorp", vec![konst("A")]);

    let tree = DerivationTree::new(
        and(shiny_a.clone(), round_a.clone()),
        InferenceRule::ConjunctionIntro,
        vec![
            DerivationTree::leaf(shiny_a.clone(), InferenceRule::PremiseMatch),
            DerivationTree::leaf(round_a.clone(), InferenceRule::PremiseMatch),
        ],
    );

    let result = check_derivation_with_defs(&[shiny_a, round_a], &glorp_a, &[def], tree);
    assert!(
        result.verified,
        "glorp(a) should δ-reconcile with shiny(a) ∧ round(a): {:?}",
        result.verification_error
    );
}

/// Multi-argument definition: `between(x,y,z) :↔ near(x,y) ∧ near(y,z)`.
/// Exercises multi-parameter λ-abstraction and the Global→Var rewrite per param.
#[test]
fn multi_argument_definition_unfolds() {
    let def = Definition {
        name: "between".to_string(),
        params: vec!["x".to_string(), "y".to_string(), "z".to_string()],
        definiens: and(
            pred("near", vec![konst("x"), konst("y")]),
            pred("near", vec![konst("y"), konst("z")]),
        ),
    };

    let near_ab = pred("near", vec![konst("A"), konst("B")]);
    let near_bc = pred("near", vec![konst("B"), konst("C")]);
    let between_abc = pred("between", vec![konst("A"), konst("B"), konst("C")]);

    let tree = DerivationTree::new(
        and(near_ab.clone(), near_bc.clone()),
        InferenceRule::ConjunctionIntro,
        vec![
            DerivationTree::leaf(near_ab.clone(), InferenceRule::PremiseMatch),
            DerivationTree::leaf(near_bc.clone(), InferenceRule::PremiseMatch),
        ],
    );

    let result = check_derivation_with_defs(&[near_ab, near_bc], &between_abc, &[def], tree);
    assert!(
        result.verified,
        "between(a,b,c) should δ-reconcile with near(a,b) ∧ near(b,c): {:?}",
        result.verification_error
    );
}

/// Stride 3 — expand-for-search. The backward chainer (`Auto`) proves a goal
/// stated with a *defined* predicate, with NO hand-built derivation: the engine
/// treats `glorp` opaquely, so the `_with_defs` path δ-expands it in the goal
/// (and premises) before search, then reconciles the folded goal type at the
/// root. `glorp(A)` is proved from `shiny(A)` and `round(A)`.
#[test]
fn engine_proves_defined_goal_via_expansion() {
    let def = Definition {
        name: "glorp".to_string(),
        params: vec!["x".to_string()],
        definiens: and(
            pred("shiny", vec![konst("x")]),
            pred("round", vec![konst("x")]),
        ),
    };
    let shiny_a = pred("shiny", vec![konst("A")]);
    let round_a = pred("round", vec![konst("A")]);
    let glorp_a = pred("glorp", vec![konst("A")]);

    let result = prove_certify_check_with_defs(&[shiny_a, round_a], &glorp_a, &[def]);
    assert!(
        result.verified,
        "engine should prove glorp(A) by expanding the definition for search: {:?}",
        result.verification_error
    );
}

/// Stride 4 — a definition whose definiens uses ANOTHER definition proves
/// end-to-end: `great(x) :↔ happy(x) ∧ tall(x)`, `happy(x) :↔ shiny(x) ∧ round(x)`.
/// `great(A)` unfolds transitively (great → happy → primitives) and is proved
/// from `shiny(A)`, `round(A)`, `tall(A)`.
#[test]
fn definition_using_another_definition_proves() {
    let happy = Definition {
        name: "happy".to_string(),
        params: vec!["x".to_string()],
        definiens: and(
            pred("shiny", vec![konst("x")]),
            pred("round", vec![konst("x")]),
        ),
    };
    let great = Definition {
        name: "great".to_string(),
        params: vec!["x".to_string()],
        definiens: and(
            pred("happy", vec![konst("x")]),
            pred("tall", vec![konst("x")]),
        ),
    };
    let premises = [
        pred("shiny", vec![konst("A")]),
        pred("round", vec![konst("A")]),
        pred("tall", vec![konst("A")]),
    ];
    let goal = pred("great", vec![konst("A")]);

    let result = prove_certify_check_with_defs(&premises, &goal, &[happy, great]);
    assert!(
        result.verified,
        "great(A) should prove by unfolding great → happy → primitives: {:?}",
        result.verification_error
    );
}

/// Stride 4 — mutually recursive definitions (`ping(x) :↔ pong(x)`,
/// `pong(x) :↔ ping(x)`) form a cycle: δ-unfolding never terminates, so the
/// pair is rejected up front with a clear circular-definition error (not a
/// silent fuel-capped search failure).
#[test]
fn mutually_recursive_definitions_are_rejected() {
    let ping = Definition {
        name: "ping".to_string(),
        params: vec!["x".to_string()],
        definiens: pred("pong", vec![konst("x")]),
    };
    let pong = Definition {
        name: "pong".to_string(),
        params: vec!["x".to_string()],
        definiens: pred("ping", vec![konst("x")]),
    };
    let goal = pred("ping", vec![konst("A")]);

    let result = prove_certify_check_with_defs(&[], &goal, &[ping, pong]);
    assert!(!result.verified, "a mutually-recursive pair must not verify");
    let msg = result.verification_error.unwrap_or_default().to_lowercase();
    assert!(
        msg.contains("circular"),
        "expected a circular-definition error, got: {msg}"
    );
}

/// Stride 4 (Rung 0b seed) — the dependency graph records `uses` edges: def→def
/// (`great` uses `happy`) and theorem→def (the goal uses `great`). This is the
/// graph that mathscrapes nodes/edges compile into.
#[test]
fn dependency_graph_records_uses_edges() {
    let happy = Definition {
        name: "happy".to_string(),
        params: vec!["x".to_string()],
        definiens: and(
            pred("shiny", vec![konst("x")]),
            pred("round", vec![konst("x")]),
        ),
    };
    let great = Definition {
        name: "great".to_string(),
        params: vec!["x".to_string()],
        definiens: and(
            pred("happy", vec![konst("x")]),
            pred("tall", vec![konst("x")]),
        ),
    };
    let goal = pred("great", vec![konst("A")]);

    let graph = dependency_graph(&[happy, great], &[], &goal);

    let uses_of = |name: &str| -> Vec<String> {
        graph
            .def_uses
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, u)| u.clone())
            .unwrap_or_default()
    };
    assert_eq!(uses_of("great"), vec!["happy".to_string()], "great uses happy");
    assert!(uses_of("happy").is_empty(), "happy uses only primitives");
    assert_eq!(
        graph.theorem_uses,
        vec!["great".to_string()],
        "the theorem uses great"
    );
}

/// Rung 0 (quantified definiens) — a definition whose body is EXISTENTIAL is
/// proved by witness. `grounded(x) :↔ ∃y. supports(y, x)`; `grounded(A)` follows
/// from `supports(B, A)` with witness `y := B`. This exercises δ-expansion of a
/// quantified definiens (the bound `y` survives; only the param `x` is replaced)
/// plus the engine's existential introduction.
#[test]
fn existential_definition_proves_by_witness() {
    let def = Definition {
        name: "grounded".to_string(),
        params: vec!["x".to_string()],
        definiens: ProofExpr::Exists {
            variable: "y".to_string(),
            body: Box::new(pred(
                "supports",
                vec![ProofTerm::Variable("y".to_string()), konst("x")],
            )),
        },
    };
    let premise = pred("supports", vec![konst("B"), konst("A")]);
    let goal = pred("grounded", vec![konst("A")]);

    let result = prove_certify_check_with_defs(&[premise], &goal, &[def]);
    assert!(
        result.verified,
        "grounded(A) should prove with witness B: {:?}",
        result.verification_error
    );
}

/// Rung 0 (quantified definiens) — a UNIVERSAL definiens used as a premise is
/// instantiated. `everywhere(x) :↔ ∀y. at(x, y)`; from `everywhere(A)` (which
/// unfolds to `∀y. at(A, y)`) we instantiate `at(A, C)`.
#[test]
fn universal_definition_in_premise_instantiates() {
    let def = Definition {
        name: "everywhere".to_string(),
        params: vec!["x".to_string()],
        definiens: ProofExpr::ForAll {
            variable: "y".to_string(),
            body: Box::new(pred(
                "at",
                vec![konst("x"), ProofTerm::Variable("y".to_string())],
            )),
        },
    };
    let premise = pred("everywhere", vec![konst("A")]);
    let goal = pred("at", vec![konst("A"), konst("C")]);

    let result = prove_certify_check_with_defs(&[premise], &goal, &[def]);
    assert!(
        result.verified,
        "at(A, C) should follow from everywhere(A) by instantiation: {:?}",
        result.verification_error
    );
}

/// Rung 0 (capture safety) — an inner binder that SHADOWS a parameter must not
/// be captured by substitution. `foo(y) :↔ ∃y. near(y)`: the param `y` is hidden
/// by the existential's own `y`, so `foo(A)` unfolds to `∃y. near(y)` (proved
/// from `near(B)`), NOT the captured `∃y. near(A)` (which `near(B)` can't prove).
#[test]
fn parameter_shadowed_by_inner_binder_is_not_captured() {
    let def = Definition {
        name: "foo".to_string(),
        params: vec!["y".to_string()],
        definiens: ProofExpr::Exists {
            variable: "y".to_string(),
            body: Box::new(pred("near", vec![ProofTerm::Variable("y".to_string())])),
        },
    };
    let premise = pred("near", vec![konst("B")]);
    let goal = pred("foo", vec![konst("A")]);

    let result = prove_certify_check_with_defs(&[premise], &goal, &[def]);
    assert!(
        result.verified,
        "foo(A) should unfold to ∃y.near(y) (shadowed param, not captured): {:?}",
        result.verification_error
    );
}

/// Rung 0 (disjunctive definiens) — `colorful(x) :↔ red(x) ∨ blue(x)` is proved
/// from just one disjunct: `colorful(A)` follows from `red(A)` via ∨-introduction
/// on the unfolded definition.
#[test]
fn disjunctive_definition_proves_via_one_disjunct() {
    let def = Definition {
        name: "colorful".to_string(),
        params: vec!["x".to_string()],
        definiens: ProofExpr::Or(
            Box::new(pred("red", vec![konst("x")])),
            Box::new(pred("blue", vec![konst("x")])),
        ),
    };
    let premise = pred("red", vec![konst("A")]);
    let goal = pred("colorful", vec![konst("A")]);

    let result = prove_certify_check_with_defs(&[premise], &goal, &[def]);
    assert!(
        result.verified,
        "colorful(A) should prove from red(A) via the left disjunct: {:?}",
        result.verification_error
    );
}

/// A self-referential definiens (`loop(x) :↔ loop(x)`) is rejected up front:
/// δ-unfolding would not terminate. The result is an honest verification failure
/// with a clear message — never a hang or a false proof.
#[test]
fn recursive_definition_is_rejected() {
    let def = Definition {
        name: "loop".to_string(),
        params: vec!["x".to_string()],
        definiens: pred("loop", vec![konst("x")]),
    };
    let goal = pred("loop", vec![konst("a")]);

    let result = prove_certify_check_with_defs(&[], &goal, &[def]);
    assert!(!result.verified, "a recursive definition must not verify");
    let msg = result.verification_error.unwrap_or_default().to_lowercase();
    assert!(
        msg.contains("recurs"),
        "expected a recursion error, got: {msg}"
    );
}
