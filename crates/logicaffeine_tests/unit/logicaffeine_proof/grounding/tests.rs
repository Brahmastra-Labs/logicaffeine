use super::*;

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

/// Clue 2 ("The Florida trip was the hunting trip") — both `In(_,Florida)` and
/// `Hunt(_)` are singletons (each has an `Exactly one` premise), so the definite
/// description becomes the BICONDITIONAL `In(t,Florida) ↔ Hunt(t)` (two rules).
/// Clue 4 ("The Yvonne trip wasn't in Kentucky") yields only the forward rule
/// `With(t,Yvonne) → ¬In(t,Kentucky)` (the negative property is not a singleton).
#[test]
fn definite_descriptions_become_implications() {
    let trip = |t: ProofTerm| pred("Trip", vec![t]);
    let in_ = |t: ProofTerm, s: &str| pred("In", vec![t, c(s)]);
    let with = |t: ProofTerm, f: &str| pred("With", vec![t, c(f)]);
    let hunt = |t: ProofTerm| pred("Hunt", vec![t]);
    // Exactly-one premises that establish the singletons.
    let exactly_one = |phi: ProofExpr, val: ProofExpr| {
        exists(
            "x",
            ProofExpr::And(
                Box::new(ProofExpr::And(Box::new(trip(v("x"))), Box::new(phi))),
                Box::new(forall(
                    "y",
                    ProofExpr::Implies(
                        Box::new(ProofExpr::And(Box::new(trip(v("y"))), Box::new(val))),
                        Box::new(ProofExpr::Identity(v("y"), v("x"))),
                    ),
                )),
            ),
        )
    };
    // Clue 2: ∃x(((Trip(x)∧In(x,Florida)) ∧ ∀y(...→y=x)) ∧ (Hunt(x)∧Trip(x)))
    let clue2 = exists(
        "x",
        ProofExpr::And(
            Box::new(ProofExpr::And(
                Box::new(ProofExpr::And(
                    Box::new(trip(v("x"))),
                    Box::new(in_(v("x"), "Florida")),
                )),
                Box::new(forall(
                    "y",
                    ProofExpr::Implies(
                        Box::new(ProofExpr::And(
                            Box::new(trip(v("y"))),
                            Box::new(in_(v("y"), "Florida")),
                        )),
                        Box::new(ProofExpr::Identity(v("y"), v("x"))),
                    ),
                )),
            )),
            Box::new(ProofExpr::And(
                Box::new(hunt(v("x"))),
                Box::new(trip(v("x"))),
            )),
        ),
    );
    // Clue 4: ∃x(((Trip(x)∧With(x,Yvonne)) ∧ ∀y(...)) ∧ ¬In(x,Kentucky))
    let clue4 = exists(
        "x",
        ProofExpr::And(
            Box::new(ProofExpr::And(
                Box::new(ProofExpr::And(
                    Box::new(trip(v("x"))),
                    Box::new(with(v("x"), "Yvonne")),
                )),
                Box::new(forall(
                    "y",
                    ProofExpr::Implies(
                        Box::new(ProofExpr::And(
                            Box::new(trip(v("y"))),
                            Box::new(with(v("y"), "Yvonne")),
                        )),
                        Box::new(ProofExpr::Identity(v("y"), v("x"))),
                    ),
                )),
            )),
            Box::new(ProofExpr::Not(Box::new(in_(v("x"), "Kentucky")))),
        ),
    );
    let premises = vec![
        trip(c("Alpha")),
        exactly_one(in_(v("x"), "Florida"), in_(v("y"), "Florida")),
        exactly_one(hunt(v("x")), hunt(v("y"))),
        exactly_one(with(v("x"), "Yvonne"), with(v("y"), "Yvonne")),
        clue2,
        clue4,
    ];
    let imps = definite_property_implications(&premises);
    let fwd = |a: ProofExpr, b: ProofExpr| ProofExpr::ForAll {
        variable: "x".to_string(),
        body: Box::new(ProofExpr::Implies(Box::new(a), Box::new(b))),
    };
    assert!(
        imps.contains(&fwd(in_(v("x"), "Florida"), hunt(v("x")))),
        "clue2 forward In(Florida)→Hunt missing; got: {imps:?}"
    );
    assert!(
        imps.contains(&fwd(hunt(v("x")), in_(v("x"), "Florida"))),
        "clue2 reverse Hunt→In(Florida) missing (biconditional); got: {imps:?}"
    );
    assert!(
        imps.contains(&fwd(
            with(v("x"), "Yvonne"),
            ProofExpr::Not(Box::new(in_(v("x"), "Kentucky")))
        )),
        "clue4 forward With(Yvonne)→¬In(Kentucky) missing; got: {imps:?}"
    );
    // Clue 4 has no reverse (the negative property is not a singleton anchor).
    assert!(
        !imps.iter().any(|e| matches!(e, ProofExpr::ForAll { body, .. }
            if matches!(body.as_ref(), ProofExpr::Implies(a, _) if matches!(a.as_ref(), ProofExpr::Not(_))))),
        "no rule may have a negative antecedent; got: {imps:?}"
    );
}

