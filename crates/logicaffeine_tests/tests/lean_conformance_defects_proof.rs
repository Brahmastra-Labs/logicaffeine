//! RED reproductions of the two PROOF-LAYER defects in the conformance corpus's audit.
//!
//! **Defect 6 — `rewrite` captures under a binder.**
//! `tactic.rs::replace_in_expr` rebuilds `ForAll`/`Exists` as
//! `{ variable: variable.clone(), body: re(body) }`: it descends into the body with no
//! α-renaming, and with no check that `variable` shadows the free variables of the pattern
//! *or* of the replacement. So rewriting with `h : x = Alpha` under a binder named `x`
//! substitutes the BOUND occurrences, and rewriting *to* a term mentioning `x` lets that `x`
//! be CAPTURED by the binder. Both are the classical substitution bug.
//!
//! The fix these tests demand is capture-avoiding substitution — α-rename the binder to a
//! fresh name, then substitute — as the ONLY substitution in the proof layer. A narrower
//! "stop descending when the binder shadows the pattern" patch is sound but silently
//! under-rewrites, and a tactic that quietly does less than it says is its own defect;
//! `rewrite_still_rewrites_the_free_occurrence_beside_a_shadowing_binder` and
//! `rewrite_does_not_capture_the_replacements_free_variable_under_a_binder` are written so
//! that patch fails too.
//!
//! **Defect 7 — a `Derivation` can be built that proves nothing.**
//! `try_ring` / `try_lia` / `try_cc` are `add_declaration`ed AXIOMS of type
//! `Syntax -> Derivation`, and a FAILED tactic still reduces to `DAxiom (SName "Error")` —
//! a perfectly good `Derivation`. The gate that makes a literate `## Theorem` sound lives in
//! `interface/repl.rs`'s `Definition` arm (normalise `concludes body`, compare it to the
//! stated goal) and fires only when `proof_goal` is `Some`, which only the literate theorem
//! surface sets; a raw `Definition d : Derivation := try_lia <false goal>` has `None` and
//! skips it. `logicaffeine_kernel/tests/literate_proof_soundness.rs` already locks the
//! theorem surface for `ring`, `lia`, `omega`, `simp`, `auto`, `decide` and the introduction
//! tactics, so the only arm pinned here is `cc`. What the ungated path is actually worth is
//! pinned twice: the derivation must not CLAIM the goal, and holding it must not hand anyone
//! a term of the false proposition's type.

use logicaffeine_kernel::interface::Repl;
use logicaffeine_proof::tactic::combinators::{assumption, intro, rewrite, seq};
use logicaffeine_proof::tactic::ProofState;
use logicaffeine_proof::{ProofExpr, ProofTerm};

// ==== LOCAL HELPERS ====

fn k(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn v(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn pr(name: &str, who: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.to_string(),
        args: vec![who],
        world: None,
    }
}
fn pr2(name: &str, a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.to_string(),
        args: vec![a, b],
        world: None,
    }
}
fn eq(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(a, b)
}
fn all(variable: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll {
        variable: variable.to_string(),
        body: Box::new(body),
    }
}
fn some(variable: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::Exists {
        variable: variable.to_string(),
        body: Box::new(body),
    }
}
fn and(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(l), Box::new(r))
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}
fn not(p: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(p))
}

/// Does the term mention `name` as a variable leaf?
fn term_mentions(t: &ProofTerm, name: &str) -> bool {
    match t {
        ProofTerm::Variable(s) | ProofTerm::BoundVarRef(s) => s == name,
        ProofTerm::Function(_, args) | ProofTerm::Group(args) => {
            args.iter().any(|a| term_mentions(a, name))
        }
        ProofTerm::Constant(_) => false,
    }
}

/// Does the expression mention `name` anywhere? A purely syntactic probe — binding
/// structure is deliberately ignored, because that is exactly what is under test.
fn mentions(e: &ProofExpr, name: &str) -> bool {
    match e {
        ProofExpr::Predicate { args, .. } => args.iter().any(|a| term_mentions(a, name)),
        ProofExpr::Identity(l, r) => term_mentions(l, name) || term_mentions(r, name),
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => mentions(l, name) || mentions(r, name),
        ProofExpr::Not(p) => mentions(p, name),
        ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => mentions(body, name),
        _ => false,
    }
}

