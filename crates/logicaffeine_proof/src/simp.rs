//! `simp` rule sets: oriented rewrite rules compiled from proved lemmas,
//! indexed for fast lookup, instantiated by matching.
//!
//! A simp rule is a lemma of shape `∀x₁…xₙ. C₁ → … → Cₖ → lhs = rhs` (or
//! `… lhs ↔ rhs`), oriented left-to-right. Orientation is enforced at
//! registration: `vars(rhs) ⊆ vars(lhs)`, `vars(Cᵢ) ⊆ vars(lhs)`, and `lhs`
//! is not a bare variable — so matching the `lhs` against a goal subterm
//! determines the entire instantiation.
//!
//! The tactic layer ([`crate::tactic::ProofState::simp`]) drives the rewrite
//! loop; this module finds one certified step at a time. Each step's equality
//! carries its own derivation — `PremiseMatch` on the ∀-lemma, a
//! [`InferenceRule::UniversalInstTerm`] chain for the instantiation,
//! `ModusPonens` per discharged condition — so the kernel re-checks every
//! rewrite like any hand-written proof. Nothing here is trusted.

use crate::discrimination::DiscTree;
use crate::unify::{apply_subst_to_expr, match_expr_pattern, match_term_pattern, Substitution};
use crate::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

/// The rewriting core of a rule: a term equation or a propositional
/// equivalence.
enum RuleCore {
    TermEq { lhs: ProofTerm, rhs: ProofTerm },
    PropIff { lhs: ProofExpr, rhs: ProofExpr },
}

/// A compiled simp rule: the peeled binder list, pending conditions, oriented
/// core, and the derivation of the un-instantiated lemma (a `PremiseMatch`
/// leaf — valid whenever the lemma is a premise or an in-scope hypothesis).
struct CompiledRule {
    #[allow(dead_code)]
    name: String,
    params: Vec<String>,
    conds: Vec<ProofExpr>,
    core: RuleCore,
    lemma: ProofExpr,
    base: DerivationTree,
}

/// An indexed set of simp rules.
#[derive(Default)]
pub struct SimpSet {
    rules: Vec<CompiledRule>,
    term_index: DiscTree<usize>,
    iff_index: DiscTree<usize>,
}

/// One rewrite step found in a goal: `eq` proves `Identity(from, to)`.
pub(crate) struct RewriteStep {
    pub eq: DerivationTree,
    pub from: ProofTerm,
    pub to: ProofTerm,
}

/// One top-level propositional step: `imp` proves `rhs → goal`, so refining
/// with `ModusPonens` reduces the goal to `rhs`.
pub(crate) struct IffStep {
    pub imp: DerivationTree,
    pub rhs: ProofExpr,
}