#[test]
fn grounds_universal_to_conjunction() {
    // ∀x. P(x)  over {a, b}  →  P(a) ∧ P(b)
    let e = forall("x", pred("P", vec![v("x")]));
    let g = ground(&e, &[c("a"), c("b")]);
    let expected = ProofExpr::And(
        Box::new(pred("P", vec![c("a")])),
        Box::new(pred("P", vec![c("b")])),
    );
    assert_eq!(g, expected);
}

#[test]
fn grounds_existential_to_disjunction() {
    // ∃x. P(x)  over {a, b}  →  P(a) ∨ P(b)
    let e = exists("x", pred("P", vec![v("x")]));
    let g = ground(&e, &[c("a"), c("b")]);
    let expected = ProofExpr::Or(
        Box::new(pred("P", vec![c("a")])),
        Box::new(pred("P", vec![c("b")])),
    );
    assert_eq!(g, expected);
}

fn is_quantifier_free(e: &ProofExpr) -> bool {
    match e {
        ProofExpr::ForAll { .. } | ProofExpr::Exists { .. } => false,
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => is_quantifier_free(l) && is_quantifier_free(r),
        ProofExpr::Not(x) | ProofExpr::Temporal { body: x, .. } => is_quantifier_free(x),
        _ => true,
    }
}

#[test]
fn domain_is_the_named_constants() {
    let premises = vec![
        pred("Trip", vec![c("Alpha")]),
        ProofExpr::And(
            Box::new(pred("Trip", vec![c("Beta")])),
            Box::new(pred("In", vec![c("Beta"), c("Maine")])),
        ),
    ];
    let d = domain_constants(&premises);
    assert_eq!(d, vec![c("Alpha"), c("Beta"), c("Maine")]);
}

#[test]
fn grounding_the_exactly_one_form_leaves_no_quantifier() {
    // The shape that hung Z3: ∃x(P(x) ∧ ∀y(P(y) → y = x)).
    let inner = ProofExpr::And(
        Box::new(pred("P", vec![v("x")])),
        Box::new(forall(
            "y",
            ProofExpr::Implies(
                Box::new(pred("P", vec![v("y")])),
                Box::new(ProofExpr::Identity(v("y"), v("x"))),
            ),
        )),
    );
    let e = exists("x", inner);
    let g = ground(&e, &[c("a"), c("b")]);
    assert!(
        is_quantifier_free(&g),
        "grounded exactly-one must be quantifier-free: {g:?}"
    );
}

