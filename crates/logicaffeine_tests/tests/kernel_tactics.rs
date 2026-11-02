//! Tactics over the kernel's own goals.
//!
//! A goal is a metavariable with its local context and its type; a tactic assigns the focused one a
//! term that may hold new metavariables, which become the new goals. Tactics are untrusted: an
//! assignment is checked against its goal when it is made, and `qed` hands the closed proof term to
//! the kernel, which checks it against the statement. Every test here states a goal the first-order
//! proof language (`proof::tactic::ProofState`) cannot: a dependent type, a recursion's equation, a
//! goal under a binder of a kernel type.

use logicaffeine_kernel::interface::TermParser;
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::tactics::{GoalState, TacticError, TacticStep};
use logicaffeine_kernel::{infer_type, is_subtype, Context, Term};

fn term(text: &str) -> Term {
    TermParser::parse(text).unwrap_or_else(|e| panic!("`{text}` parses: {e:?}"))
}

fn std_ctx() -> Context {
    StandardLibrary::context().clone()
}

/// The kernel, not the tactic layer, decides: the term `qed` returns has the statement's type.
fn proves(ctx: &Context, proof: &Term, statement: &Term) {
    let ty = infer_type(ctx, proof).unwrap_or_else(|e| panic!("the proof must type-check: {e}"));
    assert!(
        is_subtype(ctx, &ty, statement),
        "the proof proves {ty}, not {statement}"
    );
}

/// `plus n m`, recursing on `n`, so `plus (Succ k) m` computes to `Succ (plus k m)` and
/// `plus n Zero` does not compute at all for an open `n`.
fn with_plus(ctx: &mut Context) {
    ctx.declare(
        "plus",
        term("Nat -> Nat -> Nat"),
        term(
            "fix plus => fun (n : Nat) => fun (m : Nat) => \
             match n return (fun (x : Nat) => Nat) with m | fun (k : Nat) => Succ (plus k m) end",
        ),
    )
    .expect("`plus` is a structural recursion");
}

#[test]
fn intro_and_exact_close_a_dependent_goal() {
    let ctx = std_ctx();
    let statement = term("forall (A : Type), forall (x : A), Eq A x x");
    let mut st = GoalState::new(&ctx, &statement).expect("the statement is a type");
    st.intro("A").expect("a Π goal introduces");
    st.intro("x").expect("a Π goal introduces");
    st.exact(&term("refl A x")).expect("`refl A x : Eq A x x`");
    let proof = st.qed().expect("no goal is left");
    proves(&ctx, &proof, &statement);
}

#[test]
fn a_goal_shows_its_hypotheses_and_its_target() {
    let ctx = std_ctx();
    let mut st =
        GoalState::new(&ctx, &term("forall (A : Type), forall (x : A), Eq A x x")).unwrap();
    st.intro("A").unwrap();
    let goals = st.goals();
    assert_eq!(goals.len(), 1);
    assert_eq!(goals[0].hyps, vec![("A".to_string(), term("Type"))]);
    assert_eq!(
        goals[0].target,
        st.read(&term("forall (x : A), Eq A x x")),
        "`A` is the hypothesis, not a global"
    );
}

#[test]
fn a_statement_that_is_not_a_type_has_no_goal_state() {
    let ctx = std_ctx();
    assert!(
        GoalState::new(&ctx, &term("Zero")).is_err(),
        "`Zero` is a term, not a statement"
    );
}

#[test]
fn intro_on_a_goal_that_is_not_a_function_type_is_refused() {
    let ctx = std_ctx();
    let mut st = GoalState::new(&ctx, &term("Eq Nat Zero Zero")).unwrap();
    assert!(
        matches!(st.intro("h"), Err(TacticError::DoesNotApply { .. })),
        "an equation has nothing to introduce"
    );
    assert_eq!(
        st.goals().len(),
        1,
        "a refused tactic leaves the goal as it was"
    );
}

#[test]
fn exact_with_a_term_of_another_type_is_refused_naming_both() {
    let ctx = std_ctx();
    let mut st = GoalState::new(&ctx, &term("Eq Nat Zero Zero")).unwrap();
    let msg = st
        .exact(&term("refl Nat (Succ Zero)"))
        .unwrap_err()
        .to_string();
    assert!(
        msg.contains("Eq Nat Zero Zero") && msg.contains("Succ Zero"),
        "the goal and the term's type: {msg}"
    );
    assert_eq!(
        st.goals().len(),
        1,
        "a refused tactic leaves the goal as it was"
    );
}

#[test]
fn qed_with_goals_open_names_them() {
    let ctx = std_ctx();
    let mut st = GoalState::new(&ctx, &term("forall (P : Prop), P -> P")).unwrap();
    st.intro("P").unwrap();
    match st.qed() {
        Err(TacticError::GoalsRemain(goals)) => {
            assert_eq!(goals.len(), 1);
            assert_eq!(goals[0].target, st.read(&term("P -> P")));
        }
        other => panic!("an open goal must stop `qed`: {other:?}"),
    }
}

