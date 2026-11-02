use super::*;
use crate::{ProofExpr, ProofTerm};

fn c(s: &str) -> ProofTerm {
    ProofTerm::Constant(s.to_string())
}
fn v(s: &str) -> ProofTerm {
    ProofTerm::Variable(s.to_string())
}
fn pred(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.to_string(),
        args,
        world: None,
    }
}
fn imp(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(a), Box::new(b))
}
fn or(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(a), Box::new(b))
}
fn and(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(a), Box::new(b))
}
fn not(e: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(e))
}
fn id(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(a, b)
}
fn forall(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll {
        variable: var.to_string(),
        body: Box::new(body),
    }
}
fn exists(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::Exists {
        variable: var.to_string(),
        body: Box::new(body),
    }
}

fn verify(
    premises: &[ProofExpr],
    goal: &ProofExpr,
    tree: DerivationTree,
) -> crate::verify::VerifiedProof {
    crate::verify::check_derivation(premises, goal, tree)
}

// ── PHASE A: prove the emitter's tree SHAPES certify, before any search ──────

/// A propagation step (ModusPonens from a rule's reason) certifies.
#[test]
fn modus_ponens_shape_certifies() {
    let p = pred("P", vec![c("Obj")]);
    let q = pred("Q", vec![c("Obj")]);
    let premises = vec![p.clone(), imp(p.clone(), q.clone())];
    let tree = modus_ponens(q.clone(), leaf(imp(p.clone(), q.clone())), leaf(p));
    let r = verify(&premises, &q, tree);
    assert!(
        r.verified,
        "ModusPonens shape must certify; err: {:?}",
        r.verification_error
    );
}

/// A unit-clause propagation (disjunctive syllogism from the clause's reason) certifies.
#[test]
fn disjunction_elim_shape_certifies() {
    let p = pred("P", vec![c("Obj")]);
    let q = pred("Q", vec![c("Obj")]);
    let premises = vec![or(p.clone(), q.clone()), not(p.clone())];
    let tree = disjunction_elim(q.clone(), leaf(or(p.clone(), q.clone())), leaf(not(p)));
    let r = verify(&premises, &q, tree);
    assert!(
        r.verified,
        "DisjunctionElim shape must certify; err: {:?}",
        r.verification_error
    );
}

/// THE real grid shape, hand-built end to end: the 2-value bijection cell
/// `In(Beta, Maine)`, proved by closure (ModusPonens) + a reductio that fires the
/// at-most-one rule and closes the equality contradiction. Every node is a rule
/// the solver's emitter will produce — if this certifies, the emitter is sound.
#[test]
fn two_value_grid_cell_handbuilt_certifies() {
    let trip = |t: ProofTerm| pred("Trip", vec![t]);
    let in_ = |t: ProofTerm, s: ProofTerm| pred("In", vec![t, s]);
    let fl = || c("Florida");
    let me_ = || c("Maine");

    // Grounded premises (what `ground_problem` yields, split into the instances
    // the certifier registers as hypotheses):
    let closure_beta = imp(
        trip(c("Beta")),
        or(in_(c("Beta"), fl()), in_(c("Beta"), me_())),
    );
    // at-most-one instance for the pair (Beta, Alpha):
    //   ((Trip(Beta)∧In(Beta,FL)) ∧ (Trip(Alpha)∧In(Alpha,FL))) → Beta = Alpha
    let amo_beta_alpha = imp(
        and(
            and(trip(c("Beta")), in_(c("Beta"), fl())),
            and(trip(c("Alpha")), in_(c("Alpha"), fl())),
        ),
        id(c("Beta"), c("Alpha")),
    );
    let premises = vec![
        trip(c("Alpha")),
        trip(c("Beta")),
        not(id(c("Alpha"), c("Beta"))),
        closure_beta.clone(),
        amo_beta_alpha.clone(),
        in_(c("Alpha"), fl()),
    ];
    let goal = in_(c("Beta"), me_());

    // ¬In(Beta,FL): assume In(Beta,FL); fire the at-most-one rule to get
    // Beta=Alpha; symmetrise to Alpha=Beta; contradict ¬(Alpha=Beta).
    let ante = conj_intro(
        and(trip(c("Beta")), in_(c("Beta"), fl())),
        and(trip(c("Alpha")), in_(c("Alpha"), fl())),
        conj_intro(
            trip(c("Beta")),
            in_(c("Beta"), fl()),
            leaf(trip(c("Beta"))),
            leaf(in_(c("Beta"), fl())),
        ),
        conj_intro(
            trip(c("Alpha")),
            in_(c("Alpha"), fl()),
            leaf(trip(c("Alpha"))),
            leaf(in_(c("Alpha"), fl())),
        ),
    );
    let beta_eq_alpha = modus_ponens(id(c("Beta"), c("Alpha")), leaf(amo_beta_alpha), ante);
    let alpha_eq_beta = eq_sym(c("Beta"), c("Alpha"), beta_eq_alpha);
    let contra = contradiction(alpha_eq_beta, leaf(not(id(c("Alpha"), c("Beta")))));
    let neg_in_beta_fl = reductio(in_(c("Beta"), fl()), contra);

    // closure: Trip(Beta) → In(Beta,FL)∨In(Beta,ME); MP with Trip(Beta).
    let beta_disj = modus_ponens(
        or(in_(c("Beta"), fl()), in_(c("Beta"), me_())),
        leaf(closure_beta),
        leaf(trip(c("Beta"))),
    );
    // Disjunctive syllogism: (In(Beta,FL)∨In(Beta,ME)), ¬In(Beta,FL) ⊢ In(Beta,ME).
    let tree = disjunction_elim(in_(c("Beta"), me_()), beta_disj, neg_in_beta_fl);

    let r = verify(&premises, &goal, tree);
    assert!(
        r.verified,
        "hand-built 2-value grid cell must certify; err: {:?}",
        r.verification_error
    );
}

