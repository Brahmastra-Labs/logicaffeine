//! THE TACTIC SURFACE — the neighbours of every case in `lean_conformance_t08_tactics`.
//!
//! The corpus test for a case pins the one thing that case names. This file pins what a
//! WEAK implementation of that same case would also satisfy, so the pair together cannot be
//! passed by accident:
//!
//! * **The negative.** For everything the surface now accepts, the neighbouring thing it
//!   must still REFUSE — an ill-typed application, a witness that is not fresh, a `simp only`
//!   whose rule list genuinely excludes the rule the goal needs, a FALSE ring identity. A
//!   positive-only test is satisfiable by a tactic that says yes to everything.
//! * **The certification.** For every tactic made spellable, that its derivation is accepted
//!   by the KERNEL — `qed().verified`, not merely `run_script` returning `Ok`. A tactic that
//!   runs and hands back an uncertified derivation is worse than one that refuses, because
//!   the refusal is visible where the tactic failed and the bad derivation is not visible
//!   until the trust door, if anyone looks.
//! * **The regression guard.** The prose vocabulary that already worked (`by cases on h`,
//!   `rewrite with eq`) reading the same way after the argument reader stopped swallowing
//!   ordinary identifiers.

use logicaffeine_proof::tactic::{PTerm, ProofState};
use logicaffeine_proof::tactic_script::parse_script;
use logicaffeine_proof::verify::check_derivation;
use logicaffeine_proof::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

// ============================================================================================
// HELPERS — per-file, as house style requires.
// ============================================================================================

fn c(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn v(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn app(name: &str, args: Vec<ProofTerm>) -> ProofTerm {
    ProofTerm::Function(name.to_string(), args)
}
fn pred(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.to_string(),
        args,
        world: None,
    }
}
fn eq(l: ProofTerm, r: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(l, r)
}
fn and(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(l), Box::new(r))
}
fn or(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(l), Box::new(r))
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}
fn not(p: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(p))
}
fn forall(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll {
        variable: var.to_string(),
        body: Box::new(body),
    }
}
fn ex(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::Exists {
        variable: var.to_string(),
        body: Box::new(body),
    }
}
fn le(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    eq(app("le", vec![a, b]), c("true"))
}
fn p_of(t: ProofTerm) -> ProofExpr {
    pred("Phi", vec![t])
}
fn prop_p() -> ProofExpr {
    pred("p", vec![c("X")])
}
fn prop_q() -> ProofExpr {
    pred("q", vec![c("X")])
}
fn prop_r() -> ProofExpr {
    pred("r", vec![c("X")])
}

fn state(premises: Vec<(&str, ProofExpr)>, goal: ProofExpr) -> ProofState {
    let names: Vec<Option<String>> = premises
        .iter()
        .map(|(n, _)| Some((*n).to_string()))
        .collect();
    let props: Vec<ProofExpr> = premises.into_iter().map(|(_, p)| p).collect();
    ProofState::start_with_names(props, &names, goal)
}

/// Run `script` against `premises ⊢ goal` and report whether the KERNEL certified it.
fn prove(premises: Vec<(&str, ProofExpr)>, goal: ProofExpr, script: &str) -> Result<bool, String> {
    let mut st = state(premises, goal);
    st.run_script(script)
        .map_err(|e| format!("script: {}", e.0))?;
    st.qed()
        .map(|r| r.verified)
        .map_err(|e| format!("qed: {e:?}"))
}

/// True iff `tree` (or some node below it) was justified by `rule`'s discriminant.
fn uses_rule(tree: &DerivationTree, want: &InferenceRule) -> bool {
    if std::mem::discriminant(&tree.rule) == std::mem::discriminant(want) {
        return true;
    }
    tree.premises.iter().any(|p| uses_rule(p, want))
}

// ============================================================================================
// PROOF TERMS — `exact e` takes a term, so a term that proves the WRONG thing must be refused
// ============================================================================================

/// The negatives of an applied proof term. Every one of these parses; the elaborator has to
/// be the thing that says no, and it has to say no for the right reason.
#[test]
fn an_applied_proof_term_is_refused_when_the_application_is_ill_typed() {
    let premises = vec![
        ("hpq", implies(prop_p(), prop_q())),
        ("hp", prop_p()),
        ("hq", prop_q()),
    ];

    // The head is not a function of anything.
    let r = prove(premises.clone(), prop_q(), "exact (hp hpq)");
    assert!(
        r.is_err(),
        "applying a non-implication must be refused, never treated as the implication: {r:?}"
    );

    // The argument proves the wrong antecedent.
    let r = prove(premises.clone(), prop_q(), "exact (hpq hq)");
    assert!(
        r.is_err(),
        "`hpq : p → q` applied to `hq : q` has no antecedent match and must be refused: {r:?}"
    );

    // The result is a perfectly good proposition — just not the goal.
    let r = prove(premises.clone(), prop_r(), "exact (hpq hp)");
    assert!(
        r.is_err(),
        "a term that proves `q` must not close the goal `r`: {r:?}"
    );

    // An anonymous constructor against a goal with no binary introduction rule.
    let r = prove(premises.clone(), prop_q(), "exact (hp , hq)");
    assert!(
        r.is_err(),
        "an anonymous constructor against an atomic goal must be refused: {r:?}"
    );

    // A hole is `refine`'s business; `exact` must not quietly leave a goal open.
    let r = prove(premises, prop_q(), "exact ?_");
    assert!(
        r.is_err(),
        "`exact` must refuse a term with a hole in it: {r:?}"
    );
}