#[test]
fn kernel_does_grounded_disjunctive_syllogism() {
    // The core step the grounded grid needs: In(Beta,FL) ∨ In(Beta,ME), ¬In(Beta,FL) ⊢ In(Beta,ME).
    let in_ = |t: &str, s: &str| pred("In", vec![c(t), c(s)]);
    let premises = vec![
        ProofExpr::Or(
            Box::new(in_("Beta", "Florida")),
            Box::new(in_("Beta", "Maine")),
        ),
        ProofExpr::Not(Box::new(in_("Beta", "Florida"))),
    ];
    let goal = in_("Beta", "Maine");
    let r = crate::verify::prove_certify_check(&premises, &goal);
    assert!(
        r.verified,
        "grounded disjunctive syllogism; err: {:?}",
        r.verification_error
    );
}

// GROUNDING makes the grid quantifier-free and DECIDABLE, and our kernel then
// CERTIFIES it — no Z3. The disjunctive syllogism needs `¬In(Beta,FL)`, which the
// kernel DERIVES by proof-by-contradiction (assume In(Beta,FL); the grounded
// uniqueness forces Beta=Alpha; ⊥ with ¬Alpha=Beta). This is the certified-kernel
// analogue of the Z3-grounded solve: same grounded premises, our own proof term.
#[test]
fn grounded_two_value_grid_proves_with_our_kernel() {
    // The 2-value bijection that the kernel could NOT do while quantified, now
    // GROUNDED → quantifier-free → our kernel proves it (certified). Florida is
    // taken by Alpha (exactly-one), so Beta — the other trip — is in Maine.
    let trip = |t: ProofTerm| pred("Trip", vec![t]);
    let in_ = |t: ProofTerm, s: ProofTerm| pred("In", vec![t, s]);
    let fl = || c("Florida");
    let me_ = || c("Maine");
    // ∀x(Trip(x) → In(x,FL) ∨ In(x,ME))
    let closure = forall(
        "x",
        ProofExpr::Implies(
            Box::new(trip(v("x"))),
            Box::new(ProofExpr::Or(
                Box::new(in_(v("x"), fl())),
                Box::new(in_(v("x"), me_())),
            )),
        ),
    );
    // PAIRWISE uniqueness (what exactly-one entails): ∀x∀y((Trip(x)∧In(x,FL)) ∧
    // (Trip(y)∧In(y,FL)) → x=y). Grounds to a direct implication per pair.
    let exactly_one_fl = forall(
        "x",
        forall(
            "y",
            ProofExpr::Implies(
                Box::new(ProofExpr::And(
                    Box::new(ProofExpr::And(
                        Box::new(trip(v("x"))),
                        Box::new(in_(v("x"), fl())),
                    )),
                    Box::new(ProofExpr::And(
                        Box::new(trip(v("y"))),
                        Box::new(in_(v("y"), fl())),
                    )),
                )),
                Box::new(ProofExpr::Identity(v("x"), v("y"))),
            ),
        ),
    );
    let premises = vec![
        trip(c("Alpha")),
        trip(c("Beta")),
        ProofExpr::Not(Box::new(ProofExpr::Identity(c("Alpha"), c("Beta")))),
        closure,
        exactly_one_fl,
        in_(c("Alpha"), fl()),
    ];
    let goal = in_(c("Beta"), me_());
    let (gp, gg) = ground_problem(&premises, &goal);
    let r = crate::verify::prove_certify_check(&gp, &gg);
    assert!(
        r.verified,
        "grounded grid must prove via our kernel; err: {:?}",
        r.verification_error
    );
}

