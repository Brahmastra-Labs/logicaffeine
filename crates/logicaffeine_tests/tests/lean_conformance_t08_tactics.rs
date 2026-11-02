//! CONFORMANCE CORPUS — TIER 8 · TACTICS. The Lean-4 conformance corpus re-read as a
//! CAPABILITY matrix: for each of the twenty-eight cases, what Lean's tactic does and
//! whether ours does it.
//!
//! The surface under test is `tactic_script.rs` — the text a `Proof:` line carries. It
//! recognises exactly fifteen primitives (`intro exact cases rewrite exists assumption simp
//! decide omega crush split left right auto induction`), five combinators (`;` `<;>` `first`
//! `try` `repeat`), and a filler-word vocabulary. Behind it sits `tactic.rs`, whose Rust API
//! is strictly larger than the grammar that reaches it.
//!
//! Three distinctions run through the whole file and decide the verdicts:
//!
//! * **Wiring, not absence.** `ProofState::apply`, `combinators::all_goals` and
//!   `ProofState::induction_over` are built, tested and certified — and no script text can
//!   name any of them. The rational Fourier-Motzkin solver with Farkas certificates
//!   (`linarith_solve.rs` + `engine::cert_farkas`) is likewise built, and `linarith` is an
//!   alias for `omega` instead. Each of those is a loss today and a cheap win tomorrow, so
//!   each is `MUST` / `Gap` with a green companion test proving the capability exists.
//! * **Absence, full stop.** `any_goals` exists nowhere in the repository — not in the
//!   grammar and not in Rust. Neither do `calc`, `conv`, `refine`, `have`, `by_contra`, or
//!   any goal-focusing construct.
//! * **Refusal is the capability.** `sorry` has no encoding that is not worse than refusing:
//!   `add_declaration` smuggles an axiom, `Term::Hole` matches anything under `types_equal`,
//!   and a `verified == false` artefact defers the "this is not a proof" call to the reader.
//!
//! Every proof assertion here is `verified == true` — the kernel's word, through
//! `qed()` → `check_derivation`. A script that merely RAN proves nothing.
//!
//! RED is the expected state of most of this file. Each failing test names one piece of work.

use logicaffeine_proof::tactic::combinators::{
    all_goals, assumption as assumption_t, split as split_t,
};
use logicaffeine_proof::tactic::{CtorSpec, ProofState};
use logicaffeine_proof::tactic_script::parse_script;
use logicaffeine_proof::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