/// The positive, driven to the trust door — and the shape of the derivation, because the
/// elaborated application must be MODUS PONENS and not some looser "it matched" leaf.
#[test]
fn an_applied_proof_term_certifies_as_modus_ponens() {
    let premises = vec![("hpq", implies(prop_p(), prop_q())), ("hp", prop_p())];
    let mut st = state(premises, prop_q());
    st.run_script("exact (hpq hp)")
        .expect("the application elaborates");
    let tree = st.assembled().expect("the proof is complete");
    assert!(
        uses_rule(&tree, &InferenceRule::ModusPonens),
        "an applied proof term must emit ModusPonens, got {:?}",
        tree.rule
    );
    assert!(
        st.qed().expect("complete").verified,
        "the applied term must certify"
    );
}

/// A CHAIN of applications, and a universal instantiated by the same juxtaposition — the two
/// reasons Lean's `exact` takes a term rather than a name.
#[test]
fn a_proof_term_chains_applications_and_instantiates_a_universal() {
    let premises = vec![
        ("h1", implies(prop_p(), implies(prop_q(), prop_r()))),
        ("hp", prop_p()),
        ("hq", prop_q()),
    ];
    let r = prove(premises, prop_r(), "exact (h1 hp hq)");
    assert!(
        matches!(r, Ok(true)),
        "a two-argument application must certify: {r:?}"
    );

    // `h : ∀m. P m` closes `P n` after `intro n`, with no `have` and no restatement.
    let r = prove(
        vec![("h", forall("m", p_of(v("m"))))],
        forall("n", p_of(v("n"))),
        "intro n . exact h",
    );
    assert!(
        matches!(r, Ok(true)),
        "`exact h` must instantiate a ∀-hypothesis at the goal, as Lean's elaborator does: {r:?}"
    );
}

// ============================================================================================
// `refine` — one goal per hole, and nothing certified through a kernel term hole
// ============================================================================================

/// The count, the ORDER, and the certification. A `refine` that spawned the goals in the
/// wrong order would still close this proof if both components were interchangeable, so the
/// components are deliberately different propositions.
#[test]
fn refine_spawns_one_goal_per_hole_in_source_order_and_certifies() {
    let premises = vec![("hp", prop_p()), ("hq", prop_q())];
    let mut st = state(premises, and(prop_p(), prop_q()));
    st.run_script("refine (?_ , ?_)").expect("two holes");
    assert_eq!(st.open_goals(), 2, "one goal per hole");
    assert_eq!(
        st.focused_target(),
        Some(&prop_p()),
        "the FIRST hole's goal must be focused first, got {:?}",
        st.focused_target()
    );
    st.run_script("exact hp . exact hq")
        .expect("both holes discharge");
    assert!(
        st.qed().expect("complete").verified,
        "a refined proof must certify"
    );
}

/// A PARTIALLY applied refine: one component supplied, one left open. This is the shape
/// `refine` exists for, and the supplied half must not become a goal.
#[test]
fn refine_leaves_only_the_holes_open() {
    let premises = vec![("hp", prop_p()), ("hq", prop_q())];
    let mut st = state(premises, and(prop_p(), prop_q()));
    st.run_script("refine (hp , ?_)").expect("one hole");
    assert_eq!(st.open_goals(), 1, "only the hole is a goal");
    assert_eq!(
        st.focused_target(),
        Some(&prop_q()),
        "the open goal must be the HOLE's, got {:?}",
        st.focused_target()
    );
    st.run_script("exact hq").expect("the hole discharges");
    assert!(
        st.qed().expect("complete").verified,
        "a partial refine must certify"
    );
}

/// The negatives. A hole must never be a way to make a goal disappear.
#[test]
fn refine_refuses_a_shape_the_goal_does_not_have() {
    let premises = vec![("hp", prop_p()), ("hq", prop_q())];

    let mut st = state(premises.clone(), prop_p());
    assert!(
        st.run_script("refine (?_ , ?_)").is_err(),
        "an anonymous constructor against an atomic goal must be refused, not split"
    );

    // A bare hole is the identity on the goal, so accepting it would be a step that claims
    // to have done something and has not.
    assert!(
        parse_script("refine ?_").is_err(),
        "`refine ?_` leaves the goal exactly as it was and must not be a step"
    );

    // The supplied component still has to be right.
    let mut st = state(premises, and(prop_p(), prop_q()));
    assert!(
        st.run_script("refine (hq , ?_)").is_err(),
        "a supplied component that proves the wrong conjunct must be refused"
    );
}

// ============================================================================================
// `have` — a locally proved hypothesis, and one that proves only what it proves
// ============================================================================================

/// The certification, and the fact that the `have`d name really carries the PROOF: using it
/// twice emits the proof twice and both uses certify.
#[test]
fn a_have_carries_its_proof_to_every_use_and_certifies() {
    let premises = vec![("hpq", implies(prop_p(), prop_q())), ("hp", prop_p())];
    let r = prove(
        premises,
        and(prop_q(), prop_q()),
        "have hq from (hpq hp) . split . exact hq . exact hq",
    );
    assert!(
        matches!(r, Ok(true)),
        "a `have` proved by an applied term must certify at both uses: {r:?}"
    );
}

/// The negatives. `have` is the most dangerous step in the file, because a `have` that
/// accepted an unproved statement would let any proposition into scope.
#[test]
fn have_refuses_a_statement_it_cannot_prove() {
    let premises = vec![("hp", prop_p()), ("hq", prop_q())];

    let mut st = state(premises.clone(), prop_p());
    assert!(
        st.run_script("have h from nosuch").is_err(),
        "`have` from an absent name must fail, not introduce an unproved hypothesis"
    );

    let mut st = state(premises.clone(), prop_p());
    assert!(
        st.run_script("have h from (hp hq)").is_err(),
        "`have` from an ill-typed application must fail"
    );

    // The hypothesis it DOES introduce proves only its own proposition.
    let mut st = state(premises, prop_r());
    st.run_script("have hq2 from hq").expect("`hq` proves `q`");
    assert!(
        st.run_script("exact hq2").is_err(),
        "a `have`d `q` must not close the goal `r` — the name carries a proposition, not a \
         licence"
    );
}