// The SAME 2-value bijection, but with the uniqueness premise in the form the
// PARSER actually produces for "Exactly one trip is in Florida":
//   ∃x((Trip(x) ∧ In(x,FL)) ∧ ∀y((Trip(y) ∧ In(y,FL)) → y = x))
// Grounding the ∃ yields a DISJUNCTION (one disjunct per trip), unlike the
// hand-built pairwise ∀x∀y form above which grounds to a conjunction. This pins
// the REAL grid shape: the kernel must case-split on the grounded existence and,
// in the In(Beta,FL) branch, derive Beta=Alpha ⊥ to conclude In(Beta,Maine).
#[test]
fn grounded_exactly_one_existential_grid_proves_with_our_kernel() {
    let trip = |t: ProofTerm| pred("Trip", vec![t]);
    let in_ = |t: ProofTerm, s: ProofTerm| pred("In", vec![t, s]);
    let fl = || c("Florida");
    let me_ = || c("Maine");
    let closure = forall(
        "x",
        ProofExpr::Implies(
            Box::new(trip(v("x"))),
            Box::new(ProofExpr::Or(
                Box::new(in_(v("x"), fl())),
                Box::new(in_(v("x"), me_())),
            )),
        ),
    );
    // φ(t) = Trip(t) ∧ In(t, FL)
    let phi = |t: ProofTerm| ProofExpr::And(Box::new(trip(t.clone())), Box::new(in_(t, fl())));
    let exactly_one_fl = exists(
        "x",
        ProofExpr::And(
            Box::new(phi(v("x"))),
            Box::new(forall(
                "y",
                ProofExpr::Implies(
                    Box::new(phi(v("y"))),
                    Box::new(ProofExpr::Identity(v("y"), v("x"))),
                ),
            )),
        ),
    );
    let premises = vec![
        trip(c("Alpha")),
        trip(c("Beta")),
        ProofExpr::Not(Box::new(ProofExpr::Identity(c("Alpha"), c("Beta")))),
        closure,
        exactly_one_fl,
        in_(c("Alpha"), fl()),
    ];
    let goal = in_(c("Beta"), me_());
    // Grid-prep, exactly as the kernel solve path does it: augment the existence
    // form with its entailed pairwise at-most-one lemma, THEN ground.
    let mut prem = premises;
    prem.extend(at_most_one_lemmas(&prem));
    let (gp, gg) = ground_problem_sorted(&prem, &goal);
    let r = crate::verify::prove_certify_check(&gp, &gg);
    assert!(
        r.verified,
        "grounded exactly-one (∃∀) grid must prove via our kernel; err: {:?}",
        r.verification_error
    );
}

#[test]
/// A FULL-SIZE multi-category grid (4 trips × 3 four-value categories), solved by
/// PROPAGATION — no Z3. Three states are pinned; the fourth trip's state is forced
/// by elimination over the state closure + at-most-one. The YEAR and FRIEND
/// categories are present (closures + at-most-one for every value) but irrelevant
/// to the queried cell — exactly the noise that made the kernel's search EXPLODE
/// before unit propagation. With BCP it must stay fast and certified.
#[test]
fn multi_category_grid_cell_forced_by_propagation() {
    let trips = ["Alpha", "Beta", "Gamma", "Delta"];
    let and = |l: ProofExpr, r: ProofExpr| ProofExpr::And(Box::new(l), Box::new(r));
    let implies = |l: ProofExpr, r: ProofExpr| ProofExpr::Implies(Box::new(l), Box::new(r));
    let neq = |a: &str, b: &str| ProofExpr::Not(Box::new(ProofExpr::Identity(c(a), c(b))));
    // A category: a closure per trip + pairwise at-most-one per value.
    let category = |rel: &str, vals: &[&str], out: &mut Vec<ProofExpr>| {
        for t in trips {
            out.push(fold_disj(
                vals.iter().map(|val| pred(rel, vec![c(t), c(val)])),
            ));
        }
        for val in vals {
            for (i, t) in trips.iter().enumerate() {
                for u in &trips[i + 1..] {
                    out.push(implies(
                        and(pred(rel, vec![c(t), c(val)]), pred(rel, vec![c(u), c(val)])),
                        ProofExpr::Identity(c(t), c(u)),
                    ));
                }
            }
        }
    };
    let mut premises = Vec::new();
    // All trips pairwise distinct.
    for (i, t) in trips.iter().enumerate() {
        for u in &trips[i + 1..] {
            premises.push(neq(t, u));
        }
    }
    category("In", &["2001", "2002", "2003", "2004"], &mut premises); // noise
    category("In", &["CT", "FL", "KY", "ME"], &mut premises); // the queried category
    category("With", &["Bill", "Lillie", "Neal", "Yvonne"], &mut premises); // noise
                                                                            // Pin three states; the fourth (Delta → ME) is forced.
    premises.push(pred("In", vec![c("Alpha"), c("FL")]));
    premises.push(pred("In", vec![c("Beta"), c("KY")]));
    premises.push(pred("In", vec![c("Gamma"), c("CT")]));
    let goal = pred("In", vec![c("Delta"), c("ME")]);
    let r = crate::verify::prove_certify_check(&premises, &goal);
    assert!(
        r.verified,
        "Delta must be forced into ME by propagation (no Z3); err: {:?}",
        r.verification_error
    );
}