// ── PHASE B: the solver PROPAGATES and EMITS the certified tree itself ───────

/// The 2-value bijection, solved end to end by the incremental solver: ground the
/// universal premises, `grid_prove` propagates `¬In(Beta,FL)` (exclusion) then the
/// closure forces `In(Beta,Maine)`, and the emitted derivation kernel-certifies.
#[test]
fn two_value_grid_propagates_and_certifies() {
    let trip = |t: ProofTerm| pred("Trip", vec![t]);
    let in_ = |t: ProofTerm, s: ProofTerm| pred("In", vec![t, s]);
    let fl = || c("Florida");
    let me_ = || c("Maine");
    let closure = forall(
        "x",
        imp(trip(v("x")), or(in_(v("x"), fl()), in_(v("x"), me_()))),
    );
    let exactly_one_fl = forall(
        "x",
        forall(
            "y",
            imp(
                and(
                    and(trip(v("x")), in_(v("x"), fl())),
                    and(trip(v("y")), in_(v("y"), fl())),
                ),
                id(v("x"), v("y")),
            ),
        ),
    );
    let premises = vec![
        trip(c("Alpha")),
        trip(c("Beta")),
        not(id(c("Alpha"), c("Beta"))),
        closure,
        exactly_one_fl,
        in_(c("Alpha"), fl()),
    ];
    let goal = in_(c("Beta"), me_());
    let (gp, gg) = crate::grounding::ground_problem(&premises, &goal);
    let tree = grid_prove(&gp, &gg).expect("2-value cell must be forced by propagation");
    let r = verify(&gp, &gg, tree);
    assert!(
        r.verified,
        "solver-emitted 2-value proof must certify; err: {:?}",
        r.verification_error
    );
}

