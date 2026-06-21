//! Finite-domain grounding.
//!
//! A logic grid — and any finite, domain-closed problem — becomes DECIDABLE by
//! expanding bounded quantifiers over the finite set of individuals named in the
//! premises:
//!
//! ```text
//! ∀x. φ(x)   over {a, b, c, d}   →   φ(a) ∧ φ(b) ∧ φ(c) ∧ φ(d)
//! ∃x. φ(x)   over {a, b, c, d}   →   φ(a) ∨ φ(b) ∨ φ(c) ∨ φ(d)
//! ```
//!
//! The result is QUANTIFIER-FREE, so our kernel proves it (certified, explainable)
//! and Z3 decides it in milliseconds — no e-matching, no search blowup, guaranteed
//! to terminate. This is general and reusable: it knows nothing about any specific
//! puzzle, only about quantifiers and the finite domain the premises declare.

use crate::{ProofExpr, ProofTerm};

/// Ground every quantifier in `e` over `domain` (Herbrand expansion): a universal
/// becomes the conjunction of its instances, an existential the disjunction. The
/// returned expression is quantifier-free when `domain` covers the universe.
///
/// Sound AND complete for a domain-CLOSED finite problem (every individual named):
/// `∀x.φ ≡ ⋀_c φ[x:=c]` and `∃x.φ ≡ ⋁_c φ[x:=c]` exactly when the witness/
/// counter-example is guaranteed to be one of the `c`.
pub fn ground(e: &ProofExpr, domain: &[ProofTerm]) -> ProofExpr {
    match e {
        ProofExpr::ForAll { variable, body } => fold_conj(
            domain
                .iter()
                .map(|c| ground(&subst(body, variable, c), domain)),
        ),
        ProofExpr::Exists { variable, body } => fold_disj(
            domain
                .iter()
                .map(|c| ground(&subst(body, variable, c), domain)),
        ),
        ProofExpr::And(l, r) => {
            ProofExpr::And(Box::new(ground(l, domain)), Box::new(ground(r, domain)))
        }
        ProofExpr::Or(l, r) => {
            ProofExpr::Or(Box::new(ground(l, domain)), Box::new(ground(r, domain)))
        }
        ProofExpr::Implies(l, r) => {
            ProofExpr::Implies(Box::new(ground(l, domain)), Box::new(ground(r, domain)))
        }
        ProofExpr::Iff(l, r) => {
            ProofExpr::Iff(Box::new(ground(l, domain)), Box::new(ground(r, domain)))
        }
        ProofExpr::Not(x) => ProofExpr::Not(Box::new(ground(x, domain))),
        ProofExpr::Temporal { operator, body } => ProofExpr::Temporal {
            operator: operator.clone(),
            body: Box::new(ground(body, domain)),
        },
        // Quantifier-free leaves (Predicate, Identity, Atom, Term, …) are unchanged.
        leaf => leaf.clone(),
    }
}

/// Left-fold instances into a conjunction; an empty domain makes `∀` vacuously true.
fn fold_conj(mut it: impl Iterator<Item = ProofExpr>) -> ProofExpr {
    match it.next() {
        None => ProofExpr::Atom("True".to_string()),
        Some(first) => it.fold(first, |acc, e| ProofExpr::And(Box::new(acc), Box::new(e))),
    }
}

/// Left-fold instances into a disjunction; an empty domain makes `∃` false.
fn fold_disj(mut it: impl Iterator<Item = ProofExpr>) -> ProofExpr {
    match it.next() {
        None => ProofExpr::Atom("False".to_string()),
        Some(first) => it.fold(first, |acc, e| ProofExpr::Or(Box::new(acc), Box::new(e))),
    }
}

/// Substitute the bound variable `var` with the term `to` throughout `e`, stopping
/// at any quantifier that re-binds `var` (capture avoidance).
fn subst(e: &ProofExpr, var: &str, to: &ProofTerm) -> ProofExpr {
    match e {
        ProofExpr::Predicate { name, args, world } => ProofExpr::Predicate {
            name: name.clone(),
            args: args.iter().map(|a| subst_term(a, var, to)).collect(),
            world: world.clone(),
        },
        ProofExpr::Identity(a, b) => {
            ProofExpr::Identity(subst_term(a, var, to), subst_term(b, var, to))
        }
        ProofExpr::And(l, r) => {
            ProofExpr::And(Box::new(subst(l, var, to)), Box::new(subst(r, var, to)))
        }
        ProofExpr::Or(l, r) => {
            ProofExpr::Or(Box::new(subst(l, var, to)), Box::new(subst(r, var, to)))
        }
        ProofExpr::Implies(l, r) => {
            ProofExpr::Implies(Box::new(subst(l, var, to)), Box::new(subst(r, var, to)))
        }
        ProofExpr::Iff(l, r) => {
            ProofExpr::Iff(Box::new(subst(l, var, to)), Box::new(subst(r, var, to)))
        }
        ProofExpr::Not(x) => ProofExpr::Not(Box::new(subst(x, var, to))),
        ProofExpr::ForAll { variable, body } if variable != var => ProofExpr::ForAll {
            variable: variable.clone(),
            body: Box::new(subst(body, var, to)),
        },
        ProofExpr::Exists { variable, body } if variable != var => ProofExpr::Exists {
            variable: variable.clone(),
            body: Box::new(subst(body, var, to)),
        },
        ProofExpr::Temporal { operator, body } => ProofExpr::Temporal {
            operator: operator.clone(),
            body: Box::new(subst(body, var, to)),
        },
        ProofExpr::Term(t) => ProofExpr::Term(subst_term(t, var, to)),
        // Re-bound quantifiers, Atom, etc. — left as-is.
        other => other.clone(),
    }
}

