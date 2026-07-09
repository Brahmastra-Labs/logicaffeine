//! `simp` — rewrite-rule-set simplification to fixpoint, kernel-certified.
//!
//! Every test drives the SAME trust door as manual tactics: the steps simp
//! records assemble into a `DerivationTree`, `qed` certifies it, and the
//! kernel re-checks the term. `verified == true` is the only acceptance.

use logicaffeine_proof::simp::SimpSet;
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
fn p(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args, world: None }
}
fn eq(l: ProofTerm, r: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(l, r)
}
fn forall(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll { variable: var.to_string(), body: Box::new(body) }
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}
fn iff(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Iff(Box::new(l), Box::new(r))
}

/// A SimpSet from premise-sourced lemmas (each must already be a premise of
/// the proof state, so its PremiseMatch leaf certifies).
fn set_of(lemmas: &[ProofExpr]) -> SimpSet {
    let mut set = SimpSet::new();
    for (i, lemma) in lemmas.iter().enumerate() {
        assert!(
            set.register_lemma(&format!("rule{i}"), lemma),
            "lemma {lemma} must register as a simp rule"
        );
    }
    set
}

#[test]
fn universal_inst_term_certifies_alone() {
    // The new instantiation rule in isolation: ∀x. join(x, e) = x, one
    // UniversalInstTerm node at witness `a`, kernel-checked.
    use logicaffeine_proof::verify::check_derivation;
    use logicaffeine_proof::{DerivationTree, InferenceRule};
    let lemma = forall("x", eq(f("join", vec![v("x"), k("E")]), v("x")));
    let inst = eq(f("join", vec![k("A"), k("E")]), k("A"));
    let tree = DerivationTree::new(
        inst.clone(),
        InferenceRule::UniversalInstTerm(k("A")),
        vec![DerivationTree::leaf(lemma.clone(), InferenceRule::PremiseMatch)],
    );
    let r = check_derivation(&[lemma], &inst, tree);
    assert!(r.verified, "bare UniversalInstTerm: {:?}", r.verification_error);
}

#[test]
fn universal_inst_term_certifies_at_compound_witness() {
    use logicaffeine_proof::verify::check_derivation;
    use logicaffeine_proof::{DerivationTree, InferenceRule};
    let lemma = forall("x", eq(f("join", vec![v("x"), k("E")]), v("x")));
    let witness = f("f", vec![k("B")]);
    let inst = eq(f("join", vec![witness.clone(), k("E")]), witness.clone());
    let tree = DerivationTree::new(
        inst.clone(),
        InferenceRule::UniversalInstTerm(witness),
        vec![DerivationTree::leaf(lemma.clone(), InferenceRule::PremiseMatch)],
    );
    let r = check_derivation(&[lemma], &inst, tree);
    assert!(r.verified, "compound UniversalInstTerm: {:?}", r.verification_error);
}

#[test]
fn universal_inst_term_certifies_at_int_witness() {
    // The Int-domain twin of the bare probe: ∀x. add(x, 0) = x at witness 7.
    use logicaffeine_proof::verify::check_derivation;
    use logicaffeine_proof::{DerivationTree, InferenceRule};
    let lemma = forall("x", eq(f("add", vec![v("x"), k("0")]), v("x")));
    let inst = eq(f("add", vec![k("7"), k("0")]), k("7"));
    let tree = DerivationTree::new(
        inst.clone(),
        InferenceRule::UniversalInstTerm(k("7")),
        vec![DerivationTree::leaf(lemma.clone(), InferenceRule::PremiseMatch)],
    );
    let r = check_derivation(&[lemma], &inst, tree);
    assert!(r.verified, "Int UniversalInstTerm: {:?}", r.verification_error);
}

#[test]
fn simp_rewrites_to_refl() {
    // ∀x. join(x, e) = x  ⊢  join(a, e) = a, by one rewrite + reflexivity.
    let lemma = forall("x", eq(f("join", vec![v("x"), k("E")]), v("x")));
    let goal = eq(f("join", vec![k("A"), k("E")]), k("A"));
    let set = set_of(std::slice::from_ref(&lemma));
    let mut st = ProofState::start(vec![lemma], goal);
    st.simp(&set).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "simp join_unit: {:?}", r.verification_error);
}

#[test]
fn simp_rewrites_int_lemma_at_int_witness() {
    // The arithmetic-typed twin: ∀x. add(x, 0) = x at the Int witness 7.
    let lemma = forall("x", eq(f("add", vec![v("x"), k("0")]), v("x")));
    let goal = eq(f("add", vec![k("7"), k("0")]), k("7"));
    let set = set_of(std::slice::from_ref(&lemma));
    let mut st = ProofState::start(vec![lemma], goal);
    st.simp(&set).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "simp add_zero: {:?}", r.verification_error);
}