/// Split a quantified expression into `(binder, body)`.
fn quantified(e: &ProofExpr) -> Option<(&str, &ProofExpr)> {
    match e {
        ProofExpr::ForAll { variable, body } | ProofExpr::Exists { variable, body } => {
            Some((variable.as_str(), body))
        }
        _ => None,
    }
}

/// The quantified body still depends on ITS OWN binder — the α-invariant way to say
/// "the bound occurrences were not substituted away".
fn binder_is_still_used(e: &ProofExpr) -> bool {
    match quantified(e) {
        Some((variable, body)) => mentions(body, variable),
        None => false,
    }
}

fn target(st: &ProofState) -> ProofExpr {
    st.focused_target().cloned().expect("a focused goal")
}

// ==== DEFECT 6 — `rewrite` captures under a binder ====

/// The pattern side. `hp0 : x = Alpha` speaks about the FREE `x`; the goal's `x` is bound by
/// its own quantifier. Those are different variables that merely share a spelling, so the
/// pattern has no occurrence in the goal at all, and a capture-avoiding `rewrite` touches
/// nothing. Today `replace_in_expr` walks straight through the binder and substitutes the
/// bound occurrences, leaving a quantifier over a body that no longer mentions it.
#[test]
fn rewrite_does_not_substitute_occurrences_bound_by_a_shadowing_binder() {
    for goal in [all("x", pr("P", v("x"))), some("x", pr("P", v("x")))] {
        let premises = vec![eq(v("x"), k("Alpha"))];
        let mut st = ProofState::start(premises, goal.clone());
        match st.rewrite("hp0") {
            // The honest outcome: the pattern's free `x` does not occur in the goal.
            Err(_) => {}
            Ok(_) => {
                let after = target(&st);
                assert!(
                    binder_is_still_used(&after),
                    "rewrite with `hp0 : x = Alpha` must leave the occurrences BOUND inside \
                     {goal:?} alone — they are a different variable: {after:?}"
                );
            }
        }
    }
}

/// The under-rewriting guard. `P(x) ∧ ∀x. Q(x)` has ONE free `x`, in the left conjunct, and
/// bound ones in the right. Capture-avoiding substitution rewrites exactly the free one. A
/// patch that refuses the whole rewrite whenever a shadowing binder appears anywhere fails
/// this test, and so does today's code — which rewrites the bound ones as well.
#[test]
fn rewrite_still_rewrites_the_free_occurrence_beside_a_shadowing_binder() {
    let premises = vec![eq(v("x"), k("Alpha"))];
    let goal = and(pr("P", v("x")), all("x", pr("Q", v("x"))));
    let mut st = ProofState::start(premises, goal);
    st.rewrite("hp0")
        .expect("the FREE occurrence of x does occur, so rewrite must apply");
    let after = target(&st);
    let ProofExpr::And(left, right) = &after else {
        panic!("rewrite must preserve the conjunction: {after:?}");
    };
    assert_eq!(
        **left,
        pr("P", k("Alpha")),
        "the FREE occurrence of x must be rewritten to Alpha: {after:?}"
    );
    assert!(
        binder_is_still_used(right),
        "the occurrences BOUND by the inner ∀x must survive untouched: {after:?}"
    );
}

/// The replacement side — the case a "stop descending when the binder shadows the pattern"
/// patch cannot reach. The pattern `A` is a constant with no free variables, so nothing is
/// shadowed and such a patch descends happily; but the REPLACEMENT mentions `x`, and the
/// binder captures it, silently turning a claim about one entity into a claim about the
/// quantified one. Capture-avoiding substitution α-renames the binder first, so the
/// replacement's `x` stays free and stays distinct from whatever the quantifier now binds.
#[test]
fn rewrite_does_not_capture_the_replacements_free_variable_under_a_binder() {
    let premises = vec![eq(k("A"), v("x"))];
    let goal = all("x", pr2("R", v("x"), k("A")));
    let mut st = ProofState::start(premises, goal);
    st.rewrite("hp0")
        .expect("A occurs in the goal, so rewrite must apply");
    let after = target(&st);
    let Some((binder, body)) = quantified(&after) else {
        panic!("rewrite must preserve the quantifier: {after:?}");
    };
    let ProofExpr::Predicate { args, .. } = body else {
        panic!("rewrite must preserve the predicate: {after:?}");
    };
    assert_eq!(
        args[1],
        v("x"),
        "A must be rewritten to the hypothesis's FREE x: {after:?}"
    );
    assert_ne!(
        binder, "x",
        "the binder must be α-renamed away from the replacement's free x instead of \
         capturing it: {after:?}"
    );
    assert_eq!(
        args[0],
        v(binder),
        "the quantified argument must follow the renamed binder: {after:?}"
    );
}