// ============================================================================================
// `by_contra` — the step, its refusal, and the axiom footprint it must record
// ============================================================================================

/// The step's shape, and that going classical is RECORDED. The axiom footprint is part of
/// the theorem: a reductio that certified without naming `dne` would hide which axioms the
/// proof rests on.
#[test]
fn by_contra_assumes_the_negation_records_the_reductio_and_certifies() {
    let premises = vec![
        ("h", implies(not(prop_p()), prop_q())),
        ("hnq", not(prop_q())),
    ];
    let mut st = state(premises, prop_p());
    st.by_contra("hnp")
        .expect("by_contra applies to a positive goal");
    assert_eq!(
        st.focused_target(),
        Some(&ProofExpr::Atom("⊥".to_string())),
        "after `by_contra` the residual goal must be ⊥, got {:?}",
        st.focused_target()
    );
    let names: Vec<String> = st
        .focus()
        .expect("a goal is open")
        .hyps
        .iter()
        .map(|h| h.name.clone())
        .collect();
    assert!(
        names.iter().any(|n| n == "hnp"),
        "`by_contra hnp` must bind the negation under the name it was given: {names:?}"
    );
    st.run_script("auto")
        .expect("⊥ follows from ¬p, ¬p → q and ¬q");
    let tree = st.assembled().expect("the proof is complete");
    assert!(
        uses_rule(&tree, &InferenceRule::ClassicalReductio),
        "the derivation must record that it went classical: {:?}",
        tree.rule
    );
    assert!(
        st.qed().expect("complete").verified,
        "the reductio must certify"
    );
}

/// The refusal. `by_contra` on a `⊥` goal would assume `¬⊥` and leave `⊥` — the same goal,
/// with a useless hypothesis and one more classical axiom on the footprint.
#[test]
fn by_contra_declines_a_goal_that_is_already_absurdity() {
    let mut st = state(vec![("hp", prop_p())], ProofExpr::Atom("⊥".to_string()));
    assert!(
        st.by_contra("h").is_err(),
        "`by_contra` on a ⊥ goal makes no progress and must be declined"
    );
}

// ============================================================================================
// `ring` — the decision must be a DECISION, in both directions
// ============================================================================================

/// True identities across the vocabulary, each driven to the trust door.
#[test]
fn ring_certifies_the_identities_a_polynomial_normaliser_exists_for() {
    let cases: Vec<(&str, ProofExpr)> = vec![
        (
            "distributivity",
            eq(
                app("mul", vec![c("a"), app("add", vec![c("b"), c("cc")])]),
                app(
                    "add",
                    vec![
                        app("mul", vec![c("a"), c("b")]),
                        app("mul", vec![c("a"), c("cc")]),
                    ],
                ),
            ),
        ),
        (
            "commutativity",
            eq(
                app("mul", vec![c("a"), c("b")]),
                app("mul", vec![c("b"), c("a")]),
            ),
        ),
        (
            "subtraction cancels",
            eq(
                app("sub", vec![app("add", vec![c("a"), c("b")]), c("b")]),
                c("a"),
            ),
        ),
    ];
    let mut unproved = Vec::new();
    for (name, goal) in cases {
        let r = prove(vec![], goal, "ring");
        if !matches!(r, Ok(true)) {
            unproved.push(format!("{name}: {r:?}"));
        }
    }
    assert!(
        unproved.is_empty(),
        "every ring identity must be reachable from a script AND certify:\n  {}",
        unproved.join("\n  ")
    );
}

/// The negatives — a FALSE identity is the whole reason the tactic consults the oracle
/// rather than emitting the leaf and hoping. A tactic that said yes here would hand back a
/// derivation the kernel refuses, which is how a "proof" gets reported that is not one.
#[test]
fn ring_declines_what_is_not_an_identity() {
    let false_identity = eq(
        app("mul", vec![c("a"), app("add", vec![c("b"), c("cc")])]),
        app(
            "add",
            vec![
                app("mul", vec![c("a"), c("b")]),
                app("mul", vec![c("a"), c("b")]),
            ],
        ),
    );
    let mut st = state(vec![], false_identity);
    assert!(
        st.ring().is_err(),
        "`a(b+c) = ab + ab` is not a ring identity and must be DECLINED, never proved"
    );

    let mut st = state(vec![("hp", prop_p())], prop_p());
    assert!(
        st.ring().is_err(),
        "`ring` on a non-equality goal must be declined"
    );

    // And the decline is a decline all the way down: nothing was committed.
    let mut st = state(vec![], eq(c("a"), c("b")));
    assert!(st.ring().is_err(), "`a = b` is not an identity");
    assert_eq!(
        st.open_goals(),
        1,
        "a declined `ring` must leave the goal exactly as it was"
    );
}

// ============================================================================================
// `simp only` — the selection has to actually SELECT
// ============================================================================================