/// A full-size multi-category grid (4 trips × 3 four-value categories), solved by
/// pure propagation: three states pinned, the fourth (`Delta → Maine`) forced by
/// exclusion over the state closure + at-most-one, with the irrelevant year/friend
/// categories present as noise. The solver emits a certified derivation.
#[test]
fn multi_category_grid_propagates_and_certifies() {
    let trips = ["Alpha", "Beta", "Gamma", "Delta"];
    let neq = |a: &str, b: &str| not(id(c(a), c(b)));
    let mut premises: Vec<ProofExpr> = Vec::new();
    let category = |rel: &str, vals: &[&str], out: &mut Vec<ProofExpr>| {
        for t in trips {
            let mut it = vals.iter().map(|val| pred(rel, vec![c(t), c(val)]));
            let first = it.next().unwrap();
            out.push(it.fold(first, or));
        }
        for val in vals {
            for (i, t) in trips.iter().enumerate() {
                for u in &trips[i + 1..] {
                    out.push(imp(
                        and(pred(rel, vec![c(t), c(val)]), pred(rel, vec![c(u), c(val)])),
                        id(c(t), c(u)),
                    ));
                }
            }
        }
    };
    for (i, t) in trips.iter().enumerate() {
        for u in &trips[i + 1..] {
            premises.push(neq(t, u));
        }
    }
    category("In", &["2001", "2002", "2003", "2004"], &mut premises);
    category("In", &["CT", "FL", "KY", "ME"], &mut premises);
    category("With", &["Bill", "Lillie", "Neal", "Yvonne"], &mut premises);
    premises.push(pred("In", vec![c("Alpha"), c("FL")]));
    premises.push(pred("In", vec![c("Beta"), c("KY")]));
    premises.push(pred("In", vec![c("Gamma"), c("CT")]));
    let goal = pred("In", vec![c("Delta"), c("ME")]);
    let tree = grid_prove(&premises, &goal).expect("Delta→ME must be forced by propagation");
    let r = verify(&premises, &goal, tree);
    assert!(
        r.verified,
        "solver-emitted multi-category proof must certify; err: {:?}",
        r.verification_error
    );
}

// ── PHASE C: DPLL decision search over compound (of-pair) clauses ────────────

/// An of-pair clue ((In(A,FL) ∧ With(B,Neal)) ∨ (In(B,FL) ∧ With(A,Neal))) forces a
/// third trip OUT of Florida: assuming In(C,FL), the at-most-one rules exclude both
/// In(A,FL) and In(B,FL), collapsing the of-pair clause to ⊥. The solver searches
/// and emits a certified case-analysis derivation.
#[test]
fn of_pair_forces_cell_certifies() {
    let in_fl = |t: &str| pred("In", vec![c(t), c("FL")]);
    let amo = |t: &str, u: &str| imp(and(in_fl(t), in_fl(u)), id(c(t), c(u)));
    let of_pair = or(
        and(in_fl("A"), pred("With", vec![c("B"), c("Neal")])),
        and(in_fl("B"), pred("With", vec![c("A"), c("Neal")])),
    );
    let premises = vec![
        of_pair,
        amo("A", "C"),
        amo("B", "C"),
        not(id(c("A"), c("C"))),
        not(id(c("B"), c("C"))),
        not(id(c("A"), c("B"))),
    ];
    let goal = not(in_fl("C"));
    let tree = grid_prove(&premises, &goal).expect("of-pair must force ¬In(C,FL)");
    let r = verify(&premises, &goal, tree);
    assert!(
        r.verified,
        "solver-emitted of-pair proof must certify; err: {:?}",
        r.verification_error
    );
}

/// The compound of-pair shape: `(A ∧ (P ∨ Q)) ∨ (Bad ∧ C)` with `¬Bad`, `P→¬R`,
/// `Q→¬R`. Assuming `R`, the `(Bad ∧ C)` arm collapses on `¬Bad` and the `A∧(P∨Q)`
/// arm case-splits the inner `P ∨ Q`, both arms contradicting `R`. The solver finds
/// the nested case analysis and emits a certified ⊥, discharged by reductio to `¬R`.
#[test]
fn compound_of_pair_resolves_certifies() {
    let a = pred("A", vec![c("Obj")]);
    let p = pred("P", vec![c("Obj")]);
    let q = pred("Q", vec![c("Obj")]);
    let bad = pred("Bad", vec![c("Obj")]);
    let cc = pred("C", vec![c("Obj")]);
    let rr = pred("R", vec![c("Obj")]);
    let of_pair = or(and(a, or(p.clone(), q.clone())), and(bad.clone(), cc));
    let premises = vec![
        of_pair,
        not(bad),
        imp(p, not(rr.clone())),
        imp(q, not(rr.clone())),
    ];
    let goal = not(rr);
    let tree = grid_prove(&premises, &goal).expect("compound of-pair must force ¬R");
    let r = verify(&premises, &goal, tree);
    assert!(
        r.verified,
        "solver-emitted compound of-pair proof must certify; err: {:?}",
        r.verification_error
    );
}