// ============================================================================================
// HELPERS — per-file, as house style requires. Nothing here is shared with another test.
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
fn atom(s: &str) -> ProofExpr {
    ProofExpr::Atom(s.to_string())
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
fn iff(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Iff(Box::new(l), Box::new(r))
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
/// The engine's absurdity.
fn falsum() -> ProofExpr {
    atom("⊥")
}
/// `a ≤ b` in the system's canonical encoding: `Identity(le(a, b), true)`.
fn le(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    eq(app("le", vec![a, b]), c("true"))
}
/// `a < b` in the same encoding.
fn lt(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    eq(app("lt", vec![a, b]), c("true"))
}
fn add1(t: ProofTerm) -> ProofTerm {
    app("add", vec![t, c("1")])
}
/// The single-argument predicate used wherever the proposition's content is irrelevant.
fn p_of(t: ProofTerm) -> ProofExpr {
    pred("Phi", vec![t])
}
/// The corpus's propositional variables `(p q r : Prop)`. They are unary predicates over one
/// fixed entity rather than `ProofExpr::Atom`s because the certifier resolves an `Atom` as a
/// GLOBAL NAME — a bare atom that is not registered in the kernel context cannot be a premise
/// at all ("Unknown hypothesis"). Nothing in this tier turns on the difference.
fn prop_p() -> ProofExpr {
    pred("p", vec![c("X")])
}
fn prop_q() -> ProofExpr {
    pred("q", vec![c("X")])
}
fn prop_r() -> ProofExpr {
    pred("r", vec![c("X")])
}

/// A proof state whose premises carry the names the Lean source used, so a script can say
/// `exact hp` rather than `exact hp0`.
fn state(premises: Vec<(&str, ProofExpr)>, goal: ProofExpr) -> ProofState {
    let names: Vec<Option<String>> = premises
        .iter()
        .map(|(n, _)| Some((*n).to_string()))
        .collect();
    let props: Vec<ProofExpr> = premises.into_iter().map(|(_, p)| p).collect();
    ProofState::start_with_names(props, &names, goal)
}

/// Run `script` against `premises ⊢ goal` and report whether the KERNEL certified it.
///
/// The error side distinguishes the three ways this can go wrong, because they mean very
/// different things: `script:` — the surface cannot say it; `qed:` — goals were left open;
/// `Ok(false)` — the derivation was built and the kernel refused it.
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
// T08-001 · `intro` — the arity and the goal shapes
// ============================================================================================

/// **T08-001 · MUST / Gap.** In Lean `Not p` IS `p → False`, so `intro` applies to a
/// negation goal. `ProofExpr::Not` is its own constructor and `ProofState::intro` matches
/// only `Implies`/`ForAll`, so the step fails with `DoesNotApply` — at the FIRST step of a
/// proof every later step was emitted against. The disagreement is confined to the tactic
/// layer: the CERTIFIER already lowers `Not(p)` to the Pi form `p → False`, so the two
/// propositions are the same kernel type and always were.
#[test]
fn intro_on_a_negation_goal_assumes_the_negatum() {
    let premises = vec![("h", implies(prop_p(), falsum()))];

    let mut st = state(premises.clone(), not(prop_p()));
    let applied = st.intro("hp");
    assert!(
        applied.is_ok(),
        "`intro hp` must apply to a ¬-goal — Lean unfolds ¬p to p → False and introduces the \
         negatum: {:?}",
        applied.err()
    );
    assert_eq!(
        st.focused_target(),
        Some(&falsum()),
        "after `intro` on ¬p the residual goal must be ⊥, got {:?}",
        st.focused_target()
    );

    let r = prove(premises, not(prop_p()), "intro hp . auto");
    assert!(
        matches!(r, Ok(true)),
        "¬p from p → False must prove by `intro hp . auto` and certify: {r:?}"
    );
}

/// The fragment that already works, pinned so it cannot regress: `intro` on `→` and on `∀`,
/// with the ∀ case renaming the binder to the supplied name.
#[test]
fn intro_discharges_an_implication_and_a_universal() {
    let r = prove(
        vec![("hq", prop_q())],
        implies(prop_p(), prop_q()),
        "intro hp . exact hq",
    );
    assert!(matches!(r, Ok(true)), "→-introduction must certify: {r:?}");

    let r = prove(
        vec![],
        forall("n", implies(p_of(v("n")), p_of(v("n")))),
        "intro n . intro h . exact h",
    );
    assert!(matches!(r, Ok(true)), "∀-introduction must certify: {r:?}");

    let mut st = state(vec![], implies(prop_p(), implies(prop_q(), prop_p())));
    st.run_script("intro hp . intro hq")
        .expect("two separate intro atoms");
    assert_eq!(
        st.focused_target(),
        Some(&prop_p()),
        "two `intro` atoms must strip two binders, got {:?}",
        st.focused_target()
    );
}

/// Lean's `intro a b c` introduces three binders in one step. The script atom is `NAME ARG?`
/// — exactly one argument — so `b` is read as the next TACTIC NAME and the whole script dies
/// at parse time with "unknown tactic `b`", discarding the proof before any step runs.
#[test]
fn intro_takes_several_binders_in_one_step() {
    let r = prove(
        vec![("hr", prop_r())],
        implies(prop_p(), implies(prop_q(), prop_r())),
        "intro hp hq . exact hr",
    );
    assert!(
        matches!(r, Ok(true)),
        "`intro hp hq` must introduce both binders, as Lean's multi-binder `intro` does: {r:?}"
    );
}

// ============================================================================================
// T08-002 · `exact` — a hypothesis NAME, never a term
// ============================================================================================

/// **T08-002 · MUST / Gap.** Lean's `exact` takes an arbitrary TERM: `exact (hpq hp)`,
/// `exact ⟨a, b⟩`, `exact fun x => f x`. The script lexer produces only words, separators and
/// brackets, so a term argument cannot even be TOKENISED — and `exact f h` silently parses as
/// `exact f` followed by the atom `h`.
#[test]
fn exact_accepts_an_applied_proof_term() {
    let premises = vec![("hpq", implies(prop_p(), prop_q())), ("hp", prop_p())];
    let r = prove(premises, prop_q(), "exact (hpq hp)");
    assert!(
        matches!(r, Ok(true)),
        "`exact` must accept a TERM, not only a bare hypothesis name — `exact (hpq hp)` is \
         ordinary Lean: {r:?}"
    );
}

/// The fragment that works, and the two distinct failures it reports — a missing NAME is
/// `NoSuchHypothesis`, a present name with the wrong proposition is `DoesNotApply`. A
/// translator that cannot tell them apart reports a naming bug as a logic bug.
#[test]
fn exact_names_a_hypothesis_and_separates_its_two_failures() {
    let r = prove(vec![("hp", prop_p())], prop_p(), "exact hp");
    assert!(
        matches!(r, Ok(true)),
        "`exact hp` on a matching hypothesis must certify: {r:?}"
    );

    let mut st = state(vec![("hp", prop_p())], prop_q());
    let wrong_prop = st.exact("hp");
    assert!(
        wrong_prop.is_err(),
        "`exact` on a hypothesis that does not prove the goal must fail, not close it"
    );

    let mut st = state(vec![("hp", prop_p())], prop_p());
    let wrong_name = st.exact("nosuch");
    assert!(wrong_name.is_err(), "`exact` on an absent name must fail");
}

// ============================================================================================
// T08-003 · `apply` — built in Rust, unreachable from script text
// ============================================================================================

/// **T08-003 · MUST / Gap — PURE WIRING.** `ProofState::apply` is modus ponens on a
/// top-level implication and it works (see the companion below). `canonical` has no arm
/// producing `apply`, `parse_primitive` has no dispatch for it, and `combinators` wraps no
/// value for it — so no script text can invoke it. The idea is present; the surface does not
/// reach it.
#[test]
fn apply_is_reachable_from_script_text() {
    let premises = vec![("hpq", implies(prop_p(), prop_q())), ("hp", prop_p())];
    let r = prove(premises, prop_q(), "apply hpq . exact hp");
    assert!(
        matches!(r, Ok(true)),
        "`apply hpq` must reduce the goal to the antecedent from SCRIPT TEXT — the Rust \
         `ProofState::apply` already does exactly this: {r:?}"
    );
}

/// The wiring evidence: driven from Rust the very same step certifies. Nothing is missing but
/// a name in `canonical` and an arm in `parse_primitive`.
#[test]
fn the_rust_apply_already_performs_modus_ponens() {
    let rule = implies(prop_p(), prop_q());
    let mut st = ProofState::start(vec![rule.clone(), prop_p()], prop_q());
    st.apply(&rule)
        .expect("apply reduces Q to its antecedent P");
    assert_eq!(
        st.focused_target(),
        Some(&prop_p()),
        "apply must leave the antecedent as the subgoal, got {:?}",
        st.focused_target()
    );
    st.assumption().expect("P is a premise");
    let r = st.qed().expect("no goals remain");
    assert!(
        r.verified,
        "the Rust-level apply must certify: {:?}",
        r.verification_error
    );
}

// ============================================================================================
// T08-004 · `assumption` — structural search, not defeq search
// ============================================================================================

/// **T08-004 · MUST / Gap.** Lean's `assumption` matches up to reducible definitional
/// unfolding, so `h : ¬p` closes the goal `p → False`. Ours is `h.prop == g.target` on an
/// untyped `ProofExpr`, and `Not(p)` is a different constructor from `Implies(p, ⊥)`. The
/// reported error is `NoSuchHypothesis`, which reads like a naming bug.
#[test]
fn assumption_matches_a_hypothesis_up_to_definitional_unfolding() {
    let r = prove(
        vec![("h", not(prop_p()))],
        implies(prop_p(), falsum()),
        "assumption",
    );
    assert!(
        matches!(r, Ok(true)),
        "`h : ¬p` must close the goal `p → False` — they are the same proposition: {r:?}"
    );
}

/// What already works, and the part of it that is genuinely better than a bare name: a
/// conjunct introduced by `cases` closes with its `ConjunctionElim` PROJECTION, so the
/// certified term is the projection applied to the parent's proof rather than a dangling
/// reference the certifier's context does not bind.
#[test]
fn assumption_closes_a_conjunct_with_its_projection() {
    let mut st = state(vec![("h", and(prop_p(), prop_q()))], prop_p());
    st.run_script("cases h . assumption")
        .expect("∧-elimination then assumption");
    let tree = st.assembled().expect("the proof is complete");
    assert!(
        uses_rule(&tree, &InferenceRule::ConjunctionElim),
        "closing on a `cases`-extracted conjunct must emit the projection, not a bare \
         PremiseMatch leaf: {:?}",
        tree.rule
    );
    let r = st.qed().expect("no goals remain");
    assert!(
        r.verified,
        "the projection must certify: {:?}",
        r.verification_error
    );
}

// ============================================================================================
// T08-005 · `rfl` — no spelling, and no delta step at the script layer
// ============================================================================================

/// **T08-005 · MUST / Gap — NEAR WIRING.** `canonical` maps no word to `rfl`, so a single
/// stray `rfl` anywhere in a long script is a PARSE error that discards the whole proof. The
/// reflexivity closer itself exists — it is the branch of `simp` that emits
/// `InferenceRule::Reflexivity` — it simply has no name of its own. Lean's `rfl` additionally
/// works up to delta, for which the script layer has no step at all.
#[test]
fn rfl_closes_a_reflexive_goal_under_its_own_name() {
    let r = prove(vec![], eq(c("A"), c("A")), "rfl");
    assert!(
        matches!(r, Ok(true)),
        "`rfl` must be a tactic name — it is the single most-used tactic in Lean and the \
         reflexivity closer already exists inside `simp`: {r:?}"
    );
}

/// The two routes that do exist, pinned: `simp`'s reflexivity branch for a syntactic identity,
/// `decide` for one that needs evaluation.
#[test]
fn simp_and_decide_between_them_cover_syntactic_and_ground_reflexivity() {
    let mut st = ProofState::start(vec![], eq(c("A"), c("A")));
    st.run_script("simp")
        .expect("simp closes a syntactic identity");
    let tree = st.assembled().expect("the proof is complete");
    assert!(
        uses_rule(&tree, &InferenceRule::Reflexivity),
        "simp's closing branch must emit Reflexivity, got {:?}",
        tree.rule
    );
    assert!(
        st.qed().expect("complete").verified,
        "syntactic reflexivity must certify"
    );

    let r = prove(
        vec![],
        eq(app("add", vec![c("2"), c("2")]), c("4")),
        "decide",
    );
    assert!(
        matches!(r, Ok(true)),
        "reflexivity up to evaluation routes to `decide`: {r:?}"
    );
}

// ============================================================================================
// T08-006 · `constructor` — an ALIAS for `split`, which is ∧-only
// ============================================================================================

/// **T08-006 · MUST / Gap — NEAR WIRING.** Lean's `constructor` applies the first constructor
/// of ANY inductive whose conclusion matches: `And.intro`, `Or.inl`, `Iff.intro`,
/// `Exists.intro`, a structure's `mk`. The script word `constructor` is hard-wired to
/// ∧-introduction, so an `Iff` goal has no step-wise script proof at all — even though
/// `InferenceRule::BicondIntro` exists and the certifier has an arm for it.
#[test]
fn constructor_introduces_a_biconditional_as_two_implications() {
    let r = prove(
        vec![],
        iff(prop_p(), prop_p()),
        "constructor <;> (intro h . exact h)",
    );
    assert!(
        matches!(r, Ok(true)),
        "`constructor` on `p ↔ p` must leave the two implications — BicondIntro is already a \
         certified rule and no script atom introduces it: {r:?}"
    );
}

/// The ∧ fragment, and the subgoal ORDER that a translator's later steps depend on: `split`
/// pushes the LEFT conjunct to the front, matching Lean. For a symmetric conjunction a
/// reversed order still succeeds, which is why the order is asserted directly.
#[test]
fn split_leaves_the_left_conjunct_focused() {
    let mut st = state(
        vec![("hp", prop_p()), ("hq", prop_q())],
        and(prop_p(), prop_q()),
    );
    st.run_script("split").expect("split applies to ∧");
    assert_eq!(st.open_goals(), 2, "∧-introduction leaves two goals");
    assert_eq!(
        st.focused_target(),
        Some(&prop_p()),
        "the LEFT conjunct must be focused first, got {:?}",
        st.focused_target()
    );
    st.run_script("exact hp . exact hq")
        .expect("both conjuncts discharge in order");
    assert!(
        st.qed().expect("complete").verified,
        "∧-introduction must certify"
    );
}

// ============================================================================================
// T08-007 · `left` / `right` — full parity on the propositional layer
// ============================================================================================

/// **T08-007 · MUST / Native.** The names coincide, the goal shape coincides, the single
/// residual subgoal coincides, and the certified `DisjunctionIntro` node re-checks. Lean's
/// `left` on a two-constructor DATA inductive (`Sum`, a user enum) is term construction
/// rather than proof and has no proposition to translate; on the propositional layer we are
/// at parity.
#[test]
fn left_and_right_select_the_named_disjunct() {
    let mut st = state(vec![("hp", prop_p())], or(prop_p(), prop_q()));
    st.run_script("left").expect("left applies to ∨");
    assert_eq!(st.open_goals(), 1, "∨-introduction leaves exactly one goal");
    assert_eq!(
        st.focused_target(),
        Some(&prop_p()),
        "`left` must leave the LEFT disjunct, got {:?}",
        st.focused_target()
    );
    st.run_script("assumption").expect("P is a premise");
    assert!(st.qed().expect("complete").verified, "`left` must certify");

    let r = prove(
        vec![("hq", prop_q())],
        or(prop_p(), prop_q()),
        "right . assumption",
    );
    assert!(matches!(r, Ok(true)), "`right` must certify: {r:?}");

    let nested = or(prop_p(), or(prop_q(), prop_r()));
    let r = prove(vec![("hq", prop_q())], nested, "right . left . assumption");
    assert!(
        matches!(r, Ok(true)),
        "nested ∨-introduction must certify: {r:?}"
    );

    let mut st = state(vec![("hp", prop_p())], and(prop_p(), prop_p()));
    assert!(
        st.left().is_err(),
        "`left` on a non-∨ goal must be declined, not guessed"
    );
}

// ============================================================================================
// T08-008 · `cases` — fixed binder names, a counter-dependent witness
// ============================================================================================

/// **T08-008 · MUST / Gap.** Lean's `cases h with | intro hp hq` and `obtain ⟨hp, hq⟩ := h`
/// NAME the components. LOGOS names them `h_1`/`h_2` after the parent, and the script grammar
/// has nowhere to put the user's names — `cases h with hp hq` parses as `cases h` and then
/// dies with "trailing tokens after the script".
#[test]
fn cases_binds_its_components_under_the_names_the_proof_chose() {
    let r = prove(
        vec![("h", and(prop_p(), prop_q()))],
        prop_p(),
        "cases h with hp hq . exact hp",
    );
    assert!(
        matches!(r, Ok(true)),
        "`cases h with hp hq` must bind the components under the proof's own names: {r:?}"
    );
}

/// What works: one `cases` per destructuring level, with the generated names composing
/// (`h_1` then `h_1_1`). The parent survives — LOGOS's ∧ branch clones the hypothesis list
/// and pushes, where Lean's `cases` consumes the scrutinee.
#[test]
fn cases_destructures_one_level_at_a_time_and_keeps_the_parent() {
    let h = and(and(prop_p(), prop_q()), prop_r());
    let mut st = state(vec![("h", h.clone())], and(prop_p(), prop_r()));
    st.run_script("cases h . cases h_1")
        .expect("two destructuring levels");
    let names: Vec<String> = st
        .focus()
        .expect("a goal is open")
        .hyps
        .iter()
        .map(|x| x.name.clone())
        .collect();
    for want in ["h", "h_1", "h_2", "h_1_1", "h_1_2"] {
        assert!(
            names.iter().any(|n| n == want),
            "after two `cases` the scope must hold {want}; it holds {names:?}"
        );
    }
    st.run_script("split . exact h_1_1 . exact h_2")
        .expect("both conjuncts discharge");
    assert!(
        st.qed().expect("complete").verified,
        "nested destructuring must certify"
    );
}

/// The ∃ branch introduces a fresh witness CONSTANT whose index comes from a counter shared
/// with `induction`, so its name depends on how many earlier tactics ran. The name is
/// therefore something to READ OFF the state, never to predict — this pins that it is
/// readable, without pinning the counter's value.
#[test]
fn an_existential_witness_name_is_discoverable_from_the_state() {
    let h = ex("n", p_of(v("n")));
    let mut st = state(vec![("h", h)], prop_q());
    st.run_script("cases h").expect("∃-elimination");
    let hyp = st
        .focus()
        .expect("a goal is open")
        .hyps
        .iter()
        .find(|x| x.name == "h_w")
        .cloned()
        .expect("the ∃ branch names its body hypothesis `{parent}_w`");
    match &hyp.prop {
        ProofExpr::Predicate { args, .. } => match args.as_slice() {
            [ProofTerm::Constant(w)] => assert!(
                w.starts_with('W'),
                "the witness must be a fresh eigen-CONSTANT, got {w}"
            ),
            other => panic!("the witness hypothesis must be P(witness), got {other:?}"),
        },
        other => panic!("the ∃ branch must instantiate the body, got {other:?}"),
    }
}

// ============================================================================================
// T08-009 · `induction` — zero-argument, so it takes the OUTERMOST binder
// ============================================================================================

/// **T08-009 · MUST / Gap.** Lean's `induction n` NAMES its target. The script atom takes no
/// argument, so it inducts on whatever the goal's outermost `∀` happens to be — and emitting
/// `induction n` is a parse error, because `n` is read as the next tactic name. With a goal
/// `∀ m n, Q m n` Lean's `induction n` and LOGOS's `induction` prove DIFFERENT theorems, both
/// certified, and nothing downstream can tell.
#[test]
fn induction_inducts_on_the_variable_it_is_given() {
    let goal = forall("m", forall("n", pred("Q", vec![v("m"), v("n")])));
    let mut st = ProofState::start(vec![], goal);

    let ran = st.run_script("induction n");
    assert!(
        ran.is_ok(),
        "`induction n` must name the variable it inducts on: {:?}",
        ran.err()
    );
    let want = forall("m", pred("Q", vec![v("m"), c("Zero")]));
    assert_eq!(
        st.focused_target(),
        Some(&want),
        "inducting on `n` must leave the base case ∀m. Q(m, Zero) — the argumentless atom takes \
         `m` instead and proves a different theorem: {:?}",
        st.focused_target()
    );
}

/// The Nat shape that works: base first, step second, the induction hypothesis named `ih` and
/// placed FIRST so `assumption`/`auto` prefer it over a base fact and keep the step variable
/// symbolic.
#[test]
fn argumentless_induction_splits_a_nat_goal_into_base_and_step() {
    let base = p_of(c("Zero"));
    let step = forall("k", implies(p_of(v("k")), p_of(app("Succ", vec![v("k")]))));
    let goal = forall("n", p_of(v("n")));

    let mut st = ProofState::start(vec![base.clone(), step.clone()], goal.clone());
    st.run_script("induction")
        .expect("induction applies to a ∀ goal");
    assert_eq!(
        st.open_goals(),
        2,
        "Nat induction leaves the base and the step"
    );
    assert_eq!(
        st.focused_target(),
        Some(&base),
        "the BASE case must be focused first, got {:?}",
        st.focused_target()
    );

    let r = prove(
        vec![("base", base), ("step", step)],
        goal,
        "induction . auto . auto",
    );
    assert!(matches!(r, Ok(true)), "Nat induction must certify: {r:?}");
}

// ============================================================================================
// T08-010 · induction on a LIST — the script atom hard-codes Nat
// ============================================================================================

/// **T08-010 · MUST / Gap — PURE WIRING.** `ProofState::induction` substitutes `Zero` and
/// `Succ K` by name, unconditionally, because `ProofExpr` is untyped and there is nothing to
/// check the variable's type against. Against a LIST goal it produces syntactically
/// well-formed nonsense that `auto`/`simp`/`crush` will happily attack — and for a predicate
/// insensitive to the scrutinee they will SUCCEED, certifying a `Nat` induction for a
/// statement that never mentioned `Nat`. The right scheme, `induction_over`, is built,
/// certified and unreachable from script text.
#[test]
fn induction_on_a_list_goal_splits_on_nil_and_cons() {
    let base = p_of(c("ENil"));
    let step = forall(
        "h",
        forall(
            "t",
            implies(p_of(v("t")), p_of(app("ECons", vec![v("h"), v("t")]))),
        ),
    );
    let goal = forall("l", p_of(v("l")));

    let mut st = ProofState::start(vec![base.clone(), step], goal);
    st.run_script("induction")
        .expect("the `induction` atom parses");
    assert_eq!(
        st.focused_target(),
        Some(&base),
        "induction on a LIST goal must split on ENil/ECons; the script-level `induction` is \
         hard-wired to Nat and substituted a Peano constructor into a list-valued position: \
         {:?}",
        st.focused_target()
    );
}

/// The wiring evidence: the generic scheme does exactly the right thing from Rust, one
/// subgoal per constructor with an induction hypothesis per recursive argument, and the
/// kernel re-checks the eliminator for coverage, case types and termination.
#[test]
fn the_rust_induction_over_already_handles_a_two_constructor_list() {
    let base = p_of(c("ENil"));
    let step = forall(
        "h",
        forall(
            "t",
            implies(p_of(v("t")), p_of(app("ECons", vec![v("h"), v("t")]))),
        ),
    );
    let goal = forall("l", p_of(v("l")));

    let mut st = ProofState::start(vec![base.clone(), step], goal);
    st.induction_over(
        "EList",
        vec![
            CtorSpec {
                constructor: "ENil".to_string(),
                recursive: vec![],
            },
            CtorSpec {
                constructor: "ECons".to_string(),
                recursive: vec![false, true],
            },
        ],
    )
    .expect("the generic scheme applies to a ∀ goal");
    assert_eq!(
        st.focused_target(),
        Some(&base),
        "the ENil case must come first, got {:?}",
        st.focused_target()
    );
    st.run_script("auto . auto")
        .expect("both constructor cases discharge");
    let r = st.qed().expect("complete");
    assert!(
        r.verified,
        "list induction must certify: {:?}",
        r.verification_error
    );
}

// ============================================================================================
// T08-011 · `rewrite` — all occurrences, one direction, goal only
// ============================================================================================

/// **T08-011 · MUST / Gap.** Lean's `rw [h]` with `h : ∀ x, f x = g x` finds the first match,
/// fixes `x` from it, and rewrites that instance. `ProofState::rewrite` requires a GROUND
/// `ProofExpr::Identity` and refuses a `ForAll` outright, so a quantified rewrite — the
/// commonest kind in real mathematics — has no image at all.
#[test]
fn rewrite_instantiates_a_quantified_equation_at_its_first_match() {
    let h = forall("x", eq(app("f", vec![v("x")]), app("g", vec![v("x")])));
    let goal = and(
        eq(app("f", vec![c("1")]), app("g", vec![c("1")])),
        eq(app("f", vec![c("2")]), app("g", vec![c("2")])),
    );
    let mut st = state(vec![("h", h)], goal);

    let ran = st.run_script("rewrite h");
    assert!(
        ran.is_ok(),
        "`rewrite` must accept a ∀-quantified equation and instantiate it: {:?}",
        ran.err()
    );
    let want = and(
        eq(app("g", vec![c("1")]), app("g", vec![c("1")])),
        eq(app("f", vec![c("2")]), app("g", vec![c("2")])),
    );
    assert_eq!(
        st.focused_target(),
        Some(&want),
        "a quantified rewrite instantiates at the FIRST match and leaves the rest alone: {:?}",
        st.focused_target()
    );
}

/// THE CAPTURE HAZARD. `replace_in_expr` recurses into `ForAll`/`Exists` bodies and compares
/// terms structurally with no check that the pattern is the binder. Rewriting with `h : x = 0`
/// inside a goal `∀x. P(x)` therefore replaces the BOUND occurrence and yields `∀x. P(0)` —
/// which does not prove `∀x. P(x)`. Lean's `kabstract` reports "motive is not type correct"
/// instead. The step must be refused.
#[test]
fn rewrite_refuses_to_substitute_a_variable_the_goal_binds() {
    let h = eq(v("x"), c("0"));
    let goal = forall("x", p_of(v("x")));
    let mut st = state(vec![("h", h)], goal);
    assert!(
        st.rewrite("h").is_err(),
        "rewriting a variable that the GOAL BINDS must be refused: substituting under the \
         binder turns ∀x. P(x) into ∀x. P(0), and proving that does not prove the goal"
    );
}

/// The ground fragment, where the two languages agree, pinned so a translator that tests only
/// this shape is at least testing something real.
#[test]
fn rewrite_chains_ground_equalities() {
    let premises = vec![
        ("h1", eq(c("A"), c("B"))),
        ("h2", eq(c("B"), c("C"))),
        ("hp", p_of(c("C"))),
    ];
    let r = prove(
        premises,
        p_of(c("A")),
        "rewrite h1 . rewrite h2 . assumption",
    );
    assert!(
        matches!(r, Ok(true)),
        "a ground rewrite chain must certify: {r:?}"
    );
}

// ============================================================================================
// T08-012 · `simp` — no lemma list, and every in-scope hypothesis instead
// ============================================================================================

/// **T08-012 · MUST / Gap.** `simp only [foo, bar]` names its rules precisely; the script atom
/// is nullary and its rule set is every in-scope hypothesis that orients. Writing the list
/// anyway parses as the atom `simp` followed by the atom `only`, and the script dies at parse
/// time. The loss runs in the dangerous direction — LOGOS uses MORE rules than asked for, so
/// two `simp only` calls meant to stop at different normal forms collapse into one.
#[test]
fn simp_takes_a_named_rule_list() {
    let h = eq(app("f", vec![c("A")]), c("A"));
    let goal = eq(app("f", vec![app("f", vec![c("A")])]), c("A"));
    let r = prove(vec![("h", h)], goal, "simp only [h]");
    assert!(
        matches!(r, Ok(true)),
        "`simp only [h]` must name its rule set — selection is the whole point of `only`: {r:?}"
    );
}

/// The bare `simp`, whose rule set is the scope, pinned: it normalises to a fixpoint and then
/// closes by reflexivity or by an exact hypothesis.
#[test]
fn bare_simp_rewrites_with_everything_in_scope() {
    let h = eq(app("f", vec![c("A")]), c("A"));
    let goal = eq(app("f", vec![app("f", vec![c("A")])]), c("A"));
    let r = prove(vec![("h", h)], goal, "simp");
    assert!(
        matches!(r, Ok(true)),
        "bare `simp` must normalise and close: {r:?}"
    );

    let mut st = ProofState::start(vec![], p_of(c("A")));
    let set = st.scope_simp_set();
    assert!(
        st.simp(&set).is_err(),
        "`simp` that neither rewrites nor closes must fail — it is not a no-op, so it cannot \
         stand in for a Lean tactic that had nothing to do"
    );
}

// ============================================================================================
// T08-013 · `omega` — refutation only; the goal is never inspected
// ============================================================================================

/// **T08-013 · MUST / Gap.** Lean's `omega` is a DECISION PROCEDURE for the goal: it negates
/// it, adds it to the hypothesis set, and refutes. Ours never looks at the goal — it refutes
/// the in-scope `≤`/`<` facts and, on success, discharges anything by ex-falso. So
/// `x ≤ 3 ⊢ x ≤ 4` is declined, because `x ≤ 3` alone has a model.
#[test]
fn omega_decides_a_positive_linear_goal() {
    let r = prove(vec![("h", le(c("x"), c("3")))], le(c("x"), c("4")), "omega");
    assert!(
        matches!(r, Ok(true)),
        "`omega` must negate the goal and decide it, not only refute the hypotheses — \
         `x ≤ 3 ⊢ x ≤ 4` is the whole point of a decision procedure: {r:?}"
    );
}

/// The refutation fragment, which corresponds one-for-one, including the discreteness step
/// that rational solvers lack.
#[test]
fn omega_refutes_a_strict_integer_contradiction() {
    let premises = vec![("h1", lt(c("x"), c("y"))), ("h2", lt(c("y"), add1(c("x"))))];
    let r = prove(premises, pred("Q", vec![c("A")]), "omega");
    assert!(
        matches!(r, Ok(true)),
        "`x < y ∧ y < x+1` is integer-UNSAT by discreteness and must discharge any goal: {r:?}"
    );
}

/// A gate defect found while writing this tier: `omega_close` returns `None` unless at least
/// one STRICT hypothesis was tightened, on the assumption that the caller already tried the
/// rational core. `ProofState::omega` never does — so a PURE `≤` contradiction is declined,
/// and the message says "the integer hypotheses are satisfiable" about a system that is not.
#[test]
fn omega_refutes_a_contradictory_pure_le_system() {
    let premises = vec![("h1", le(c("5"), c("x"))), ("h2", le(c("x"), c("3")))];
    let r = prove(premises, pred("Q", vec![c("A")]), "omega");
    assert!(
        matches!(r, Ok(true)),
        "`5 ≤ x` and `x ≤ 3` are jointly unsatisfiable and `cert_farkas` finds the certificate; \
         `omega` must not decline them as satisfiable: {r:?}"
    );
}

// ============================================================================================
// T08-014 · `decide` — an evaluator router, not a `Decidable` instance
// ============================================================================================

/// **T08-014 · MUST / Gap.** Lean's `decide` works for any proposition with a `Decidable`
/// instance — a derived `DecidableEq`, a bounded quantifier, a structural equality. Ours is a
/// hard-coded router over ground `i64` arithmetic, ground comparisons and `Bool` equalities,
/// with no `Not` arm and no instance resolution, and it reports "not a closed decidable goal
/// (or it is false)" for both an unsupported goal and a FALSE one.
#[test]
fn decide_covers_the_decidable_fragment_not_only_ground_int_and_bool() {
    let disequality = not(eq(c("2"), c("3")));
    let r = prove(vec![], disequality, "decide");
    assert!(
        matches!(r, Ok(true)),
        "`decide` must settle a ground DISEQUALITY — `decide_expr` has no `Not` arm at all: {r:?}"
    );

    let big = eq(
        app("add", vec![c("9223372036854775808"), c("1")]),
        c("9223372036854775809"),
    );
    let r = prove(vec![], big, "decide");
    assert!(
        matches!(r, Ok(true)),
        "`decide` must reach past the i64 evaluator window — the kernel's own `Literal` is a \
         BigInt, so the boundary is the router's, not the kernel's: {r:?}"
    );
}

/// The ground fragment, and the decline that must stay a decline: a FALSE goal is never
/// guessed.
#[test]
fn decide_closes_ground_arithmetic_and_declines_a_false_goal() {
    let r = prove(
        vec![],
        eq(app("mul", vec![c("6"), c("7")]), c("42")),
        "decide",
    );
    assert!(
        matches!(r, Ok(true)),
        "ground arithmetic must certify: {r:?}"
    );

    let r = prove(vec![], lt(c("5"), c("2")), "decide");
    assert!(
        r.is_err(),
        "a false ground comparison must be DECLINED, never proved: {r:?}"
    );
}

// ============================================================================================
// T08-015 · `ring` — literate surface only, `Int`/`Nat` only
// ============================================================================================

/// **T08-015 · MUST / Gap.** `ring` is reachable only from the literate kernel surface, where
/// it is `is_ring_type`-gated to `Int` and `Nat` and reports failure by REDUCING to
/// `DAxiom (SName "Error")` rather than failing. The tactic-script surface — the one a
/// `Proof:` line and every tactic in this tier use — has no `ring` at all, and the three
/// vernacular surfaces do not interoperate.
#[test]
fn ring_normalises_a_polynomial_identity_from_a_script() {
    let goal = eq(
        app("mul", vec![c("a"), app("add", vec![c("b"), c("cc")])]),
        app(
            "add",
            vec![
                app("mul", vec![c("a"), c("b")]),
                app("mul", vec![c("a"), c("cc")]),
            ],
        ),
    );
    let r = prove(vec![], goal, "ring");
    assert!(
        matches!(r, Ok(true)),
        "`ring` must be reachable from a tactic script — distributivity is the shape a \
         polynomial normaliser exists for: {r:?}"
    );
}

// ============================================================================================
// T08-016 · `linarith` — an alias for `omega`, which is unsound over an ordered field
// ============================================================================================

/// **T08-016 · MUST / Gap — WIRING, and the tier's soundness case.** `canonical` maps
/// `linarith` to `omega`, so the integer discreteness step fires under the name of the
/// rational solver. `x < y` and `y < x + 1` are UNSAT over ℤ and SAT over ℚ (`x=0, y=½`);
/// Lean's `linarith` correctly derives nothing from them, ours refutes them and discharges
/// ANY goal by ex-falso. Nothing in `ProofExpr` records the carrier, so the kernel cannot
/// catch it — it is checking a correct ℤ proof of a statement that was never about ℤ.
/// The rational core with Farkas certificates is BUILT (`linarith_solve` + `cert_farkas`);
/// it is simply not reachable under its own name.
#[test]
fn linarith_is_rational_and_declines_an_integer_only_refutation() {
    let premises = vec![("h1", lt(c("a"), c("b"))), ("h2", lt(c("b"), add1(c("a"))))];
    let mut st = state(premises, pred("Q", vec![c("A")]));
    let ran = st.run_script("linarith").is_ok();
    let certified = ran && st.qed().map(|r| r.verified).unwrap_or(false);
    assert!(
        !certified,
        "`linarith` must be the RATIONAL solver: `a < b ∧ b < a+1` is satisfiable over every \
         dense ordered field, so deriving anything from it is unsound for the carrier the \
         source named — and nothing in ProofExpr records that carrier"
    );
}

/// The wiring evidence: the rational core gets the right answer on exactly that system. It is
/// the NAME that is mis-wired, not the mathematics.
#[test]
fn the_rational_farkas_core_finds_no_certificate_for_a_dense_system() {
    use logicaffeine_proof::linarith_solve::{find_farkas, parse_lin};
    let a_minus_b = parse_lin(&app("sub", vec![c("a"), c("b")])).expect("linear");
    let b_minus_a1 = parse_lin(&app("sub", vec![c("b"), add1(c("a"))])).expect("linear");
    assert!(
        find_farkas(&[a_minus_b, b_minus_a1]).is_none(),
        "the rational Fourier-Motzkin core must report `a < b ∧ b < a+1` SATISFIABLE — that is \
         the answer `linarith` owes, and it is already computed"
    );
}

// ============================================================================================
// T08-017 · `exists` / `use` — the witness is always a bare `Constant`
// ============================================================================================

/// **T08-017 · MUST / Gap.** Lean's witness is an arbitrary term: `use (n + 1)`, `use (f x)`,
/// `use ⟨a, b⟩`. The script's `arg()` reads exactly one word and the lexer rejects `+`
/// outright, so a compound witness cannot be tokenised — and `,` is a step SEPARATOR, so
/// `use a, b` for nested existentials is not one step either.
#[test]
fn exists_accepts_a_compound_witness_term() {
    let goal = ex("m", eq(v("m"), add1(v("n"))));
    let r = prove(vec![], goal, "exists n + 1 . simp");
    assert!(
        matches!(r, Ok(true)),
        "`exists (n + 1)` must supply a compound witness — the script lexer cannot even \
         tokenise the `+`: {r:?}"
    );
}

/// Not a corpus case of its own — a defect the tier turned up, recorded where it will be seen.
/// `skip_filler` runs INSIDE `arg()` and discards thirteen connectives case-insensitively, so an
/// argument spelled `a`, `an`, `it`, `we`, `on`, `of`, `to`, `the`, `that`, `now`, `with`, `by`
/// or `and` never reaches the tactic. `exists A` and `cases it` are unspellable, and the reported
/// error blames the tactic for having no argument rather than the vocabulary for eating it.
#[test]
fn a_witness_or_hypothesis_named_like_a_filler_word_is_swallowed() {
    let mut swallowed = Vec::new();
    for word in ["A", "an", "it", "we", "On", "of", "to"] {
        if parse_script(&format!("exists {word}")).is_err() {
            swallowed.push(word);
        }
    }
    assert!(
        swallowed.is_empty(),
        "these are ordinary identifiers and must be usable as tactic arguments; the filler-word \
         vocabulary eats them before `arg()` sees them: {swallowed:?}"
    );
}

/// THE VARIABLE-BECOMES-CONSTANT TRAP. `intro n` binds the introduced variable as
/// `ProofTerm::Variable("n")`; `exists n` wraps its argument in `ProofTerm::Constant`
/// unconditionally. The instantiated goal then mentions `Constant("n")` while the context
/// mentions `Variable("n")`, the derived `PartialEq` finds no match, and both terms `Display`
/// as `n` — so the difference is invisible in every diagnostic the user sees.
#[test]
fn exists_reusing_an_introduced_variable_matches_its_binding() {
    let goal = forall("n", ex("m", eq(v("m"), v("n"))));
    let r = prove(vec![], goal, "intro n . exists n . simp");
    assert!(
        matches!(r, Ok(true)),
        "a witness that names an `intro`-bound variable must BE that variable; `exists` makes \
         it a Constant and the two print identically: {r:?}"
    );
}

/// The closed-witness fragment, pinned — and the trailing goal Lean's `exists` would have
/// discharged itself and ours always leaves open.
#[test]
fn exists_instantiates_the_body_and_leaves_it_open() {
    let goal = ex("n", p_of(v("n")));
    let mut st = state(vec![("hp", p_of(c("Sk")))], goal);
    st.run_script("exists Sk").expect("a closed atomic witness");
    assert_eq!(
        st.open_goals(),
        1,
        "`exists` never attempts a trailing `rfl`; the instantiated goal stays open"
    );
    assert_eq!(
        st.focused_target(),
        Some(&p_of(c("Sk"))),
        "the body must be instantiated at the witness, got {:?}",
        st.focused_target()
    );
    st.run_script("assumption")
        .expect("the instantiated body is a premise");
    assert!(
        st.qed().expect("complete").verified,
        "∃-introduction must certify"
    );
}

// ============================================================================================
// T08-018 · `<;>` — the open-set leak
// ============================================================================================

/// **T08-018 · MUST / Gap.** Lean's `<;>` applies the right-hand tactic to exactly the goals
/// the left-hand one produced. `then_all` runs `all_goals`, which iterates the COMPLETE open
/// deque — including goals a branching step two rounds earlier left waiting. When the
/// right-hand tactic happens to discharge those too, the script succeeds and is one goal
/// further along than the proof believes; when it does not, the failure propagates for a goal
/// the source never asked it to look at.
#[test]
fn then_all_touches_only_the_goals_its_left_hand_produced() {
    let goal = and(or(prop_p(), prop_q()), prop_q());
    let mut st = state(vec![("hp", prop_p()), ("hq", prop_q())], goal);
    st.run_script("split . left <;> assumption")
        .expect("the chain runs");
    assert_eq!(
        st.open_goals(),
        1,
        "`<;>` must apply `assumption` only to the goal `left` produced — the right conjunct of \
         the `split` was still open and must remain so"
    );
    st.run_script("assumption")
        .expect("the right conjunct discharges");
    assert!(
        st.qed().expect("complete").verified,
        "the Lean-shaped script must certify"
    );
}

/// The shape where the two agree — `<;>` as the FIRST branching step, where the open set holds
/// only the left-hand tactic's output. Almost every textbook use has this shape, which is why
/// the leak above survives a large test suite.
#[test]
fn then_all_agrees_with_lean_when_it_is_the_first_branching_step() {
    let r = prove(
        vec![("hp", prop_p())],
        and(prop_p(), prop_p()),
        "split <;> assumption",
    );
    assert!(
        matches!(r, Ok(true)),
        "`split <;> assumption` must certify: {r:?}"
    );

    let goal = and(and(prop_p(), prop_q()), and(prop_r(), prop_p()));
    let premises = vec![("hp", prop_p()), ("hq", prop_q()), ("hr", prop_r())];
    let r = prove(premises, goal, "split <;> split <;> assumption");
    assert!(matches!(r, Ok(true)), "a chained `<;>` must certify: {r:?}");
}

// ============================================================================================
// T08-019 · `first` / `try` / `repeat` — full parity
// ============================================================================================

/// **T08-019 · MUST / Native.** All three exist, speculate on a clone of the state, and commit
/// only on success — the same semantics as Lean's. `first` backtracks over whole alternatives,
/// `try` and `repeat` always succeed, and `repeat` stops when its body stops applying. The
/// spelling differs (`first [ a | b ]` rather than Lean's leading bar) and the three keywords
/// are matched case-sensitively where every primitive is not; neither changes what can be
/// proved.
#[test]
fn first_try_and_repeat_speculate_and_commit_like_leans() {
    let r = prove(
        vec![("hp", prop_p())],
        prop_p(),
        "first [ omega | assumption ]",
    );
    assert!(
        matches!(r, Ok(true)),
        "`first` must fall through to the alternative: {r:?}"
    );

    let r = prove(vec![("hp", prop_p())], prop_p(), "try omega . assumption");
    assert!(
        matches!(r, Ok(true)),
        "`try` on an inapplicable tactic must be a no-op: {r:?}"
    );

    let goal = implies(prop_p(), implies(prop_q(), prop_r()));
    let r = prove(vec![("hr", prop_r())], goal, "repeat intro h . assumption");
    assert!(
        matches!(r, Ok(true)),
        "`repeat` must strip both binders and stop: {r:?}"
    );

    // An alternative that applies a step and THEN fails must leave no trace.
    let premises = vec![("hp", prop_p()), ("hq", prop_q())];
    let r = prove(
        premises,
        and(prop_p(), prop_q()),
        "first [ (split . omega) | (split <;> assumption) ]",
    );
    assert!(
        matches!(r, Ok(true)),
        "`first` must roll a partially-applied alternative back entirely: {r:?}"
    );

    let r = prove(
        vec![("hp", prop_p())],
        prop_p(),
        "repeat omega . assumption",
    );
    assert!(
        matches!(r, Ok(true)),
        "`repeat` on a tactic that never applies is a no-op: {r:?}"
    );
}

// ============================================================================================
// T08-020 · `all_goals` / `any_goals`
// ============================================================================================

/// **T08-020 · MUST / Gap — HALF WIRING.** `combinators::all_goals` is built and tested; no
/// script text constructs it, because `<;>` is its only script-reachable consumer and that
/// always runs a left-hand tactic first. `any_goals` is different in kind: a grep of the whole
/// repository finds it NOWHERE — not in the grammar and not in Rust — so it must be built, and
/// substituting `all_goals` for it turns a permissive step into a strict one.
#[test]
fn all_goals_and_any_goals_are_spellable_in_a_script() {
    let premises = vec![("hp", prop_p()), ("hq", prop_q())];
    let r = prove(
        premises,
        and(prop_p(), prop_q()),
        "split . all_goals assumption",
    );
    assert!(
        matches!(r, Ok(true)),
        "`all_goals t` must be reachable from script text — the Rust combinator already does \
         exactly this: {r:?}"
    );

    let goal = and(prop_p(), eq(c("0"), c("0")));
    let r = prove(
        vec![("hp", prop_p())],
        goal,
        "split . any_goals assumption . decide",
    );
    assert!(
        matches!(r, Ok(true)),
        "`any_goals t` must succeed when `t` closes SOME goal and leave the rest — it exists \
         nowhere in the repository and has to be built: {r:?}"
    );
}

/// The wiring evidence for the first half: from Rust, `all_goals` closes every open goal and
/// the proof certifies.
#[test]
fn the_rust_all_goals_already_applies_a_tactic_to_every_goal() {
    let mut st = ProofState::start(vec![prop_p(), prop_q()], and(prop_p(), prop_q()));
    st.run(&split_t()).expect("∧-introduction");
    st.run(&all_goals(assumption_t()))
        .expect("both goals close at once");
    assert_eq!(st.open_goals(), 0, "all_goals must consume every open goal");
    let r = st.qed().expect("complete");
    assert!(
        r.verified,
        "all_goals must certify: {:?}",
        r.verification_error
    );
}

// ============================================================================================
// T08-021 · `calc`
// ============================================================================================

/// **T08-021 · MUST / Gap.** `calc` exists on no LOGOS surface. The pure-`=` chain expands
/// into `rewrite` steps (see the companion), but a MIXED chain does not: Lean's `Trans`
/// composes `=` with `<` with `≤` and computes the chain's overall relation, where LOGOS has
/// eight individual `Int` order axioms and nothing to compose them with. `calc` is also an
/// EXPRESSION in Lean 4, so a translator meets it while translating a term, before it has
/// chosen a proof surface at all.
#[test]
fn calc_composes_a_mixed_relation_chain() {
    let premises = vec![
        ("h1", eq(c("a"), c("b"))),
        ("h2", lt(c("b"), c("cc"))),
        ("h3", le(c("cc"), c("d"))),
    ];
    let r = prove(premises, lt(c("a"), c("d")), "auto");
    assert!(
        matches!(r, Ok(true)),
        "`a = b`, `b < c`, `c ≤ d` must compose into `a < d` — that composition is what `calc` \
         and Lean's `Trans` are for, and LOGOS has neither: {r:?}"
    );
}

/// The degenerate, pure-`=` chain, which DOES have an expansion — so the diagnostic for a
/// `calc` must be driven by chain length or by a non-`=` relation, not by the word.
#[test]
fn a_pure_equality_chain_expands_into_rewrite_steps() {
    let premises = vec![
        ("h1", eq(c("a"), c("b"))),
        ("h2", eq(c("b"), c("cc"))),
        ("h3", eq(c("cc"), c("d"))),
    ];
    let r = prove(
        premises,
        eq(c("a"), c("d")),
        "rewrite h1 . rewrite h2 . rewrite h3 . simp",
    );
    assert!(
        matches!(r, Ok(true)),
        "a pure-= chain must certify through rewrites: {r:?}"
    );
}

// ============================================================================================
// T08-022 · `by_contra` / `push_neg`
// ============================================================================================

/// **T08-022 · MUST / Gap.** Lean's `by_contra h` is a STEP: it adds `h : ¬G` and leaves
/// `False` for the rest of the block. LOGOS's classical reductio is a SEARCH STRATEGY inside
/// `auto`, tried only after everything else, only at depth 0, and only for a positive
/// non-`False` goal — so a Lean proof shaped `intro x; by_contra h; <twenty steps>` must hand
/// the whole subproof to `auto` as one opaque call, with no way to help it if it fails.
#[test]
fn by_contra_is_a_step_that_leaves_false_as_the_goal() {
    let premises = vec![
        ("h", implies(not(prop_p()), prop_q())),
        ("hnq", not(prop_q())),
    ];
    let r = prove(premises, prop_p(), "by_contra hnp . auto");
    assert!(
        matches!(r, Ok(true)),
        "`by_contra h` must be a STEP that assumes ¬G and leaves ⊥, so the rest of the proof \
         can be written: {r:?}"
    );
}

/// The capability that does exist, inside `auto`: double-negation elimination through the
/// single classical axiom `dne`, certified as a `ClassicalReductio` node.
#[test]
fn auto_reaches_classical_reductio_for_a_positive_goal() {
    let mut st = state(vec![("h", not(not(prop_p())))], prop_p());
    st.run_script("auto")
        .expect("the backward chainer closes ¬¬p ⊢ p");
    let r = st.qed().expect("complete");
    assert!(
        r.verified,
        "double-negation elimination must certify: {:?}",
        r.verification_error
    );
    let tree = st.assembled().expect("the proof is complete");
    assert!(
        uses_rule(&tree, &InferenceRule::ClassicalReductio)
            || uses_rule(&tree, &InferenceRule::DoubleNegation),
        "the derivation must record that it went classical — the axiom footprint is part of \
         the theorem: {:?}",
        tree.rule
    );
}

// ============================================================================================
// T08-023 · `conv` — positional rewriting
// ============================================================================================

/// **T08-023 · MUST / Gap.** `conv` exists in Lean precisely because `rw` is not positional
/// enough. LOGOS's only rewriting primitive is `replace_in_expr`, which takes no position, no
/// occurrence index and no early exit — and `InferenceRule::Rewrite` carries only the two
/// terms, so even a Rust-level caller has nowhere to record which occurrence it meant. Here
/// the all-occurrences behaviour destroys the proof: rewriting only the FIRST argument leaves
/// exactly the hypothesis, rewriting both leaves something nothing proves.
#[test]
fn rewriting_one_selected_occurrence_leaves_the_others_alone() {
    let premises = vec![
        ("h", eq(c("a"), c("b"))),
        ("hp", pred("P", vec![c("b"), c("a")])),
    ];
    let goal = pred("P", vec![c("a"), c("a")]);
    let mut st = state(premises, goal);
    st.rewrite("h")
        .expect("the all-occurrences rewrite applies");
    assert_eq!(
        st.focused_target(),
        Some(&pred("P", vec![c("b"), c("a")])),
        "a positional rewrite must change only the occurrence it navigates to; LOGOS rewrote \
         every occurrence and produced {:?}, which the hypothesis P(b, a) no longer closes",
        st.focused_target()
    );
    st.run_script("assumption")
        .expect("the rewritten goal IS the hypothesis");
    assert!(
        st.qed().expect("complete").verified,
        "the positional rewrite must certify"
    );
}

// ============================================================================================
// T08-024 · `tauto` / `aesop` ⟼ `auto`
// ============================================================================================

/// **T08-024 · MUST / Native.** Lean's `tauto` and `aesop` map onto our `auto` — the backward
/// chainer over the in-scope hypotheses — with `crush` (a grind-class e-matching plus
/// congruence-closure closer) behind it. The searches have different shapes and neither
/// contains the other, but on the goals this case names ours closes them and the KERNEL
/// certifies the result, which is the bar. Two things a translator must record and the tests
/// below pin: `auto` may reach the classical axiom `dne` where Lean's `tauto` stays
/// constructive, and `ProofState::auto` returning `Ok` means a derivation was BUILT — only
/// `qed()` consults the kernel.
#[test]
fn tauto_and_aesop_goals_close_under_our_automation() {
    let cases: Vec<(&str, Vec<(&str, ProofExpr)>, ProofExpr)> = vec![
        (
            "propTauto",
            vec![("hp", prop_p()), ("hq", prop_q())],
            and(prop_p(), prop_q()),
        ),
        ("classicalTauto", vec![], or(prop_p(), not(prop_p()))),
        (
            "aesopMixed",
            vec![
                ("h", or(prop_p(), prop_q())),
                ("hpq", implies(prop_p(), prop_q())),
            ],
            prop_q(),
        ),
        (
            "irrelevantHypothesis",
            vec![
                ("hp", prop_p()),
                ("hr", prop_r()),
                ("hpq", implies(prop_p(), prop_q())),
            ],
            prop_q(),
        ),
        (
            "hardForAuto",
            vec![
                ("hp", prop_p()),
                ("h1", implies(prop_p(), prop_q())),
                ("h2", implies(prop_q(), prop_r())),
            ],
            and(prop_p(), and(prop_q(), prop_r())),
        ),
    ];
    let mut unclosed = Vec::new();
    for (name, premises, goal) in cases {
        let r = prove(premises, goal, "first [ auto | crush ]");
        if !matches!(r, Ok(true)) {
            unclosed.push(format!("{name}: {r:?}"));
        }
    }
    assert!(
        unclosed.is_empty(),
        "every `tauto`/`aesop` goal must close under our automation and be KERNEL-certified — a \
         script that merely ran proves nothing:\n  {}",
        unclosed.join("\n  ")
    );
}

// ============================================================================================
// T08-025 · `·` focusing, `case`, `next`
// ============================================================================================

/// **T08-025 · MUST / Gap.** A script is a FLAT list of chains. Because `refine` pushes new
/// subgoals to the front, execution is depth-first in exactly the order Lean's bullets appear
/// — so stripping the bullets and concatenating the blocks WORKS, right up until a block
/// leaves a goal open or closes two, at which point the blocks silently misalign and (with
/// generic enough tactics) the proof still succeeds, having proved the goals in a pairing the
/// source never specified. Lean's `·` makes that impossible: a focused block that does not
/// close its goal is an error. `Goal` additionally carries no name or tag, so `case inl => …`
/// has nothing to select on.
#[test]
fn a_focused_block_must_close_exactly_the_goal_it_opened() {
    let premises = vec![("hp", prop_p()), ("hq", prop_q())];
    let mut st = state(premises, and(prop_p(), prop_q()));
    let ran = st.run_script("split . focus (exact hp) . focus (exact hq)");
    assert!(
        ran.is_ok(),
        "a focusing construct (`·` / `focus` / `case` / `next`) must exist, and must make it an \
         ERROR for a block to leave its goal open: {:?}",
        ran.err()
    );
    assert!(
        st.qed().expect("complete").verified,
        "the focused proof must certify"
    );
}

// ============================================================================================
// T08-026 · `have` / `show` / `suffices`
// ============================================================================================

/// **T08-026 · MUST / Gap.** Nothing in the fifteen primitives introduces a hypothesis with
/// an accompanying proof: `intro` takes one from the goal's antecedent, `cases` from an
/// existing hypothesis's shape, and that is the complete set of ways a name enters
/// `Goal::hyps`. The suggested lift — one `ScriptedTheorem` per `have`, cited by name — only
/// works for a CLOSED statement, because `prove_scripted_library` assembles each theorem's
/// premises as a flat list before the proof starts. A `have` mentioning an `intro`-bound
/// variable, as here, cannot be lifted at all.
#[test]
fn have_introduces_a_locally_proved_hypothesis() {
    let premises = vec![("h", forall("m", p_of(v("m"))))];
    let goal = forall("n", and(p_of(v("n")), p_of(v("n"))));
    let r = prove(
        premises,
        goal,
        "intro n . have hn from h . split . exact hn . exact hn",
    );
    assert!(
        matches!(r, Ok(true)),
        "`have h : T := …` must be a STEP that adds a locally-proved hypothesis, including one \
         whose statement mentions an `intro`-bound variable: {r:?}"
    );
}

// ============================================================================================
// T08-027 · `refine` with `?` holes
// ============================================================================================

/// **T08-027 · MUST / Gap.** There is no `refine` atom, and the script lexer has no `?` at
/// all — `?_` falls into the "unexpected character" arm, so a hole cannot even be tokenised.
/// The tempting term-level workaround is forbidden for a separate and worse reason: the
/// kernel's `types_equal` treats `Term::Hole` as matching ANYTHING, so a term with a hole in
/// it typechecks and the resulting certificate is indistinguishable from a real one. A
/// `refine` must therefore spawn PROOF-STATE goals, the way `ProofState`'s own `Node::Hole`
/// does, and never a term hole.
#[test]
fn refine_with_holes_spawns_one_goal_per_hole() {
    let lexed = parse_script("refine ?_");
    assert!(
        lexed.is_err(),
        "today the lexer cannot even tokenise a `?` hole — this pins the starting point"
    );

    let premises = vec![("hp", prop_p()), ("hq", prop_q())];
    let r = prove(
        premises,
        and(prop_p(), prop_q()),
        "refine (?_ , ?_) . exact hp . exact hq",
    );
    assert!(
        matches!(r, Ok(true)),
        "`refine ⟨?_, ?_⟩` must spawn one goal per hole, in the order the holes appear: {r:?}"
    );
}

// ============================================================================================
// T08-028 · `sorry` / `admit` — the refusal IS the capability
// ============================================================================================

/// **T08-028 · OUT / Refused.** Every encoding of an admitted proof is worse than refusing.
/// `add_declaration("sorryAx", T)` is one line, compiles, resolves through `get_global`
/// exactly like `dne` or `le_trans`, and — LOGOS having no attribute system and no
/// `#print axioms` analogue — leaves no marker at all. `Term::Hole` is worse still, because
/// `types_equal` matches it against anything so the gap does not even localise. A
/// `verified == false` artefact defers the "this is not a proof" call to whoever reads the
/// report. This test asserts the clean refusal: the words are unknown, a parse error leaves
/// the state untouched, open goals cannot be `qed`'d, and a derivation the kernel rejects is
/// never an acceptance.
#[test]
fn sorry_is_refused_and_smuggles_nothing() {
    for word in ["sorry", "admit", "sorryAx"] {
        assert!(
            parse_script(word).is_err(),
            "`{word}` must not be a tactic name — an admitted proof has no sound encoding"
        );
    }

    // A parse error leaves the state exactly as it was: a stray `sorry` cannot half-commit.
    let mut st = state(vec![("hp", prop_p())], and(prop_p(), prop_q()));
    assert!(
        st.run_script("split . exact hp . sorry").is_err(),
        "the script must be rejected"
    );
    assert_eq!(
        st.open_goals(),
        1,
        "a rejected script must not have run any of its steps"
    );

    // The partial proof cannot be finished, and `qed` says so by counting.
    let mut st = state(vec![("hp", prop_p())], and(prop_p(), prop_q()));
    st.run_script("split . exact hp")
        .expect("the real half of the proof");
    let finished = st.qed();
    assert!(
        finished.is_err(),
        "a proof with a goal still open must not produce a VerifiedProof at all: {:?}",
        finished.map(|r| r.verified)
    );

    // And the third encoding — hand the caller a derivation the kernel refuses — is not an
    // acceptance either: `verified` is the only word that counts.
    use logicaffeine_proof::verify::check_derivation;
    let lie = lt(c("5"), c("2"));
    let tree = DerivationTree::leaf(lie.clone(), InferenceRule::NativeDecide);
    let r = check_derivation(&[], &lie, tree);
    assert!(
        !r.verified,
        "a derivation that does not typecheck must report verified == false, never be passed \
         off as a proof"
    );
}