/// The positive and its certification, plus the negative that makes `only` mean something:
/// the SAME goal, with the rule it needs left out of the list, must fail.
#[test]
fn simp_only_uses_exactly_the_rules_it_names() {
    let h = eq(app("f", vec![c("A")]), c("A"));
    let other = eq(app("g", vec![c("B")]), c("B"));
    let goal = eq(app("f", vec![app("f", vec![c("A")])]), c("A"));

    let r = prove(
        vec![("h", h.clone()), ("k", other.clone())],
        goal.clone(),
        "simp only [h]",
    );
    assert!(
        matches!(r, Ok(true)),
        "`simp only [h]` must normalise and certify: {r:?}"
    );

    let mut st = state(vec![("h", h.clone()), ("k", other)], goal.clone());
    assert!(
        st.run_script("simp only [k]").is_err(),
        "`simp only [k]` must NOT reach for `h` — selection is the whole point of `only`, and \
         a set that silently widens collapses two normal forms into one"
    );

    // An unknown name is an error, not an empty rule set.
    let mut st = state(vec![("h", h)], goal);
    assert!(
        st.run_script("simp only [nosuch]").is_err(),
        "`simp only` on a name that is not in scope must fail rather than simplify with nothing"
    );
}

// ============================================================================================
// `cases … with` — the names, the arity, and the eigenvariable condition
// ============================================================================================

/// The ∧ and ∨ branches under chosen names, each certified.
#[test]
fn cases_with_binds_both_branches_under_chosen_names_and_certifies() {
    let r = prove(
        vec![("h", and(prop_p(), prop_q()))],
        and(prop_q(), prop_p()),
        "cases h with hp hq . split . exact hq . exact hp",
    );
    assert!(
        matches!(r, Ok(true)),
        "a named ∧-destructuring must certify: {r:?}"
    );

    let r = prove(
        vec![
            ("h", or(prop_p(), prop_q())),
            ("hpr", implies(prop_p(), prop_r())),
            ("hqr", implies(prop_q(), prop_r())),
        ],
        prop_r(),
        "cases h with hp hq . exact (hpr hp) . exact (hqr hq)",
    );
    assert!(
        matches!(r, Ok(true)),
        "a named ∨-case-split must certify: {r:?}"
    );
}

/// The negatives. The arity check catches a mistake about what `h` IS; the freshness check
/// is the eigenvariable condition that makes ∃-elimination sound at all.
#[test]
fn cases_with_checks_its_arity_and_the_witness_it_is_asked_to_name() {
    let mut st = state(vec![("h", and(prop_p(), prop_q()))], prop_p());
    assert!(
        st.cases_with("h", &["hp".to_string()]).is_err(),
        "a conjunction binds TWO names; one is a mistake about what `h` is"
    );

    let mut st = state(vec![("h", and(prop_p(), prop_q()))], prop_p());
    assert!(
        st.cases_with("h", &["a".to_string(), "b".to_string(), "cc".to_string()])
            .is_err(),
        "three names for a conjunction must be refused"
    );

    // The ∃ branch names its witness — but only a name nothing else in scope already uses.
    let mut st = state(vec![("h", ex("n", p_of(v("n"))))], pred("Q", vec![c("Sk")]));
    assert!(
        st.cases_with("h", &["Sk".to_string(), "hw".to_string()])
            .is_err(),
        "a witness name the GOAL already mentions is not fresh, and ∃-elimination is sound \
         only for a fresh one"
    );

    let mut st = state(vec![("h", ex("n", p_of(v("n"))))], prop_q());
    st.cases_with("h", &["W".to_string(), "hw".to_string()])
        .expect("a genuinely fresh witness name is accepted");
    let hyps = &st.focus().expect("a goal is open").hyps;
    assert!(
        hyps.iter()
            .any(|x| x.name == "hw" && x.prop == p_of(c("W"))),
        "the body must be bound under the chosen name at the chosen witness: {:?}",
        hyps.iter().map(|x| (&x.name, &x.prop)).collect::<Vec<_>>()
    );
}

// ============================================================================================
// `focus` — the block must close the goal it opened
// ============================================================================================

/// The refusal IS the capability: a block that leaves its goal open is an error, and the
/// state is untouched so a `try`/`first` around it sees nothing half-applied.
#[test]
fn a_focused_block_that_leaves_its_goal_open_is_an_error_and_commits_nothing() {
    let premises = vec![("hp", prop_p()), ("hq", prop_q())];
    let mut st = state(premises, and(prop_p(), and(prop_q(), prop_p())));
    st.run_script("split").expect("∧-introduction");
    assert_eq!(st.open_goals(), 2, "two goals before the block");

    let mut st2 = st.clone();
    assert!(
        st2.run_script("focus (split)").is_err(),
        "a block whose body leaves two goals open must be an ERROR — that is exactly the \
         misalignment a flat script cannot detect"
    );
    assert_eq!(
        st2.open_goals(),
        2,
        "a failed focused block must leave the open set exactly as it was"
    );

    // And the successful shape still closes and certifies.
    st.run_script("focus (exact hp) . focus (split . exact hq . exact hp)")
        .expect("each block closes its own goal");
    assert!(
        st.qed().expect("complete").verified,
        "the focused proof must certify"
    );
}

// ============================================================================================
// `<;>` — scoping, in the failing direction as well as the counting one
// ============================================================================================

/// The counting test says the untouched goal stays open. This says the untouched goal is not
/// even ATTEMPTED: a right-hand tactic that would fail on it must not make the chain fail.
#[test]
fn then_all_does_not_attempt_the_goals_its_left_hand_did_not_produce() {
    let goal = and(or(prop_p(), prop_q()), ProofExpr::Atom("⊥".to_string()));
    let mut st = state(vec![("hp", prop_p())], goal);
    st.run_script("split").expect("∧-introduction");
    // The second conjunct (⊥) has no hypothesis that closes it. Under an `all_goals`-shaped
    // `<;>` the `assumption` would be run against it and the whole chain would fail.
    st.run_script("left <;> assumption")
        .expect("`<;>` must only touch the goal `left` produced");
    assert_eq!(
        st.open_goals(),
        1,
        "the ⊥ goal must still be open and untouched"
    );
    assert_eq!(
        st.focused_target(),
        Some(&ProofExpr::Atom("⊥".to_string())),
        "the untouched goal must be the ⊥ one, got {:?}",
        st.focused_target()
    );
}