/// The DOMAIN of a problem: every constant named across `exprs`. For a
/// domain-closed finite problem this is the universe of individuals to ground over.
pub fn domain_constants(exprs: &[ProofExpr]) -> Vec<ProofTerm> {
    let mut out: Vec<String> = Vec::new();
    for e in exprs {
        collect_constants(e, &mut out);
    }
    out.sort();
    out.dedup();
    out.into_iter().map(ProofTerm::Constant).collect()
}

fn collect_constants(e: &ProofExpr, out: &mut Vec<String>) {
    fn term(t: &ProofTerm, out: &mut Vec<String>) {
        match t {
            ProofTerm::Constant(s) => out.push(s.clone()),
            ProofTerm::Function(_, args) | ProofTerm::Group(args) => {
                args.iter().for_each(|a| term(a, out))
            }
            _ => {}
        }
    }
    match e {
        ProofExpr::Predicate { args, .. } => args.iter().for_each(|a| term(a, out)),
        ProofExpr::Identity(a, b) => {
            term(a, out);
            term(b, out);
        }
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            collect_constants(l, out);
            collect_constants(r, out);
        }
        ProofExpr::Not(x) => collect_constants(x, out),
        ProofExpr::ForAll { body, .. }
        | ProofExpr::Exists { body, .. }
        | ProofExpr::Temporal { body, .. } => collect_constants(body, out),
        ProofExpr::Term(t) => term(t, out),
        _ => {}
    }
}

use std::collections::HashMap;

/// Per-sort domains: for each UNARY predicate used as a sort guard (`Trip`,
/// `Year`, …), the constants asserted to have it. Built from the ground facts the
/// declarations produce (`Trip(Alpha)`, `Trip(Beta)`, …). This is what lets
/// grounding expand `∀x(Trip(x)→…)` over the 4 trips, not the whole universe.
pub fn sort_domains(premises: &[ProofExpr]) -> HashMap<String, Vec<ProofTerm>> {
    fn collect(e: &ProofExpr, m: &mut HashMap<String, Vec<ProofTerm>>) {
        match e {
            ProofExpr::Predicate { name, args, .. } if args.len() == 1 => {
                if let ProofTerm::Constant(_) = &args[0] {
                    let dom = m.entry(name.clone()).or_default();
                    if !dom.contains(&args[0]) {
                        dom.push(args[0].clone());
                    }
                }
            }
            // Sort facts are ground premises, possibly under top-level conjunction;
            // do NOT descend into quantifiers (those are rules, not facts).
            ProofExpr::And(l, r) => {
                collect(l, m);
                collect(r, m);
            }
            _ => {}
        }
    }
    let mut m = HashMap::new();
    for p in premises {
        collect(p, &mut m);
    }
    m
}

/// The unary-predicate SORT guarding `var` in `e` (e.g. `Trip` in `Trip(x) → …` or
/// `Trip(x) ∧ …`), if any — used to pick the right finite domain to ground over.
fn guard_sort(e: &ProofExpr, var: &str) -> Option<String> {
    match e {
        ProofExpr::Predicate { name, args, .. }
            if args.len() == 1 && matches!(&args[0], ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) if v == var) =>
        {
            Some(name.clone())
        }
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => guard_sort(l, var).or_else(|| guard_sort(r, var)),
        ProofExpr::Not(x) | ProofExpr::Temporal { body: x, .. } => guard_sort(x, var),
        // Recurse through a nested quantifier (that does not re-bind `var`) so an
        // of-pair "∃x(∃y(Trip(x) ∧ … ))" still finds x's guard `Trip` and grounds x
        // over the 4 trips, not the whole universe (where a phantom non-trip could
        // satisfy it).
        ProofExpr::ForAll { variable, body } | ProofExpr::Exists { variable, body }
            if variable != var =>
        {
            guard_sort(body, var)
        }
        _ => None,
    }
}