impl SimpSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Compile and index `lemma` as a rewrite rule. Returns `false` (and
    /// registers nothing) if the lemma is not an oriented rule: no equational
    /// or iff core, a bare-variable lhs, rhs/condition variables the lhs does
    /// not bind, or shadowed binder names.
    ///
    /// The rule's derivation base is a `PremiseMatch` leaf, so the lemma must
    /// be in scope (a premise or hypothesis) wherever the rule fires.
    pub fn register_lemma(&mut self, name: &str, lemma: &ProofExpr) -> bool {
        // Peel the ∀-prefix.
        let mut params: Vec<String> = Vec::new();
        let mut body = lemma;
        while let ProofExpr::ForAll { variable, body: inner } = body {
            if params.contains(variable) {
                return false; // shadowed binder — instantiation would be ambiguous
            }
            params.push(variable.clone());
            body = inner;
        }
        // Peel the condition chain.
        let mut conds: Vec<ProofExpr> = Vec::new();
        while let ProofExpr::Implies(c, rest) = body {
            conds.push((**c).clone());
            body = rest;
        }
        let core = match body {
            ProofExpr::Identity(l, r) => {
                if matches!(l, ProofTerm::Variable(_)) {
                    return false; // a bare-variable lhs matches everything
                }
                RuleCore::TermEq { lhs: l.clone(), rhs: r.clone() }
            }
            ProofExpr::Iff(l, r) => {
                RuleCore::PropIff { lhs: (**l).clone(), rhs: (**r).clone() }
            }
            _ => return false,
        };
        // Orientation: everything the rhs and the conditions mention, the lhs
        // must bind — matching the lhs then determines the instantiation.
        let lhs_vars = match &core {
            RuleCore::TermEq { lhs, .. } => term_vars(lhs),
            RuleCore::PropIff { lhs, .. } => expr_vars(lhs),
        };
        let rhs_vars = match &core {
            RuleCore::TermEq { rhs, .. } => term_vars(rhs),
            RuleCore::PropIff { rhs, .. } => expr_vars(rhs),
        };
        if !rhs_vars.iter().all(|v| lhs_vars.contains(v)) {
            return false;
        }
        for c in &conds {
            if !expr_vars(c).iter().all(|v| lhs_vars.contains(v)) {
                return false;
            }
        }
        // Every parameter must actually be determined by the lhs.
        if !params.iter().all(|p| lhs_vars.contains(p)) {
            return false;
        }

        let idx = self.rules.len();
        match &core {
            RuleCore::TermEq { lhs, .. } => self.term_index.insert_term(lhs, idx),
            RuleCore::PropIff { lhs, .. } => self.iff_index.insert_expr(lhs, idx),
        }
        self.rules.push(CompiledRule {
            name: name.to_string(),
            params,
            conds,
            core,
            lemma: lemma.clone(),
            base: DerivationTree::leaf(lemma.clone(), InferenceRule::PremiseMatch),
        });
        true
    }

    /// Find one term-rewrite step in `goal`, innermost subterms first.
    /// `hyps` are the in-scope facts (proposition + its derivation), used to
    /// discharge rule conditions.
    pub(crate) fn find_term_step(
        &self,
        goal: &ProofExpr,
        hyps: &[(&ProofExpr, &DerivationTree)],
    ) -> Option<RewriteStep> {
        let mut subterms = Vec::new();
        collect_goal_terms(goal, &mut subterms);
        for t in subterms {
            let mut cand: Vec<usize> = self.term_index.candidates_term(t).into_iter().copied().collect();
            cand.sort_unstable();
            cand.dedup();
            for idx in cand {
                if let Some(step) = self.try_term_rule(idx, t, hyps) {
                    return Some(step);
                }
            }
        }
        None
    }

    /// Find a top-level propositional step: an iff rule whose lhs matches the
    /// whole goal.
    pub(crate) fn find_iff_step(
        &self,
        goal: &ProofExpr,
        hyps: &[(&ProofExpr, &DerivationTree)],
    ) -> Option<IffStep> {
        let mut cand: Vec<usize> = self.iff_index.candidates_expr(goal).into_iter().copied().collect();
        cand.sort_unstable();
        cand.dedup();
        for idx in cand {
            let rule = &self.rules[idx];
            let RuleCore::PropIff { lhs, .. } = &rule.core else { continue };
            let Some(subst) = match_expr_pattern(lhs, goal) else { continue };
            let Some((tree, concl)) = instantiate_rule(rule, &subst, hyps) else { continue };
            let ProofExpr::Iff(l, r) = &concl else { continue };
            if l.as_ref() != goal {
                continue;
            }
            // Project the right-to-left implication (Iff ≡ And(→, ←)).
            let imp = DerivationTree::new(
                ProofExpr::Implies(r.clone(), l.clone()),
                InferenceRule::ConjunctionElim,
                vec![tree],
            );
            return Some(IffStep { imp, rhs: (**r).clone() });
        }
        None
    }

    fn try_term_rule(
        &self,
        idx: usize,
        t: &ProofTerm,
        hyps: &[(&ProofExpr, &DerivationTree)],
    ) -> Option<RewriteStep> {
        let rule = &self.rules[idx];
        let RuleCore::TermEq { lhs, rhs } = &rule.core else { return None };
        let subst = match_term_pattern(lhs, t)?;
        let (tree, concl) = instantiate_rule(rule, &subst, hyps)?;
        let ProofExpr::Identity(from, to) = concl else { return None };
        if &from != t {
            return None;
        }
        // A no-op rewrite (t = t) makes no progress and would loop.
        if from == to {
            return None;
        }
        Some(RewriteStep { eq: tree, from, to })
    }
}

/// Instantiate a compiled rule under `subst`: chain `UniversalInstTerm` for
/// each parameter, then discharge each condition by `ModusPonens` against the
/// in-scope facts (or ground arithmetic). Returns the derivation and its
/// conclusion (the instantiated core).
fn instantiate_rule(
    rule: &CompiledRule,
    subst: &Substitution,
    hyps: &[(&ProofExpr, &DerivationTree)],
) -> Option<(DerivationTree, ProofExpr)> {
    let mut tree = rule.base.clone();
    let mut expr = rule.lemma.clone();
    for param in &rule.params {
        let witness = subst.get(param)?.clone();
        let ProofExpr::ForAll { variable, body } = expr else { return None };
        debug_assert_eq!(&variable, param);
        let mut single = Substitution::new();
        single.insert(param.clone(), witness.clone());
        let inst = apply_subst_to_expr(&body, &single);
        tree = DerivationTree::new(
            inst.clone(),
            InferenceRule::UniversalInstTerm(witness),
            vec![tree],
        );
        expr = inst;
    }
    for _ in 0..rule.conds.len() {
        let ProofExpr::Implies(cond, rest) = expr else { return None };
        let cond_proof = discharge_condition(&cond, hyps)?;
        tree = DerivationTree::new(
            (*rest).clone(),
            InferenceRule::ModusPonens,
            vec![tree, cond_proof],
        );
        expr = *rest;
    }
    Some((tree, expr))
}