/// An OF-PAIR clue ("of A and B, one is in Florida, the other with Neal") forces
/// a third trip OUT of Florida — and only CASE ANALYSIS on the clue can show it:
/// in BOTH arms Florida lands on A or B, so C (distinct, by at-most-one) is not in
/// Florida. This exercises the DPLL decision (`DisjunctionCases`) layered on unit
/// propagation, all kernel-certified, no Z3.
#[test]
fn of_pair_clue_forces_cell_by_case_analysis() {
    let and = |l: ProofExpr, r: ProofExpr| ProofExpr::And(Box::new(l), Box::new(r));
    let implies = |l: ProofExpr, r: ProofExpr| ProofExpr::Implies(Box::new(l), Box::new(r));
    let in_fl = |t: &str| pred("In", vec![c(t), c("FL")]);
    let neq = |a: &str, b: &str| ProofExpr::Not(Box::new(ProofExpr::Identity(c(a), c(b))));
    let amo = |t: &str, u: &str| implies(and(in_fl(t), in_fl(u)), ProofExpr::Identity(c(t), c(u)));
    let of_pair = ProofExpr::Or(
        Box::new(and(in_fl("A"), pred("With", vec![c("B"), c("Neal")]))),
        Box::new(and(in_fl("B"), pred("With", vec![c("A"), c("Neal")]))),
    );
    let premises = vec![
        of_pair,
        amo("A", "C"),
        amo("B", "C"),
        neq("A", "C"),
        neq("B", "C"),
        neq("A", "B"),
    ];
    let goal = ProofExpr::Not(Box::new(in_fl("C")));
    let r = crate::verify::prove_certify_check(&premises, &goal);
    assert!(
        r.verified,
        "the of-pair clue must force ¬In(C, FL) by case analysis (no Z3); err: {:?}",
        r.verification_error
    );
}

/// The REAL of-pair shape a grid clue grounds to: a disjunction whose disjuncts
/// are CONJUNCTIONS that themselves carry an inner either/or
/// (`(A ∧ (P ∨ Q)) ∨ (Bad ∧ C)`). Refuting one outer disjunct by a false conjunct
/// (`¬Bad`) collapses it (disjunctive syllogism over a compound disjunct), the
/// survivor is DECOMPOSED to surface its inner `P ∨ Q`, and case analysis on that
/// — both arms forcing `¬R` — discharges the goal. This is exactly the structure
/// that made naive grounding explode; here it is propagated + decided, certified.
#[test]
fn compound_of_pair_disjunction_resolves() {
    let and = |l: ProofExpr, r: ProofExpr| ProofExpr::And(Box::new(l), Box::new(r));
    let or = |l: ProofExpr, r: ProofExpr| ProofExpr::Or(Box::new(l), Box::new(r));
    let implies = |l: ProofExpr, r: ProofExpr| ProofExpr::Implies(Box::new(l), Box::new(r));
    let not = |e: ProofExpr| ProofExpr::Not(Box::new(e));
    let a = pred("A", vec![c("Obj")]);
    let p = pred("P", vec![c("Obj")]);
    let q = pred("Q", vec![c("Obj")]);
    let bad = pred("Bad", vec![c("Obj")]);
    let cc = pred("C", vec![c("Obj")]);
    let r = pred("R", vec![c("Obj")]);
    let of_pair = or(and(a, or(p.clone(), q.clone())), and(bad.clone(), cc));
    let premises = vec![
        of_pair,
        not(bad),
        implies(p, not(r.clone())),
        implies(q, not(r.clone())),
    ];
    let goal = not(r);
    let res = crate::verify::prove_certify_check(&premises, &goal);
    assert!(
        res.verified,
        "compound of-pair must resolve by refute + decompose + case analysis; err: {:?}",
        res.verification_error
    );
}