/// SORT-AWARE grounding: a guarded quantifier expands only over its sort's domain
/// (`∀x(Trip(x)→…)` over the trips), falling back to `fallback` (the full universe)
/// for an unsorted quantifier. Sound for a domain-closed problem and dramatically
/// smaller than universe-grounding — the optimization the larger puzzles need.
pub fn ground_sorted(
    e: &ProofExpr,
    sorts: &HashMap<String, Vec<ProofTerm>>,
    fallback: &[ProofTerm],
) -> ProofExpr {
    let domain_for = |body: &ProofExpr, var: &str| -> Vec<ProofTerm> {
        guard_sort(body, var)
            .and_then(|s| sorts.get(&s).cloned())
            .unwrap_or_else(|| fallback.to_vec())
    };
    match e {
        ProofExpr::ForAll { variable, body } => {
            let dom = domain_for(body, variable);
            fold_conj(
                dom.iter()
                    .map(|c| ground_sorted(&subst(body, variable, c), sorts, fallback)),
            )
        }
        ProofExpr::Exists { variable, body } => {
            let dom = domain_for(body, variable);
            fold_disj(
                dom.iter()
                    .map(|c| ground_sorted(&subst(body, variable, c), sorts, fallback)),
            )
        }
        ProofExpr::And(l, r) => ProofExpr::And(
            Box::new(ground_sorted(l, sorts, fallback)),
            Box::new(ground_sorted(r, sorts, fallback)),
        ),
        ProofExpr::Or(l, r) => ProofExpr::Or(
            Box::new(ground_sorted(l, sorts, fallback)),
            Box::new(ground_sorted(r, sorts, fallback)),
        ),
        ProofExpr::Implies(l, r) => ProofExpr::Implies(
            Box::new(ground_sorted(l, sorts, fallback)),
            Box::new(ground_sorted(r, sorts, fallback)),
        ),
        ProofExpr::Iff(l, r) => ProofExpr::Iff(
            Box::new(ground_sorted(l, sorts, fallback)),
            Box::new(ground_sorted(r, sorts, fallback)),
        ),
        ProofExpr::Not(x) => ProofExpr::Not(Box::new(ground_sorted(x, sorts, fallback))),
        ProofExpr::Temporal { operator, body } => ProofExpr::Temporal {
            operator: operator.clone(),
            body: Box::new(ground_sorted(body, sorts, fallback)),
        },
        leaf => leaf.clone(),
    }
}

/// [`ground_problem`] but sort-aware (each quantifier grounds over its guard
/// sort). The default entry point for finite-domain solving.
pub fn ground_problem_sorted(
    premises: &[ProofExpr],
    goal: &ProofExpr,
) -> (Vec<ProofExpr>, ProofExpr) {
    let mut all: Vec<ProofExpr> = premises.to_vec();
    all.push(goal.clone());
    let fallback = domain_constants(&all);
    let sorts = sort_domains(premises);
    let gp = premises
        .iter()
        .map(|p| ground_sorted(p, &sorts, &fallback))
        .collect();
    let gg = ground_sorted(goal, &sorts, &fallback);
    (gp, gg)
}

/// Ground a whole problem — premises AND goal — over the domain of constants they
/// name. The returned premises and goal are quantifier-free (so the kernel proves
/// them, certified, and Z3 decides them instantly).
pub fn ground_problem(
    premises: &[ProofExpr],
    goal: &ProofExpr,
) -> (Vec<ProofExpr>, ProofExpr) {
    let mut all: Vec<ProofExpr> = premises.to_vec();
    all.push(goal.clone());
    let domain = domain_constants(&all);
    let grounded_premises = premises.iter().map(|p| ground(p, &domain)).collect();
    let grounded_goal = ground(goal, &domain);
    (grounded_premises, grounded_goal)
}

/// "Exactly one φ" entails "at most one φ" — for every premise of the form
/// `∃x(φ(x) ∧ ∀y(φ(y) → y = x))` (what `Cardinal(1)` produces), emit the PAIRWISE
/// uniqueness `∀x∀y((φ(x) ∧ φ(y)) → x = y)`.
///
/// Why: the existence form grounds to a DISJUNCTION whose disjuncts nest the
/// uniqueness rule, which the certified backward-chainer cannot use directly; the
/// pairwise form grounds to plain conjunctive implications it discharges by forward
/// chaining (the shape `grounded_two_value_grid_proves_with_our_kernel` proves). The
/// lemma is logically ENTAILED by the original, so adding it is sound, and it is
/// GENERAL over any "exactly one" statement — never tied to a particular puzzle.
pub fn at_most_one_lemmas(premises: &[ProofExpr]) -> Vec<ProofExpr> {
    premises.iter().filter_map(at_most_one_of).collect()
}

