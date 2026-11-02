//! LANE T_tactics — the behaviour the corpus does NOT ask about.
//!
//! Five corpus cases named five capabilities. Each fix has a neighbourhood the case never
//! mentions, and that neighbourhood is where the bugs live, so every capability here comes as
//! a PAIR: the thing that must now be accepted, and the neighbouring thing that must still be
//! refused. A positive-only test is satisfiable by a weak implementation — `decide` could
//! "settle" `¬(2 = 3)` by settling every negation — and a positive/negative pair is not.
//!
//! Three kinds of assertion appear throughout:
//!
//! * **`verified == true`** — the KERNEL's word, through `qed()`/`check_derivation`. A tactic
//!   that returns `Ok` has built an object, not proved a theorem.
//! * **A hand-MUTATED certificate must be refused.** The `proof_certificate_lock` pattern:
//!   take the golden tree, change one field to a lie, and require the checker to flip. This is
//!   what distinguishes a rule that re-derives its claim from one that is believed.
//! * **`double_check` must never `Disagree`.** Two independently-written kernels; a
//!   disagreement is the soundness alarm a second kernel exists to raise. `Agreed` is the
//!   strong outcome and is asserted where the term is in the re-checker's fragment.

use logicaffeine_kernel::{double_check, DoubleCheck};
use logicaffeine_proof::tactic::{combinators as c, ProofState};
use logicaffeine_proof::verify::{check_derivation, close_over_context_locals};
use logicaffeine_proof::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

// ============================================================================================
// HELPERS — per-file, as house style requires.
// ============================================================================================