/// **The soundness form.** Two shapes of the false conclusion the capture makes reachable in
/// the TACTIC's own bookkeeping, driven all the way to the trust door.
///
/// *Flat*: from `x = Alpha`, `P(Alpha)` and `¬P(Beta)` — jointly satisfiable, and between
/// them saying nothing whatever about every entity — `∀x. P(x)` does not follow; with
/// `¬P(Beta)` in scope it is refuted outright. Capture reaches it anyway: `rewrite hp0`
/// rewrites the BOUND `x` and collapses the goal to `∀x. P(Alpha)`, which `intro` +
/// `assumption` close from the premise.
///
/// *Nested*: `∀x. (x = Alpha → ∀x. P(x))` is false for the same reason (instantiate the outer
/// `x` at `Alpha` and the antecedent discharges itself), and here the shadowed hypothesis is
/// introduced INSIDE the proof, so the capture happens with `x` legitimately in scope.
///
/// Whichever way the script goes, `qed` must not come back verified.
#[test]
fn capture_under_a_binder_never_reaches_a_verified_false_conclusion() {
    let flat = (
        vec![
            eq(v("x"), k("Alpha")),
            pr("P", k("Alpha")),
            not(pr("P", k("Beta"))),
        ],
        all("x", pr("P", v("x"))),
        vec![rewrite("hp0"), intro("x"), assumption()],
    );
    let nested = (
        vec![pr("P", k("Alpha")), not(pr("P", k("Beta")))],
        all(
            "x",
            implies(eq(v("x"), k("Alpha")), all("x", pr("P", v("x")))),
        ),
        vec![
            intro("x"),
            intro("h"),
            rewrite("h"),
            intro("x"),
            assumption(),
        ],
    );

    for (premises, goal, script) in [flat, nested] {
        let mut st = ProofState::start(premises, goal.clone());
        match st.run(&seq(script)) {
            // A capture-avoiding `rewrite` never applies, so the script cannot be built.
            Err(_) => {}
            Ok(_) => {
                let r = st.qed().expect("the script left no open goals");
                assert!(
                    !r.verified,
                    "{goal:?} does not follow from its premises — rewriting under the binder \
                     that shadows the equation turned one instance into a universal claim, \
                     and the trust door must refuse it: {:?}",
                    r.proof_term
                );
            }
        }
    }
}

// ==== DEFECT 7 — a `Derivation` that proves nothing ====

/// The false goal used throughout: `48 = 6 * 9`, in the reflected `Syntax` the tactic axioms
/// consume. Ring reduces `try_ring` of it to `DAxiom (SName "Error")` — a well-typed
/// `Derivation` that concludes nothing.
const FALSE_GOAL_SYNTAX: &str = "Definition goal : Syntax := \
     SApp (SApp (SApp (SName \"Eq\") (SName \"Int\")) (SLit 48)) \
     (SApp (SApp (SName \"mul\") (SLit 6)) (SLit 9)).";

