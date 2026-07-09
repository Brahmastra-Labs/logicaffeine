//! `crush` — the grind-style closer: E-match `∀`-equality lemmas into the
//! ground e-graph, then close by certified congruence. Every proof is
//! kernel-checked (`verified == true` is the only acceptance), and each test
//! targets something plain `auto` cannot do.

use logicaffeine_proof::tactic::ProofState;
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn k(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn v(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn f(name: &str, args: Vec<ProofTerm>) -> ProofTerm {
    ProofTerm::Function(name.to_string(), args)
}
fn pr(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args, world: None }
}
fn eq(l: ProofTerm, r: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(l, r)
}
fn forall(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll { variable: var.to_string(), body: Box::new(body) }
}

#[test]
fn crush_signature_grind_demo() {
    // ∀x. f(x)=g(x), a=b, P(g(b)) ⊢ P(f(a)). Needs instantiation at x=a,
    // congruence g(a)=g(b) from a=b, then a rewrite — the demo `auto` fails.
    let lemma = forall("x", eq(f("f", vec![v("x")]), f("g", vec![v("x")])));
    let mut st = ProofState::start(
        vec![lemma, eq(k("A"), k("B")), pr("P", vec![f("g", vec![k("B")])])],
        pr("P", vec![f("f", vec![k("A")])]),
    );
    st.crush().unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "signature grind demo: {:?}", r.verification_error);
}

#[test]
fn crush_pure_instantiation_equality_goal() {
    // ∀x. f(x)=g(x) ⊢ f(A)=g(A): a single E-match instance closes the goal.
    let lemma = forall("x", eq(f("f", vec![v("x")]), f("g", vec![v("x")])));
    let mut st = ProofState::start(vec![lemma], eq(f("f", vec![k("A")]), f("g", vec![k("A")])));
    st.crush().unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "instantiation equality: {:?}", r.verification_error);
}

#[test]
fn crush_congruence_through_equality_chain() {
    // f(A)=g(A), A=B, g(B)=h(C) ⊢ f(A)=h(C): pure congruence closure (no
    // lemma), the transitive chain the e-graph explains.
    let mut st = ProofState::start(
        vec![
            eq(f("f", vec![k("A")]), f("g", vec![k("A")])),
            eq(k("A"), k("B")),
            eq(f("g", vec![k("B")]), f("h", vec![k("C")])),
        ],
        eq(f("f", vec![k("A")]), f("h", vec![k("C")])),
    );
    st.crush().unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "congruence chain: {:?}", r.verification_error);
}

#[test]
fn crush_predicate_rewrite_via_instance() {
    // ∀x. h(x)=C, P(C) ⊢ P(h(A)): instantiate h(A)=C, rewrite P(C) → P(h(A)).
    let lemma = forall("x", eq(f("h", vec![v("x")]), k("C")));
    let mut st = ProofState::start(
        vec![lemma, pr("P", vec![k("C")])],
        pr("P", vec![f("h", vec![k("A")])]),
    );
    st.crush().unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "predicate rewrite: {:?}", r.verification_error);
}

#[test]
fn crush_declines_when_no_instance_helps() {
    // ∀x. f(x)=g(x), goal Q(A) unrelated: crush cannot manufacture Q(A).
    let lemma = forall("x", eq(f("f", vec![v("x")]), f("g", vec![v("x")])));
    let mut st = ProofState::start(vec![lemma], pr("Q", vec![k("A")]));
    assert!(st.crush().is_err(), "crush must decline an unreachable goal");
}

#[test]
fn crush_beats_auto_on_signature_demo() {
    // Head-to-head: `auto` fails the signature demo, `crush` proves it.
    let lemma = forall("x", eq(f("f", vec![v("x")]), f("g", vec![v("x")])));
    let build = || {
        ProofState::start(
            vec![lemma.clone(), eq(k("A"), k("B")), pr("P", vec![f("g", vec![k("B")])])],
            pr("P", vec![f("f", vec![k("A")])]),
        )
    };
    let mut auto_st = build();
    // Whatever `auto` does with this goal, it is NOT a kernel-certified proof:
    // without the Z3 oracle the structural search fails outright; with it, `auto`
    // closes only via an `OracleVerification` leaf the kernel refuses to certify
    // (certifier.rs — oracle results attest satisfiability, produce no proof term).
    let auto_closed = auto_st.auto().is_ok();
    let auto_certified = auto_closed && auto_st.qed().map_or(false, |r| r.verified);
    assert!(!auto_certified, "auto must not KERNEL-CERTIFY the grind demo");
    let mut crush_st = build();
    crush_st.crush().unwrap();
    assert!(
        crush_st.qed().unwrap().verified,
        "crush proves what auto cannot — with a kernel-certified proof",
    );
}

#[test]
fn script_crush_prose_works() {
    let lemma = forall("x", eq(f("f", vec![v("x")]), f("g", vec![v("x")])));
    let mut st = ProofState::start(
        vec![lemma, eq(k("A"), k("B")), pr("P", vec![f("g", vec![k("B")])])],
        pr("P", vec![f("f", vec![k("A")])]),
    );
    st.run_script("Grind.").unwrap();
    assert!(st.qed().unwrap().verified, "Grind. prose must certify");
}