// ============================================================================================
// `induction n` — the named variable, its certification, and its refusal
// ============================================================================================

/// Inducting on an INNER variable is a generalization, so the result has to be checked by
/// the kernel and not merely have the right shape. The premises here state the induction
/// principle for the generalized motive `λn. ∀m. Q m n`.
#[test]
fn induction_on_an_inner_variable_certifies() {
    let base = forall("m", p_of(c("Zero")));
    let step = forall(
        "k",
        implies(
            forall("m", p_of(v("k"))),
            forall("m", p_of(app("Succ", vec![v("k")]))),
        ),
    );
    let goal = forall("m", forall("n", p_of(v("n"))));
    let mut st = state(vec![("base", base), ("step", step)], goal);
    st.run_script("induction n . auto . auto")
        .expect("both cases close");
    let r = st.qed().expect("no goals remain");
    assert!(
        r.verified,
        "`induction n` on an inner binder must reach the KERNEL, not merely produce the right \
         subgoals — the generalization is only sound if the trust door accepts it: {:?}",
        r.verification_error
    );
}

/// The refusal: a variable the goal's prefix does not bind is a mistake about the goal, and
/// guessing the outermost binder instead is how two different theorems get proved under one
/// name.
#[test]
fn induction_declines_a_variable_the_goal_does_not_bind() {
    let goal = forall("m", forall("n", pred("Q", vec![v("m"), v("n")])));
    let mut st = ProofState::start(vec![], goal);
    assert!(
        st.induction_on("z").is_err(),
        "`induction z` on a goal that binds only `m` and `n` must be refused, never silently \
         re-aimed at whatever binder is outermost"
    );
    assert_eq!(
        st.open_goals(),
        1,
        "a declined induction must leave the goal alone"
    );
}

/// The argumentless form still means the OUTERMOST binder, which is the whole reason naming
/// the variable matters — the two spellings must be able to disagree.
#[test]
fn argumentless_induction_and_a_named_inner_variable_split_differently() {
    let goal = forall("m", forall("n", pred("Q", vec![v("m"), v("n")])));

    let mut outer = ProofState::start(vec![], goal.clone());
    outer
        .run_script("induction")
        .expect("the argumentless atom applies");
    assert_eq!(
        outer.focused_target(),
        Some(&forall("n", pred("Q", vec![c("Zero"), v("n")]))),
        "argumentless induction takes `m`, got {:?}",
        outer.focused_target()
    );

    let mut inner = ProofState::start(vec![], goal);
    inner
        .run_script("induction n")
        .expect("the named atom applies");
    assert_eq!(
        inner.focused_target(),
        Some(&forall("m", pred("Q", vec![v("m"), c("Zero")]))),
        "`induction n` takes `n`, got {:?}",
        inner.focused_target()
    );
}

// ============================================================================================
// `induction` over an inferred inductive — and what it must NOT infer
// ============================================================================================

/// The list scheme, driven to the trust door: the kernel re-checks the eliminator for
/// coverage, case types and termination, so a scheme invented out of the constructor NAMES
/// would be rejected there.
#[test]
fn an_inferred_list_induction_certifies() {
    let base = p_of(c("ENil"));
    let step = forall(
        "h",
        forall(
            "t",
            implies(p_of(v("t")), p_of(app("ECons", vec![v("h"), v("t")]))),
        ),
    );
    let goal = forall("l", p_of(v("l")));
    let r = prove(
        vec![("base", base), ("step", step)],
        goal,
        "induction . auto . auto",
    );
    assert!(
        matches!(r, Ok(true)),
        "a list goal whose ENil/ECons premises are in scope must induct over EList and \
         certify: {r:?}"
    );
}

/// The Peano goal keeps the Peano scheme — the specialised `Nat` recursor, not the generic
/// eliminator — and still certifies. This is the boundary one step below the inference.
#[test]
fn a_nat_goal_still_uses_the_nat_recursor() {
    let base = p_of(c("Zero"));
    let step = forall("k", implies(p_of(v("k")), p_of(app("Succ", vec![v("k")]))));
    let goal = forall("n", p_of(v("n")));

    let mut st = ProofState::start(vec![base.clone(), step.clone()], goal.clone());
    st.run_script("induction . auto . auto")
        .expect("Nat induction closes");
    let tree = st.assembled().expect("the proof is complete");
    assert!(
        uses_rule(
            &tree,
            &InferenceRule::StructuralInduction {
                variable: String::new(),
                ind_type: String::new(),
                step_var: String::new(),
            }
        ),
        "a Nat goal must keep the specialised recursor, got {:?}",
        tree.rule
    );
    assert!(
        st.qed().expect("complete").verified,
        "Nat induction must certify"
    );
}

/// With no constructor premises in scope there is nothing to infer, and inventing a scheme
/// would be worse than the documented Peano default — so the default is what happens.
#[test]
fn induction_without_constructor_premises_falls_back_to_peano() {
    let goal = forall("n", p_of(v("n")));
    let mut st = ProofState::start(vec![], goal);
    st.run_script("induction").expect("the atom applies");
    assert_eq!(
        st.focused_target(),
        Some(&p_of(c("Zero"))),
        "with nothing in scope to read a scheme off, `induction` is Peano, got {:?}",
        st.focused_target()
    );
}