/// The `concludes` gate, on the one tactic arm
/// `logicaffeine_kernel/tests/literate_proof_soundness.rs` leaves uncovered. That file
/// already pins `ring` (`ring_rejects_false_closed_equality`, `ring_rejects_non_identity`),
/// `lia` (`lia_rejects_false_inequality`, `lia_rejects_strict_reflexive`) and `omega`
/// (`omega_rejects_false_inequality`), so those are deliberately not duplicated. `cc` has no
/// soundness pin at all: congruence closure over uninterpreted symbols must prove reflexivity
/// and congruence, and refuse everything else — a conclusion that does not follow from the
/// hypotheses, and a congruence claimed across two DIFFERENT function symbols.
#[test]
fn cc_proves_a_congruence_and_refuses_a_non_congruence() {
    let refl =
        Repl::new().execute("## Theorem: CCR\n    Statement: (Eq (f a) (f a)).\n    Proof: cc.");
    assert!(
        refl.is_ok(),
        "cc must prove f(a) = f(a) by reflexivity, got {refl:?}"
    );

    let cong = Repl::new().execute(
        "## Theorem: CCT\n    Statement: (implies (Eq a b) (Eq (f a) (f b))).\n    Proof: cc.",
    );
    assert!(
        cong.is_ok(),
        "cc must prove a = b -> f(a) = f(b) by congruence, got {cong:?}"
    );

    let unfounded =
        Repl::new().execute("## Theorem: CCF1\n    Statement: (Eq (f a) (f b)).\n    Proof: cc.");
    assert!(
        unfounded.is_err(),
        "cc must REJECT f(a) = f(b) with no hypothesis relating a and b, got {unfounded:?}"
    );

    let wrong_symbol = Repl::new().execute(
        "## Theorem: CCF2\n    Statement: (implies (Eq a b) (Eq (f a) (g b))).\n    Proof: cc.",
    );
    assert!(
        wrong_symbol.is_err(),
        "cc must REJECT a = b -> f(a) = g(b): f and g are different symbols, got {wrong_symbol:?}"
    );
}

/// The ungated path, part one: what the `Derivation` itself says. A raw
/// `Definition d : Derivation := try_ring <false goal>` never sets `proof_goal`, so the
/// `concludes` gate never runs and the definition is accepted — deliberately, and
/// `literate_proof_soundness::raw_error_derivation_definition_is_still_allowed` already pins
/// that acceptance. What must hold on top of it is that the object is HONEST: `concludes d`
/// must not reduce to the goal, and the same false statement put through the gated theorem
/// surface with `d` in scope must still be refused.
#[test]
fn an_ungated_derivation_does_not_conclude_its_false_goal() {
    let mut r = Repl::new();
    r.execute(FALSE_GOAL_SYNTAX)
        .expect("define the false goal in Syntax form");
    r.execute("Definition d : Derivation := try_ring goal.")
        .expect("the ungated raw derivation definition is allowed by design");

    let concluded = r
        .execute("Eval concludes d.")
        .expect("evaluate `concludes d`");
    let stated = r.execute("Eval goal.").expect("evaluate the goal");
    assert_ne!(
        concluded, stated,
        "a derivation built from a FAILED tactic must not conclude its goal"
    );

    let restated =
        r.execute("## Theorem: F\n    Statement: 48 equals (mul 6 9).\n    Proof: ring.");
    assert!(
        restated.is_err(),
        "holding `d` must not make the false statement provable at the gated surface: \
         {restated:?}"
    );
}

/// The ungated path, part two — the assertion that decides whether defect 7 is an API-shape
/// problem or a soundness hole. A `Derivation` is DATA: `d` has type `Derivation`, not type
/// `48 = 6 * 9`, so no amount of holding it may yield an inhabitant of the false proposition.
/// `Repl`'s `Definition` arm infers the body's type and then throws it away
/// (`let ty = ty.unwrap_or(inferred_ty)`), never checking the DECLARED type against it — so
/// today `Definition p : (Eq Int 48 (mul 6 9)) := d.` is accepted and `Check p` reports the
/// false proposition as `p`'s type. That is the step that turns "a derivation proves nothing"
/// into "a derivation proves anything", and it must be refused.
#[test]
fn an_ungated_derivation_cannot_be_typed_as_the_false_proposition_it_names() {
    let mut r = Repl::new();
    r.execute(FALSE_GOAL_SYNTAX)
        .expect("define the false goal in Syntax form");
    r.execute("Definition d : Derivation := try_ring goal.")
        .expect("the ungated raw derivation definition is allowed by design");

    let coerced = r.execute("Definition p : (Eq Int 48 (mul 6 9)) := d.");
    assert!(
        coerced.is_err(),
        "a `Derivation` must not be declarable at the type of the proposition it merely \
         mentions — the declared type has to be checked against the inferred one: {coerced:?}"
    );

    let witnessed = r.execute("Check p.");
    assert!(
        witnessed.is_err(),
        "nothing may inhabit `48 = 6 * 9`; the rejected definition must leave no `p` behind: \
         {witnessed:?}"
    );
}
