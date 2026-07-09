//! Backward chaining proof engine.
//!
//! "The machine that crawls backward from the Conclusion to the Axioms."
//!
//! This module implements the core proof search algorithm. It takes inference
//! rules and *hunts* for proofs using backward chaining and unification.
//!
//! ## Backward Chaining Strategy
//!
//! 1. Start with the goal we want to prove
//! 2. Find rules whose conclusions unify with our goal
//! 3. Recursively prove the premises of those rules
//! 4. Build the derivation tree as we succeed
//!
//! ## Example
//!
//! ```text
//! Goal: Mortal(socrates)
//!
//! Knowledge Base:
//!   - Human(socrates)
//!   - ∀x(Human(x) → Mortal(x))
//!
//! Search:
//!   1. Goal matches conclusion of ∀x(Human(x) → Mortal(x)) with x=socrates
//!   2. New subgoal: Human(socrates)
//!   3. Human(socrates) matches knowledge base fact
//!   4. Build derivation tree: ModusPonens(UniversalInst, PremiseMatch)
//! ```

use crate::error::{ProofError, ProofResult};
use crate::unify::{
    apply_subst_to_expr, apply_subst_to_term, beta_reduce, compose_substitutions, unify_exprs,
    unify_pattern, unify_terms, Substitution,
};
use crate::{DerivationTree, InferenceRule, ProofExpr, ProofGoal, ProofTerm};

/// Default maximum depth for proof search.
const DEFAULT_MAX_DEPTH: usize = 100;

/// Default node budget for proof search — the total number of `prove_goal` invocations
/// allowed for one top-level proof. The depth bound alone does NOT guarantee bounded
/// *time*: with a recursive axiom in scope the branching factor is large, so a depth-100
/// search can visit `b^100` nodes — effectively a hang. Counting nodes and stopping at the
/// budget makes every search terminate in bounded time (fail-fast → `verified = false`)
/// instead of running forever. Generous enough that no legitimate proof reaches it; the
/// relevance ordering in `try_backward_chain` keeps real proofs far below it.
const DEFAULT_STEP_BUDGET: usize = 100_000;

/// The backward chaining proof engine.
///
/// Searches for proofs by working backwards from the goal, finding rules
/// whose conclusions match, and recursively proving their premises.
pub struct BackwardChainer {
    /// Knowledge base: facts and rules available to the prover.
    knowledge_base: Vec<ProofExpr>,

    /// Maximum proof depth (prevents infinite loops).
    max_depth: usize,

    /// Counter for generating fresh variable names.
    var_counter: usize,

    /// Existentials already opened on the current search branch. Forward
    /// existential elimination skolemizes `∃x.P(x)` into witness facts; without
    /// this guard the same existential is re-opened with a fresh constant on
    /// every recursive `prove_goal`, never adding new reasoning power but
    /// spinning down to the depth limit (where the oracle, the last resort,
    /// catches the goal and yields an uncertifiable proof). Recording the
    /// opened existential collapses that loop to a single elimination.
    eliminated_existentials: Vec<ProofExpr>,

    /// Loop-detection stack: the canonical keys of the goals currently being
    /// proved along the active branch (an ancestor chain, pushed on entry to
    /// `prove_goal` and popped on exit). A *recursive* axiom — one whose
    /// conclusion shares a predicate with its own antecedent, like Tarski's inner
    /// transitivity — lets the search re-derive a goal from a fresh instance of
    /// itself: `Cong(?,?,A,B)` ⇐ `Cong(?',?',A,B)` ⇐ … with new existentials each
    /// time. The depth bound only stops that after `max_depth` native frames,
    /// which overflows the stack first. Keying goals modulo their existential
    /// renaming and refusing to re-enter one already on the branch collapses the
    /// regress to a finite search — without touching the productive branches,
    /// which discharge their antecedents against premises/facts, not by re-entry.
    active_goals: Vec<String>,

    /// Nodes (`prove_goal` invocations) spent on the current top-level proof. Reset when a
    /// proof starts (depth 0) and incremented on each `prove_goal`; the search aborts once
    /// it passes `step_budget`. This is the *time* bound the depth limit cannot provide.
    steps: usize,

    /// The node budget — see [`DEFAULT_STEP_BUDGET`].
    step_budget: usize,
}

// =============================================================================
// HELPER FUNCTIONS
// =============================================================================

/// Convert a ProofTerm to a ProofExpr for reduction.
///
/// Terms embed into expressions as atoms or constructors.
fn term_to_expr(term: &ProofTerm) -> ProofExpr {
    match term {
        ProofTerm::Constant(s) => ProofExpr::Atom(s.clone()),
        ProofTerm::Variable(s) => ProofExpr::Atom(s.clone()),
        ProofTerm::BoundVarRef(s) => ProofExpr::Atom(s.clone()),
        ProofTerm::Function(name, args) => {
            // Check if this is a known constructor
            if matches!(name.as_str(), "Zero" | "Succ" | "Nil" | "Cons") {
                ProofExpr::Ctor {
                    name: name.clone(),
                    args: args.iter().map(term_to_expr).collect(),
                }
            } else {
                // Otherwise it's a predicate/function
                ProofExpr::Predicate {
                    name: name.clone(),
                    args: args.clone(),
                    world: None,
                }
            }
        }
        ProofTerm::Group(terms) => {
            // Groups become nested predicates or just the single element
            if terms.len() == 1 {
                term_to_expr(&terms[0])
            } else {
                // Multi-term groups - convert to predicate
                ProofExpr::Predicate {
                    name: "Group".into(),
                    args: terms.clone(),
                    world: None,
                }
            }
        }
    }
}

/// Whether an expression is falsum (⊥). The engine represents absurdity as the
/// atom `⊥`; we also accept the spelled forms for robustness across producers.
fn is_falsum(expr: &ProofExpr) -> bool {
    matches!(expr, ProofExpr::Atom(s) if s == "⊥" || s == "False" || s == "false")
}

/// Flatten a (possibly nested) conjunction into its individual conjuncts.
/// `A ∧ (B ∧ C)` becomes `[A, B, C]`; a non-conjunction becomes a singleton.
fn flatten_conjuncts(expr: &ProofExpr) -> Vec<ProofExpr> {
    match expr {
        ProofExpr::And(left, right) => {
            let mut conjuncts = flatten_conjuncts(left);
            conjuncts.extend(flatten_conjuncts(right));
            conjuncts
        }
        other => vec![other.clone()],
    }
}

/// Reassemble per-conjunct `proofs` (in `flatten_conjuncts` order) into a proof tree that
/// mirrors `ante`'s `And` structure with BINARY [`InferenceRule::ConjunctionIntro`] nodes —
/// the only shape the certifier accepts. Each `And` becomes one binary node over the proofs
/// of its two sides; each leaf consumes the next proof. Recursing left-then-right matches
/// `flatten_conjuncts`, so the leaves line up with the proofs without reordering.
fn build_conjunction_tree(
    ante: &ProofExpr,
    subst: &Substitution,
    proofs: &mut std::vec::IntoIter<DerivationTree>,
) -> DerivationTree {
    match ante {
        ProofExpr::And(left, right) => {
            let lt = build_conjunction_tree(left, subst, proofs);
            let rt = build_conjunction_tree(right, subst, proofs);
            DerivationTree::new(
                apply_subst_to_expr(ante, subst),
                InferenceRule::ConjunctionIntro,
                vec![lt, rt],
            )
        }
        _ => proofs
            .next()
            .expect("flatten_conjuncts yields exactly one proof per non-And leaf"),
    }
}

/// Whether an expression is a ground/atomic *fact* — the kind a subgoal can be
/// discharged against directly by unification (as opposed to a rule to chain).
fn is_atomic_fact(expr: &ProofExpr) -> bool {
    matches!(
        expr,
        ProofExpr::Predicate { .. } | ProofExpr::Identity(_, _) | ProofExpr::Atom(_)
    )
}

// =============================================================================
// CERTIFIABLE CONTRADICTION FINDER (gapless, for verified conflict detection)
// =============================================================================
//
// Every derivation these helpers build certifies end-to-end: each step is an
// explicit PremiseMatch / UniversalInst / ConjunctionIntro / ModusPonens /
// Contradiction / CaseAnalysis node. Forward-chaining saturates the known facts;
// a bounded case-analysis layer handles self-referential paradoxes (e.g. the
// Barber stated with simple predicates) by splitting on a candidate atom and
// driving both branches to ⊥ — intuitionistically, so the kernel needs no
// excluded middle.

/// A Horn-style rule: `ante → cons`, optionally under a `∀ binder`.
struct CertRule {
    source: ProofExpr,
    binder: Option<String>,
    ante: ProofExpr,
    cons: ProofExpr,
}

fn cert_is_rule(e: &ProofExpr) -> bool {
    matches!(e, ProofExpr::Implies(..))
        || matches!(e, ProofExpr::ForAll { body, .. }
            if matches!(body.as_ref(), ProofExpr::Implies(..)))
}

fn cert_extract_rules(all: &[ProofExpr]) -> Vec<CertRule> {
    let mut rules = Vec::new();
    for e in all {
        match e {
            ProofExpr::Implies(a, c) => rules.push(CertRule {
                source: e.clone(),
                binder: None,
                ante: (**a).clone(),
                cons: (**c).clone(),
            }),
            ProofExpr::ForAll { variable, body } => {
                if let ProofExpr::Implies(a, c) = body.as_ref() {
                    rules.push(CertRule {
                        source: e.clone(),
                        binder: Some(variable.clone()),
                        ante: (**a).clone(),
                        cons: (**c).clone(),
                    });
                }
            }
            _ => {}
        }
    }
    rules
}

/// Seed the non-rule premises as `PremiseMatch` leaves.
fn cert_seed_facts(all: &[ProofExpr]) -> Vec<(ProofExpr, DerivationTree)> {
    let mut known: Vec<(ProofExpr, DerivationTree)> = Vec::new();
    for e in all {
        if cert_is_rule(e) {
            continue;
        }
        if !known.iter().any(|(p, _)| exprs_structurally_equal(p, e)) {
            known.push((
                e.clone(),
                DerivationTree::leaf(e.clone(), InferenceRule::PremiseMatch),
            ));
        }
    }
    known
}

/// Build a certifiable proof that `a = c`, given a directed edge graph in which
/// each `(b, edge_tree)` in `adj[a]` proves `a = b`. The returned tree chains the
/// edges along a path from `a` to `c` using `Rewrite` (Leibniz): from a proof of
/// `a = b` (the running accumulator, `P(b)` for `P = λz. a = z`) and a proof of
/// `b = d` (the next edge, the equality), `Eq_rec` rewrites `b ↦ d` to yield
/// `a = d`. A `None` means no path exists. Every node certifies.
fn cert_eq_path_proof(
    a: &ProofTerm,
    c: &ProofTerm,
    adj: &std::collections::HashMap<String, Vec<(ProofTerm, DerivationTree)>>,
) -> Option<DerivationTree> {
    fn term_key(t: &ProofTerm) -> Option<String> {
        term_skey(t)
    }
    let a_key = term_key(a)?;
    let c_key = term_key(c)?;
    if a_key == c_key {
        return None;
    }
    // BFS over constants, carrying the running proof of `a = current`.
    let start_proof = DerivationTree::leaf(
        ProofExpr::Identity(a.clone(), a.clone()),
        InferenceRule::Reflexivity,
    );
    let mut queue: std::collections::VecDeque<(String, ProofTerm, DerivationTree)> =
        std::collections::VecDeque::new();
    queue.push_back((a_key.clone(), a.clone(), start_proof));
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    visited.insert(a_key.clone());

    while let Some((cur_key, cur_term, acc_proof)) = queue.pop_front() {
        if cur_key == c_key {
            // The accumulator already proves `a = c` once we've stepped onto `c`.
            // (The start node `a = a` is reflexive and never the target, guarded
            // above.)
            return Some(acc_proof);
        }
        let Some(edges) = adj.get(&cur_key) else {
            continue;
        };
        for (next_term, edge_tree) in edges {
            let Some(next_key) = term_key(next_term) else {
                continue;
            };
            if visited.contains(&next_key) {
                continue;
            }
            visited.insert(next_key.clone());
            // acc_proof : a = cur ;  edge_tree : cur = next.
            // Rewrite cur ↦ next in `a = cur` to get `a = next`.
            let new_concl = ProofExpr::Identity(a.clone(), next_term.clone());
            let step = if cur_key == a_key {
                // First step: `a = a` rewritten by `a = next` is just the edge.
                edge_tree.clone()
            } else {
                DerivationTree::new(
                    new_concl,
                    InferenceRule::Rewrite {
                        from: cur_term.clone(),
                        to: next_term.clone(),
                    },
                    vec![edge_tree.clone(), acc_proof.clone()],
                )
            };
            queue.push_back((next_key, next_term.clone(), step));
        }
    }
    None
}

/// Close an equality contradiction: a known `¬(x = y)` paired with a chain of
/// known equalities that entails `x = y`. The equalities form an undirected
/// graph (each `a = b` fact also gives `b = a` via `EqualitySymmetry`); if `x`
/// and `y` lie in one component, a path proof of `x = y` (or `y = x`, then
/// symmetrised) discharges the negation. Every emitted node certifies.
fn cert_equality_close(known: &[(ProofExpr, DerivationTree)]) -> Option<DerivationTree> {
    use std::collections::HashMap;
    // Build the symmetric adjacency: each known `a = b` contributes the edge
    // a→b (its own proof) and b→a (its proof, symmetrised).
    let mut adj: HashMap<String, Vec<(ProofTerm, DerivationTree)>> = HashMap::new();
    fn key(t: &ProofTerm) -> Option<String> {
        match t {
            ProofTerm::Constant(s) | ProofTerm::Variable(s) | ProofTerm::BoundVarRef(s) => {
                Some(s.clone())
            }
            _ => None,
        }
    }
    for (prop, tree) in known {
        if let ProofExpr::Identity(l, r) = prop {
            let (Some(lk), Some(rk)) = (key(l), key(r)) else {
                continue;
            };
            if lk == rk {
                continue;
            }
            adj.entry(lk).or_default().push((r.clone(), tree.clone()));
            let sym = DerivationTree::new(
                ProofExpr::Identity(r.clone(), l.clone()),
                InferenceRule::EqualitySymmetry,
                vec![tree.clone()],
            );
            adj.entry(rk).or_default().push((l.clone(), sym));
        }
    }
    if adj.is_empty() {
        return None;
    }

    for (prop, neg_tree) in known {
        let ProofExpr::Not(inner) = prop else {
            continue;
        };
        let ProofExpr::Identity(x, y) = inner.as_ref() else {
            continue;
        };
        // Prove the EXACT polarity the negation refutes: ¬(x = y) needs `x = y`.
        if let Some(eq_proof) = cert_eq_path_proof(x, y, &adj) {
            return Some(DerivationTree::new(
                ProofExpr::Atom("⊥".into()),
                InferenceRule::Contradiction,
                vec![eq_proof, neg_tree.clone()],
            ));
        }
    }
    None
}

/// A structural key for a term — atomic names map to themselves, applications to
/// `head(arg₀,…,argₙ)` recursively. This lets the equality graph treat compound
/// terms (e.g. `F(A)`) as first-class nodes, the precondition for congruence.
fn term_skey(t: &ProofTerm) -> Option<String> {
    match t {
        ProofTerm::Constant(s) | ProofTerm::Variable(s) | ProofTerm::BoundVarRef(s) => {
            Some(s.clone())
        }
        ProofTerm::Function(f, args) => {
            let mut parts = Vec::with_capacity(args.len());
            for a in args {
                parts.push(term_skey(a)?);
            }
            Some(format!("{}({})", f, parts.join(",")))
        }
        ProofTerm::Group(args) => {
            let mut parts = Vec::with_capacity(args.len());
            for a in args {
                parts.push(term_skey(a)?);
            }
            Some(format!("({})", parts.join(",")))
        }
        _ => None,
    }
}

/// Collect a term and all its subterms (for the congruence universe).
fn collect_subterms(t: &ProofTerm, out: &mut Vec<ProofTerm>) {
    out.push(t.clone());
    if let ProofTerm::Function(_, args) | ProofTerm::Group(args) = t {
        for a in args {
            collect_subterms(a, out);
        }
    }
}

/// Build a proof of `head(xs) = head(ys)` from per-argument equality proofs,
/// rewriting one differing argument at a time. `arg_proofs[i]` is `None` when
/// `xs[i]` and `ys[i]` are structurally identical, else a proof of `xs[i]=ys[i]`.
/// Each step is a `Rewrite` (Leibniz / `Eq_rec`): its motive `λz. head(xs)=head(…z…)`
/// abstracts the i-th argument of the right-hand side, so the source premise is the
/// running `head(xs)=head(cur)` and the equality premise the argument proof.
fn build_congruence_proof(
    head: &str,
    xs: &[ProofTerm],
    ys: &[ProofTerm],
    arg_proofs: &[Option<DerivationTree>],
) -> DerivationTree {
    let f = |args: &[ProofTerm]| ProofTerm::Function(head.to_string(), args.to_vec());
    let mut cur: Vec<ProofTerm> = xs.to_vec();
    let mut acc = DerivationTree::leaf(
        ProofExpr::Identity(f(xs), f(xs)),
        InferenceRule::Reflexivity,
    );
    for i in 0..xs.len() {
        let Some(arg_proof) = &arg_proofs[i] else {
            continue;
        };
        let mut next = cur.clone();
        next[i] = ys[i].clone();
        let concl = ProofExpr::Identity(f(xs), f(&next));
        acc = DerivationTree::new(
            concl,
            InferenceRule::Rewrite {
                from: xs[i].clone(),
                to: ys[i].clone(),
            },
            vec![arg_proof.clone(), acc],
        );
        cur = next;
    }
    acc
}

/// Prove `lhs = rhs` by congruence closure over the hypothesis equalities. Build an
/// equality graph whose nodes are all subterms (compound included), seed it with the
/// hypotheses (and their symmetrics), then saturate with congruence edges — for any
/// two present applications `F(xs)`, `F(ys)` whose arguments are already pairwise
/// connected, add `F(xs)=F(ys)` (its proof built by [`build_congruence_proof`]) — to
/// a fixpoint. Finally search for a transitive path `lhs → rhs`. Every emitted node
/// certifies (Reflexivity / EqualitySymmetry / Rewrite / EqualityTransitivity).
pub(crate) fn cert_congruence_path(
    lhs: &ProofTerm,
    rhs: &ProofTerm,
    hyps: &[(ProofExpr, DerivationTree)],
) -> Option<DerivationTree> {
    use std::collections::HashMap;
    let mut adj: HashMap<String, Vec<(ProofTerm, DerivationTree)>> = HashMap::new();

    // Seed the graph with hypothesis equalities, both directions.
    for (prop, tree) in hyps {
        let ProofExpr::Identity(l, r) = prop else {
            continue;
        };
        let (Some(lk), Some(rk)) = (term_skey(l), term_skey(r)) else {
            continue;
        };
        if lk == rk {
            continue;
        }
        adj.entry(lk).or_default().push((r.clone(), tree.clone()));
        let sym = DerivationTree::new(
            ProofExpr::Identity(r.clone(), l.clone()),
            InferenceRule::EqualitySymmetry,
            vec![tree.clone()],
        );
        adj.entry(rk).or_default().push((l.clone(), sym));
    }

    // The universe of subterms (deduplicated by structural key).
    let mut universe: Vec<ProofTerm> = Vec::new();
    for (prop, _) in hyps {
        if let ProofExpr::Identity(l, r) = prop {
            collect_subterms(l, &mut universe);
            collect_subterms(r, &mut universe);
        }
    }
    collect_subterms(lhs, &mut universe);
    collect_subterms(rhs, &mut universe);
    let funcs: Vec<ProofTerm> = {
        let mut seen = std::collections::HashSet::new();
        universe
            .into_iter()
            .filter(|t| matches!(t, ProofTerm::Function(..)))
            .filter(|t| term_skey(t).is_some_and(|k| seen.insert(k)))
            .collect()
    };

    // Saturate congruence edges to a fixpoint.
    loop {
        let mut added = false;
        for i in 0..funcs.len() {
            for j in 0..funcs.len() {
                if i == j {
                    continue;
                }
                let (ProofTerm::Function(fi, xs), ProofTerm::Function(fj, ys)) =
                    (&funcs[i], &funcs[j])
                else {
                    continue;
                };
                if fi != fj || xs.len() != ys.len() {
                    continue;
                }
                let (ki, kj) = (term_skey(&funcs[i]).unwrap(), term_skey(&funcs[j]).unwrap());
                let already = adj.get(&ki).is_some_and(|es| {
                    es.iter().any(|(t, _)| term_skey(t).as_deref() == Some(kj.as_str()))
                });
                if already {
                    continue;
                }
                let mut arg_proofs: Vec<Option<DerivationTree>> = Vec::with_capacity(xs.len());
                let mut ok = true;
                for (x, y) in xs.iter().zip(ys.iter()) {
                    if term_skey(x) == term_skey(y) {
                        arg_proofs.push(None);
                    } else if let Some(p) = cert_eq_path_proof(x, y, &adj) {
                        arg_proofs.push(Some(p));
                    } else {
                        ok = false;
                        break;
                    }
                }
                if !ok {
                    continue;
                }
                let cong = build_congruence_proof(fi, xs, ys, &arg_proofs);
                let sym = DerivationTree::new(
                    ProofExpr::Identity(funcs[j].clone(), funcs[i].clone()),
                    InferenceRule::EqualitySymmetry,
                    vec![cong.clone()],
                );
                adj.entry(ki).or_default().push((funcs[j].clone(), cong));
                adj.entry(kj).or_default().push((funcs[i].clone(), sym));
                added = true;
            }
        }
        if !added {
            break;
        }
    }

    cert_eq_path_proof(lhs, rhs, &adj)
}

/// The inequality `a ≤ b`, encoded as the Prop `le(a, b) = true`.
pub(crate) fn le_eq(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(
        ProofTerm::Function("le".to_string(), vec![a, b]),
        ProofTerm::Constant("true".to_string()),
    )
}

/// If `e` is an inequality `le(a, b) = true`, return `(a, b)`.
pub(crate) fn as_le_pair(e: &ProofExpr) -> Option<(ProofTerm, ProofTerm)> {
    if let ProofExpr::Identity(lhs, rhs) = e {
        if let (ProofTerm::Function(name, args), ProofTerm::Constant(t)) = (lhs, rhs) {
            if name == "le" && args.len() == 2 && t == "true" {
                return Some((args[0].clone(), args[1].clone()));
            }
        }
    }
    None
}

/// An integer-literal operand, if this term is one.
fn as_int_literal(t: &ProofTerm) -> Option<i64> {
    match t {
        ProofTerm::Constant(s) => s.parse::<i64>().ok(),
        _ => None,
    }
}

/// Resolve a term through a substitution to a fixpoint, following chains
/// (`x → y → c`). A witness that is only transitively bound (the existential cut
/// leaves `a → _G9` while `_G9 → Q` lives elsewhere in the same substitution) is
/// thereby fully ground.
fn resolve_term(t: &ProofTerm, subst: &Substitution) -> ProofTerm {
    let mut cur = t.clone();
    for _ in 0..256 {
        let next = apply_subst_to_term(&cur, subst);
        if next == cur {
            break;
        }
        cur = next;
    }
    cur
}

/// A term with no free variables — a witness ready to instantiate a quantifier.
fn is_ground_term(t: &ProofTerm) -> bool {
    match t {
        ProofTerm::Variable(_) | ProofTerm::BoundVarRef(_) => false,
        ProofTerm::Constant(_) => true,
        ProofTerm::Function(_, args) | ProofTerm::Group(args) => args.iter().all(is_ground_term),
    }
}

/// The fully-resolved, GROUND witness for `var` under `subst`, or `None` if it is
/// unbound or only partially determined — in which case the instantiation must be
/// rejected rather than leak a dangling metavariable into the certificate.
fn ground_witness(var: &str, subst: &Substitution) -> Option<ProofTerm> {
    let resolved = resolve_term(subst.get(var)?, subst);
    is_ground_term(&resolved).then_some(resolved)
}

/// Collect the free-variable names of `t` in first-occurrence order (no dups),
/// for the loop-detection key — so two goals that differ only by which fresh
/// existential names they use produce the same canonical renaming.
fn collect_vars_ordered_term(t: &ProofTerm, acc: &mut Vec<String>) {
    match t {
        ProofTerm::Variable(v) => {
            if !acc.iter().any(|x| x == v) {
                acc.push(v.clone());
            }
        }
        ProofTerm::Function(_, args) | ProofTerm::Group(args) => {
            for a in args {
                collect_vars_ordered_term(a, acc);
            }
        }
        ProofTerm::Constant(_) | ProofTerm::BoundVarRef(_) => {}
    }
}

/// Collect the free-variable names of `e` in first-occurrence order (no dups).
/// Covers every `ProofExpr` variant so the canonical key never silently leaves a
/// variable un-renamed (which would weaken — never break — loop detection).
fn collect_vars_ordered_expr(e: &ProofExpr, acc: &mut Vec<String>) {
    match e {
        ProofExpr::Predicate { args, .. } => {
            for a in args {
                collect_vars_ordered_term(a, acc);
            }
        }
        ProofExpr::Identity(l, r) => {
            collect_vars_ordered_term(l, acc);
            collect_vars_ordered_term(r, acc);
        }
        ProofExpr::Term(t) => collect_vars_ordered_term(t, acc),
        ProofExpr::NeoEvent { roles, .. } => {
            for (_, t) in roles {
                collect_vars_ordered_term(t, acc);
            }
        }
        ProofExpr::Ctor { args, .. } => {
            for a in args {
                collect_vars_ordered_expr(a, acc);
            }
        }
        ProofExpr::Not(p)
        | ProofExpr::ForAll { body: p, .. }
        | ProofExpr::Exists { body: p, .. }
        | ProofExpr::Modal { body: p, .. }
        | ProofExpr::Temporal { body: p, .. }
        | ProofExpr::Lambda { body: p, .. }
        | ProofExpr::Fixpoint { body: p, .. } => collect_vars_ordered_expr(p, acc),
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r)
        | ProofExpr::App(l, r)
        | ProofExpr::TemporalBinary { left: l, right: r, .. } => {
            collect_vars_ordered_expr(l, acc);
            collect_vars_ordered_expr(r, acc);
        }
        ProofExpr::Counterfactual { antecedent, consequent } => {
            collect_vars_ordered_expr(antecedent, acc);
            collect_vars_ordered_expr(consequent, acc);
        }
        ProofExpr::Match { scrutinee, arms } => {
            collect_vars_ordered_expr(scrutinee, acc);
            for arm in arms {
                collect_vars_ordered_expr(&arm.body, acc);
            }
        }
        ProofExpr::Atom(_)
        | ProofExpr::TypedVar { .. }
        | ProofExpr::Hole(_)
        | ProofExpr::Unsupported(_) => {}
    }
}

/// Does `var` occur as a free variable anywhere in `expr`?
fn expr_mentions_var(expr: &ProofExpr, var: &str) -> bool {
    let mut vs = Vec::new();
    collect_vars_ordered_expr(expr, &mut vs);
    vs.iter().any(|v| v == var)
}

/// Witnesses for the `∀`-bound variables of an axiom being instantiated, in binder
/// order. Each is its pinned ground binding if it has one; a variable that does NOT
/// occur in `matrix` is *vacuous* — the matrix (and hence the goal) is independent of
/// it, so it is soundly instantiated with any in-scope entity (here a sibling's
/// witness). Returns `None` only when a variable is genuinely underdetermined:
/// unpinned yet occurring in the matrix, which would leak a metavariable into the
/// certificate.
fn instantiation_witnesses(
    bound_vars: &[String],
    subst: &Substitution,
    matrix: &ProofExpr,
) -> Option<Vec<ProofTerm>> {
    let fallback = bound_vars.iter().find_map(|v| ground_witness(v, subst));
    bound_vars
        .iter()
        .map(|v| {
            ground_witness(v, subst).or_else(|| {
                if expr_mentions_var(matrix, v) {
                    None
                } else {
                    fallback.clone()
                }
            })
        })
        .collect()
}

/// Rewrite every occurrence of the eigenconstant `eigen` back to `Variable(var)`
/// in a term — the generalization step of `∀`-introduction, undoing the opaque
/// substitution the search ran under so the certifier can abstract `λ(var). …`.
fn rewrite_const_to_var_term(t: &ProofTerm, eigen: &str, var: &str) -> ProofTerm {
    match t {
        ProofTerm::Constant(c) if c == eigen => ProofTerm::Variable(var.to_string()),
        ProofTerm::Function(n, args) => ProofTerm::Function(
            n.clone(),
            args.iter().map(|a| rewrite_const_to_var_term(a, eigen, var)).collect(),
        ),
        ProofTerm::Group(args) => {
            ProofTerm::Group(args.iter().map(|a| rewrite_const_to_var_term(a, eigen, var)).collect())
        }
        other => other.clone(),
    }
}

/// Rewrite `Constant(eigen)` back to `Variable(var)` throughout an expression.
fn rewrite_const_to_var_expr(e: &ProofExpr, eigen: &str, var: &str) -> ProofExpr {
    let re = |x: &ProofExpr| Box::new(rewrite_const_to_var_expr(x, eigen, var));
    let rt = |x: &ProofTerm| rewrite_const_to_var_term(x, eigen, var);
    match e {
        ProofExpr::Predicate { name, args, world } => ProofExpr::Predicate {
            name: name.clone(),
            args: args.iter().map(rt).collect(),
            world: world.clone(),
        },
        ProofExpr::Identity(l, r) => ProofExpr::Identity(rt(l), rt(r)),
        ProofExpr::Atom(s) => ProofExpr::Atom(s.clone()),
        ProofExpr::And(l, r) => ProofExpr::And(re(l), re(r)),
        ProofExpr::Or(l, r) => ProofExpr::Or(re(l), re(r)),
        ProofExpr::Implies(l, r) => ProofExpr::Implies(re(l), re(r)),
        ProofExpr::Iff(l, r) => ProofExpr::Iff(re(l), re(r)),
        ProofExpr::Not(p) => ProofExpr::Not(re(p)),
        ProofExpr::ForAll { variable, body } => {
            ProofExpr::ForAll { variable: variable.clone(), body: re(body) }
        }
        ProofExpr::Exists { variable, body } => {
            ProofExpr::Exists { variable: variable.clone(), body: re(body) }
        }
        ProofExpr::Modal { domain, force, flavor, body } => ProofExpr::Modal {
            domain: domain.clone(),
            force: *force,
            flavor: flavor.clone(),
            body: re(body),
        },
        ProofExpr::Counterfactual { antecedent, consequent } => ProofExpr::Counterfactual {
            antecedent: re(antecedent),
            consequent: re(consequent),
        },
        ProofExpr::Temporal { operator, body } => {
            ProofExpr::Temporal { operator: operator.clone(), body: re(body) }
        }
        ProofExpr::TemporalBinary { operator, left, right } => ProofExpr::TemporalBinary {
            operator: operator.clone(),
            left: re(left),
            right: re(right),
        },
        ProofExpr::Lambda { variable, body } => {
            ProofExpr::Lambda { variable: variable.clone(), body: re(body) }
        }
        ProofExpr::App(l, r) => ProofExpr::App(re(l), re(r)),
        ProofExpr::NeoEvent { event_var, verb, roles } => ProofExpr::NeoEvent {
            event_var: event_var.clone(),
            verb: verb.clone(),
            roles: roles.iter().map(|(role, t)| (role.clone(), rt(t))).collect(),
        },
        ProofExpr::Ctor { name, args } => ProofExpr::Ctor {
            name: name.clone(),
            args: args.iter().map(|a| rewrite_const_to_var_expr(a, eigen, var)).collect(),
        },
        ProofExpr::Match { scrutinee, arms } => ProofExpr::Match {
            scrutinee: re(scrutinee),
            arms: arms
                .iter()
                .map(|arm| crate::MatchArm {
                    ctor: arm.ctor.clone(),
                    bindings: arm.bindings.clone(),
                    body: rewrite_const_to_var_expr(&arm.body, eigen, var),
                })
                .collect(),
        },
        ProofExpr::Fixpoint { name, body } => {
            ProofExpr::Fixpoint { name: name.clone(), body: re(body) }
        }
        ProofExpr::TypedVar { name, typename } => {
            ProofExpr::TypedVar { name: name.clone(), typename: typename.clone() }
        }
        ProofExpr::Hole(s) => ProofExpr::Hole(s.clone()),
        ProofExpr::Term(t) => ProofExpr::Term(rt(t)),
        ProofExpr::Unsupported(s) => ProofExpr::Unsupported(s.clone()),
    }
}