// ============================================================================================
// `rewrite` — the quantified instantiation, its certification, and its refusals
// ============================================================================================

/// The instantiated rewrite, driven to the trust door: the equality premise is a
/// `UniversalInstTerm` chain over the rule, so the kernel re-checks the instantiation too.
#[test]
fn a_quantified_rewrite_certifies_through_its_instantiation() {
    let rule = forall("x", eq(app("f", vec![v("x")]), app("g", vec![v("x")])));
    let premises = vec![("h", rule), ("hg", eq(app("g", vec![c("Aa")]), c("Z")))];
    let goal = eq(app("f", vec![c("Aa")]), c("Z"));
    let mut st = state(premises, goal);
    st.run_script("rewrite h")
        .expect("the quantified rule instantiates at f(Aa)");
    let target = st.focused_target().cloned().expect("a goal is open");
    assert_eq!(
        target,
        eq(app("g", vec![c("Aa")]), c("Z")),
        "the instantiated left-hand side must be the thing rewritten, got {target:?}"
    );
    st.run_script("assumption")
        .expect("the rewritten goal IS the hypothesis");
    let tree = st.assembled().expect("the proof is complete");
    assert!(
        uses_rule(
            &tree,
            &InferenceRule::UniversalInstTerm(ProofTerm::Constant(String::new()))
        ),
        "the equality premise must record the instantiation: {:?}",
        tree.rule
    );
    let r = st.qed().expect("complete");
    assert!(
        r.verified,
        "a quantified rewrite must certify: {:?}",
        r.verification_error
    );
}

/// The refusals. A rewrite that quietly did nothing, or that instantiated a variable the
/// match never determined, would be a step that claims progress it did not make.
#[test]
fn a_quantified_rewrite_declines_what_it_cannot_instantiate() {
    // Nothing in the goal matches the pattern.
    let rule = forall("x", eq(app("f", vec![v("x")]), app("g", vec![v("x")])));
    let mut st = state(vec![("h", rule)], pred("P", vec![c("A")]));
    assert!(
        st.rewrite("h").is_err(),
        "a quantified rule whose left-hand side matches nothing must be declined"
    );

    // The right-hand side mentions a binder the left-hand side never pins down, so the
    // instantiation is not determined by the match.
    let loose = forall(
        "x",
        forall(
            "y",
            eq(app("f", vec![v("x")]), app("k", vec![v("x"), v("y")])),
        ),
    );
    let mut st = state(vec![("h", loose)], eq(app("f", vec![c("1")]), c("Z")));
    assert!(
        st.rewrite("h").is_err(),
        "a rule whose right-hand side depends on an undetermined variable must be refused — \
         choosing a witness would prove a different theorem"
    );

    // The only occurrence is beneath a binder that re-binds the pattern's variable.
    let mut st = state(
        vec![("h", eq(v("x"), c("0")))],
        forall("x", forall("y", pred("R", vec![v("x"), v("y")]))),
    );
    assert!(
        st.rewrite("h").is_err(),
        "an occurrence the goal BINDS is not an occurrence of the pattern"
    );
}

// ============================================================================================
// `exists` — the witness the rule can carry, and the one it cannot
// ============================================================================================

/// A witness that names an `intro`-bound variable must BE that variable, and the whole thing
/// must reach the kernel — the two `Display` identically, so nothing but certification
/// distinguishes them.
#[test]
fn a_witness_naming_a_bound_variable_is_that_variable_and_certifies() {
    let goal = forall("n", ex("m", eq(v("m"), v("n"))));
    let r = prove(vec![], goal, "intro n . exists n . simp");
    assert!(
        matches!(r, Ok(true)),
        "a variable witness must certify: {r:?}"
    );

    // …and a witness that names nothing in scope is a CONSTANT, which is also certified.
    let goal = ex("n", p_of(v("n")));
    let r = prove(vec![("hp", p_of(c("Sk")))], goal, "exists Sk . assumption");
    assert!(
        matches!(r, Ok(true)),
        "a constant witness must certify: {r:?}"
    );
}

