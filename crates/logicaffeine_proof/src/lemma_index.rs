//! `exact?` / `apply?` and premise selection: search over a NAMED, certified
//! lemma library.
//!
//! Each lemma is peeled to `∀xs. A₁ → … → Aₖ → C`; its conclusion `C` is indexed
//! in a discrimination tree ([`crate::discrimination`]). A query matches the
//! goal against candidate conclusions with the one-sided matcher
//! ([`crate::unify::match_expr_pattern`]) — so a quantified lemma instantiates
//! at the (possibly ground) goal for free. `find_exact` reports lemmas whose
//! conclusion IS the goal; `find_apply` reports lemmas whose conclusion matches
//! and lists the antecedents you would still owe. `select_premises` ranks the
//! whole library for relevance, the premise filter that keeps `auto` tractable
//! on a large axiom base.
//!
//! This is the direct answer to Lean's `exact?`/`apply?` and to premise
//! selection — search over a citable, kernel-checkable library rather than an
//! opaque model.

use crate::discrimination::DiscTree;
use crate::unify::{apply_subst_to_expr, match_expr_pattern, Substitution};
use crate::{ProofExpr, ProofTerm};

/// A search result: which lemma, the suggested tactic text, the instantiation,
/// and (for `apply?`) the antecedents still to prove.
#[derive(Debug, Clone)]
pub struct Suggestion {
    pub lemma: String,
    pub tactic_text: String,
    pub subst: Substitution,
    pub remaining: Vec<ProofExpr>,
}

/// A lemma peeled into binders, antecedents, and conclusion.
struct CompiledLemma {
    name: String,
    conclusion: ProofExpr,
    premises: Vec<ProofExpr>,
}

/// An index over a named lemma library.
pub struct LemmaIndex {
    lemmas: Vec<CompiledLemma>,
    concl_index: DiscTree<usize>,
}

impl LemmaIndex {
    /// Build the index from named lemmas (axioms and proved theorems alike).
    pub fn build(named: &[(String, ProofExpr)]) -> Self {
        let mut lemmas = Vec::new();
        let mut concl_index = DiscTree::new();
        for (name, formula) in named {
            let (premises, conclusion) = peel(formula);
            let idx = lemmas.len();
            concl_index.insert_expr(&conclusion, idx);
            lemmas.push(CompiledLemma { name: name.clone(), conclusion, premises });
        }
        LemmaIndex { lemmas, concl_index }
    }

    /// Lemmas whose conclusion IS the goal (an instance of it). Ranked most-
    /// specific first (fewest variable bindings). `exact?`.
    pub fn find_exact(&self, goal: &ProofExpr) -> Vec<Suggestion> {
        let mut out = self.matches(goal, "exact");
        out.sort_by_key(|s| s.subst.len());
        out
    }

    /// Lemmas whose conclusion matches the goal AND have antecedents — applying
    /// them reduces the goal to those (instantiated) antecedents. `apply?`.
    pub fn find_apply(&self, goal: &ProofExpr) -> Vec<Suggestion> {
        let mut out: Vec<Suggestion> =
            self.matches(goal, "apply").into_iter().filter(|s| !s.remaining.is_empty()).collect();
        out.sort_by_key(|s| (s.remaining.len(), s.subst.len()));
        out
    }

    /// Rank the whole library by relevance to `goal` and return the top `k`
    /// names — the premise filter for `auto` on a large base. Relevance is: an
    /// exact/apply conclusion match first, then head-symbol overlap; lemmas
    /// sharing no symbol with the goal are dropped entirely.
    pub fn select_premises(&self, goal: &ProofExpr, k: usize) -> Vec<String> {
        let goal_syms = expr_symbols(goal);
        let matching: std::collections::HashSet<usize> =
            self.concl_index.candidates_expr(goal).into_iter().copied().collect();

        let mut scored: Vec<(i64, usize)> = self
            .lemmas
            .iter()
            .enumerate()
            .filter_map(|(i, lem)| {
                let overlap = expr_symbols(&lem.conclusion)
                    .intersection(&goal_syms)
                    .count() as i64;
                if overlap == 0 && !matching.contains(&i) {
                    return None; // no shared structure — irrelevant
                }
                let mut score = overlap;
                if matching.contains(&i) {
                    score += 100; // a conclusion match dominates
                }
                Some((score, i))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.truncate(k);
        scored.into_iter().map(|(_, i)| self.lemmas[i].name.clone()).collect()
    }

    fn matches(&self, goal: &ProofExpr, verb: &str) -> Vec<Suggestion> {
        let mut out = Vec::new();
        let mut cand: Vec<usize> =
            self.concl_index.candidates_expr(goal).into_iter().copied().collect();
        cand.sort_unstable();
        cand.dedup();
        for i in cand {
            let lem = &self.lemmas[i];
            if let Some(subst) = match_expr_pattern(&lem.conclusion, goal) {
                let remaining =
                    lem.premises.iter().map(|p| apply_subst_to_expr(p, &subst)).collect();
                out.push(Suggestion {
                    lemma: lem.name.clone(),
                    tactic_text: format!("{verb} {}", lem.name),
                    subst,
                    remaining,
                });
            }
        }
        out
    }
}

/// Peel `∀xs. A₁ → … → Aₖ → C` into `([A₁, …, Aₖ], C)`.
fn peel(formula: &ProofExpr) -> (Vec<ProofExpr>, ProofExpr) {
    let mut body = formula;
    while let ProofExpr::ForAll { body: inner, .. } = body {
        body = inner;
    }
    let mut premises = Vec::new();
    while let ProofExpr::Implies(a, rest) = body {
        premises.push((**a).clone());
        body = rest;
    }
    (premises, body.clone())
}

/// The set of predicate/function/atom head symbols mentioned in an expression —
/// the coarse relevance signature for premise selection.
fn expr_symbols(e: &ProofExpr) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    collect_expr_symbols(e, &mut out);
    out
}

fn collect_expr_symbols(e: &ProofExpr, out: &mut std::collections::HashSet<String>) {
    match e {
        ProofExpr::Predicate { name, args, .. } => {
            out.insert(format!("p:{name}"));
            for a in args {
                collect_term_symbols(a, out);
            }
        }
        ProofExpr::Atom(s) => {
            out.insert(format!("a:{s}"));
        }
        ProofExpr::Identity(l, r) => {
            out.insert("=".to_string());
            collect_term_symbols(l, out);
            collect_term_symbols(r, out);
        }
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            collect_expr_symbols(l, out);
            collect_expr_symbols(r, out);
        }
        ProofExpr::Not(p) => collect_expr_symbols(p, out),
        ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
            collect_expr_symbols(body, out)
        }
        _ => {}
    }
}

fn collect_term_symbols(t: &ProofTerm, out: &mut std::collections::HashSet<String>) {
    match t {
        ProofTerm::Constant(s) => {
            out.insert(format!("c:{s}"));
        }
        ProofTerm::Function(name, args) => {
            out.insert(format!("f:{name}"));
            for a in args {
                collect_term_symbols(a, out);
            }
        }
        // Variables carry no relevance signal.
        _ => {}
    }
}