/// FUNCTIONALITY — the missing half of a grid's bijection. Each closure
/// `∀x(guard(x) → L1 ∨ L2 ∨ … ∨ Ln)` says a row takes AT LEAST one value; this emits
/// the AT MOST ONE companion `∀x(Li → ¬Lj)` for every ordered pair of distinct
/// disjuncts. A trip is in exactly one state (one year, …), so a value assigned to a
/// row EXCLUDES the row's other values — the propagation that, together with
/// value-uniqueness, determines the whole grid. Sound: for a square grid functionality
/// is entailed by closure + value-uniqueness + counting, and it is in any case the
/// inherent single-valuedness of the attribute. Each `Li` must be an atom mentioning
/// the bound variable (a genuine closure), so unrelated `∀→∨` premises are skipped.
pub fn functionality_lemmas(premises: &[ProofExpr]) -> Vec<ProofExpr> {
    let mut out = Vec::new();
    for p in premises {
        let ProofExpr::ForAll { variable, body } = p else {
            continue;
        };
        let ProofExpr::Implies(_guard, cons) = body.as_ref() else {
            continue;
        };
        let lits = flatten_or(cons);
        let all_atoms_on_var = lits.len() >= 2
            && lits.iter().all(|l| {
                matches!(l, ProofExpr::Predicate { args, .. }
                    if args.iter().any(|a| matches!(a, ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) if v == variable)))
            });
        if !all_atoms_on_var {
            continue;
        }
        for (i, li) in lits.iter().enumerate() {
            for (j, lj) in lits.iter().enumerate() {
                if i == j {
                    continue;
                }
                out.push(ProofExpr::ForAll {
                    variable: variable.clone(),
                    body: Box::new(ProofExpr::Implies(
                        Box::new(li.clone()),
                        Box::new(ProofExpr::Not(Box::new(lj.clone()))),
                    )),
                });
            }
        }
    }
    out
}

/// HIDDEN SINGLE — the COLUMN closure, dual of the row closure. Each `Exactly one φ`
/// premise (`∃x(φ(x) ∧ ∀y(φ(y) → y = x))`, e.g. "exactly one trip is in Florida")
/// entails that AT LEAST ONE row holds the value: `⋁_{r∈dom} value(r)`. The row closure
/// says a row takes SOME value; this says a value is taken by SOME row. Together with
/// functionality (a row takes at most one value), it is the deduction a human calls a
/// "hidden single" — once every other row is excluded from a value, the remaining row
/// MUST hold it, and the EXISTING literal unit-propagation forces it.
///
/// Grounded over the guard sort's finite domain into a pure value-literal disjunction
/// (the sort guard `Trip(r)` is dropped — every `r` in the domain is a trip). Sound:
/// `∃x φ(x)` is just the existence half of "exactly one", here merely grounded — `compile`
/// otherwise SKIPS the `∃`, so the solver never sees the column closure without this.
/// Structural over any declared square bijection; never keyed to a particular puzzle.
pub fn column_closure_lemmas(premises: &[ProofExpr]) -> Vec<ProofExpr> {
    let domains = sort_domains(premises);
    let mut flat: Vec<ProofExpr> = Vec::new();
    for p in premises {
        flatten_and_into(p, &mut flat);
    }
    let mut out = Vec::new();
    for p in &flat {
        // Only the bare "exactly one" existentials — `at_most_one_of` recognises exactly
        // that shape (nested ∃∃ of-pairs and definite descriptions are NOT it).
        if at_most_one_of(p).is_none() {
            continue;
        }
        let ProofExpr::Exists { variable: x, body } = p else {
            continue;
        };
        let ProofExpr::And(phi_x, _uniq) = body.as_ref() else {
            continue;
        };
        let Some(sort) = guard_sort(phi_x, x) else {
            continue;
        };
        let Some(domain) = domains.get(&sort) else {
            continue;
        };
        if domain.len() < 2 {
            continue;
        }
        // The VALUE part of φ (φ with the sort guard `sort(x)` stripped), grounded at
        // each row of the domain and OR-folded: `value(r1) ∨ … ∨ value(rn)`.
        let value = strip_sort_guard(phi_x, &sort, x);
        let mut disj: Option<ProofExpr> = None;
        for r in domain {
            let lit_r = subst(&value, x, r);
            disj = Some(match disj {
                None => lit_r,
                Some(acc) => ProofExpr::Or(Box::new(acc), Box::new(lit_r)),
            });
        }
        if let Some(d) = disj {
            out.push(d);
        }
    }
    out
}