/// Rewrite `Constant(eigen)` back to `Variable(var)` throughout a whole derivation
/// tree — its conclusions, the terms its rules carry (instantiation witnesses,
/// rewrite endpoints, case formulae), and the recorded substitutions. The
/// generalization that turns an eigenconstant body proof into a `∀`-abstraction.
fn rewrite_tree_const_to_var(tree: &DerivationTree, eigen: &str, var: &str) -> DerivationTree {
    let rule = match &tree.rule {
        InferenceRule::UniversalInst(s) if s == eigen => InferenceRule::UniversalInst(var.to_string()),
        InferenceRule::ExistentialIntro { witness, witness_type } if witness == eigen => {
            InferenceRule::ExistentialIntro {
                witness: var.to_string(),
                witness_type: witness_type.clone(),
            }
        }
        InferenceRule::ExistentialElim { witness } if witness == eigen => {
            InferenceRule::ExistentialElim { witness: var.to_string() }
        }
        InferenceRule::Rewrite { from, to } => InferenceRule::Rewrite {
            from: rewrite_const_to_var_term(from, eigen, var),
            to: rewrite_const_to_var_term(to, eigen, var),
        },
        InferenceRule::CaseAnalysis { case_formula } => InferenceRule::CaseAnalysis {
            case_formula: Box::new(rewrite_const_to_var_expr(case_formula, eigen, var)),
        },
        other => other.clone(),
    };
    DerivationTree {
        conclusion: rewrite_const_to_var_expr(&tree.conclusion, eigen, var),
        rule,
        premises: tree.premises.iter().map(|p| rewrite_tree_const_to_var(p, eigen, var)).collect(),
        depth: tree.depth,
        substitution: tree
            .substitution
            .iter()
            .map(|(k, v)| (k.clone(), rewrite_const_to_var_term(v, eigen, var)))
            .collect(),
    }
}

/// Project `target` out of a conjunctive hypothesis. Given `conj`, a derivation
/// whose conclusion is a (possibly nested) conjunction, return a derivation of the
/// matching conjunct — a chain of `ConjunctionElim` steps down to it — paired with
/// the substitution that unifies that conjunct with `target`. This is forward
/// ∧-elimination: it lets the search use a conjunct of an assumed `A ∧ B` as a
/// standalone fact (so the existential middle of a cut pins against it), while the
/// certifier independently re-derives the projection. Returns `None` if no conjunct
/// matches.
fn extract_conjunct(
    conj: &DerivationTree,
    target: &ProofExpr,
) -> Option<(DerivationTree, Substitution)> {
    if let ProofExpr::And(l, r) = &conj.conclusion {
        let left =
            DerivationTree::new((**l).clone(), InferenceRule::ConjunctionElim, vec![conj.clone()]);
        if let Some(found) = extract_conjunct(&left, target) {
            return Some(found);
        }
        let right =
            DerivationTree::new((**r).clone(), InferenceRule::ConjunctionElim, vec![conj.clone()]);
        extract_conjunct(&right, target)
    } else if let Ok(subst) = unify_exprs(target, &conj.conclusion) {
        Some((conj.clone(), subst))
    } else {
        None
    }
}

/// Prove `a ≤ b` by chaining the known `≤` facts. `adj` is the DIRECTED graph of
/// hypothesis inequalities (`≤` is not symmetric); a path `a → … → b` folds into a
/// left-nested `le_trans`, and `a = b` closes by `le_refl`. Every node certifies.
fn cert_le_path(
    a: &ProofTerm,
    b: &ProofTerm,
    adj: &std::collections::HashMap<String, Vec<(ProofTerm, DerivationTree)>>,
) -> Option<DerivationTree> {
    let a_key = term_skey(a)?;
    let b_key = term_skey(b)?;
    if a_key == b_key {
        return Some(DerivationTree::leaf(
            le_eq(a.clone(), a.clone()),
            InferenceRule::LeRefl,
        ));
    }
    let mut queue: std::collections::VecDeque<(String, Option<DerivationTree>)> =
        std::collections::VecDeque::new();
    queue.push_back((a_key.clone(), None));
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    visited.insert(a_key);
    while let Some((cur_key, acc)) = queue.pop_front() {
        let Some(edges) = adj.get(&cur_key) else {
            continue;
        };
        for (next_term, edge_tree) in edges {
            let Some(next_key) = term_skey(next_term) else {
                continue;
            };
            if visited.contains(&next_key) {
                continue;
            }
            visited.insert(next_key.clone());
            // `acc` proves `a ≤ cur`; `edge_tree` proves `cur ≤ next`.
            let step = match &acc {
                None => edge_tree.clone(), // cur == a: the edge already proves a ≤ next
                Some(acc_proof) => DerivationTree::new(
                    le_eq(a.clone(), next_term.clone()),
                    InferenceRule::LeTrans,
                    vec![acc_proof.clone(), edge_tree.clone()],
                ),
            };
            if next_key == b_key {
                return Some(step);
            }
            queue.push_back((next_key, Some(step)));
        }
    }
    None
}

/// Detect a linear contradiction in the known set: a chain of `≤` facts proving
/// `le(m, n) = true` for ground `m > n` (impossible — `le m n ⇝ false`). Returns a
/// `⊥` derivation through the Bool no-confusion discriminator, so contradictory
/// linear bounds (e.g. `x ≤ 3` with `5 ≤ x`) close any goal by ex falso.
fn cert_linarith_close(known: &[(ProofExpr, DerivationTree)]) -> Option<DerivationTree> {
    use std::collections::HashMap;
    let mut adj: HashMap<String, Vec<(ProofTerm, DerivationTree)>> = HashMap::new();
    let mut grounds: Vec<(i64, ProofTerm)> = Vec::new();
    for (prop, tree) in known {
        if let Some((x, y)) = as_le_pair(prop) {
            if let Some(xk) = term_skey(&x) {
                adj.entry(xk).or_default().push((y.clone(), tree.clone()));
            }
            for t in [&x, &y] {
                if let Some(v) = as_int_literal(t) {
                    grounds.push((v, t.clone()));
                }
            }
        }
    }
    if adj.is_empty() {
        return None;
    }
    for (mv, m) in &grounds {
        for (nv, n) in &grounds {
            if mv > nv {
                if let Some(le_proof) = cert_le_path(m, n, &adj) {
                    return Some(DerivationTree::new(
                        ProofExpr::Atom("⊥".into()),
                        InferenceRule::LinFalse,
                        vec![le_proof],
                    ));
                }
            }
        }
    }
    None
}

/// General linear refutation (Farkas). Decide unsatisfiability of the `≤`
/// hypotheses with Fourier-Motzkin ([`crate::linarith_solve::find_farkas`]), then
/// RECONSTRUCT a kernel proof from the non-negative multipliers `λᵢ`: each `lᵢ ≤ rᵢ`
/// becomes `0 ≤ rᵢ - lᵢ` (`le_sub`), scaled by `λᵢ` (`le_mul_nonneg`), summed
/// (`le_add_mono`) into `le(BigL, BigR)`; the variables cancel, so `BigL` ring-reduces
/// to `0` and `BigR` to `-d` (`d > 0`), giving the ground-false `le(0, -d)` that the
/// discriminator turns into `⊥`. Handles arbitrary coefficients (where the chain
/// `cert_linarith_close` cannot).
pub(crate) fn cert_farkas(known: &[(ProofExpr, DerivationTree)]) -> Option<DerivationTree> {
    use crate::linarith_solve::{combine, find_farkas, parse_lin};

    let mut le_hyps: Vec<(ProofTerm, ProofTerm, DerivationTree)> = Vec::new();
    let mut constraints = Vec::new();
    for (prop, tree) in known {
        if let Some((l, r)) = as_le_pair(prop) {
            if let (Some(ll), Some(rl)) = (parse_lin(&l), parse_lin(&r)) {
                constraints.push(ll.sub(&rl)); // l - r ≤ 0
                le_hyps.push((l, r, tree.clone()));
            }
        }
    }
    if le_hyps.is_empty() {
        return None;
    }
    let multipliers = find_farkas(&constraints)?;
    let combo = combine(&constraints, &multipliers);
    if !combo.is_const() || combo.constant <= 0 {
        return None;
    }
    let d = combo.constant; // Σ λᵢ(lᵢ - rᵢ) = d > 0

    let pc = |n: i64| ProofTerm::Constant(n.to_string());
    let pf = |op: &str, x: ProofTerm, y: ProofTerm| ProofTerm::Function(op.to_string(), vec![x, y]);

    // Per active hypothesis: 0 ≤ rᵢ - lᵢ (le_sub), scaled by λᵢ (le_mul_nonneg).
    let mut scaled: Vec<DerivationTree> = Vec::new();
    for (&i, &lam) in &multipliers {
        if lam <= 0 {
            continue;
        }
        let (l, r, hyp_tree) = &le_hyps[i];
        // r + (-1)·l  — the `le_sub` form (sub-free, ring-oracle friendly).
        let diff = pf("add", r.clone(), pf("mul", pc(-1), l.clone()));
        let sub_i = DerivationTree::new(
            le_eq(pc(0), diff.clone()),
            InferenceRule::LeSub,
            vec![hyp_tree.clone()],
        );
        let ground = DerivationTree::leaf(le_eq(pc(0), pc(lam)), InferenceRule::Reflexivity);
        scaled.push(DerivationTree::new(
            le_eq(pf("mul", pc(lam), pc(0)), pf("mul", pc(lam), diff)),
            InferenceRule::LeMulNonneg,
            vec![ground, sub_i],
        ));
    }
    // Sum them: le(BigL, BigR).
    let summed = scaled.into_iter().reduce(|acc, s| {
        let (al, ar) = as_le_pair(&acc.conclusion).expect("scaled is an le-fact");
        let (sl, sr) = as_le_pair(&s.conclusion).expect("scaled is an le-fact");
        DerivationTree::new(
            le_eq(pf("add", al, sl), pf("add", ar, sr)),
            InferenceRule::LeAddMono,
            vec![acc, s],
        )
    })?;
    let (big_l, big_r) = as_le_pair(&summed.conclusion)?;

    // Ring-normalize: rewrite BigR → -d, then BigL → 0, giving the ground `le(0, -d)`.
    let rw1 = DerivationTree::new(
        le_eq(big_l.clone(), pc(-d)),
        InferenceRule::Rewrite { from: big_r.clone(), to: pc(-d) },
        vec![
            DerivationTree::leaf(
                ProofExpr::Identity(big_r, pc(-d)),
                InferenceRule::ArithDecision,
            ),
            summed,
        ],
    );
    let rw2 = DerivationTree::new(
        le_eq(pc(0), pc(-d)),
        InferenceRule::Rewrite { from: big_l.clone(), to: pc(0) },
        vec![
            DerivationTree::leaf(
                ProofExpr::Identity(big_l, pc(0)),
                InferenceRule::ArithDecision,
            ),
            rw1,
        ],
    );
    Some(DerivationTree::new(
        ProofExpr::Atom("⊥".into()),
        InferenceRule::LinFalse,
        vec![rw2],
    ))
}

/// Close a `P` / `¬P` pair in the known set into a `Contradiction` (⊥) node.
fn cert_close(known: &[(ProofExpr, DerivationTree)]) -> Option<DerivationTree> {
    for (prop, neg_tree) in known {
        if let ProofExpr::Not(inner) = prop {
            for (other, pos_tree) in known {
                if exprs_structurally_equal(other, inner) {
                    return Some(DerivationTree::new(
                        ProofExpr::Atom("⊥".into()),
                        InferenceRule::Contradiction,
                        vec![pos_tree.clone(), neg_tree.clone()],
                    ));
                }
            }
        }
    }
    None
}

/// The atomic sub-formulas of a (possibly conjunctive / negated) proposition.
fn cert_atoms_of(expr: &ProofExpr) -> Vec<ProofExpr> {
    match expr {
        ProofExpr::And(l, r) => {
            let mut v = cert_atoms_of(l);
            v.extend(cert_atoms_of(r));
            v
        }
        ProofExpr::Not(inner) => cert_atoms_of(inner),
        other => vec![other.clone()],
    }
}

/// Whether a proposition mentions no free variables in its predicate arguments.
fn cert_is_ground(expr: &ProofExpr) -> bool {
    fn term_ground(t: &ProofTerm) -> bool {
        match t {
            ProofTerm::Variable(_) | ProofTerm::BoundVarRef(_) => false,
            ProofTerm::Function(_, args) | ProofTerm::Group(args) => args.iter().all(term_ground),
            ProofTerm::Constant(_) => true,
        }
    }
    match expr {
        ProofExpr::Predicate { args, .. } => args.iter().all(term_ground),
        ProofExpr::Not(inner) => cert_is_ground(inner),
        ProofExpr::And(l, r) | ProofExpr::Or(l, r) | ProofExpr::Implies(l, r) => {
            cert_is_ground(l) && cert_is_ground(r)
        }
        ProofExpr::Atom(_) => true,
        _ => false,
    }
}