/// Discharge an instantiated condition: an in-scope fact that matches it
/// exactly, or a ground arithmetic identity. Depth-1 by design — conditions
/// do not recursively invoke the rewriter.
fn discharge_condition(
    cond: &ProofExpr,
    hyps: &[(&ProofExpr, &DerivationTree)],
) -> Option<DerivationTree> {
    if let Some((_, proof)) = hyps.iter().find(|(prop, _)| *prop == cond) {
        return Some((*proof).clone());
    }
    if let ProofExpr::Identity(l, r) = cond {
        if ground_int(l).is_some() && ground_int(r).is_some() {
            return Some(DerivationTree::leaf(cond.clone(), InferenceRule::ArithDecision));
        }
    }
    None
}

/// Find one ground arithmetic fold (`add(2, 3) ⇝ 5`), innermost first. The
/// equality is an `ArithDecision` leaf — the certifier re-runs the arithmetic
/// oracle and the kernel checks the resulting term.
pub(crate) fn find_ground_fold(goal: &ProofExpr) -> Option<RewriteStep> {
    let mut subterms = Vec::new();
    collect_goal_terms(goal, &mut subterms);
    for t in subterms {
        let ProofTerm::Function(op, args) = t else { continue };
        if args.len() != 2 {
            continue;
        }
        let (Some(a), Some(b)) = (ground_int(&args[0]), ground_int(&args[1])) else {
            continue;
        };
        let result = match op.as_str() {
            "add" => a.checked_add(b),
            "sub" => a.checked_sub(b),
            "mul" => a.checked_mul(b),
            _ => None,
        }?;
        let to = ProofTerm::Constant(result.to_string());
        if &to == t {
            continue;
        }
        let eq = DerivationTree::leaf(
            ProofExpr::Identity(t.clone(), to.clone()),
            InferenceRule::ArithDecision,
        );
        return Some(RewriteStep { eq, from: t.clone(), to });
    }
    None
}

fn ground_int(t: &ProofTerm) -> Option<i64> {
    match t {
        ProofTerm::Constant(s) => s.parse::<i64>().ok(),
        _ => None,
    }
}

/// The subterms of a goal in post-order (innermost first), without duplicates.
fn collect_goal_terms<'a>(e: &'a ProofExpr, out: &mut Vec<&'a ProofTerm>) {
    fn walk_term<'a>(t: &'a ProofTerm, out: &mut Vec<&'a ProofTerm>) {
        match t {
            ProofTerm::Function(_, args) | ProofTerm::Group(args) => {
                for a in args {
                    walk_term(a, out);
                }
            }
            _ => {}
        }
        if !out.contains(&t) {
            out.push(t);
        }
    }
    match e {
        ProofExpr::Predicate { args, .. } => {
            for a in args {
                walk_term(a, out);
            }
        }
        ProofExpr::Identity(l, r) => {
            walk_term(l, out);
            walk_term(r, out);
        }
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            collect_goal_terms(l, out);
            collect_goal_terms(r, out);
        }
        ProofExpr::Not(p) => collect_goal_terms(p, out),
        ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
            collect_goal_terms(body, out)
        }
        _ => {}
    }
}

fn term_vars(t: &ProofTerm) -> Vec<String> {
    let mut out = Vec::new();
    collect_term_vars(t, &mut out);
    out
}

fn collect_term_vars(t: &ProofTerm, out: &mut Vec<String>) {
    match t {
        ProofTerm::Variable(n) => {
            if !out.contains(n) {
                out.push(n.clone());
            }
        }
        ProofTerm::Function(_, args) | ProofTerm::Group(args) => {
            for a in args {
                collect_term_vars(a, out);
            }
        }
        _ => {}
    }
}

fn expr_vars(e: &ProofExpr) -> Vec<String> {
    let mut out = Vec::new();
    collect_expr_vars(e, &mut out);
    out
}

fn collect_expr_vars(e: &ProofExpr, out: &mut Vec<String>) {
    match e {
        ProofExpr::Predicate { args, .. } => {
            for a in args {
                collect_term_vars(a, out);
            }
        }
        ProofExpr::Identity(l, r) => {
            collect_term_vars(l, out);
            collect_term_vars(r, out);
        }
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            collect_expr_vars(l, out);
            collect_expr_vars(r, out);
        }
        ProofExpr::Not(p) => collect_expr_vars(p, out),
        ProofExpr::ForAll { variable, body } | ProofExpr::Exists { variable, body } => {
            let mut inner = Vec::new();
            collect_expr_vars(body, &mut inner);
            for v in inner {
                if v != *variable && !out.contains(&v) {
                    out.push(v);
                }
            }
        }
        _ => {}
    }
}