/// Drop the unary sort-guard conjunct `sort(var)` from a (conjunctive) property `phi`,
/// leaving the value literal(s). `Trip(x) ∧ In(x, Florida) ↦ In(x, Florida)`.
fn strip_sort_guard(phi: &ProofExpr, sort: &str, var: &str) -> ProofExpr {
    let is_guard = |e: &ProofExpr| {
        matches!(e, ProofExpr::Predicate { name, args, .. }
            if name == sort && args.len() == 1
                && matches!(&args[0], ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) if v == var))
    };
    if let ProofExpr::And(l, r) = phi {
        if is_guard(l) {
            return strip_sort_guard(r, sort, var);
        }
        if is_guard(r) {
            return strip_sort_guard(l, sort, var);
        }
        return ProofExpr::And(
            Box::new(strip_sort_guard(l, sort, var)),
            Box::new(strip_sort_guard(r, sort, var)),
        );
    }
    phi.clone()
}

fn flatten_or(e: &ProofExpr) -> Vec<ProofExpr> {
    match e {
        ProofExpr::Or(l, r) => {
            let mut v = flatten_or(l);
            v.extend(flatten_or(r));
            v
        }
        _ => vec![e.clone()],
    }
}

/// Discharge known ground UNARY facts (`Trip(Alpha)`, …) throughout the premises:
/// each is true, so replace it with `True` and simplify `True ∧ X ↦ X`,
/// `True → X ↦ X`. Unit propagation on the sort facts — it strips the sort guards out
/// of every grounded clause (`(Trip(x) ∧ In(x,FL)) ∧ … ↦ In(x,FL) ∧ …`, and a closure
/// `Trip(t) → D ↦ D` becomes a bare disjunction fact), leaving the pure value-literal
/// clauses propagation runs on in ONE step. Sound: a true conjunct/antecedent carries
/// no information.
pub fn discharge_unary_facts(premises: &[ProofExpr]) -> Vec<ProofExpr> {
    // Collect ground unary facts from FLATTENED premises — a multi-entity declaration
    // ("Alpha, Beta, Gamma, Delta are four different trips") arrives as one conjunction,
    // so `Trip(Alpha)…` live nested inside it, not as top-level premises.
    let mut flat: Vec<ProofExpr> = Vec::new();
    for p in premises {
        flatten_and_into(p, &mut flat);
    }
    let mut facts: Vec<(String, String)> = Vec::new();
    for p in &flat {
        if let ProofExpr::Predicate { name, args, .. } = p {
            if let [ProofTerm::Constant(c)] = args.as_slice() {
                let key = (name.clone(), c.clone());
                if !facts.contains(&key) {
                    facts.push(key);
                }
            }
        }
    }
    premises.iter().map(|p| discharge(p, &facts)).collect()
}

fn is_true(e: &ProofExpr) -> bool {
    matches!(e, ProofExpr::Atom(s) if s == "True")
}

fn is_false(e: &ProofExpr) -> bool {
    matches!(e, ProofExpr::Atom(s) if s == "False")
}

/// Fold away trivial (reflexive) identities and the boolean constants they create: a
/// grounded `∃x∃y` of-pair has a DIAGONAL instance (`x = y = c`) carrying `¬(c = c)`,
/// which is `False`, so that disjunct must drop. Without it the solver would face an
/// unsatisfiable disjunct it cannot refute (the kernel has no reflexivity rule to prove
/// `c = c`). `Identity(c,c) ↦ True`, `¬True ↦ False`, then `∧`/`∨`/`→` constant-fold.
pub fn simplify_trivial_identities(premises: &[ProofExpr]) -> Vec<ProofExpr> {
    premises.iter().map(simplify_ident).collect()
}

fn simplify_ident(e: &ProofExpr) -> ProofExpr {
    match e {
        ProofExpr::Identity(a, b) if a == b => ProofExpr::Atom("True".to_string()),
        ProofExpr::Not(inner) => {
            let s = simplify_ident(inner);
            if is_true(&s) {
                ProofExpr::Atom("False".to_string())
            } else if is_false(&s) {
                ProofExpr::Atom("True".to_string())
            } else {
                ProofExpr::Not(Box::new(s))
            }
        }
        ProofExpr::And(l, r) => {
            let (l, r) = (simplify_ident(l), simplify_ident(r));
            if is_false(&l) || is_false(&r) {
                ProofExpr::Atom("False".to_string())
            } else if is_true(&l) {
                r
            } else if is_true(&r) {
                l
            } else {
                ProofExpr::And(Box::new(l), Box::new(r))
            }
        }
        ProofExpr::Or(l, r) => {
            let (l, r) = (simplify_ident(l), simplify_ident(r));
            if is_true(&l) || is_true(&r) {
                ProofExpr::Atom("True".to_string())
            } else if is_false(&l) {
                r
            } else if is_false(&r) {
                l
            } else {
                ProofExpr::Or(Box::new(l), Box::new(r))
            }
        }
        ProofExpr::Implies(l, r) => {
            let (l, r) = (simplify_ident(l), simplify_ident(r));
            if is_false(&l) || is_true(&r) {
                ProofExpr::Atom("True".to_string())
            } else if is_true(&l) {
                r
            } else {
                ProofExpr::Implies(Box::new(l), Box::new(r))
            }
        }
        ProofExpr::ForAll { variable, body } => ProofExpr::ForAll {
            variable: variable.clone(),
            body: Box::new(simplify_ident(body)),
        },
        ProofExpr::Exists { variable, body } => ProofExpr::Exists {
            variable: variable.clone(),
            body: Box::new(simplify_ident(body)),
        },
        other => other.clone(),
    }
}