#[test]
fn apply_leaves_each_premise_it_cannot_infer_as_a_goal() {
    let ctx = std_ctx();
    let statement = term("forall (P : Prop), forall (Q : Prop), P -> Q -> And P Q");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    for x in ["P", "Q", "hp", "hq"] {
        st.intro(x).unwrap();
    }
    st.apply(&term("conj"))
        .expect("`conj`'s conclusion `And ?a ?b` unifies with `And P Q`");
    let targets: Vec<Term> = st.goals().into_iter().map(|g| g.target).collect();
    assert_eq!(
        targets,
        vec![st.read(&term("P")), st.read(&term("Q"))],
        "`conj`'s two premises, in order; `P` and `Q` were inferred"
    );
    st.exact(&term("hp")).unwrap();
    st.exact(&term("hq")).unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn rfl_closes_an_equation_whose_sides_convert() {
    let mut ctx = std_ctx();
    with_plus(&mut ctx);
    let statement = term("Eq Nat (plus (Succ Zero) (Succ Zero)) (Succ (Succ Zero))");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    st.rfl().expect("both sides compute to 2");
    proves(&ctx, &st.qed().unwrap(), &statement);

    let mut st = GoalState::new(&ctx, &term("Eq Nat (Succ Zero) Zero")).unwrap();
    assert!(st.rfl().is_err(), "1 and 0 do not convert");
}

#[test]
fn induction_change_and_rewrite_prove_a_recursions_equation() {
    let mut ctx = std_ctx();
    with_plus(&mut ctx);
    // `plus n Zero = n` does not compute for an open `n`: it takes induction.
    let statement = term("forall (n : Nat), Eq Nat (plus n Zero) n");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    st.intro("n").unwrap();
    st.induction("n", &[&[], &["k", "ih"]])
        .expect("`n : Nat` is a local of an inductive type");
    assert_eq!(st.goals().len(), 2, "one goal per constructor");
    assert_eq!(st.goals()[0].target, term("Eq Nat (plus Zero Zero) Zero"));
    st.rfl().expect("`plus Zero Zero` computes to `Zero`");
    let goals = st.goals();
    let step = &goals[0];
    assert_eq!(
        step.target,
        st.read(&term("Eq Nat (plus (Succ k) Zero) (Succ k)"))
    );
    assert!(
        step.hyps
            .contains(&("ih".to_string(), st.read(&term("Eq Nat (plus k Zero) k")))),
        "the step names its induction hypothesis: {:?}",
        step.hyps
    );
    st.change(&term("Eq Nat (Succ (plus k Zero)) (Succ k)"))
        .expect("one δι step of `plus`");
    st.rewrite(&term("ih"), false)
        .expect("`ih : plus k Zero = k` rewrites the goal");
    assert_eq!(
        st.goals()[0].target,
        st.read(&term("Eq Nat (Succ k) (Succ k)"))
    );
    st.rfl().unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn change_refuses_a_target_that_does_not_convert() {
    let mut ctx = std_ctx();
    with_plus(&mut ctx);
    let mut st = GoalState::new(&ctx, &term("Eq Nat (plus (Succ Zero) Zero) (Succ Zero)")).unwrap();
    assert!(
        st.change(&term("Eq Nat Zero (Succ Zero)")).is_err(),
        "`plus 1 0` is not `0`"
    );
}

#[test]
fn rewrite_finds_its_left_side_under_a_binder() {
    let ctx = std_ctx();
    // `← h`, with `h : Eq Nat a b`, rewrites the `b` of `forall (y : Nat), Eq Nat a y -> Eq Nat b y`,
    // an occurrence under the binder `y`.
    let statement = term(
        "forall (a : Nat), forall (b : Nat), Eq Nat a b -> forall (y : Nat), Eq Nat a y -> Eq Nat b y",
    );
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    for x in ["a", "b", "h"] {
        st.intro(x).unwrap();
    }
    st.rewrite(&term("h"), true)
        .expect("`← h` rewrites `b` to `a`");
    assert_eq!(
        st.goals()[0].target,
        st.read(&term("forall (y : Nat), Eq Nat a y -> Eq Nat a y"))
    );
    st.intro("y").unwrap();
    st.intro("e").unwrap();
    st.exact(&term("e")).unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn a_tactic_cannot_close_a_goal_with_a_proof_of_something_else() {
    // The tactic layer is untrusted, so `apply` must not leave a goal it cannot state: applying a
    // lemma whose conclusion does not unify with the goal is refused, and the goal stays open.
    let ctx = std_ctx();
    let mut st = GoalState::new(
        &ctx,
        &term("forall (P : Prop), forall (Q : Prop), P -> Or P Q"),
    )
    .unwrap();
    for x in ["P", "Q", "hp"] {
        st.intro(x).unwrap();
    }
    assert!(
        st.apply(&term("conj")).is_err(),
        "`And ?a ?b` does not unify with `Or P Q`"
    );
    st.apply(&term("left"))
        .expect("`left`'s conclusion is `Or ?a ?b`");
    st.exact(&term("hp")).unwrap();
    assert!(st.qed().is_ok());
}

#[test]
fn assumption_closes_a_goal_a_hypothesis_states() {
    let ctx = std_ctx();
    let statement = term("forall (P : Prop), forall (Q : Prop), P -> Q -> Q");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    for x in ["P", "Q", "hp", "hq"] {
        st.intro(x).unwrap();
    }
    st.assumption().expect("`hq : Q`");
    proves(&ctx, &st.qed().unwrap(), &statement);

    let mut st = GoalState::new(&ctx, &term("forall (P : Prop), P -> Eq Nat Zero Zero")).unwrap();
    st.intro("P").unwrap();
    st.intro("hp").unwrap();
    assert!(
        st.assumption().is_err(),
        "no hypothesis states `Eq Nat Zero Zero`"
    );
}

#[test]
fn exfalso_reduces_any_goal_to_false() {
    let ctx = std_ctx();
    let statement = term("False -> Eq Nat Zero (Succ Zero)");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    st.intro("h").unwrap();
    st.exfalso().unwrap();
    assert_eq!(st.goals()[0].target, term("False"));
    st.exact(&term("h")).unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn constructor_left_and_right_build_a_value_of_the_goals_family() {
    let ctx = std_ctx();
    let statement = term("forall (P : Prop), forall (Q : Prop), P -> Q -> And P Q");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    for x in ["P", "Q", "hp", "hq"] {
        st.intro(x).unwrap();
    }
    st.constructor().expect("`And`'s one constructor");
    assert_eq!(st.goals().len(), 2);
    st.assumption().unwrap();
    st.assumption().unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);

    let mut st = GoalState::new(&ctx, &term("True")).unwrap();
    st.constructor()
        .expect("`True`'s one constructor, with no premise");
    assert!(st.goals().is_empty());

    for (side, premise) in [("left", "P"), ("right", "Q")] {
        let statement = term(&format!(
            "forall (P : Prop), forall (Q : Prop), {premise} -> Or P Q"
        ));
        let mut st = GoalState::new(&ctx, &statement).unwrap();
        for x in ["P", "Q", "h"] {
            st.intro(x).unwrap();
        }
        if side == "left" {
            st.left().unwrap()
        } else {
            st.right().unwrap()
        }
        assert_eq!(
            st.goals()[0].target,
            st.read(&term(premise)),
            "`{side}` leaves its own disjunct"
        );
        st.exact(&term("h")).unwrap();
        proves(&ctx, &st.qed().unwrap(), &statement);
    }
}

#[test]
fn exists_supplies_the_witness_and_leaves_its_property() {
    let ctx = std_ctx();
    let statement = term("Ex Nat (fun (n : Nat) => Eq Nat n Zero)");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    st.exists(&term("Zero"))
        .expect("`Ex`'s constructor at the witness `Zero`");
    assert_eq!(
        st.goals()[0].target,
        term("Eq Nat Zero Zero"),
        "the property at the witness, β-reduced"
    );
    st.rfl().unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn have_states_an_intermediate_fact_proved_first() {
    let ctx = std_ctx();
    let statement = term("forall (P : Prop), forall (Q : Prop), (P -> Q) -> P -> And Q Q");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    for x in ["P", "Q", "f", "hp"] {
        st.intro(x).unwrap();
    }
    st.have("hq", &term("Q")).expect("a fact stated as a type");
    assert_eq!(st.goals()[0].target, st.read(&term("Q")), "the fact first");
    st.exact(&term("f hp")).unwrap();
    assert!(
        st.goals()[0].hyps.iter().any(|(n, _)| n == "hq"),
        "then the goal, with the fact as a hypothesis"
    );
    st.constructor().unwrap();
    st.exact(&term("hq")).unwrap();
    st.exact(&term("hq")).unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);
}

/// `Vect (a : Type) : Nat → Type := VNil : Vect a Zero | VCons : Π(n : Nat). a → Vect a n → Vect a (Succ n)`,
/// with its recursor derived at the declaration.
fn with_vect(ctx: &mut Context) {
    ctx.set_auto_auxiliaries(true);
    ctx.add_indexed_inductive("Vect", term("forall (a : Type), Nat -> Type"), 1);
    ctx.add_constructor("VNil", "Vect", term("forall (a : Type), Vect a Zero"));
    ctx.add_constructor(
        "VCons",
        "Vect",
        term("forall (a : Type), forall (n : Nat), a -> Vect a n -> Vect a (Succ n)"),
    );
}

#[test]
fn induction_over_an_indexed_family_generalizes_its_index() {
    let mut ctx = std_ctx();
    with_vect(&mut ctx);
    // About the index, not the vector: every `Vect A n` has `n` zero or a successor.
    let statement = term(
        "forall (A : Type), forall (n : Nat), forall (v : Vect A n), \
         Or (Eq Nat n Zero) (Ex Nat (fun (m : Nat) => Eq Nat n (Succ m)))",
    );
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    for x in ["A", "n", "v"] {
        st.intro(x).unwrap();
    }
    st.induction("v", &[&[], &["k", "x", "w", "ih"]])
        .expect("`v : Vect A n`, its index `n` a hypothesis");
    assert_eq!(
        st.goals()[0].target,
        st.read(&term(
            "Or (Eq Nat Zero Zero) (Ex Nat (fun (m : Nat) => Eq Nat Zero (Succ m)))"
        )),
        "`VNil` is at index `Zero`"
    );
    assert!(
        st.goals()[0].hyps.iter().all(|(h, _)| h != "n" && h != "v"),
        "the index and the vector are generalized"
    );
    st.left().unwrap();
    st.rfl().unwrap();
    assert_eq!(
        st.goals()[0].target,
        st.read(&term(
            "Or (Eq Nat (Succ k) Zero) (Ex Nat (fun (m : Nat) => Eq Nat (Succ k) (Succ m)))"
        )),
        "`VCons` is at index `Succ k`"
    );
    st.right().unwrap();
    st.exists(&term("k")).unwrap();
    st.rfl().unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn induction_refuses_an_index_that_is_not_a_hypothesis_of_its_own() {
    let mut ctx = std_ctx();
    with_vect(&mut ctx);
    let mut st = GoalState::new(
        &ctx,
        &term("forall (A : Type), forall (v : Vect A (Succ Zero)), True"),
    )
    .unwrap();
    st.intro("A").unwrap();
    st.intro("v").unwrap();
    let err = st.induction("v", &[]).unwrap_err().to_string();
    assert!(
        err.contains("Succ Zero"),
        "the index that cannot be generalized is named: {err}"
    );
}

#[test]
fn cases_closes_a_goal_no_constructor_can_reach() {
    // `refl`'s index is its parameter: an `Eq Nat (Succ a) Zero` has no constructor at all, so the
    // match owes no case, as Lean's `noConfusion` closes it.
    let ctx = std_ctx();
    let statement = term("forall (a : Nat), Eq Nat (Succ a) Zero -> False");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    st.intro("a").unwrap();
    st.intro("h").unwrap();
    st.cases("h", &[])
        .expect("no constructor of `Eq` produces `Eq Nat (Succ a) Zero`");
    assert!(st.goals().is_empty(), "nothing is owed");
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn cases_splits_a_value_into_its_constructors() {
    let ctx = std_ctx();
    let statement = term("forall (b : Bool), Or (Eq Bool b true) (Eq Bool b false)");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    st.intro("b").unwrap();
    st.cases("b", &[]).unwrap();
    let targets: Vec<Term> = st.goals().into_iter().map(|g| g.target).collect();
    assert_eq!(
        targets,
        vec![
            term("Or (Eq Bool true true) (Eq Bool true false)"),
            term("Or (Eq Bool false true) (Eq Bool false false)"),
        ],
        "one goal per constructor, `b` replaced by it"
    );
    st.left().unwrap();
    st.rfl().unwrap();
    st.right().unwrap();
    st.rfl().unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn cases_names_a_constructors_fields_and_carries_the_dependents() {
    // `h` depends on `n`; splitting `n` carries `h` into each case at the constructor.
    let ctx = std_ctx();
    let statement = term("forall (n : Nat), Eq Nat n n -> Or (Eq Nat n Zero) (Ex Nat (fun (m : Nat) => Eq Nat n (Succ m)))");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    st.intro("n").unwrap();
    st.intro("h").unwrap();
    st.cases("n", &[&[], &["k"]]).unwrap();
    assert!(
        st.goals()[0]
            .hyps
            .contains(&("h".to_string(), term("Eq Nat Zero Zero"))),
        "{:?}",
        st.goals()[0].hyps
    );
    st.left().unwrap();
    st.rfl().unwrap();
    assert!(st.goals()[0]
        .hyps
        .contains(&("h".to_string(), st.read(&term("Eq Nat (Succ k) (Succ k)")))));
    st.right().unwrap();
    st.exists(&term("k")).unwrap();
    st.rfl().unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);
}

/// `Leq (n : Nat) : Nat → Prop := leq_n : Leq n n | leq_S : Π(m : Nat). Leq n m → Leq n (Succ m)`.
fn with_leq(ctx: &mut Context) {
    ctx.add_indexed_inductive("Leq", term("forall (n : Nat), Nat -> Prop"), 1);
    ctx.add_constructor("leq_n", "Leq", term("forall (n : Nat), Leq n n"));
    ctx.add_constructor(
        "leq_S",
        "Leq",
        term("forall (n : Nat), forall (m : Nat), Leq n m -> Leq n (Succ m)"),
    );
}

#[test]
fn cases_solves_the_index_equations_of_a_value_whose_index_is_a_term() {
    // `v : Vect A (Succ k)`: `VNil`'s index `Zero` clashes, so one case; `VCons`'s index `Succ n`
    // meets `Succ k`, whose injection makes `n` be `k`, so the tail is a `Vect A k`.
    let mut ctx = std_ctx();
    with_vect(&mut ctx);
    let statement = term("forall (A : Type), forall (k : Nat), forall (v : Vect A (Succ k)), Ex A (fun (a : A) => True)");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    for x in ["A", "k", "v"] {
        st.intro(x).unwrap();
    }
    st.cases("v", &[&["n", "a", "w"]])
        .expect("the index equations are solved");
    let goals = st.goals();
    assert_eq!(goals.len(), 1, "`VNil` cannot produce `Vect A (Succ k)`");
    assert!(
        goals[0]
            .hyps
            .contains(&("w".to_string(), st.read(&term("Vect A k")))),
        "the tail is at `k`: {:?}",
        goals[0].hyps
    );
    assert!(
        goals[0].hyps.iter().all(|(h, _)| h != "n"),
        "`n` was solved as `k`: {:?}",
        goals[0].hyps
    );
    st.exists(&term("a")).unwrap();
    st.constructor().unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn cases_carries_a_goal_that_mentions_the_value_through_its_index_equations() {
    let mut ctx = std_ctx();
    with_vect(&mut ctx);
    let statement = term(
        "forall (A : Type), forall (k : Nat), forall (v : Vect A (Succ k)), \
         Ex A (fun (a : A) => Ex (Vect A k) (fun (w : Vect A k) => Eq (Vect A (Succ k)) v (VCons A k a w)))",
    );
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    for x in ["A", "k", "v"] {
        st.intro(x).unwrap();
    }
    st.cases("v", &[&["n", "x", "t"]]).unwrap();
    assert_eq!(
        st.goals()[0].target,
        st.read(&term("Ex A (fun (a : A) => Ex (Vect A k) (fun (w : Vect A k) => Eq (Vect A (Succ k)) (VCons A k x t) (VCons A k a w)))")),
        "the value is the constructor, its index solved"
    );
    st.exists(&term("x")).unwrap();
    st.exists(&term("t")).unwrap();
    st.rfl().unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn cases_inverts_an_inductive_relation() {
    // `Leq n (Succ m)` came from `leq_n` (so `n` is `Succ m`) or from `leq_S` (so `Leq n m`).
    let mut ctx = std_ctx();
    with_leq(&mut ctx);
    let statement = term(
        "forall (n : Nat), forall (m : Nat), Leq n (Succ m) -> Or (Eq Nat n (Succ m)) (Leq n m)",
    );
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    for x in ["n", "m", "h"] {
        st.intro(x).unwrap();
    }
    st.cases("h", &[&[], &["m1", "h1"]]).unwrap();
    assert_eq!(st.goals().len(), 2);
    st.left().unwrap();
    st.rfl().unwrap();
    assert!(
        st.goals()[0]
            .hyps
            .contains(&("h1".to_string(), st.read(&term("Leq n m")))),
        "{:?}",
        st.goals()[0].hyps
    );
    st.right().unwrap();
    st.exact(&term("h1")).unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);
}

/// `HV (A : Type) (a : A) : Type 1 := hv : Π(A : Type). Π(a : A). HV A a`: the second index's type
/// is the first index.
fn with_hv(ctx: &mut Context) {
    ctx.add_indexed_inductive("HV", term("forall (A : Type), A -> Type1"), 0);
    ctx.add_constructor(
        "hv",
        "HV",
        term("forall (A : Type), forall (a : A), HV A a"),
    );
}

#[test]
fn cases_refuses_index_equations_that_would_be_heterogeneous() {
    let mut ctx = std_ctx();
    with_hv(&mut ctx);
    let mut st = GoalState::new(&ctx, &term("forall (h : HV Nat Zero), True")).unwrap();
    st.intro("h").unwrap();
    let err = st.cases("h", &[]).unwrap_err().to_string();
    assert!(
        err.contains("heterogeneous"),
        "the second index's type depends on the first: {err}"
    );
    assert_eq!(
        st.goals().len(),
        1,
        "a refused tactic leaves the goal as it was"
    );
}

#[test]
fn subst_replaces_a_hypothesis_by_what_an_equation_makes_it() {
    let ctx = std_ctx();
    let statement = term("forall (a : Nat), forall (b : Nat), Eq Nat a b -> Eq Nat b a");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    for x in ["a", "b", "h"] {
        st.intro(x).unwrap();
    }
    st.subst("h")
        .expect("`b` is a hypothesis the equation determines");
    let goals = st.goals();
    assert!(
        goals[0].hyps.iter().all(|(n, _)| n != "b" && n != "h"),
        "{:?}",
        goals[0].hyps
    );
    assert_eq!(goals[0].target, st.read(&term("Eq Nat a a")));
    st.rfl().unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);

    let statement = term(
        "forall (a : Nat), forall (b : Nat), Eq Nat b (Succ a) -> Eq Nat (Succ b) (Succ (Succ a))",
    );
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    for x in ["a", "b", "h"] {
        st.intro(x).unwrap();
    }
    st.subst("h")
        .expect("a hypothesis on the left is substituted too");
    assert_eq!(
        st.goals()[0].target,
        st.read(&term("Eq Nat (Succ (Succ a)) (Succ (Succ a))"))
    );
    st.rfl().unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn subst_refuses_an_equation_between_two_terms() {
    let ctx = std_ctx();
    let mut st = GoalState::new(
        &ctx,
        &term("forall (a : Nat), Eq Nat (Succ a) (Succ Zero) -> True"),
    )
    .unwrap();
    st.intro("a").unwrap();
    st.intro("h").unwrap();
    let err = st.subst("h").unwrap_err().to_string();
    assert!(
        err.contains("hypothesis"),
        "neither side is a hypothesis to replace: {err}"
    );
}

#[test]
fn injection_states_the_equations_of_a_constructors_fields() {
    let ctx = std_ctx();
    let statement =
        term("forall (a : Nat), forall (b : Nat), Eq Nat (Succ a) (Succ b) -> Eq Nat a b");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    for x in ["a", "b", "h"] {
        st.intro(x).unwrap();
    }
    st.injection("h", &["e"]).expect("`Succ` is injective");
    assert!(
        st.goals()[0]
            .hyps
            .contains(&("e".to_string(), st.read(&term("Eq Nat a b")))),
        "{:?}",
        st.goals()[0].hyps
    );
    st.exact(&term("e")).unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn injection_closes_a_goal_whose_equation_has_two_constructors() {
    let ctx = std_ctx();
    let statement = term("forall (a : Nat), Eq Nat (Succ a) Zero -> Eq Nat a (Succ a)");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    st.intro("a").unwrap();
    st.intro("h").unwrap();
    st.injection("h", &[])
        .expect("`Succ a` and `Zero` are never equal");
    assert!(st.goals().is_empty());
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn refine_leaves_each_hole_unification_cannot_fill_as_a_goal() {
    let ctx = std_ctx();
    let statement = term("forall (P : Prop), forall (Q : Prop), P -> Q -> And P Q");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    for x in ["P", "Q", "p", "q"] {
        st.intro(x).unwrap();
    }
    st.refine(&term("conj P Q _ _"))
        .expect("`conj P Q : P -> Q -> And P Q`");
    let targets: Vec<Term> = st.goals().into_iter().map(|g| g.target).collect();
    assert_eq!(
        targets,
        vec![st.read(&term("P")), st.read(&term("Q"))],
        "one goal per hole, in order"
    );
    st.exact(&term("p")).unwrap();
    st.exact(&term("q")).unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn refine_scopes_a_hole_under_the_binders_around_it() {
    let ctx = std_ctx();
    let statement = term("forall (P : Prop), P -> P");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    st.intro("P").unwrap();
    st.refine(&term("fun (p : P) => _")).unwrap();
    let goals = st.goals();
    assert!(
        goals[0].hyps.iter().any(|(n, _)| n == "p"),
        "the hole sees the binder `p`: {:?}",
        goals[0].hyps
    );
    assert_eq!(goals[0].target, st.read(&term("P")));
    st.exact(&term("p")).unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn refine_refuses_a_term_of_another_type_naming_the_goal() {
    let ctx = std_ctx();
    let mut st = GoalState::new(&ctx, &term("Eq Nat Zero Zero")).unwrap();
    let err = st
        .refine(&term("refl Nat (Succ _)"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("Eq Nat Zero Zero"), "{err}");
    assert_eq!(
        st.goals().len(),
        1,
        "a refused tactic leaves the goal as it was"
    );
}

#[test]
fn a_script_runs_step_by_step_to_a_proof() {
    let ctx = std_ctx();
    let statement = term("forall (A : Type), forall (x : A), Eq A x x");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    st.run(&[
        TacticStep::Intro(vec!["A".into(), "x".into()]),
        TacticStep::Exact(term("refl A x")),
    ])
    .expect("the script proves the statement");
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn a_failing_step_is_named_and_leaves_the_state_as_it_found_it() {
    let ctx = std_ctx();
    let mut st = GoalState::new(&ctx, &term("forall (A : Type), Eq Type A A")).unwrap();
    let before = st.goals();
    // `intro A h`: the first introduction applies, the second cannot (`Eq Type A A` is no Π).
    let err = st
        .run(&[
            TacticStep::Intro(vec!["A".into(), "h".into()]),
            TacticStep::Rfl,
        ])
        .expect_err("the second name has nothing to introduce");
    assert_eq!(err.step, 0, "the first step failed");
    assert!(err.to_string().contains("step 1 (`intro A h`)"), "{err}");
    assert_eq!(st.goals(), before, "the half-applied `intro A h` is undone");
}

// ---- the vernacular: `Theorem name : P := by tac; tac.` ----

use logicaffeine_kernel::interface::{InterfaceError, Repl};

#[test]
fn a_theorem_proved_by_a_script_is_published_and_checked() {
    let mut repl = Repl::new();
    repl.execute("Theorem refl_any : forall (A : Type), forall (x : A), Eq A x x := by intro A x; exact (refl A x).")
        .expect("the script proves the statement and the kernel checks its proof");
    let shown = repl
        .execute("Check refl_any.")
        .expect("the theorem is published");
    assert!(shown.contains("Eq"), "{shown}");
    repl.execute("Theorem use_it : Eq Nat Zero Zero := by exact (refl_any Nat Zero).")
        .expect("a later script cites it");
}

#[test]
fn a_script_that_leaves_a_goal_open_is_refused_and_publishes_nothing() {
    let mut repl = Repl::new();
    match repl.execute("Theorem half : forall (P : Prop), P -> P := by intro P.") {
        Err(InterfaceError::Tactic(e)) => {
            assert!(e.tactic.is_none(), "no step is at fault: {e}");
            assert!(e.to_string().contains("after the last step"), "{e}");
        }
        other => panic!("an unfinished script must be refused: {other:?}"),
    }
    assert!(
        repl.execute("Check half.").is_err(),
        "nothing was published"
    );
}

#[test]
fn a_scripts_failing_step_is_named_with_its_goal() {
    let mut repl = Repl::new();
    match repl.execute("Theorem wrong : forall (P : Prop), P -> P := by intro P h; exact P.") {
        Err(InterfaceError::Tactic(e)) => {
            assert_eq!(e.step, 1, "the second step, `exact P`, fails");
            let shown = e.to_string();
            assert!(
                shown.contains("step 2") && shown.contains("exact"),
                "{shown}"
            );
        }
        other => panic!("`exact P` proves nothing about `P`: {other:?}"),
    }
}

#[test]
fn a_script_runs_induction_and_rewriting() {
    let mut repl = Repl::new();
    repl.execute(
        "Definition plus : Nat -> Nat -> Nat := fix plus => fun (n : Nat) => fun (m : Nat) => \
         match n return (fun (x : Nat) => Nat) with m | fun (k : Nat) => Succ (plus k m) end.",
    )
    .expect("`plus` is a structural recursion");
    repl.execute(
        "Theorem plus_zero : forall (n : Nat), Eq Nat (plus n Zero) n := by \
         intro n; induction n as [ | k ih]; rfl; change Eq Nat (Succ (plus k Zero)) (Succ k); rw [ih]; rfl.",
    )
    .expect("induction, change, rewrite and rfl, as a script");
}

#[test]
fn a_scripted_theorem_prints_back_to_itself() {
    use logicaffeine_kernel::interface::{parse_command, print_command, Command};
    let written = "Theorem t : forall (P : Prop), P -> And P P := by intro P h; constructor; exact h; assumption.";
    let command = parse_command(written).expect("the script parses");
    let Command::TheoremBy { script, .. } = &command else {
        panic!("a `by` proof is a script: {command:?}")
    };
    assert_eq!(script.len(), 4, "intro, constructor, exact, assumption");
    let printed = print_command(&command).expect("a script prints");
    let reread =
        parse_command(&printed).unwrap_or_else(|e| panic!("`{printed}` reads back: {e:?}"));
    let Command::TheoremBy { script: again, .. } = reread else {
        panic!("{printed}")
    };
    assert_eq!(&again, script, "`{printed}` is the same script");
}

#[test]
fn a_script_inverts_injects_substitutes_and_refines() {
    let mut repl = Repl::new();
    repl.execute("Inductive Leq (n : Nat) : Nat -> Prop := leq_n : Leq n n | leq_S : (m : Nat) -> Leq n m -> Leq n (Succ m).")
        .expect("an inductive relation");
    for (theorem, what) in [
        (
            "Theorem leq_succ_inv : forall (n : Nat), forall (m : Nat), Leq n (Succ m) -> Or (Eq Nat n (Succ m)) (Leq n m) := \
             by intro n m h; cases h as [ | m1 h1]; left; rfl; right; exact h1.",
            "inversion by index equations",
        ),
        (
            "Theorem succ_inj_by_injection : forall (a : Nat), forall (b : Nat), Eq Nat (Succ a) (Succ b) -> Eq Nat a b := \
             by intro a b h; injection h as [e]; exact e.",
            "injection",
        ),
        (
            "Theorem sym_by_subst : forall (a : Nat), forall (b : Nat), Eq Nat a b -> Eq Nat b a := by intro a b h; subst h; rfl.",
            "substitution",
        ),
        (
            "Theorem pair_up : forall (P : Prop), forall (Q : Prop), P -> Q -> And P Q := \
             by intro P Q p q; refine (conj P Q _ _); exact p; exact q.",
            "refinement",
        ),
    ] {
        repl.execute(theorem).unwrap_or_else(|e| panic!("{what}: {e}"));
    }
}

#[test]
fn the_new_tactics_print_back_to_themselves() {
    use logicaffeine_kernel::interface::{parse_command, print_command, Command};
    let written = "Theorem t : forall (a : Nat), forall (b : Nat), Eq Nat (Succ a) (Succ b) -> Eq Nat b a := \
                   by intro a b h; injection h as [e]; subst e; refine (refl Nat _).";
    let command = parse_command(written).expect("the script parses");
    let Command::TheoremBy { script, .. } = &command else {
        panic!("a `by` proof is a script: {command:?}")
    };
    assert_eq!(script.len(), 4, "intro, injection, subst, refine");
    let printed = print_command(&command).expect("a script prints");
    let reread =
        parse_command(&printed).unwrap_or_else(|e| panic!("`{printed}` reads back: {e:?}"));
    let Command::TheoremBy { script: again, .. } = reread else {
        panic!("{printed}")
    };
    assert_eq!(&again, script, "`{printed}` is the same script");
}

#[test]
fn calc_chains_equations_and_leaves_a_step_proved_by_a_hole_as_a_goal() {
    let ctx = std_ctx();
    let statement = term("forall (a : Nat), forall (b : Nat), forall (c : Nat), Eq Nat a b -> Eq Nat b c -> Eq Nat a c");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    for x in ["a", "b", "c", "h1", "h2"] {
        st.intro(x).unwrap();
    }
    st.calc(&[
        (term("Eq Nat a b"), term("h1")),
        (term("Eq Nat _ c"), term("_")),
    ])
    .expect("`a = b`, then `b = c`, proves `a = c`");
    let goals = st.goals();
    assert_eq!(goals.len(), 1, "the step proved by `_` is the one goal");
    assert_eq!(goals[0].target, st.read(&term("Eq Nat b c")));
    st.exact(&term("h2")).unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn calc_refuses_steps_that_do_not_meet() {
    let ctx = std_ctx();
    let mut st = GoalState::new(
        &ctx,
        &term("forall (a : Nat), forall (b : Nat), forall (c : Nat), Eq Nat a b -> Eq Nat a c"),
    )
    .unwrap();
    for x in ["a", "b", "c", "h"] {
        st.intro(x).unwrap();
    }
    let err = st
        .calc(&[
            (term("Eq Nat a b"), term("h")),
            (term("Eq Nat c c"), term("refl Nat c")),
        ])
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("step 2") && err.contains("starts at c"),
        "{err}"
    );
    assert_eq!(
        st.goals().len(),
        1,
        "a refused tactic leaves the goal as it was"
    );
}

#[test]
fn calc_refuses_a_conclusion_that_is_not_the_goal() {
    let ctx = std_ctx();
    let mut st = GoalState::new(
        &ctx,
        &term("forall (a : Nat), forall (b : Nat), Eq Nat a b -> Eq Nat b a"),
    )
    .unwrap();
    for x in ["a", "b", "h"] {
        st.intro(x).unwrap();
    }
    let err = st
        .calc(&[(term("Eq Nat a b"), term("h"))])
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("Eq Nat a b") && err.contains("Eq Nat b a"),
        "the conclusion and the goal: {err}"
    );
}

#[test]
fn a_calculation_reads_and_prints_in_the_vernacular() {
    use logicaffeine_kernel::interface::{parse_command, print_command, Command};
    let mut repl = Repl::new();
    repl.execute(
        "Theorem trans3 : forall (a : Nat), forall (b : Nat), forall (c : Nat), forall (d : Nat), \
         Eq Nat a b -> Eq Nat b c -> Eq Nat c d -> Eq Nat a d := \
         by intro a b c d h1 h2 h3; calc [Eq Nat a b := h1, Eq Nat _ c := h2, Eq Nat _ d := _]; exact h3.",
    )
    .expect("a three-step calculation, its last step a goal");
    let command = parse_command(
        "Theorem t : forall (a : Nat), Eq Nat a a := by intro a; calc [Eq Nat a a := refl Nat a].",
    )
    .unwrap();
    let printed = print_command(&command).expect("prints");
    let Command::TheoremBy { script, .. } =
        parse_command(&printed).unwrap_or_else(|e| panic!("`{printed}` reads back: {e:?}"))
    else {
        panic!("{printed}")
    };
    let Command::TheoremBy {
        script: original, ..
    } = command
    else {
        unreachable!()
    };
    assert_eq!(script, original, "{printed}");
}

/// `f : Nat -> Nat` and `g : Nat -> Nat -> Nat`, opaque, for congruence.
fn with_functions(ctx: &mut Context) {
    ctx.add_declaration("f", term("Nat -> Nat"));
    ctx.add_declaration("g", term("Nat -> Nat -> Nat"));
}

#[test]
fn cc_closes_an_equation_the_hypotheses_entail_by_congruence() {
    let mut ctx = std_ctx();
    with_functions(&mut ctx);
    let statement = term(
        "forall (a : Nat), forall (b : Nat), forall (c : Nat), \
         Eq Nat a b -> Eq Nat c b -> Eq Nat (g (f a) c) (g (f c) a)",
    );
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    for x in ["a", "b", "c", "h1", "h2"] {
        st.intro(x).unwrap();
    }
    st.cc()
        .expect("a = b = c, so f a = f c and g (f a) c = g (f c) a");
    assert!(st.goals().is_empty());
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn cc_uses_an_equation_in_either_direction_and_closes_reflexive_goals() {
    let mut ctx = std_ctx();
    with_functions(&mut ctx);
    let statement = term("forall (a : Nat), forall (b : Nat), Eq Nat (f a) b -> Eq Nat b (f a)");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    for x in ["a", "b", "h"] {
        st.intro(x).unwrap();
    }
    st.cc().unwrap();
    proves(&ctx, &st.qed().unwrap(), &statement);
    let mut st = GoalState::new(&ctx, &term("forall (a : Nat), Eq Nat (f a) (f a)")).unwrap();
    st.intro("a").unwrap();
    st.cc()
        .expect("reflexivity is congruence with nothing to merge");
}

#[test]
fn cc_refuses_an_equation_the_hypotheses_do_not_entail() {
    let mut ctx = std_ctx();
    with_functions(&mut ctx);
    let mut st = GoalState::new(
        &ctx,
        &term("forall (a : Nat), forall (b : Nat), Eq Nat (f a) (f b) -> Eq Nat a b"),
    )
    .unwrap();
    for x in ["a", "b", "h"] {
        st.intro(x).unwrap();
    }
    let err = st.cc().unwrap_err().to_string();
    assert!(err.contains("not entail"), "f is not injective: {err}");
    assert_eq!(
        st.goals().len(),
        1,
        "a refused tactic leaves the goal as it was"
    );
}

#[test]
fn a_script_closes_by_congruence() {
    let mut repl = Repl::new();
    repl.execute("Axiom h1f : Nat -> Nat.").unwrap();
    repl.execute(
        "Theorem cc_chain : forall (a : Nat), forall (b : Nat), forall (c : Nat), \
         Eq Nat a b -> Eq Nat b c -> Eq Nat (h1f a) (h1f c) := by intro a b c p q; cc.",
    )
    .expect("congruence closure, as a script step");
}

#[test]
fn decide_proves_a_decidable_proposition_by_computing_its_instance() {
    let ctx = std_ctx();
    for statement in [
        "Eq Nat (Succ (Succ Zero)) (Succ (Succ Zero))",
        "Not (Eq Nat (Succ Zero) (Succ (Succ Zero)))",
    ] {
        let statement = term(statement);
        let mut st = GoalState::new(&ctx, &statement).unwrap();
        st.decide().unwrap_or_else(|e| panic!("{statement}: {e}"));
        assert!(st.goals().is_empty());
        proves(&ctx, &st.qed().unwrap(), &statement);
    }
}

#[test]
fn decide_refuses_a_proposition_its_instance_decides_false() {
    let ctx = std_ctx();
    let mut st = GoalState::new(&ctx, &term("Eq Nat (Succ Zero) (Succ (Succ Zero))")).unwrap();
    let err = st.decide().unwrap_err().to_string();
    assert!(
        err.contains("false"),
        "the instance decides it false: {err}"
    );
    assert_eq!(
        st.goals().len(),
        1,
        "a refused tactic leaves the goal as it was"
    );
}

#[test]
fn decide_refuses_a_proposition_no_instance_decides() {
    let ctx = std_ctx();
    let mut st = GoalState::new(&ctx, &term("forall (P : Prop), P -> P")).unwrap();
    let err = st.decide().unwrap_err().to_string();
    assert!(err.contains("Decidable"), "no instance: {err}");
}

#[test]
fn a_script_decides() {
    let mut repl = Repl::new();
    repl.execute("Theorem two_ne_one : Not (Eq Nat (Succ (Succ Zero)) (Succ Zero)) := by decide.")
        .expect("decided by computation, checked by the kernel");
}

// ---- simp: rewriting by lemmas, each rewrite a transport, the lemmas cited ----

fn with_idempotent(ctx: &mut Context) {
    ctx.add_declaration("f", term("Nat -> Nat"));
    ctx.add_declaration("f_idem", term("forall (n : Nat), Eq Nat (f (f n)) (f n)"));
}

fn mentions(t: &Term, name: &str) -> bool {
    match t {
        Term::Global(n) | Term::Const { name: n, .. } => n == name,
        Term::App(a, b) => mentions(a, name) || mentions(b, name),
        Term::Lambda {
            param_type, body, ..
        } => mentions(param_type, name) || mentions(body, name),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => mentions(param_type, name) || mentions(body_type, name),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            mentions(discriminant, name)
                || mentions(motive, name)
                || cases.iter().any(|c| mentions(c, name))
        }
        _ => false,
    }
}

#[test]
fn simp_rewrites_by_a_hypothesis_until_no_instance_is_left_and_closes_by_rfl() {
    let mut ctx = std_ctx();
    with_idempotent(&mut ctx);
    let statement =
        term("forall (h : forall (n : Nat), Eq Nat (f n) n), forall (a : Nat), Eq Nat (f (f a)) a");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    st.intro("h").unwrap();
    st.intro("a").unwrap();
    st.simp(&[term("h")])
        .expect("f (f a) rewrites to f a, then to a");
    assert!(
        st.goals().is_empty(),
        "the sides agree after the rewrites, so rfl closes the goal"
    );
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn simp_instantiates_a_lemma_by_matching_and_its_proof_cites_the_lemma() {
    let mut ctx = std_ctx();
    with_idempotent(&mut ctx);
    let statement = term("forall (a : Nat), Eq Nat (f (f (f a))) (f a)");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    st.intro("a").unwrap();
    st.simp(&[term("f_idem")])
        .expect("f_idem at n := f a, then at n := a");
    assert!(st.goals().is_empty());
    let proof = st.qed().unwrap();
    proves(&ctx, &proof, &statement);
    assert!(
        mentions(&proof, "f_idem"),
        "the proof is the chain of transports along f_idem: {proof}"
    );
}

#[test]
fn simp_with_no_lemma_rewrites_by_the_contexts_simp_set() {
    let mut ctx = std_ctx();
    with_idempotent(&mut ctx);
    ctx.add_simp_lemma("f_idem")
        .expect("an equation with a pattern on its left");
    let statement = term("forall (a : Nat), Eq Nat (f (f a)) (f a)");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    st.intro("a").unwrap();
    st.simp(&[]).expect("the @[simp] set holds f_idem");
    assert!(st.goals().is_empty());
    let proof = st.qed().unwrap();
    proves(&ctx, &proof, &statement);
    assert!(
        mentions(&proof, "f_idem"),
        "cited, not assumed by conversion: {proof}"
    );
}

#[test]
fn simp_instantiates_every_variable_of_a_lemma_at_its_own_position() {
    let mut ctx = std_ctx();
    ctx.add_declaration("g", term("Nat -> Nat -> Nat"));
    ctx.add_declaration(
        "first",
        term("forall (a : Nat), forall (b : Nat), Eq Nat (g a b) a"),
    );
    let statement = term("forall (x : Nat), forall (y : Nat), Eq Nat (g (g x y) y) x");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    st.intro("x").unwrap();
    st.intro("y").unwrap();
    st.simp(&[term("first")])
        .expect("first at a := g x y, b := y, then at a := x, b := y");
    assert!(st.goals().is_empty());
    proves(&ctx, &st.qed().unwrap(), &statement);
}

#[test]
fn simp_leaves_a_goal_it_simplified_but_cannot_close() {
    let mut ctx = std_ctx();
    with_idempotent(&mut ctx);
    ctx.add_declaration("g", term("Nat -> Nat"));
    let statement = term("forall (a : Nat), Eq Nat (f (f a)) (g a)");
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    st.intro("a").unwrap();
    st.simp(&[term("f_idem")]).expect("one rewrite applies");
    let goals = st.goals();
    assert_eq!(goals.len(), 1, "f a and g a do not agree");
    assert_eq!(goals[0].target, st.read(&term("Eq Nat (f a) (g a)")));
}

#[test]
fn simp_refuses_when_no_lemma_rewrites_the_goal_and_leaves_it() {
    let mut ctx = std_ctx();
    with_idempotent(&mut ctx);
    let mut st = GoalState::new(&ctx, &term("forall (a : Nat), Eq Nat (f a) (f a)")).unwrap();
    st.intro("a").unwrap();
    let before = st.goals();
    let err = st.simp(&[term("f_idem")]).unwrap_err().to_string();
    assert!(err.contains("no lemma rewrites"), "{err}");
    assert_eq!(
        st.goals(),
        before,
        "a refused simp leaves the goal as it was"
    );
}

#[test]
fn simp_refuses_a_rewrite_that_does_not_stop_and_leaves_the_goal() {
    let mut ctx = std_ctx();
    with_idempotent(&mut ctx);
    let statement = term(
        "forall (h : forall (n : Nat), Eq Nat (f n) (f (f n))), forall (a : Nat), Eq Nat (f a) a",
    );
    let mut st = GoalState::new(&ctx, &statement).unwrap();
    st.intro("h").unwrap();
    st.intro("a").unwrap();
    let before = st.goals();
    let err = st.simp(&[term("h")]).unwrap_err().to_string();
    assert!(err.contains("still rewriting"), "{err}");
    assert_eq!(
        st.goals(),
        before,
        "every rewrite of the refused run is undone"
    );
}

#[test]
fn simp_refuses_a_lemma_that_is_not_an_equation() {
    let mut ctx = std_ctx();
    with_idempotent(&mut ctx);
    let mut st = GoalState::new(&ctx, &term("forall (a : Nat), Eq Nat (f (f a)) (f a)")).unwrap();
    st.intro("a").unwrap();
    let err = st.simp(&[term("a")]).unwrap_err().to_string();
    assert!(err.contains("not a rewrite"), "{err}");
}

/// An instance under a binder mentions the bound variable, which is no term of the goal's context:
/// it is not rewritten (a rewrite there needs function extensionality's congruence).
#[test]
fn simp_does_not_rewrite_an_instance_under_the_binder_it_mentions() {
    let mut ctx = std_ctx();
    with_idempotent(&mut ctx);
    let mut st = GoalState::new(
        &ctx,
        &term("Eq (Nat -> Nat) (fun (x : Nat) => f (f x)) (fun (x : Nat) => f x)"),
    )
    .unwrap();
    let err = st.simp(&[term("f_idem")]).unwrap_err().to_string();
    assert!(err.contains("no lemma rewrites"), "{err}");
}

#[test]
fn a_script_simplifies_and_prints_back_as_written() {
    let mut repl = Repl::new();
    repl.execute("Axiom sf : Nat -> Nat.").unwrap();
    repl.execute("Axiom sf_idem : forall (n : Nat), Eq Nat (sf (sf n)) (sf n).")
        .unwrap();
    repl.execute("Theorem sf_thrice : forall (a : Nat), Eq Nat (sf (sf (sf a))) (sf a) := by intro a; simp [sf_idem].")
        .expect("simp as a script step");
    assert_eq!(
        TacticStep::Simp(vec![term("sf_idem")]).to_string(),
        "simp [sf_idem]"
    );
    assert_eq!(TacticStep::Simp(Vec::new()).to_string(), "simp");
}