/// A COMPOUND witness travels as a TERM, and the kernel certifies it.
/// `InferenceRule::ExistentialIntro` carries its witness as a `String` that the certifier
/// resolves as one NAME, so `add(n, 1)` handed to it certifies to a global literally called
/// `"add(n, 1)"`, which no context declares. `InferenceRule::ExistentialIntroTerm` carries the
/// `ProofTerm`, the way `UniversalInstTerm` sits beside `UniversalInst`, so `exists` takes the
/// step — Lean's `exists (n + 1)` — and the certificate reaches the kernel intact.
///
/// The neighbours it must still refuse: the name-carrying rule handed the same witness, which
/// the trust door rejects; and a tuple, which is not one witness and is declined at the step,
/// leaving the goal as it was.
#[test]
fn exists_carries_a_compound_witness_as_a_term_and_certifies() {
    let witness = app("add", vec![v("n"), c("1")]);
    let goal = ex("m", eq(v("m"), witness.clone()));
    let mut st = ProofState::start(vec![], goal.clone());
    st.exists(witness.clone())
        .expect("a compound term is a witness");
    assert_eq!(
        st.open_goals(),
        1,
        "the step leaves the instantiated body `add(n, 1) = add(n, 1)`"
    );
    st.rfl()
        .expect("the instantiated body closes by reflexivity");
    let tree = st.assembled().expect("every goal is closed");
    let witness_type = match &tree.rule {
        InferenceRule::ExistentialIntroTerm {
            witness: carried,
            witness_type,
        } => {
            assert_eq!(
                *carried, witness,
                "the witness must travel as the TERM `add(n, 1)`, not as its rendering"
            );
            witness_type.clone()
        }
        other => panic!(
            "`exists` must emit `ExistentialIntroTerm` for a compound witness — \
             `ExistentialIntro` can only carry a name: {other:?}"
        ),
    };
    let r = st.qed().expect("no goal is left open");
    assert!(
        r.verified,
        "the step `exists` took must be the one the KERNEL accepts — a derivation refused at \
         the trust door is a worse outcome than a step that says why it cannot be taken: {:?}",
        r.verification_error
    );

    // The name-carrying rule, handed the same witness at the same domain, is refused: the
    // term rule is what makes the step certifiable, not a relaxed trust door.
    let body = eq(witness.clone(), witness.clone());
    let by_name = DerivationTree::new(
        goal.clone(),
        InferenceRule::ExistentialIntro {
            witness: witness.to_string(),
            witness_type,
        },
        vec![DerivationTree::leaf(body, InferenceRule::Reflexivity)],
    );
    let refused = check_derivation(&[], &goal, by_name);
    assert!(
        !refused.verified,
        "`ExistentialIntro` resolves `{witness}` as ONE name that no context declares; the \
         kernel must refuse it: {:?}",
        refused.proof_term
    );

    // A tuple is not one witness: declined at the step, and the goal is left as it was.
    let mut st = ProofState::start(vec![], goal);
    let attempted = st.exists(ProofTerm::Group(vec![v("n"), c("1")]));
    assert!(
        attempted.is_err(),
        "a tuple is not one witness — `exists` must decline it, not guess a component"
    );
    assert_eq!(
        st.open_goals(),
        1,
        "a declined `exists` must leave the goal as it was"
    );
}

// ============================================================================================
// THE ARGUMENT READER — ordinary identifiers, and the prose that must still read the same
// ============================================================================================

/// The thirteen swallowed identifiers, in every argument position that takes a name.
#[test]
fn a_filler_spelled_identifier_is_usable_wherever_a_name_is() {
    let filler = [
        "a", "an", "it", "we", "on", "of", "to", "the", "that", "now", "with", "by", "and",
    ];
    let mut unspellable = Vec::new();
    for word in filler {
        for form in [
            format!("exists {word}"),
            format!("cases {word}"),
            format!("intro {word}"),
            format!("exact {word}"),
            format!("rewrite {word}"),
            format!("apply {word}"),
        ] {
            if parse_script(&form).is_err() {
                unspellable.push(form);
            }
        }
    }
    assert!(
        unspellable.is_empty(),
        "these are ordinary identifiers and must be usable as tactic arguments: {unspellable:?}"
    );
}

/// The regression guard: the connective vocabulary still reads as prose when there IS a
/// following word for it to connect to. Fixing the swallow must not cost the prose surface.
#[test]
fn the_prose_vocabulary_still_connects_a_verb_to_its_object() {
    let r = prove(
        vec![("h", and(prop_p(), prop_q()))],
        prop_p(),
        "By cases on h, then by assumption.",
    );
    assert!(
        matches!(r, Ok(true)),
        "`By cases on h` must still mean `cases h`: {r:?}"
    );

    let premises = vec![
        ("h1", eq(c("A"), c("B"))),
        ("h2", eq(c("B"), c("C"))),
        ("hp", p_of(c("C"))),
    ];
    let r = prove(
        premises,
        p_of(c("A")),
        "rewrite with h1 . rewrite with h2 . assumption",
    );
    assert!(
        matches!(r, Ok(true)),
        "`rewrite with h` must still mean `rewrite h`: {r:?}"
    );
}

/// Multi-binder `intro`, and the boundary one step past it: a word that NAMES A TACTIC ends
/// the binder list, so a script that omitted a separator still reads the way it always did.
#[test]
fn intro_takes_a_binder_list_but_stops_at_a_tactic_name() {
    let goal = implies(prop_p(), implies(prop_q(), implies(prop_r(), prop_p())));
    let mut st = state(vec![], goal);
    st.run_script("intro hp hq hr")
        .expect("three binders in one step");
    assert_eq!(
        st.open_goals(),
        1,
        "the binders reduce one goal, they do not branch"
    );
    assert_eq!(
        st.focused_target(),
        Some(&prop_p()),
        "three `intro` names must strip three binders, got {:?}",
        st.focused_target()
    );

    // A word that NAMES A TACTIC is not a binder. The grammar still wants a separator
    // between steps, so the script is rejected — what matters is that `assumption` was not
    // quietly bound as a hypothesis called "assumption", which would strip a second binder
    // and leave the proof one step further along than the source said.
    let mut st = state(
        vec![("hr", prop_r())],
        implies(prop_p(), implies(prop_q(), prop_r())),
    );
    assert!(
        st.run_script("intro hp assumption").is_err(),
        "a tactic name must end the binder list, not join it"
    );
    assert_eq!(
        st.focused_target(),
        Some(&implies(prop_p(), implies(prop_q(), prop_r()))),
        "a rejected script must not have run any of its steps, got {:?}",
        st.focused_target()
    );

    // The separator makes it read as two steps, and then it closes.
    let mut st = state(
        vec![("hr", prop_r())],
        implies(prop_p(), implies(prop_q(), prop_r())),
    );
    st.run_script("intro hp hq . assumption")
        .expect("two binders, then the closer");
    assert_eq!(st.open_goals(), 0, "the closer must have run");
    assert!(
        st.qed().expect("complete").verified,
        "the proof must certify"
    );
}