#[test]
fn at_most_one_lemma_extracted_from_exactly_one() {
    // ∃x((Trip(x) ∧ In(x,FL)) ∧ ∀y((Trip(y) ∧ In(y,FL)) → y = x))
    let phi = |t: ProofTerm| {
        ProofExpr::And(
            Box::new(pred("Trip", vec![t.clone()])),
            Box::new(pred("In", vec![t, c("Florida")])),
        )
    };
    let exactly_one = exists(
        "x",
        ProofExpr::And(
            Box::new(phi(v("x"))),
            Box::new(forall(
                "y",
                ProofExpr::Implies(
                    Box::new(phi(v("y"))),
                    Box::new(ProofExpr::Identity(v("y"), v("x"))),
                ),
            )),
        ),
    );
    let lemmas = at_most_one_lemmas(&[exactly_one]);
    assert_eq!(lemmas.len(), 1, "one exactly-one ⇒ one at-most-one lemma");
    // The lemma is a pairwise ∀∀ uniqueness implication (no existential).
    assert!(
        matches!(&lemmas[0], ProofExpr::ForAll { body, .. }
            if matches!(body.as_ref(), ProofExpr::ForAll { .. })),
        "lemma must be ∀x∀y(…); got {:?}",
        lemmas[0]
    );
    // A plain fact yields no lemma.
    assert!(at_most_one_lemmas(&[pred("Trip", vec![c("Alpha")])]).is_empty());
}

fn conjuncts(e: &ProofExpr) -> usize {
    match e {
        ProofExpr::And(l, r) => conjuncts(l) + conjuncts(r),
        _ => 1,
    }
}

#[test]
fn sort_domains_collects_declared_members() {
    let premises = vec![
        pred("Trip", vec![c("Alpha")]),
        ProofExpr::And(
            Box::new(pred("Trip", vec![c("Beta")])),
            Box::new(pred("Year", vec![c("2001")])),
        ),
    ];
    let sorts = sort_domains(&premises);
    assert_eq!(sorts.get("Trip"), Some(&vec![c("Alpha"), c("Beta")]));
    assert_eq!(sorts.get("Year"), Some(&vec![c("2001")]));
}

#[test]
fn sort_aware_grounds_over_guard_sort_only() {
    // ∀x(Trip(x) → In(x, Maine)) grounds x over the 2 TRIPS, not the 4-constant
    // universe — the optimization that keeps grids tractable as they grow.
    let e = forall(
        "x",
        ProofExpr::Implies(
            Box::new(pred("Trip", vec![v("x")])),
            Box::new(pred("In", vec![v("x"), c("Maine")])),
        ),
    );
    let mut sorts = std::collections::HashMap::new();
    sorts.insert("Trip".to_string(), vec![c("Alpha"), c("Beta")]);
    let fallback = vec![c("Alpha"), c("Beta"), c("Maine"), c("Florida")];
    let g = ground_sorted(&e, &sorts, &fallback);
    assert_eq!(
        conjuncts(&g),
        2,
        "should ground over the 2 trips only: {g:?}"
    );
}

#[test]
fn ground_problem_removes_all_quantifiers() {
    let premises = vec![
        pred("Trip", vec![c("Alpha")]),
        forall(
            "x",
            ProofExpr::Implies(
                Box::new(pred("Trip", vec![v("x")])),
                Box::new(pred("In", vec![v("x"), c("Maine")])),
            ),
        ),
    ];
    let goal = pred("In", vec![c("Alpha"), c("Maine")]);
    let (gp, gg) = ground_problem(&premises, &goal);
    assert!(
        gp.iter().all(is_quantifier_free) && is_quantifier_free(&gg),
        "ground_problem must remove every quantifier"
    );
}