fn count_rule(t: &DerivationTree, rule_dbg: &str) -> usize {
    let here = usize::from(format!("{:?}", t.rule).starts_with(rule_dbg));
    here + t
        .premises
        .iter()
        .map(|c| count_rule(c, rule_dbg))
        .sum::<usize>()
}

/// WAVE 1: a compound (of-pair-shaped) clause `(P∧Q) ∨ (R∧S)` with `¬R` refuting the
/// second disjunct — disjunctive syllogism forces the first, so `P` and `Q` are forced
/// by PROPAGATION (no search). The emitted proof must be LINEAR: zero `DisjunctionCases`.
#[test]
fn compound_clause_unit_propagates_linearly() {
    let o = || c("Obj");
    let p = pred("P", vec![o()]);
    let q = pred("Q", vec![o()]);
    let r = pred("R", vec![o()]);
    let s = pred("S", vec![o()]);
    let clause = or(and(p.clone(), q.clone()), and(r.clone(), s.clone()));
    let premises = vec![clause, not(r)];
    let tree = grid_prove(&premises, &p).expect("P must be forced by compound-clause propagation");
    assert_eq!(
        count_rule(&tree, "DisjunctionCases"),
        0,
        "compound-clause propagation must emit a LINEAR proof (no DisjunctionCases)"
    );
    let res = verify(&premises, &p, tree);
    assert!(
        res.verified,
        "linear compound-clause proof must certify; err: {:?}",
        res.verification_error
    );
}

// ── PHASE E (de-risk): the REAL studio Simon, through prepare_premises ───────

/// The studio `Simon` example (2 trips × 2 categories: year {2003,2004} + state
/// {Florida,Maine}), run through the SAME preparation `ui_bridge::prepare_premises`
/// uses — `at_most_one_lemmas` + sort-aware grounding + `discharge_unary_facts` —
/// then solved and certified by `grid_solver`. The grounded `∃`-existence clauses
/// are skipped by `compile`; closures + at-most-one + the pins force `Beta∈Maine`.
#[test]
fn studio_simon_two_category_certifies() {
    use crate::grounding::{
        at_most_one_lemmas, discharge_unary_facts, domain_constants, ground_sorted, sort_domains,
    };
    let trip = |t: ProofTerm| pred("Trip", vec![t]);
    let in_ = |t: ProofTerm, s: ProofTerm| pred("In", vec![t, s]);
    let exactly_one = |val: &str| {
        let phi = |t: ProofTerm| and(trip(t.clone()), in_(t, c(val)));
        exists(
            "x",
            and(
                phi(v("x")),
                forall("y", imp(phi(v("y")), id(v("y"), v("x")))),
            ),
        )
    };
    let mut premises = vec![
        trip(c("Alpha")),
        trip(c("Beta")),
        not(id(c("Alpha"), c("Beta"))),
        forall(
            "x",
            imp(
                trip(v("x")),
                or(in_(v("x"), c("2003")), in_(v("x"), c("2004"))),
            ),
        ),
        forall(
            "x",
            imp(
                trip(v("x")),
                or(in_(v("x"), c("Florida")), in_(v("x"), c("Maine"))),
            ),
        ),
        exactly_one("2003"),
        exactly_one("Florida"),
        in_(c("Alpha"), c("2003")),
        in_(c("Alpha"), c("Florida")),
    ];
    let goal = in_(c("Beta"), c("Maine"));
    // prepare_premises (tense erasure is a no-op for present-tense "is in").
    premises.extend(at_most_one_lemmas(&premises));
    let mut all = premises.clone();
    all.push(goal.clone());
    let fallback = domain_constants(&all);
    let sorts = sort_domains(&premises);
    let grounded: Vec<ProofExpr> = premises
        .iter()
        .map(|p| ground_sorted(p, &sorts, &fallback))
        .collect();
    let prepared = discharge_unary_facts(&grounded);
    let tree = grid_prove(&prepared, &goal).expect("studio Simon must solve by propagation");
    let r = verify(&prepared, &goal, tree);
    assert!(
        r.verified,
        "studio Simon (2-category) must certify via grid_solver; err: {:?}",
        r.verification_error
    );
}