fn k(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn v(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn f(name: &str, args: Vec<ProofTerm>) -> ProofTerm {
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
fn not(p: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(p))
}
fn or(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(l), Box::new(r))
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
/// `a ≤ b` in the system's canonical encoding: `Identity(le(a, b), true)`.
fn le(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    eq(f("le", vec![a, b]), k("true"))
}
/// `a < b` in the same encoding.
fn lt(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    eq(f("lt", vec![a, b]), k("true"))
}
fn add1(t: ProofTerm) -> ProofTerm {
    f("add", vec![t, k("1")])
}

/// A named-premise proof state, so a script can say `exact h`.
fn state(premises: Vec<(&str, ProofExpr)>, goal: ProofExpr) -> ProofState {
    let names: Vec<Option<String>> = premises
        .iter()
        .map(|(n, _)| Some((*n).to_string()))
        .collect();
    let props: Vec<ProofExpr> = premises.into_iter().map(|(_, p)| p).collect();
    ProofState::start_with_names(props, &names, goal)
}

/// Run `script` and report whether the KERNEL certified it.
fn prove(premises: Vec<(&str, ProofExpr)>, goal: ProofExpr, script: &str) -> Result<bool, String> {
    let mut st = state(premises, goal);
    st.run_script(script)
        .map_err(|e| format!("script: {}", e.0))?;
    st.qed()
        .map(|r| r.verified)
        .map_err(|e| format!("qed: {e:?}"))
}

/// Run `script`, require the kernel to certify it, and require the SECOND kernel not to
/// disagree — then hand back the assembled tree so a test can assert which rules carried it.
fn prove_tree(premises: Vec<(&str, ProofExpr)>, goal: ProofExpr, script: &str) -> DerivationTree {
    let mut st = state(premises, goal);
    st.run_script(script)
        .unwrap_or_else(|e| panic!("script `{script}` must apply: {}", e.0));
    let r = st.qed().expect("no goals may remain");
    assert!(
        r.verified,
        "the kernel must certify `{script}`: {:?}",
        r.verification_error
    );
    let term = r.proof_term.as_ref().expect("a verified proof has a term");
    // The re-checker has no notion of a context's LOCAL bindings, so a statement with a free
    // variable must be handed to both kernels in its closed form — see
    // `close_over_context_locals`.
    let closed = close_over_context_locals(&r.kernel_ctx, term);
    assert!(
        !matches!(
            double_check(&r.kernel_ctx, &closed),
            DoubleCheck::Disagree(_)
        ),
        "the independent re-checker must never DISAGREE with a verified proof: {:?}",
        double_check(&r.kernel_ctx, &closed)
    );
    st.assembled().expect("a complete proof assembles")
}

/// True iff `tree` (or some node below it) was justified by `rule`'s discriminant.
fn uses_rule(tree: &DerivationTree, want: &InferenceRule) -> bool {
    if std::mem::discriminant(&tree.rule) == std::mem::discriminant(want) {
        return true;
    }
    tree.premises.iter().any(|p| uses_rule(p, want))
}

/// One past `i64::MAX` — the old evaluator window, and the value the kernel's own `Literal`
/// has always been able to hold.
const PAST_I64: &str = "9223372036854775808";
const PAST_I64_PLUS_1: &str = "9223372036854775809";

// ============================================================================================
// T08-013 · `omega` — the goal-directed half, and what must still be declined
// ============================================================================================

/// The capability, with the ROUTE pinned: the totality split is what makes a positive goal
/// reachable, so the certificate must actually contain it. A test that only checked
/// `Ok(true)` would be satisfied by an `omega` that got lucky some other way.
#[test]
fn omega_decides_a_positive_le_goal_through_the_totality_split() {
    let tree = prove_tree(vec![("h", le(k("x"), k("3")))], le(k("x"), k("4")), "omega");
    assert!(
        uses_rule(&tree, &InferenceRule::LeTotal),
        "the goal-directed half runs on `le_total`, which is already in the trusted base: {tree}"
    );
    assert!(
        uses_rule(&tree, &InferenceRule::DisjunctionCases),
        "totality is useless without the case split that consumes it: {tree}"
    );
}

/// THE NEGATIVE. A decision procedure that proves a goal by splitting on totality can be
/// written wrong in exactly one direction — by treating the left branch as discharged instead
/// of as the goal — and then it proves anything. `x ≤ 3 ⊢ 4 ≤ x` is FALSE (`x = 0`), and
/// `x ≤ 3 ⊢ x ≤ 2` is false too (`x = 3`); neither may be certified.
#[test]
fn omega_refuses_a_le_goal_that_does_not_follow() {
    for goal in [le(k("4"), k("x")), le(k("x"), k("2"))] {
        let r = prove(vec![("h", le(k("x"), k("3")))], goal.clone(), "omega");
        assert!(
            !matches!(r, Ok(true)),
            "`x ≤ 3` has integer models refuting {goal:?}, so `omega` must NOT certify it: {r:?}"
        );
    }
}

/// The zero-slack boundary, one rung below the slack case: the goal IS the hypothesis. This
/// must close on the direct route, because the totality split cannot reach it — `x ≤ 3`
/// together with `3 ≤ x` is perfectly satisfiable (`x = 3`).
#[test]
fn omega_proves_the_zero_slack_goal_directly() {
    let r = prove(vec![("h", le(k("x"), k("3")))], le(k("x"), k("3")), "omega");
    assert!(
        matches!(r, Ok(true)),
        "`x ≤ 3 ⊢ x ≤ 3` is the hypothesis itself: {r:?}"
    );
}

/// One rung above: a CHAIN. Neither operand is a literal and the goal is not a hypothesis, so
/// this is `le_trans` composition, not a premise match.
#[test]
fn omega_chains_le_hypotheses_into_a_positive_goal() {
    let premises = vec![("h1", le(k("a"), k("b"))), ("h2", le(k("b"), k("cc")))];
    let r = prove(premises, le(k("a"), k("cc")), "omega");
    assert!(
        matches!(r, Ok(true)),
        "`a ≤ b ≤ cc ⊢ a ≤ cc` must compose: {r:?}"
    );
}

/// The gate. `omega` used to require an arithmetic HYPOTHESIS, so Lean's no-hypothesis cases
/// (`⊢ n + 0 = n`, `⊢ 2 ≤ 3`) were declined for the absence of something they do not need.
#[test]
fn omega_decides_goals_with_no_hypotheses_at_all() {
    let r = prove(vec![], le(k("2"), k("3")), "omega");
    assert!(
        matches!(r, Ok(true)),
        "a ground comparison is decided by computation: {r:?}"
    );

    let r = prove(vec![], eq(f("add", vec![k("n"), k("0")]), k("n")), "omega");
    assert!(
        matches!(r, Ok(true)),
        "`n + 0 = n` is a linear goal Lean's `omega` proves: {r:?}"
    );
}

/// …and the gate must still be a gate. A goal with no arithmetic content at all, and no
/// arithmetic hypotheses, is not `omega`'s business, and the message must say so about the
/// GOAL rather than about the hypotheses.
#[test]
fn omega_declines_a_goal_with_no_arithmetic_content() {
    let mut st = state(vec![], pred("Q", vec![k("A")]));
    let err = match st.run_script("omega") {
        Err(e) => e,
        Ok(_) => panic!("there is nothing linear to decide here"),
    };
    assert!(
        err.0.contains("not an arithmetic goal"),
        "the decline must name the GOAL, not blame absent hypotheses: {}",
        err.0
    );
}

/// The refutation half, unchanged — and the proof that "goal-directed" was ADDED rather than
/// substituted. A contradictory hypothesis set still discharges a goal with no arithmetic
/// content whatever.
#[test]
fn omega_still_refutes_and_discharges_any_goal() {
    let premises = vec![("h1", le(k("5"), k("x"))), ("h2", le(k("x"), k("3")))];
    let r = prove(premises, pred("Q", vec![k("A")]), "omega");
    assert!(
        matches!(r, Ok(true)),
        "`5 ≤ x ≤ 3` is UNSAT and closes anything: {r:?}"
    );
}

/// SOUNDNESS, the mutated certificate. `LeTotal` is a LEAF: nothing about it is checked by a
/// premise, so everything about it must be checked by its conclusion. A node claiming a
/// disjunction that is not an instance of `a ≤ b ∨ b ≤ a` must be refused — otherwise the
/// rule is a licence to assert any disjunction of two inequalities.
#[test]
fn a_le_total_leaf_that_is_not_totality_is_refused() {
    let goal = or(le(k("1"), k("2")), le(k("7"), k("3")));
    // The mismatched pair: `1 ≤ 2 ∨ 7 ≤ 3` is not `le_total` of anything (`7 ≤ 3` is false,
    // and totality's second disjunct must be the first one flipped).
    let bogus = DerivationTree::leaf(goal.clone(), InferenceRule::LeTotal);
    let r = check_derivation(&[], &goal, bogus);
    assert!(
        !r.verified,
        "a `LeTotal` leaf whose disjuncts are not one flipped pair must be refused — it would \
         otherwise certify `7 ≤ 3`: {:?}",
        r.proof_term
    );

    // …and the golden object it was mutated from IS accepted, so the refusal above is about
    // the mutation and not about `LeTotal` being dead.
    let good = or(le(k("1"), k("2")), le(k("2"), k("1")));
    let golden = DerivationTree::leaf(good.clone(), InferenceRule::LeTotal);
    let r = check_derivation(&[], &good, golden);
    assert!(
        r.verified,
        "`1 ≤ 2 ∨ 2 ≤ 1` IS totality: {:?}",
        r.verification_error
    );
}

// ============================================================================================
// T08-014 · `decide` — the decidable fragment, and the decline that must stay a decline
// ============================================================================================

/// Ground DISEQUALITY, which had no arm at all. The route is pinned because there are two
/// ways to build it and only one of them stays inside the trusted base: `GroundRefutation`
/// goes through `Bool`'s no-confusion discriminator, where an `of_decide_eq_false` /
/// `Decidable (Eq Int _ _)` route would have been a NEW AXIOM.
#[test]
fn decide_settles_a_ground_int_disequality() {
    let tree = prove_tree(vec![], not(eq(k("2"), k("3"))), "decide");
    assert!(
        uses_rule(&tree, &InferenceRule::GroundRefutation),
        "the refutation channel must be the one that needs no new axiom: {tree}"
    );
}

/// THE NEGATIVE, and the dangerous one. `¬(2 = 2)` is FALSE. A `decide` that reached the
/// refutation channel by "the equality is not syntactically reflexive" would prove it.
#[test]
fn decide_refuses_the_negation_of_a_true_equality() {
    for goal in [
        not(eq(k("2"), k("2"))),
        not(eq(f("add", vec![k("1"), k("1")]), k("2"))),
    ] {
        let r = prove(vec![], goal.clone(), "decide");
        assert!(
            r.is_err(),
            "{goal:?} is FALSE and must be declined, never guessed: {r:?}"
        );
    }
}

/// The two declines are two different answers, and conflating them is what lets a search walk
/// past a false goal and close it later from an unrelated contradiction. `decide` knows which
/// it is; the message must say which it is.
#[test]
fn decide_distinguishes_a_false_goal_from_one_outside_the_fragment() {
    let mut false_goal = state(vec![], lt(k("5"), k("2")));
    let e1 = match false_goal.run_script("decide") {
        Err(e) => e,
        Ok(_) => panic!("`5 < 2` is FALSE and must never be proved"),
    };
    assert!(
        e1.0.contains("FALSE"),
        "a goal decide EVALUATED and found false must say so: {}",
        e1.0
    );

    let mut open_goal = state(vec![], le(k("x"), k("3")));
    let e2 = match open_goal.run_script("decide") {
        Err(e) => e,
        Ok(_) => panic!("`x ≤ 3` is not ground and must not be decided"),
    };
    assert!(
        e2.0.contains("not a CLOSED goal"),
        "a goal decide could not evaluate must say THAT, not that it is false: {}",
        e2.0
    );
    assert_ne!(e1.0, e2.0, "the two situations must not share one message");
}

/// THE NUMERAL BOUNDARY. Both kernels compute integers exactly; four proof-layer functions
/// each had their own `parse::<i64>`, and together they made `i64::MAX` the system's
/// arithmetic ceiling — invisible from the Lean source and absent from the kernel.
#[test]
fn decide_reaches_past_the_i64_window() {
    // One rung below the old ceiling: worked before, must still work.
    let inside = eq(
        f("add", vec![k("9223372036854775806"), k("1")]),
        k("9223372036854775807"),
    );
    let r = prove(vec![], inside, "decide");
    assert!(
        matches!(r, Ok(true)),
        "an identity inside `i64` must still decide: {r:?}"
    );

    // One rung above it: the case.
    let outside = eq(f("add", vec![k(PAST_I64), k("1")]), k(PAST_I64_PLUS_1));
    let r = prove(vec![], outside, "decide");
    assert!(
        matches!(r, Ok(true)),
        "`{PAST_I64} + 1 = {PAST_I64_PLUS_1}` is an exact integer fact both kernels can \
         compute: {r:?}"
    );

    // …and the boundary must not become a rubber stamp: a FALSE big identity is declined.
    let wrong = eq(f("add", vec![k(PAST_I64), k("1")]), k(PAST_I64));
    let r = prove(vec![], wrong, "decide");
    assert!(
        r.is_err(),
        "a false bignum identity must be DECLINED: {r:?}"
    );

    // A big COMPARISON too, both ways.
    let r = prove(vec![], le(k("9223372036854775807"), k(PAST_I64)), "decide");
    assert!(
        matches!(r, Ok(true)),
        "`i64::MAX ≤ i64::MAX+1` must decide true: {r:?}"
    );
    let r = prove(vec![], le(k(PAST_I64), k("9223372036854775807")), "decide");
    assert!(r.is_err(), "…and its converse must decide FALSE: {r:?}");
}

/// The `Bool` half of the refutation channel, and the mirrored encoding the old router
/// declined outright (`true = le(a, b)`, which a translator writes just as readily).
#[test]
fn decide_settles_bool_disequalities_in_either_orientation() {
    for goal in [
        not(eq(k("true"), k("false"))),
        not(eq(f("le", vec![k("5"), k("2")]), k("true"))),
        not(eq(k("true"), f("le", vec![k("5"), k("2")]))),
    ] {
        let r = prove(vec![], goal.clone(), "decide");
        assert!(
            matches!(r, Ok(true)),
            "{goal:?} is a ground Bool disequality: {r:?}"
        );
    }
    let r = prove(
        vec![],
        eq(k("true"), f("le", vec![k("2"), k("5")])),
        "decide",
    );
    assert!(
        matches!(r, Ok(true)),
        "`true = le(2, 5)` must decide: {r:?}"
    );
}

/// SOUNDNESS, the mutated certificate. `GroundRefutation` is a leaf, so its claim must be
/// re-decided at the trust door. A leaf asserting `¬(2 = 2)` is a proof of `False` if it is
/// ever believed.
#[test]
fn a_ground_refutation_leaf_that_lies_is_refused() {
    let lie = not(eq(k("2"), k("2")));
    let bogus = DerivationTree::leaf(lie.clone(), InferenceRule::GroundRefutation);
    let r = check_derivation(&[], &lie, bogus);
    assert!(
        !r.verified,
        "a `GroundRefutation` leaf claiming `¬(2 = 2)` must be refused — believing it puts a \
         closed term of type False in reach: {:?}",
        r.proof_term
    );

    // The golden object, so the refusal is about the lie and not about the rule being dead.
    let truth = not(eq(k("2"), k("3")));
    let golden = DerivationTree::leaf(truth.clone(), InferenceRule::GroundRefutation);
    let r = check_derivation(&[], &truth, golden);
    assert!(
        r.verified,
        "`¬(2 = 3)` IS refutable by computation: {:?}",
        r.verification_error
    );
}

// ============================================================================================
// T08-017 · `exists` — a compound witness, and the free variable behind it
// ============================================================================================

/// The capability, with the rule pinned: a compound witness must travel as a TERM. If it
/// still went through `ExistentialIntro`'s `String`, the certifier would resolve it as one
/// name and the kernel would reject a global called `"add(n, 1)"`.
#[test]
fn exists_carries_a_compound_witness_as_a_term() {
    let goal = ex("m", eq(v("m"), add1(v("n"))));
    let tree = prove_tree(vec![], goal, "exists n + 1 . simp");
    assert!(
        uses_rule(
            &tree,
            &InferenceRule::ExistentialIntroTerm {
                witness: k("_"),
                witness_type: String::new(),
            }
        ),
        "the witness must be carried as a ProofTerm, not a name: {tree}"
    );
}

/// THE NEGATIVE. Accepting any compound term is not the capability — accepting the RIGHT one
/// is. A witness that does not satisfy the body must leave a goal nothing closes.
#[test]
fn exists_with_a_wrong_compound_witness_proves_nothing() {
    let goal = ex("m", eq(v("m"), add1(v("n"))));
    let r = prove(vec![], goal, "exists n + 2 . simp");
    assert!(
        !matches!(r, Ok(true)),
        "`n + 2` does not witness `∃m. m = n + 1`; taking the step must not certify: {r:?}"
    );
}

/// The BOUNDARY on the other side of "compound": an uninterpreted application, whose domain
/// is the first-order `Entity` rather than `Int`. The binder domain is COMPUTED, so both
/// shapes must work — a fix that hard-coded `Int` for compound witnesses would break this.
#[test]
fn exists_accepts_an_uninterpreted_application_as_a_witness() {
    let goal = ex("m", eq(v("m"), f("g", vec![k("A")])));
    let r = prove(vec![], goal, "exists g A . simp");
    assert!(
        matches!(r, Ok(true)),
        "`g A` is a witness too, at the Entity domain: {r:?}"
    );
}

/// THE FREE VARIABLE. `∃m. m = n` with `n` free in the THEOREM is Lean's `variable (n : Int)`:
/// the certifier lowers it to `Term::Var`, and `Context::get` reads only bindings, so without
/// a local binding the GOAL TYPE itself is unbound and the failure (`UnboundVariable("n")`)
/// names the certifier's last step rather than the missing declaration.
#[test]
fn a_free_variable_in_the_statement_is_a_kernel_local() {
    let r = prove(vec![], ex("m", eq(v("m"), v("n"))), "exists n . simp");
    assert!(
        matches!(r, Ok(true)),
        "a free `n` must be statable and provable: {r:?}"
    );

    // And under a binder, where `n` is NOT free, the ∀-introduction still supplies it.
    let r = prove(
        vec![],
        forall("n", ex("m", eq(v("m"), v("n")))),
        "intro n . exists n . simp",
    );
    assert!(
        matches!(r, Ok(true)),
        "the bound case must keep working: {r:?}"
    );
}

/// SOUNDNESS, the mutated certificate. `witness_type` is the ∃-binder's DOMAIN, and the goal
/// type is built from the same computation — a node that declares a different one describes a
/// different theorem, and must be refused rather than quietly reinterpreted.
#[test]
fn an_existential_intro_term_with_the_wrong_domain_is_refused() {
    let goal = ex("m", eq(v("m"), add1(k("1"))));
    let body = eq(v("m"), add1(k("1")));
    let inner = eq(add1(k("1")), add1(k("1")));
    let good = DerivationTree::new(
        goal.clone(),
        InferenceRule::ExistentialIntroTerm {
            witness: add1(k("1")),
            witness_type: "Int".to_string(),
        },
        vec![DerivationTree::leaf(
            inner.clone(),
            InferenceRule::Reflexivity,
        )],
    );
    let r = check_derivation(&[], &goal, good);
    assert!(
        r.verified,
        "the golden ∃-introduction must certify: {:?}",
        r.verification_error
    );
    let _ = body;

    let mutated = DerivationTree::new(
        goal.clone(),
        InferenceRule::ExistentialIntroTerm {
            witness: add1(k("1")),
            witness_type: "Entity".to_string(),
        },
        vec![DerivationTree::leaf(inner, InferenceRule::Reflexivity)],
    );
    let r = check_derivation(&[], &goal, mutated);
    assert!(
        !r.verified,
        "an `Ex Entity …` certificate does not prove an `Ex Int …` goal: {:?}",
        r.proof_term
    );
}

// ============================================================================================
// T08-023 · `rewrite` — one occurrence, and the capture that must still be avoided
// ============================================================================================

/// The capability and its neighbour in one test: occurrence 0 and occurrence 1 of the SAME
/// pattern in the SAME goal produce two DIFFERENT goals, each closed by its own hypothesis.
/// An all-occurrences rewrite reaches neither.
#[test]
fn rewrite_at_selects_the_occurrence_and_each_one_certifies() {
    for (occurrence, witness) in [
        (0usize, pred("P", vec![k("b"), k("a")])),
        (1, pred("P", vec![k("a"), k("b")])),
    ] {
        let premises = vec![("h", eq(k("a"), k("b"))), ("hp", witness.clone())];
        let mut st = state(premises, pred("P", vec![k("a"), k("a")]));
        st.rewrite_at("h", occurrence)
            .unwrap_or_else(|e| panic!("occurrence {occurrence} must exist: {e:?}"));
        assert_eq!(
            st.focused_target(),
            Some(&witness),
            "occurrence {occurrence} must be the ONLY one rewritten"
        );
        st.run_script("assumption")
            .expect("the rewritten goal IS the hypothesis");
        let r = st.qed().expect("complete");
        assert!(
            r.verified,
            "the positional rewrite must certify at occurrence {occurrence}: {:?}",
            r.verification_error
        );
    }
}

/// THE NEGATIVE. An occurrence that does not exist is an error, not a silent no-op and not a
/// clamp to the last one — a clamp would make `rewrite h at 9` quietly mean something else.
#[test]
fn rewrite_at_a_nonexistent_occurrence_is_refused() {
    let premises = vec![("h", eq(k("a"), k("b")))];
    let mut st = state(premises, pred("P", vec![k("a"), k("a")]));
    let e = match st.rewrite_at("h", 5) {
        Err(e) => e,
        Ok(_) => panic!("there is no sixth occurrence of `a` to rewrite"),
    };
    assert!(
        format!("{e:?}").contains("occurrence"),
        "the error must name the occurrence, not the pattern: {e:?}"
    );
    assert_eq!(
        st.open_goals(),
        1,
        "a refused rewrite must leave the goal as it was"
    );
}

/// CAPTURE IS STILL AVOIDED with occurrence selection on. This campaign replaced `rewrite`'s
/// substitution with a capture-avoiding one; threading an index through it must not regress
/// that. Two halves, both from the same discipline:
///
/// * a pattern the GOAL BINDS is a different variable, so it is not an occurrence — and
///   therefore is not COUNTED either, which is what keeps a caller's index meaningful;
/// * a binder that would capture the REPLACEMENT's free variable is α-renamed away first.
#[test]
fn occurrence_selection_still_avoids_capture_and_shadowing() {
    // Shadowing: `h : x = 0` speaks about the FREE `x`; `∀x. P(x)`'s `x` is a different one.
    let mut st = state(
        vec![("h", eq(v("x"), k("0")))],
        forall("x", pred("P", vec![v("x")])),
    );
    assert!(
        st.rewrite_at("h", 0).is_err(),
        "the bound occurrence must not be an occurrence — rewriting it turns ∀x. P(x) into \
         ∀x. P(0), which does not prove the goal"
    );

    // Capture: the pattern `A` is closed, but the REPLACEMENT mentions `x`, which the binder
    // would capture.
    let mut st = state(
        vec![("h", eq(k("A"), v("x")))],
        forall(
            "x",
            ProofExpr::Predicate {
                name: "R".to_string(),
                args: vec![v("x"), k("A")],
                world: None,
            },
        ),
    );
    st.rewrite_at("h", 0).expect("A occurs free in the goal");
    let after = st.focused_target().cloned().expect("a goal remains");
    let ProofExpr::ForAll { variable, body } = &after else {
        panic!("the quantifier must survive: {after:?}");
    };
    assert_ne!(
        variable, "x",
        "the binder must be α-renamed away from the replacement's free `x`: {after:?}"
    );
    let ProofExpr::Predicate { args, .. } = body.as_ref() else {
        panic!("the predicate must survive: {after:?}");
    };
    assert_eq!(
        args[1],
        v("x"),
        "A must become the hypothesis's FREE x: {after:?}"
    );
}

/// The all-occurrences sweep is still REACHABLE — it is now spelled as what it is, a repeated
/// one-occurrence step, and every step is separately kernel-checked.
#[test]
fn repeat_rewrite_recovers_the_all_occurrences_sweep() {
    let premises = vec![
        ("h", eq(k("a"), k("b"))),
        ("hp", pred("P", vec![k("b"), k("b")])),
    ];
    let mut st = state(premises, pred("P", vec![k("a"), k("a")]));
    st.run(&c::repeat(c::rewrite("h")))
        .expect("both occurrences rewrite, one step each");
    assert_eq!(
        st.focused_target(),
        Some(&pred("P", vec![k("b"), k("b")])),
        "`repeat (rewrite h)` must reach the all-occurrences result: {:?}",
        st.focused_target()
    );
    st.run_script("assumption")
        .expect("the swept goal is the hypothesis");
    assert!(
        st.qed().expect("complete").verified,
        "every swept step must certify"
    );
}

/// SOUNDNESS, the mutated certificate. The occurrence travels to the trust door, and the
/// kernel is what checks it: `Eq_rec` demands the source premise inhabit `P from`, so an
/// index naming the wrong occurrence builds a motive the subgoal does not prove. A wrong
/// position must be UNPROVABLE, never unsound.
#[test]
fn a_rewrite_at_with_the_wrong_occurrence_is_refused() {
    let goal = pred("P", vec![k("a"), k("a")]);
    let hyp = pred("P", vec![k("b"), k("a")]);
    let equality = eq(k("b"), k("a"));
    // The GOLDEN object: the motive abstracts occurrence 0, and the source premise is
    // `P(b, a)` — exactly `P[from := b]` for that motive.
    let golden = DerivationTree::new(
        goal.clone(),
        InferenceRule::RewriteAt {
            from: k("b"),
            to: k("a"),
            occurrence: 0,
        },
        vec![
            DerivationTree::leaf(equality.clone(), InferenceRule::PremiseMatch),
            DerivationTree::leaf(hyp.clone(), InferenceRule::PremiseMatch),
        ],
    );
    let r = check_derivation(&[equality.clone(), hyp.clone()], &goal, golden);
    assert!(
        r.verified,
        "the golden positional rewrite must certify: {:?}",
        r.verification_error
    );

    // The mutation: occurrence 1 abstracts the OTHER `a`, whose motive is inhabited by
    // `P(a, b)` — which is not the premise supplied.
    let mutated = DerivationTree::new(
        goal.clone(),
        InferenceRule::RewriteAt {
            from: k("b"),
            to: k("a"),
            occurrence: 1,
        },
        vec![
            DerivationTree::leaf(equality.clone(), InferenceRule::PremiseMatch),
            DerivationTree::leaf(hyp.clone(), InferenceRule::PremiseMatch),
        ],
    );
    let r = check_derivation(&[equality, hyp], &goal, mutated);
    assert!(
        !r.verified,
        "occurrence 1's motive is not inhabited by the `P(b, a)` premise, so the kernel must \
         refuse it: {:?}",
        r.proof_term
    );
}

// ============================================================================================
// T08-021 · the mixed relation chain — what composes today, and where it stops
// ============================================================================================

/// The COMPOSITION, which is the capability `calc` and Lean's `Trans` name: a chain whose
/// relation changes. The old order graph walked `le` edges only and had no arm that could
/// consume an equality at all, so one `=` step anywhere made the chain unreachable.
#[test]
fn an_equality_edge_composes_with_le_edges() {
    let premises = vec![
        ("h1", eq(k("a"), k("b"))),
        ("h2", le(k("b"), k("cc"))),
        ("h3", le(k("cc"), k("d"))),
    ];
    let tree = prove_tree(premises, le(k("a"), k("d")), "auto");
    assert!(
        uses_rule(&tree, &InferenceRule::LeTrans),
        "the `≤` steps must compose through `le_trans`: {tree}"
    );
    assert!(
        uses_rule(
            &tree,
            &InferenceRule::RewriteAt {
                from: k("x"),
                to: k("y"),
                occurrence: 0
            }
        ),
        "the equality edge is Leibniz on ONE of `le_refl`'s two arguments, which is the first \
         thing in the engine that needs a POSITIONAL rewrite: {tree}"
    );
}

/// The equality edge is directionless — `a = b` gives both `a ≤ b` and `b ≤ a`. A fix that
/// only built the left-to-right edge would pass the test above and fail this one.
#[test]
fn the_equality_edge_runs_in_both_directions() {
    let forward = vec![("h1", eq(k("a"), k("b"))), ("h2", le(k("b"), k("cc")))];
    let r = prove(forward, le(k("a"), k("cc")), "auto");
    assert!(matches!(r, Ok(true)), "`a = b`, `b ≤ cc ⊢ a ≤ cc`: {r:?}");

    let backward = vec![("h1", eq(k("b"), k("a"))), ("h2", le(k("b"), k("cc")))];
    let r = prove(backward, le(k("a"), k("cc")), "auto");
    assert!(
        matches!(r, Ok(true)),
        "`b = a`, `b ≤ cc ⊢ a ≤ cc` — same edge, other way: {r:?}"
    );
}

/// THE NEGATIVE. A chain prover must not invent an edge. `e` is mentioned nowhere.
#[test]
fn the_order_chain_does_not_invent_an_edge() {
    let premises = vec![("h1", eq(k("a"), k("b"))), ("h2", le(k("b"), k("cc")))];
    let r = prove(premises, le(k("a"), k("e")), "auto");
    assert!(
        !matches!(r, Ok(true)),
        "nothing relates `cc` to `e`, so `a ≤ e` must not be certified: {r:?}"
    );
}

/// THE STRICT EDGE, composed as far as the trusted base allows. `a = b`, `b < cc`, `cc ≤ d`
/// yields `a + 1 ≤ d` — the tightened form — because `lt_succ_le` turns the strict hypothesis
/// into a `≤` fact that composes. What has NO image is the strict CONCLUSION `a < d`: of the
/// eight `Int` order axioms none concludes a `lt` (`lt_succ_le` and `lt_add1_le` both take one
/// as a PREMISE), so the missing primitive is the converse
/// `le_succ_lt : Π a b. (a + 1) ≤ b → a < b`. That is a new axiom, and widening a tactic by
/// growing the trusted base is the one trade this campaign forbids — so this test pins the
/// strongest thing that IS provable, and the gap is recorded rather than bought.
#[test]
fn a_strict_hypothesis_composes_into_its_tightened_conclusion() {
    let premises = vec![
        ("h1", eq(k("a"), k("b"))),
        ("h2", lt(k("b"), k("cc"))),
        ("h3", le(k("cc"), k("d"))),
    ];
    let r = prove(premises, le(add1(k("a")), k("d")), "auto");
    assert!(
        matches!(r, Ok(true)),
        "`a = b`, `b < cc`, `cc ≤ d ⊢ a + 1 ≤ d` — the strict edge tightens and composes: {r:?}"
    );
}

/// `calc` ON THE SURFACE. The composition is only half the capability — the case's `@target`
/// says `calc` exists on no LOGOS surface at all, and a translator meeting a Lean `calc` block
/// has to emit something. Naming the steps is the point: `auto` searches everything in scope,
/// while a `calc` block says WHICH steps compose, in order, and fails if they do not.
#[test]
fn calc_composes_a_named_mixed_chain() {
    let premises = vec![
        ("h1", eq(k("a"), k("b"))),
        ("h2", le(k("b"), k("cc"))),
        ("h3", le(k("cc"), k("d"))),
        ("noise", le(k("d"), k("zz"))),
    ];
    let r = prove(premises, le(k("a"), k("d")), "calc h1 h2 h3");
    assert!(
        matches!(r, Ok(true)),
        "`calc h1 h2 h3` must compose `=` with `≤` with `≤`: {r:?}"
    );
}

/// THE NEGATIVE, twice. A `calc` block that names steps which do not meet must FAIL — that is
/// the difference between a chain and a search — and a step that is not a hypothesis at all is
/// an error about the NAME, not a silently shorter chain.
#[test]
fn calc_fails_on_a_broken_or_misnamed_chain() {
    let premises = vec![
        ("h1", eq(k("a"), k("b"))),
        ("h2", le(k("b"), k("cc"))),
        ("h3", le(k("cc"), k("d"))),
    ];
    // The middle step is omitted, so `b` and `cc` are not related by the named set.
    let r = prove(premises.clone(), le(k("a"), k("d")), "calc h1 h3");
    assert!(
        !matches!(r, Ok(true)),
        "a chain with a hole must not close — `calc` names its steps for exactly this: {r:?}"
    );

    let mut st = state(premises, le(k("a"), k("d")));
    let e = match st.run_script("calc h1 hzzz h3") {
        Err(e) => e,
        Ok(_) => panic!("`hzzz` is not a hypothesis"),
    };
    assert!(
        e.0.contains("hzzz") || e.0.to_lowercase().contains("hypothesis"),
        "the error must name the unknown step: {}",
        e.0
    );
}

/// The degenerate, pure-`=` block, which is the case's other half: the diagnostic for a `calc`
/// must be driven by the chain, not by the word — a pure-equality chain composes too.
#[test]
fn calc_chains_a_pure_equality_block() {
    let premises = vec![
        ("h1", eq(k("a"), k("b"))),
        ("h2", eq(k("b"), k("cc"))),
        ("h3", eq(k("cc"), k("d"))),
    ];
    let r = prove(premises, eq(k("a"), k("d")), "calc h1 h2 h3");
    assert!(
        matches!(r, Ok(true)),
        "a pure-`=` calc block must certify: {r:?}"
    );
}

/// THE `Decidable` INSTANCE, resolved rather than tabulated. `decide`'s kernel route used to
/// pick its instance from two hard-coded type names, which is the corpus case's complaint: the
/// tactic covered a LIST OF TYPES, so anything decidable by other means was outside it by
/// construction and widening it meant editing the certifier. The lookup is now a search of the
/// context for a `Π x̄. Decidable <pattern>`.
///
/// The pair that pins it: a `Bool` equality has a registered instance and must certify, while
/// an `Entity` equality has none and must be REFUSED — which is the half that matters, because
/// the prelude also registers `Classical_propDecidable : Π(p:Prop). Decidable p`. That instance
/// matches every proposition and decides none of them (it is built from `Classical_choice`), so
/// a search that took it would both shadow every real instance and drag the classical axiom
/// into the footprint of a computation.
#[test]
fn the_decidable_instance_is_resolved_from_the_context() {
    let decidable = eq(k("true"), k("true"));
    let leaf = DerivationTree::leaf(decidable.clone(), InferenceRule::NativeDecide);
    let r = check_derivation(&[], &decidable, leaf);
    assert!(
        r.verified,
        "`Eq Bool true true` has a registered instance (`decEqBool`) and the search must find \
         it: {:?}",
        r.verification_error
    );

    let no_instance = eq(k("Alpha"), k("Alpha"));
    let leaf = DerivationTree::leaf(no_instance.clone(), InferenceRule::NativeDecide);
    let r = check_derivation(&[], &no_instance, leaf);
    assert!(
        !r.verified,
        "`Eq Entity Alpha Alpha` has NO decision procedure; taking the universal classical \
         instance would make every proposition `decide`-able and smuggle `Classical_choice` \
         into a computation: {:?}",
        r.proof_term
    );
}

/// THE ONE MISSING PRIMITIVE, pinned so the day it lands is a green test and not a surprise.
///
/// Everything a strict chain needs is built: `a = b`, `b < cc`, `cc ≤ d` composes into
/// `a + 1 ≤ d` (see above). What has no image is the LAST step — turning `a + 1 ≤ d` back into
/// `a < d` — because of the eight `Int` order axioms none CONCLUDES a `lt`: `lt_succ_le` and
/// `lt_add1_le` both take one as a premise. The primitive is the converse
/// `le_succ_lt : Π a b. (a + 1) ≤ b → a < b`.
///
/// This test is correct in BOTH worlds and vacuous in neither, so it is not a blocker-pin: it
/// asks the KERNEL whether the primitive is registered, then requires the behaviour that
/// answer entails. While it is absent the tactic must DECLINE and name it — never hand `qed` a
/// derivation the kernel has to refuse, which is how an honest gap stays visible.
#[test]
fn a_strict_conclusion_is_exactly_one_primitive_away() {
    let mut kctx = logicaffeine_kernel::Context::new();
    logicaffeine_kernel::prelude::StandardLibrary::register(&mut kctx);
    let registered =
        kctx.get_global("le_succ_lt").is_some() || kctx.get_definition_type("le_succ_lt").is_some();

    let premises = vec![
        ("h1", eq(k("a"), k("b"))),
        ("h2", lt(k("b"), k("cc"))),
        ("h3", le(k("cc"), k("d"))),
    ];
    let goal = lt(k("a"), k("d"));

    if registered {
        let r = prove(premises.clone(), goal.clone(), "auto");
        assert!(
            matches!(r, Ok(true)),
            "`le_succ_lt` IS registered, so the mixed chain must now conclude `a < d`: {r:?}"
        );
        let r = prove(premises, goal, "calc h1 h2 h3");
        assert!(
            matches!(r, Ok(true)),
            "…and `calc` must compose it by name too: {r:?}"
        );
    } else {
        let r = prove(premises.clone(), goal.clone(), "auto");
        assert!(
            !matches!(r, Ok(true)),
            "with no primitive concluding a `lt`, a strict goal must NOT be certified — a \
             tactic that returns a derivation the kernel refuses turns a capability gap into \
             an unverified proof: {r:?}"
        );

        let mut st = state(premises, goal);
        let e = match st.run_script("calc h1 h2 h3") {
            Err(e) => e,
            Ok(_) => panic!("a strict conclusion cannot be built from the current axioms"),
        };
        assert!(
            e.0.contains("le_succ_lt"),
            "the decline must name the missing primitive, so the gap is actionable rather \
             than mysterious: {}",
            e.0
        );
    }
}

// ============================================================================================
// WAVE 2 · THE DEFECTS THE CORPUS DOES NOT NAME
//
// Every tactic in this crate reports success by handing `qed` a `DerivationTree`, and the
// kernel is what turns that tree into a theorem. So there are two ways to be wrong, and only
// one of them any existing test looks for: a tactic can return `Err` when it should close a
// goal, and a tactic can return `Ok` having built an object the kernel must REFUSE. The
// second is worse — the script reads as a proof, every tactic-level assertion passes, and the
// failure surfaces only as `verified == false` at a trust door nobody asserted on.
//
// The audit below sweeps the Leibniz (`Eq_rec`) steps the engine builds for itself. Wave 1
// added `InferenceRule::RewriteAt` because an ALL-occurrences motive cannot express "carry one
// of these two `x`s", and fixed the one site the order chain needed. The same hazard is in
// every other site that rewrites ONE position: `crush`'s per-argument walk, the congruence
// edge, and the equality path. Each test below is a goal those routines accept and the kernel
// then refuses, so each asserts the KERNEL's word (`Ok(true)`) and not the tactic's.
// ============================================================================================

/// `crush` closes `P(b, b)` from `P(a, cc)` by rewriting the arguments one at a time. The
/// SECOND rewrite lands on a predicate whose two arguments have become the same term, and an
/// all-occurrences motive `λz. P(z, z)` demands a source `P(cc, cc)` that was never proved —
/// the step meant `λz. P(b, z)`.
#[test]
fn crush_certifies_when_two_arguments_converge_on_one_term() {
    let premises = vec![
        ("h1", eq(k("a"), k("b"))),
        ("h2", eq(k("cc"), k("b"))),
        ("hp", pred("P", vec![k("a"), k("cc")])),
    ];
    let r = prove(premises, pred("P", vec![k("b"), k("b")]), "crush");
    assert!(
        matches!(r, Ok(true)),
        "`a = b`, `cc = b`, `P(a, cc)` ⊢ `P(b, b)` is ordinary congruence — a tactic that \
         returns Ok here must hand the kernel a motive it can check: {r:?}"
    );
}

/// THE NEGATIVE for that route: rewriting arguments must still need a REASON for each one.
/// Nothing relates `cc` to `b` here, so the second argument cannot move and the goal must not
/// close.
#[test]
fn crush_does_not_move_an_argument_it_has_no_equation_for() {
    let premises = vec![
        ("h1", eq(k("a"), k("b"))),
        ("hp", pred("P", vec![k("a"), k("cc")])),
    ];
    let r = prove(premises, pred("P", vec![k("b"), k("b")]), "crush");
    assert!(
        !matches!(r, Ok(true)),
        "no equation relates `cc` to `b`, so `P(b, b)` must not be reachable: {r:?}"
    );
}

/// The CONGRUENCE EDGE, one level down: `g(a, b) = g(b, b)` from `a = b`. The motive abstracts
/// every `b` in the conclusion, so it reads `λz. g(a, z) = g(z, z)` and asks for a reflexive
/// source `g(a, a) = g(a, a)` — while the source in hand is `g(a, b) = g(a, b)`.
#[test]
fn the_congruence_edge_certifies_when_the_replacement_recurs_beside_it() {
    let premises = vec![
        ("h1", eq(k("a"), k("b"))),
        ("hp", pred("P", vec![f("g", vec![k("a"), k("b")])])),
    ];
    let r = prove(
        premises,
        pred("P", vec![f("g", vec![k("b"), k("b")])]),
        "crush",
    );
    assert!(
        matches!(r, Ok(true)),
        "`a = b` ⊢ `g(a, b) = g(b, b)` is one congruence step: {r:?}"
    );
}

/// The EQUALITY PATH: `g(cc) = m`, `m = cc` ⊢ `g(cc) = cc`. The last hop rewrites the waypoint
/// `m` to `cc`, and `cc` already occurs inside the path's START — so the all-occurrences motive
/// `λz. g(z) = z` asks for `g(m) = m` where the accumulator proves `g(cc) = m`.
#[test]
fn an_equality_path_certifies_when_the_endpoint_occurs_inside_the_start() {
    let premises = vec![
        ("h1", eq(f("g", vec![k("cc")]), k("m"))),
        ("h2", eq(k("m"), k("cc"))),
    ];
    let r = prove(premises, eq(f("g", vec![k("cc")]), k("cc")), "crush");
    assert!(
        matches!(r, Ok(true)),
        "`g(cc) = m`, `m = cc` ⊢ `g(cc) = cc` is transitivity: {r:?}"
    );
}

/// THE NEGATIVE for the path: a path prover must not invent a hop. `m` is related to nothing.
#[test]
fn an_equality_path_does_not_invent_a_hop() {
    let premises = vec![("h1", eq(f("g", vec![k("cc")]), k("m")))];
    let r = prove(premises, eq(f("g", vec![k("cc")]), k("cc")), "crush");
    assert!(
        !matches!(r, Ok(true)),
        "nothing relates `m` to `cc`, so the path must not close: {r:?}"
    );
}

/// The whole class in one assertion, and the reason it is a CLASS: a Leibniz step that carries
/// one position must be certified by a motive that abstracts one position. The engine now picks
/// the occurrence so the motive it builds is the one the source premise inhabits, which is
/// exactly `RewriteAt`'s contract — so the derivations above must MENTION it.
#[test]
fn a_one_position_leibniz_step_travels_as_a_positional_rewrite() {
    let premises = vec![
        ("h1", eq(k("a"), k("b"))),
        ("h2", eq(k("cc"), k("b"))),
        ("hp", pred("P", vec![k("a"), k("cc")])),
    ];
    let tree = prove_tree(premises, pred("P", vec![k("b"), k("b")]), "crush");
    assert!(
        uses_rule(
            &tree,
            &InferenceRule::RewriteAt {
                from: k("x"),
                to: k("y"),
                occurrence: 0
            }
        ),
        "the converging step cannot be expressed by an all-occurrences motive, so it must \
         travel positionally: {tree}"
    );
}

// ============================================================================================
// T08-021 · `calc` with a QUANTIFIED step — the case's `chainWithQuantifiedStep` guard
// ============================================================================================

/// The corpus case's fourth Lean theorem: `calc f a = f b := by rw [h1]; _ = g b := h2 b`,
/// whose SECOND step is justified by a `∀`-lemma. `calc` took each named step as a ground
/// fact, so a quantified step had no image at all and the block was unreachable — which is the
/// case note's "`rewrite` requires a GROUND `Identity`, so a `calc` step justified by a
/// quantified lemma has no image", re-stated about `calc` itself.
#[test]
fn calc_instantiates_a_quantified_step_at_the_chains_waypoints() {
    let premises = vec![
        ("h1", eq(k("a"), k("b"))),
        (
            "h2",
            forall("x", eq(f("ff", vec![v("x")]), f("gg", vec![v("x")]))),
        ),
    ];
    let goal = eq(f("ff", vec![k("a")]), f("gg", vec![k("b")]));
    let r = prove(premises, goal, "calc h1 h2");
    assert!(
        matches!(r, Ok(true)),
        "`a = b` and `∀x. ff x = gg x` compose into `ff a = gg b`: {r:?}"
    );
}

/// THE NEGATIVE. `calc` names its steps, and a block that omits the step it needs must FAIL —
/// that is the entire difference between `calc` and `auto`. Without `h1` there is nothing to
/// carry `a` to `b`.
#[test]
fn calc_still_needs_every_step_it_uses_when_one_of_them_is_quantified() {
    let premises = vec![
        ("h1", eq(k("a"), k("b"))),
        (
            "h2",
            forall("x", eq(f("ff", vec![v("x")]), f("gg", vec![v("x")]))),
        ),
    ];
    let goal = eq(f("ff", vec![k("a")]), f("gg", vec![k("b")]));
    let r = prove(premises, goal, "calc h2");
    assert!(
        !matches!(r, Ok(true)),
        "`∀x. ff x = gg x` alone does not relate `ff a` to `gg b`: {r:?}"
    );
}

/// The BOUNDARY: a quantified step in an ORDER chain, not an equality one. The instantiation
/// is a property of the step, not of the relation the chain concludes, so both paths must see
/// it — a fix that only taught the `=` path would leave `≤` behind.
#[test]
fn a_quantified_step_also_composes_in_an_order_chain() {
    let premises = vec![
        (
            "h1",
            forall("x", eq(f("ff", vec![v("x")]), f("gg", vec![v("x")]))),
        ),
        ("h2", le(f("gg", vec![k("a")]), k("d"))),
    ];
    let r = prove(premises, le(f("ff", vec![k("a")]), k("d")), "calc h1 h2");
    assert!(
        matches!(r, Ok(true)),
        "`∀x. ff x = gg x` is an equality EDGE once instantiated at `a`: {r:?}"
    );
}

// ============================================================================================
// T12-012 · the door lane L asked for — a proof line that NAMES its inference rule
// ============================================================================================

/// `ProofStrategy::ByRule(name)` is a public variant of the language AST that nothing
/// constructs and no driver has an arm for, because the proof layer offered no way to APPLY a
/// rule by name. This is that door, and it is a real tactic rather than a label: the named
/// rule must actually discharge the goal from what is in scope, and the object it builds is
/// certified before the tactic reports success.
#[test]
fn by_rule_applies_the_named_inference_rule_and_the_kernel_checks_it() {
    let premises = vec![
        (
            "h1",
            ProofExpr::Implies(
                Box::new(pred("Happy", vec![k("Bob")])),
                Box::new(pred("Sad", vec![k("Bob")])),
            ),
        ),
        ("h2", pred("Happy", vec![k("Bob")])),
    ];
    let tree = prove_tree(premises, pred("Sad", vec![k("Bob")]), "by_rule ModusPonens");
    assert_eq!(
        std::mem::discriminant(&tree.rule),
        std::mem::discriminant(&InferenceRule::ModusPonens),
        "`by_rule R` must be discharged by R at the ROOT — naming a rule is the point: {tree}"
    );
}

/// THE NEGATIVE, and the one that matters. `by_rule` must not degenerate into "prove it
/// somehow": a rule that does not discharge this goal is an error, even when the goal is
/// perfectly provable by another rule.
#[test]
fn by_rule_refuses_a_rule_that_does_not_discharge_the_goal() {
    let premises = vec![
        (
            "h1",
            ProofExpr::Implies(
                Box::new(pred("Happy", vec![k("Bob")])),
                Box::new(pred("Sad", vec![k("Bob")])),
            ),
        ),
        ("h2", pred("Happy", vec![k("Bob")])),
    ];
    let mut st = state(premises, pred("Sad", vec![k("Bob")]));
    let e = match st.run_script("by_rule ConjunctionIntro") {
        Err(e) => e,
        Ok(_) => panic!("`Sad(Bob)` is not a conjunction and ∧-introduction cannot reach it"),
    };
    assert!(
        e.0.contains("ConjunctionIntro"),
        "the decline must name the rule that was asked for: {}",
        e.0
    );
    assert_eq!(
        st.open_goals(),
        1,
        "a refused `by_rule` leaves the goal as it was"
    );
}

/// A name that is not an inference rule at all must fail as a NAME error, not as a proof
/// failure — a translator handing us a Lean rule name needs to know which of the two it hit.
#[test]
fn by_rule_refuses_a_name_that_is_not_an_inference_rule() {
    let mut st = state(vec![], pred("Q", vec![k("A")]));
    let e = match st.run_script("by_rule NotARealRule") {
        Err(e) => e,
        Ok(_) => panic!("`NotARealRule` names nothing"),
    };
    assert!(
        e.0.contains("NotARealRule") && e.0.contains("inference rule"),
        "the decline must say the NAME is unknown, not that the proof failed: {}",
        e.0
    );
}

/// THE HONEST GAP. `InferenceRule` carries seven variants with no certifier arm, each of which
/// `uncertifiable_reason` already documents. `by_rule` must report THAT reason rather than
/// searching for premises that can never make such a node check — this is the one place in the
/// crate where a producer can consult the list before emitting a tree, and it must.
#[test]
fn by_rule_names_the_reason_a_declared_rule_can_never_certify() {
    let premises = vec![("h1", le(k("a"), k("b"))), ("h2", le(k("b"), k("cc")))];
    let mut st = state(premises, le(k("a"), k("cc")));
    let e = match st.run_script("by_rule TemporalTransitivity") {
        Err(e) => e,
        Ok(_) => panic!("`TemporalTransitivity` has no certifier arm"),
    };
    assert!(
        e.0.contains("no certifier arm"),
        "a rule that is declared but unimplemented must say so, with the translation its own \
         `uncertifiable_reason` gives: {}",
        e.0
    );
}

/// The BOUNDARY on the other side of "one step": a LEAF rule, which takes no premises at all.
/// `by_rule PremiseMatch` against a goal that IS a hypothesis is the degenerate case, and it
/// must work — otherwise the door only opens for binary rules.
#[test]
fn by_rule_reaches_a_leaf_rule_too() {
    let premises = vec![("h", pred("Q", vec![k("A")]))];
    let r = prove(premises, pred("Q", vec![k("A")]), "by_rule PremiseMatch");
    assert!(matches!(r, Ok(true)), "the goal IS the hypothesis: {r:?}");
}

/// …and the leaf must still be checked. `PremiseMatch` against a goal that is NOT in scope is
/// a lie the certifier catches, and `by_rule` must not report success and leave the failure to
/// the trust door.
#[test]
fn by_rule_refuses_a_leaf_rule_whose_claim_is_not_in_scope() {
    let premises = vec![("h", pred("Q", vec![k("A")]))];
    let mut st = state(premises, pred("Q", vec![k("B")]));
    assert!(
        st.run_script("by_rule PremiseMatch").is_err(),
        "`Q(B)` is not a hypothesis; the leaf must be refused where it is BUILT"
    );
}

/// Rule names arrive from a translator, so the spelling must be forgiving in the ways a
/// translator varies and strict everywhere else: the enum's own name, and the snake_case a
/// script is written in. Nothing else.
#[test]
fn by_rule_accepts_the_spellings_a_translator_writes() {
    for spelling in ["ModusPonens", "modus_ponens"] {
        let premises = vec![
            (
                "h1",
                ProofExpr::Implies(
                    Box::new(pred("Happy", vec![k("Bob")])),
                    Box::new(pred("Sad", vec![k("Bob")])),
                ),
            ),
            ("h2", pred("Happy", vec![k("Bob")])),
        ];
        let r = prove(
            premises,
            pred("Sad", vec![k("Bob")]),
            &format!("by_rule {spelling}"),
        );
        assert!(
            matches!(r, Ok(true)),
            "`{spelling}` must name the rule: {r:?}"
        );
    }
}

// ============================================================================================
// WAVE 2 · `auto` and the SCHEMATIC equational axiom
//
// The backward chainer's equational-rewrite branch UNIFIES a knowledge-base equality against
// the goal, renaming the equation's variables first — i.e. it reads a premise's free variables
// as universally quantified. The trust door reads them the other way: `close_over_context_locals`
// λ-abstracts a free `Term::Var` as a LOCAL, which is Lean's `variable (x : Int)` — a statement
// about one opaque `x`, not a schema. Two layers, two readings of the same proposition.
//
// The tests below do not pick the reading. They pin the one outcome that is wrong under BOTH:
// a tactic reporting `Ok` and handing `qed` a tree the kernel then refuses. `Ok(false)` is the
// forbidden answer — `auto` must either prove it or decline.
// ============================================================================================

/// THE CAPABILITY, stated the sound way: an equation with an explicit `∀` is a schema, and
/// using it at an instance is `UniversalInstTerm` — a step the certifier has an arm for. This
/// is Lean's `simp [Nat.add_zero]`.
#[test]
fn a_quantified_equation_is_used_at_an_instance_and_certifies() {
    let axiom = forall("x", eq(f("ff", vec![v("x")]), f("gg", vec![v("x")])));
    let goal = eq(f("ff", vec![k("A")]), f("gg", vec![k("A")]));
    let r = prove(vec![("ax", axiom)], goal, "crush");
    assert!(
        matches!(r, Ok(true)),
        "`∀x. ff x = gg x` at `x := A` is the whole proof: {r:?}"
    );
}

/// THE NEGATIVE for it: the schema must still MATCH. `hh A` is not an instance of `ff x`, and
/// a rewriter that believed its own substitution would prove it.
#[test]
fn a_quantified_equation_is_not_applied_where_it_does_not_match() {
    let axiom = forall("x", eq(f("ff", vec![v("x")]), f("gg", vec![v("x")])));
    let goal = eq(f("hh", vec![k("A")]), f("gg", vec![k("A")]));
    let r = prove(vec![("ax", axiom)], goal, "crush");
    assert!(
        !matches!(r, Ok(true)),
        "`hh A` is not an instance of `ff x`: {r:?}"
    );
}

/// THE DEFECT, pinned as the invariant rather than as a verdict on the reading. `auto`'s
/// equational branch fires on a free-variable equation and emits a Leibniz node whose equality
/// premise is the axiom AS STATED, not the instance the rewrite used — so the certifier is
/// handed a premise that does not prove `from = to`, and the whole proof fails at a door the
/// tactic already told the caller it had passed.
///
/// Whichever reading of a free variable wins, `Ok(false)` is wrong: either the instantiation is
/// licensed and the certificate must be built, or it is not and the tactic must DECLINE.
#[test]
fn auto_never_reports_a_proof_the_trust_door_then_refuses() {
    let axiom = eq(f("Add", vec![v("x"), k("Zero")]), v("x"));
    let goal = eq(
        f("Add", vec![f("Succ", vec![k("Zero")]), k("Zero")]),
        f("Succ", vec![k("Zero")]),
    );
    let r = prove(vec![("ax", axiom)], goal, "auto");
    assert!(
        !matches!(r, Ok(false)),
        "`auto` must prove this or decline it — reporting success and leaving an \
         uncertifiable tree for `qed` is the one answer that is wrong under either reading of \
         a free variable: {r:?}"
    );
}

/// The same invariant one rung along the chain, where the rewrite does not land on the goal's
/// right-hand side in one step and the branch recurses. Both exits of that branch build the
/// same node, so a fix that repaired only the direct hit would leave this one lying.
#[test]
fn auto_never_reports_an_uncertifiable_two_step_rewrite() {
    let premises = vec![
        ("ax1", eq(f("Add", vec![v("x"), k("Zero")]), v("x"))),
        ("ax2", eq(f("Succ", vec![k("Zero")]), k("One"))),
    ];
    let goal = eq(
        f("Add", vec![f("Succ", vec![k("Zero")]), k("Zero")]),
        k("One"),
    );
    let r = prove(premises, goal, "auto");
    assert!(
        !matches!(r, Ok(false)),
        "the recursive exit of the equational branch must be honest too: {r:?}"
    );
}

// ============================================================================================
// T02-013 · `@[simp]` — the registry, and the proof layer that has to READ it
//
// The kernel now carries a real simp-lemma registry (`Context::add_simp_lemma`), which is the
// declaration side. A registry nothing consults is a list; these pin the other half — that a
// registered lemma reaches the rewriter that actually rewrites, and that an UNregistered one
// does not, which is the only thing that makes registration mean anything.
// ============================================================================================

/// Build a kernel context carrying `add_zero : Π(n:Int). Eq Int (add n 0) n` as a declaration,
/// registered as a `@[simp]` lemma or not, per `registered`.
fn ctx_with_add_zero(registered: bool) -> logicaffeine_kernel::Context {
    use logicaffeine_kernel::{prelude::StandardLibrary, Context, Literal, Term};
    let mut kctx = Context::new();
    StandardLibrary::register(&mut kctx);
    let g = |n: &str| Term::Global(n.to_string());
    let app = |f: Term, a: Term| Term::App(Box::new(f), Box::new(a));
    // add n 0
    let lhs = app(
        app(g("add"), Term::Var("n".to_string())),
        Term::Lit(Literal::Int(0)),
    );
    // Eq Int (add n 0) n
    let body = app(app(app(g("Eq"), g("Int")), lhs), Term::Var("n".to_string()));
    let stmt = Term::Pi {
        param: "n".to_string(),
        param_type: Box::new(g("Int")),
        body_type: Box::new(body),
    };
    kctx.add_declaration("add_zero", stmt);
    if registered {
        kctx.add_simp_lemma("add_zero")
            .expect("`∀n. add n 0 = n` orients left to right");
    }
    kctx
}

/// THE WIRING. A lemma registered `@[simp]` in the kernel becomes a rule in the proof layer's
/// rewriter — which is where rewriting actually happens, since the registry holds kernel
/// `Term`s and the tactic layer rewrites `ProofExpr`s.
#[test]
fn a_registered_simp_lemma_becomes_a_rule_in_the_proof_layers_rewriter() {
    let mut set = logicaffeine_proof::simp::SimpSet::new();
    let added = set.register_context_lemmas(&ctx_with_add_zero(true));
    assert_eq!(
        added, 1,
        "the registered lemma must lower into exactly one rewrite rule"
    );
    assert_eq!(set.len(), 1);
}

/// THE NEGATIVE, and the one that gives registration its meaning: the SAME lemma, declared
/// and true and in scope, contributes NOTHING while it is not registered. A wiring that read
/// every declaration would make `@[simp]` a no-op and the attribute a decoration.
#[test]
fn an_unregistered_lemma_contributes_no_rule_however_true_it_is() {
    let mut set = logicaffeine_proof::simp::SimpSet::new();
    let added = set.register_context_lemmas(&ctx_with_add_zero(false));
    assert_eq!(
        added, 0,
        "`add_zero` is declared but not `@[simp]` — it is not a rule"
    );
    assert!(set.is_empty());
}

/// …and the rule must FIRE. A set that registers rules nobody applies is the same list with
/// extra steps, so this drives `simp` with the registry's set ALONE — no in-scope rule set —
/// and requires the kernel to certify the result.
#[test]
fn the_registered_rule_rewrites_the_goal_and_the_kernel_certifies_it() {
    let lemma = forall("n", eq(f("add", vec![v("n"), k("0")]), v("n")));
    let goal = eq(f("add", vec![k("cc"), k("0")]), k("cc"));

    let mut st = state(vec![("add_zero", lemma.clone())], goal.clone());
    let mut set = logicaffeine_proof::simp::SimpSet::new();
    set.register_context_lemmas(&ctx_with_add_zero(true));
    st.simp(&set)
        .expect("the registered rule rewrites `add(cc, 0)` to `cc`");
    let r = st.qed().expect("no goals remain");
    assert!(
        r.verified,
        "a registered simp lemma is a RULE, and the rewrite it licenses must still be a \
         kernel-checked step: {:?}",
        r.verification_error
    );

    // The same goal, the same scope, the lemma UNregistered: the rule set is empty and the
    // goal does not close. This is the pair — the rewrite came from the registry.
    let mut st = state(vec![("add_zero", lemma)], goal);
    let mut set = logicaffeine_proof::simp::SimpSet::new();
    set.register_context_lemmas(&ctx_with_add_zero(false));
    assert!(
        st.simp(&set).is_err(),
        "with nothing registered there is no rule, and `add(cc, 0) = cc` is not reflexive"
    );
}

// ============================================================================================
// THE SWEEP — the pit-of-success lock this wave leaves behind
// ============================================================================================

/// **`Ok` with no goal open MUST imply `verified`.**
///
/// Every tactic reports success by handing `qed` a `DerivationTree`, and the kernel is what
/// decides whether that object is a theorem. Those two answers must not come apart. When they
/// do, a script reads as a proof, every tactic-level assertion in every test passes, and
/// nothing looks wrong until somebody asserts on `verified` — which is exactly why four
/// separate uncertifiable-certificate bugs lived in this crate with a green suite around them.
///
/// The corpus is the shapes that broke, one per defect, plus their neighbours:
///
/// * two predicate arguments that CONVERGE on one term, so a one-position Leibniz step cannot
///   be written with an all-occurrences motive;
/// * a congruence whose introduced argument RECURS beside itself;
/// * an equality path whose endpoint occurs INSIDE its start;
/// * an equation with a FREE variable, used at an instance the trust door does not license;
/// * and the ordinary cases, so the lock is not satisfied by a tactic that declines everything.
///
/// One shape is deliberately ABSENT and is reported rather than pinned: a term that mixes the
/// arithmetic name list (`Add`/`Sub`/`Mul`/`Div`, which the English front end emits for `plus`)
/// with a prelude inductive's constructors — `Add(Succ(Zero), Zero)` — is typed `Int` by
/// `verify::SymbolCollector`'s hard-coded list while its operands are `Nat`, so the statement
/// is ill-sorted before any tactic runs. That is a sort-inference gap, not a certificate gap,
/// and closing it by name-list surgery would only move which spellings are wrong.
///
/// The invariant is one line and it is the right one to hold for ever: a tactic that cannot
/// certify must DECLINE. Add a shape here whenever a new one is found.
#[test]
fn no_tactic_reports_success_and_leaves_the_kernel_to_refuse_it() {
    let corpus: Vec<(Vec<(&str, ProofExpr)>, ProofExpr)> = vec![
        // Converging predicate arguments.
        (
            vec![
                ("h1", eq(k("a"), k("b"))),
                ("h2", eq(k("cc"), k("b"))),
                ("hp", pred("P", vec![k("a"), k("cc")])),
            ],
            pred("P", vec![k("b"), k("b")]),
        ),
        // A congruence whose introduced argument recurs beside itself.
        (
            vec![
                ("h1", eq(k("a"), k("b"))),
                ("hp", pred("P", vec![f("g", vec![k("a"), k("b")])])),
            ],
            pred("P", vec![f("g", vec![k("b"), k("b")])]),
        ),
        // An equality path whose endpoint occurs inside its start.
        (
            vec![
                ("h1", eq(f("g", vec![k("cc")]), k("m"))),
                ("h2", eq(k("m"), k("cc"))),
            ],
            eq(f("g", vec![k("cc")]), k("cc")),
        ),
        // A free-variable equation used at an instance — a premise's variables are LOCALS,
        // so the honest answer is to decline rather than to build a leaf naming a
        // proposition nobody registered.
        (
            vec![("ax", eq(f("ff", vec![v("x")]), f("gg", vec![v("x")])))],
            eq(f("ff", vec![k("A")]), f("gg", vec![k("A")])),
        ),
        // A quantified equation used at an instance — the sound spelling of the same thing.
        (
            vec![(
                "ax",
                forall("x", eq(f("ff", vec![v("x")]), f("gg", vec![v("x")]))),
            )],
            eq(f("ff", vec![k("A")]), f("gg", vec![k("A")])),
        ),
        // A quantified equation reaching a goal through a further equality.
        (
            vec![
                (
                    "ax",
                    forall("x", eq(f("ff", vec![v("x")]), f("gg", vec![v("x")]))),
                ),
                ("h1", eq(k("a"), k("b"))),
            ],
            eq(f("ff", vec![k("a")]), f("gg", vec![k("b")])),
        ),
        // The ordinary order chain, mixed relations.
        (
            vec![
                ("h1", eq(k("a"), k("b"))),
                ("h2", le(k("b"), k("cc"))),
                ("h3", le(k("cc"), k("d"))),
            ],
            le(k("a"), k("d")),
        ),
        // A strict hypothesis and a tightened conclusion.
        (
            vec![("h1", lt(k("b"), k("cc"))), ("h2", le(k("cc"), k("d")))],
            le(add1(k("b")), k("d")),
        ),
        // A contradictory integer system, which closes anything by ex falso.
        (
            vec![("h1", le(k("5"), k("x"))), ("h2", le(k("x"), k("3")))],
            pred("Q", vec![k("A")]),
        ),
        // Ground arithmetic, at and past the old `i64` window.
        (vec![], le(k("2"), k("3"))),
        (
            vec![],
            eq(f("add", vec![k(PAST_I64), k("1")]), k(PAST_I64_PLUS_1)),
        ),
        // A repeated waypoint, the corpus case's `chainRepeatedWaypoint`.
        (
            vec![
                ("h1", eq(k("a"), k("b"))),
                ("hp", pred("P", vec![k("b"), k("b")])),
            ],
            pred("P", vec![k("a"), k("a")]),
        ),
    ];

    const SCRIPTS: &[&str] = &[
        "auto",
        "crush",
        "simp",
        "decide",
        "omega",
        "linarith",
        "ring",
        "assumption",
        "rfl",
        "repeat (rewrite h1)",
        "by_rule PremiseMatch",
        "by_rule ModusPonens",
    ];

    let mut broken: Vec<String> = Vec::new();
    for (premises, goal) in &corpus {
        for script in SCRIPTS {
            let mut st = state(premises.clone(), goal.clone());
            if st.run_script(script).is_err() {
                continue; // an honest decline is the other acceptable answer
            }
            if st.open_goals() != 0 {
                continue; // the script made progress and left work; not a claim of a theorem
            }
            match st.qed() {
                Ok(r) if r.verified => {}
                Ok(r) => broken.push(format!(
                    "`{script}` on `{goal}` reported SUCCESS and the kernel refused the tree: \
                     {:?}",
                    r.verification_error
                )),
                Err(e) => broken.push(format!(
                    "`{script}` on `{goal}` closed every goal and then `qed` failed: {e:?}"
                )),
            }
        }
    }
    assert!(
        broken.is_empty(),
        "a tactic must prove it or decline it — never both:\n  {}",
        broken.join("\n  ")
    );
}

/// A FIFTH defect the sweep found, and the shape it takes: `ring` decided `le(2, 3) = true`.
///
/// The polynomial oracle answers about VALUES, and `le(2, 3)` computes to `true`, so it agreed
/// the two sides are the same "polynomial". The proposition is true — and the certificate
/// `ring` emits is an `ArithDecision`, which lowers to an `Eq Int` fact, so the kernel reported
/// "expected Int, found Bool" about a statement that is perfectly correct. A true goal, a true
/// oracle, and an uncertifiable proof.
///
/// The pair: `ring` must decline every Bool fact, and must still decide the Int identities that
/// are its whole job.
#[test]
fn ring_declines_a_bool_fact_even_a_true_one() {
    for goal in [
        le(k("2"), k("3")),
        lt(k("2"), k("3")),
        eq(k("true"), k("true")),
        eq(f("le", vec![k("x"), k("y")]), k("false")),
    ] {
        let mut st = state(vec![], goal.clone());
        let e = match st.ring() {
            Err(e) => e,
            Ok(_) => panic!("`{goal:?}` is a Bool fact; `ring` is an Int ring tactic"),
        };
        assert!(
            format!("{e:?}").contains("Bool"),
            "the decline must say WHY — it is the wrong sort, not a failed identity: {e:?}"
        );
        assert_eq!(
            st.open_goals(),
            1,
            "a declined `ring` leaves the goal as it was"
        );
    }
}

/// …and `ring` still does its job. The neighbouring Int identity must certify, so the guard
/// above is about the sort and not a blanket refusal.
#[test]
fn ring_still_decides_the_int_identities_it_is_for() {
    let goal = eq(
        f("mul", vec![f("add", vec![k("x"), k("y")]), k("2")]),
        f(
            "add",
            vec![
                f("mul", vec![k("x"), k("2")]),
                f("mul", vec![k("y"), k("2")]),
            ],
        ),
    );
    let r = prove(vec![], goal, "ring");
    assert!(
        matches!(r, Ok(true)),
        "`(x + y) * 2 = x*2 + y*2` is a ring identity: {r:?}"
    );
}

// ============================================================================================
// T08-023 · the LOCATION half — `rw [h] at hp`
//
// The corpus case for `conv` says positional rewriting has two aspects, the POSITION and the
// LOCATION, and that "a translator that fixes only one of them has fixed nothing". Wave 1
// fixed the position. `ProofState::rewrite` still read and wrote the GOAL only, so a Lean proof
// that normalises a hypothesis before using it — `rw [h] at hp` — had no image at all.
// ============================================================================================

/// The capability, and the two things that make it a rewrite of the CONTEXT: the hypothesis
/// moves, the goal does not, and the kernel certifies what the hypothesis now claims.
#[test]
fn rewriting_a_hypothesis_moves_the_context_and_leaves_the_goal_alone() {
    let premises = vec![
        ("h", eq(k("a"), k("b"))),
        ("hp", pred("P", vec![k("a"), k("a")])),
    ];
    let goal = pred("P", vec![k("b"), k("a")]);
    let mut st = state(premises, goal.clone());
    st.rewrite_hyp("h", "hp").expect("`a` occurs in `hp`");
    assert_eq!(
        st.focused_target(),
        Some(&goal),
        "a rewrite AT a hypothesis must not touch the goal: {:?}",
        st.focused_target()
    );
    st.run_script("assumption")
        .expect("the rewritten hypothesis IS the goal");
    let r = st.qed().expect("complete");
    assert!(
        r.verified,
        "the rewritten hypothesis carries its own `Eq_rec` derivation and the kernel must \
         accept it: {:?}",
        r.verification_error
    );
}

/// The POSITION and the LOCATION compose: occurrence 1 of the same pattern in the same
/// hypothesis is a different hypothesis, and each closes its own goal.
#[test]
fn rewriting_a_hypothesis_selects_the_occurrence_too() {
    for (occurrence, want) in [
        (0usize, pred("P", vec![k("b"), k("a")])),
        (1, pred("P", vec![k("a"), k("b")])),
    ] {
        let premises = vec![
            ("h", eq(k("a"), k("b"))),
            ("hp", pred("P", vec![k("a"), k("a")])),
        ];
        let mut st = state(premises, want.clone());
        st.rewrite_hyp_at("h", "hp", occurrence)
            .unwrap_or_else(|e| panic!("occurrence {occurrence} must exist: {e:?}"));
        st.run_script("assumption")
            .unwrap_or_else(|e| panic!("occurrence {occurrence} must produce `{want}`: {e:?}"));
        let r = st.qed().expect("complete");
        assert!(
            r.verified,
            "occurrence {occurrence} must certify: {:?}",
            r.verification_error
        );
    }
}

/// THE NEGATIVES. Three ways this must refuse, each with a different message, and none of them
/// leaving the context half-moved.
#[test]
fn rewriting_a_hypothesis_refuses_what_it_cannot_do() {
    let premises = vec![
        ("h", eq(k("a"), k("b"))),
        ("hp", pred("P", vec![k("a"), k("a")])),
        ("hq", pred("Q", vec![k("cc")])),
    ];

    // An unknown location.
    let mut st = state(premises.clone(), pred("P", vec![k("b"), k("a")]));
    assert!(
        matches!(st.rewrite_hyp("h", "hzzz"), Err(e) if format!("{e:?}").contains("hzzz")),
        "the error must name the location that does not exist"
    );

    // A location the pattern does not occur in — a silent no-op here would leave a caller
    // believing the context moved.
    let mut st = state(premises.clone(), pred("P", vec![k("b"), k("a")]));
    let e = match st.rewrite_hyp("h", "hq") {
        Err(e) => e,
        Ok(_) => panic!("`a` does not occur in `Q(cc)`"),
    };
    assert!(
        format!("{e:?}").contains("hq"),
        "the error must name the location: {e:?}"
    );
    assert_eq!(
        st.focused_target(),
        Some(&pred("P", vec![k("b"), k("a")])),
        "a refused rewrite changes nothing"
    );

    // An equation rewriting ITSELF, which would turn `a = b` into `b = b` and lose the fact.
    let mut st = state(premises, pred("P", vec![k("b"), k("a")]));
    assert!(
        st.rewrite_hyp("h", "h").is_err(),
        "an equation may not rewrite itself into a tautology"
    );
}

/// A QUANTIFIED equation at a hypothesis: the pattern is looked for in the LOCATION, not in the
/// goal, so a lemma that matches the hypothesis and not the goal must still fire.
#[test]
fn a_quantified_equation_rewrites_a_hypothesis_at_its_own_match() {
    let premises = vec![
        (
            "h",
            forall("x", eq(f("ff", vec![v("x")]), f("gg", vec![v("x")]))),
        ),
        ("hp", pred("P", vec![f("ff", vec![k("A")])])),
    ];
    let goal = pred("P", vec![f("gg", vec![k("A")])]);
    let r = prove(premises, goal, "rewrite h at hp . assumption");
    assert!(
        matches!(r, Ok(true)),
        "`∀x. ff x = gg x` must instantiate at the HYPOTHESIS's `ff A`: {r:?}"
    );
}

/// THE SURFACE, all three spellings of `at`. A number is the POSITION (Lean's `nth_rewrite`), a
/// name is the LOCATION (Lean's `rw … at h`), and a name followed by a number is both.
#[test]
fn the_at_clause_reads_position_and_location() {
    // `at <k>` — the goal, at occurrence k. (Unchanged from wave 1; pinned so the new
    // location form did not steal the position form's spelling.)
    let premises = vec![
        ("h", eq(k("a"), k("b"))),
        ("hp", pred("P", vec![k("a"), k("b")])),
    ];
    let r = prove(
        premises,
        pred("P", vec![k("a"), k("a")]),
        "rewrite h at 1 . assumption",
    );
    assert!(
        matches!(r, Ok(true)),
        "`at 1` must still mean the goal's occurrence 1: {r:?}"
    );

    // `at <name>` — the hypothesis.
    let premises = vec![
        ("h", eq(k("a"), k("b"))),
        ("hp", pred("P", vec![k("a"), k("a")])),
    ];
    let r = prove(
        premises,
        pred("P", vec![k("b"), k("a")]),
        "rewrite h at hp . assumption",
    );
    assert!(
        matches!(r, Ok(true)),
        "`at hp` must mean the hypothesis: {r:?}"
    );

    // `at <name> <k>` — both.
    let premises = vec![
        ("h", eq(k("a"), k("b"))),
        ("hp", pred("P", vec![k("a"), k("a")])),
    ];
    let r = prove(
        premises,
        pred("P", vec![k("a"), k("b")]),
        "rewrite h at hp 1 . assumption",
    );
    assert!(
        matches!(r, Ok(true)),
        "`at hp 1` must mean occurrence 1 of the hypothesis: {r:?}"
    );
}

/// EVERY NAMEABLE RULE ROUND-TRIPS, and the set does not silently shrink.
///
/// `by_rule` resolves a name against `InferenceRule::NAMEABLE`, a `const` list — and a `const`
/// list cannot be exhaustive the way a `match` is, so a variant added later can quietly fail to
/// be nameable. This is the drift guard a `match` would have given: every entry's spelling is
/// its own `Debug` name, its snake_case form resolves to the same rule, and the set is at least
/// as large as it was when the door was built.
#[test]
fn every_nameable_rule_round_trips_and_the_set_has_not_shrunk() {
    for rule in InferenceRule::NAMEABLE {
        let name = rule.canonical_name();
        assert!(
            !name.contains(' ') && !name.contains('('),
            "a NAMEABLE rule must be payload-free — `{name}` carries an argument, and a name \
             cannot supply one"
        );
        assert_eq!(
            InferenceRule::from_name(&name).as_ref(),
            Some(rule),
            "`{name}` must resolve back to itself"
        );
        let snake = name
            .chars()
            .enumerate()
            .flat_map(|(i, c)| {
                let sep = i > 0 && c.is_ascii_uppercase();
                sep.then_some('_')
                    .into_iter()
                    .chain(std::iter::once(c.to_ascii_lowercase()))
            })
            .collect::<String>();
        assert_eq!(
            InferenceRule::from_name(&snake).as_ref(),
            Some(rule),
            "the script spelling `{snake}` must resolve to `{name}` too"
        );
    }
    assert!(
        InferenceRule::NAMEABLE.len() >= 39,
        "the nameable set is a RATCHET — it had 39 rules when `by_rule` was built, and a \
         variant that stops being nameable makes a capability vanish silently: {}",
        InferenceRule::NAMEABLE.len()
    );
    // …and nothing outside the list resolves.
    assert!(
        InferenceRule::from_name("Rewrite").is_none(),
        "`Rewrite` needs its equation"
    );
    assert!(
        InferenceRule::from_name("MODUSPONENS").is_none(),
        "spelling is exact, not fuzzy"
    );
}

// ============================================================================================
// `induction n` — TOTALITY. The spelling our own doc comment teaches.
// ============================================================================================
//
// `ProofStrategy::Induction`'s worked example is `Proof: Induction on n.`, and the block it is
// written against — `Given: Bob is happy. Prove: Bob is happy.` — binds no `n` anywhere. Lean
// runs `induction n` on a goal like that without complaint: it REVERTS every hypothesis that
// mentions `n`, generalizes, applies the recursor, and re-introduces what it reverted. Refusing
// instead left our own documentation teaching a spelling that does not work.
//
// Every assertion below comes as a pair, because totality is exactly the property a weak
// implementation satisfies by saying yes to everything.

/// THE CAPABILITY. The variable is bound nowhere at all — the vacuous generalization.
#[test]
fn induction_on_a_variable_the_goal_never_mentions_generalises_and_certifies() {
    let happy = pred("Happy", vec![k("Bob")]);
    let r = prove(
        vec![("h", happy.clone())],
        happy,
        "induction n . assumption . assumption",
    );
    assert!(
        matches!(r, Ok(true)),
        "`induction n` on a goal that binds no `n` must GENERALIZE, not refuse — it is the \
         verbatim worked example in `ProofStrategy::Induction`'s doc comment: {r:?}"
    );
}

/// THE ROUTE. Generalizing is only allowed to cost what it says it costs: the eliminator plus
/// universal instantiation, both of which already existed. A certificate that reached
/// `[kernel-verified]` through some new assumption would be a different (and worse) answer.
#[test]
fn the_generalising_induction_is_built_from_the_eliminator_and_instantiation_alone() {
    let happy = pred("Happy", vec![k("Bob")]);
    let tree = prove_tree(
        vec![("h", happy.clone())],
        happy,
        "induction n . assumption . assumption",
    );
    assert!(
        uses_rule(
            &tree,
            &InferenceRule::StructuralInduction {
                variable: String::new(),
                ind_type: String::new(),
                step_var: String::new(),
            }
        ),
        "the generalized goal must be proved by the RECURSOR, not by some shortcut that \
         noticed the variable was vacuous: {tree:?}"
    );
    assert!(
        uses_rule(&tree, &InferenceRule::UniversalInstTerm(k("Zero"))),
        "…and the descent back to the original goal must be universal instantiation at a \
         closed inhabitant: {tree:?}"
    );
    assert!(
        !uses_rule(&tree, &InferenceRule::Axiom),
        "nothing here may be taken on the user's word: {tree:?}"
    );
}

/// THE NEGATIVE. A vacuous `∀` proves nothing that was not already true — `Sad(Bob)` does not
/// follow from `Happy(Bob)`, with or without an induction wrapped around it.
#[test]
fn a_generalising_induction_cannot_prove_what_did_not_follow() {
    let r = prove(
        vec![("h", pred("Happy", vec![k("Bob")]))],
        pred("Sad", vec![k("Bob")]),
        "induction n . auto . auto",
    );
    assert!(
        !matches!(r, Ok(true)),
        "generalizing over an unmentioned variable must not make an unrelated goal provable: \
         {r:?}"
    );
}

/// THE BOUNDARY, one rung on the bound side: naming a variable the prefix DOES bind still
/// picks that binder, and still proves the theorem that was stated.
#[test]
fn naming_a_bound_variable_still_inducts_on_that_binder() {
    let p = |t: ProofTerm| pred("P", vec![t]);
    let base = p(k("Zero"));
    let step = forall(
        "kk",
        ProofExpr::Implies(Box::new(p(v("kk"))), Box::new(p(f("Succ", vec![v("kk")])))),
    );
    let goal = forall("n", p(v("n")));
    let r = prove(
        vec![("hb", base), ("hs", step)],
        goal,
        "induction n . auto . auto",
    );
    assert!(
        matches!(r, Ok(true)),
        "the bound-variable path must be untouched by the totality work: {r:?}"
    );
}

/// THE OTHER BOUNDARY, and the one the wave-1 pin in `tactic_surface_extra` is about: a goal
/// that QUANTIFIES has a binder the tactic was meant for, so naming one that is not there is a
/// mistake about the goal — and taking a different binder is how two theorems get proved under
/// one name. The refusal must say which binders exist, and must leave the goal untouched.
///
/// This is what tells the two vacuous cases apart. `Bob is happy` binds nothing, so there is no
/// binder that could have been meant and `induction n` can only mean Lean's generalize-and-
/// induct; `∀m n. Q m n` binds two, and `induction z` meant one of them.
#[test]
fn a_quantified_goal_refuses_a_name_its_prefix_does_not_bind() {
    let goal = forall("m", forall("n", pred("Q", vec![v("m"), v("n")])));
    let mut st = state(vec![], goal.clone());
    let e = match st.induction_on("z") {
        Err(e) => format!("{e:?}"),
        Ok(_) => panic!("`induction z` must not be silently re-aimed at `m` or `n`"),
    };
    assert!(
        e.contains('m') && e.contains('n') && e.contains('z'),
        "the refusal must name the binders that DO exist as well as the one that does not: {e}"
    );
    assert_eq!(
        st.open_goals(),
        1,
        "a refused induction leaves the goal exactly as it was"
    );
    assert_eq!(st.focused_target(), Some(&goal), "…including its target");

    // …and the same goal with the name it DOES bind still splits on that binder.
    let mut st = state(
        vec![],
        forall("m", forall("n", pred("Q", vec![v("m"), v("n")]))),
    );
    st.induction_on("n")
        .expect("`induction n` names a binder the prefix has");
    assert_eq!(
        st.open_goals(),
        2,
        "the named binder splits into a base and a step"
    );
}

/// THE CAPTURE TRAP, which is why the generalization REVERTS instead of just wrapping a `∀`
/// around the goal. A hypothesis stated about the LOCAL `n` would sit under the eliminator's
/// own `λn`, which shadows it: the branch would be reading a hypothesis about a different `n`.
/// So no branch may keep a hypothesis mentioning the induction variable — and what was taken
/// away must come back, under its own name, the way Lean's `induction` restores it.
#[test]
fn induction_on_a_free_local_reverts_every_hypothesis_about_it() {
    let p = |t: ProofTerm| pred("P", vec![t]);
    let scheme = vec![
        ("hb", p(k("Zero"))),
        (
            "hs",
            forall(
                "kk",
                ProofExpr::Implies(Box::new(p(v("kk"))), Box::new(p(f("Succ", vec![v("kk")])))),
            ),
        ),
        ("h", p(v("n"))),
    ];
    let mut st = state(scheme, p(v("n")));
    st.induction_on("n")
        .expect("`induction n` must apply to a free local");
    assert_eq!(
        st.open_goals(),
        2,
        "a Nat induction leaves a base case and a step case"
    );
    for branch in 0..2 {
        let g = st.focus().expect("a branch is open").clone();
        assert!(
            !g.hyps
                .iter()
                .any(|h| format!("{:?}", h.prop).contains("Variable(\"n\")")),
            "branch {branch} still holds a hypothesis about the LOCAL `n`, which the \
             eliminator's own binder shadows: {:?}",
            g.hyps
                .iter()
                .map(|h| (&h.name, &h.prop))
                .collect::<Vec<_>>()
        );
        assert!(
            g.hyps.iter().any(|h| h.name == "h"),
            "branch {branch} must have `h` restored under its own name: {:?}",
            g.hyps.iter().map(|h| &h.name).collect::<Vec<_>>()
        );
        st.assumption()
            .expect("the restored hypothesis closes the branch");
    }
    let r = st.qed().expect("no goals remain");
    assert!(
        r.verified,
        "the reverting generalization must CERTIFY — every step of it is `ModusPonens`, \
         `UniversalInstTerm` and the recursor: {:?} [local `n` in the kernel context: {:?}]",
        r.verification_error,
        r.kernel_ctx.get("n")
    );
}

/// The same theorem at the script surface, which is where a human meets it: nothing in the
/// script says a revert happened.
#[test]
fn the_reverted_hypotheses_come_back_under_their_own_names() {
    let p = |t: ProofTerm| pred("P", vec![t]);
    let premises = vec![
        ("hb", p(k("Zero"))),
        (
            "hs",
            forall(
                "kk",
                ProofExpr::Implies(Box::new(p(v("kk"))), Box::new(p(f("Succ", vec![v("kk")])))),
            ),
        ),
        ("h", p(v("n"))),
    ];
    let r = prove(premises, p(v("n")), "induction n . assumption . assumption");
    assert!(
        matches!(r, Ok(true)),
        "`induction n` must leave each branch ready for `assumption`, hypotheses restored: {r:?}"
    );
}

/// THE REFUSAL, and it is the honest half. `ProofExpr` carries no types, so a generalization
/// over a variable the statement DOES talk about has to learn its domain from somewhere; the
/// only place it can is the induction principle stated as premises. With nothing in scope
/// saying what `n` ranges over, guessing `Nat` would build an eliminator over a domain nothing
/// supports — a tree the kernel must refuse, reported as success. The vacuous case is
/// different and must still go through: it binds a variable the motive never mentions, so any
/// inhabited inductive serves.
#[test]
fn a_free_local_with_no_scheme_in_scope_is_refused_rather_than_guessed() {
    let mut st = state(
        vec![("h", pred("P", vec![v("n")]))],
        pred("P", vec![v("n")]),
    );
    let e = match st.induction_on("n") {
        Err(e) => format!("{e:?}"),
        Ok(_) => panic!("nothing says what `n` ranges over — this must not be guessed"),
    };
    assert!(
        e.contains("free local") && e.contains("induction principle"),
        "the refusal must say WHAT is missing and how to supply it: {e}"
    );
    assert_eq!(st.open_goals(), 1, "a refused tactic changes nothing");

    // …while the vacuous case, which needs no domain, still applies.
    let happy = pred("Happy", vec![k("Bob")]);
    let mut st = state(vec![("h", happy.clone())], happy);
    st.induction_on("n")
        .expect("a vacuous generalization needs no scheme");
    assert_eq!(st.open_goals(), 2, "…and it splits into a base and a step");
}

// ============================================================================================
// `DerivationTree::substitution` — the field the certifier never reads
// ============================================================================================
//
// Ten sites in the backward chainer call `with_substitution`, and `grep` for `.substitution`
// in `certifier.rs` and `verify.rs` returns NOTHING. A recorded substitution is therefore only
// ever true by accident, and the way it can be WRONG is precise: a node whose conclusion still
// mentions a variable its own substitution binds is claiming a proposition the search had
// already moved past. The certified term's type would then be one proposition and the node's
// recorded conclusion another — and every rule that computes from its OWN conclusion
// (`StructuralInduction`'s motive, `Rewrite`'s, `extract_motive_body`) would compute from the
// stale one.
//
// The invariant that makes the field honest without reading it: **every node's conclusion is a
// fixed point of its own substitution.** This sweeps the chainer over goal shapes that exercise
// each `with_substitution` site and checks it. It is an instrument, not a capability: if it
// ever goes red, a stale conclusion is in a certificate.

fn substitution_is_idempotent_on(tree: &DerivationTree, path: &str, out: &mut Vec<String>) {
    let applied =
        logicaffeine_proof::unify::apply_subst_to_expr(&tree.conclusion, &tree.substitution);
    if applied != tree.conclusion {
        out.push(format!(
            "{path} [{}]: conclusion {:?} is not closed under its own substitution {:?} \
             (it would have been {:?})",
            tree.rule.canonical_name(),
            tree.conclusion,
            tree.substitution,
            applied
        ));
    }
    for (i, p) in tree.premises.iter().enumerate() {
        substitution_is_idempotent_on(p, &format!("{path}.{i}"), out);
    }
}

#[test]
fn no_certified_node_records_a_substitution_its_conclusion_has_not_taken() {
    // Each row drives a different `with_substitution` site: direct fact match, backward chain
    // through an implication, universal instantiation, modus tollens, disjunctive syllogism,
    // and a multi-binder rule whose middle term is bound while solving the antecedent.
    let rows: Vec<(&str, Vec<(&str, ProofExpr)>, ProofExpr)> = vec![
        (
            "direct fact match",
            vec![("h", pred("P", vec![k("a")]))],
            pred("P", vec![k("a")]),
        ),
        (
            "universal fact instantiated",
            vec![("h", forall("x", pred("P", vec![v("x")])))],
            pred("P", vec![k("a")]),
        ),
        (
            "backward chain through a rule",
            vec![
                (
                    "h1",
                    forall(
                        "x",
                        ProofExpr::Implies(
                            Box::new(pred("P", vec![v("x")])),
                            Box::new(pred("Q", vec![v("x")])),
                        ),
                    ),
                ),
                ("h2", pred("P", vec![k("a")])),
            ],
            pred("Q", vec![k("a")]),
        ),
        (
            "a middle term bound while solving the antecedent",
            vec![
                (
                    "h1",
                    forall(
                        "x",
                        forall(
                            "y",
                            ProofExpr::Implies(
                                Box::new(ProofExpr::And(
                                    Box::new(pred("R", vec![v("x"), v("y")])),
                                    Box::new(pred("S", vec![v("y")])),
                                )),
                                Box::new(pred("T", vec![v("x")])),
                            ),
                        ),
                    ),
                ),
                ("h2", pred("R", vec![k("a"), k("b")])),
                ("h3", pred("S", vec![k("b")])),
            ],
            pred("T", vec![k("a")]),
        ),
        (
            "disjunctive syllogism",
            vec![
                ("h1", or(pred("P", vec![k("a")]), pred("Q", vec![k("a")]))),
                ("h2", not(pred("P", vec![k("a")]))),
            ],
            pred("Q", vec![k("a")]),
        ),
        (
            "modus tollens",
            vec![
                (
                    "h1",
                    forall(
                        "x",
                        ProofExpr::Implies(
                            Box::new(pred("P", vec![v("x")])),
                            Box::new(pred("Q", vec![v("x")])),
                        ),
                    ),
                ),
                ("h2", not(pred("Q", vec![k("a")]))),
            ],
            not(pred("P", vec![k("a")])),
        ),
    ];

    let mut stale = Vec::new();
    let mut proved = 0usize;
    for (label, premises, goal) in rows {
        let mut st = state(premises, goal);
        if st.auto().is_err() {
            continue;
        }
        let Some(tree) = st.assembled() else { continue };
        proved += 1;
        substitution_is_idempotent_on(&tree, label, &mut stale);
    }
    assert!(
        proved >= 3,
        "the instrument must actually reach the chainer — only {proved} of the rows proved, so \
         the sweep would pass vacuously"
    );
    assert!(
        stale.is_empty(),
        "a certified node records a substitution its own conclusion never took — the certifier \
         never reads `DerivationTree::substitution`, so nothing downstream would notice:\n{}",
        stale.join("\n")
    );
}

// ============================================================================================
// `decide`'s integer window — the FIFTH engine, and the one that disagreed
// ============================================================================================
//
// `reduction::ediv_emod_i64`'s doc comment is the SPECIFICATION every engine that folds
// `div`/`mod` on literals implements, each written from the text:
//
//     b ≠ 0  ⟹  a = b·q + r  ∧  0 ≤ r < |b|      (the unique such pair)
//     b = 0  ⟹  q = 0  ∧  r = a
//
// `decide::eval_int` was a fifth copy and it TRUNCATED, so the router answered `div(-7, 2) =
// -3` where all four kernel engines answer `-4`, and got stuck at a zero divisor where they
// answer `0`. Both directions are harmful and neither is loud: the router calls a TRUE goal
// `Verdict::False` — whose documented meaning is "nothing else will prove it either" — and
// hands a FALSE goal to the trust door as an `ArithDecision` the arithmetic oracle then refuses,
// which is a tactic reporting success without a checkable object.
//
// The oracle is the independent model here: `arith::prove_int_eq` decides through the KERNEL's
// `normalize`, so every row below is checked against an implementation the router shares no
// code with.

fn int(n: i64) -> ProofTerm {
    k(&n.to_string())
}

#[test]
fn decide_folds_division_the_way_all_four_kernel_engines_do() {
    // (dividend, divisor, quotient, remainder) — straight from the specification, with the
    // negative dividends that tell Euclidean from truncated apart, and the zero divisor that
    // tells TOTAL from stuck apart.
    let rows: [(i64, i64, i64, i64); 10] = [
        (7, 2, 3, 1),
        (-7, 2, -4, 1),
        (7, -2, -3, 1),
        (-7, -2, 4, 1),
        (-6, 3, -2, 0),
        (6, -3, -2, 0),
        (-1, 5, -1, 4),
        (5, 0, 0, 5),
        (-5, 0, 0, -5),
        (0, 0, 0, 0),
    ];
    for (a, b, q, r) in rows {
        // `a = b·q + r` with `0 ≤ r < |b|` — the specification restated, so a row that is
        // itself wrong cannot be satisfied by a wrong implementation agreeing with it.
        if b != 0 {
            assert_eq!(a, b * q + r, "row ({a}, {b}) is not a division at all");
            assert!(
                0 <= r && r < b.abs(),
                "row ({a}, {b}) has a remainder outside [0, |b|)"
            );
        }
        for (op, want) in [("div", q), ("mod", r)] {
            let truth = eq(f(op, vec![int(a), int(b)]), int(want));
            let mut st = state(vec![], truth.clone());
            let applied = st.decide().map(|_| ()).map_err(|e| format!("{e:?}"));
            assert!(
                applied.is_ok(),
                "`decide` must settle `{op}({a}, {b}) = {want}` — it is what the kernel \
                 computes: {applied:?}"
            );
            let r_ = st.qed().map(|v| v.verified);
            assert!(
                matches!(r_, Ok(true)),
                "…and the certificate must survive the trust door, where the arithmetic \
                 oracle re-derives it through the kernel's own `normalize`: {r_:?}"
            );
        }
    }
}

/// THE NEGATIVE, which is the half that catches a router that simply agreed with everything:
/// the TRUNCATED answer must now be refused, and refused as FALSE (not as "undecided"), because
/// the router evaluated it and knows.
#[test]
fn decide_refuses_the_truncated_answer_and_says_it_is_false() {
    for (a, b, truncated) in [(-7i64, 2i64, -3i64), (7, -2, -4), (-1, 5, 0)] {
        let wrong = eq(f("div", vec![int(a), int(b)]), int(truncated));
        let mut st = state(vec![], wrong);
        let e = match st.decide() {
            Err(e) => format!("{e:?}"),
            Ok(_) => panic!("`div({a}, {b}) = {truncated}` is the TRUNCATED answer and is false"),
        };
        assert!(
            e.contains("FALSE"),
            "the router evaluated it, so it must report FALSE rather than `undecided`: {e}"
        );
    }
    // A zero divisor is no longer "not in the fragment" either — it is total, and a wrong
    // total answer is FALSE.
    let mut st = state(vec![], eq(f("div", vec![int(5), int(0)]), int(1)));
    let e = format!(
        "{:?}",
        st.decide()
            .map(|_| ())
            .expect_err("`div(5, 0) = 1` is false")
    );
    assert!(
        e.contains("FALSE"),
        "`div(5, 0)` is 0, so `= 1` is false, not undecided: {e}"
    );
}

/// The comparison router reads the same evaluator, so the Euclidean pair has to reach it too —
/// a ground `le(div(-7, 2), -4) = true` is decided by computation on both sides of the door.
#[test]
fn a_ground_comparison_over_a_negative_quotient_decides_the_same_way() {
    let goal = le(f("div", vec![int(-7), int(2)]), int(-4));
    let mut st = state(vec![], goal);
    st.decide()
        .expect("`div(-7, 2) ≤ -4` holds — the quotient IS -4");
    let r = st.qed().map(|v| v.verified);
    assert!(matches!(r, Ok(true)), "…and it must certify: {r:?}");

    let strict = lt(f("div", vec![int(-7), int(2)]), int(-4));
    let mut st = state(vec![], strict);
    let e = format!(
        "{:?}",
        st.decide()
            .map(|_| ())
            .expect_err("`div(-7, 2) < -4` is false")
    );
    assert!(
        e.contains("FALSE"),
        "a truncating router would have called this TRUE: {e}"
    );
}

// ============================================================================================
// THE SECOND KERNEL'S BLIND SPOT — certificates for the tactic reduction primitives
//
// `recheck.rs`'s `PRIMITIVE_RULES` marks 36 of 45 reduction rules `Unchecked`, and the note
// beside them is honest: re-implementing a ring normaliser or Fourier–Motzkin in the second
// kernel would be "a copy wearing an independence badge". Lean does not do that either. Lean's
// `omega` runs its search in UNTRUSTED code and emits a `Justification`
// (`Lean/Elab/Tactic/Omega/Core.lean:39-56`) that the kernel then CHECKS by replaying
// arithmetic through `combo_sat'` (`Init/Omega/Constraint.lean:370`). The search is a big
// program; the check is a small one.
//
// `logicaffeine_proof::tactic_cert` is that small program for `try_ring`, `try_lia` and
// `try_cc`. These tests are its trust surface, and they come in the four shapes this campaign
// has learned to demand:
//
//   * the POSITIVE — the checker verifies what the first kernel solves;
//   * the NEGATIVE — a FORGED certificate is refused, which is what separates a checker from a
//     believer;
//   * the BOUNDARY — one rung either side of every gate (the sort, the fragment, the window);
//   * the DIFFERENTIAL — over an adversarial grid, the checker never verifies a goal the FIRST
//     kernel refuses. That direction is the cry-wolf direction: a re-checker that out-decided
//     the first kernel would make `double_check` report a soundness alarm about a valid proof,
//     and an alarm that fires on valid proofs is an alarm nobody reads.
// ============================================================================================

use logicaffeine_kernel::{normalize, prelude::StandardLibrary, BigInt, Context, Term as KTerm};
use logicaffeine_proof::tactic_cert::{self, CcCert, CcStep, CertifiablePrimitive, LiaCert, Syn};

fn sn(s: &str) -> Syn {
    Syn::Name(s.to_string())
}
fn sl(n: i64) -> Syn {
    Syn::Lit(n)
}
fn sbin(op: &str, a: Syn, b: Syn) -> Syn {
    tactic_cert::syn_binary(op, a, b)
}
fn sapp(fun: Syn, x: Syn) -> Syn {
    tactic_cert::syn_apply(fun, x)
}
fn seq3(ty: &str, a: Syn, b: Syn) -> Syn {
    tactic_cert::syn_typed_eq(ty, a, b)
}
fn seq2(a: Syn, b: Syn) -> Syn {
    sbin("Eq", a, b)
}
fn simplies(h: Syn, c: Syn) -> Syn {
    sbin("implies", h, c)
}

/// The FIRST kernel's own verdict on `<rule> goal`: `true` exactly when its reduction rule
/// produces the primitive's success constructor.
///
/// This is the only way to ask the question that does not presuppose the answer — it runs
/// `normalize` over the very term the checker was handed, through
/// `tactic_cert::term_of_syn`, so a differential between the two is a differential and not two
/// different questions asked of two different terms.
fn first_kernel_solves(ctx: &Context, p: CertifiablePrimitive, goal: &Syn) -> bool {
    let app = KTerm::App(
        Box::new(KTerm::Global(p.rule_name().to_string())),
        Box::new(tactic_cert::term_of_syn(goal)),
    );
    match normalize(ctx, &app) {
        KTerm::App(head, _) => {
            matches!(head.as_ref(), KTerm::Global(g) if g == p.success_constructor())
        }
        _ => false,
    }
}

fn kernel_context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

/// The adversarial grid: every goal shape the three primitives can be handed, with the ones
/// that must be REFUSED outnumbering the ones that must be accepted.
///
/// A positive-only grid is satisfied by a checker that says `true` to everything, which is why
/// the near-misses are here: `(x+y)² = x² + y²` differs from the identity by one monomial,
/// `3 ≤ 2` from `2 ≤ 3` by the order of two literals, and `f p = f q` from the congruence by
/// having no hypothesis to stand on.
fn certificate_grid() -> Vec<(CertifiablePrimitive, Syn, &'static str)> {
    use CertifiablePrimitive::{Cc, Lia, Ring};
    let x = || sn("x");
    let y = || sn("y");
    let z = || sn("z");
    let sq = |t: Syn| sbin("mul", t.clone(), t);
    vec![
        // ── RING ───────────────────────────────────────────────────────────────────────────
        (
            Ring,
            seq3(
                "Int",
                sq(sbin("add", x(), y())),
                sbin(
                    "add",
                    sbin("add", sq(x()), sbin("mul", sl(2), sbin("mul", x(), y()))),
                    sq(y()),
                ),
            ),
            "(x+y)² = x² + 2xy + y²",
        ),
        (
            Ring,
            seq3(
                "Int",
                sq(sbin("add", x(), y())),
                sbin("add", sq(x()), sq(y())),
            ),
            "(x+y)² = x² + y² — the missing cross term",
        ),
        (
            Ring,
            seq3(
                "Int",
                sbin("mul", sbin("sub", x(), y()), sbin("add", x(), y())),
                sbin("sub", sq(x()), sq(y())),
            ),
            "(x−y)(x+y) = x² − y²",
        ),
        (Ring, seq3("Int", sbin("add", x(), sl(0)), x()), "x + 0 = x"),
        (
            Ring,
            seq3("Int", sbin("add", x(), sl(1)), x()),
            "x + 1 = x — off by one",
        ),
        (
            Ring,
            seq3("Int", sbin("mul", x(), sl(0)), sl(0)),
            "x · 0 = 0",
        ),
        (Ring, seq3("Int", sbin("sub", x(), x()), sl(0)), "x − x = 0"),
        (
            Ring,
            seq3("Int", sbin("add", x(), y()), sbin("add", y(), x())),
            "commutativity",
        ),
        (
            Ring,
            seq3(
                "Int",
                sbin("mul", x(), sbin("add", y(), z())),
                sbin("add", sbin("mul", x(), y()), sbin("mul", x(), z())),
            ),
            "distributivity in three variables",
        ),
        (
            Ring,
            seq3("Int", sbin("div", x(), y()), sbin("div", x(), y())),
            "div is outside the polynomial fragment even when both sides agree",
        ),
        (
            Ring,
            seq3("Bool", x(), x()),
            "a reflexive equation at a NON-ring type",
        ),
        (
            Ring,
            seq3("Nat", sbin("add", x(), y()), sbin("add", y(), x())),
            "commutativity at Nat, which the first kernel also admits as a ring",
        ),
        (
            Ring,
            seq2(x(), x()),
            "the UNTYPED equation `ring` does not read",
        ),
        (
            Ring,
            sbin("le", sl(2), sl(3)),
            "a Bool order fact handed to the ring tactic",
        ),
        // ── LIA ────────────────────────────────────────────────────────────────────────────
        (Lia, sbin("le", sl(2), sl(3)), "2 ≤ 3"),
        (Lia, sbin("le", sl(3), sl(2)), "3 ≤ 2 — false"),
        (Lia, sbin("le", sl(3), sl(3)), "3 ≤ 3 — the boundary"),
        (
            Lia,
            sbin("lt", sl(3), sl(3)),
            "3 < 3 — one rung past the boundary, false",
        ),
        (Lia, sbin("lt", sl(2), sl(3)), "2 < 3"),
        (Lia, sbin("ge", sl(3), sl(2)), "3 ≥ 2"),
        (Lia, sbin("gt", sl(2), sl(3)), "2 > 3 — false"),
        (
            Lia,
            sbin("le", x(), x()),
            "x ≤ x — valid for every x, by cancellation",
        ),
        (Lia, sbin("lt", x(), x()), "x < x — false for every x"),
        (Lia, sbin("le", x(), sbin("add", x(), sl(1))), "x ≤ x + 1"),
        (
            Lia,
            sbin("le", sbin("add", x(), sl(1)), x()),
            "x + 1 ≤ x — false",
        ),
        (
            Lia,
            sbin("le", sbin("mul", sl(2), x()), sbin("mul", sl(2), x())),
            "2x ≤ 2x",
        ),
        (
            Lia,
            sbin("le", x(), y()),
            "x ≤ y — true of some assignments, valid for none",
        ),
        (
            Lia,
            sbin("le", sbin("mul", x(), y()), sbin("mul", x(), y())),
            "x·y is not linear, even reflexively",
        ),
        (
            Lia,
            seq2(x(), x()),
            "an equation handed to the order tactic",
        ),
        // ── CC ─────────────────────────────────────────────────────────────────────────────
        (Cc, seq2(x(), x()), "x = x by reflexivity"),
        (Cc, seq2(x(), y()), "x = y with nothing to stand on"),
        (
            Cc,
            simplies(seq2(x(), y()), seq2(sapp(sn("f"), x()), sapp(sn("f"), y()))),
            "x = y ⟹ f x = f y",
        ),
        (
            Cc,
            simplies(seq2(x(), y()), seq2(sapp(sn("f"), y()), sapp(sn("f"), x()))),
            "…and in the other orientation",
        ),
        (
            Cc,
            simplies(
                seq2(sn("a"), sn("b")),
                seq2(sapp(sn("g"), sn("p")), sapp(sn("g"), sn("q"))),
            ),
            "an unrelated hypothesis proves no congruence",
        ),
        (
            Cc,
            simplies(seq2(x(), y()), simplies(seq2(y(), z()), seq2(x(), z()))),
            "x = y ⟹ y = z ⟹ x = z",
        ),
        (
            Cc,
            simplies(seq2(x(), y()), simplies(seq2(y(), z()), seq2(x(), sn("w")))),
            "…but the chain does not reach an unmentioned w",
        ),
        (
            Cc,
            simplies(
                seq2(x(), y()),
                seq2(
                    sapp(sn("f"), sapp(sn("f"), x())),
                    sapp(sn("f"), sapp(sn("f"), y())),
                ),
            ),
            "congruence twice over: f (f x) = f (f y)",
        ),
        (
            Cc,
            simplies(
                seq2(x(), y()),
                seq2(sbin("h", x(), z()), sbin("h", y(), z())),
            ),
            "a two-argument symbol congrues in its first argument",
        ),
        (
            Cc,
            simplies(
                seq2(x(), y()),
                seq2(sbin("h", z(), x()), sbin("h", z(), y())),
            ),
            "…and in its second",
        ),
        (
            Cc,
            sbin("le", sl(2), sl(3)),
            "an order fact handed to congruence closure",
        ),
    ]
}

/// **THE DIFFERENTIAL.** The checker may verify only what the first kernel also solves.
///
/// This is the one-sidedness the `Unchecked` → `Checked` flip rests on, and it is asserted in
/// the dangerous direction: a `true` here where the first kernel says `false` would be a second
/// kernel reducing a redex the first leaves stuck, which surfaces as `DoubleCheck::Disagree`
/// on a proof that is perfectly valid. The other direction is allowed and is reported, not
/// asserted — that is the honest incompleteness `Unchecked` already knows how to say.
#[test]
fn the_certificate_checker_never_out_decides_the_first_kernel() {
    let ctx = kernel_context();
    let mut overreach = Vec::new();
    let mut shortfall = Vec::new();
    for (p, goal, note) in certificate_grid() {
        let theirs = first_kernel_solves(&ctx, p, &goal);
        let ours = p.verify(&goal);
        if ours && !theirs {
            overreach.push(format!("{} on `{note}`", p.rule_name()));
        }
        if theirs && !ours {
            shortfall.push(format!("{} on `{note}`", p.rule_name()));
        }
    }
    assert!(
        overreach.is_empty(),
        "the re-checker decided a goal the FIRST kernel leaves stuck — that is a \
         `DoubleCheck::Disagree` waiting to fire on a valid proof:\n  {}",
        overreach.join("\n  ")
    );
    assert!(
        shortfall.is_empty(),
        "these are goals the first kernel solves and the certificate layer cannot corroborate. \
         Each is honest (the caller falls back to `Unchecked`), but each is also a rung the \
         second kernel is still blind on, so they are pinned rather than tolerated:\n  {}",
        shortfall.join("\n  ")
    );
}

/// The positive half, stated as a floor rather than as a list: the checker must corroborate at
/// least this many of the grid's goals, or it has become a function that says `false`.
///
/// A checker that verifies nothing passes every soundness test ever written. This is the test
/// that such a checker fails.
#[test]
fn the_certificate_checker_corroborates_a_real_share_of_the_grid() {
    let mut verified = Vec::new();
    for (p, goal, note) in certificate_grid() {
        if p.verify(&goal) {
            verified.push((p.rule_name(), note));
        }
    }
    for want in ["try_ring", "try_lia", "try_cc"] {
        assert!(
            verified.iter().any(|(r, _)| *r == want),
            "`{want}` corroborated NOTHING in the grid — a checker that never verifies is not a \
             checker. Verified: {verified:?}"
        );
    }
    assert!(
        verified.len() >= 16,
        "the grid has sixteen goals the first kernel solves; the checker reached {} of them: \
         {verified:?}",
        verified.len()
    );
}

/// **THE FORGERY, LIA.** A Farkas certificate is believed because it is CHECKED, so every way
/// of lying with one has to be refused.
///
/// Four lies, each aimed at a different clause of the check: a negative multiplier (not a
/// Farkas weight at all), an all-zero combination (refutes nothing, and `0 ≤ 0` is not a
/// contradiction), a length that does not match the system, and a genuine certificate pointed
/// at a DIFFERENT goal.
#[test]
fn a_forged_farkas_certificate_is_refused_four_ways() {
    let true_goal = sbin("le", sl(2), sl(3));
    let false_goal = sbin("le", sl(3), sl(2));

    let genuine = tactic_cert::emit_lia(&true_goal).expect("¬(2 ≤ 3) is 1 < 0 — refutable");
    assert!(
        tactic_cert::check_lia(&true_goal, &genuine),
        "the genuine certificate must check"
    );

    assert!(
        !tactic_cert::check_lia(
            &true_goal,
            &LiaCert::from_multipliers(vec![BigInt::from_i64(-1)])
        ),
        "a NEGATIVE multiplier is not a Farkas weight — with one, every system is refutable"
    );
    assert!(
        !tactic_cert::check_lia(&true_goal, &LiaCert::from_multipliers(vec![BigInt::zero()])),
        "the all-zero combination derives `0 ≤ 0`, which is satisfied, not contradicted"
    );
    assert!(
        !tactic_cert::check_lia(
            &true_goal,
            &LiaCert::from_multipliers(vec![BigInt::from_i64(1), BigInt::from_i64(1)])
        ),
        "a certificate must have exactly one multiplier per constraint of the system it refutes"
    );
    assert!(
        !tactic_cert::check_lia(&false_goal, &genuine),
        "the multipliers that refute ¬(2 ≤ 3) must not be accepted about ¬(3 ≤ 2) — a \
         certificate names a system, and checking it means re-deriving the contradiction from \
         THAT system"
    );
}

/// A Farkas weight scaled by a positive integer is still a Farkas weight, and a weight scaled
/// by zero is not — the boundary of the non-negativity clause, one rung either side.
#[test]
fn farkas_multipliers_scale_but_do_not_vanish() {
    let goal = sbin("le", sl(2), sl(3));
    let genuine = tactic_cert::emit_lia(&goal).expect("refutable");
    let scaled: Vec<BigInt> = genuine
        .multipliers()
        .iter()
        .map(|m| m.mul(&BigInt::from_i64(7)))
        .collect();
    assert!(
        tactic_cert::check_lia(&goal, &LiaCert::from_multipliers(scaled)),
        "7·λ refutes whatever λ refutes — Farkas is a cone, not a point"
    );
    let vanished: Vec<BigInt> = genuine
        .multipliers()
        .iter()
        .map(|_| BigInt::zero())
        .collect();
    assert!(
        !tactic_cert::check_lia(&goal, &LiaCert::from_multipliers(vanished)),
        "0·λ is the empty combination and refutes nothing"
    );
}

/// **THE FORGERY, CC.** A congruence proof is a list of steps, so the lies are about the
/// indices: a step citing itself, a step citing a later one, a hypothesis that does not exist,
/// a `Trans` whose middles do not meet, and a proof whose last step is not the goal.
#[test]
fn a_forged_congruence_proof_is_refused_five_ways() {
    let goal = simplies(
        seq2(sn("x"), sn("y")),
        seq2(sapp(sn("f"), sn("x")), sapp(sn("f"), sn("y"))),
    );
    let genuine = tactic_cert::emit_cc(&goal).expect("congruence proves f x = f y from x = y");
    assert!(
        tactic_cert::check_cc(&goal, &genuine),
        "the genuine proof must check"
    );

    for (cert, why) in [
        (vec![CcStep::Sym(0)], "a step may not cite ITSELF"),
        (
            vec![CcStep::Hyp(0), CcStep::Sym(5)],
            "a step may not cite one that comes after it",
        ),
        (vec![CcStep::Hyp(7)], "there is no seventh hypothesis"),
        (
            vec![CcStep::Hyp(0), CcStep::Refl(0), CcStep::Trans(0, 1)],
            "`Trans` needs the two steps to MEET in the middle",
        ),
        (vec![CcStep::Hyp(0)], "the hypothesis is not the conclusion"),
    ] {
        assert!(
            !tactic_cert::check_cc(&goal, &CcCert::from_steps(cert.clone())),
            "{why} — refused, or the checker is a believer: {cert:?}"
        );
    }
    assert!(
        !tactic_cert::check_cc(&goal, &CcCert::from_steps(Vec::new())),
        "the empty proof proves nothing"
    );
}

/// A congruence proof for one goal must not check against another with the same SHAPE.
///
/// The step list mentions hypotheses and terms by index, so a checker that rebuilt the table
/// from the certificate rather than from the GOAL would accept this — which is exactly the
/// mistake that makes a certificate format meaningless.
#[test]
fn a_congruence_proof_does_not_transfer_to_a_different_goal() {
    let proved = simplies(
        seq2(sn("x"), sn("y")),
        seq2(sapp(sn("f"), sn("x")), sapp(sn("f"), sn("y"))),
    );
    let other = simplies(
        seq2(sn("x"), sn("y")),
        seq2(sapp(sn("f"), sn("x")), sapp(sn("g"), sn("y"))),
    );
    let cert = tactic_cert::emit_cc(&proved).expect("the congruence holds");
    assert!(tactic_cert::check_cc(&proved, &cert));
    assert!(
        !tactic_cert::check_cc(&other, &cert),
        "`f x = g y` does not follow from `x = y`, and the steps that prove the congruence \
         must not be readable as a proof of it"
    );
}

/// The congruence certificate is TRIMMED to what the conclusion rests on — the saturation that
/// found it derives hundreds of irrelevant pairs, and a certificate carrying them would still
/// check.
///
/// Not a soundness property: a readability and cost one. The checker's work must be the size of
/// the PROOF, not the size of the SEARCH.
#[test]
fn a_congruence_certificate_carries_only_the_steps_its_conclusion_rests_on() {
    let goal = simplies(
        seq2(sn("x"), sn("y")),
        simplies(seq2(sn("y"), sn("z")), seq2(sn("x"), sn("z"))),
    );
    let cert = tactic_cert::emit_cc(&goal).expect("transitivity of two hypotheses");
    assert!(tactic_cert::check_cc(&goal, &cert));
    assert!(
        cert.steps().len() <= 8,
        "`x = y, y = z ⊢ x = z` is three steps of reasoning; the certificate holds {}: {:?}",
        cert.steps().len(),
        cert.steps()
    );
}

/// **THE READER'S WINDOW.** The certificate layer must read exactly the numerals the first
/// kernel reads — no more.
///
/// The kernel's `reify::extract_slit` matches `Literal::Int` only, so a reflected `SLit` that
/// carries a `Literal::BigInt` is not an `SLit` to it. A re-checker that read that numeral
/// would decide goals the first kernel leaves stuck, which is the cry-wolf direction. This is
/// the one place where matching an i64 window is the CORRECT behaviour rather than a defect,
/// and it is pinned so that widening it can only happen on both sides at once.
#[test]
fn the_certificate_reader_matches_the_first_kernels_numeral_window() {
    let small = KTerm::App(
        Box::new(KTerm::Global("SLit".to_string())),
        Box::new(KTerm::Lit(logicaffeine_kernel::Literal::Int(7))),
    );
    assert_eq!(tactic_cert::syn_of_term(&small), Some(Syn::Lit(7)));

    let big = KTerm::App(
        Box::new(KTerm::Global("SLit".to_string())),
        Box::new(KTerm::Lit(logicaffeine_kernel::Literal::BigInt(
            BigInt::from_i64(7),
        ))),
    );
    assert_eq!(
        tactic_cert::syn_of_term(&big),
        None,
        "the first kernel's `extract_slit` refuses a `BigInt` payload, so this one must too"
    );

    let not_reflection = KTerm::Global("Nat".to_string());
    assert_eq!(tactic_cert::syn_of_term(&not_reflection), None);
}

/// Every reflected goal the checker is handed must survive the round trip through the kernel's
/// `Term` and back, or the differential above is comparing two different terms.
#[test]
fn every_grid_goal_round_trips_through_the_kernels_term_representation() {
    for (p, goal, note) in certificate_grid() {
        let back = tactic_cert::syn_of_term(&tactic_cert::term_of_syn(&goal));
        assert_eq!(
            back.as_ref(),
            Some(&goal),
            "`{}` on `{note}` did not survive Syn → Term → Syn",
            p.rule_name()
        );
    }
}

/// The three primitives, their rule names and their success constructors, kept in step with the
/// names `reduction.rs` actually dispatches on.
///
/// A certificate layer that verified `try_lia` and told the caller to build `DRingSolve` would
/// be checked, correct, and useless. The names are the interface, so they are pinned against
/// the first kernel by RUNNING it: each constructor below is the head the kernel's own
/// reduction produces on a goal that primitive solves.
#[test]
fn each_primitives_success_constructor_is_the_one_the_first_kernel_builds() {
    let ctx = kernel_context();
    let witnesses = [
        (
            CertifiablePrimitive::Ring,
            seq3("Int", sbin("add", sn("x"), sl(0)), sn("x")),
        ),
        (CertifiablePrimitive::Lia, sbin("le", sl(2), sl(3))),
        (CertifiablePrimitive::Cc, seq2(sn("x"), sn("x"))),
    ];
    for (p, goal) in witnesses {
        let app = KTerm::App(
            Box::new(KTerm::Global(p.rule_name().to_string())),
            Box::new(tactic_cert::term_of_syn(&goal)),
        );
        let out = normalize(&ctx, &app);
        let head = match &out {
            KTerm::App(h, _) => match h.as_ref() {
                KTerm::Global(g) => g.clone(),
                other => panic!("`{}` reduced to a non-global head {other:?}", p.rule_name()),
            },
            other => panic!(
                "`{}` did not reduce to an application: {other:?}",
                p.rule_name()
            ),
        };
        assert_eq!(
            head,
            p.success_constructor(),
            "`{}` builds `{head}`, and the certificate layer tells its caller to build `{}`",
            p.rule_name(),
            p.success_constructor()
        );
        assert!(
            p.verify(&goal),
            "…and the certificate layer must corroborate that very goal"
        );
    }
    assert_eq!(CertifiablePrimitive::ALL.len(), 3);
    for p in CertifiablePrimitive::ALL {
        assert_eq!(CertifiablePrimitive::of_rule(p.rule_name()), Some(p));
    }
    assert_eq!(
        CertifiablePrimitive::of_rule("try_omega"),
        None,
        "omega is not yet certified"
    );
}

/// **THE INDEPENDENCE CLAIM, MADE FALSIFIABLE.** The checker must refuse a certificate for a
/// system that HAS a model — which is the one thing a transcription of the search could never
/// be caught failing to do.
///
/// A satisfiable system admits no Farkas refutation, by the lemma. So for every multiplier
/// vector a test can invent, the checker must say no. Sweeping a grid of them is the strongest
/// statement available without a proof: the checker's answer is a property of the ARITHMETIC,
/// not of who asked.
#[test]
fn no_multiplier_vector_refutes_a_satisfiable_system() {
    let satisfiable = [
        sbin("le", sl(3), sl(2)),
        sbin("lt", sn("x"), sn("x")),
        sbin("le", sbin("add", sn("x"), sl(1)), sn("x")),
        sbin("le", sn("x"), sn("y")),
    ];
    for goal in satisfiable {
        for m in [-3i64, -1, 0, 1, 2, 7, 1_000_000] {
            let cert = LiaCert::from_multipliers(vec![BigInt::from_i64(m)]);
            assert!(
                !tactic_cert::check_lia(&goal, &cert),
                "the negation of `{goal:?}` is satisfiable, so NO multiplier refutes it — \
                 λ = {m} was accepted"
            );
        }
        assert!(
            tactic_cert::emit_lia(&goal).is_none(),
            "…and the search must not claim to have found one either: {goal:?}"
        );
    }
}

/// The Kronecker decision procedure must be exact at a magnitude where a machine word is not.
///
/// The bound `B = 2C + 1` is built from the SUM of coefficient magnitudes, so an identity whose
/// coefficients pass `i64` drives the substitution point past it too. If the evaluation wrapped
/// anywhere, the two sides of a NON-identity would collide and the checker would verify a false
/// equation — which is precisely the failure the first kernel's own doc comment gives as its
/// reason for arbitrary precision.
#[test]
fn the_ring_decision_stays_exact_past_the_machine_word() {
    let big = sl(4_000_000_000);
    let x = sn("x");
    // (4e9·x)² = 1.6e19·x², a coefficient no `i64` holds. Stated as a product so the checker
    // has to do the multiplication rather than read the answer.
    let lhs = sbin(
        "mul",
        sbin("mul", big.clone(), x.clone()),
        sbin("mul", big.clone(), x.clone()),
    );
    let rhs = sbin(
        "mul",
        sbin("mul", big.clone(), big.clone()),
        sbin("mul", x.clone(), x.clone()),
    );
    assert!(
        tactic_cert::verify_ring(&seq3("Int", lhs.clone(), rhs.clone())),
        "(4e9·x)·(4e9·x) and (4e9·4e9)·(x·x) are the same polynomial"
    );
    let off_by_one = sbin("add", rhs, sl(1));
    assert!(
        !tactic_cert::verify_ring(&seq3("Int", lhs, off_by_one)),
        "…and one more than it is not — a wrapped evaluation would have equated them"
    );
}

/// The `sub` that is not subtraction: `div` and `mod` are not polynomial, and the checker must
/// refuse them even where both sides are syntactically identical.
///
/// Reflexivity is the trap. `x / y = x / y` is TRUE, and a checker that evaluated it would
/// happily agree — but `ring`'s answer is about polynomials, and a division is not one, so
/// agreeing here would be agreeing about a different question from the one the primitive
/// decides. The first kernel refuses it (`ring::reify` returns `NonPolynomial`); so must this.
#[test]
fn the_ring_fragment_excludes_division_even_reflexively() {
    for op in ["div", "mod"] {
        let e = sbin(op, sn("x"), sn("y"));
        assert!(
            !tactic_cert::verify_ring(&seq3("Int", e.clone(), e)),
            "`{op}` is outside the polynomial fragment; a reflexive instance is still outside it"
        );
    }
    // …and the neighbouring polynomial operation is still inside it.
    assert!(tactic_cert::verify_ring(&seq3(
        "Int",
        sbin("mul", sn("x"), sn("y")),
        sbin("mul", sn("y"), sn("x"))
    )));
}

/// The `Syn` reader keeps a de Bruijn occurrence and a named global APART, where the first
/// kernel's `VarInterner` keeps them apart only by allocating names from `-1_000_000` downwards.
///
/// A hand-built `SVar (-1000000)` collides with the first named global that interner meets, and
/// `ring` would then decide `SVar (-1000000) = x`. This checker cannot: its keys are
/// structural, so `Bound(-1000000)` and `Named("x")` are different unknowns whatever their
/// numbering would have been. The test pins the SAFE direction — the checker must refuse — and
/// the collision itself is reported to the kernel lane as a defect of the first kernel's
/// reification, not fixed here.
#[test]
fn a_de_bruijn_occurrence_is_never_identified_with_a_named_global() {
    let collide = seq3("Int", Syn::Var(-1_000_000), sn("x"));
    assert!(
        !tactic_cert::verify_ring(&collide),
        "a de Bruijn occurrence and a named global are different unknowns"
    );
    let collide_lia = sbin("le", Syn::Var(-1_000_000), sn("x"));
    assert!(
        !tactic_cert::verify_lia(&collide_lia),
        "…and the same two unknowns stay apart in the order fragment"
    );
    // The neighbouring TRUE statement about the same de Bruijn occurrence still decides, so
    // this is about identity and not a blanket refusal of `SVar`.
    assert!(tactic_cert::verify_lia(&sbin(
        "le",
        Syn::Var(0),
        Syn::Var(0)
    )));
    assert!(tactic_cert::verify_ring(&seq3(
        "Int",
        Syn::Var(0),
        Syn::Var(0)
    )));
}