#[test]
fn simp_instantiates_at_compound_terms() {
    // The witness is f(b), not a bare name — instantiation must handle
    // arbitrary terms. ∀x. join(x, e) = x ⊢ join(f(b), e) = f(b).
    let lemma = forall("x", eq(f("join", vec![v("x"), k("E")]), v("x")));
    let goal = eq(
        f("join", vec![f("f", vec![k("B")]), k("E")]),
        f("f", vec![k("B")]),
    );
    let set = set_of(std::slice::from_ref(&lemma));
    let mut st = ProofState::start(vec![lemma], goal);
    st.simp(&set).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "compound witness: {:?}", r.verification_error);
}

#[test]
fn simp_chains_rules_to_fixpoint() {
    // ∀x. f(x) = g(x);  ∀x. g(x) = c;  P(c)  ⊢  P(f(a)).
    let r1 = forall("x", eq(f("f", vec![v("x")]), f("g", vec![v("x")])));
    let r2 = forall("x", eq(f("g", vec![v("x")]), k("C")));
    let fact = p("P", vec![k("C")]);
    let goal = p("P", vec![f("f", vec![k("A")])]);
    let set = set_of(&[r1.clone(), r2.clone()]);
    let mut st = ProofState::start(vec![r1, r2, fact], goal);
    st.simp(&set).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "chained rewrites: {:?}", r.verification_error);
}

#[test]
fn simp_conditional_rule_discharges_side_goal() {
    // ∀x. P(x) → f(x) = c;  P(a)  ⊢  f(a) = c.
    let rule = forall(
        "x",
        implies(p("P", vec![v("x")]), eq(f("f", vec![v("x")]), k("C"))),
    );
    let fact = p("P", vec![k("A")]);
    let goal = eq(f("f", vec![k("A")]), k("C"));
    let set = set_of(std::slice::from_ref(&rule));
    let mut st = ProofState::start(vec![rule, fact], goal);
    st.simp(&set).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "conditional rewrite: {:?}", r.verification_error);
}

#[test]
fn simp_ground_arithmetic_folds() {
    // ⊢ add(2, 3) = 5 — no rules at all; the ground fold closes it.
    let goal = eq(f("add", vec![k("2"), k("3")]), k("5"));
    let set = SimpSet::new();
    let mut st = ProofState::start(vec![], goal);
    st.simp(&set).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "ground fold: {:?}", r.verification_error);
}

#[test]
fn simp_terminates_on_commuting_rules() {
    // a = b and b = a registered together: simp must return (seen-set/budget),
    // never hang, and never loop the goal forever.
    let r1 = eq(k("A"), k("B"));
    let r2 = eq(k("B"), k("A"));
    let goal = p("P", vec![k("A")]);
    let set = set_of(&[r1.clone(), r2.clone()]);
    let mut st = ProofState::start(vec![r1, r2], goal);
    // Progress is made (at least one rewrite fires), so simp itself succeeds;
    // the goal stays open in some flipped form.
    st.simp(&set).unwrap();
    assert_eq!(st.open_goals(), 1, "commuting rules leave exactly one open goal");
}

#[test]
fn simp_leaves_normalized_goal_open() {
    // Only ∀x. f(x) = g(x): P(f(a)) normalizes to P(g(a)) and stays open.
    let rule = forall("x", eq(f("f", vec![v("x")]), f("g", vec![v("x")])));
    let goal = p("P", vec![f("f", vec![k("A")])]);
    let set = set_of(std::slice::from_ref(&rule));
    let mut st = ProofState::start(vec![rule], goal);
    st.simp(&set).unwrap();
    assert_eq!(st.open_goals(), 1);
    assert_eq!(
        st.focused_target().expect("one open goal"),
        &p("P", vec![f("g", vec![k("A")])]),
        "goal must be left in normalized form"
    );
}

#[test]
fn simp_makes_no_progress_is_an_error() {
    // No rule touches the goal and it is not closable: simp reports
    // DoesNotApply rather than silently succeeding.
    let rule = forall("x", eq(f("f", vec![v("x")]), k("C")));
    let goal = p("Q", vec![k("Z")]);
    let set = set_of(std::slice::from_ref(&rule));
    let mut st = ProofState::start(vec![rule], goal);
    assert!(st.simp(&set).is_err(), "no-progress simp must fail loudly");
}

#[test]
fn simp_iff_rule_rewrites_prop() {
    // ∀x. Q(x) ↔ R(x);  R(a)  ⊢  Q(a): the iff instantiates at the whole goal,
    // reducing it to R(a), which the fact closes.
    let rule = forall("x", iff(p("Q", vec![v("x")]), p("R", vec![v("x")])));
    let fact = p("R", vec![k("A")]);
    let goal = p("Q", vec![k("A")]);
    let set = set_of(std::slice::from_ref(&rule));
    let mut st = ProofState::start(vec![rule, fact], goal);
    st.simp(&set).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "iff rewrite: {:?}", r.verification_error);
}