// ============================================================================================
// `omega` — the pure-`≤` refutation the cascade used to decline
// ============================================================================================

/// The gate defect: `omega_close` returns `None` unless a STRICT hypothesis was tightened,
/// so a purely non-strict contradiction was reported as "satisfiable". The rational core
/// runs first now — and the integer-only system must still route past it.
#[test]
fn omega_refutes_both_a_pure_le_system_and_a_strict_integer_one() {
    let pure = vec![("h1", le(c("5"), c("x"))), ("h2", le(c("x"), c("3")))];
    let r = prove(pure, pred("Q", vec![c("A")]), "omega");
    assert!(
        matches!(r, Ok(true)),
        "`5 ≤ x` and `x ≤ 3` are jointly unsatisfiable over ℚ, hence over ℤ: {r:?}"
    );

    let strict = vec![
        ("h1", eq(app("lt", vec![c("x"), c("y")]), c("true"))),
        (
            "h2",
            eq(
                app("lt", vec![c("y"), app("add", vec![c("x"), c("1")])]),
                c("true"),
            ),
        ),
    ];
    let r = prove(strict, pred("Q", vec![c("A")]), "omega");
    assert!(
        matches!(r, Ok(true)),
        "`x < y ∧ y < x+1` needs DISCRETENESS, which the rational core does not have and \
         `omega` must still reach: {r:?}"
    );

    // And a satisfiable system is still declined.
    let mut st = state(vec![("h", le(c("x"), c("3")))], pred("Q", vec![c("A")]));
    assert!(
        st.omega().is_err(),
        "`x ≤ 3` alone has a model, so nothing follows from it by refutation"
    );
}

// ============================================================================================
// THE TERM LANGUAGE — the Rust-level door, so the script is not the only way in
// ============================================================================================

/// `PTerm` is public because the script is one front end for it, not the only one. A Rust
/// caller building the same term must get the same answer.
#[test]
fn a_rust_built_proof_term_elaborates_the_same_way_a_script_does() {
    let premises = vec![("hpq", implies(prop_p(), prop_q())), ("hp", prop_p())];
    let term = PTerm::App(
        Box::new(PTerm::Ident("hpq".to_string())),
        vec![PTerm::Ident("hp".to_string())],
    );
    let mut st = state(premises.clone(), prop_q());
    st.exact_term(&term)
        .expect("the Rust-built term elaborates");
    assert!(
        st.qed().expect("complete").verified,
        "the Rust-built term must certify"
    );

    let mut scripted = state(premises, prop_q());
    scripted
        .run_script("exact (hpq hp)")
        .expect("the script elaborates");
    assert!(
        scripted.qed().expect("complete").verified,
        "the script and the Rust door must agree"
    );
}

// ============================================================================================
// `auto` — prove it or decline it, and a decline leaves the goal as it was
// ============================================================================================

/// Goals the backward chainer cannot certify. Built with the `verification` feature, its last
/// resort is the Z3 oracle, which attests each of these with an `OracleVerification` leaf and no
/// proof term; without the feature it finds nothing. Either way `auto` must end in one of two
/// states: a proof the kernel certifies, or a decline that leaves the focused goal, its
/// hypotheses and the open-goal count exactly as they were, so another tactic can still prove
/// it. The neighbour that certifies closes each one the honest way, which is what makes the
/// decline worth keeping the goal for.
#[test]
fn auto_either_certifies_or_declines_and_a_decline_leaves_the_goal_as_it_was() {
    let cases: Vec<(Vec<(&str, ProofExpr)>, ProofExpr, &str)> = vec![
        (
            vec![
                ("h1", eq(c("a"), c("b"))),
                ("h2", eq(c("cc"), c("b"))),
                ("hp", pred("P", vec![c("a"), c("cc")])),
            ],
            pred("P", vec![c("b"), c("b")]),
            "crush",
        ),
        (
            vec![
                (
                    "ax",
                    forall("x", eq(app("ff", vec![v("x")]), app("gg", vec![v("x")]))),
                ),
                ("h1", eq(c("a"), c("b"))),
            ],
            eq(app("ff", vec![c("a")]), app("gg", vec![c("b")])),
            "crush",
        ),
    ];
    for (premises, goal, honest) in cases {
        let mut st = state(premises.clone(), goal.clone());
        let before = st.focus().expect("one goal is open").clone();
        match st.auto() {
            Ok(_) => {
                let r = st.qed().expect("`auto` closed the goal");
                assert!(
                    r.verified,
                    "`auto` reported success on `{goal}` and the kernel refused the tree: {:?}",
                    r.verification_error
                );
            }
            Err(_) => {
                assert_eq!(
                    st.open_goals(),
                    1,
                    "a declined `auto` must not consume `{goal}`"
                );
                let after = st.focus().expect("the goal is still open");
                assert_eq!(
                    after.target, before.target,
                    "the declined goal must be `{goal}` still"
                );
                assert_eq!(
                    after
                        .hyps
                        .iter()
                        .map(|h| (&h.name, &h.prop))
                        .collect::<Vec<_>>(),
                    before
                        .hyps
                        .iter()
                        .map(|h| (&h.name, &h.prop))
                        .collect::<Vec<_>>(),
                    "a declined `auto` must leave the hypotheses of `{goal}` as they were"
                );
                st.run_script(honest).unwrap_or_else(|e| {
                    panic!(
                        "`{honest}` proves `{goal}` after `auto` declined it: {}",
                        e.0
                    )
                });
                let r = st.qed().expect("the honest tactic closed the goal");
                assert!(
                    r.verified,
                    "`{honest}` on `{goal}` must certify: {:?}",
                    r.verification_error
                );
            }
        }
    }
}
