//! Generic structural induction over ANY user-declared inductive type
//! (`InferenceRule::InductionScheme`), kernel-certified.
//!
//! The legacy `StructuralInduction` rule is fixed to the Nat shape — one nullary
//! base constructor and one unary recursive constructor — and the engine gates it
//! on a hardcoded `matches!(typename, "Nat" | "List")`. These tests pin the
//! generalization: an arbitrary constructor set (more than two constructors;
//! constructors with several recursive arguments, each contributing its own
//! induction hypothesis), certified to a `Fix` over an N-ary `Match` — the
//! dependent eliminator the kernel re-checks for coverage, case types, and
//! termination. Soundness is therefore kernel-enforced: an ill-formed scheme fails
//! `infer_type`, it does not admit a false theorem.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, Context, Term, Universe};
use logicaffeine_proof::certifier::{certify, CertificationContext};
use logicaffeine_proof::{
    DerivationTree, InductionArg, InductionCase, InferenceRule, ProofExpr, ProofTerm,
};

// --- kernel term helpers ---
fn ind(name: &str) -> Term {
    Term::Global(name.to_string())
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn pi(param: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: param.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn kvar(n: &str) -> Term {
    Term::Var(n.to_string())
}

// --- proof-expr helpers ---
fn pred(name: &str, arg: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.to_string(),
        args: vec![arg],
        world: None,
    }
}
fn pvar(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn pconst(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn named(hyp: &str) -> DerivationTree {
    DerivationTree::leaf(ProofExpr::Atom(hyp.to_string()), InferenceRule::PremiseMatch)
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}
fn forall(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll {
        variable: var.to_string(),
        body: Box::new(body),
    }
}

/// Generalization beyond the two-constructor Nat shape: a three-constructor enum.
/// `⊢ ∀c:Light. P(c)` from `P(Red)`, `P(Yellow)`, `P(Green)` — a Match with three
/// cases the legacy single-step path cannot express.
#[test]
fn induction_over_three_constructor_enum_covers_all_cases() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Light : Type 0  with  Red, Yellow, Green : Light
    ctx.add_inductive("Light", Term::Sort(Universe::Type(0)));
    for c in ["Red", "Yellow", "Green"] {
        ctx.add_constructor(c, "Light", ind("Light"));
    }
    // P : Light -> Prop, and a hypothesis P(c) per constructor
    ctx.add_declaration("P", pi("_", ind("Light"), prop()));
    ctx.add_declaration("hRed", app(ind("P"), ind("Red")));
    ctx.add_declaration("hYellow", app(ind("P"), ind("Yellow")));
    ctx.add_declaration("hGreen", app(ind("P"), ind("Green")));

    let tree = DerivationTree::new(
        forall("c", pred("P", pvar("c"))),
        InferenceRule::InductionScheme {
            variable: "c".to_string(),
            ind_type: "Light".to_string(),
            cases: vec![
                InductionCase { constructor: "Red".to_string(), args: vec![] },
                InductionCase { constructor: "Yellow".to_string(), args: vec![] },
                InductionCase { constructor: "Green".to_string(), args: vec![] },
            ],
        },
        vec![named("hRed"), named("hYellow"), named("hGreen")],
    );

    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("generic induction over Light should certify");
    let inferred =
        infer_type(&ctx, &term).expect("the certified eliminator must type-check in the kernel");
    assert!(
        matches!(inferred, Term::Pi { .. }),
        "expected ∀c:Light. P(c), got {}",
        inferred
    );
}

/// The real generalization: a constructor with TWO recursive arguments, each
/// carrying its own induction hypothesis. `⊢ ∀t:Tree. P(t)` where the `Node l r`
/// case consumes `P(l)` and `P(r)` — both must become recursive calls `rec_t l`
/// and `rec_t r` for the term to certify and type-check.
#[test]
fn induction_over_binary_tree_threads_both_hypotheses() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Tree : Type 0  with  Leaf : Tree,  Node : Tree -> Tree -> Tree
    ctx.add_inductive("Tree", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("Leaf", "Tree", ind("Tree"));
    ctx.add_constructor(
        "Node",
        "Tree",
        pi("l", ind("Tree"), pi("r", ind("Tree"), ind("Tree"))),
    );

    // P : Tree -> Prop
    ctx.add_declaration("P", pi("_", ind("Tree"), prop()));
    // hLeaf : P Leaf
    ctx.add_declaration("hLeaf", app(ind("P"), ind("Leaf")));
    // hNode : Π(l:Tree)(r:Tree). P l -> P r -> P (Node l r)
    ctx.add_declaration(
        "hNode",
        pi(
            "l",
            ind("Tree"),
            pi(
                "r",
                ind("Tree"),
                pi(
                    "_",
                    app(ind("P"), kvar("l")),
                    pi(
                        "_",
                        app(ind("P"), kvar("r")),
                        app(ind("P"), app(app(ind("Node"), kvar("l")), kvar("r"))),
                    ),
                ),
            ),
        ),
    );

    // P(Node l r), and the two induction hypotheses (resolved to rec_t l / rec_t r)
    let p_node = pred(
        "P",
        ProofTerm::Function("Node".to_string(), vec![pvar("l"), pvar("r")]),
    );
    let ih_l = DerivationTree::leaf(pred("P", pvar("l")), InferenceRule::PremiseMatch);
    let ih_r = DerivationTree::leaf(pred("P", pvar("r")), InferenceRule::PremiseMatch);

    // hNode l : ∀r. P l -> P r -> P(Node l r)
    let hnode_l = DerivationTree::new(
        forall(
            "r",
            implies(
                pred("P", pvar("l")),
                implies(pred("P", pvar("r")), p_node.clone()),
            ),
        ),
        InferenceRule::UniversalInst("l".to_string()),
        vec![named("hNode")],
    );
    // hNode l r : P l -> P r -> P(Node l r)
    let hnode_lr = DerivationTree::new(
        implies(
            pred("P", pvar("l")),
            implies(pred("P", pvar("r")), p_node.clone()),
        ),
        InferenceRule::UniversalInst("r".to_string()),
        vec![hnode_l],
    );
    // hNode l r (rec_t l) : P r -> P(Node l r)
    let app_ihl = DerivationTree::new(
        implies(pred("P", pvar("r")), p_node.clone()),
        InferenceRule::ModusPonens,
        vec![hnode_lr, ih_l],
    );
    // hNode l r (rec_t l) (rec_t r) : P(Node l r)
    let node_proof = DerivationTree::new(
        p_node.clone(),
        InferenceRule::ModusPonens,
        vec![app_ihl, ih_r],
    );

    let tree = DerivationTree::new(
        forall("t", pred("P", pvar("t"))),
        InferenceRule::InductionScheme {
            variable: "t".to_string(),
            ind_type: "Tree".to_string(),
            cases: vec![
                InductionCase { constructor: "Leaf".to_string(), args: vec![] },
                InductionCase {
                    constructor: "Node".to_string(),
                    args: vec![
                        InductionArg { name: "l".to_string(), recursive: true },
                        InductionArg { name: "r".to_string(), recursive: true },
                    ],
                },
            ],
        },
        vec![named("hLeaf"), node_proof],
    );

    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("generic induction over Tree should certify");

    // Both recursive subtrees must be visited: rec_t l AND rec_t r.
    let rec_calls = format!("{}", term).matches("rec_t").count();
    assert!(
        rec_calls >= 2,
        "expected two induction-hypothesis recursive calls, found {}: {}",
        rec_calls,
        term
    );

    let inferred =
        infer_type(&ctx, &term).expect("the certified Tree eliminator must type-check");
    assert!(
        matches!(inferred, Term::Pi { .. }),
        "expected ∀t:Tree. P(t), got {}",
        inferred
    );
}

/// Soundness: a scheme that omits a constructor builds a non-exhaustive `Match`,
/// which the kernel's coverage check must REJECT. The de Bruijn safety net — a
/// bad eliminator never becomes a theorem.
#[test]
fn induction_missing_a_case_fails_kernel_coverage() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx.add_inductive("Light", Term::Sort(Universe::Type(0)));
    for c in ["Red", "Yellow", "Green"] {
        ctx.add_constructor(c, "Light", ind("Light"));
    }
    ctx.add_declaration("P", pi("_", ind("Light"), prop()));
    ctx.add_declaration("hRed", app(ind("P"), ind("Red")));
    ctx.add_declaration("hYellow", app(ind("P"), ind("Yellow")));

    // Only Red and Yellow — Green is omitted.
    let tree = DerivationTree::new(
        forall("c", pred("P", pvar("c"))),
        InferenceRule::InductionScheme {
            variable: "c".to_string(),
            ind_type: "Light".to_string(),
            cases: vec![
                InductionCase { constructor: "Red".to_string(), args: vec![] },
                InductionCase { constructor: "Yellow".to_string(), args: vec![] },
            ],
        },
        vec![named("hRed"), named("hYellow")],
    );

    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("the term itself builds");
    assert!(
        infer_type(&ctx, &term).is_err(),
        "a Match missing the Green case must fail the kernel's coverage check"
    );
}