#[test]
fn simp_rejects_unoriented_rules() {
    let mut set = SimpSet::new();
    // rhs introduces a variable the lhs does not bind — not a rewrite rule.
    let bad = forall("x", forall("y", eq(f("f", vec![v("x")]), f("g", vec![v("y")]))));
    assert!(!set.register_lemma("bad", &bad));
    // A bare variable lhs would match everything.
    let trivial = forall("x", eq(v("x"), k("C")));
    assert!(!set.register_lemma("trivial", &trivial));
    // A condition mentioning a variable the lhs does not determine cannot be
    // instantiated by matching.
    let dangling = forall(
        "x",
        forall(
            "y",
            implies(p("P", vec![v("y")]), eq(f("f", vec![v("x")]), k("C"))),
        ),
    );
    assert!(!set.register_lemma("dangling", &dangling));
}

#[test]
fn script_simplify_prose_works() {
    // The English surface: `Simplify.` compiles to simp over the in-scope
    // premises (the rule and the fact are both hypotheses).
    let rule = forall("x", eq(f("f", vec![v("x")]), k("C")));
    let fact = p("P", vec![k("C")]);
    let goal = p("P", vec![f("f", vec![k("A")])]);
    let mut st = ProofState::start(vec![rule, fact], goal);
    st.run_script("Simplify.").unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "Simplify. prose: {:?}", r.verification_error);
}

#[test]
fn scripted_library_simp_pool_reaches_untagged_citations() {
    // A proved `[simp]`-tagged theorem joins the pool: a LATER theorem's simp
    // uses it WITHOUT citing it.
    use logicaffeine_proof::tactic_script::{prove_scripted_library, ScriptedTheorem};
    let rule = forall("x", eq(f("f", vec![v("x")]), k("C")));
    let lemma = ScriptedTheorem {
        name: "f_collapses".to_string(),
        premises: vec![rule.clone()],
        goal: rule,
        script: "assumption".to_string(),
        cites: vec![],
        simp: true,
    };
    let user = ScriptedTheorem {
        name: "uses_pool".to_string(),
        premises: vec![p("P", vec![k("C")])],
        goal: p("P", vec![f("f", vec![k("A")])]),
        script: "simp".to_string(),
        cites: vec![], // deliberately NOT citing f_collapses
        simp: false,
    };
    let results = prove_scripted_library(&[], &[lemma, user]);
    assert!(results[0].verified, "lemma: {:?}", results[0].verification_error);
    assert!(
        results[1].verified,
        "pool user: {:?}",
        results[1].verification_error
    );
}

#[test]
fn development_simp_attribute_registers() {
    use logicaffeine_proof::development::parse_development;
    let dev = parse_development(
        "Axiom add_zero [simp]: for all x, Cong(x, x, x, x).\n\
         Axiom plain: for all a b, Cong(a, b, b, a).\n\
         Theorem t1 [simp] cites plain: prove for all a b, Cong(a, b, b, a).",
    )
    .expect("development parses");
    assert_eq!(dev.simp_lemmas(), &["add_zero".to_string(), "t1".to_string()]);
    assert_eq!(dev.axioms.len(), 2);
    assert_eq!(dev.theorems.len(), 1);
    assert_eq!(dev.theorems[0].cites, vec!["plain".to_string()]);
}

#[test]
fn simp_rewrites_inside_hypthesis_rich_goals() {
    // Rewriting must reach subterms under connectives: ∀x. h(x) = c ⊢
    // P(h(a)) ∧ Q(h(b)) → P(c) ∧ Q(c) — then closes from the two facts.
    let rule = forall("x", eq(f("h", vec![v("x")]), k("C")));
    let fp = p("P", vec![k("C")]);
    let fq = p("Q", vec![k("C")]);
    let goal = ProofExpr::And(
        Box::new(p("P", vec![f("h", vec![k("A")])])),
        Box::new(p("Q", vec![f("h", vec![k("B")])])),
    );
    let set = set_of(std::slice::from_ref(&rule));
    let mut st = ProofState::start(vec![rule, fp, fq], goal);
    st.simp(&set).unwrap();
    // simp normalizes to P(c) ∧ Q(c); split + assumption closes it.
    if st.open_goals() > 0 {
        st.split().unwrap();
        st.assumption().unwrap();
        st.assumption().unwrap();
    }
    let r = st.qed().unwrap();
    assert!(r.verified, "rewrite under ∧: {:?}", r.verification_error);
}