fn discharge(e: &ProofExpr, facts: &[(String, String)]) -> ProofExpr {
    match e {
        ProofExpr::Predicate { name, args, .. } => {
            if let [ProofTerm::Constant(c)] = args.as_slice() {
                if facts.iter().any(|(n, k)| n == name && k == c) {
                    return ProofExpr::Atom("True".to_string());
                }
            }
            e.clone()
        }
        ProofExpr::And(l, r) => {
            let (l, r) = (discharge(l, facts), discharge(r, facts));
            match (is_true(&l), is_true(&r)) {
                (true, _) => r,
                (_, true) => l,
                _ => ProofExpr::And(Box::new(l), Box::new(r)),
            }
        }
        ProofExpr::Implies(l, r) => {
            let (l, r) = (discharge(l, facts), discharge(r, facts));
            if is_true(&l) {
                r
            } else {
                ProofExpr::Implies(Box::new(l), Box::new(r))
            }
        }
        ProofExpr::Or(l, r) => {
            ProofExpr::Or(Box::new(discharge(l, facts)), Box::new(discharge(r, facts)))
        }
        ProofExpr::Not(x) => ProofExpr::Not(Box::new(discharge(x, facts))),
        ProofExpr::ForAll { variable, body } => ProofExpr::ForAll {
            variable: variable.clone(),
            body: Box::new(discharge(body, facts)),
        },
        ProofExpr::Exists { variable, body } => ProofExpr::Exists {
            variable: variable.clone(),
            body: Box::new(discharge(body, facts)),
        },
        ProofExpr::Temporal { operator, body } => ProofExpr::Temporal {
            operator: operator.clone(),
            body: Box::new(discharge(body, facts)),
        },
        other => other.clone(),
    }
}

fn at_most_one_of(e: &ProofExpr) -> Option<ProofExpr> {
    let ProofExpr::Exists { variable: x, body } = e else {
        return None;
    };
    let ProofExpr::And(phi_x, uniq) = body.as_ref() else {
        return None;
    };
    let ProofExpr::ForAll { variable: y, body: uniq_body } = uniq.as_ref() else {
        return None;
    };
    // The inner ∀ must be the uniqueness clause `φ(y) → y = x` (either order).
    let ProofExpr::Implies(_phi_y, ident) = uniq_body.as_ref() else {
        return None;
    };
    let ProofExpr::Identity(l, r) = ident.as_ref() else {
        return None;
    };
    let is_var = |t: &ProofTerm, name: &str| {
        matches!(t, ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) if v == name)
    };
    if !((is_var(l, y) && is_var(r, x)) || (is_var(l, x) && is_var(r, y))) {
        return None;
    }
    // ∀x∀y'((φ(x) ∧ φ(y')) → x = y'), with a fresh y' so it never collides with x.
    let y2 = format!("{x}_amo");
    let phi_x = phi_x.as_ref().clone();
    let phi_y2 = subst(&phi_x, x, &ProofTerm::Variable(y2.clone()));
    let body = ProofExpr::Implies(
        Box::new(ProofExpr::And(Box::new(phi_x), Box::new(phi_y2))),
        Box::new(ProofExpr::Identity(
            ProofTerm::Variable(x.clone()),
            ProofTerm::Variable(y2.clone()),
        )),
    );
    Some(ProofExpr::ForAll {
        variable: x.clone(),
        body: Box::new(ProofExpr::ForAll {
            variable: y2,
            body: Box::new(body),
        }),
    })
}