/// Collect constant names appearing in predicate arguments of known facts.
fn cert_collect_constants(known: &[(ProofExpr, DerivationTree)]) -> Vec<String> {
    fn from_term(t: &ProofTerm, out: &mut Vec<String>) {
        match t {
            ProofTerm::Constant(c) => {
                if !out.contains(c) {
                    out.push(c.clone());
                }
            }
            ProofTerm::Function(_, args) | ProofTerm::Group(args) => {
                for a in args {
                    from_term(a, out);
                }
            }
            _ => {}
        }
    }
    fn from_expr(e: &ProofExpr, out: &mut Vec<String>) {
        match e {
            ProofExpr::Predicate { args, .. } => {
                for a in args {
                    from_term(a, out);
                }
            }
            ProofExpr::Not(i) => from_expr(i, out),
            ProofExpr::And(l, r) | ProofExpr::Or(l, r) | ProofExpr::Implies(l, r) => {
                from_expr(l, out);
                from_expr(r, out);
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (p, _) in known {
        from_expr(p, &mut out);
    }
    out
}

/// Candidate substitutions under which a rule may fire: each binds the rule's
/// `∀`-variable to a constant drawn from a known fact that matches one of the
/// antecedent's atoms.
fn cert_rule_substs(rule: &CertRule, known: &[(ProofExpr, DerivationTree)]) -> Vec<Substitution> {
    match &rule.binder {
        None => vec![Substitution::new()],
        Some(v) => {
            let mut substs: Vec<Substitution> = Vec::new();
            for atom in cert_atoms_of(&rule.ante) {
                for (fact, _) in known {
                    if let Ok(s) = unify_exprs(fact, &atom) {
                        if s.contains_key(v) && !substs.iter().any(|e| e == &s) {
                            substs.push(s);
                        }
                    }
                }
            }
            substs
        }
    }
}

/// Build a gapless proof of `ante` (under `subst`) from the known facts,
/// introducing `∧` via `ConjunctionIntro`. Returns `None` if any conjunct is
/// not derivable.
fn cert_prove_ante(
    ante: &ProofExpr,
    subst: &Substitution,
    known: &[(ProofExpr, DerivationTree)],
) -> Option<DerivationTree> {
    match ante {
        ProofExpr::And(l, r) => {
            let lt = cert_prove_ante(l, subst, known)?;
            let rt = cert_prove_ante(r, subst, known)?;
            let li = apply_subst_to_expr(l, subst);
            let ri = apply_subst_to_expr(r, subst);
            Some(DerivationTree::new(
                ProofExpr::And(Box::new(li), Box::new(ri)),
                InferenceRule::ConjunctionIntro,
                vec![lt, rt],
            ))
        }
        _ => {
            let goal = apply_subst_to_expr(ante, subst);
            known
                .iter()
                .find(|(p, _)| exprs_structurally_equal(p, &goal))
                .map(|(_, t)| t.clone())
        }
    }
}

/// The negation of a proposition, collapsing double negation: `¬¬P ↦ P`.
fn cert_neg(e: &ProofExpr) -> ProofExpr {
    match e {
        ProofExpr::Not(inner) => (**inner).clone(),
        _ => ProofExpr::Not(Box::new(e.clone())),
    }
}

/// A proof of `target` from the known set, by exact structural match.
fn cert_lookup(
    known: &[(ProofExpr, DerivationTree)],
    target: &ProofExpr,
) -> Option<DerivationTree> {
    known
        .iter()
        .find(|(p, _)| exprs_structurally_equal(p, target))
        .map(|(_, t)| t.clone())
}

/// The proof of the implication `rule.ante → cons_inst` at this instance — a bare
/// `PremiseMatch` for a ground rule, or `UniversalInst` of the source `∀` rule.
fn cert_rule_impl_tree(
    rule: &CertRule,
    subst: &Substitution,
    cons_inst: &ProofExpr,
) -> Option<DerivationTree> {
    match &rule.binder {
        None => Some(DerivationTree::leaf(
            rule.source.clone(),
            InferenceRule::PremiseMatch,
        )),
        Some(v) => {
            let witness = match subst.get(v) {
                Some(ProofTerm::Constant(c)) | Some(ProofTerm::Variable(c)) => c.clone(),
                _ => return None,
            };
            let ante_inst = apply_subst_to_expr(&rule.ante, subst);
            Some(DerivationTree::new(
                ProofExpr::Implies(Box::new(ante_inst), Box::new(cons_inst.clone())),
                InferenceRule::UniversalInst(witness),
                vec![DerivationTree::leaf(
                    rule.source.clone(),
                    InferenceRule::PremiseMatch,
                )],
            ))
        }
    }
}

/// n-ary DISJUNCTIVE SYLLOGISM (the heart of unit propagation): peel every refuted
/// disjunct off a known disjunction by certified `DisjunctionElim`, returning the
/// surviving sub-formula (a single literal once all but one is refuted). `None` if no
/// disjunct is refuted (no progress).
/// Prove `¬d` when the disjunct `d` is REFUTED by the known set — not only when
/// `¬d` is itself known, but when a CONJUNCT of `d` is false (so the whole
/// conjunction is), or `d` is a negation of a known fact. This is what lets
/// disjunctive syllogism collapse an of-pair clue: its disjuncts are conjunctions
/// (`Hunt(x) ∧ In(y,2004) ∧ …`), and a single false conjunct kills the disjunct.
/// Every returned tree is certified (`ConjunctionElim` / `Contradiction` / reductio).
fn cert_refute(d: &ProofExpr, known: &[(ProofExpr, DerivationTree)]) -> Option<DerivationTree> {
    // Directly known negation.
    if let Some(t) = cert_lookup(known, &cert_neg(d)) {
        return Some(t);
    }
    match d {
        // A conjunction is refuted if either conjunct is — assume the conjunction,
        // ∧-eliminate the false conjunct, contradict its refutation, reductio.
        ProofExpr::And(l, r) => {
            for (side, _other_first) in [(l.as_ref(), true), (r.as_ref(), false)] {
                if let Some(neg_side) = cert_refute(side, known) {
                    let assume = DerivationTree::leaf(d.clone(), InferenceRule::PremiseMatch);
                    let elim = DerivationTree::new(
                        side.clone(),
                        InferenceRule::ConjunctionElim,
                        vec![assume.clone()],
                    );
                    let contra = DerivationTree::new(
                        ProofExpr::Atom("⊥".into()),
                        InferenceRule::Contradiction,
                        vec![elim, neg_side],
                    );
                    return Some(DerivationTree::new(
                        cert_neg(d),
                        InferenceRule::ReductioAdAbsurdum,
                        vec![assume, contra],
                    ));
                }
            }
            None
        }
        // `¬x` is refuted when `x` is known — assume `¬x`, contradict `x`, reductio.
        ProofExpr::Not(x) => {
            let x_tree = cert_lookup(known, x)?;
            let assume = DerivationTree::leaf(d.clone(), InferenceRule::PremiseMatch);
            let contra = DerivationTree::new(
                ProofExpr::Atom("⊥".into()),
                InferenceRule::Contradiction,
                vec![x_tree, assume.clone()],
            );
            Some(DerivationTree::new(
                cert_neg(d),
                InferenceRule::ReductioAdAbsurdum,
                vec![assume, contra],
            ))
        }
        _ => None,
    }
}

fn cert_peel_disjunction(
    or_tree: &DerivationTree,
    known: &[(ProofExpr, DerivationTree)],
) -> Option<(ProofExpr, DerivationTree)> {
    let ProofExpr::Or(l, r) = &or_tree.conclusion else {
        return None;
    };
    let (l, r) = ((**l).clone(), (**r).clone());
    if let Some(neg_l) = cert_refute(&l, known) {
        let elim = DerivationTree::new(
            r.clone(),
            InferenceRule::DisjunctionElim,
            vec![or_tree.clone(), neg_l],
        );
        return Some(cert_peel_disjunction(&elim, known).unwrap_or((r, elim)));
    }
    if let Some(neg_r) = cert_refute(&r, known) {
        let elim = DerivationTree::new(
            l.clone(),
            InferenceRule::DisjunctionElim,
            vec![or_tree.clone(), neg_r],
        );
        return Some(cert_peel_disjunction(&elim, known).unwrap_or((l, elim)));
    }
    None
}

/// Forward UNIT PROPAGATION to a fixpoint — the DPLL inner loop, every step a
/// certified inference. Four rules drive a grounded logic grid without any
/// case-splitting:
///   • **ModusPonens** — a rule whose antecedent is satisfied fires its consequent.
///   • **ModusTollens** — a rule whose consequent is refuted refutes its antecedent
///     (via reductio: assume the antecedent, derive the consequent, contradict).
///   • **Disjunctive syllogism** — a disjunction with all-but-one disjunct refuted
///     forces the survivor ([`cert_peel_disjunction`]). This collapses domain
///     closures (`In(t,A) ∨ … ∨ In(t,D)`) as the row fills, with NO branching.
///   • **Conjunction-negation** — `¬(A ∧ B)` with `A` established refutes `B` (via
///     reductio). This is how "exactly one in Florida" (an at-most-one rule, refuted
///     by `ModusTollens`) excludes Florida from every other row.
/// Together these are the propagation a logic-grid solver runs; case analysis
/// ([`cert_derive_falsum`]) is only the rare residual decision.
fn cert_saturate(
    rules: &[CertRule],
    mut known: Vec<(ProofExpr, DerivationTree)>,
) -> Vec<(ProofExpr, DerivationTree)> {
    let max_rounds = 64;
    let mut push_new =
        |known: &mut Vec<(ProofExpr, DerivationTree)>, fact: ProofExpr, tree: DerivationTree| -> bool {
            if known.iter().any(|(p, _)| exprs_structurally_equal(p, &fact)) {
                false
            } else {
                known.push((fact, tree));
                true
            }
        };
    for _ in 0..max_rounds {
        let snapshot = known.clone();
        let mut added = false;

        // (1) ModusPonens — fire a rule whose antecedent holds.
        for rule in rules {
            for subst in cert_rule_substs(rule, &snapshot) {
                let cons_inst = apply_subst_to_expr(&rule.cons, &subst);
                if known.iter().any(|(p, _)| exprs_structurally_equal(p, &cons_inst)) {
                    continue;
                }
                let Some(ante_proof) = cert_prove_ante(&rule.ante, &subst, &snapshot) else {
                    continue;
                };
                let Some(impl_tree) = cert_rule_impl_tree(rule, &subst, &cons_inst) else {
                    continue;
                };
                let mp = DerivationTree::new(
                    cons_inst.clone(),
                    InferenceRule::ModusPonens,
                    vec![impl_tree, ante_proof],
                );
                added |= push_new(&mut known, cons_inst, mp);
            }
        }

        // (2) ModusTollens — a ground rule whose consequent is refuted refutes its
        // antecedent. Built from certified primitives: assume the antecedent, derive
        // the consequent by ModusPonens, contradict the known negation, reductio.
        for rule in rules {
            if rule.binder.is_some() {
                continue;
            }
            let neg_cons = cert_neg(&rule.cons);
            let Some(nc_tree) = cert_lookup(&snapshot, &neg_cons) else {
                continue;
            };
            let neg_ante = cert_neg(&rule.ante);
            if cert_lookup(&known, &neg_ante).is_some() {
                continue;
            }
            let assume = DerivationTree::leaf(rule.ante.clone(), InferenceRule::PremiseMatch);
            let rule_tree =
                DerivationTree::leaf(rule.source.clone(), InferenceRule::PremiseMatch);
            let mp = DerivationTree::new(
                rule.cons.clone(),
                InferenceRule::ModusPonens,
                vec![rule_tree, assume.clone()],
            );
            let contra = DerivationTree::new(
                ProofExpr::Atom("⊥".into()),
                InferenceRule::Contradiction,
                vec![mp, nc_tree],
            );
            let tree = DerivationTree::new(
                neg_ante.clone(),
                InferenceRule::ReductioAdAbsurdum,
                vec![assume, contra],
            );
            added |= push_new(&mut known, neg_ante, tree);
        }

        // (3) Disjunctive syllogism — peel refuted disjuncts off known disjunctions.
        for (p, or_tree) in &snapshot {
            if !matches!(p, ProofExpr::Or(..)) {
                continue;
            }
            if let Some((lit, tree)) = cert_peel_disjunction(or_tree, &snapshot) {
                added |= push_new(&mut known, lit, tree);
            }
        }

        // (4) Conjunction-negation — `¬(A ∧ B)` with `A` established refutes `B`
        // (and symmetrically). Built by reductio: assume B, ∧-introduce A∧B,
        // contradict ¬(A∧B).
        for (p, neg_tree) in &snapshot {
            let ProofExpr::Not(inner) = p else { continue };
            let ProofExpr::And(a, b) = inner.as_ref() else {
                continue;
            };
            for (known_is_left, known_side, other) in
                [(true, a.as_ref(), b.as_ref()), (false, b.as_ref(), a.as_ref())]
            {
                let Some(side_proof) = cert_prove_ante(known_side, &Substitution::new(), &snapshot)
                else {
                    continue;
                };
                let neg_other = cert_neg(other);
                if cert_lookup(&known, &neg_other).is_some() {
                    continue;
                }
                let assume = DerivationTree::leaf(other.clone(), InferenceRule::PremiseMatch);
                let (l_proof, r_proof) = if known_is_left {
                    (side_proof.clone(), assume.clone())
                } else {
                    (assume.clone(), side_proof.clone())
                };
                let conj_intro = DerivationTree::new(
                    (**inner).clone(),
                    InferenceRule::ConjunctionIntro,
                    vec![l_proof, r_proof],
                );
                let contra = DerivationTree::new(
                    ProofExpr::Atom("⊥".into()),
                    InferenceRule::Contradiction,
                    vec![conj_intro, neg_tree.clone()],
                );
                let tree = DerivationTree::new(
                    neg_other.clone(),
                    InferenceRule::ReductioAdAbsurdum,
                    vec![assume, contra],
                );
                added |= push_new(&mut known, neg_other, tree);
            }
        }

        // (5) Conjunction decomposition — a known `A ∧ B` yields `A` and `B`
        // (`ConjunctionElim`). When disjunctive syllogism leaves one surviving
        // of-pair disjunct (a conjunction), this surfaces its inner either/or as a
        // top-level disjunction the case-split can then decide.
        for (p, and_tree) in &snapshot {
            let ProofExpr::And(a, b) = p else { continue };
            for side in [a.as_ref(), b.as_ref()] {
                let elim = DerivationTree::new(
                    side.clone(),
                    InferenceRule::ConjunctionElim,
                    vec![and_tree.clone()],
                );
                added |= push_new(&mut known, side.clone(), elim);
            }
        }

        if !added {
            break;
        }
    }
    known
}

/// Self-referential pivots to case-split on: rule consequent/antecedent atoms
/// instantiated at known constants (ground, with negations stripped).
fn cert_candidates(known: &[(ProofExpr, DerivationTree)], rules: &[CertRule]) -> Vec<ProofExpr> {
    let consts = cert_collect_constants(known);
    let mut cands: Vec<ProofExpr> = Vec::new();
    let mut push = |c: ProofExpr, cands: &mut Vec<ProofExpr>| {
        if cert_is_ground(&c) && !cands.iter().any(|e| exprs_structurally_equal(e, &c)) {
            cands.push(c);
        }
    };
    for rule in rules {
        let atoms: Vec<ProofExpr> = cert_atoms_of(&rule.cons)
            .into_iter()
            .chain(cert_atoms_of(&rule.ante))
            .collect();
        match &rule.binder {
            None => {
                for a in atoms {
                    push(a, &mut cands);
                }
            }
            Some(v) => {
                for k in &consts {
                    let mut s = Substitution::new();
                    s.insert(v.clone(), ProofTerm::Constant(k.clone()));
                    for a in &atoms {
                        push(apply_subst_to_expr(a, &s), &mut cands);
                    }
                }
            }
        }
    }
    cands
}

/// Derive ⊥ from `seed` facts: saturate, then (bounded) case-split. Every node
/// of the returned tree is certifiable.
fn cert_derive_falsum(
    rules: &[CertRule],
    seed: &[(ProofExpr, DerivationTree)],
    depth: usize,
    fresh: &mut u32,
) -> Option<DerivationTree> {
    // First: eliminate an existential premise by introducing a fresh witness.
    // `∃x.P(x)` becomes `P(c)` for a fresh constant `c`, and the whole ⊥
    // derivation is wrapped in `ExistentialElim` (a `Match` on `Ex`).
    if let Some((idx, exist_tree)) = seed.iter().enumerate().find_map(|(i, (p, t))| {
        if matches!(p, ProofExpr::Exists { .. }) {
            Some((i, t.clone()))
        } else {
            None
        }
    }) {
        let (variable, body) = match &seed[idx].0 {
            ProofExpr::Exists { variable, body } => (variable.clone(), body.as_ref().clone()),
            _ => unreachable!(),
        };
        let c = format!("__sk{}", *fresh);
        *fresh += 1;
        let mut subst = Substitution::new();
        subst.insert(variable, ProofTerm::Constant(c.clone()));
        let p_c = apply_subst_to_expr(&body, &subst);

        let mut seed2: Vec<(ProofExpr, DerivationTree)> = seed
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != idx)
            .map(|(_, x)| x.clone())
            .collect();
        seed2.push((
            p_c.clone(),
            DerivationTree::leaf(p_c, InferenceRule::PremiseMatch),
        ));

        return cert_derive_falsum(rules, &seed2, depth, fresh).map(|body_proof| {
            DerivationTree::new(
                ProofExpr::Atom("⊥".into()),
                InferenceRule::ExistentialElim { witness: c },
                vec![exist_tree, body_proof],
            )
        });
    }

    let known = cert_saturate(rules, seed.to_vec());
    if let Some(t) = cert_close(&known) {
        return Some(t);
    }
    if let Some(t) = cert_equality_close(&known) {
        return Some(t);
    }
    if let Some(t) = cert_linarith_close(&known) {
        return Some(t);
    }
    if let Some(t) = cert_farkas(&known) {
        return Some(t);
    }
    // Integer discreteness: tighten strict `<` hypotheses to `a+1 ≤ b` and retry
    // the Farkas refutation — catches the strict contradictions rational
    // Fourier-Motzkin reports satisfiable (`x < y ∧ y < x+1`).
    if let Some(t) = crate::omega_solve::omega_close(&known) {
        return Some(t);
    }
    if depth == 0 {
        return None;
    }
    // Case-split on a disjunction — the DPLL *decision*, taken only after unit
    // propagation (`cert_saturate`) is exhausted. From `A ∨ B` derive ⊥ in BOTH the
    // (+A) and (+B) branches; each branch assumes its disjunct's conjuncts, which the
    // `DisjunctionCases` certifier binds as local hypotheses.
    //
    // PRODUCTIVITY GATE (not a structural cap): split a disjunction only when assuming
    // SOME disjunct IMMEDIATELY closes — saturates to ⊥ at depth 0, with no further
    // splitting. That is the signature of a genuine case decision (an of-pair /
    // either-or clue, where one arm is already refuted by what's known). A domain
    // CLOSURE has no such disjunct — a row may legitimately sit in any value — so it is
    // only ever propagated, never split. This is what stops the search from exploding
    // over a grid's many closures while still deciding every real clue branch.
    let known_has = |x: &ProofExpr, k: &[(ProofExpr, DerivationTree)]| {
        k.iter().any(|(q, _)| exprs_structurally_equal(q, x))
    };
    let seed_with = |d: &ProofExpr| -> Vec<(ProofExpr, DerivationTree)> {
        let mut s = seed.to_vec();
        for conj in flatten_conjuncts(d) {
            if !s.iter().any(|(q, _)| exprs_structurally_equal(q, &conj)) {
                s.push((conj.clone(), DerivationTree::leaf(conj, InferenceRule::PremiseMatch)));
            }
        }
        s
    };
    let established = |x: &ProofExpr, k: &[(ProofExpr, DerivationTree)]| {
        flatten_conjuncts(x).iter().all(|c| known_has(c, k))
    };
    // DECISION (DPLL): GUESS one undetermined disjunction and require BOTH branches to
    // close. Strong in-saturation propagation (disjunctive syllogism, at-most-one
    // exclusion, conjunction-negation) prunes each branch; a forced disjunction closes
    // one branch immediately. ONE decision per level keeps the tree finite-domain
    // bounded. By DPLL completeness, if premises + ¬goal are unsatisfiable then any
    // decision's branches both close; if this one's do not, the set is satisfiable and
    // the goal is not entailed (so don't fall through to the atom split).
    if let Some((p, tree_or)) = known.iter().find_map(|(p, t)| match p {
        ProofExpr::Or(a, b) if !(established(a, &known) || established(b, &known)) => {
            Some((p.clone(), t.clone()))
        }
        _ => None,
    }) {
        let ProofExpr::Or(a, b) = &p else { unreachable!() };
        if let (Some(pa), Some(pb)) = (
            cert_derive_falsum(rules, &seed_with(a), depth - 1, fresh),
            cert_derive_falsum(rules, &seed_with(b), depth - 1, fresh),
        ) {
            return Some(DerivationTree::new(
                ProofExpr::Atom("⊥".into()),
                InferenceRule::DisjunctionCases,
                vec![tree_or, pa, pb],
            ));
        }
        // A disjunction was available to branch on; by DPLL completeness its failure
        // to close means the set is satisfiable. Don't fall through to the atom split
        // (that is for disjunction-free paradoxes and would re-explore the grid).
        return None;
    }
    for c in cert_candidates(&known, rules) {
        let not_c = ProofExpr::Not(Box::new(c.clone()));
        // Skip pivots already settled — splitting on them cannot help.
        if known
            .iter()
            .any(|(p, _)| exprs_structurally_equal(p, &c) || exprs_structurally_equal(p, &not_c))
        {
            continue;
        }
        let mut seed_pos = seed.to_vec();
        seed_pos.push((c.clone(), DerivationTree::leaf(c.clone(), InferenceRule::PremiseMatch)));
        let mut seed_neg = seed.to_vec();
        seed_neg.push((
            not_c.clone(),
            DerivationTree::leaf(not_c.clone(), InferenceRule::PremiseMatch),
        ));
        if let (Some(p), Some(n)) = (
            cert_derive_falsum(rules, &seed_pos, depth - 1, fresh),
            cert_derive_falsum(rules, &seed_neg, depth - 1, fresh),
        ) {
            return Some(DerivationTree::new(
                ProofExpr::Atom("⊥".into()),
                InferenceRule::CaseAnalysis {
                    case_formula: Box::new(c.clone()),
                },
                vec![p, n],
            ));
        }
    }
    None
}

/// Check if two expressions are structurally equal.
///
/// This is syntactic equality after normalization - no unification needed.
fn exprs_structurally_equal(left: &ProofExpr, right: &ProofExpr) -> bool {
    match (left, right) {
        (ProofExpr::Atom(a), ProofExpr::Atom(b)) => a == b,

        (ProofExpr::Ctor { name: n1, args: a1 }, ProofExpr::Ctor { name: n2, args: a2 }) => {
            n1 == n2 && a1.len() == a2.len() && a1.iter().zip(a2).all(|(x, y)| exprs_structurally_equal(x, y))
        }

        (
            ProofExpr::Predicate { name: n1, args: a1, .. },
            ProofExpr::Predicate { name: n2, args: a2, .. },
        ) => n1 == n2 && a1.len() == a2.len() && a1.iter().zip(a2).all(|(x, y)| terms_structurally_equal(x, y)),

        (ProofExpr::Identity(l1, r1), ProofExpr::Identity(l2, r2)) => {
            terms_structurally_equal(l1, l2) && terms_structurally_equal(r1, r2)
        }

        (ProofExpr::And(l1, r1), ProofExpr::And(l2, r2))
        | (ProofExpr::Or(l1, r1), ProofExpr::Or(l2, r2))
        | (ProofExpr::Implies(l1, r1), ProofExpr::Implies(l2, r2))
        | (ProofExpr::Iff(l1, r1), ProofExpr::Iff(l2, r2)) => {
            exprs_structurally_equal(l1, l2) && exprs_structurally_equal(r1, r2)
        }

        (ProofExpr::Not(a), ProofExpr::Not(b)) => exprs_structurally_equal(a, b),

        (
            ProofExpr::ForAll { variable: v1, body: b1 },
            ProofExpr::ForAll { variable: v2, body: b2 },
        )
        | (
            ProofExpr::Exists { variable: v1, body: b1 },
            ProofExpr::Exists { variable: v2, body: b2 },
        ) => v1 == v2 && exprs_structurally_equal(b1, b2),

        (
            ProofExpr::Lambda { variable: v1, body: b1 },
            ProofExpr::Lambda { variable: v2, body: b2 },
        ) => v1 == v2 && exprs_structurally_equal(b1, b2),

        (ProofExpr::App(f1, a1), ProofExpr::App(f2, a2)) => {
            exprs_structurally_equal(f1, f2) && exprs_structurally_equal(a1, a2)
        }

        (
            ProofExpr::TypedVar { name: n1, typename: t1 },
            ProofExpr::TypedVar { name: n2, typename: t2 },
        ) => n1 == n2 && t1 == t2,

        (
            ProofExpr::Fixpoint { name: n1, body: b1 },
            ProofExpr::Fixpoint { name: n2, body: b2 },
        ) => n1 == n2 && exprs_structurally_equal(b1, b2),

        _ => false,
    }
}

/// Check if two terms are structurally equal.
fn terms_structurally_equal(left: &ProofTerm, right: &ProofTerm) -> bool {
    match (left, right) {
        (ProofTerm::Constant(a), ProofTerm::Constant(b)) => a == b,
        (ProofTerm::Variable(a), ProofTerm::Variable(b)) => a == b,
        (ProofTerm::BoundVarRef(a), ProofTerm::BoundVarRef(b)) => a == b,
        (ProofTerm::Function(n1, a1), ProofTerm::Function(n2, a2)) => {
            n1 == n2 && a1.len() == a2.len() && a1.iter().zip(a2).all(|(x, y)| terms_structurally_equal(x, y))
        }
        (ProofTerm::Group(a1), ProofTerm::Group(a2)) => {
            a1.len() == a2.len() && a1.iter().zip(a2).all(|(x, y)| terms_structurally_equal(x, y))
        }
        _ => false,
    }
}

impl BackwardChainer {
    /// Create a new proof engine with empty knowledge base.
    pub fn new() -> Self {
        Self {
            knowledge_base: Vec::new(),
            max_depth: DEFAULT_MAX_DEPTH,
            var_counter: 0,
            eliminated_existentials: Vec::new(),
            active_goals: Vec::new(),
            steps: 0,
            step_budget: DEFAULT_STEP_BUDGET,
        }
    }

    /// Set the maximum proof search depth.
    pub fn set_max_depth(&mut self, depth: usize) {
        self.max_depth = depth;
    }

    /// Set the node budget — the number of search nodes allowed for one top-level proof
    /// before the search aborts. See [`DEFAULT_STEP_BUDGET`].
    pub fn set_step_budget(&mut self, budget: usize) {
        self.step_budget = budget;
    }

    /// Charge one node against the budget; abort the search once it is spent. Called at
    /// EVERY recursion hub — `prove_goal` AND the `solve_subgoals`/`prove_rule_antecedent`
    /// mutual recursion, which does NOT pass through `prove_goal` and so would otherwise
    /// run unbounded behind a recursive axiom. The depth bound caps recursion depth; this
    /// caps total work, so the search always terminates in bounded time.
    fn charge_budget(&mut self) -> ProofResult<()> {
        self.steps += 1;
        if self.steps > self.step_budget {
            Err(ProofError::DepthExceeded)
        } else {
            Ok(())
        }
    }

    /// Get a reference to the knowledge base (for debugging).
    pub fn knowledge_base(&self) -> &[ProofExpr] {
        &self.knowledge_base
    }

    /// Add an axiom/fact/rule to the knowledge base.
    ///
    /// Event semantics are automatically abstracted to simple predicates for efficient proof search.
    pub fn add_axiom(&mut self, expr: ProofExpr) {
        // Pre-process: abstract event semantics to simple predicates
        let abstracted = self.abstract_all_events(&expr);
        // Simplify definite description conjunctions (e.g., butler(butler) ∧ P → P)
        let simplified = self.simplify_definite_description_conjunction(&abstracted);
        self.knowledge_base.push(simplified);
    }

    /// Attempt to prove a goal.
    ///
    /// Returns a derivation tree if successful, explaining how the proof was constructed.
    /// Event semantics in the goal are automatically abstracted (but De Morgan is not applied
    /// to preserve goal pattern matching for reductio strategies).
    pub fn prove(&mut self, goal: ProofExpr) -> ProofResult<DerivationTree> {
        // Pre-process: unify definite descriptions across all axioms
        // This handles Russell's theory of definite descriptions, where multiple
        // "the X" references should refer to the same entity.
        self.unify_definite_descriptions();

        // Pre-process: abstract event semantics in the goal
        // Use abstract_events_only which doesn't apply De Morgan (to preserve ¬∃ pattern)
        let abstracted_goal = self.abstract_events_only(&goal);
        // Simplify definite description conjunctions
        let normalized_goal = self.simplify_definite_description_conjunction(&abstracted_goal);
        // The whole prove→certify pipeline runs on a large-stack thread (see
        // `verify::on_big_stack`), so the recursive search here needs no thread of its
        // own — the native-stack ceiling is already lifted for legitimately deep proofs.
        self.prove_goal(ProofGoal::new(normalized_goal), 0)
    }

    /// Prove a goal with pre-populated context assumptions.
    ///
    /// This allows proving goals like "x > 5" given assumptions like "x > 10" in the context.
    /// The oracle (Z3) will use these context assumptions when verifying.
    pub fn prove_with_goal(&mut self, goal: ProofGoal) -> ProofResult<DerivationTree> {
        self.unify_definite_descriptions();

        // Normalize the target
        let abstracted_target = self.abstract_events_only(&goal.target);
        let normalized_target = self.simplify_definite_description_conjunction(&abstracted_target);

        // Normalize each context assumption
        let normalized_context: Vec<ProofExpr> = goal
            .context
            .iter()
            .map(|expr| {
                let abstracted = self.abstract_events_only(expr);
                self.simplify_definite_description_conjunction(&abstracted)
            })
            .collect();

        let normalized_goal = ProofGoal::with_context(normalized_target, normalized_context);
        self.prove_goal(normalized_goal, 0)
    }

    /// Unify definite descriptions across axioms.
    ///
    /// When multiple axioms contain the same definite description pattern
    /// (e.g., "the barber" creates `∃x ((barber(x) ∧ ∀y (barber(y) → y=x)) ∧ P(x))`),
    /// this function:
    /// 1. Identifies all axioms with the same defining predicate
    /// 2. Extracts the properties attributed to the definite description
    /// 3. Replaces them with a unified Skolem constant and extracted properties
    fn unify_definite_descriptions(&mut self) {
        // Collect definite descriptions by their defining predicate
        let mut definite_descs: std::collections::HashMap<String, Vec<(usize, String, ProofExpr)>> = std::collections::HashMap::new();

        for (idx, axiom) in self.knowledge_base.iter().enumerate() {
            if let Some((pred_name, var_name, property)) = self.extract_definite_description(axiom) {
                definite_descs.entry(pred_name).or_default().push((idx, var_name, property));
            }
        }

        // Discourse-telescoped premises reference a description's variable
        // FREE ("The barber shaves..." after "The barber is a man." emits
        // shave(x, z) with x anaphoric). Those occurrences must be bound to
        // the same unified constant, or the telescoped axioms are inert.
        let mut anaphor_bindings: Vec<(String, ProofTerm)> = Vec::new();

        // For each group of definite descriptions with the same predicate
        for (pred_name, descs) in definite_descs {
            if descs.is_empty() {
                continue;
            }

            // Create a unified Skolem constant for this definite description
            let skolem_name = format!("the_{}", pred_name);
            let skolem_const = ProofTerm::Constant(skolem_name.clone());

            // Add the defining property: pred(skolem)
            let defining_fact = ProofExpr::Predicate {
                name: pred_name.clone(),
                args: vec![skolem_const.clone()],
                world: None,
            };
            self.knowledge_base.push(defining_fact);

            // CRITICAL: Add uniqueness constraint: ∀y (pred(y) → y = skolem)
            // This is essential for proofs that assume ∃x pred(x) - they need to
            // unify their Skolem constant with our unified constant.
            let uniqueness = ProofExpr::ForAll {
                variable: "_u".to_string(),
                body: Box::new(ProofExpr::Implies(
                    Box::new(ProofExpr::Predicate {
                        name: pred_name.clone(),
                        args: vec![ProofTerm::Variable("_u".to_string())],
                        world: None,
                    }),
                    Box::new(ProofExpr::Identity(
                        ProofTerm::Variable("_u".to_string()),
                        skolem_const.clone(),
                    )),
                )),
            };
            self.knowledge_base.push(uniqueness);

            // Replace axioms with the extracted properties
            let mut indices_to_remove: Vec<usize> = Vec::new();
            for (idx, var_name, property) in descs {
                anaphor_bindings.push((var_name.clone(), skolem_const.clone()));
                // Substitute the original variable with the Skolem constant
                let substituted = self.substitute_term_in_expr(
                    &property,
                    &ProofTerm::Variable(var_name),
                    &skolem_const,
                );
                // Normalize the property (especially for ∀x ¬(P ∧ Q) → ∀x (P → ¬Q))
                let normalized = self.normalize_for_proof(&substituted);
                self.knowledge_base.push(normalized);
                indices_to_remove.push(idx);
            }

            // Remove the original existential axioms (in reverse order to preserve indices)
            indices_to_remove.sort_unstable_by(|a, b| b.cmp(a));
            for idx in indices_to_remove {
                self.knowledge_base.remove(idx);
            }
        }

        // Bind free anaphoric occurrences of each description's variable
        // throughout the knowledge base (free occurrences only — binders that
        // shadow the name keep their bodies untouched).
        if !anaphor_bindings.is_empty() {
            let kb = std::mem::take(&mut self.knowledge_base);
            self.knowledge_base = kb
                .into_iter()
                .map(|axiom| {
                    let mut bound = axiom;
                    for (var_name, skolem) in &anaphor_bindings {
                        bound = self.substitute_free_var_in_expr(&bound, var_name, skolem);
                    }
                    bound
                })
                .collect();
        }
    }

    /// Normalize an expression for proof search.
    ///
    /// Applies transformations like: ∀x ¬(P ∧ Q) → ∀x (P → ¬Q)
    fn normalize_for_proof(&self, expr: &ProofExpr) -> ProofExpr {
        match expr {
            ProofExpr::ForAll { variable, body } => {
                // Check for pattern: ∀x ¬(P ∧ Q) → ∀x (P → ¬Q)
                if let ProofExpr::Not(inner) = body.as_ref() {
                    if let ProofExpr::And(left, right) = inner.as_ref() {
                        return ProofExpr::ForAll {
                            variable: variable.clone(),
                            body: Box::new(ProofExpr::Implies(
                                Box::new(self.normalize_for_proof(left)),
                                Box::new(ProofExpr::Not(Box::new(self.normalize_for_proof(right)))),
                            )),
                        };
                    }
                }
                ProofExpr::ForAll {
                    variable: variable.clone(),
                    body: Box::new(self.normalize_for_proof(body)),
                }
            }
            ProofExpr::And(left, right) => ProofExpr::And(
                Box::new(self.normalize_for_proof(left)),
                Box::new(self.normalize_for_proof(right)),
            ),
            ProofExpr::Or(left, right) => ProofExpr::Or(
                Box::new(self.normalize_for_proof(left)),
                Box::new(self.normalize_for_proof(right)),
            ),
            ProofExpr::Implies(left, right) => ProofExpr::Implies(
                Box::new(self.normalize_for_proof(left)),
                Box::new(self.normalize_for_proof(right)),
            ),
            ProofExpr::Not(inner) => ProofExpr::Not(Box::new(self.normalize_for_proof(inner))),
            ProofExpr::Exists { variable, body } => ProofExpr::Exists {
                variable: variable.clone(),
                body: Box::new(self.normalize_for_proof(body)),
            },
            other => other.clone(),
        }
    }

    /// Extract a definite description from an axiom.
    ///
    /// Pattern: ∃x ((P(x) ∧ ∀y (P(y) → y = x)) ∧ Q(x))
    /// Returns: Some((predicate_name, variable_name, Q(x)))
    fn extract_definite_description(&self, expr: &ProofExpr) -> Option<(String, String, ProofExpr)> {
        // Match: ∃x (body)
        let (var, body) = match expr {
            ProofExpr::Exists { variable, body } => (variable.clone(), body.as_ref()),
            _ => return None,
        };

        // Match: (defining_part ∧ property)
        let (defining_part, property) = match body {
            ProofExpr::And(left, right) => (left.as_ref(), right.as_ref().clone()),
            _ => return None,
        };

        // Match defining_part: (P(x) ∧ ∀y (P(y) → y = x))
        let (type_pred, uniqueness) = match defining_part {
            ProofExpr::And(left, right) => (left.as_ref(), right.as_ref()),
            _ => return None,
        };

        // Extract predicate name from P(x)
        let pred_name = match type_pred {
            ProofExpr::Predicate { name, args, .. } if args.len() == 1 => {
                // Verify the arg is our variable
                if let ProofTerm::Variable(v) = &args[0] {
                    if v == &var {
                        name.clone()
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            }
            _ => return None,
        };

        // Verify uniqueness constraint: ∀y (P(y) → y = x)
        match uniqueness {
            ProofExpr::ForAll { variable: _, body: inner_body } => {
                match inner_body.as_ref() {
                    ProofExpr::Implies(ante, cons) => {
                        // Verify antecedent is P(y)
                        if let ProofExpr::Predicate { name, .. } = ante.as_ref() {
                            if name != &pred_name {
                                return None;
                            }
                        } else {
                            return None;
                        }
                        // Verify consequent is an identity (y = x)
                        if !matches!(cons.as_ref(), ProofExpr::Identity(_, _)) {
                            return None;
                        }
                    }
                    _ => return None,
                }
            }
            _ => return None,
        }

        Some((pred_name, var, property))
    }

    /// Internal proof search with depth tracking.
    /// The canonical loop-detection key of a goal: its target with every free
    /// variable renamed to a positional placeholder in first-occurrence order, plus
    /// a fingerprint of its context. Two goals that differ only by their fresh
    /// existential names — the signature of a recursive-axiom regress — map to the
    /// same key; a goal proved under a *richer* context (an added hypothesis from
    /// →I or ∨-elim) gets a different key, so legitimate re-proofs are never pruned.
    fn loop_key(goal: &ProofGoal) -> String {
        use std::hash::{Hash, Hasher};
        let mut order: Vec<String> = Vec::new();
        collect_vars_ordered_expr(&goal.target, &mut order);
        let mut subst = Substitution::new();
        for (i, v) in order.iter().enumerate() {
            subst.insert(v.clone(), ProofTerm::Constant(format!("\u{27ea}{i}\u{27eb}")));
        }
        let canon = apply_subst_to_expr(&goal.target, &subst);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        goal.context.len().hash(&mut hasher);
        for c in &goal.context {
            format!("{c:?}").hash(&mut hasher);
        }
        format!("{canon:?}#{:x}", hasher.finish())
    }

    fn prove_goal(&mut self, goal: ProofGoal, depth: usize) -> ProofResult<DerivationTree> {
        // Node budget: a guarantee of bounded *time*. Reset at the start of a top-level
        // proof, then count every node; once the budget is spent the search fails-fast
        // (reusing `DepthExceeded` — both are resource bounds) rather than running forever
        // on the exponential branching a recursive axiom can induce.
        if depth == 0 {
            self.steps = 0;
        }
        self.charge_budget()?;

        // Check depth limit
        if depth > self.max_depth {
            return Err(ProofError::DepthExceeded);
        }

        // Loop detection: refuse to re-enter a goal already being proved on this
        // branch. A recursive axiom can otherwise regress through ever-fresh
        // existential instances of the same goal until the native stack overflows
        // (the depth bound trips too late). Pruning the re-entry leaves the
        // productive branches — which discharge antecedents against premises or
        // facts, not by re-deriving the same goal — untouched.
        let key = Self::loop_key(&goal);

        // Loop detection: refuse to re-enter a goal already being proved on this
        // branch. A recursive axiom can otherwise regress through ever-fresh
        // existential instances of the same goal until the native stack overflows
        // (the depth bound trips too late). Pruning the re-entry leaves the
        // productive branches — which discharge antecedents against premises or
        // facts, not by re-deriving the same goal — untouched.
        if self.active_goals.contains(&key) {
            return Err(ProofError::NoProofFound);
        }
        self.active_goals.push(key);
        let result = self.prove_goal_inner(goal, depth);
        self.active_goals.pop();
        result
    }

    fn prove_goal_inner(&mut self, goal: ProofGoal, depth: usize) -> ProofResult<DerivationTree> {
        // Check depth limit
        if depth > self.max_depth {
            return Err(ProofError::DepthExceeded);
        }

        // PRIORITY: Check for inductive goals FIRST
        // Goals with TypedVar (e.g., n:Nat) require structural induction,
        // not direct unification which would incorrectly ground the variable.
        if let Some((_, typename)) = self.find_typed_var(&goal.target) {
            // For known inductive types, require induction to succeed
            // Falling back to direct matching would incorrectly unify the TypedVar
            let is_known_inductive = matches!(typename.as_str(), "Nat" | "List");

            if let Some(tree) = self.try_structural_induction(&goal, depth)? {
                return Ok(tree);
            }

            // For known inductive types, if induction fails, the proof fails
            // (don't allow incorrect direct unification)
            if is_known_inductive {
                return Err(ProofError::NoProofFound);
            }
            // For unknown types, fall through to other strategies
        }

        // Strategy 0a: Falsum goal — conflict detection.
        // A goal of ⊥ asks whether the premises are jointly inconsistent. Search
        // the KB + context for a contradiction; the resulting derivation certifies
        // to a kernel-checked proof of `False`.
        if is_falsum(&goal.target) {
            // Prefer the gapless finder (its derivations certify end-to-end);
            // fall back to the heuristic one so a contradiction is still surfaced
            // for inspection even when it cannot yet be certified.
            if let Some(tree) = self.find_certifiable_contradiction(&goal.context, depth)? {
                return Ok(tree);
            }
            if let Some(tree) = self.find_contradiction(&goal.context, depth)? {
                return Ok(tree);
            }
            return Err(ProofError::NoProofFound);
        }

        // Strategy 0: Reflexivity by computation
        // Try to prove a = b by normalizing both sides
        if let Some(tree) = self.try_reflexivity(&goal)? {
            return Ok(tree);
        }

        // Strategy 0b: Arithmetic decision over Int (proof-producing oracle).
        // Discharges Int equalities (e.g. x+y = y+x) the certifier turns into a
        // kernel-checked proof via the ring axioms.
        if let Some(tree) = self.try_arithmetic(&goal)? {
            return Ok(tree);
        }

        // Strategy 0c: Linear arithmetic over `Int` — chain `≤` facts by transitivity,
        // add inequalities, and decide ground facts by computation, certified by the
        // `le_trans`/`le_refl`/`le_add_mono` axioms.
        if let Some(tree) = self.try_linarith(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 1: Direct fact matching
        if let Some(tree) = self.try_match_fact(&goal)? {
            return Ok(tree);
        }

        // Strategy 1b: a NEGATION goal in a finite-domain (grid) setting is proved
        // fastest and most completely by the certified contradiction finder — unit
        // propagation + bounded case analysis. Run it HERE, before the disjunction-
        // elimination strategy (5b), which can blow up on a clue's nested either/or.
        // Assume the negated proposition and drive it to ⊥. It only short-circuits
        // when it actually closes, so a miss leaves the later strategies untouched.
        // A DIRECT ModusTollens (a rule whose consequent is refuted) is more specific,
        // so prefer it — the grid contradiction finder only fires when it does not apply.
        if matches!(goal.target, ProofExpr::Not(_)) {
            if let Some(tree) = self.try_modus_tollens(&goal, depth)? {
                return Ok(tree);
            }
            // A double-negation goal `¬¬X` is introduced by `DoubleNegation` (prove X);
            // don't let the grid contradiction finder preempt that more specific rule.
            let is_double_neg = matches!(&goal.target, ProofExpr::Not(inner) if matches!(inner.as_ref(), ProofExpr::Not(_)));
            if !is_double_neg {
                if let Some(tree) = self.try_reductio_ad_absurdum(&goal, depth)? {
                    return Ok(tree);
                }
            }
        }

        // Strategy 2: Introduction rules (structural decomposition)
        if let Some(tree) = self.try_intro_rules(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 3: Backward chaining on implications
        if let Some(tree) = self.try_backward_chain(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 3b: Modus Tollens (from P → Q and ¬Q, derive ¬P)
        if let Some(tree) = self.try_modus_tollens(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 4: Universal instantiation
        if let Some(tree) = self.try_universal_inst(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 5: Existential introduction
        if let Some(tree) = self.try_existential_intro(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 5b: Disjunction elimination (disjunctive syllogism)
        if let Some(tree) = self.try_disjunction_elimination(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 5c: Proof by contradiction (reductio ad absurdum)
        // For negation goals, assume the positive and derive contradiction
        if let Some(tree) = self.try_reductio_ad_absurdum(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 5d: Existential elimination from premises
        // Extract witnesses from ∃x P(x) premises and add to context
        if let Some(tree) = self.try_existential_elimination(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 6: Equality rewriting (Leibniz's Law)
        if let Some(tree) = self.try_equality_rewrite(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 7: Ex falso quodlibet — a last resort, only at the top level.
        // If the premises are jointly contradictory, any goal follows. The
        // contradiction is a property of the KB, so one check at depth 0 suffices;
        // a consistent KB returns `None` and the goal still fails honestly.
        if depth == 0 && !is_falsum(&goal.target) {
            if let Some(false_tree) = self.find_certifiable_contradiction(&goal.context, depth)? {
                return Ok(DerivationTree::new(
                    goal.target.clone(),
                    InferenceRule::ExFalso,
                    vec![false_tree],
                ));
            }
        }

        // Strategy 8: Classical reductio (proof by contradiction) — last resort at the
        // top level for a POSITIVE goal G: assume ¬G and search for ⊥; if found, G
        // follows via the `dne` axiom. Classical, explicit, kernel-checked.
        if depth == 0 && !is_falsum(&goal.target) && !matches!(goal.target, ProofExpr::Not(_)) {
            let mut ext = goal.context.clone();
            ext.push(ProofExpr::Not(Box::new(goal.target.clone())));
            if let Some(false_tree) = self.find_certifiable_contradiction(&ext, depth)? {
                return Ok(DerivationTree::new(
                    goal.target.clone(),
                    InferenceRule::ClassicalReductio,
                    vec![false_tree],
                ));
            }
        }

        // Strategy 9: Oracle fallback (Z3) — the ABSOLUTE last resort. It is a COMPLETE solver over
        // the whole goal + knowledge base, so (a) it runs only at the TOP LEVEL (`depth == 0`):
        // invoking it per recursive subgoal is an O(nodes) Z3-call blowup that hangs a deep
        // backward-chaining search, and is redundant since a decomposed subgoal is oracle-provable
        // only if the goal it came from is; and (b) it runs only AFTER every certifiable kernel
        // strategy (incl. ex-falso / reductio above), so a kernel-checkable proof is never lost to a
        // non-certifiable Z3 "Verified by Z3" result.
        #[cfg(feature = "verification")]
        if depth == 0 {
            if let Some(tree) = self.try_oracle_fallback(&goal)? {
                return Ok(tree);
            }
        }

        // No proof found
        Err(ProofError::NoProofFound)
    }

    // =========================================================================
    // STRATEGY 0: REFLEXIVITY BY COMPUTATION
    // =========================================================================

    /// Try to prove an identity a = b by normalizing both sides.
    ///
    /// If both sides reduce to structurally identical expressions,
    /// the proof is by reflexivity (a = a).
    fn try_reflexivity(&self, goal: &ProofGoal) -> ProofResult<Option<DerivationTree>> {
        if let ProofExpr::Identity(left_term, right_term) = &goal.target {
            // Convert terms to expressions for reduction
            let left_expr = term_to_expr(left_term);
            let right_expr = term_to_expr(right_term);

            // Normalize both sides using full reduction (beta + iota + fix)
            let left_normal = beta_reduce(&left_expr);
            let right_normal = beta_reduce(&right_expr);

            // Check structural equality after normalization
            if exprs_structurally_equal(&left_normal, &right_normal) {
                return Ok(Some(DerivationTree::leaf(
                    goal.target.clone(),
                    InferenceRule::Reflexivity,
                )));
            }
        }
        Ok(None)
    }

    /// Strategy 0b: decide an `Int` equality with the proof-producing oracle.
    ///
    /// The oracle ([`crate::arith::prove_int_eq`]) searches for a real kernel
    /// proof (computation + the ring axioms). If it finds one, we emit an
    /// `ArithDecision` leaf; the certifier re-runs the oracle to produce the
    /// kernel term, which `infer_type` re-checks — so the oracle is never
    /// trusted. Non-arithmetic identities are left to other strategies.
    fn try_arithmetic(&self, goal: &ProofGoal) -> ProofResult<Option<DerivationTree>> {
        if let ProofExpr::Identity(left_term, right_term) = &goal.target {
            // Only engage for genuine Int-arithmetic identities (an arithmetic
            // operator at the head of a side). This keeps the strategy from
            // hijacking unrelated reflexive identities (e.g. f(a)=f(a) over
            // non-Int terms), which it would otherwise close with a type-wrong
            // `refl Int …` that fails certification.
            let is_arith = |t: &ProofTerm| {
                matches!(
                    t,
                    ProofTerm::Function(n, _)
                        if matches!(n.as_str(), "add" | "sub" | "mul" | "div" | "mod")
                )
            };
            if !is_arith(left_term) && !is_arith(right_term) {
                return Ok(None);
            }
            let (kl, kr) = match (
                crate::certifier::proof_term_to_kernel_term(left_term),
                crate::certifier::proof_term_to_kernel_term(right_term),
            ) {
                (Ok(a), Ok(b)) => (a, b),
                _ => return Ok(None),
            };

            let mut ctx = logicaffeine_kernel::Context::new();
            logicaffeine_kernel::prelude::StandardLibrary::register(&mut ctx);

            if crate::arith::prove_int_eq(&ctx, &kl, &kr).is_some() {
                return Ok(Some(DerivationTree::leaf(
                    goal.target.clone(),
                    InferenceRule::ArithDecision,
                )));
            }
        }
        Ok(None)
    }

    // =========================================================================
    // STRATEGY 1: DIRECT FACT MATCHING
    // =========================================================================

    /// Try to match the goal directly against a fact in the knowledge base.
    fn try_match_fact(&self, goal: &ProofGoal) -> ProofResult<Option<DerivationTree>> {
        // Also check local context
        for fact in goal.context.iter().chain(self.knowledge_base.iter()) {
            if let Ok(subst) = unify_exprs(&goal.target, fact) {
                return Ok(Some(
                    DerivationTree::leaf(goal.target.clone(), InferenceRule::PremiseMatch)
                        .with_substitution(subst),
                ));
            }
        }
        // Forward ∧-elimination: a conjunct of an assumed `A ∧ B` is itself a usable
        // fact. Project it out with a `ConjunctionElim` chain — this is what lets the
        // existential middle of a cut pin against a conjunctive hypothesis instead of
        // fanning out into an unbounded search (the difference between milliseconds and
        // a blow-up for `(Cong ∧ Cong) → Cong`-shaped theorems).
        for fact in goal.context.iter().chain(self.knowledge_base.iter()) {
            if matches!(fact, ProofExpr::And(_, _)) {
                let leaf = DerivationTree::leaf(fact.clone(), InferenceRule::PremiseMatch);
                if let Some((tree, subst)) = extract_conjunct(&leaf, &goal.target) {
                    return Ok(Some(tree.with_substitution(subst)));
                }
            }
        }
        Ok(None)
    }

    // =========================================================================
    // STRATEGY 2: INTRODUCTION RULES
    // =========================================================================

    /// Try introduction rules based on the goal's structure.
    fn try_intro_rules(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        match &goal.target {
            // Conjunction Introduction: To prove A ∧ B, prove A and prove B
            ProofExpr::And(left, right) => {
                let left_goal = ProofGoal::with_context((**left).clone(), goal.context.clone());
                let right_goal = ProofGoal::with_context((**right).clone(), goal.context.clone());

                // Try to prove both sides
                if let (Ok(left_proof), Ok(right_proof)) = (
                    self.prove_goal(left_goal, depth + 1),
                    self.prove_goal(right_goal, depth + 1),
                ) {
                    return Ok(Some(DerivationTree::new(
                        goal.target.clone(),
                        InferenceRule::ConjunctionIntro,
                        vec![left_proof, right_proof],
                    )));
                }
            }

            // Disjunction Introduction: To prove A ∨ B, prove A or prove B
            ProofExpr::Or(left, right) => {
                // Try left side first
                let left_goal = ProofGoal::with_context((**left).clone(), goal.context.clone());
                if let Ok(left_proof) = self.prove_goal(left_goal, depth + 1) {
                    return Ok(Some(DerivationTree::new(
                        goal.target.clone(),
                        InferenceRule::DisjunctionIntro,
                        vec![left_proof],
                    )));
                }

                // Try right side
                let right_goal = ProofGoal::with_context((**right).clone(), goal.context.clone());
                if let Ok(right_proof) = self.prove_goal(right_goal, depth + 1) {
                    return Ok(Some(DerivationTree::new(
                        goal.target.clone(),
                        InferenceRule::DisjunctionIntro,
                        vec![right_proof],
                    )));
                }
            }

            // Biconditional Introduction (↔I): to prove P ↔ Q, prove both directions
            // P → Q and Q → P (each by →I), then combine. Certified as `conj` over the
            // two implications (Iff ≡ And (P→Q) (Q→P)).
            ProofExpr::Iff(left, right) => {
                let pq = ProofExpr::Implies(left.clone(), right.clone());
                let qp = ProofExpr::Implies(right.clone(), left.clone());
                let pq_goal = ProofGoal::with_context(pq, goal.context.clone());
                let qp_goal = ProofGoal::with_context(qp, goal.context.clone());
                if let (Ok(pq_proof), Ok(qp_proof)) = (
                    self.prove_goal(pq_goal, depth + 1),
                    self.prove_goal(qp_goal, depth + 1),
                ) {
                    return Ok(Some(DerivationTree::new(
                        goal.target.clone(),
                        InferenceRule::BicondIntro,
                        vec![pq_proof, qp_proof],
                    )));
                }
            }

            // Universal Introduction (∀I): to prove ∀x.φ(x), prove φ(c) for a FRESH
            // EIGENCONSTANT c — an opaque term the search can neither unify nor
            // instantiate — then generalize c back to x. Proving the body with x left
            // as a `Variable` is unsound for search: a strategy could bind x by
            // unification (e.g. match goal `Cong(c,d,a,b)` against hypothesis
            // `Cong(a,b,c,d)` by setting a:=c,…), producing a tree the kernel rejects
            // but the engine has already committed to. The eigenconstant makes such a
            // match fail, forcing the genuinely-arbitrary proof; the certifier then
            // abstracts `λ(x:Entity). …` over the generalized body.
            ProofExpr::ForAll { variable, body } => {
                let eigen = self.fresh_eigenconstant();
                let mut subst = Substitution::new();
                subst.insert(variable.clone(), ProofTerm::Constant(eigen.clone()));
                let body_expr = apply_subst_to_expr(body, &subst);
                let body_ctx: Vec<ProofExpr> =
                    goal.context.iter().map(|c| apply_subst_to_expr(c, &subst)).collect();
                let body_goal = ProofGoal::with_context(body_expr, body_ctx);
                if let Ok(body_proof) = self.prove_goal(body_goal, depth + 1) {
                    let generalized = rewrite_tree_const_to_var(&body_proof, &eigen, variable);
                    return Ok(Some(DerivationTree::new(
                        goal.target.clone(),
                        InferenceRule::UniversalIntro {
                            variable: variable.clone(),
                            var_type: "Entity".to_string(),
                        },
                        vec![generalized],
                    )));
                }
            }

            // Implication Introduction (→I): to prove P → Q, assume P (discharge it
            // into the local context) and prove Q. The certifier binds P as a local
            // hypothesis and wraps the Q-proof in `λ(hp:P). …`.
            ProofExpr::Implies(ant, con) => {
                let mut ext = goal.context.clone();
                ext.push((**ant).clone());
                let con_goal = ProofGoal::with_context((**con).clone(), ext);
                if let Ok(con_proof) = self.prove_goal(con_goal, depth + 1) {
                    return Ok(Some(DerivationTree::new(
                        goal.target.clone(),
                        InferenceRule::ImpliesIntro,
                        vec![con_proof],
                    )));
                }
            }

            // Double Negation: To prove ¬¬P, prove P
            ProofExpr::Not(inner) => {
                if let ProofExpr::Not(core) = &**inner {
                    let core_goal = ProofGoal::with_context((**core).clone(), goal.context.clone());
                    if let Ok(core_proof) = self.prove_goal(core_goal, depth + 1) {
                        return Ok(Some(DerivationTree::new(
                            goal.target.clone(),
                            InferenceRule::DoubleNegation,
                            vec![core_proof],
                        )));
                    }
                }
            }

            _ => {}
        }

        Ok(None)
    }

    // =========================================================================
    // STRATEGY 3: BACKWARD CHAINING ON IMPLICATIONS
    // =========================================================================

    /// Try backward chaining: find P → Goal in KB, then prove P.
    fn try_backward_chain(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Collect every KB implication whose consequent unifies with the goal, and SCORE
        // each by how many of its (instantiated) antecedent conjuncts can be discharged
        // DIRECTLY against a known fact or hypothesis. Trying the most-pinnable rule first
        // finds a direct proof — an axiom whose whole antecedent is already among the
        // premises (e.g. Tarski's five-segment, with all seven antecedents as givens) —
        // before a recursive axiom (inner transitivity) sends the search into exponential
        // re-derivation. This is the relevance signal that lets the prover pick the rule it
        // actually needs; it only REORDERS candidates, so nothing that was provable becomes
        // unprovable.
        struct Candidate {
            impl_expr: ProofExpr,
            antecedent: ProofExpr,
            subst: Substitution,
            score: usize,
        }
        let kb_implications: Vec<ProofExpr> = self
            .knowledge_base
            .iter()
            .filter(|e| matches!(e, ProofExpr::Implies(_, _)))
            .cloned()
            .collect();
        let mut candidates: Vec<Candidate> = Vec::new();
        for impl_expr in kb_implications {
            let renamed = self.rename_variables(&impl_expr);
            if let ProofExpr::Implies(ant, con) = renamed {
                if let Ok(subst) = unify_exprs(&goal.target, &con) {
                    let antecedent = apply_subst_to_expr(&ant, &subst);
                    let score = self.antecedent_pinnability(&antecedent, &goal.context);
                    candidates.push(Candidate { impl_expr, antecedent, subst, score });
                }
            }
        }
        // Most-pinnable antecedent first; stable, so equal scores keep KB order.
        candidates.sort_by(|a, b| b.score.cmp(&a.score));

        for cand in candidates {
            let ant_goal = ProofGoal::with_context(cand.antecedent, goal.context.clone());
            if let Ok(ant_proof) = self.prove_goal(ant_goal, depth + 1) {
                let impl_leaf =
                    DerivationTree::leaf(cand.impl_expr.clone(), InferenceRule::PremiseMatch);
                return Ok(Some(
                    DerivationTree::new(
                        goal.target.clone(),
                        InferenceRule::ModusPonens,
                        vec![impl_leaf, ant_proof],
                    )
                    .with_substitution(cand.subst),
                ));
            }
        }

        Ok(None)
    }

    /// How many of `ant`'s conjuncts can be discharged immediately against a known fact or
    /// a conjunct of a conjunctive hypothesis — the relevance score for
    /// [`Self::try_backward_chain`]. A rule whose antecedent is entirely pinnable needs no
    /// recursive search at all, so it is the one to try first.
    fn antecedent_pinnability(&self, ant: &ProofExpr, context: &[ProofExpr]) -> usize {
        flatten_conjuncts(ant)
            .iter()
            .filter(|c| self.subgoal_pinnable(c, context))
            .count()
    }

    /// Relevance of a (possibly `∀`-wrapped) rule to `goal` — how worthwhile it is to try
    /// FIRST when backward-chaining. A universal FACT whose matrix unifies with the goal
    /// is the best (direct instantiation, no antecedent, terminating). Next, an implication
    /// whose consequent unifies with the goal, ranked by how many of its antecedent
    /// conjuncts are already discharge-able against hypotheses — so the axiom that needs no
    /// recursive search outranks a recursive one. A non-matching rule scores 0. This is a
    /// pure ORDERING signal: every rule is still tried, just in a smarter order, so the
    /// direct proof is reached before a recursive axiom explodes the search.
    fn rule_relevance(&mut self, rule: &ProofExpr, goal: &ProofGoal) -> usize {
        let renamed = self.rename_variables(rule);
        let mut matrix: &ProofExpr = &renamed;
        while let ProofExpr::ForAll { body, .. } = matrix {
            matrix = body.as_ref();
        }
        match matrix {
            ProofExpr::Implies(ant, con) => {
                if let Ok(subst) = unify_exprs(&goal.target, con) {
                    let ant_inst = apply_subst_to_expr(ant, &subst);
                    // +2 so any matching implication outranks a non-match; pinnable
                    // antecedent conjuncts add on top (a fully-pinned antecedent wins).
                    2 + self.antecedent_pinnability(&ant_inst, &goal.context)
                } else {
                    0
                }
            }
            other => {
                // A universal fact that unifies: direct, terminating — rank above any
                // implication so it discharges before a (possibly recursive) rule chain.
                if unify_exprs(&goal.target, other).is_ok() {
                    1000
                } else {
                    0
                }
            }
        }
    }

    // =========================================================================
    // STRATEGY 3b: MODUS TOLLENS
    // =========================================================================

    /// Try Modus Tollens: from P → Q and ¬Q, derive ¬P.
    ///
    /// If the goal is ¬P:
    /// 1. Look for implications P → Q in the KB
    /// 2. Check if ¬Q is known (in KB or context) OR can be proved
    /// 3. If so, derive ¬P by Modus Tollens
    fn try_modus_tollens(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Modus Tollens only applies when goal is a negation: ¬P
        let inner_goal = match &goal.target {
            ProofExpr::Not(inner) => (**inner).clone(),
            _ => return Ok(None),
        };

        // Collect all implications from KB, including those inside ForAll
        let implications: Vec<ProofExpr> = self
            .knowledge_base
            .iter()
            .chain(goal.context.iter())
            .flat_map(|expr| {
                match expr {
                    ProofExpr::Implies(_, _) => vec![expr.clone()],
                    ProofExpr::ForAll { body, .. } => {
                        // Extract implications from inside universal quantifiers
                        if let ProofExpr::Implies(_, _) = body.as_ref() {
                            vec![*body.clone()]
                        } else {
                            vec![]
                        }
                    }
                    _ => vec![],
                }
            })
            .collect();

        // Collect all negations from KB and context (for direct matching)
        let negations: Vec<ProofExpr> = self
            .knowledge_base
            .iter()
            .chain(goal.context.iter())
            .filter_map(|expr| {
                if let ProofExpr::Not(inner) = expr {
                    Some((**inner).clone())
                } else {
                    None
                }
            })
            .collect();

        // For each implication P → Q
        for impl_expr in &implications {
            if let ProofExpr::Implies(antecedent, consequent) = impl_expr {
                // Check if the antecedent P matches our inner goal (we want to prove ¬P)
                if let Ok(subst) = unify_exprs(&inner_goal, antecedent) {
                    // Apply substitution to the consequent Q
                    let q = apply_subst_to_expr(consequent, &subst);

                    // First, check if ¬Q is directly in our known facts
                    for negated in &negations {
                        if exprs_structurally_equal(negated, &q) {
                            // We have P → Q and ¬Q, so we can derive ¬P
                            let impl_leaf = DerivationTree::leaf(
                                impl_expr.clone(),
                                InferenceRule::PremiseMatch,
                            );
                            let neg_q_leaf = DerivationTree::leaf(
                                ProofExpr::Not(Box::new(q.clone())),
                                InferenceRule::PremiseMatch,
                            );

                            return Ok(Some(
                                DerivationTree::new(
                                    goal.target.clone(),
                                    InferenceRule::ModusTollens,
                                    vec![impl_leaf, neg_q_leaf],
                                )
                                .with_substitution(subst),
                            ));
                        }
                    }

                    // Second, try to prove ¬Q recursively (for chaining)
                    let neg_q_goal = ProofGoal::with_context(
                        ProofExpr::Not(Box::new(q.clone())),
                        goal.context.clone(),
                    );

                    if let Ok(neg_q_proof) = self.prove_goal(neg_q_goal, depth + 1) {
                        // We proved ¬Q, so we can derive ¬P
                        let impl_leaf = DerivationTree::leaf(
                            impl_expr.clone(),
                            InferenceRule::PremiseMatch,
                        );

                        return Ok(Some(
                            DerivationTree::new(
                                goal.target.clone(),
                                InferenceRule::ModusTollens,
                                vec![impl_leaf, neg_q_proof],
                            )
                            .with_substitution(subst),
                        ));
                    }
                }
            }
        }

        Ok(None)
    }

    // =========================================================================
    // STRATEGY 4: UNIVERSAL INSTANTIATION
    // =========================================================================

    /// Try universal instantiation: if KB has ∀x.P(x), try to prove P(t) for some term t.
    fn try_universal_inst(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Look for universal quantifiers in KB, then ORDER them by relevance to the goal:
        // the rule whose antecedent is most directly dischargeable (or a universal fact,
        // which terminates) is tried first, so a direct proof is found before a recursive
        // axiom drives the search into exponential re-derivation. Pure reordering — every
        // rule is still tried — so nothing provable becomes unprovable.
        let universal_exprs: Vec<ProofExpr> = self
            .knowledge_base
            .iter()
            .filter(|expr| matches!(expr, ProofExpr::ForAll { .. }))
            .cloned()
            .collect();
        let mut scored: Vec<(usize, ProofExpr)> = universal_exprs
            .into_iter()
            .map(|u| (self.rule_relevance(&u, goal), u))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        let universals: Vec<ProofExpr> = scored.into_iter().map(|(_, u)| u).collect();

        for forall_expr in universals {
            // Freshen every variable so instantiation cannot capture variables in
            // the goal, then peel *all* leading universal quantifiers. A rule may
            // bind several variables (`∀x∀y∀z …`); only peeling one leaves the
            // matrix still quantified and unusable — that was the transitivity gap.
            let renamed = self.rename_variables(&forall_expr);
            let mut body: &ProofExpr = &renamed;
            let mut bound_vars: Vec<String> = Vec::new();
            while let ProofExpr::ForAll { variable, body: inner } = body {
                bound_vars.push(variable.clone());
                body = inner.as_ref();
            }
            if bound_vars.is_empty() {
                continue;
            }

            // Case A: the matrix unifies directly with the goal — a universal fact
            // (e.g. ∀x∀y R(x,y) discharging R(a,b)), including one whose matrix is
            // itself existential (∀b c d. ∃x. Cong(b,x,c,d) discharging ∃x. Cong(P,x,Q,R)).
            if let Ok(subst) = unify_exprs(&goal.target, body) {
                // Build NESTED UniversalInst nodes so a multi-binder fact applies to ALL
                // witnesses, not just the first. A variable vacuous in the matrix is
                // defaulted to an entity; only a genuinely-underdetermined one aborts.
                if let Some(witnesses) = instantiation_witnesses(&bound_vars, &subst, body) {
                    let mut inst_node =
                        DerivationTree::leaf(forall_expr.clone(), InferenceRule::PremiseMatch);
                    for witness in &witnesses {
                        inst_node = DerivationTree::new(
                            goal.target.clone(),
                            InferenceRule::UniversalInst(format!("{}", witness)),
                            vec![inst_node],
                        );
                    }
                    return Ok(Some(inst_node.with_substitution(subst)));
                }
            }

            // Case B: the matrix is an implication ∀…(P → Q). Unify Q with the
            // goal, then prove P. Any bound variable that does NOT appear in Q (a
            // "middle term", e.g. the `y` in transitivity) is left open by that
            // unification and is resolved while proving P against the KB.
            if let ProofExpr::Implies(ant, con) = body {
                if let Ok(subst) = unify_exprs(&goal.target, &**con) {
                    let ant_inst = apply_subst_to_expr(&**ant, &subst);
                    if let Ok((ant_proof, final_subst)) =
                        self.prove_rule_antecedent(&ant_inst, &subst, &goal.context, depth + 1)
                    {
                        // Each bound variable must be pinned by the conclusion
                        // unification or the antecedent solve — or be vacuous in the
                        // matrix (then defaulted). A genuinely-underdetermined variable
                        // aborts this rule rather than leak a metavariable into the cert.
                        let witnesses =
                            match instantiation_witnesses(&bound_vars, &final_subst, body) {
                                Some(w) => w,
                                None => continue,
                            };
                        // Build NESTED UniversalInst nodes — one per bound variable, in
                        // binder order — so the certifier applies the universal proof to
                        // ALL witnesses (`((forall x) y) z`), fully instantiating
                        // ∀x∀y∀z. A single node with one witness left the inner
                        // quantifiers unfilled (the "expected Entity, found And" bug).
                        let instantiated = apply_subst_to_expr(body, &final_subst);
                        let mut inst_node =
                            DerivationTree::leaf(forall_expr.clone(), InferenceRule::PremiseMatch);
                        for witness in &witnesses {
                            inst_node = DerivationTree::new(
                                instantiated.clone(),
                                InferenceRule::UniversalInst(format!("{}", witness)),
                                vec![inst_node],
                            );
                        }
                        return Ok(Some(
                            DerivationTree::new(
                                goal.target.clone(),
                                InferenceRule::ModusPonens,
                                vec![inst_node, ant_proof],
                            )
                            .with_substitution(final_subst),
                        ));
                    }
                }
            }
        }

        Ok(None)
    }

    /// Prove the antecedent of an instantiated rule, returning both its proof and
    /// the bindings it forced. The antecedent may be a conjunction whose conjuncts
    /// share a still-open variable (a middle term); [`Self::solve_subgoals`]
    /// resolves it by threading bindings left to right.
    fn prove_rule_antecedent(
        &mut self,
        antecedent: &ProofExpr,
        subst: &Substitution,
        context: &[ProofExpr],
        depth: usize,
    ) -> ProofResult<(DerivationTree, Substitution)> {
        let conjuncts = flatten_conjuncts(antecedent);
        let (proofs, final_subst) = self.solve_subgoals(&conjuncts, subst, context, depth)?;
        let tree = if proofs.len() == 1 {
            proofs.into_iter().next().expect("exactly one antecedent proof")
        } else {
            // Fold the per-conjunct proofs into a BINARY-nested `ConjunctionIntro` tree
            // that mirrors the antecedent's own `And` structure — the certifier's
            // `ConjunctionIntro` is binary (`conj P Q · ·`), so a 7-conjunct antecedent
            // becomes 6 nested binary nodes, not one illegal flat node. O(n), no extra
            // proving: `flatten_conjuncts` and this fold share the same left-to-right
            // recursion, so the flat proofs align exactly with the leaves.
            let mut proofs = proofs.into_iter();
            build_conjunction_tree(antecedent, &final_subst, &mut proofs)
        };
        Ok((tree, final_subst))
    }

    /// Whether `g` can be discharged *immediately* by a known fact — either it
    /// unifies with an atomic fact, or with a conjunct of a conjunctive hypothesis
    /// (forward ∧-elimination). Such a subgoal pins its variables cheaply, so it is
    /// preferred over one that would need a recursive rule search.
    fn subgoal_pinnable(&self, g: &ProofExpr, context: &[ProofExpr]) -> bool {
        context.iter().chain(self.knowledge_base.iter()).any(|fact| {
            if is_atomic_fact(fact) {
                unify_exprs(g, fact).is_ok()
            } else if matches!(fact, ProofExpr::And(_, _)) {
                flatten_conjuncts(fact).iter().any(|c| unify_exprs(g, c).is_ok())
            } else {
                false
            }
        })
    }

    /// Choose which subgoal to solve next, cheapest first: a ground subgoal (no
    /// branching) over a fact-pinnable one (binds a middle term immediately) over an
    /// open one that needs a rule search. Conjunctions are left in place (score
    /// highest) — the caller expands them at the head. Returns an index into
    /// `subgoals`; ties resolve to the lowest index, keeping the search deterministic.
    fn select_subgoal(
        &self,
        subgoals: &[ProofExpr],
        subst: &Substitution,
        context: &[ProofExpr],
    ) -> usize {
        let mut best = 0;
        let mut best_score = u8::MAX;
        for (i, sg) in subgoals.iter().enumerate() {
            let g = apply_subst_to_expr(sg, subst);
            let score = if matches!(g, ProofExpr::And(_, _)) {
                3
            } else if self.collect_variables(&g).is_empty() {
                0
            } else if self.subgoal_pinnable(&g, context) {
                1
            } else {
                2
            };
            if score < best_score {
                best_score = score;
                best = i;
                if score == 0 {
                    break;
                }
            }
        }
        best
    }

    /// Prove a flat list of subgoals under a shared substitution, threading the
    /// bindings discovered for each subgoal into the rest — SLD resolution with
    /// backtracking. A subgoal that still mentions a variable after substitution
    /// is *open*: its variable is a middle term, bound from the knowledge base by
    /// trying facts first (terminating) and then rule consequents (recursive).
    fn solve_subgoals(
        &mut self,
        subgoals: &[ProofExpr],
        subst: &Substitution,
        context: &[ProofExpr],
        depth: usize,
    ) -> ProofResult<(Vec<DerivationTree>, Substitution)> {
        self.charge_budget()?;
        if depth > self.max_depth {
            return Err(ProofError::DepthExceeded);
        }
        if subgoals.is_empty() {
            return Ok((Vec::new(), subst.clone()));
        }
        // A nested conjunction at the head expands into the subgoal list.
        let head = apply_subst_to_expr(&subgoals[0], subst);
        if matches!(head, ProofExpr::And(_, _)) {
            let mut expanded = flatten_conjuncts(&head);
            expanded.extend_from_slice(&subgoals[1..]);
            return self.solve_subgoals(&expanded, subst, context, depth);
        }

        // Cheapest-first selection: solve a GROUND subgoal (no branching), or one a
        // direct fact / conjunct pins immediately, before an open subgoal that would
        // otherwise drive an unbounded rule search. This is constraint propagation —
        // a shared middle term gets bound by its easiest sibling first — and is what
        // keeps the existential cut behind a recursive axiom from blowing up. The
        // chosen subgoal is SOLVED first but its proof is re-INSERTED at its original
        // position, so the returned order still matches the (order-sensitive)
        // `ConjunctionIntro` reconstruction in `prove_rule_antecedent`.
        let idx = self.select_subgoal(subgoals, subst, context);
        let first = &subgoals[idx];
        let rest: Vec<ProofExpr> = subgoals
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != idx)
            .map(|(_, e)| e.clone())
            .collect();
        let g = apply_subst_to_expr(first, subst);

        // Ground subgoal (no open variable): prove it with the full strategy
        // suite, so non-Horn antecedents (negations, disjunctions, induction, …)
        // stay exactly as provable as a top-level goal. No new bindings arise.
        // Recurse at `depth + 1` so a recursive axiom (whose antecedent re-enters the
        // chainer) is bounded by `max_depth` instead of overflowing the stack.
        if self.collect_variables(&g).is_empty() {
            let proof =
                self.prove_goal(ProofGoal::with_context(g.clone(), context.to_vec()), depth + 1)?;
            let (mut proofs, out) = self.solve_subgoals(&rest, subst, context, depth)?;
            proofs.insert(idx, proof);
            return Ok((proofs, out));
        }

        // Open subgoal — Option 1: discharge directly against a known fact, OR against
        // a conjunct of a conjunctive hypothesis (forward ∧-elimination), binding the
        // middle term; backtrack over the choice if the rest cannot close.
        let facts: Vec<ProofExpr> = context
            .iter()
            .chain(self.knowledge_base.iter())
            .filter(|e| is_atomic_fact(e) || matches!(e, ProofExpr::And(_, _)))
            .cloned()
            .collect();
        for fact in &facts {
            let matched: Option<(DerivationTree, Substitution)> =
                if matches!(fact, ProofExpr::And(_, _)) {
                    let src = DerivationTree::leaf(fact.clone(), InferenceRule::PremiseMatch);
                    extract_conjunct(&src, &g)
                } else if is_atomic_fact(fact) {
                    unify_exprs(&g, fact).ok().map(|delta| {
                        let leaf = DerivationTree::leaf(
                            apply_subst_to_expr(&g, &delta),
                            InferenceRule::PremiseMatch,
                        );
                        (leaf, delta)
                    })
                } else {
                    None
                };
            if let Some((leaf, delta)) = matched {
                let combined = compose_substitutions(subst.clone(), delta.clone());
                if let Ok((mut proofs, out)) =
                    self.solve_subgoals(&rest, &combined, context, depth)
                {
                    proofs.insert(idx, leaf.with_substitution(delta));
                    return Ok((proofs, out));
                }
            }
        }

        // Open subgoal — Option 2: chain through a rule whose consequent unifies
        // with the subgoal, recursively proving that rule's antecedent.
        let rules: Vec<ProofExpr> = self
            .knowledge_base
            .iter()
            .filter(|e| matches!(e, ProofExpr::ForAll { .. } | ProofExpr::Implies(_, _)))
            .cloned()
            .collect();
        for rule in &rules {
            let renamed = self.rename_variables(rule);
            let mut matrix: &ProofExpr = &renamed;
            while let ProofExpr::ForAll { body, .. } = matrix {
                matrix = body.as_ref();
            }
            if let ProofExpr::Implies(ant, con) = matrix {
                if let Ok(delta) = unify_exprs(&g, &**con) {
                    let combined = compose_substitutions(subst.clone(), delta.clone());
                    let ant_inst = apply_subst_to_expr(&**ant, &combined);
                    if let Ok((ant_proof, s_ant)) =
                        self.prove_rule_antecedent(&ant_inst, &combined, context, depth + 1)
                    {
                        // Build NESTED UniversalInst nodes — one per bound variable, in
                        // binder order — so a multi-binder rule is fully instantiated.
                        // Each variable must be pinned or vacuous in the matrix; a
                        // genuinely-underdetermined one aborts rather than leak a metavar.
                        let mut bound_vars = Vec::new();
                        let mut peel: &ProofExpr = &renamed;
                        while let ProofExpr::ForAll { variable, body } = peel {
                            bound_vars.push(variable.clone());
                            peel = body;
                        }
                        let witnesses = match instantiation_witnesses(&bound_vars, &s_ant, matrix) {
                            Some(w) => w,
                            None => continue,
                        };
                        let resolved = apply_subst_to_expr(&g, &s_ant);
                        let instantiated = apply_subst_to_expr(matrix, &s_ant);
                        let mut inst_node =
                            DerivationTree::leaf(rule.clone(), InferenceRule::PremiseMatch);
                        for witness in &witnesses {
                            inst_node = DerivationTree::new(
                                instantiated.clone(),
                                InferenceRule::UniversalInst(format!("{}", witness)),
                                vec![inst_node],
                            );
                        }
                        let mp = DerivationTree::new(
                            resolved,
                            InferenceRule::ModusPonens,
                            vec![inst_node, ant_proof],
                        );
                        if let Ok((mut proofs, out)) =
                            self.solve_subgoals(&rest, &s_ant, context, depth)
                        {
                            proofs.insert(idx, mp);
                            return Ok((proofs, out));
                        }
                    }
                }
            } else if let Ok(delta) = unify_exprs(&g, matrix) {
                // A universally-quantified FACT (no antecedent) — e.g. Tarski A1
                // `∀a b. Cong(a,b,b,a)`: instantiate it by unifying its matrix with
                // the subgoal directly. Without this only `Implies` rules are chained,
                // so a universal fact is unreachable when it must discharge an open
                // antecedent (the existential cut behind recursive axioms).
                let combined = compose_substitutions(subst.clone(), delta.clone());
                let resolved = apply_subst_to_expr(&g, &combined);
                // Recover the original ∀-variable order and each variable's witness by
                // unifying the ORIGINAL matrix against the resolved (ground) goal, then
                // apply the fact to each witness in turn (nested `UniversalInst`).
                let mut vars = Vec::new();
                let mut orig_body: &ProofExpr = rule;
                while let ProofExpr::ForAll { variable, body } = orig_body {
                    vars.push(variable.clone());
                    orig_body = body;
                }
                if let Ok(wsubst) = unify_exprs(orig_body, &resolved) {
                    let witnesses = match instantiation_witnesses(&vars, &wsubst, orig_body) {
                        Some(w) => w,
                        None => continue,
                    };
                    let mut node =
                        DerivationTree::leaf(rule.clone(), InferenceRule::PremiseMatch);
                    for witness in &witnesses {
                        node = DerivationTree::new(
                            resolved.clone(),
                            InferenceRule::UniversalInst(format!("{}", witness)),
                            vec![node],
                        );
                    }
                    if let Ok((mut proofs, out)) =
                        self.solve_subgoals(&rest, &combined, context, depth)
                    {
                        proofs.insert(idx, node);
                        return Ok((proofs, out));
                    }
                }
            }
        }

        Err(ProofError::NoProofFound)
    }

    // =========================================================================
    // STRATEGY 5: EXISTENTIAL INTRODUCTION
    // =========================================================================

    /// Try existential introduction: to prove ∃x.P(x), find a witness t and prove P(t).
    fn try_existential_intro(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        if let ProofExpr::Exists { variable, body } = &goal.target {
            // We need to find a witness that makes the body true
            // Try each constant/ground term in our KB as a potential witness
            let witnesses = self.collect_witnesses();

            for witness in witnesses {
                // Create a substitution mapping the variable to the witness
                let mut subst = Substitution::new();
                subst.insert(variable.clone(), witness.clone());

                // Apply substitution to get the instantiated body
                let instantiated = apply_subst_to_expr(body, &subst);

                // Try to prove the instantiated body
                let inst_goal = ProofGoal::with_context(instantiated, goal.context.clone());

                if let Ok(body_proof) = self.prove_goal(inst_goal, depth + 1) {
                    let witness_str = format!("{}", witness);
                    // Witness type from an explicit TypedVar if present (arithmetic /
                    // induction), else the FOL domain `Entity` — matching the rest of
                    // the encoding (`∀` ranges over Entity, predicates are Entity→Prop),
                    // so an untyped `∃y.P(y)` certifies as `Ex Entity …`, not `Ex Nat …`.
                    let witness_type = extract_type_from_exists_body(body)
                        .unwrap_or_else(|| "Entity".to_string());
                    return Ok(Some(DerivationTree::new(
                        goal.target.clone(),
                        InferenceRule::ExistentialIntro {
                            witness: witness_str,
                            witness_type,
                        },
                        vec![body_proof],
                    )));
                }
            }
        }

        Ok(None)
    }

    // =========================================================================
    // STRATEGY 5b: DISJUNCTION ELIMINATION (DISJUNCTIVE SYLLOGISM)
    // =========================================================================

    /// Try disjunction elimination: if KB has A ∨ B and ¬A, conclude B (and vice versa).
    ///
    /// Disjunctive syllogism:
    /// - From A ∨ B and ¬A, derive B
    /// - From A ∨ B and ¬B, derive A
    fn try_disjunction_elimination(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Collect disjunctions from KB and context
        let disjunctions: Vec<(ProofExpr, ProofExpr, ProofExpr)> = self
            .knowledge_base
            .iter()
            .chain(goal.context.iter())
            .filter_map(|expr| {
                if let ProofExpr::Or(left, right) = expr {
                    Some((expr.clone(), (**left).clone(), (**right).clone()))
                } else {
                    None
                }
            })
            .collect();

        // Collect negations from KB and context
        let negations: Vec<ProofExpr> = self
            .knowledge_base
            .iter()
            .chain(goal.context.iter())
            .filter_map(|expr| {
                if let ProofExpr::Not(inner) = expr {
                    Some((**inner).clone())
                } else {
                    None
                }
            })
            .collect();

        // For each disjunction A ∨ B
        for (disj_expr, left, right) in &disjunctions {
            // Check if ¬left is in KB (so right must be true)
            for negated in &negations {
                if exprs_structurally_equal(negated, left) {
                    // We have A ∨ B and ¬A, so B is true
                    // Check if B matches our goal
                    if let Ok(subst) = unify_exprs(&goal.target, right) {
                        let disj_leaf = DerivationTree::leaf(
                            disj_expr.clone(),
                            InferenceRule::PremiseMatch,
                        );
                        let neg_leaf = DerivationTree::leaf(
                            ProofExpr::Not(Box::new(left.clone())),
                            InferenceRule::PremiseMatch,
                        );
                        return Ok(Some(
                            DerivationTree::new(
                                goal.target.clone(),
                                InferenceRule::DisjunctionElim,
                                vec![disj_leaf, neg_leaf],
                            )
                            .with_substitution(subst),
                        ));
                    }
                }

                // Check if ¬right is in KB (so left must be true)
                if exprs_structurally_equal(negated, right) {
                    // We have A ∨ B and ¬B, so A is true
                    // Check if A matches our goal
                    if let Ok(subst) = unify_exprs(&goal.target, left) {
                        let disj_leaf = DerivationTree::leaf(
                            disj_expr.clone(),
                            InferenceRule::PremiseMatch,
                        );
                        let neg_leaf = DerivationTree::leaf(
                            ProofExpr::Not(Box::new(right.clone())),
                            InferenceRule::PremiseMatch,
                        );
                        return Ok(Some(
                            DerivationTree::new(
                                goal.target.clone(),
                                InferenceRule::DisjunctionElim,
                                vec![disj_leaf, neg_leaf],
                            )
                            .with_substitution(subst),
                        ));
                    }
                }
            }
        }

        // No literal disjunction settled the goal. The disjunction the syllogism
        // needs is often itself a CONSEQUENCE — "Every color is red or blue" at a
        // witness yields `Red(c) ∨ Blue(c)` only after universal instantiation +
        // modus ponens. So, for a goal `B` and each known negation `¬A`, try to
        // PROVE the disjunction `A ∨ B` (or `B ∨ A`); if it goes through, the
        // proven disjunction becomes the first premise of the elimination. This
        // is the general elimination step every disjunctive-syllogism chain over
        // a closed domain relies on. The conclusion is a single disjunct, never
        // itself a disjunction — skipping `Or` goals keeps the recursive
        // disjunction proof from chasing ever-larger `A ∨ (A ∨ …)` targets.
        if depth < self.max_depth && !matches!(goal.target, ProofExpr::Or(..)) {
            for negated in &negations {
                if exprs_structurally_equal(negated, &goal.target) {
                    continue;
                }
                for orient_left in [true, false] {
                    let disjunction = if orient_left {
                        ProofExpr::Or(
                            Box::new(negated.clone()),
                            Box::new(goal.target.clone()),
                        )
                    } else {
                        ProofExpr::Or(
                            Box::new(goal.target.clone()),
                            Box::new(negated.clone()),
                        )
                    };
                    // Already covered by a literal disjunction above.
                    if disjunctions
                        .iter()
                        .any(|(d, _, _)| exprs_structurally_equal(d, &disjunction))
                    {
                        continue;
                    }
                    let disj_goal =
                        ProofGoal::with_context(disjunction.clone(), goal.context.clone());
                    // Derive the disjunction from a FACT or a universal rule's
                    // consequent only. Routing through full `prove_goal` would
                    // hand the `A ∨ B` goal to disjunction INTRODUCTION, which
                    // re-proves a disjunct — bouncing back to this very goal in an
                    // unbounded `Blue ↦ Red ∨ Blue ↦ Blue` cycle. A closed-domain
                    // disjunction is always a consequence, never an introduction.
                    let disj_proof = self
                        .try_match_fact(&disj_goal)?
                        .map(Ok)
                        .or_else(|| self.try_universal_inst(&disj_goal, depth + 1).transpose())
                        .transpose()?;
                    if let Some(disj_proof) = disj_proof {
                        let neg_leaf = DerivationTree::leaf(
                            ProofExpr::Not(Box::new(negated.clone())),
                            InferenceRule::PremiseMatch,
                        );
                        return Ok(Some(DerivationTree::new(
                            goal.target.clone(),
                            InferenceRule::DisjunctionElim,
                            vec![disj_proof, neg_leaf],
                        )));
                    }
                }
            }
        }

        // The negation the syllogism needs may itself have to be PROVED, not just
        // looked up. A closed-domain rule `∀x(ante(x) → (D₁(x) ∨ D₂(x)))` at a
        // witness yields `D₁(c) ∨ D₂(c)`; when the goal is one disjunct (say
        // `D₂(c)`) and the OTHER disjunct is refutable (`¬D₁(c)` follows from a
        // uniqueness/identity constraint), disjunctive syllogism still applies —
        // we derive the disjunction from the rule and prove `¬D₁(c)` as a
        // sub-goal (which the negation strategies discharge by reductio). This is
        // the elimination step a logic grid leans on once it has pinned a cell.
        if depth < self.max_depth && !matches!(goal.target, ProofExpr::Or(..)) {
            let disjunctive_rules: Vec<(ProofExpr, ProofExpr)> = self
                .knowledge_base
                .iter()
                .chain(goal.context.iter())
                .filter_map(|e| match e {
                    ProofExpr::ForAll { body, .. } => match body.as_ref() {
                        ProofExpr::Implies(_, cons) => match cons.as_ref() {
                            ProofExpr::Or(l, r) => Some(((**l).clone(), (**r).clone())),
                            _ => None,
                        },
                        _ => None,
                    },
                    ProofExpr::Implies(_, cons) => match cons.as_ref() {
                        ProofExpr::Or(l, r) => Some(((**l).clone(), (**r).clone())),
                        _ => None,
                    },
                    ProofExpr::Or(l, r) => Some(((**l).clone(), (**r).clone())),
                    _ => None,
                })
                .collect();

            for (d_left, d_right) in &disjunctive_rules {
                // Match the goal against one disjunct template; the OTHER, under
                // the same binding, is the disjunct to refute.
                for goal_is_right in [true, false] {
                    let (goal_template, other_template) = if goal_is_right {
                        (d_right, d_left)
                    } else {
                        (d_left, d_right)
                    };
                    let Ok(subst) = unify_exprs(&goal.target, goal_template) else {
                        continue;
                    };
                    let other = apply_subst_to_expr(other_template, &subst);
                    if self.contains_quantifier(&other) {
                        continue;
                    }
                    // The disjunction `A ∨ B` (in the rule's orientation) the
                    // elimination consumes. Derive it from the rule, never by
                    // introduction (which would re-enter this goal).
                    let goal_disjunct = apply_subst_to_expr(goal_template, &subst);
                    let disjunction = if goal_is_right {
                        ProofExpr::Or(Box::new(other.clone()), Box::new(goal_disjunct))
                    } else {
                        ProofExpr::Or(Box::new(goal_disjunct), Box::new(other.clone()))
                    };
                    let disj_goal =
                        ProofGoal::with_context(disjunction.clone(), goal.context.clone());
                    let disj_proof = match self.try_match_fact(&disj_goal)? {
                        Some(p) => Some(p),
                        None => match self.try_universal_inst(&disj_goal, depth + 1)? {
                            Some(p) => Some(p),
                            // A GROUNDED rule `ante → (A ∨ B)` yields the disjunction
                            // by modus ponens once `ante` is proved. Backward-chain
                            // into it: the implication's consequent IS the disjunction,
                            // so this derives it without disjunction introduction (no
                            // re-proving a single disjunct, no cycle).
                            None => self.try_backward_chain(&disj_goal, depth + 1)?,
                        },
                    };
                    let Some(disj_proof) = disj_proof else {
                        continue;
                    };
                    // Prove the negation of the other disjunct as a sub-goal.
                    let neg_goal = ProofGoal::with_context(
                        ProofExpr::Not(Box::new(other.clone())),
                        goal.context.clone(),
                    );
                    if let Ok(neg_proof) = self.prove_goal(neg_goal, depth + 1) {
                        return Ok(Some(DerivationTree::new(
                            goal.target.clone(),
                            InferenceRule::DisjunctionElim,
                            vec![disj_proof, neg_proof],
                        )));
                    }
                }
            }
        }

        // Last resort — general case analysis (intuitionistic ∨-elimination): for a
        // disjunction `A ∨ B` in the KB, prove the goal under EACH disjunct. Unlike
        // disjunctive syllogism above, no negation is needed. Skip a disjunction
        // whose disjunct is already in context (it has been decided — re-splitting
        // would loop). The certifier's `DisjunctionCases` binds each disjunct as a
        // local hypothesis that the branch proof cites.
        if depth < self.max_depth {
            for (disj_expr, left, right) in &disjunctions {
                let decided = goal.context.iter().any(|c| {
                    exprs_structurally_equal(c, left) || exprs_structurally_equal(c, right)
                });
                if decided {
                    continue;
                }
                let mut left_ctx = goal.context.clone();
                left_ctx.push(left.clone());
                let left_branch = match self.prove_goal(
                    ProofGoal::with_context(goal.target.clone(), left_ctx),
                    depth + 1,
                ) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                let mut right_ctx = goal.context.clone();
                right_ctx.push(right.clone());
                let right_branch = match self.prove_goal(
                    ProofGoal::with_context(goal.target.clone(), right_ctx),
                    depth + 1,
                ) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                let disj_leaf =
                    DerivationTree::leaf(disj_expr.clone(), InferenceRule::PremiseMatch);
                return Ok(Some(DerivationTree::new(
                    goal.target.clone(),
                    InferenceRule::DisjunctionCases,
                    vec![disj_leaf, left_branch, right_branch],
                )));
            }
        }

        Ok(None)
    }

    // =========================================================================
    // STRATEGY 5c: PROOF BY CONTRADICTION (REDUCTIO AD ABSURDUM)
    // =========================================================================

    /// Try proof by contradiction: to prove ¬P, assume P and derive a contradiction.
    ///
    /// This implements reductio ad absurdum:
    /// 1. To prove ¬∃x P(x), assume ∃x P(x), derive contradiction, conclude ¬∃x P(x)
    /// 2. To prove ¬P, assume P, derive contradiction, conclude ¬P
    ///
    /// A contradiction is detected when both Q and ¬Q are derivable.
    fn try_reductio_ad_absurdum(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Only apply to negation goals
        let assumed = match &goal.target {
            ProofExpr::Not(inner) => (**inner).clone(),
            _ => return Ok(None),
        };

        // Aggressive depth limit - reductio is expensive
        if depth > 5 {
            return Ok(None);
        }

        // Special handling for existence negation goals: ¬∃x P(x)
        // This is crucial for paradoxes like the Barber Paradox
        if let ProofExpr::Exists { .. } = &assumed {
            return self.try_existence_negation_proof(&goal, &assumed, depth);
        }

        // For non-existence goals, skip if they contain other quantifiers
        // (to avoid infinite loops with universal instantiation)
        if self.contains_quantifier(&assumed) {
            return Ok(None);
        }

        // Create a temporary context with the assumption added
        let mut extended_context = goal.context.clone();
        extended_context.push(assumed.clone());

        // Also Skolemize existentials from the assumption (but be careful)
        let skolemized = self.skolemize_existential(&assumed);
        for sk in &skolemized {
            extended_context.push(sk.clone());
        }

        // Prefer the gapless certifiable contradiction finder: its ⊥-derivation
        // certifies end to end (saturation + equality closure + bounded case
        // split), so the reductio it feeds becomes a kernel-checked `¬P`. Fall
        // back to the heuristic finder only when the certifiable one cannot close.
        if let Some(contradiction_proof) =
            self.find_certifiable_contradiction(&extended_context, depth)?
        {
            let assumption_leaf =
                DerivationTree::leaf(assumed.clone(), InferenceRule::PremiseMatch);
            return Ok(Some(DerivationTree::new(
                goal.target.clone(),
                InferenceRule::ReductioAdAbsurdum,
                vec![assumption_leaf, contradiction_proof],
            )));
        }

        // Look for contradiction in the extended context + KB
        // Note: find_contradiction does NOT call prove_goal recursively
        if let Some(contradiction_proof) = self.find_contradiction(&extended_context, depth)? {
            // Found a contradiction! Build the reductio proof
            let assumption_leaf = DerivationTree::leaf(
                assumed.clone(),
                InferenceRule::PremiseMatch,
            );

            return Ok(Some(DerivationTree::new(
                goal.target.clone(),
                InferenceRule::ReductioAdAbsurdum,
                vec![assumption_leaf, contradiction_proof],
            )));
        }

        Ok(None)
    }

    /// Try to prove ¬∃x P(x) by assuming ∃x P(x) and deriving contradiction.
    ///
    /// This is the core strategy for existence paradoxes like the Barber Paradox.
    /// Steps:
    /// 1. Assume ∃x P(x)
    /// 2. Skolemize to get P(c) for fresh constant c
    /// 3. Skolemize KB existentials (definite descriptions) to extract inner structure
    /// 4. Abstract event semantics to simple predicates
    /// 5. Instantiate universal premises with the Skolem constant
    /// 6. Extract uniqueness constraints and derive equalities
    /// 7. Look for contradiction (possibly via case analysis)
    fn try_existence_negation_proof(
        &mut self,
        goal: &ProofGoal,
        assumed_existence: &ProofExpr,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Skolemize the assumed existence: ∃x P(x) → P(c)
        let witness_facts = self.skolemize_existential(assumed_existence);

        if witness_facts.is_empty() {
            return Ok(None);
        }

        // Build extended context with witness facts
        let mut extended_context = goal.context.clone();
        extended_context.push(assumed_existence.clone());

        // Add witness facts, abstracting events
        for fact in &witness_facts {
            let abstracted = self.abstract_all_events(fact);
            if !extended_context.contains(&abstracted) {
                extended_context.push(abstracted);
            }
            if !extended_context.contains(fact) {
                extended_context.push(fact.clone());
            }
        }

        // Extract any Skolem constants from the witness facts
        let mut skolem_constants = self.extract_skolem_constants(&witness_facts);

        // CRITICAL: Skolemize KB existentials to extract definite description structure.
        // Natural language "The barber" creates:
        // ∃y ((barber(y) ∧ ∀z (barber(z) → z = y)) ∧ ∀x ...)
        // We need to Skolemize these to access the inner universals.
        let kb_skolemized = self.skolemize_kb_existentials();
        for fact in &kb_skolemized {
            let abstracted = self.abstract_all_events(fact);
            if !extended_context.contains(&abstracted) {
                extended_context.push(abstracted);
            }
            if !extended_context.contains(fact) {
                extended_context.push(fact.clone());
            }
        }

        // Extract additional Skolem constants from KB
        let kb_skolems = self.extract_skolem_constants(&kb_skolemized);
        for sk in kb_skolems {
            if !skolem_constants.contains(&sk) {
                skolem_constants.push(sk);
            }
        }

        // Also extract unified definite description constants (e.g., "the_barber")
        // These are created by unify_definite_descriptions and should be treated like Skolems
        for expr in &self.knowledge_base {
            self.collect_unified_constants(expr, &mut skolem_constants);
        }

        // Instantiate universal premises with Skolem constants
        let instantiated = self.instantiate_universals_with_constants(
            &extended_context,
            &skolem_constants,
        );
        for inst in &instantiated {
            let abstracted = self.abstract_all_events(inst);
            if !extended_context.contains(&abstracted) {
                extended_context.push(abstracted);
            }
        }

        // Also process KB universals
        let kb_instantiated = self.instantiate_kb_universals_with_constants(&skolem_constants);
        for inst in &kb_instantiated {
            let abstracted = self.abstract_all_events(inst);
            if !extended_context.contains(&abstracted) {
                extended_context.push(abstracted);
            }
        }

        // CRITICAL: Extract uniqueness constraints from definite descriptions
        // and derive equalities between Skolem constants and KB witnesses.
        // This handles Russell's definite descriptions: "The barber" creates
        // ∃y ((barber(y) ∧ ∀z (barber(z) → z = y)) ∧ ...)
        let derived_equalities = self.derive_equalities_from_uniqueness_constraints(
            &extended_context,
            &skolem_constants,
        );

        // Add derived equalities to context
        for eq in &derived_equalities {
            if !extended_context.contains(eq) {
                extended_context.push(eq.clone());
            }
        }

        // Apply derived equalities to substitute terms throughout context
        // This unifies facts about different barbers (sk_0, y, v) into a single entity
        let unified_context = self.apply_equalities_to_context(&extended_context, &derived_equalities);

        // Look for direct contradiction first (in unified context)
        if let Some(contradiction_proof) = self.find_contradiction(&unified_context, depth)? {
            let assumption_leaf = DerivationTree::leaf(
                assumed_existence.clone(),
                InferenceRule::PremiseMatch,
            );

            return Ok(Some(DerivationTree::new(
                goal.target.clone(),
                InferenceRule::ReductioAdAbsurdum,
                vec![assumption_leaf, contradiction_proof],
            )));
        }

        // Try case analysis for self-referential structures (like Barber Paradox)
        if let Some(case_proof) = self.try_case_analysis_contradiction(&unified_context, &skolem_constants, depth)? {
            let assumption_leaf = DerivationTree::leaf(
                assumed_existence.clone(),
                InferenceRule::PremiseMatch,
            );

            return Ok(Some(DerivationTree::new(
                goal.target.clone(),
                InferenceRule::ReductioAdAbsurdum,
                vec![assumption_leaf, case_proof],
            )));
        }

        Ok(None)
    }

    /// Skolemize all existential expressions in the KB.
    ///
    /// This is essential for definite descriptions from natural language.
    /// "The barber" creates `∃y ((barber(y) ∧ ∀z (barber(z) → z = y)) ∧ ...)`.
    /// We Skolemize to extract the inner structure.
    fn skolemize_kb_existentials(&mut self) -> Vec<ProofExpr> {
        let mut results = Vec::new();

        for expr in &self.knowledge_base.clone() {
            if let ProofExpr::Exists { .. } = expr {
                let skolemized = self.skolemize_existential(expr);
                results.extend(skolemized);
            }
        }

        results
    }

    // =========================================================================
    // EQUATIONAL REASONING FOR DEFINITE DESCRIPTIONS
    // =========================================================================

    /// Derive equalities from uniqueness constraints in definite descriptions.
    ///
    /// Given facts like `barber(sk_0)` and uniqueness constraints like
    /// `∀z (barber(z) → z = y)`, derive `sk_0 = y`.
    ///
    /// This is essential for Russell's definite descriptions where
    /// "The barber" creates `∃y ((barber(y) ∧ ∀z (barber(z) → z = y)) ∧ ...)`.
    fn derive_equalities_from_uniqueness_constraints(
        &self,
        context: &[ProofExpr],
        skolem_constants: &[String],
    ) -> Vec<ProofExpr> {
        let mut equalities = Vec::new();

        // Collect all uniqueness constraints from KB and context
        // Pattern: ∀z (P(z) → z = c) where c is a constant/variable
        let uniqueness_constraints = self.extract_uniqueness_constraints(context);

        // For each Skolem constant, check if it satisfies predicates
        // with uniqueness constraints
        for skolem in skolem_constants {
            for (predicate_name, unique_entity) in &uniqueness_constraints {
                // Check if we have predicate(skolem) in context
                let skolem_term = ProofTerm::Constant(skolem.clone());
                let skolem_satisfies_predicate = context.iter().any(|expr| {
                    self.predicate_matches(expr, predicate_name, &skolem_term)
                });

                if skolem_satisfies_predicate {
                    // Derive: skolem = unique_entity
                    let equality = ProofExpr::Identity(
                        skolem_term.clone(),
                        unique_entity.clone(),
                    );
                    if !equalities.contains(&equality) {
                        equalities.push(equality);
                    }

                    // Also add the symmetric version for easier matching
                    let sym_equality = ProofExpr::Identity(
                        unique_entity.clone(),
                        skolem_term.clone(),
                    );
                    if !equalities.contains(&sym_equality) {
                        equalities.push(sym_equality);
                    }
                }
            }
        }

        // Derive transitive equalities: if sk_0 = y and sk_0 = v, then y = v
        let mut transitive_equalities = Vec::new();
        for eq1 in &equalities {
            if let ProofExpr::Identity(t1, t2) = eq1 {
                for eq2 in &equalities {
                    if let ProofExpr::Identity(t3, t4) = eq2 {
                        // If t1 = t2 and t1 = t4, then t2 = t4
                        if t1 == t3 && t2 != t4 {
                            let trans_eq = ProofExpr::Identity(t2.clone(), t4.clone());
                            if !equalities.contains(&trans_eq) && !transitive_equalities.contains(&trans_eq) {
                                transitive_equalities.push(trans_eq);
                            }
                        }
                        // If t1 = t2 and t3 = t1, then t2 = t3
                        if t1 == t4 && t2 != t3 {
                            let trans_eq = ProofExpr::Identity(t2.clone(), t3.clone());
                            if !equalities.contains(&trans_eq) && !transitive_equalities.contains(&trans_eq) {
                                transitive_equalities.push(trans_eq);
                            }
                        }
                    }
                }
            }
        }
        equalities.extend(transitive_equalities);

        equalities
    }

    /// Extract uniqueness constraints from context and KB.
    ///
    /// Looks for patterns like `∀z (P(z) → z = c)` which establish
    /// that c is the unique entity satisfying P.
    fn extract_uniqueness_constraints(&self, context: &[ProofExpr]) -> Vec<(String, ProofTerm)> {
        let mut constraints = Vec::new();

        for expr in context.iter().chain(self.knowledge_base.iter()) {
            self.extract_uniqueness_from_expr(expr, &mut constraints);
        }

        constraints
    }

    /// Recursively extract uniqueness constraints from an expression.
    fn extract_uniqueness_from_expr(&self, expr: &ProofExpr, constraints: &mut Vec<(String, ProofTerm)>) {
        match expr {
            // Direct uniqueness pattern: ∀z (P(z) → z = c)
            ProofExpr::ForAll { variable, body } => {
                if let ProofExpr::Implies(ante, cons) = body.as_ref() {
                    if let ProofExpr::Identity(left, right) = cons.as_ref() {
                        // Check if it's "z = c" where z is the quantified variable
                        let var_term = ProofTerm::Variable(variable.clone());
                        if left == &var_term {
                            // Extract the predicate name from the antecedent
                            if let Some(pred_name) = self.extract_unary_predicate_name(ante, variable) {
                                // right is the unique entity
                                constraints.push((pred_name, right.clone()));
                            }
                        } else if right == &var_term {
                            // Check c = z form
                            if let Some(pred_name) = self.extract_unary_predicate_name(ante, variable) {
                                constraints.push((pred_name, left.clone()));
                            }
                        }
                    }
                }
                // Recurse into body for nested structures
                self.extract_uniqueness_from_expr(body, constraints);
            }

            // Conjunction: extract from both sides
            ProofExpr::And(left, right) => {
                self.extract_uniqueness_from_expr(left, constraints);
                self.extract_uniqueness_from_expr(right, constraints);
            }

            // Existential: extract from body (definite descriptions are wrapped in ∃)
            ProofExpr::Exists { body, .. } => {
                self.extract_uniqueness_from_expr(body, constraints);
            }

            _ => {}
        }
    }

    /// Extract the predicate name from a unary predicate application.
    ///
    /// Given P(z) where z is the variable, returns "P".
    fn extract_unary_predicate_name(&self, expr: &ProofExpr, var: &str) -> Option<String> {
        match expr {
            ProofExpr::Predicate { name, args, .. } => {
                if args.len() == 1 {
                    if let ProofTerm::Variable(v) = &args[0] {
                        if v == var {
                            return Some(name.clone());
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Check if an expression is a predicate with the given name applied to the term.
    fn predicate_matches(&self, expr: &ProofExpr, pred_name: &str, term: &ProofTerm) -> bool {
        match expr {
            ProofExpr::Predicate { name, args, .. } => {
                name == pred_name && args.len() == 1 && &args[0] == term
            }
            _ => false,
        }
    }

    /// Apply derived equalities to substitute terms throughout context.
    ///
    /// This unifies facts about different entities (sk_0, y, v) by replacing
    /// all occurrences with a canonical representative (the first Skolem constant).
    fn apply_equalities_to_context(
        &self,
        context: &[ProofExpr],
        equalities: &[ProofExpr],
    ) -> Vec<ProofExpr> {
        if equalities.is_empty() {
            return context.to_vec();
        }

        // Build a substitution map from equalities
        // Use the first term as the canonical representative
        let mut substitutions: Vec<(&ProofTerm, &ProofTerm)> = Vec::new();
        for eq in equalities {
            if let ProofExpr::Identity(t1, t2) = eq {
                // Prefer Skolem constants as canonical (they're from our assumption)
                if let ProofTerm::Constant(c) = t1 {
                    if c.starts_with("sk_") {
                        substitutions.push((t2, t1)); // t2 → t1 (Skolem)
                        continue;
                    }
                }
                if let ProofTerm::Constant(c) = t2 {
                    if c.starts_with("sk_") {
                        substitutions.push((t1, t2)); // t1 → t2 (Skolem)
                        continue;
                    }
                }
                // Default: first term is canonical
                substitutions.push((t2, t1));
            }
        }

        // Apply substitutions to each expression in context
        let mut unified_context = Vec::new();
        for expr in context {
            let mut unified = expr.clone();
            for (from, to) in &substitutions {
                unified = self.substitute_term_in_expr(&unified, from, to);
            }
            // Add abstracted version too
            let abstracted = self.abstract_all_events(&unified);
            if !unified_context.contains(&unified) {
                unified_context.push(unified);
            }
            if !unified_context.contains(&abstracted) {
                unified_context.push(abstracted);
            }
        }

        // Also add implications with substituted terms
        // This ensures cyclic implications like P(sk,sk) → ¬P(sk,sk) are in context
        for expr in context {
            if let ProofExpr::ForAll { variable, body } = expr {
                if let ProofExpr::Implies(_, _) = body.as_ref() {
                    // Find any Skolem constants and instantiate
                    for (from, to) in &substitutions {
                        if let ProofTerm::Constant(c) = to {
                            if c.starts_with("sk_") {
                                // Instantiate this universal with the Skolem constant
                                let mut subst = Substitution::new();
                                subst.insert(variable.clone(), (*to).clone());
                                let instantiated = apply_subst_to_expr(body, &subst);
                                let abstracted = self.abstract_all_events(&instantiated);
                                if !unified_context.contains(&abstracted) {
                                    unified_context.push(abstracted);
                                }
                            }
                        }
                    }
                }
            }
        }

        unified_context
    }

    /// Extract Skolem constants from a list of expressions.
    fn extract_skolem_constants(&self, exprs: &[ProofExpr]) -> Vec<String> {
        let mut constants = Vec::new();
        for expr in exprs {
            self.collect_skolem_constants_from_expr(expr, &mut constants);
        }
        constants.sort();
        constants.dedup();
        constants
    }

    /// Helper to collect Skolem constants from an expression.
    fn collect_skolem_constants_from_expr(&self, expr: &ProofExpr, constants: &mut Vec<String>) {
        match expr {
            ProofExpr::Predicate { args, .. } => {
                for arg in args {
                    self.collect_skolem_constants_from_term(arg, constants);
                }
            }
            ProofExpr::And(l, r) | ProofExpr::Or(l, r) | ProofExpr::Implies(l, r) | ProofExpr::Iff(l, r) => {
                self.collect_skolem_constants_from_expr(l, constants);
                self.collect_skolem_constants_from_expr(r, constants);
            }
            ProofExpr::Not(inner) => {
                self.collect_skolem_constants_from_expr(inner, constants);
            }
            ProofExpr::Identity(l, r) => {
                self.collect_skolem_constants_from_term(l, constants);
                self.collect_skolem_constants_from_term(r, constants);
            }
            ProofExpr::NeoEvent { roles, .. } => {
                for (_, term) in roles {
                    self.collect_skolem_constants_from_term(term, constants);
                }
            }
            _ => {}
        }
    }

    /// Helper to collect Skolem constants from a term.
    /// Collect unified definite description constants (e.g., "the_barber")
    /// These are created by unify_definite_descriptions and start with "the_".
    fn collect_unified_constants(&self, expr: &ProofExpr, constants: &mut Vec<String>) {
        match expr {
            ProofExpr::Predicate { args, .. } => {
                for arg in args {
                    if let ProofTerm::Constant(name) = arg {
                        if name.starts_with("the_") && !constants.contains(name) {
                            constants.push(name.clone());
                        }
                    }
                }
            }
            ProofExpr::And(left, right) | ProofExpr::Or(left, right) |
            ProofExpr::Implies(left, right) | ProofExpr::Iff(left, right) => {
                self.collect_unified_constants(left, constants);
                self.collect_unified_constants(right, constants);
            }
            ProofExpr::Not(inner) => self.collect_unified_constants(inner, constants),
            ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
                self.collect_unified_constants(body, constants);
            }
            ProofExpr::Identity(t1, t2) => {
                if let ProofTerm::Constant(name) = t1 {
                    if name.starts_with("the_") && !constants.contains(name) {
                        constants.push(name.clone());
                    }
                }
                if let ProofTerm::Constant(name) = t2 {
                    if name.starts_with("the_") && !constants.contains(name) {
                        constants.push(name.clone());
                    }
                }
            }
            _ => {}
        }
    }

    fn collect_skolem_constants_from_term(&self, term: &ProofTerm, constants: &mut Vec<String>) {
        match term {
            ProofTerm::Constant(name) if name.starts_with("sk_") => {
                constants.push(name.clone());
            }
            ProofTerm::Function(_, args) => {
                for arg in args {
                    self.collect_skolem_constants_from_term(arg, constants);
                }
            }
            ProofTerm::Group(terms) => {
                for t in terms {
                    self.collect_skolem_constants_from_term(t, constants);
                }
            }
            _ => {}
        }
    }

    /// Instantiate universal quantifiers in the context with given constants.
    fn instantiate_universals_with_constants(
        &self,
        context: &[ProofExpr],
        constants: &[String],
    ) -> Vec<ProofExpr> {
        let mut results = Vec::new();

        for expr in context {
            if let ProofExpr::ForAll { variable, body } = expr {
                for constant in constants {
                    let mut subst = Substitution::new();
                    subst.insert(variable.clone(), ProofTerm::Constant(constant.clone()));
                    let instantiated = apply_subst_to_expr(body, &subst);
                    results.push(instantiated);
                }
            }
        }

        results
    }

    /// Instantiate universal quantifiers in KB with given constants.
    fn instantiate_kb_universals_with_constants(&self, constants: &[String]) -> Vec<ProofExpr> {
        let mut results = Vec::new();

        for expr in &self.knowledge_base {
            if let ProofExpr::ForAll { variable, body } = expr {
                for constant in constants {
                    let mut subst = Substitution::new();
                    subst.insert(variable.clone(), ProofTerm::Constant(constant.clone()));
                    let instantiated = apply_subst_to_expr(body, &subst);
                    results.push(instantiated);
                }
            }
        }

        results
    }

    // =========================================================================
    // CASE ANALYSIS (TERTIUM NON DATUR)
    // =========================================================================

    /// Try case analysis to derive a contradiction.
    ///
    /// For self-referential structures like the Barber Paradox:
    /// - Split on a predicate P(c, c) where c is a Skolem constant
    /// - Case 1: Assume P(c, c), derive ¬P(c, c) → contradiction
    /// - Case 2: Assume ¬P(c, c), derive P(c, c) → contradiction
    /// Either way we get contradiction (law of excluded middle).
    fn try_case_analysis_contradiction(
        &mut self,
        context: &[ProofExpr],
        skolem_constants: &[String],
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Find candidate predicates for case splitting
        // Look for self-referential predicates: P(c, c) where c is a Skolem constant
        let candidates = self.find_case_split_candidates(context, skolem_constants);

        for candidate in candidates {
            // Case 1: Assume the candidate is true
            let mut context_with_pos = context.to_vec();
            if !context_with_pos.contains(&candidate) {
                context_with_pos.push(candidate.clone());
            }

            // Case 2: Assume the candidate is false
            let negated = ProofExpr::Not(Box::new(candidate.clone()));
            let mut context_with_neg = context.to_vec();
            if !context_with_neg.contains(&negated) {
                context_with_neg.push(negated.clone());
            }

            // Try to derive contradiction in both cases
            let case1_contradiction = self.find_contradiction(&context_with_pos, depth)?;
            let case2_contradiction = self.find_contradiction(&context_with_neg, depth)?;

            // If both cases lead to contradiction, we have a proof
            if let (Some(case1_proof), Some(case2_proof)) = (case1_contradiction, case2_contradiction) {
                // Build the case analysis proof tree
                let case1_tree = DerivationTree::new(
                    ProofExpr::Atom("⊥".into()),
                    InferenceRule::PremiseMatch,
                    vec![case1_proof],
                );
                let case2_tree = DerivationTree::new(
                    ProofExpr::Atom("⊥".into()),
                    InferenceRule::PremiseMatch,
                    vec![case2_proof],
                );

                return Ok(Some(DerivationTree::new(
                    ProofExpr::Atom("⊥".into()),
                    InferenceRule::CaseAnalysis {
                        case_formula: Box::new(candidate.clone()),
                    },
                    vec![case1_tree, case2_tree],
                )));
            }
        }

        Ok(None)
    }

    /// Find candidate predicates for case splitting.
    ///
    /// Looks for:
    /// 1. Self-referential predicates: P(c, c) where c is a Skolem constant
    /// 2. Predicates that appear in contradictory implications: P → ¬P and ¬P → P
    fn find_case_split_candidates(
        &self,
        context: &[ProofExpr],
        skolem_constants: &[String],
    ) -> Vec<ProofExpr> {
        let mut candidates = Vec::new();

        // Strategy 1: Find self-referential predicates P(c, c)
        for expr in context {
            if let ProofExpr::Predicate { name, args, world } = expr {
                // Check if it's a binary predicate with the same Skolem constant twice
                if args.len() == 2 {
                    if let (ProofTerm::Constant(c1), ProofTerm::Constant(c2)) = (&args[0], &args[1]) {
                        if c1 == c2 && skolem_constants.contains(c1) {
                            candidates.push(expr.clone());
                        }
                    }
                }
            }
        }

        // Strategy 2: Find predicates involved in cyclic implications
        // Look for patterns like: (P → ¬P) ∧ (¬P → P)
        let implications: Vec<(ProofExpr, ProofExpr)> = context.iter()
            .chain(self.knowledge_base.iter())
            .filter_map(|e| {
                if let ProofExpr::Implies(ante, cons) = e {
                    Some(((**ante).clone(), (**cons).clone()))
                } else {
                    None
                }
            })
            .collect();

        for (ante, cons) in &implications {
            // Check for P → ¬P pattern
            if let ProofExpr::Not(inner) = cons {
                if exprs_structurally_equal(ante, inner) {
                    // Found P → ¬P, check if ¬P → P also exists
                    let neg_ante = ProofExpr::Not(Box::new(ante.clone()));
                    for (a2, c2) in &implications {
                        if exprs_structurally_equal(a2, &neg_ante) && exprs_structurally_equal(c2, ante) {
                            // Found the cyclic pair - ante is a good candidate
                            if !candidates.contains(ante) {
                                candidates.push(ante.clone());
                            }
                        }
                    }
                }
            }
        }

        // Strategy 3: Generate self-referential predicates for Skolem constants
        // For each Skolem constant sk_N, look for predicates P and create P(sk_N, sk_N)
        for const_name in skolem_constants {
            // Look for action predicates in implications
            for expr in context.iter().chain(self.knowledge_base.iter()) {
                if let ProofExpr::Implies(ante, cons) = expr {
                    // Extract predicate names from consequences
                    self.extract_predicate_template(cons, const_name, &mut candidates);
                }
            }
        }

        candidates
    }

    /// Extract a predicate template and instantiate with a Skolem constant.
    fn extract_predicate_template(
        &self,
        expr: &ProofExpr,
        skolem: &str,
        candidates: &mut Vec<ProofExpr>,
    ) {
        match expr {
            ProofExpr::Predicate { name, args, world } if args.len() == 2 => {
                // Create a self-referential version: P(sk, sk)
                let self_ref = ProofExpr::Predicate {
                    name: name.clone(),
                    args: vec![
                        ProofTerm::Constant(skolem.to_string()),
                        ProofTerm::Constant(skolem.to_string()),
                    ],
                    world: world.clone(),
                };
                if !candidates.contains(&self_ref) {
                    candidates.push(self_ref);
                }
            }
            ProofExpr::Not(inner) => {
                self.extract_predicate_template(inner, skolem, candidates);
            }
            ProofExpr::NeoEvent { verb, .. } => {
                // Create abstracted predicate version
                let self_ref = ProofExpr::Predicate {
                    name: verb.to_lowercase(),
                    args: vec![
                        ProofTerm::Constant(skolem.to_string()),
                        ProofTerm::Constant(skolem.to_string()),
                    ],
                    world: None,
                };
                if !candidates.contains(&self_ref) {
                    candidates.push(self_ref);
                }
            }
            _ => {}
        }
    }

    // =========================================================================
    // STRATEGY 5d: EXISTENTIAL ELIMINATION
    // =========================================================================

    /// Try to eliminate existential quantifiers from premises.
    ///
    /// For each ∃x P(x) in the KB or context:
    /// 1. Generate a fresh Skolem constant c
    /// 2. Add P(c) to the context
    /// 3. Abstract any event semantics to simple predicates
    /// 4. Try to prove the goal with the extended context
    fn try_existential_elimination(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Depth guard to prevent infinite loops
        if depth > 8 {
            return Ok(None);
        }

        // Find existential expressions in KB and context
        let existentials: Vec<ProofExpr> = self.knowledge_base.iter()
            .chain(goal.context.iter())
            .filter(|e| matches!(e, ProofExpr::Exists { .. }))
            .cloned()
            .collect();

        if existentials.is_empty() {
            return Ok(None);
        }

        // Try eliminating each existential
        for exist_expr in existentials {
            // Open each existential at most once per branch — re-opening it with a
            // fresh witness adds no facts the first opening did not, and only feeds
            // the depth-limit/oracle loop.
            if self
                .eliminated_existentials
                .iter()
                .any(|e| exprs_structurally_equal(e, &exist_expr))
            {
                continue;
            }

            // Skolemize a single level, keeping the witness constant `c` so the
            // certificate's `Ex`-match binds the SAME constant the body is proved
            // over (the certifier mirrors this one elimination as one match).
            let Some((witness_const, witness_facts)) =
                self.skolemize_existential_with_const(&exist_expr)
            else {
                continue;
            };

            if witness_facts.is_empty() {
                continue;
            }

            // Abstract event semantics in witness facts
            let abstracted_facts: Vec<ProofExpr> = witness_facts.iter()
                .map(|f| self.abstract_all_events(f))
                .collect();

            // Build extended context with witness facts
            let mut extended_context = goal.context.clone();
            for fact in &abstracted_facts {
                if !extended_context.contains(fact) {
                    extended_context.push(fact.clone());
                }
            }

            // Also add the original witness facts (in case abstraction changes things)
            for fact in &witness_facts {
                if !extended_context.contains(fact) {
                    extended_context.push(fact.clone());
                }
            }

            // Try to prove the goal with the extended context
            let extended_goal = ProofGoal::with_context(goal.target.clone(), extended_context);

            // Mark this existential opened for the duration of the inner search,
            // then restore so sibling branches may open it themselves.
            self.eliminated_existentials.push(exist_expr.clone());
            let inner_result = self.prove_goal(extended_goal, depth + 1);
            self.eliminated_existentials.pop();

            if let Ok(inner_proof) = inner_result {
                // The existential premise is discharged from the KB/context, so a
                // `PremiseMatch` leaf carrying `∃x.P(x)` is its derivation. The
                // certifier consumes `[existential, body]` and produces a single
                // `Ex`-match bound to `witness_const`, with the witness body's
                // conjuncts available to the body proof as projected hypotheses.
                let existential_premise = DerivationTree::leaf(
                    exist_expr.clone(),
                    InferenceRule::PremiseMatch,
                );

                return Ok(Some(DerivationTree::new(
                    goal.target.clone(),
                    InferenceRule::ExistentialElim { witness: witness_const },
                    vec![existential_premise, inner_proof],
                )));
            }
        }

        Ok(None)
    }

    /// Check if an expression contains quantifiers.
    fn contains_quantifier(&self, expr: &ProofExpr) -> bool {
        match expr {
            ProofExpr::ForAll { .. } | ProofExpr::Exists { .. } => true,
            ProofExpr::And(l, r)
            | ProofExpr::Or(l, r)
            | ProofExpr::Implies(l, r)
            | ProofExpr::Iff(l, r) => self.contains_quantifier(l) || self.contains_quantifier(r),
            ProofExpr::Not(inner) => self.contains_quantifier(inner),
            _ => false,
        }
    }

    /// Skolemize an existential expression.
    ///
    /// Given ∃x P(x), introduce a fresh Skolem constant c and return P(c).
    /// For nested structures like ∃x((type(x) ∧ unique(x)) ∧ prop(x)),
    /// we extract the predicates with the Skolem constant.
    fn skolemize_existential(&mut self, expr: &ProofExpr) -> Vec<ProofExpr> {
        let mut results = Vec::new();

        if let ProofExpr::Exists { variable, body } = expr {
            // Generate a fresh Skolem constant
            let skolem = format!("sk_{}", self.fresh_var());

            // Apply substitution to the body
            let mut subst = Substitution::new();
            subst.insert(variable.clone(), ProofTerm::Constant(skolem.clone()));

            let instantiated = apply_subst_to_expr(body, &subst);

            // Flatten conjunctions into separate facts
            self.flatten_conjunction(&instantiated, &mut results);

            // Handle nested existentials in the result
            let mut i = 0;
            while i < results.len() {
                if let ProofExpr::Exists { .. } = &results[i] {
                    let nested = results.remove(i);
                    let nested_skolem = self.skolemize_existential(&nested);
                    results.extend(nested_skolem);
                } else {
                    i += 1;
                }
            }
        }

        results
    }

    /// Skolemize a single level of an existential, exposing the witness constant.
    ///
    /// Given `∃x.P(x)`, introduce a fresh Skolem constant `c` and return
    /// `(c, [conjuncts of P(c)])` — the body instantiated at `c` and flattened
    /// into its top-level conjuncts. Unlike [`skolemize_existential`], this does
    /// NOT recurse into nested existentials in the result: it eliminates exactly
    /// one quantifier so the certifier can mirror it with a single `Ex`-match
    /// bound to `c`. The returned constant is recorded as the `ExistentialElim`
    /// witness so the certificate's Match-bound variable lines up with the facts
    /// the body proof references.
    fn skolemize_existential_with_const(&mut self, expr: &ProofExpr) -> Option<(String, Vec<ProofExpr>)> {
        if let ProofExpr::Exists { variable, body } = expr {
            let skolem = format!("sk_{}", self.fresh_var());
            let mut subst = Substitution::new();
            subst.insert(variable.clone(), ProofTerm::Constant(skolem.clone()));
            let instantiated = apply_subst_to_expr(body, &subst);
            let mut results = Vec::new();
            self.flatten_conjunction(&instantiated, &mut results);
            Some((skolem, results))
        } else {
            None
        }
    }

    /// Flatten a conjunction into a list of its components.
    fn flatten_conjunction(&self, expr: &ProofExpr, results: &mut Vec<ProofExpr>) {
        match expr {
            ProofExpr::And(left, right) => {
                self.flatten_conjunction(left, results);
                self.flatten_conjunction(right, results);
            }
            other => results.push(other.clone()),
        }
    }

    // =========================================================================
    // DEFINITE DESCRIPTION SIMPLIFICATION
    // =========================================================================

    /// Check if a predicate is a tautological identity check: name(name)
    /// This occurs when parsing "the butler" creates butler(butler)
    fn is_tautological_identity(&self, expr: &ProofExpr) -> bool {
        if let ProofExpr::Predicate { name, args, .. } = expr {
            args.len() == 1 && matches!(
                &args[0],
                ProofTerm::Constant(c) | ProofTerm::BoundVarRef(c) | ProofTerm::Variable(c) if c == name
            )
        } else {
            false
        }
    }

    /// Simplify conjunction by removing tautological identity predicates.
    /// (butler(butler) ∧ P) → P when butler is a constant
    fn simplify_definite_description_conjunction(&self, expr: &ProofExpr) -> ProofExpr {
        match expr {
            ProofExpr::And(left, right) => {
                // First simplify children
                let left_simplified = self.simplify_definite_description_conjunction(left);
                let right_simplified = self.simplify_definite_description_conjunction(right);

                // Remove tautological identities from the conjunction
                if self.is_tautological_identity(&left_simplified) {
                    return right_simplified;
                }
                if self.is_tautological_identity(&right_simplified) {
                    return left_simplified;
                }

                ProofExpr::And(
                    Box::new(left_simplified),
                    Box::new(right_simplified),
                )
            }
            ProofExpr::Or(left, right) => ProofExpr::Or(
                Box::new(self.simplify_definite_description_conjunction(left)),
                Box::new(self.simplify_definite_description_conjunction(right)),
            ),
            ProofExpr::Implies(left, right) => ProofExpr::Implies(
                Box::new(self.simplify_definite_description_conjunction(left)),
                Box::new(self.simplify_definite_description_conjunction(right)),
            ),
            ProofExpr::Iff(left, right) => ProofExpr::Iff(
                Box::new(self.simplify_definite_description_conjunction(left)),
                Box::new(self.simplify_definite_description_conjunction(right)),
            ),
            ProofExpr::Not(inner) => ProofExpr::Not(
                Box::new(self.simplify_definite_description_conjunction(inner)),
            ),
            ProofExpr::ForAll { variable, body } => ProofExpr::ForAll {
                variable: variable.clone(),
                body: Box::new(self.simplify_definite_description_conjunction(body)),
            },
            ProofExpr::Exists { variable, body } => ProofExpr::Exists {
                variable: variable.clone(),
                body: Box::new(self.simplify_definite_description_conjunction(body)),
            },
            ProofExpr::Temporal { operator, body } => ProofExpr::Temporal {
                operator: operator.clone(),
                body: Box::new(self.simplify_definite_description_conjunction(body)),
            },
            ProofExpr::TemporalBinary { operator, left, right } => ProofExpr::TemporalBinary {
                operator: operator.clone(),
                left: Box::new(self.simplify_definite_description_conjunction(left)),
                right: Box::new(self.simplify_definite_description_conjunction(right)),
            },
            _ => expr.clone(),
        }
    }

    // =========================================================================
    // EVENT SEMANTICS ABSTRACTION
    // =========================================================================

    /// Abstract Neo-Davidsonian event semantics to simple predicates.
    ///
    /// Converts: ∃e(Shave(e) ∧ Agent(e, x) ∧ Theme(e, y)) → shaves(x, y)
    ///
    /// This allows the proof engine to reason about events using simpler
    /// predicate logic, which is essential for paradoxes like the Barber Paradox.
    fn abstract_event_to_predicate(&self, expr: &ProofExpr) -> Option<ProofExpr> {
        match expr {
            // Direct NeoEvent abstraction
            ProofExpr::NeoEvent { verb, roles, .. } => {
                // Extract Agent and Theme/Patient roles
                let agent = roles.iter()
                    .find(|(role, _)| role == "Agent")
                    .map(|(_, term)| term.clone());

                let theme = roles.iter()
                    .find(|(role, _)| role == "Theme" || role == "Patient")
                    .map(|(_, term)| term.clone());

                // Build a simple predicate: verb(agent, theme) or verb(agent)
                let mut args = Vec::new();
                if let Some(a) = agent {
                    args.push(a);
                }
                if let Some(t) = theme {
                    args.push(t);
                }

                // Lowercase the verb for predicate naming convention
                let pred_name = verb.to_lowercase();

                Some(ProofExpr::Predicate {
                    name: pred_name,
                    args,
                    world: None,
                })
            }

            // Handle Exists wrapping an event expression
            ProofExpr::Exists { variable, body } => {
                // Check if this is an event quantification
                if !self.is_event_variable(variable) {
                    return None;
                }

                // Try direct NeoEvent abstraction
                if let Some(abstracted) = self.abstract_event_to_predicate(body) {
                    return Some(abstracted);
                }

                // Try to parse conjunction of event predicates
                // Pattern: ∃e(Verb(e) ∧ Agent(e, x) ∧ Theme(e, y)) → verb(x, y)
                if let Some(abstracted) = self.abstract_event_conjunction(variable, body) {
                    return Some(abstracted);
                }

                None
            }

            _ => None,
        }
    }

    /// Abstract a conjunction of event predicates to a simple predicate.
    ///
    /// Handles: Verb(e) ∧ Agent(e, x) ∧ Theme(e, y) → verb(x, y)
    fn abstract_event_conjunction(&self, event_var: &str, body: &ProofExpr) -> Option<ProofExpr> {
        // Flatten the conjunction to get all components
        let mut components = Vec::new();
        self.flatten_conjunction(body, &mut components);

        // Find verb predicate (single arg that matches event_var)
        let mut verb_name: Option<String> = None;
        let mut agent: Option<ProofTerm> = None;
        let mut theme: Option<ProofTerm> = None;

        for comp in &components {
            if let ProofExpr::Predicate { name, args, .. } = comp {
                // Check if first arg is the event variable
                let first_is_event = args.first().map_or(false, |arg| {
                    matches!(arg, ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) if v == event_var)
                });

                if !first_is_event && args.len() == 1 {
                    // Single arg predicate that's the event var
                    if let Some(ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v)) = args.first() {
                        if v == event_var {
                            verb_name = Some(name.clone());
                            continue;
                        }
                    }
                }

                if first_is_event {
                    match name.as_str() {
                        "Agent" if args.len() == 2 => {
                            agent = Some(args[1].clone());
                        }
                        "Theme" | "Patient" if args.len() == 2 => {
                            theme = Some(args[1].clone());
                        }
                        _ if args.len() == 1 && verb_name.is_none() => {
                            // This is probably the verb predicate: Verb(e)
                            verb_name = Some(name.clone());
                        }
                        _ => {}
                    }
                }
            }
        }

        // If we found a verb, construct the simple predicate
        if let Some(verb) = verb_name {
            let mut args = Vec::new();
            if let Some(a) = agent {
                args.push(a);
            }
            if let Some(t) = theme {
                args.push(t);
            }

            return Some(ProofExpr::Predicate {
                name: verb.to_lowercase(),
                args,
                world: None,
            });
        }

        None
    }

    /// Check if a variable name looks like an event variable.
    ///
    /// Event variables are typically named "e", "e1", "e2", etc.
    fn is_event_variable(&self, var: &str) -> bool {
        var == "e" || var.starts_with("e_") ||
        (var.starts_with('e') && var.len() == 2 && var.chars().nth(1).map_or(false, |c| c.is_ascii_digit()))
    }

    /// Recursively abstract all events in an expression.
    ///
    /// This transforms the entire expression tree, replacing event semantics
    /// with simple predicates wherever possible. `pub(crate)` so the
    /// verify/certify boundary can register hypotheses and the goal in the
    /// SAME abstracted language the search runs in.
    pub(crate) fn abstract_all_events(&self, expr: &ProofExpr) -> ProofExpr {
        // First try direct abstraction
        if let Some(abstracted) = self.abstract_event_to_predicate(expr) {
            return abstracted;
        }

        // Otherwise recurse into the structure
        match expr {
            ProofExpr::And(left, right) => ProofExpr::And(
                Box::new(self.abstract_all_events(left)),
                Box::new(self.abstract_all_events(right)),
            ),
            ProofExpr::Or(left, right) => ProofExpr::Or(
                Box::new(self.abstract_all_events(left)),
                Box::new(self.abstract_all_events(right)),
            ),
            ProofExpr::Implies(left, right) => ProofExpr::Implies(
                Box::new(self.abstract_all_events(left)),
                Box::new(self.abstract_all_events(right)),
            ),
            ProofExpr::Iff(left, right) => ProofExpr::Iff(
                Box::new(self.abstract_all_events(left)),
                Box::new(self.abstract_all_events(right)),
            ),
            ProofExpr::Not(inner) => {
                // Apply De Morgan for quantifiers: ¬∃x.P ≡ ∀x.¬P
                // This normalization is crucial for efficient proof search
                // (Converting negated existentials to universals helps the prover)
                if let ProofExpr::Exists { variable, body } = inner.as_ref() {
                    return ProofExpr::ForAll {
                        variable: variable.clone(),
                        body: Box::new(self.abstract_all_events(&ProofExpr::Not(body.clone()))),
                    };
                }
                // Note: We do NOT convert ¬∀x.P to ∃x.¬P because the prover
                // works better with universal quantifiers for backward chaining.
                ProofExpr::Not(Box::new(self.abstract_all_events(inner)))
            }
            ProofExpr::ForAll { variable, body } => {
                // Check for pattern: ∀x ¬(P ∧ Q) → ∀x (P → ¬Q)
                // This converts to implication form for better backward chaining
                if let ProofExpr::Not(inner) = body.as_ref() {
                    if let ProofExpr::And(left, right) = inner.as_ref() {
                        return ProofExpr::ForAll {
                            variable: variable.clone(),
                            body: Box::new(ProofExpr::Implies(
                                Box::new(self.abstract_all_events(left)),
                                Box::new(self.abstract_all_events(&ProofExpr::Not(right.clone()))),
                            )),
                        };
                    }
                }
                ProofExpr::ForAll {
                    variable: variable.clone(),
                    body: Box::new(self.abstract_all_events(body)),
                }
            }
            ProofExpr::Exists { variable, body } => {
                // Check if this is an event quantification that should be abstracted
                if self.is_event_variable(variable) {
                    if let Some(abstracted) = self.abstract_event_to_predicate(body) {
                        return abstracted;
                    }
                }
                // Otherwise keep the existential and recurse
                ProofExpr::Exists {
                    variable: variable.clone(),
                    body: Box::new(self.abstract_all_events(body)),
                }
            }
            // For other expressions, return as-is
            other => other.clone(),
        }
    }

    /// Abstract event semantics WITHOUT applying De Morgan transformations.
    ///
    /// This is used for goals where we want to preserve the ¬∃ pattern
    /// for reductio ad absurdum strategies.
    fn abstract_events_only(&self, expr: &ProofExpr) -> ProofExpr {
        // First try direct abstraction
        if let Some(abstracted) = self.abstract_event_to_predicate(expr) {
            return abstracted;
        }

        // Otherwise recurse into the structure
        match expr {
            ProofExpr::And(left, right) => ProofExpr::And(
                Box::new(self.abstract_events_only(left)),
                Box::new(self.abstract_events_only(right)),
            ),
            ProofExpr::Or(left, right) => ProofExpr::Or(
                Box::new(self.abstract_events_only(left)),
                Box::new(self.abstract_events_only(right)),
            ),
            ProofExpr::Implies(left, right) => ProofExpr::Implies(
                Box::new(self.abstract_events_only(left)),
                Box::new(self.abstract_events_only(right)),
            ),
            ProofExpr::Iff(left, right) => ProofExpr::Iff(
                Box::new(self.abstract_events_only(left)),
                Box::new(self.abstract_events_only(right)),
            ),
            ProofExpr::Not(inner) => {
                // Just recurse, no De Morgan transformation
                ProofExpr::Not(Box::new(self.abstract_events_only(inner)))
            }
            ProofExpr::ForAll { variable, body } => ProofExpr::ForAll {
                variable: variable.clone(),
                body: Box::new(self.abstract_events_only(body)),
            },
            ProofExpr::Exists { variable, body } => {
                // Check if this is an event quantification that should be abstracted
                if self.is_event_variable(variable) {
                    if let Some(abstracted) = self.abstract_event_to_predicate(body) {
                        return abstracted;
                    }
                }
                // Otherwise keep the existential and recurse
                ProofExpr::Exists {
                    variable: variable.clone(),
                    body: Box::new(self.abstract_events_only(body)),
                }
            }
            // For other expressions, return as-is
            other => other.clone(),
        }
    }

    /// Look for a contradiction in the knowledge base and context.
    ///
    /// A contradiction exists when both P and ¬P are derivable.
    /// Find a contradiction in KB + context and return a **gapless** derivation
    /// the certifier can turn into a kernel proof of `False`.
    ///
    /// Unlike the heuristic [`find_contradiction`], every step here is justified
    /// by an explicit rule — `PremiseMatch`, `UniversalInst`, `ModusPonens`,
    /// `Contradiction` — so the resulting tree certifies end-to-end. It forward-
    /// chains over Horn-style rules (`A → C` and `∀x. A(x) → C(x)`) until some
    /// proposition `P` and its negation `¬P` are both derivable.
    ///
    /// This powers verified conflict detection on clean predicate rule sets. The
    /// heuristic finder remains for the paradox/reductio path (which needs a
    /// separate gapless-emission pass before it can certify).
    fn find_certifiable_contradiction(
        &self,
        context: &[ProofExpr],
        _depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        let all: Vec<ProofExpr> = self
            .knowledge_base
            .iter()
            .chain(context.iter())
            .cloned()
            .collect();
        let rules = cert_extract_rules(&all);
        let seed = cert_seed_facts(&all);
        // `depth` bounds nested case splits / DPLL decisions. A small grid's
        // determined cells resolve by propagation within a few decisions; kept
        // moderate so the per-cell solve stays fast.
        let mut fresh = 0u32;
        Ok(cert_derive_falsum(&rules, &seed, 6, &mut fresh))
    }

    fn find_contradiction(
        &mut self,
        context: &[ProofExpr],
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Collect all expressions from KB and context
        let all_exprs: Vec<ProofExpr> = self.knowledge_base.iter()
            .chain(context.iter())
            .cloned()
            .collect();

        // Strategy 1: Look for direct P and ¬P pairs
        for expr in &all_exprs {
            if let ProofExpr::Not(inner) = expr {
                // We have ¬P, check if P exists directly
                for other in &all_exprs {
                    if exprs_structurally_equal(other, inner) {
                        // Found both P and ¬P directly
                        let pos_leaf = DerivationTree::leaf(
                            (**inner).clone(),
                            InferenceRule::PremiseMatch,
                        );
                        let neg_leaf = DerivationTree::leaf(
                            expr.clone(),
                            InferenceRule::PremiseMatch,
                        );
                        return Ok(Some(DerivationTree::new(
                            ProofExpr::Atom("⊥".into()),
                            InferenceRule::Contradiction,
                            vec![pos_leaf, neg_leaf],
                        )));
                    }
                }
            }
        }

        // Strategy 2: Look for implications that derive contradictory results
        // Check if context fact P triggers P → ¬P (immediate contradiction)
        // Or if P triggers P → Q where ¬Q is also in context
        // Note: Extract implications from both top-level and inside ForAll quantifiers
        let mut implications: Vec<(ProofExpr, ProofExpr)> = Vec::new();
        for e in &all_exprs {
            if let ProofExpr::Implies(ante, cons) = e {
                implications.push(((**ante).clone(), (**cons).clone()));
            }
            // Also extract from inside ForAll (important for barber paradox!)
            if let ProofExpr::ForAll { body, .. } = e {
                if let ProofExpr::Implies(ante, cons) = body.as_ref() {
                    implications.push(((**ante).clone(), (**cons).clone()));
                }
            }
        }

        // For each fact in the context, see if it triggers contradictory implications
        for fact in context {
            // Find all implications where fact matches the antecedent
            let mut derivable_consequences: Vec<ProofExpr> = Vec::new();

            for (ante, cons) in &implications {
                // Try to unify the antecedent with the fact
                if let Ok(subst) = unify_exprs(fact, ante) {
                    let instantiated_cons = apply_subst_to_expr(cons, &subst);
                    derivable_consequences.push(instantiated_cons);
                }

                // Also try matching conjunctive antecedents with multiple facts
                if let ProofExpr::And(left, right) = ante {
                    // Try to find facts matching both parts of the conjunction
                    if let Some(subst) = self.try_match_conjunction_antecedent(
                        left, right, &all_exprs
                    ) {
                        let instantiated_cons = apply_subst_to_expr(cons, &subst);
                        if !derivable_consequences.contains(&instantiated_cons) {
                            derivable_consequences.push(instantiated_cons);
                        }
                    }
                }
            }

            // Check if any derived consequence contradicts the triggering fact
            for cons in &derivable_consequences {
                // Check if cons = ¬fact (the classic barber structure: P → ¬P)
                if let ProofExpr::Not(inner) = cons {
                    if exprs_structurally_equal(inner, fact) {
                        // fact triggered an implication that derives ¬fact
                        // This is a contradiction: fact ∧ ¬fact
                        let pos_leaf = DerivationTree::leaf(
                            fact.clone(),
                            InferenceRule::PremiseMatch,
                        );
                        let neg_leaf = DerivationTree::leaf(
                            cons.clone(),
                            InferenceRule::ModusPonens,
                        );
                        return Ok(Some(DerivationTree::new(
                            ProofExpr::Atom("⊥".into()),
                            InferenceRule::Contradiction,
                            vec![pos_leaf, neg_leaf],
                        )));
                    }
                }

                // Check if cons contradicts any other fact in context
                for other in context {
                    if std::ptr::eq(fact as *const _, other as *const _) {
                        continue; // Skip the triggering fact itself
                    }
                    // Check if cons = ¬other
                    if let ProofExpr::Not(inner) = cons {
                        if exprs_structurally_equal(inner, other) {
                            let pos_leaf = DerivationTree::leaf(
                                other.clone(),
                                InferenceRule::PremiseMatch,
                            );
                            let neg_leaf = DerivationTree::leaf(
                                cons.clone(),
                                InferenceRule::ModusPonens,
                            );
                            return Ok(Some(DerivationTree::new(
                                ProofExpr::Atom("⊥".into()),
                                InferenceRule::Contradiction,
                                vec![pos_leaf, neg_leaf],
                            )));
                        }
                    }
                    // Check if other = ¬cons
                    if let ProofExpr::Not(inner_other) = other {
                        if exprs_structurally_equal(inner_other, cons) {
                            let pos_leaf = DerivationTree::leaf(
                                cons.clone(),
                                InferenceRule::ModusPonens,
                            );
                            let neg_leaf = DerivationTree::leaf(
                                other.clone(),
                                InferenceRule::PremiseMatch,
                            );
                            return Ok(Some(DerivationTree::new(
                                ProofExpr::Atom("⊥".into()),
                                InferenceRule::Contradiction,
                                vec![pos_leaf, neg_leaf],
                            )));
                        }
                    }
                }
            }

            // Check if any pair of consequences contradicts each other
            for i in 0..derivable_consequences.len() {
                for j in (i + 1)..derivable_consequences.len() {
                    let cons1 = &derivable_consequences[i];
                    let cons2 = &derivable_consequences[j];

                    // Check if cons1 = ¬cons2 or cons2 = ¬cons1
                    if let ProofExpr::Not(inner1) = cons1 {
                        if exprs_structurally_equal(inner1, cons2) {
                            // cons1 = ¬cons2, contradiction!
                            let pos_leaf = DerivationTree::leaf(
                                cons2.clone(),
                                InferenceRule::ModusPonens,
                            );
                            let neg_leaf = DerivationTree::leaf(
                                cons1.clone(),
                                InferenceRule::ModusPonens,
                            );
                            return Ok(Some(DerivationTree::new(
                                ProofExpr::Atom("⊥".into()),
                                InferenceRule::Contradiction,
                                vec![pos_leaf, neg_leaf],
                            )));
                        }
                    }
                    if let ProofExpr::Not(inner2) = cons2 {
                        if exprs_structurally_equal(inner2, cons1) {
                            // cons2 = ¬cons1, contradiction!
                            let pos_leaf = DerivationTree::leaf(
                                cons1.clone(),
                                InferenceRule::ModusPonens,
                            );
                            let neg_leaf = DerivationTree::leaf(
                                cons2.clone(),
                                InferenceRule::ModusPonens,
                            );
                            return Ok(Some(DerivationTree::new(
                                ProofExpr::Atom("⊥".into()),
                                InferenceRule::Contradiction,
                                vec![pos_leaf, neg_leaf],
                            )));
                        }
                    }
                }
            }
        }

        // Strategy 3: Try to find self-referential contradictions (like Barber Paradox)
        if let Some(proof) = self.find_self_referential_contradiction(context, depth)? {
            return Ok(Some(proof));
        }

        Ok(None)
    }

    /// Try to match a conjunctive antecedent with facts in the context.
    ///
    /// For an antecedent like (man(z) ∧ shave(z,z)), we need to find facts
    /// that match both parts with consistent variable bindings.
    fn try_match_conjunction_antecedent(
        &self,
        left: &ProofExpr,
        right: &ProofExpr,
        facts: &[ProofExpr],
    ) -> Option<Substitution> {
        // Try to find a fact that matches the left part
        for fact1 in facts {
            if let Ok(subst1) = unify_exprs(fact1, left) {
                // Apply this substitution to the right part
                let instantiated_right = apply_subst_to_expr(right, &subst1);
                // Now look for a fact that matches the instantiated right part
                for fact2 in facts {
                    if let Ok(subst2) = unify_exprs(fact2, &instantiated_right) {
                        // Combine substitutions
                        let mut combined = subst1.clone();
                        for (k, v) in subst2.iter() {
                            combined.insert(k.clone(), v.clone());
                        }
                        return Some(combined);
                    }
                }
            }
        }
        // Also try right then left
        for fact1 in facts {
            if let Ok(subst1) = unify_exprs(fact1, right) {
                let instantiated_left = apply_subst_to_expr(left, &subst1);
                for fact2 in facts {
                    if let Ok(subst2) = unify_exprs(fact2, &instantiated_left) {
                        let mut combined = subst1.clone();
                        for (k, v) in subst2.iter() {
                            combined.insert(k.clone(), v.clone());
                        }
                        return Some(combined);
                    }
                }
            }
        }
        None
    }

    /// Special case: find self-referential contradictions (like the Barber Paradox).
    ///
    /// Pattern: If we have ∀x(P(x) → Q(b, x)) and ∀x(P(x) → ¬Q(b, x)),
    /// then for x = b with P(b), we get Q(b, b) ∧ ¬Q(b, b).
    ///
    /// This uses direct pattern matching WITHOUT recursive prove_goal calls
    /// to avoid infinite recursion.
    fn find_self_referential_contradiction(
        &mut self,
        context: &[ProofExpr],
        _depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Collect all expressions from KB and context
        let all_exprs: Vec<ProofExpr> = self.knowledge_base.iter()
            .chain(context.iter())
            .cloned()
            .collect();

        // Look for pairs of universal implications with contradictory conclusions
        // that can be instantiated with the same witness
        for expr1 in &all_exprs {
            if let ProofExpr::ForAll { variable: var1, body: body1 } = expr1 {
                if let ProofExpr::Implies(ante1, cons1) = body1.as_ref() {
                    for expr2 in &all_exprs {
                        if std::ptr::eq(expr1, expr2) {
                            continue; // Skip same expression
                        }
                        if let ProofExpr::ForAll { variable: var2, body: body2 } = expr2 {
                            if let ProofExpr::Implies(ante2, cons2) = body2.as_ref() {
                                // Check if cons2 = ¬cons1 (structurally)
                                if let ProofExpr::Not(neg_cons2) = cons2.as_ref() {
                                    // Check if cons1 and neg_cons2 have matching structure
                                    // For barber: cons1 = shaves(barber, x), neg_cons2 = shaves(barber, x)

                                    // Try instantiating with x = barber (the self-referential case)
                                    // We look for constant terms in cons1 that could be witnesses
                                    let witnesses = self.extract_constants_from_expr(cons1);

                                    for witness_name in &witnesses {
                                        let witness = ProofTerm::Constant(witness_name.clone());

                                        // Instantiate both antecedents and consequents with this witness
                                        let mut subst1 = Substitution::new();
                                        subst1.insert(var1.clone(), witness.clone());
                                        let ante1_inst = apply_subst_to_expr(ante1, &subst1);
                                        let cons1_inst = apply_subst_to_expr(cons1, &subst1);

                                        let mut subst2 = Substitution::new();
                                        subst2.insert(var2.clone(), witness.clone());
                                        let ante2_inst = apply_subst_to_expr(ante2, &subst2);
                                        let cons2_inst = apply_subst_to_expr(cons2, &subst2);

                                        // Check if cons1_inst and ¬cons2_inst contradict
                                        // cons2_inst should be ¬X where X = cons1_inst
                                        if let ProofExpr::Not(inner2) = &cons2_inst {
                                            if exprs_structurally_equal(&cons1_inst, inner2) {
                                                // Now check if both antecedents could hold
                                                // ante1 typically is ¬P(x,x) and ante2 is P(x,x)
                                                // These are complementary - one must hold
                                                // For the paradox, we consider BOTH cases

                                                // If ante1 = ¬P(x,x) and ante2 = P(x,x), and x = witness,
                                                // we have a tertium non datur case:
                                                // - Either P(w,w) holds → cons2_inst = ¬cons1_inst
                                                // - Or ¬P(w,w) holds → cons1_inst

                                                // Check if ante1 and ante2 are complements
                                                if self.are_complements(&ante1_inst, &ante2_inst) {
                                                    // By excluded middle, one antecedent holds
                                                    // If cons1_inst and cons2_inst = ¬cons1_inst,
                                                    // we have a contradiction
                                                    let pos_leaf = DerivationTree::leaf(
                                                        cons1_inst.clone(),
                                                        InferenceRule::ModusPonens,
                                                    );
                                                    let neg_leaf = DerivationTree::leaf(
                                                        cons2_inst,
                                                        InferenceRule::ModusPonens,
                                                    );
                                                    return Ok(Some(DerivationTree::new(
                                                        ProofExpr::Atom("⊥".into()),
                                                        InferenceRule::Contradiction,
                                                        vec![pos_leaf, neg_leaf],
                                                    )));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Check if two expressions are complements (one is the negation of the other).
    fn are_complements(&self, expr1: &ProofExpr, expr2: &ProofExpr) -> bool {
        // Check if expr1 = ¬expr2
        if let ProofExpr::Not(inner1) = expr1 {
            if exprs_structurally_equal(inner1, expr2) {
                return true;
            }
        }
        // Check if expr2 = ¬expr1
        if let ProofExpr::Not(inner2) = expr2 {
            if exprs_structurally_equal(inner2, expr1) {
                return true;
            }
        }
        false
    }

    /// Extract constant names from an expression.
    fn extract_constants_from_expr(&self, expr: &ProofExpr) -> Vec<String> {
        let mut constants = Vec::new();
        self.extract_constants_recursive(expr, &mut constants);
        constants
    }

    fn extract_constants_recursive(&self, expr: &ProofExpr, constants: &mut Vec<String>) {
        match expr {
            ProofExpr::Predicate { args, .. } => {
                for arg in args {
                    self.extract_constants_from_term_recursive(arg, constants);
                }
            }
            ProofExpr::Identity(l, r) => {
                self.extract_constants_from_term_recursive(l, constants);
                self.extract_constants_from_term_recursive(r, constants);
            }
            ProofExpr::And(l, r)
            | ProofExpr::Or(l, r)
            | ProofExpr::Implies(l, r)
            | ProofExpr::Iff(l, r) => {
                self.extract_constants_recursive(l, constants);
                self.extract_constants_recursive(r, constants);
            }
            ProofExpr::Not(inner) => {
                self.extract_constants_recursive(inner, constants);
            }
            ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
                self.extract_constants_recursive(body, constants);
            }
            _ => {}
        }
    }

    fn extract_constants_from_term_recursive(&self, term: &ProofTerm, constants: &mut Vec<String>) {
        match term {
            ProofTerm::Constant(name) => {
                if !constants.contains(name) {
                    constants.push(name.clone());
                }
            }
            ProofTerm::Function(_, args) => {
                for arg in args {
                    self.extract_constants_from_term_recursive(arg, constants);
                }
            }
            ProofTerm::Group(terms) => {
                for t in terms {
                    self.extract_constants_from_term_recursive(t, constants);
                }
            }
            _ => {}
        }
    }

    // =========================================================================
    // STRATEGY 6: EQUALITY REWRITING (LEIBNIZ'S LAW)
    // =========================================================================

    /// Try rewriting using equalities in the knowledge base.
    ///
    /// Leibniz's Law: If a = b and P(a), then P(b).
    /// Also handles symmetry (a = b ⊢ b = a) and transitivity (a = b, b = c ⊢ a = c).
    fn try_equality_rewrite(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Collect equalities from KB and context
        let equalities: Vec<(ProofTerm, ProofTerm)> = self
            .knowledge_base
            .iter()
            .chain(goal.context.iter())
            .filter_map(|expr| {
                if let ProofExpr::Identity(l, r) = expr {
                    Some((l.clone(), r.clone()))
                } else {
                    None
                }
            })
            .collect();

        if equalities.is_empty() {
            return Ok(None);
        }

        // Handle special case: goal is itself an equality (symmetry/transitivity)
        if let ProofExpr::Identity(goal_l, goal_r) = &goal.target {
            // Try symmetry: a = b ⊢ b = a
            if let Some(tree) = self.try_equality_symmetry(goal_l, goal_r, &equalities, depth)? {
                return Ok(Some(tree));
            }

            // Try transitivity: a = b, b = c ⊢ a = c
            if let Some(tree) = self.try_equality_transitivity(goal_l, goal_r, &equalities, depth)? {
                return Ok(Some(tree));
            }

            // Try equational rewriting: use axioms to rewrite LHS step by step
            // Only if we have depth budget remaining (prevents infinite recursion)
            if depth + 3 < self.max_depth {
                if let Some(tree) = self.try_equational_identity_rewrite(goal, goal_l, goal_r, depth)? {
                    return Ok(Some(tree));
                }
            }

            // General congruence closure: `F(a)=F(b)` from `a=b`, closed through
            // transitivity and nested applications ("equals added to equals").
            if let Some(tree) = self.try_congruence(goal)? {
                return Ok(Some(tree));
            }

            return Ok(None);
        }

        // Try rewriting: substitute one term for another (for non-Identity goals)
        for (eq_from, eq_to) in &equalities {
            // Try forward: a = b, P(a) ⊢ P(b)
            if let Some(tree) = self.try_rewrite_with_equality(
                goal, eq_from, eq_to, depth,
            )? {
                return Ok(Some(tree));
            }

            // Try backward: a = b, P(b) ⊢ P(a)
            if let Some(tree) = self.try_rewrite_with_equality(
                goal, eq_to, eq_from, depth,
            )? {
                return Ok(Some(tree));
            }
        }

        Ok(None)
    }

    /// Try to prove goal by substituting `from` with `to` in some known fact.
    fn try_rewrite_with_equality(
        &mut self,
        goal: &ProofGoal,
        from: &ProofTerm,
        to: &ProofTerm,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Create the "source" expression by substituting `to` with `from` in the goal
        // If goal is P(b) and we have a = b, we want to find P(a)
        let source_goal = self.substitute_term_in_expr(&goal.target, to, from);

        // Check if source_goal differs from the goal (substitution had effect)
        if source_goal == goal.target {
            return Ok(None);
        }

        // Try to prove the source goal
        let source_proof_goal = ProofGoal::with_context(source_goal.clone(), goal.context.clone());
        if let Ok(source_proof) = self.prove_goal(source_proof_goal, depth + 1) {
            // Also need a proof of the equality
            let equality = ProofExpr::Identity(from.clone(), to.clone());
            let eq_proof_goal = ProofGoal::with_context(equality.clone(), goal.context.clone());

            if let Ok(eq_proof) = self.prove_goal(eq_proof_goal, depth + 1) {
                return Ok(Some(DerivationTree::new(
                    goal.target.clone(),
                    InferenceRule::Rewrite {
                        from: from.clone(),
                        to: to.clone(),
                    },
                    vec![eq_proof, source_proof],
                )));
            }
        }

        Ok(None)
    }

    /// Try equality symmetry: a = b ⊢ b = a
    fn try_equality_symmetry(
        &mut self,
        goal_l: &ProofTerm,
        goal_r: &ProofTerm,
        equalities: &[(ProofTerm, ProofTerm)],
        _depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Check if we have r = l in KB (so we can derive l = r)
        for (eq_l, eq_r) in equalities {
            if eq_l == goal_r && eq_r == goal_l {
                // Found r = l, can derive l = r by symmetry
                let source = ProofExpr::Identity(goal_r.clone(), goal_l.clone());
                return Ok(Some(DerivationTree::new(
                    ProofExpr::Identity(goal_l.clone(), goal_r.clone()),
                    InferenceRule::EqualitySymmetry,
                    vec![DerivationTree::leaf(source, InferenceRule::PremiseMatch)],
                )));
            }
        }
        Ok(None)
    }

    /// Prove an `Identity` goal by congruence closure over the known equalities
    /// (knowledge base + local context). Decision and proof reconstruction live in
    /// [`cert_congruence_path`]; every emitted node certifies, so a spurious
    /// congruence would fail the kernel rather than admit a false equation.
    fn try_congruence(&mut self, goal: &ProofGoal) -> ProofResult<Option<DerivationTree>> {
        let ProofExpr::Identity(lhs, rhs) = &goal.target else {
            return Ok(None);
        };
        let hyps: Vec<(ProofExpr, DerivationTree)> = self
            .knowledge_base
            .iter()
            .chain(goal.context.iter())
            .filter_map(|e| match e {
                ProofExpr::Identity(l, r) => Some((
                    e.clone(),
                    DerivationTree::leaf(
                        ProofExpr::Identity(l.clone(), r.clone()),
                        InferenceRule::PremiseMatch,
                    ),
                )),
                _ => None,
            })
            .collect();
        if hyps.is_empty() {
            return Ok(None);
        }
        Ok(cert_congruence_path(lhs, rhs, &hyps))
    }

    /// Prove an inequality goal `a ≤ b` (encoded `le(a, b) = true`) by chaining the
    /// known `≤` facts transitively. The directed-graph search and proof
    /// reconstruction live in [`cert_le_path`]; every node certifies via the
    /// `le_trans`/`le_refl` axioms, so a spurious chain fails the kernel.
    fn try_linarith(&mut self, goal: &ProofGoal, depth: usize) -> ProofResult<Option<DerivationTree>> {
        let Some((ga, gb)) = as_le_pair(&goal.target) else {
            return Ok(None);
        };
        // Ground case: both operands are literals — decided by computation. `le m n`
        // reduces to `true`/`false`, so `m ≤ n` closes by reflexivity exactly when it
        // holds (and is left unproven, soundly, when it does not).
        if let (Some(av), Some(bv)) = (as_int_literal(&ga), as_int_literal(&gb)) {
            return Ok((av <= bv)
                .then(|| DerivationTree::leaf(goal.target.clone(), InferenceRule::Reflexivity)));
        }
        // Addition: `a + c ≤ b + d` from `a ≤ b` and `c ≤ d`, each proved recursively
        // (a Farkas primitive — `le_add_mono`).
        if let (ProofTerm::Function(fl, la), ProofTerm::Function(fr, ra)) = (&ga, &gb) {
            if fl == "add" && fr == "add" && la.len() == 2 && ra.len() == 2 {
                let g0 =
                    ProofGoal::with_context(le_eq(la[0].clone(), ra[0].clone()), goal.context.clone());
                let g1 =
                    ProofGoal::with_context(le_eq(la[1].clone(), ra[1].clone()), goal.context.clone());
                if let (Ok(p0), Ok(p1)) =
                    (self.prove_goal(g0, depth + 1), self.prove_goal(g1, depth + 1))
                {
                    return Ok(Some(DerivationTree::new(
                        goal.target.clone(),
                        InferenceRule::LeAddMono,
                        vec![p0, p1],
                    )));
                }
            }
        }
        let mut adj: std::collections::HashMap<String, Vec<(ProofTerm, DerivationTree)>> =
            std::collections::HashMap::new();
        for e in goal.context.iter().chain(self.knowledge_base.iter()) {
            if let Some((x, y)) = as_le_pair(e) {
                if let Some(xk) = term_skey(&x) {
                    let proof = DerivationTree::leaf(e.clone(), InferenceRule::PremiseMatch);
                    adj.entry(xk).or_default().push((y, proof));
                }
            }
        }
        Ok(cert_le_path(&ga, &gb, &adj))
    }

    /// Try equality transitivity: a = b, b = c ⊢ a = c
    fn try_equality_transitivity(
        &mut self,
        goal_l: &ProofTerm,
        goal_r: &ProofTerm,
        equalities: &[(ProofTerm, ProofTerm)],
        _depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Look for a = b and b = c where we want a = c
        for (eq1_l, eq1_r) in equalities {
            if eq1_l == goal_l {
                // Found a = b, now look for b = c
                for (eq2_l, eq2_r) in equalities {
                    if eq2_l == eq1_r && eq2_r == goal_r {
                        // Found a = b and b = c, derive a = c
                        let premise1 = ProofExpr::Identity(eq1_l.clone(), eq1_r.clone());
                        let premise2 = ProofExpr::Identity(eq2_l.clone(), eq2_r.clone());
                        return Ok(Some(DerivationTree::new(
                            ProofExpr::Identity(goal_l.clone(), goal_r.clone()),
                            InferenceRule::EqualityTransitivity,
                            vec![
                                DerivationTree::leaf(premise1, InferenceRule::PremiseMatch),
                                DerivationTree::leaf(premise2, InferenceRule::PremiseMatch),
                            ],
                        )));
                    }
                }
            }
        }
        Ok(None)
    }

    /// Try equational rewriting for Identity goals.
    ///
    /// For a goal `f(a) = b`, find an axiom `f(x) = g(x)` that matches,
    /// rewrite to get `g(a) = b`, and recursively prove that.
    fn try_equational_identity_rewrite(
        &mut self,
        goal: &ProofGoal,
        goal_l: &ProofTerm,
        goal_r: &ProofTerm,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // First, try congruence: if both sides have the same outermost function/ctor,
        // recursively prove the arguments are equal.
        if let (
            ProofTerm::Function(name_l, args_l),
            ProofTerm::Function(name_r, args_r),
        ) = (goal_l, goal_r)
        {
            if name_l == name_r && args_l.len() == args_r.len() {
                // Prove each differing argument equal; identical arguments need no
                // rewrite. (`None` marks a position left untouched.)
                let mut arg_proofs: Vec<Option<DerivationTree>> = Vec::new();
                let mut all_ok = true;
                for (arg_l, arg_r) in args_l.iter().zip(args_r.iter()) {
                    if arg_l == arg_r {
                        arg_proofs.push(None);
                        continue;
                    }
                    let arg_goal_expr = ProofExpr::Identity(arg_l.clone(), arg_r.clone());
                    let arg_goal = ProofGoal::with_context(arg_goal_expr, goal.context.clone());
                    match self.prove_goal(arg_goal, depth + 1) {
                        Ok(proof) => arg_proofs.push(Some(proof)),
                        Err(_) => {
                            all_ok = false;
                            break;
                        }
                    }
                }
                if all_ok {
                    // Congruence: `f(a₀…) = f(b₀…)`, each differing argument rewritten
                    // in turn via `Rewrite` (Leibniz) — a real certificate, not a
                    // reflexivity placeholder that ignores the argument proofs.
                    return Ok(Some(build_congruence_proof(
                        name_l, args_l, args_r, &arg_proofs,
                    )));
                }
            }
        }
        // Collect Identity axioms from KB
        let axioms: Vec<(ProofTerm, ProofTerm)> = self
            .knowledge_base
            .iter()
            .filter_map(|e| {
                if let ProofExpr::Identity(l, r) = e {
                    Some((l.clone(), r.clone()))
                } else {
                    None
                }
            })
            .collect();

        // Try each axiom to rewrite the goal's LHS
        for (axiom_l, axiom_r) in &axioms {
            // Rename variables in axiom to avoid capture (use same map for both sides!)
            let mut var_map = std::collections::HashMap::new();
            let renamed_l = self.rename_term_vars_with_map(axiom_l, &mut var_map);
            let renamed_r = self.rename_term_vars_with_map(axiom_r, &mut var_map);

            // Try to unify axiom LHS with goal LHS
            // e.g., unify(Add(Succ(k), n), Add(Succ(Zero), Succ(Zero)))
            //       => {k: Zero, n: Succ(Zero)}
            if let Ok(subst) = unify_terms(&renamed_l, goal_l) {
                // Apply substitution to axiom RHS to get the rewritten term
                let rewritten = self.apply_subst_to_term(&renamed_r, &subst);

                // First check: does rewritten equal goal_r directly?
                if terms_structurally_equal(&rewritten, goal_r) {
                    // Direct match! Build the proof
                    let axiom_expr = ProofExpr::Identity(axiom_l.clone(), axiom_r.clone());
                    return Ok(Some(DerivationTree::new(
                        goal.target.clone(),
                        InferenceRule::Rewrite {
                            from: goal_l.clone(),
                            to: rewritten,
                        },
                        vec![DerivationTree::leaf(axiom_expr, InferenceRule::PremiseMatch)],
                    )));
                }

                // Otherwise, create a new goal with the rewritten LHS
                let new_goal_expr = ProofExpr::Identity(rewritten.clone(), goal_r.clone());
                let new_goal = ProofGoal::with_context(new_goal_expr.clone(), goal.context.clone());

                // Recursively try to prove the new goal
                if let Ok(sub_proof) = self.prove_goal(new_goal, depth + 1) {
                    // Success! Build the full proof
                    let axiom_expr = ProofExpr::Identity(axiom_l.clone(), axiom_r.clone());
                    return Ok(Some(DerivationTree::new(
                        goal.target.clone(),
                        InferenceRule::Rewrite {
                            from: goal_l.clone(),
                            to: rewritten,
                        },
                        vec![
                            DerivationTree::leaf(axiom_expr, InferenceRule::PremiseMatch),
                            sub_proof,
                        ],
                    )));
                }
            }
        }

        Ok(None)
    }

    /// Rename variables in a term to fresh names (consistently).
    fn rename_term_vars(&mut self, term: &ProofTerm) -> ProofTerm {
        let mut var_map = std::collections::HashMap::new();
        self.rename_term_vars_with_map(term, &mut var_map)
    }

    fn rename_term_vars_with_map(
        &mut self,
        term: &ProofTerm,
        var_map: &mut std::collections::HashMap<String, String>,
    ) -> ProofTerm {
        match term {
            ProofTerm::Variable(name) => {
                // Check if we've already renamed this variable
                if let Some(fresh) = var_map.get(name) {
                    ProofTerm::Variable(fresh.clone())
                } else {
                    // Create fresh name and remember it
                    let fresh = format!("_v{}", self.var_counter);
                    self.var_counter += 1;
                    var_map.insert(name.clone(), fresh.clone());
                    ProofTerm::Variable(fresh)
                }
            }
            ProofTerm::Function(name, args) => {
                ProofTerm::Function(
                    name.clone(),
                    args.iter().map(|a| self.rename_term_vars_with_map(a, var_map)).collect(),
                )
            }
            ProofTerm::Group(terms) => {
                ProofTerm::Group(
                    terms.iter().map(|t| self.rename_term_vars_with_map(t, var_map)).collect(),
                )
            }
            other => other.clone(),
        }
    }

    /// Apply a substitution to a term.
    fn apply_subst_to_term(&self, term: &ProofTerm, subst: &Substitution) -> ProofTerm {
        match term {
            ProofTerm::Variable(name) => {
                if let Some(replacement) = subst.get(name) {
                    replacement.clone()
                } else {
                    term.clone()
                }
            }
            ProofTerm::Function(name, args) => {
                ProofTerm::Function(
                    name.clone(),
                    args.iter().map(|a| self.apply_subst_to_term(a, subst)).collect(),
                )
            }
            ProofTerm::Group(terms) => {
                ProofTerm::Group(terms.iter().map(|t| self.apply_subst_to_term(t, subst)).collect())
            }
            other => other.clone(),
        }
    }

    /// Substitute a term for another in an expression.
    /// Substitute only FREE occurrences of a variable: binders that shadow the
    /// name (∀x, ∃x, λx) leave their bodies untouched. Used to bind anaphoric
    /// free variables (discourse-telescoped definite descriptions) without
    /// capturing unrelated bound variables that share the name.
    fn substitute_free_var_in_expr(
        &self,
        expr: &ProofExpr,
        var_name: &str,
        to: &ProofTerm,
    ) -> ProofExpr {
        fn in_term(term: &ProofTerm, var_name: &str, to: &ProofTerm) -> ProofTerm {
            match term {
                ProofTerm::Variable(v) if v == var_name => to.clone(),
                ProofTerm::Function(name, args) => ProofTerm::Function(
                    name.clone(),
                    args.iter().map(|a| in_term(a, var_name, to)).collect(),
                ),
                ProofTerm::Group(terms) => ProofTerm::Group(
                    terms.iter().map(|t| in_term(t, var_name, to)).collect(),
                ),
                other => other.clone(),
            }
        }

        match expr {
            ProofExpr::Predicate { name, args, world } => ProofExpr::Predicate {
                name: name.clone(),
                args: args.iter().map(|a| in_term(a, var_name, to)).collect(),
                world: world.clone(),
            },
            ProofExpr::Identity(l, r) => ProofExpr::Identity(
                in_term(l, var_name, to),
                in_term(r, var_name, to),
            ),
            ProofExpr::NeoEvent { event_var, verb, roles } => ProofExpr::NeoEvent {
                event_var: event_var.clone(),
                verb: verb.clone(),
                roles: roles
                    .iter()
                    .map(|(role, t)| (role.clone(), in_term(t, var_name, to)))
                    .collect(),
            },
            ProofExpr::And(l, r) => ProofExpr::And(
                Box::new(self.substitute_free_var_in_expr(l, var_name, to)),
                Box::new(self.substitute_free_var_in_expr(r, var_name, to)),
            ),
            ProofExpr::Or(l, r) => ProofExpr::Or(
                Box::new(self.substitute_free_var_in_expr(l, var_name, to)),
                Box::new(self.substitute_free_var_in_expr(r, var_name, to)),
            ),
            ProofExpr::Implies(l, r) => ProofExpr::Implies(
                Box::new(self.substitute_free_var_in_expr(l, var_name, to)),
                Box::new(self.substitute_free_var_in_expr(r, var_name, to)),
            ),
            ProofExpr::Iff(l, r) => ProofExpr::Iff(
                Box::new(self.substitute_free_var_in_expr(l, var_name, to)),
                Box::new(self.substitute_free_var_in_expr(r, var_name, to)),
            ),
            ProofExpr::Not(inner) => ProofExpr::Not(Box::new(
                self.substitute_free_var_in_expr(inner, var_name, to),
            )),
            ProofExpr::ForAll { variable, body } if variable != var_name => ProofExpr::ForAll {
                variable: variable.clone(),
                body: Box::new(self.substitute_free_var_in_expr(body, var_name, to)),
            },
            ProofExpr::Exists { variable, body } if variable != var_name => ProofExpr::Exists {
                variable: variable.clone(),
                body: Box::new(self.substitute_free_var_in_expr(body, var_name, to)),
            },
            ProofExpr::Lambda { variable, body } if variable != var_name => ProofExpr::Lambda {
                variable: variable.clone(),
                body: Box::new(self.substitute_free_var_in_expr(body, var_name, to)),
            },
            ProofExpr::Modal { domain, force, flavor, body } => ProofExpr::Modal {
                domain: domain.clone(),
                force: *force,
                flavor: flavor.clone(),
                body: Box::new(self.substitute_free_var_in_expr(body, var_name, to)),
            },
            ProofExpr::Temporal { operator, body } => ProofExpr::Temporal {
                operator: operator.clone(),
                body: Box::new(self.substitute_free_var_in_expr(body, var_name, to)),
            },
            ProofExpr::TemporalBinary { operator, left, right } => ProofExpr::TemporalBinary {
                operator: operator.clone(),
                left: Box::new(self.substitute_free_var_in_expr(left, var_name, to)),
                right: Box::new(self.substitute_free_var_in_expr(right, var_name, to)),
            },
            // Shadowing binders (the guards above failed) and atoms pass through.
            other => other.clone(),
        }
    }

    fn substitute_term_in_expr(
        &self,
        expr: &ProofExpr,
        from: &ProofTerm,
        to: &ProofTerm,
    ) -> ProofExpr {
        match expr {
            ProofExpr::Predicate { name, args, world } => {
                let new_args: Vec<_> = args
                    .iter()
                    .map(|arg| self.substitute_in_term(arg, from, to))
                    .collect();
                ProofExpr::Predicate {
                    name: name.clone(),
                    args: new_args,
                    world: world.clone(),
                }
            }
            ProofExpr::Identity(l, r) => ProofExpr::Identity(
                self.substitute_in_term(l, from, to),
                self.substitute_in_term(r, from, to),
            ),
            ProofExpr::And(l, r) => ProofExpr::And(
                Box::new(self.substitute_term_in_expr(l, from, to)),
                Box::new(self.substitute_term_in_expr(r, from, to)),
            ),
            ProofExpr::Or(l, r) => ProofExpr::Or(
                Box::new(self.substitute_term_in_expr(l, from, to)),
                Box::new(self.substitute_term_in_expr(r, from, to)),
            ),
            ProofExpr::Implies(l, r) => ProofExpr::Implies(
                Box::new(self.substitute_term_in_expr(l, from, to)),
                Box::new(self.substitute_term_in_expr(r, from, to)),
            ),
            ProofExpr::Not(inner) => {
                ProofExpr::Not(Box::new(self.substitute_term_in_expr(inner, from, to)))
            }
            ProofExpr::ForAll { variable, body } => ProofExpr::ForAll {
                variable: variable.clone(),
                body: Box::new(self.substitute_term_in_expr(body, from, to)),
            },
            ProofExpr::Exists { variable, body } => ProofExpr::Exists {
                variable: variable.clone(),
                body: Box::new(self.substitute_term_in_expr(body, from, to)),
            },
            // For other expressions, return as-is
            other => other.clone(),
        }
    }

    /// Substitute a term for another in a ProofTerm.
    fn substitute_in_term(
        &self,
        term: &ProofTerm,
        from: &ProofTerm,
        to: &ProofTerm,
    ) -> ProofTerm {
        if term == from {
            return to.clone();
        }
        match term {
            ProofTerm::Function(name, args) => {
                let new_args: Vec<_> = args
                    .iter()
                    .map(|arg| self.substitute_in_term(arg, from, to))
                    .collect();
                ProofTerm::Function(name.clone(), new_args)
            }
            ProofTerm::Group(terms) => {
                let new_terms: Vec<_> = terms
                    .iter()
                    .map(|t| self.substitute_in_term(t, from, to))
                    .collect();
                ProofTerm::Group(new_terms)
            }
            other => other.clone(),
        }
    }

    // =========================================================================
    // STRATEGY 7: STRUCTURAL INDUCTION
    // =========================================================================

    /// Try structural induction on inductive types (Nat, List, etc.).
    ///
    /// First attempts to infer the motive using Miller pattern unification
    /// (`?Motive(#n) = Goal` → `?Motive = λn.Goal`). Falls back to crude
    /// substitution if pattern unification fails.
    ///
    /// When the goal contains a TypedVar like `n:Nat`, we split into:
    /// - Base case: P(Zero)
    /// - Step case: ∀k. P(k) → P(Succ(k))
    fn try_structural_induction(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Look for TypedVar in the goal
        if let Some((var_name, typename)) = self.find_typed_var(&goal.target) {
            // Try motive inference via pattern unification first
            if let Some(motive) = self.try_infer_motive(&goal.target, &var_name) {
                match typename.as_str() {
                    "Nat" => {
                        if let Ok(Some(proof)) =
                            self.try_nat_induction_with_motive(goal, &var_name, &motive, depth)
                        {
                            return Ok(Some(proof));
                        }
                    }
                    "List" => {
                        // TODO: Add try_list_induction_with_motive
                    }
                    _ => {}
                }
            }

            // Fallback: crude substitution approach
            match typename.as_str() {
                "Nat" => self.try_nat_induction(goal, &var_name, depth),
                "List" => self.try_list_induction(goal, &var_name, depth),
                _ => Ok(None), // Unknown inductive type
            }
        } else {
            Ok(None)
        }
    }

    /// Try to infer the induction motive using Miller pattern unification.
    ///
    /// Given a goal like `Add(n:Nat, Zero) = n:Nat`, creates the pattern
    /// `?Motive(#n) = Goal` and solves for `?Motive = λn. Goal`.
    fn try_infer_motive(&self, goal: &ProofExpr, var_name: &str) -> Option<ProofExpr> {
        // Create the pattern: ?Motive(#var_name)
        let motive_hole = ProofExpr::Hole("Motive".to_string());
        let pattern = ProofExpr::App(
            Box::new(motive_hole),
            Box::new(ProofExpr::Term(ProofTerm::BoundVarRef(var_name.to_string()))),
        );

        // The body is the goal itself (with TypedVar replaced by Variable for unification)
        let body = self.convert_typed_var_to_variable(goal, var_name);

        // Unify: ?Motive(#n) = body
        match unify_pattern(&pattern, &body) {
            Ok(solution) => solution.get("Motive").cloned(),
            Err(_) => None,
        }
    }

    /// Convert TypedVar to regular Variable for pattern unification.
    ///
    /// Pattern unification expects Variable("n") in the body to match BoundVarRef("n")
    /// in the pattern, but our goals have TypedVar { name: "n", typename: "Nat" }.
    fn convert_typed_var_to_variable(&self, expr: &ProofExpr, var_name: &str) -> ProofExpr {
        match expr {
            ProofExpr::TypedVar { name, .. } if name == var_name => {
                // Convert to Atom so it becomes a Variable in terms
                ProofExpr::Atom(name.clone())
            }
            ProofExpr::Identity(l, r) => ProofExpr::Identity(
                self.convert_typed_var_in_term(l, var_name),
                self.convert_typed_var_in_term(r, var_name),
            ),
            ProofExpr::Predicate { name, args, world } => ProofExpr::Predicate {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|a| self.convert_typed_var_in_term(a, var_name))
                    .collect(),
                world: world.clone(),
            },
            ProofExpr::And(l, r) => ProofExpr::And(
                Box::new(self.convert_typed_var_to_variable(l, var_name)),
                Box::new(self.convert_typed_var_to_variable(r, var_name)),
            ),
            ProofExpr::Or(l, r) => ProofExpr::Or(
                Box::new(self.convert_typed_var_to_variable(l, var_name)),
                Box::new(self.convert_typed_var_to_variable(r, var_name)),
            ),
            ProofExpr::Not(inner) => {
                ProofExpr::Not(Box::new(self.convert_typed_var_to_variable(inner, var_name)))
            }
            _ => expr.clone(),
        }
    }

    /// Convert TypedVar to Variable in a ProofTerm.
    fn convert_typed_var_in_term(&self, term: &ProofTerm, var_name: &str) -> ProofTerm {
        match term {
            ProofTerm::Variable(v) => {
                // Check for "name:Type" pattern
                if v == var_name || v.starts_with(&format!("{}:", var_name)) {
                    ProofTerm::Variable(var_name.to_string())
                } else {
                    term.clone()
                }
            }
            ProofTerm::Function(name, args) => ProofTerm::Function(
                name.clone(),
                args.iter()
                    .map(|a| self.convert_typed_var_in_term(a, var_name))
                    .collect(),
            ),
            ProofTerm::Group(terms) => ProofTerm::Group(
                terms
                    .iter()
                    .map(|t| self.convert_typed_var_in_term(t, var_name))
                    .collect(),
            ),
            _ => term.clone(),
        }
    }

    /// Perform structural induction on Nat using pattern unification.
    ///
    /// Uses Miller pattern unification to infer the motive, then applies
    /// it to constructors via beta reduction.
    ///
    /// Base case: P(Zero)
    /// Step case: ∀k. P(k) → P(Succ(k))
    fn try_nat_induction_with_motive(
        &mut self,
        goal: &ProofGoal,
        var_name: &str,
        motive: &ProofExpr,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Base case: P(Zero)
        // Apply the motive lambda to Zero constructor
        let zero_ctor = ProofExpr::Ctor {
            name: "Zero".into(),
            args: vec![],
        };
        let base_goal_expr = beta_reduce(&ProofExpr::App(
            Box::new(motive.clone()),
            Box::new(zero_ctor),
        ));

        let base_goal = ProofGoal::with_context(base_goal_expr, goal.context.clone());
        let base_proof = match self.prove_goal(base_goal, depth + 1) {
            Ok(proof) => proof,
            Err(_) => return Ok(None),
        };

        // Step case: ∀k. P(k) → P(Succ(k))
        let fresh_k = self.fresh_var();
        let k_var = ProofExpr::Atom(fresh_k.clone());

        // Induction hypothesis: P(k)
        let ih = beta_reduce(&ProofExpr::App(
            Box::new(motive.clone()),
            Box::new(k_var.clone()),
        ));

        // Step conclusion: P(Succ(k))
        let succ_k = ProofExpr::Ctor {
            name: "Succ".into(),
            args: vec![k_var],
        };
        let step_goal_expr = beta_reduce(&ProofExpr::App(
            Box::new(motive.clone()),
            Box::new(succ_k),
        ));

        // Add IH to context for step case
        let mut step_context = goal.context.clone();
        step_context.push(ih.clone());

        let step_goal = ProofGoal::with_context(step_goal_expr, step_context);
        let step_proof = match self.try_step_case_with_equational_reasoning(&step_goal, &ih, depth)
        {
            Ok(proof) => proof,
            Err(_) => return Ok(None),
        };

        Ok(Some(DerivationTree::new(
            goal.target.clone(),
            InferenceRule::StructuralInduction {
                variable: var_name.to_string(),
                ind_type: "Nat".to_string(),
                step_var: fresh_k,
            },
            vec![base_proof, step_proof],
        )))
    }

    /// Perform structural induction on Nat (legacy crude substitution).
    ///
    /// Base case: P(Zero)
    /// Step case: ∀k. P(k) → P(Succ(k))
    fn try_nat_induction(
        &mut self,
        goal: &ProofGoal,
        var_name: &str,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Create Zero constructor
        let zero = ProofExpr::Ctor {
            name: "Zero".into(),
            args: vec![],
        };

        // Base case: substitute Zero for the induction variable
        let base_goal_expr = self.substitute_typed_var(&goal.target, var_name, &zero);
        let base_goal = ProofGoal::with_context(base_goal_expr, goal.context.clone());

        // Try to prove base case
        let base_proof = match self.prove_goal(base_goal, depth + 1) {
            Ok(proof) => proof,
            Err(_) => return Ok(None), // Can't prove base case
        };

        // Step case: assume P(k), prove P(Succ(k))
        let fresh_k = self.fresh_var();

        // Create k as a variable
        let k_var = ProofExpr::Atom(fresh_k.clone());

        // Create Succ(k)
        let succ_k = ProofExpr::Ctor {
            name: "Succ".into(),
            args: vec![k_var.clone()],
        };

        // Induction hypothesis: P(k)
        let ih = self.substitute_typed_var(&goal.target, var_name, &k_var);

        // Step goal: P(Succ(k))
        let step_goal_expr = self.substitute_typed_var(&goal.target, var_name, &succ_k);

        // Add IH to context for step case
        let mut step_context = goal.context.clone();
        step_context.push(ih.clone());

        let step_goal = ProofGoal::with_context(step_goal_expr, step_context);

        // Try to prove step case with IH in context
        let step_proof = match self.try_step_case_with_equational_reasoning(&step_goal, &ih, depth)
        {
            Ok(proof) => proof,
            Err(_) => return Ok(None), // Can't prove step case
        };

        // Build the induction proof tree
        Ok(Some(DerivationTree::new(
            goal.target.clone(),
            InferenceRule::StructuralInduction {
                variable: var_name.to_string(),
                ind_type: "Nat".to_string(),
                step_var: fresh_k,
            },
            vec![base_proof, step_proof],
        )))
    }

    /// Perform structural induction on List.
    ///
    /// Base case: P(Nil)
    /// Step case: ∀h,t. P(t) → P(Cons(h,t))
    fn try_list_induction(
        &mut self,
        goal: &ProofGoal,
        var_name: &str,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Create Nil constructor
        let nil = ProofExpr::Ctor {
            name: "Nil".into(),
            args: vec![],
        };

        // Base case: substitute Nil for the induction variable
        let base_goal_expr = self.substitute_typed_var(&goal.target, var_name, &nil);
        let base_goal = ProofGoal::with_context(base_goal_expr, goal.context.clone());

        // Try to prove base case
        let base_proof = match self.prove_goal(base_goal, depth + 1) {
            Ok(proof) => proof,
            Err(_) => return Ok(None),
        };

        // Step case: assume P(t), prove P(Cons(h, t))
        let fresh_h = self.fresh_var();
        let fresh_t = self.fresh_var();

        let h_var = ProofExpr::Atom(fresh_h);
        let t_var = ProofExpr::Atom(fresh_t.clone());

        let cons_ht = ProofExpr::Ctor {
            name: "Cons".into(),
            args: vec![h_var, t_var.clone()],
        };

        // Induction hypothesis: P(t)
        let ih = self.substitute_typed_var(&goal.target, var_name, &t_var);

        // Step goal: P(Cons(h, t))
        let step_goal_expr = self.substitute_typed_var(&goal.target, var_name, &cons_ht);

        let mut step_context = goal.context.clone();
        step_context.push(ih.clone());

        let step_goal = ProofGoal::with_context(step_goal_expr, step_context);

        let step_proof = match self.try_step_case_with_equational_reasoning(&step_goal, &ih, depth)
        {
            Ok(proof) => proof,
            Err(_) => return Ok(None),
        };

        Ok(Some(DerivationTree::new(
            goal.target.clone(),
            InferenceRule::StructuralInduction {
                variable: var_name.to_string(),
                ind_type: "List".to_string(),
                step_var: fresh_t,
            },
            vec![base_proof, step_proof],
        )))
    }

    /// Try to prove the step case, potentially using equational reasoning.
    ///
    /// The step case often requires:
    /// 1. Applying a recursive axiom to simplify the goal
    /// 2. Using the induction hypothesis
    /// 3. Congruence reasoning (e.g., Succ(x) = Succ(y) if x = y)
    fn try_step_case_with_equational_reasoning(
        &mut self,
        goal: &ProofGoal,
        ih: &ProofExpr,
        depth: usize,
    ) -> ProofResult<DerivationTree> {
        // First, try direct proof (might work for simple cases)
        if let Ok(proof) = self.prove_goal(goal.clone(), depth + 1) {
            return Ok(proof);
        }

        // For Identity goals, try equational reasoning
        if let ProofExpr::Identity(lhs, rhs) = &goal.target {
            // Try to rewrite LHS using axioms and see if we can reach RHS
            if let Some(proof) = self.try_equational_proof(goal, lhs, rhs, ih, depth)? {
                return Ok(proof);
            }
        }

        Err(ProofError::NoProofFound)
    }

    /// Try equational reasoning: rewrite LHS to match RHS using axioms and IH.
    ///
    /// For the step case of induction, we need to:
    /// 1. Find an axiom that matches the goal's LHS pattern
    /// 2. Use the axiom to rewrite LHS
    /// 3. Apply the induction hypothesis to simplify
    /// 4. Check if the result equals RHS
    fn try_equational_proof(
        &mut self,
        goal: &ProofGoal,
        lhs: &ProofTerm,
        rhs: &ProofTerm,
        ih: &ProofExpr,
        _depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Find applicable equations from KB (Identity axioms)
        let equations: Vec<ProofExpr> = self
            .knowledge_base
            .iter()
            .filter(|e| matches!(e, ProofExpr::Identity(_, _)))
            .cloned()
            .collect();

        // Try each equation to rewrite LHS
        for eq_axiom in &equations {
            if let ProofExpr::Identity(_, _) = &eq_axiom {
                // Rename variables in the axiom to avoid capture
                let renamed_axiom = self.rename_variables(&eq_axiom);
                if let ProofExpr::Identity(renamed_lhs, renamed_rhs) = renamed_axiom {
                    // Unify axiom LHS with goal LHS
                    // This binds axiom variables to goal terms
                    // e.g., unify(Add(Succ(x), m), Add(Succ(k), Zero)) gives {x->k, m->Zero}
                    if let Ok(subst) = unify_terms(&renamed_lhs, lhs) {
                        // Apply the substitution to the axiom's RHS
                        // This gives us what LHS rewrites to
                        let rewritten = self.apply_subst_to_term_with(&renamed_rhs, &subst);

                        // Now check if rewritten equals RHS (possibly using IH)
                        if self.terms_equal_with_ih(&rewritten, rhs, ih) {
                            // Success! Build proof using the axiom and IH
                            let axiom_leaf =
                                DerivationTree::leaf(eq_axiom.clone(), InferenceRule::PremiseMatch);

                            let ih_leaf =
                                DerivationTree::leaf(ih.clone(), InferenceRule::PremiseMatch);

                            return Ok(Some(DerivationTree::new(
                                goal.target.clone(),
                                InferenceRule::PremiseMatch, // Equational step
                                vec![axiom_leaf, ih_leaf],
                            )));
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Check if two terms are equal, potentially using the induction hypothesis.
    fn terms_equal_with_ih(&self, t1: &ProofTerm, t2: &ProofTerm, ih: &ProofExpr) -> bool {
        // Direct equality
        if t1 == t2 {
            return true;
        }

        // Try using IH: if IH is `x = y`, and t1 contains x, replace with y
        if let ProofExpr::Identity(ih_lhs, ih_rhs) = ih {
            // Check if t1 can be transformed to t2 using IH
            let t1_with_ih = self.rewrite_term_with_equation(t1, ih_lhs, ih_rhs);
            if &t1_with_ih == t2 {
                return true;
            }

            // Also try the other direction
            let t2_with_ih = self.rewrite_term_with_equation(t2, ih_rhs, ih_lhs);
            if t1 == &t2_with_ih {
                return true;
            }
        }

        false
    }

    /// Rewrite occurrences of `from` to `to` in the term.
    fn rewrite_term_with_equation(
        &self,
        term: &ProofTerm,
        from: &ProofTerm,
        to: &ProofTerm,
    ) -> ProofTerm {
        // If term matches `from`, return `to`
        if term == from {
            return to.clone();
        }

        // Recursively rewrite in subterms
        match term {
            ProofTerm::Function(name, args) => {
                let new_args: Vec<ProofTerm> = args
                    .iter()
                    .map(|a| self.rewrite_term_with_equation(a, from, to))
                    .collect();
                ProofTerm::Function(name.clone(), new_args)
            }
            ProofTerm::Group(terms) => {
                let new_terms: Vec<ProofTerm> = terms
                    .iter()
                    .map(|t| self.rewrite_term_with_equation(t, from, to))
                    .collect();
                ProofTerm::Group(new_terms)
            }
            _ => term.clone(),
        }
    }

    /// Apply substitution to a ProofTerm with given substitution.
    fn apply_subst_to_term_with(&self, term: &ProofTerm, subst: &Substitution) -> ProofTerm {
        match term {
            ProofTerm::Variable(v) => subst.get(v).cloned().unwrap_or_else(|| term.clone()),
            ProofTerm::Function(name, args) => ProofTerm::Function(
                name.clone(),
                args.iter()
                    .map(|a| self.apply_subst_to_term_with(a, subst))
                    .collect(),
            ),
            ProofTerm::Group(terms) => ProofTerm::Group(
                terms
                    .iter()
                    .map(|t| self.apply_subst_to_term_with(t, subst))
                    .collect(),
            ),
            ProofTerm::Constant(_) => term.clone(),
            ProofTerm::BoundVarRef(_) => term.clone(),
        }
    }

    /// Find a TypedVar in the expression.
    fn find_typed_var(&self, expr: &ProofExpr) -> Option<(String, String)> {
        match expr {
            ProofExpr::TypedVar { name, typename } => Some((name.clone(), typename.clone())),
            ProofExpr::Identity(l, r) => {
                self.find_typed_var_in_term(l).or_else(|| self.find_typed_var_in_term(r))
            }
            ProofExpr::Predicate { args, .. } => {
                for arg in args {
                    if let Some(tv) = self.find_typed_var_in_term(arg) {
                        return Some(tv);
                    }
                }
                None
            }
            ProofExpr::And(l, r)
            | ProofExpr::Or(l, r)
            | ProofExpr::Implies(l, r)
            | ProofExpr::Iff(l, r) => self.find_typed_var(l).or_else(|| self.find_typed_var(r)),
            ProofExpr::Not(inner) => self.find_typed_var(inner),
            ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
                self.find_typed_var(body)
            }
            _ => None,
        }
    }

    /// Find a TypedVar embedded in a ProofTerm.
    fn find_typed_var_in_term(&self, term: &ProofTerm) -> Option<(String, String)> {
        match term {
            ProofTerm::Variable(v) => {
                // Check if this variable name is in our KB as a TypedVar
                // Actually, TypedVar should be in the expression, not the term
                // Let's check if the variable name contains type annotation
                if v.contains(':') {
                    let parts: Vec<&str> = v.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        return Some((parts[0].to_string(), parts[1].to_string()));
                    }
                }
                None
            }
            ProofTerm::Function(_, args) => {
                for arg in args {
                    if let Some(tv) = self.find_typed_var_in_term(arg) {
                        return Some(tv);
                    }
                }
                None
            }
            ProofTerm::Group(terms) => {
                for t in terms {
                    if let Some(tv) = self.find_typed_var_in_term(t) {
                        return Some(tv);
                    }
                }
                None
            }
            ProofTerm::Constant(_) => None,
            ProofTerm::BoundVarRef(_) => None, // Pattern-level, no TypedVar
        }
    }

    /// Substitute a TypedVar with a given expression throughout the goal.
    fn substitute_typed_var(
        &self,
        expr: &ProofExpr,
        var_name: &str,
        replacement: &ProofExpr,
    ) -> ProofExpr {
        match expr {
            ProofExpr::TypedVar { name, .. } if name == var_name => replacement.clone(),
            ProofExpr::Identity(l, r) => {
                let new_l = self.substitute_typed_var_in_term(l, var_name, replacement);
                let new_r = self.substitute_typed_var_in_term(r, var_name, replacement);
                ProofExpr::Identity(new_l, new_r)
            }
            ProofExpr::Predicate { name, args, world } => {
                let new_args: Vec<ProofTerm> = args
                    .iter()
                    .map(|a| self.substitute_typed_var_in_term(a, var_name, replacement))
                    .collect();
                ProofExpr::Predicate {
                    name: name.clone(),
                    args: new_args,
                    world: world.clone(),
                }
            }
            ProofExpr::And(l, r) => ProofExpr::And(
                Box::new(self.substitute_typed_var(l, var_name, replacement)),
                Box::new(self.substitute_typed_var(r, var_name, replacement)),
            ),
            ProofExpr::Or(l, r) => ProofExpr::Or(
                Box::new(self.substitute_typed_var(l, var_name, replacement)),
                Box::new(self.substitute_typed_var(r, var_name, replacement)),
            ),
            ProofExpr::Implies(l, r) => ProofExpr::Implies(
                Box::new(self.substitute_typed_var(l, var_name, replacement)),
                Box::new(self.substitute_typed_var(r, var_name, replacement)),
            ),
            ProofExpr::Iff(l, r) => ProofExpr::Iff(
                Box::new(self.substitute_typed_var(l, var_name, replacement)),
                Box::new(self.substitute_typed_var(r, var_name, replacement)),
            ),
            ProofExpr::Not(inner) => {
                ProofExpr::Not(Box::new(self.substitute_typed_var(inner, var_name, replacement)))
            }
            ProofExpr::ForAll { variable, body } => ProofExpr::ForAll {
                variable: variable.clone(),
                body: Box::new(self.substitute_typed_var(body, var_name, replacement)),
            },
            ProofExpr::Exists { variable, body } => ProofExpr::Exists {
                variable: variable.clone(),
                body: Box::new(self.substitute_typed_var(body, var_name, replacement)),
            },
            _ => expr.clone(),
        }
    }

    /// Substitute a TypedVar in a ProofTerm.
    fn substitute_typed_var_in_term(
        &self,
        term: &ProofTerm,
        var_name: &str,
        replacement: &ProofExpr,
    ) -> ProofTerm {
        match term {
            ProofTerm::Variable(v) => {
                // Check for TypedVar pattern "name:Type"
                if v == var_name || v.starts_with(&format!("{}:", var_name)) {
                    self.expr_to_term(replacement)
                } else {
                    term.clone()
                }
            }
            ProofTerm::Function(name, args) => ProofTerm::Function(
                name.clone(),
                args.iter()
                    .map(|a| self.substitute_typed_var_in_term(a, var_name, replacement))
                    .collect(),
            ),
            ProofTerm::Group(terms) => ProofTerm::Group(
                terms
                    .iter()
                    .map(|t| self.substitute_typed_var_in_term(t, var_name, replacement))
                    .collect(),
            ),
            ProofTerm::Constant(_) => term.clone(),
            ProofTerm::BoundVarRef(_) => term.clone(),
        }
    }

    /// Convert a ProofExpr to a ProofTerm (for use in substitution).
    fn expr_to_term(&self, expr: &ProofExpr) -> ProofTerm {
        match expr {
            ProofExpr::Atom(s) => ProofTerm::Variable(s.clone()),
            ProofExpr::Ctor { name, args } => {
                ProofTerm::Function(name.clone(), args.iter().map(|a| self.expr_to_term(a)).collect())
            }
            ProofExpr::TypedVar { name, .. } => ProofTerm::Variable(name.clone()),
            _ => ProofTerm::Constant(format!("{}", expr)),
        }
    }

    // =========================================================================
    // HELPER METHODS
    // =========================================================================

    /// Generate a fresh variable name.
    fn fresh_var(&mut self) -> String {
        self.var_counter += 1;
        format!("_G{}", self.var_counter)
    }

    /// A fresh eigenconstant name for universal introduction: an opaque `Constant`
    /// the search cannot unify against (unlike a `Variable`, which it could bind),
    /// so the bound variable of a `∀` is proved as a genuinely arbitrary element.
    /// The distinctive prefix can never be a numeral or collide with a real symbol.
    fn fresh_eigenconstant(&mut self) -> String {
        self.var_counter += 1;
        format!("__eigen{}", self.var_counter)
    }

    /// Rename all variables in an expression to fresh names.
    fn rename_variables(&mut self, expr: &ProofExpr) -> ProofExpr {
        let vars = self.collect_variables(expr);
        let mut subst = Substitution::new();

        for var in vars {
            let fresh = self.fresh_var();
            subst.insert(var, ProofTerm::Variable(fresh));
        }

        apply_subst_to_expr(expr, &subst)
    }

    /// Collect all variable names in an expression.
    fn collect_variables(&self, expr: &ProofExpr) -> Vec<String> {
        let mut vars = Vec::new();
        self.collect_variables_recursive(expr, &mut vars);
        vars
    }

    fn collect_variables_recursive(&self, expr: &ProofExpr, vars: &mut Vec<String>) {
        match expr {
            ProofExpr::Predicate { args, .. } => {
                for arg in args {
                    self.collect_term_variables(arg, vars);
                }
            }
            ProofExpr::Identity(l, r) => {
                self.collect_term_variables(l, vars);
                self.collect_term_variables(r, vars);
            }
            ProofExpr::And(l, r)
            | ProofExpr::Or(l, r)
            | ProofExpr::Implies(l, r)
            | ProofExpr::Iff(l, r) => {
                self.collect_variables_recursive(l, vars);
                self.collect_variables_recursive(r, vars);
            }
            ProofExpr::Not(inner) => self.collect_variables_recursive(inner, vars),
            ProofExpr::ForAll { variable, body } | ProofExpr::Exists { variable, body } => {
                if !vars.contains(variable) {
                    vars.push(variable.clone());
                }
                self.collect_variables_recursive(body, vars);
            }
            ProofExpr::Lambda { variable, body } => {
                if !vars.contains(variable) {
                    vars.push(variable.clone());
                }
                self.collect_variables_recursive(body, vars);
            }
            ProofExpr::App(f, a) => {
                self.collect_variables_recursive(f, vars);
                self.collect_variables_recursive(a, vars);
            }
            ProofExpr::NeoEvent { roles, .. } => {
                for (_, term) in roles {
                    self.collect_term_variables(term, vars);
                }
            }
            _ => {}
        }
    }

    fn collect_term_variables(&self, term: &ProofTerm, vars: &mut Vec<String>) {
        match term {
            ProofTerm::Variable(v) => {
                if !vars.contains(v) {
                    vars.push(v.clone());
                }
            }
            ProofTerm::Function(_, args) => {
                for arg in args {
                    self.collect_term_variables(arg, vars);
                }
            }
            ProofTerm::Group(terms) => {
                for t in terms {
                    self.collect_term_variables(t, vars);
                }
            }
            ProofTerm::Constant(_) => {}
            ProofTerm::BoundVarRef(_) => {} // Pattern-level, no variables
        }
    }

    /// Collect potential witnesses (constants) from the knowledge base.
    fn collect_witnesses(&self) -> Vec<ProofTerm> {
        let mut witnesses = Vec::new();

        for expr in &self.knowledge_base {
            self.collect_constants_from_expr(expr, &mut witnesses);
        }

        witnesses
    }

    fn collect_constants_from_expr(&self, expr: &ProofExpr, constants: &mut Vec<ProofTerm>) {
        match expr {
            ProofExpr::Predicate { args, .. } => {
                for arg in args {
                    self.collect_constants_from_term(arg, constants);
                }
            }
            ProofExpr::Identity(l, r) => {
                self.collect_constants_from_term(l, constants);
                self.collect_constants_from_term(r, constants);
            }
            ProofExpr::And(l, r)
            | ProofExpr::Or(l, r)
            | ProofExpr::Implies(l, r)
            | ProofExpr::Iff(l, r) => {
                self.collect_constants_from_expr(l, constants);
                self.collect_constants_from_expr(r, constants);
            }
            ProofExpr::Not(inner) => self.collect_constants_from_expr(inner, constants),
            ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
                self.collect_constants_from_expr(body, constants);
            }
            ProofExpr::NeoEvent { roles, .. } => {
                for (_, term) in roles {
                    self.collect_constants_from_term(term, constants);
                }
            }
            _ => {}
        }
    }

    fn collect_constants_from_term(&self, term: &ProofTerm, constants: &mut Vec<ProofTerm>) {
        match term {
            ProofTerm::Constant(_) => {
                if !constants.contains(term) {
                    constants.push(term.clone());
                }
            }
            ProofTerm::Function(_, args) => {
                // The function application itself could be a witness
                if !constants.contains(term) {
                    constants.push(term.clone());
                }
                for arg in args {
                    self.collect_constants_from_term(arg, constants);
                }
            }
            ProofTerm::Group(terms) => {
                for t in terms {
                    self.collect_constants_from_term(t, constants);
                }
            }
            ProofTerm::Variable(_) => {}
            ProofTerm::BoundVarRef(_) => {} // Pattern-level, not a constant
        }
    }

    // =========================================================================
    // STRATEGY 7: ORACLE FALLBACK (Z3)
    // =========================================================================

    /// Attempt to prove using Z3 as an oracle.
    ///
    /// This is the fallback when all structural proof strategies fail.
    /// Z3 will verify arithmetic, comparisons, and uninterpreted function reasoning.
    #[cfg(feature = "verification")]
    fn try_oracle_fallback(&self, goal: &ProofGoal) -> ProofResult<Option<DerivationTree>> {
        crate::oracle::try_oracle(goal, &self.knowledge_base)
    }
}

// =============================================================================
// HELPER FUNCTIONS
// =============================================================================

/// Extract the type from an existential body if it contains type information.
///
/// Looks for TypedVar patterns in the body that might indicate the type
/// of the existentially quantified variable. Returns None if no type
/// information is found.
fn extract_type_from_exists_body(body: &ProofExpr) -> Option<String> {
    match body {
        // Direct TypedVar in body
        ProofExpr::TypedVar { typename, .. } => Some(typename.clone()),

        // Recurse into conjunctions
        ProofExpr::And(l, r) => {
            extract_type_from_exists_body(l).or_else(|| extract_type_from_exists_body(r))
        }

        // Recurse into disjunctions
        ProofExpr::Or(l, r) => {
            extract_type_from_exists_body(l).or_else(|| extract_type_from_exists_body(r))
        }

        // Recurse into nested quantifiers
        ProofExpr::Exists { body, .. } | ProofExpr::ForAll { body, .. } => {
            extract_type_from_exists_body(body)
        }

        // No type information found
        _ => None,
    }
}

impl Default for BackwardChainer {
    fn default() -> Self {
        Self::new()
    }
}