/// "The unique D-trip has property P." Each single-`∃` definite-description clue
/// `∃x(D(x) ∧ … ∧ P(x))` whose anchor `D` is a SINGLETON grid value (it carries an
/// `Exactly one …` premise, so the description denotes a unique row) entails the
/// propagatable rule `∀t(D(t) → P(t))`; when `P` is ALSO a singleton value the
/// biconditional `∀t(P(t) → D(t))` too — e.g. "the Florida trip is the hunting trip"
/// ⇒ `In(t,Florida) ↔ Hunt(t)`. Sound: a singleton value names a unique row, so any row
/// holding it IS that row and therefore carries the clue's property. This replaces the
/// existential (whose sort-aware grounding still explodes to a row-disjunction) with
/// row-indexed implications the incremental solver propagates.
///
/// Structural only — singleton-ness is read from the `Exactly one` premises, never the
/// literal value names — so it is not specific to any one puzzle. Nested `∃x∃y`
/// of-pair clues are left untouched (they ground to compound clauses the DPLL decides).
pub fn definite_property_implications(premises: &[ProofExpr]) -> Vec<ProofExpr> {
    let row_sorts = row_sort_predicates(premises);
    let singletons = singleton_value_signatures(premises, &row_sorts);
    let mut flat: Vec<ProofExpr> = Vec::new();
    for p in premises {
        flatten_and_into(p, &mut flat);
    }
    let mut out = Vec::new();
    for p in &flat {
        let ProofExpr::Exists { variable: x, body } = p else {
            continue;
        };
        // Nested ∃∃ (of-pair) and the bare `Exactly one` premises are not definite
        // descriptions of a property — leave them to grounding / at-most-one.
        if matches!(body.as_ref(), ProofExpr::Exists { .. }) || at_most_one_of(p).is_some() {
            continue;
        }
        let lits = clue_literals_on(body, x, &row_sorts);
        for (i, anchor) in lits.iter().enumerate() {
            let Some(sig) = positive_signature(anchor) else {
                continue;
            };
            if !singletons.contains(&sig) {
                continue;
            }
            for (j, other) in lits.iter().enumerate() {
                if i == j {
                    continue;
                }
                out.push(ProofExpr::ForAll {
                    variable: x.clone(),
                    body: Box::new(ProofExpr::Implies(Box::new(anchor.clone()), Box::new(other.clone()))),
                });
            }
        }
    }
    out
}

fn flatten_and_into(e: &ProofExpr, out: &mut Vec<ProofExpr>) {
    match e {
        ProofExpr::And(l, r) => {
            flatten_and_into(l, out);
            flatten_and_into(r, out);
        }
        _ => out.push(e.clone()),
    }
}

/// Unary predicate names that appear as a ground fact (`Trip(Alpha)`): the row sort
/// guard, dropped from clue bodies so only the value/property literals remain.
fn row_sort_predicates(premises: &[ProofExpr]) -> std::collections::HashSet<String> {
    let mut flat: Vec<ProofExpr> = Vec::new();
    for p in premises {
        flatten_and_into(p, &mut flat);
    }
    let mut s = std::collections::HashSet::new();
    for p in &flat {
        if let ProofExpr::Predicate { name, args, .. } = p {
            if matches!(args.as_slice(), [ProofTerm::Constant(_)]) {
                s.insert(name.clone());
            }
        }
    }
    s
}

/// `(predicate, value)` signatures of the singleton grid values — the value literal of
/// every `Exactly one …` (`∃x((guard(x) ∧ φ(x)) ∧ ∀y(φ(y) → y = x))`) premise.
fn singleton_value_signatures(
    premises: &[ProofExpr],
    row_sorts: &std::collections::HashSet<String>,
) -> std::collections::HashSet<(String, String)> {
    let mut s = std::collections::HashSet::new();
    for p in premises {
        let ProofExpr::Exists { variable: x, body } = p else {
            continue;
        };
        let ProofExpr::And(phi, uniq) = body.as_ref() else {
            continue;
        };
        if !matches!(uniq.as_ref(), ProofExpr::ForAll { .. }) {
            continue;
        }
        for lit in clue_literals_on(phi, x, row_sorts) {
            if let Some(sig) = positive_signature(&lit) {
                s.insert(sig);
            }
        }
    }
    s
}

/// `(name, value)` for a positive value literal (`In(x,Florida)` → `("In","Florida")`,
/// the unary `Hunt(x)` → `("Hunt","")`); `None` for negatives / non-value atoms.
fn positive_signature(e: &ProofExpr) -> Option<(String, String)> {
    let ProofExpr::Predicate { name, args, .. } = e else {
        return None;
    };
    match args.as_slice() {
        [_, ProofTerm::Constant(v)] => Some((name.clone(), v.clone())),
        [_] => Some((name.clone(), String::new())),
        _ => None,
    }
}

fn term_is_var(t: &ProofTerm, x: &str) -> bool {
    matches!(t, ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) if v == x)
}

fn mentions_var(args: &[ProofTerm], x: &str) -> bool {
    args.iter().any(|a| term_is_var(a, x))
}

/// The literal value/property conjuncts of a clue body that mention `x`: a conjunction
/// is split, the row-sort guard is dropped, uniqueness `∀` clauses are ignored, and a
/// negated conjunction `¬(guard(x) ∧ L(x))` collapses to `¬L(x)`.
fn clue_literals_on(
    e: &ProofExpr,
    x: &str,
    row_sorts: &std::collections::HashSet<String>,
) -> Vec<ProofExpr> {
    match e {
        ProofExpr::And(l, r) => {
            let mut v = clue_literals_on(l, x, row_sorts);
            v.extend(clue_literals_on(r, x, row_sorts));
            v
        }
        ProofExpr::Predicate { name, args, .. } => {
            if mentions_var(args, x) && !row_sorts.contains(name) {
                vec![e.clone()]
            } else {
                vec![]
            }
        }
        ProofExpr::Not(inner) => match drop_row_guards(inner, x, row_sorts) {
            Some(lit) => vec![ProofExpr::Not(Box::new(lit))],
            None => vec![],
        },
        _ => vec![],
    }
}

/// Strip row-sort guards from a (possibly conjunctive) expression, returning the single
/// remaining value literal on `x` (or `None` if it is not a clean single literal).
fn drop_row_guards(
    e: &ProofExpr,
    x: &str,
    row_sorts: &std::collections::HashSet<String>,
) -> Option<ProofExpr> {
    let mut lits = clue_literals_on(e, x, row_sorts);
    if lits.len() == 1 {
        Some(lits.pop().unwrap())
    } else {
        None
    }
}

fn subst_term(t: &ProofTerm, var: &str, to: &ProofTerm) -> ProofTerm {
    match t {
        ProofTerm::Variable(x) if x == var => to.clone(),
        ProofTerm::BoundVarRef(x) if x == var => to.clone(),
        ProofTerm::Function(name, args) => {
            ProofTerm::Function(name.clone(), args.iter().map(|a| subst_term(a, var, to)).collect())
        }
        ProofTerm::Group(args) => {
            ProofTerm::Group(args.iter().map(|a| subst_term(a, var, to)).collect())
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
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
                    Box::new(ProofExpr::And(Box::new(trip(v("x"))), Box::new(in_(v("x"), "Florida")))),
                    Box::new(forall(
                        "y",
                        ProofExpr::Implies(
                            Box::new(ProofExpr::And(Box::new(trip(v("y"))), Box::new(in_(v("y"), "Florida")))),
                            Box::new(ProofExpr::Identity(v("y"), v("x"))),
                        ),
                    )),
                )),
                Box::new(ProofExpr::And(Box::new(hunt(v("x"))), Box::new(trip(v("x"))))),
            ),
        );
        // Clue 4: ∃x(((Trip(x)∧With(x,Yvonne)) ∧ ∀y(...)) ∧ ¬In(x,Kentucky))
        let clue4 = exists(
            "x",
            ProofExpr::And(
                Box::new(ProofExpr::And(
                    Box::new(ProofExpr::And(Box::new(trip(v("x"))), Box::new(with(v("x"), "Yvonne")))),
                    Box::new(forall(
                        "y",
                        ProofExpr::Implies(
                            Box::new(ProofExpr::And(Box::new(trip(v("y"))), Box::new(with(v("y"), "Yvonne")))),
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
            imps.contains(&fwd(with(v("x"), "Yvonne"), ProofExpr::Not(Box::new(in_(v("x"), "Kentucky"))))),
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
        assert!(is_quantifier_free(&g), "grounded exactly-one must be quantifier-free: {g:?}");
    }

    #[test]
    fn kernel_does_grounded_disjunctive_syllogism() {
        // The core step the grounded grid needs: In(Beta,FL) ∨ In(Beta,ME), ¬In(Beta,FL) ⊢ In(Beta,ME).
        let in_ = |t: &str, s: &str| pred("In", vec![c(t), c(s)]);
        let premises = vec![
            ProofExpr::Or(Box::new(in_("Beta", "Florida")), Box::new(in_("Beta", "Maine"))),
            ProofExpr::Not(Box::new(in_("Beta", "Florida"))),
        ];
        let goal = in_("Beta", "Maine");
        let r = crate::verify::prove_certify_check(&premises, &goal);
        assert!(r.verified, "grounded disjunctive syllogism; err: {:?}", r.verification_error);
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
                        Box::new(ProofExpr::And(Box::new(trip(v("x"))), Box::new(in_(v("x"), fl())))),
                        Box::new(ProofExpr::And(Box::new(trip(v("y"))), Box::new(in_(v("y"), fl())))),
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
        let phi = |t: ProofTerm| {
            ProofExpr::And(Box::new(trip(t.clone())), Box::new(in_(t, fl())))
        };
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
                out.push(fold_disj(vals.iter().map(|val| pred(rel, vec![c(t), c(val)]))));
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
        let amo = |t: &str, u: &str| {
            implies(and(in_fl(t), in_fl(u)), ProofExpr::Identity(c(t), c(u)))
        };
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
        assert_eq!(conjuncts(&g), 2, "should ground over the 2 trips only: {g:?}");
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
}
