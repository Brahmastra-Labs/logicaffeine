//! A tactic framework with a first-class goal state — the interactive proof
//! interface (ROOT R5 of the Lean-competitive architecture).
//!
//! A [`ProofState`] holds the open [`Goal`]s; a *tactic* transforms the focused
//! goal, replacing it with zero or more subgoals and recording the inference rule
//! used. When no goals remain, [`ProofState::qed`] assembles the recorded steps
//! into a single [`DerivationTree`] and runs it through the SAME trust door as
//! every other proof ([`check_derivation`]): the certifier turns it into a kernel
//! term and the kernel re-checks that term against the goal. So a tactic proof is
//! exactly as trusted as an automated one — the tactics only *build* the
//! derivation the kernel validates, they are never themselves trusted.
//!
//! The backward chainer is hosted here as the [`ProofState::auto`] tactic, so the
//! existing automation is one tactic among many rather than the whole story.

use std::collections::VecDeque;

use crate::engine::BackwardChainer;
use crate::unify::{apply_subst_to_expr, unify_exprs, Substitution};
use crate::verify::{check_derivation, VerifiedProof};
use crate::{
    DerivationTree, InductionArg, InductionCase, InferenceRule, ProofExpr, ProofTerm,
};

/// A local hypothesis: its `name`, its proposition `prop`, and `proof` — a
/// derivation that proves `prop` in the current local context. For a hypothesis
/// that is directly in scope (a premise, an `intro`-discharged antecedent, or a
/// branch hypothesis bound by `cases`) the proof is a `PremiseMatch` leaf the
/// certifier resolves against the bound/registered hypothesis. For a conjunct
/// extracted by `cases` from `A ∧ B`, the proof is the `ConjunctionElim`
/// PROJECTION of the parent — so using the conjunct emits the projection, not a
/// dangling reference. This is what lets elimination compose with the rest.
#[derive(Debug, Clone)]
pub struct Hyp {
    pub name: String,
    pub prop: ProofExpr,
    proof: DerivationTree,
}

/// One constructor of an inductive, as given to [`ProofState::induction_over`]: its
/// name and, per positional argument, whether that argument is itself of the
/// inductive type — a recursive position that carries an induction hypothesis.
/// `Nil` is `CtorSpec { constructor: "Nil", recursive: vec![] }`; `Cons` (head then
/// tail) is `CtorSpec { constructor: "Cons", recursive: vec![false, true] }`.
#[derive(Debug, Clone)]
pub struct CtorSpec {
    /// The constructor's name, as registered in the kernel (e.g. `"Cons"`).
    pub constructor: String,
    /// One flag per argument, in order: `true` iff that argument is recursive.
    pub recursive: Vec<bool>,
}

/// A single proof obligation: prove `target` under the local `hyps`.
#[derive(Debug, Clone)]
pub struct Goal {
    /// Named local hypotheses in scope (introduced by `intro`/`cases`, or premises).
    pub hyps: Vec<Hyp>,
    /// The proposition to prove.
    pub target: ProofExpr,
}

/// What went wrong applying a tactic.
#[derive(Debug, Clone, PartialEq)]
pub enum TacticError {
    /// No focused goal — the proof state is already complete.
    NoOpenGoals,
    /// The tactic does not apply to the focused goal's shape (e.g. `intro` on an atom).
    DoesNotApply(String),
    /// No hypothesis (or premise) matches the goal for `assumption`/`exact`.
    NoSuchHypothesis(String),
    /// The backward chainer could not close the goal.
    AutoFailed,
    /// `qed` was called with goals still open.
    GoalsRemain(usize),
}

/// A child of a refinement: either an already-complete proof or a fresh subgoal.
enum Child {
    Closed(DerivationTree),
    Sub(Goal),
}

/// A node in the proof under construction.
#[derive(Clone)]
enum Node {
    /// An unproved goal (a metavariable / hole).
    Hole(Goal),
    /// A complete sub-derivation (closed by `exact`/`assumption`/`auto`).
    Done(DerivationTree),
    /// A refinement: `rule` applied with the given children (indices into `nodes`).
    Filled {
        conclusion: ProofExpr,
        rule: InferenceRule,
        children: Vec<usize>,
    },
}

/// The interactive proof state: the partial derivation plus the queue of open
/// goals (front = focused). Tactics consume the focused goal and push any subgoals
/// they create back to the front, so the proof is built depth-first, first-subgoal
/// first — the order a reader expects. `Clone` is cheap relative to a proof and is
/// what lets the backtracking combinators (`first`/`try`/`repeat`) speculate.
#[derive(Clone)]
pub struct ProofState {
    premises: Vec<ProofExpr>,
    goal: ProofExpr,
    nodes: Vec<Node>,
    root: usize,
    open: VecDeque<usize>,
    fresh: usize,
}

/// Replace every occurrence of the term `from` with `to` inside a term.
fn replace_in_term(t: &ProofTerm, from: &ProofTerm, to: &ProofTerm) -> ProofTerm {
    if t == from {
        return to.clone();
    }
    match t {
        ProofTerm::Function(n, args) => ProofTerm::Function(
            n.clone(),
            args.iter().map(|a| replace_in_term(a, from, to)).collect(),
        ),
        ProofTerm::Group(args) => {
            ProofTerm::Group(args.iter().map(|a| replace_in_term(a, from, to)).collect())
        }
        other => other.clone(),
    }
}

/// Replace every occurrence of the term `from` with `to` throughout an expression —
/// the Leibniz substitution `rewrite` performs on the goal.
fn replace_in_expr(e: &ProofExpr, from: &ProofTerm, to: &ProofTerm) -> ProofExpr {
    let re = |x: &ProofExpr| Box::new(replace_in_expr(x, from, to));
    let rt = |x: &ProofTerm| replace_in_term(x, from, to);
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
        other => other.clone(),
    }
}

/// Rewrite the eigen-`Constant` `name` back to `Variable(name)` throughout a
/// derivation — the generalization step for `induction`'s step case, turning the
/// rigid search variable into the one the certifier's `Match` arm binds.
fn eigen_to_var_tree(tree: &DerivationTree, name: &str) -> DerivationTree {
    let from = ProofTerm::Constant(name.to_string());
    let to = ProofTerm::Variable(name.to_string());
    let rule = match &tree.rule {
        InferenceRule::Rewrite { from: rf, to: rt } => InferenceRule::Rewrite {
            from: replace_in_term(rf, &from, &to),
            to: replace_in_term(rt, &from, &to),
        },
        other => other.clone(),
    };
    DerivationTree {
        conclusion: replace_in_expr(&tree.conclusion, &from, &to),
        rule,
        premises: tree.premises.iter().map(|p| eigen_to_var_tree(p, name)).collect(),
        depth: tree.depth,
        substitution: tree.substitution.clone(),
    }
}

/// A hypothesis directly in scope (premise / `intro` / `cases`-branch): its proof
/// is a `PremiseMatch` leaf the certifier resolves against the bound or registered
/// hypothesis of the same proposition.
fn direct_hyp(name: String, prop: ProofExpr) -> Hyp {
    let proof = DerivationTree::leaf(prop.clone(), InferenceRule::PremiseMatch);
    Hyp { name, prop, proof }
}

impl ProofState {
    /// Begin proving `goal` from `premises`, naming the hypotheses `hp0`, `hp1`, ….
    pub fn start(premises: Vec<ProofExpr>, goal: ProofExpr) -> Self {
        Self::start_with_names(premises, &[], goal)
    }

    /// Begin proving `goal` from `premises`, using the given `names` for the
    /// hypotheses where present (parallel to `premises`), falling back to the
    /// positional `hp{i}` otherwise. Lets a surface `Given (h): …` name a premise so a
    /// `Proof:` script can say `cases h` instead of `cases hp0`.
    pub fn start_with_names(
        premises: Vec<ProofExpr>,
        names: &[Option<String>],
        goal: ProofExpr,
    ) -> Self {
        let hyps = premises
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let name = names
                    .get(i)
                    .and_then(|n| n.as_ref())
                    .cloned()
                    .unwrap_or_else(|| format!("hp{i}"));
                direct_hyp(name, p.clone())
            })
            .collect();
        let root_goal = Goal { hyps, target: goal.clone() };
        ProofState {
            premises,
            goal,
            nodes: vec![Node::Hole(root_goal)],
            root: 0,
            open: VecDeque::from([0]),
            fresh: 0,
        }
    }

    /// A fresh entity-constant name for an `∃`-elimination witness. Uppercase-leading
    /// so the verifier reads it as a constant, not a variable.
    fn fresh_witness(&mut self) -> String {
        self.fresh += 1;
        format!("W{}", self.fresh)
    }

    /// Replace the focused hole's goal in place (used by `cases` on `∧`, which
    /// enriches the hypotheses without splitting the goal).
    fn set_focused_goal(&mut self, goal: Goal) {
        if let Some(&idx) = self.open.front() {
            self.nodes[idx] = Node::Hole(goal);
        }
    }

    /// The focused goal, or `None` if the proof is complete.
    pub fn focus(&self) -> Option<&Goal> {
        let idx = *self.open.front()?;
        match &self.nodes[idx] {
            Node::Hole(g) => Some(g),
            _ => None,
        }
    }

    /// Number of still-open goals.
    pub fn open_goals(&self) -> usize {
        self.open.len()
    }

    /// The focused goal's target, if any goal is open — the tactic-script and
    /// IDE surface for inspecting where a proof stands.
    pub fn focused_target(&self) -> Option<&ProofExpr> {
        self.focus().map(|g| &g.target)
    }

    /// A [`crate::simp::SimpSet`] compiled from every in-scope hypothesis that
    /// orients as a rewrite rule — the default rule set for the script-level
    /// `simp` (a premise or cited lemma of rule shape is automatically a rule).
    pub fn scope_simp_set(&self) -> crate::simp::SimpSet {
        let mut set = crate::simp::SimpSet::new();
        if let Some(g) = self.focus() {
            for h in &g.hyps {
                set.register_lemma(&h.name, &h.prop);
            }
        }
        set
    }

    fn focused_goal(&self) -> Result<Goal, TacticError> {
        self.focus().cloned().ok_or(TacticError::NoOpenGoals)
    }

    /// Replace the focused hole with a refinement node whose children are the given
    /// closed proofs / subgoals (in order). New subgoals are focused next, in order.
    fn refine(&mut self, conclusion: ProofExpr, rule: InferenceRule, children: Vec<Child>) {
        let idx = self.open.pop_front().expect("a focused goal");
        let mut child_indices = Vec::with_capacity(children.len());
        let mut new_holes = Vec::new();
        for child in children {
            let cidx = self.nodes.len();
            match child {
                Child::Closed(tree) => self.nodes.push(Node::Done(tree)),
                Child::Sub(goal) => {
                    self.nodes.push(Node::Hole(goal));
                    new_holes.push(cidx);
                }
            }
            child_indices.push(cidx);
        }
        // Focus the new subgoals next, first one at the front.
        for &h in new_holes.iter().rev() {
            self.open.push_front(h);
        }
        self.nodes[idx] = Node::Filled { conclusion, rule, children: child_indices };
    }

    /// Close the focused goal with an already-complete derivation.
    fn close(&mut self, tree: DerivationTree) {
        let idx = self.open.pop_front().expect("a focused goal");
        self.nodes[idx] = Node::Done(tree);
    }

    // === Tactics ===

    /// `intro name`: for a goal `P → Q`, assume `P` as hypothesis `name` and reduce
    /// to `Q`; for `∀x. φ(x)`, introduce the bound variable under `name` and reduce
    /// to `φ(name)`. Mirrors Lean's `intro`.
    pub fn intro(&mut self, name: &str) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        match &g.target {
            ProofExpr::Implies(p, q) => {
                let mut hyps = g.hyps.clone();
                hyps.push(direct_hyp(name.to_string(), (**p).clone()));
                let sub = Goal { hyps, target: (**q).clone() };
                self.refine(g.target.clone(), InferenceRule::ImpliesIntro, vec![Child::Sub(sub)]);
                Ok(self)
            }
            ProofExpr::ForAll { variable, body } => {
                // Introduce the bound variable under `name`. When `name` already IS the
                // binder, skip the rename — substituting `{z ↦ z}` would chase its own
                // chain forever in `apply_subst_to_term`. The eigenvariable soundness
                // worry of search does not apply here: the user constructs the proof,
                // they never *instantiate* the introduced variable.
                let (conclusion, renamed) = if name == variable {
                    (g.target.clone(), (**body).clone())
                } else {
                    let mut subst = Substitution::new();
                    subst.insert(variable.clone(), ProofTerm::Variable(name.to_string()));
                    let renamed = apply_subst_to_expr(body, &subst);
                    let conclusion = ProofExpr::ForAll {
                        variable: name.to_string(),
                        body: Box::new(renamed.clone()),
                    };
                    (conclusion, renamed)
                };
                let sub = Goal { hyps: g.hyps.clone(), target: renamed };
                self.refine(
                    conclusion,
                    InferenceRule::UniversalIntro {
                        variable: name.to_string(),
                        var_type: "Entity".to_string(),
                    },
                    vec![Child::Sub(sub)],
                );
                Ok(self)
            }
            other => Err(TacticError::DoesNotApply(format!(
                "intro expects → or ∀, got {other:?}"
            ))),
        }
    }

    /// `assumption`: close the goal with any hypothesis whose proposition is exactly
    /// the target, emitting THAT hypothesis's proof (a direct reference, or a
    /// `ConjunctionElim` projection for a conjunct extracted by `cases`).
    pub fn assumption(&mut self) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        if let Some(h) = g.hyps.iter().find(|h| h.prop == g.target) {
            let proof = h.proof.clone();
            self.close(proof);
            Ok(self)
        } else {
            Err(TacticError::NoSuchHypothesis(format!(
                "no hypothesis proves {:?}",
                g.target
            )))
        }
    }

    /// `exact name`: close the goal with the specifically-named hypothesis, which
    /// must prove the target.
    pub fn exact(&mut self, name: &str) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        match g.hyps.iter().find(|h| h.name == name) {
            Some(h) if h.prop == g.target => {
                let proof = h.proof.clone();
                self.close(proof);
                Ok(self)
            }
            Some(h) => Err(TacticError::DoesNotApply(format!(
                "hypothesis {name} : {:?} does not prove {:?}",
                h.prop, g.target
            ))),
            None => Err(TacticError::NoSuchHypothesis(name.to_string())),
        }
    }

    /// `constructor` / `split`: for a goal `A ∧ B`, reduce to the two subgoals `A`
    /// and `B`.
    pub fn split(&mut self) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        match &g.target {
            ProofExpr::And(a, b) => {
                let ga = Goal { hyps: g.hyps.clone(), target: (**a).clone() };
                let gb = Goal { hyps: g.hyps.clone(), target: (**b).clone() };
                self.refine(
                    g.target.clone(),
                    InferenceRule::ConjunctionIntro,
                    vec![Child::Sub(ga), Child::Sub(gb)],
                );
                Ok(self)
            }
            other => Err(TacticError::DoesNotApply(format!("split expects ∧, got {other:?}"))),
        }
    }

    /// `left`: prove `A ∨ B` by proving `A`.
    pub fn left(&mut self) -> Result<&mut Self, TacticError> {
        self.disjunct(true)
    }

    /// `right`: prove `A ∨ B` by proving `B`.
    pub fn right(&mut self) -> Result<&mut Self, TacticError> {
        self.disjunct(false)
    }

    fn disjunct(&mut self, take_left: bool) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        match &g.target {
            ProofExpr::Or(a, b) => {
                let chosen = if take_left { (**a).clone() } else { (**b).clone() };
                let sub = Goal { hyps: g.hyps.clone(), target: chosen };
                self.refine(
                    g.target.clone(),
                    InferenceRule::DisjunctionIntro,
                    vec![Child::Sub(sub)],
                );
                Ok(self)
            }
            other => Err(TacticError::DoesNotApply(format!(
                "left/right expects ∨, got {other:?}"
            ))),
        }
    }

    /// `exists w`: prove `∃x. φ(x)` by exhibiting the witness `w` and reducing to
    /// `φ(w)`.
    pub fn exists(&mut self, witness: ProofTerm) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        match &g.target {
            ProofExpr::Exists { variable, body } => {
                let mut subst = Substitution::new();
                subst.insert(variable.clone(), witness.clone());
                let instantiated = apply_subst_to_expr(body, &subst);
                let sub = Goal { hyps: g.hyps.clone(), target: instantiated };
                self.refine(
                    g.target.clone(),
                    InferenceRule::ExistentialIntro {
                        witness: format!("{witness}"),
                        witness_type: "Entity".to_string(),
                    },
                    vec![Child::Sub(sub)],
                );
                Ok(self)
            }
            other => Err(TacticError::DoesNotApply(format!("exists expects ∃, got {other:?}"))),
        }
    }

    /// `apply rule`: `rule` is `P → Goal` (a hypothesis or premise). Reduce the goal
    /// to its antecedent `P` by modus ponens.
    pub fn apply(&mut self, rule: &ProofExpr) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        if let ProofExpr::Implies(ant, con) = rule {
            if unify_exprs(&g.target, con).is_ok() {
                let rule_leaf = DerivationTree::leaf(rule.clone(), InferenceRule::PremiseMatch);
                let sub = Goal { hyps: g.hyps.clone(), target: (**ant).clone() };
                self.refine(
                    g.target.clone(),
                    InferenceRule::ModusPonens,
                    vec![Child::Closed(rule_leaf), Child::Sub(sub)],
                );
                return Ok(self);
            }
        }
        Err(TacticError::DoesNotApply(format!(
            "apply: {rule:?} is not an implication whose conclusion is the goal"
        )))
    }

    /// `cases h` / `destruct h`: eliminate a hypothesis by its shape.
    /// - `A ∧ B`: add `h_1 : A` and `h_2 : B` to the goal, each backed by the
    ///   `ConjunctionElim` projection of `h` — so using a conjunct emits the
    ///   projection, never a dangling reference.
    /// - `A ∨ B`: split into two goals, one assuming `A`, one assuming `B`
    ///   (`DisjunctionCases`).
    /// - `∃x. φ`: introduce a fresh witness `c` and the hypothesis `φ(c)`
    ///   (`ExistentialElim`); the goal must not mention `c`, which holds since it is
    ///   fresh.
    pub fn cases(&mut self, name: &str) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        let hyp = g
            .hyps
            .iter()
            .find(|h| h.name == name)
            .cloned()
            .ok_or_else(|| TacticError::NoSuchHypothesis(name.to_string()))?;
        match &hyp.prop {
            ProofExpr::And(a, b) => {
                let proj_a = DerivationTree::new(
                    (**a).clone(),
                    InferenceRule::ConjunctionElim,
                    vec![hyp.proof.clone()],
                );
                let proj_b = DerivationTree::new(
                    (**b).clone(),
                    InferenceRule::ConjunctionElim,
                    vec![hyp.proof.clone()],
                );
                let mut hyps = g.hyps.clone();
                hyps.push(Hyp { name: format!("{name}_1"), prop: (**a).clone(), proof: proj_a });
                hyps.push(Hyp { name: format!("{name}_2"), prop: (**b).clone(), proof: proj_b });
                self.set_focused_goal(Goal { hyps, target: g.target.clone() });
                Ok(self)
            }
            ProofExpr::Or(a, b) => {
                let mut ha = g.hyps.clone();
                ha.push(direct_hyp(format!("{name}_l"), (**a).clone()));
                let ga = Goal { hyps: ha, target: g.target.clone() };
                let mut hb = g.hyps.clone();
                hb.push(direct_hyp(format!("{name}_r"), (**b).clone()));
                let gb = Goal { hyps: hb, target: g.target.clone() };
                self.refine(
                    g.target.clone(),
                    InferenceRule::DisjunctionCases,
                    vec![Child::Closed(hyp.proof.clone()), Child::Sub(ga), Child::Sub(gb)],
                );
                Ok(self)
            }
            ProofExpr::Exists { variable, body } => {
                let witness = self.fresh_witness();
                let mut subst = Substitution::new();
                subst.insert(variable.clone(), ProofTerm::Constant(witness.clone()));
                let phi_c = apply_subst_to_expr(body, &subst);
                let mut hyps = g.hyps.clone();
                hyps.push(direct_hyp(format!("{name}_w"), phi_c));
                let gbody = Goal { hyps, target: g.target.clone() };
                self.refine(
                    g.target.clone(),
                    InferenceRule::ExistentialElim { witness },
                    vec![Child::Closed(hyp.proof.clone()), Child::Sub(gbody)],
                );
                Ok(self)
            }
            other => Err(TacticError::DoesNotApply(format!(
                "cases expects ∧/∨/∃ hypothesis, got {other:?}"
            ))),
        }
    }

    /// `induction`: on a goal `∀n. P(n)` over `Nat`, split into the base case `P(Zero)`
    /// and the step case `P(Succ k)` with the induction hypothesis `ih : P(k)` in scope.
    /// Certified as a `Fix`/`Match` over the kernel's `Nat` recursor; the IH resolves to
    /// the recursive call. (Nat only for now — `List`/general inductives need the
    /// dependent `InductionScheme`.)
    pub fn induction(&mut self) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        let (variable, body) = match &g.target {
            ProofExpr::ForAll { variable, body } => (variable.clone(), (**body).clone()),
            other => {
                return Err(TacticError::DoesNotApply(format!(
                    "induction expects a ∀ goal, got {other:?}"
                )))
            }
        };
        self.fresh += 1;
        // Uppercase so the search treats the step variable as a rigid eigen-CONSTANT
        // (never a unifiable metavariable, which the backward chainer would either
        // ground to `Zero` or expand into an infinite `P(Succ^n k)` chain). It is
        // remapped back to a bound `Variable` when the proof is assembled (see
        // `assemble`), which is what the certifier's `Match` arm binds.
        let step_var = format!("K{}", self.fresh);
        let subst_to = |term: ProofTerm| {
            let mut s = Substitution::new();
            s.insert(variable.clone(), term);
            apply_subst_to_expr(&body, &s)
        };
        let base_target = subst_to(ProofTerm::Constant("Zero".to_string()));
        let succ_k =
            ProofTerm::Function("Succ".to_string(), vec![ProofTerm::Constant(step_var.clone())]);
        let step_target = subst_to(succ_k);
        let ih_prop = subst_to(ProofTerm::Constant(step_var.clone()));

        let base_goal = Goal { hyps: g.hyps.clone(), target: base_target };
        // Put the induction hypothesis FIRST so `auto`/`assumption` discharge the step's
        // recursive obligation `P(k)` against the IH (keeping `k` symbolic) rather than
        // against a base fact like `P(Zero)` (which would unsoundly ground `k := Zero`).
        let mut step_hyps = vec![direct_hyp("ih".to_string(), ih_prop)];
        step_hyps.extend(g.hyps.clone());
        let step_goal = Goal { hyps: step_hyps, target: step_target };

        self.refine(
            g.target.clone(),
            InferenceRule::StructuralInduction {
                variable,
                ind_type: "Nat".to_string(),
                step_var,
            },
            vec![Child::Sub(base_goal), Child::Sub(step_goal)],
        );
        Ok(self)
    }

    /// `induction_over`: generic structural induction over ANY inductive `ind_type`,
    /// one subgoal per constructor. On a goal `∀x. P(x)`, each constructor `C a₀…aₙ`
    /// yields the subgoal `P(C a₀ … aₙ)`; every recursive argument `aᵢ` (one of the
    /// inductive type) contributes an induction hypothesis `P(aᵢ)`, placed FIRST so
    /// `auto`/`assumption` discharge the recursive obligation against the IH rather
    /// than a base fact. Generalizes [`induction`](Self::induction) (fixed to the
    /// nullary-base + unary-step Nat shape) to constructors with several — or zero —
    /// recursive positions (`Nil`/`Cons`, `Leaf`/`Node`, an N-way enum). Assembled as
    /// the dependent `InductionScheme` eliminator (`fix rec. λx. match x { … }`), which
    /// the kernel re-checks for coverage, case types, and termination.
    ///
    /// The constructor arguments run the search as rigid eigen-CONSTANTS (so the
    /// backward chainer neither grounds nor unrolls them); [`assemble`](Self::assemble)
    /// remaps each back to the bound `Variable` the certifier's `match` arm binds.
    pub fn induction_over(
        &mut self,
        ind_type: &str,
        ctors: Vec<CtorSpec>,
    ) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        let (variable, body) = match &g.target {
            ProofExpr::ForAll { variable, body } => (variable.clone(), (**body).clone()),
            other => {
                return Err(TacticError::DoesNotApply(format!(
                    "induction expects a ∀ goal, got {other:?}"
                )))
            }
        };
        let subst_to = |term: ProofTerm| {
            let mut s = Substitution::new();
            s.insert(variable.clone(), term);
            apply_subst_to_expr(&body, &s)
        };

        let mut cases = Vec::with_capacity(ctors.len());
        let mut children = Vec::with_capacity(ctors.len());
        for ctor in &ctors {
            // A fresh rigid eigenconstant per constructor argument.
            let arg_names: Vec<String> = ctor
                .recursive
                .iter()
                .map(|_| {
                    self.fresh += 1;
                    format!("K{}", self.fresh)
                })
                .collect();

            // The constructor applied to its eigenconstant arguments — `C` (a bare
            // `Constant`) when nullary, else `C(a₀, …, aₙ)`.
            let ctor_term = if arg_names.is_empty() {
                ProofTerm::Constant(ctor.constructor.clone())
            } else {
                ProofTerm::Function(
                    ctor.constructor.clone(),
                    arg_names.iter().map(|n| ProofTerm::Constant(n.clone())).collect(),
                )
            };
            let case_target = subst_to(ctor_term);

            // IHs for the recursive arguments, FIRST (see the Nat induction rationale).
            let mut hyps = Vec::new();
            for (arg, &is_rec) in arg_names.iter().zip(ctor.recursive.iter()) {
                if is_rec {
                    let ih_prop = subst_to(ProofTerm::Constant(arg.clone()));
                    hyps.push(direct_hyp(format!("ih_{arg}"), ih_prop));
                }
            }
            hyps.extend(g.hyps.clone());
            children.push(Child::Sub(Goal { hyps, target: case_target }));

            cases.push(InductionCase {
                constructor: ctor.constructor.clone(),
                args: arg_names
                    .iter()
                    .zip(ctor.recursive.iter())
                    .map(|(name, &recursive)| InductionArg { name: name.clone(), recursive })
                    .collect(),
            });
        }

        self.refine(
            g.target.clone(),
            InferenceRule::InductionScheme { variable, ind_type: ind_type.to_string(), cases },
            children,
        );
        Ok(self)
    }

    /// `rewrite h`: `h : lhs = rhs` rewrites every occurrence of `lhs` to `rhs` in the
    /// goal, reducing it to `goal[lhs := rhs]`. Certified by `Eq_rec` (Leibniz): the
    /// node concludes the original goal (which contains `lhs = to`), its equality
    /// premise proves `rhs = lhs` (the symmetric of `h`, so `from = rhs`, `to = lhs`),
    /// and its source premise is the rewritten subgoal. Fails if `lhs` does not occur.
    pub fn rewrite(&mut self, name: &str) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        let hyp = g
            .hyps
            .iter()
            .find(|h| h.name == name)
            .cloned()
            .ok_or_else(|| TacticError::NoSuchHypothesis(name.to_string()))?;
        let (lhs, rhs) = match &hyp.prop {
            ProofExpr::Identity(l, r) => (l.clone(), r.clone()),
            other => {
                return Err(TacticError::DoesNotApply(format!(
                    "rewrite expects an equality hypothesis, got {other:?}"
                )))
            }
        };
        let subgoal_target = replace_in_expr(&g.target, &lhs, &rhs);
        if subgoal_target == g.target {
            return Err(TacticError::DoesNotApply(format!(
                "rewrite: {lhs} does not occur in the goal"
            )));
        }
        // The equality premise must conclude `rhs = lhs` — the symmetric of `h`.
        let eq_sym = DerivationTree::new(
            ProofExpr::Identity(rhs.clone(), lhs.clone()),
            InferenceRule::EqualitySymmetry,
            vec![hyp.proof.clone()],
        );
        let sub = Goal { hyps: g.hyps.clone(), target: subgoal_target };
        self.refine(
            g.target.clone(),
            InferenceRule::Rewrite { from: rhs, to: lhs },
            vec![Child::Closed(eq_sym), Child::Sub(sub)],
        );
        Ok(self)
    }

    /// `simp`: normalize the focused goal by the rule set, left-to-right to a
    /// fixpoint, then close it if it became trivial (reflexivity or an exact
    /// hypothesis). Mirrors Lean's `simp`.
    ///
    /// Each rewrite is one certified `Rewrite` refinement — the instantiated
    /// rule equality (a `UniversalInstTerm` chain over the lemma, conditions
    /// discharged by `ModusPonens`) is the closed premise, the rewritten goal
    /// the open one — so `qed` re-checks every step in the kernel. Loop
    /// protection is a step budget plus a seen-goal set (commuting rules like
    /// `a = b, b = a` terminate instead of ping-ponging). Fails only if it
    /// neither rewrote nor closed anything.
    pub fn simp(&mut self, set: &crate::simp::SimpSet) -> Result<&mut Self, TacticError> {
        const BUDGET: usize = 1000;
        let mut progress = false;
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        seen.insert(self.focused_goal()?.target.to_string());

        for _ in 0..BUDGET {
            let g = self.focused_goal()?;
            let hyp_view: Vec<(&ProofExpr, &DerivationTree)> =
                g.hyps.iter().map(|h| (&h.prop, &h.proof)).collect();

            // One term rewrite (rule step or ground arithmetic fold).
            let step = set
                .find_term_step(&g.target, &hyp_view)
                .or_else(|| crate::simp::find_ground_fold(&g.target));
            if let Some(step) = step {
                let new_target = replace_in_expr(&g.target, &step.from, &step.to);
                if new_target == g.target || !seen.insert(new_target.to_string()) {
                    break;
                }
                // Same shape as `rewrite`: the equality premise proves
                // `to = from` (the symmetric of the step equality).
                let eq_sym = DerivationTree::new(
                    ProofExpr::Identity(step.to.clone(), step.from.clone()),
                    InferenceRule::EqualitySymmetry,
                    vec![step.eq],
                );
                let sub = Goal { hyps: g.hyps.clone(), target: new_target };
                self.refine(
                    g.target.clone(),
                    InferenceRule::Rewrite { from: step.to, to: step.from },
                    vec![Child::Closed(eq_sym), Child::Sub(sub)],
                );
                progress = true;
                continue;
            }

            // A top-level propositional step: an iff rule matching the whole
            // goal reduces it to the rule's right-hand side via ModusPonens.
            if let Some(iff) = set.find_iff_step(&g.target, &hyp_view) {
                if !seen.insert(iff.rhs.to_string()) {
                    break;
                }
                let sub = Goal { hyps: g.hyps.clone(), target: iff.rhs };
                self.refine(
                    g.target.clone(),
                    InferenceRule::ModusPonens,
                    vec![Child::Closed(iff.imp), Child::Sub(sub)],
                );
                progress = true;
                continue;
            }

            break;
        }

        // Closing attempts on the normalized goal.
        let g = self.focused_goal()?;
        if let ProofExpr::Identity(l, r) = &g.target {
            if l == r {
                self.close(DerivationTree::leaf(
                    g.target.clone(),
                    InferenceRule::Reflexivity,
                ));
                return Ok(self);
            }
        }
        if self.assumption().is_ok() {
            return Ok(self);
        }
        if progress {
            Ok(self)
        } else {
            Err(TacticError::DoesNotApply(
                "simp: no rule rewrites the goal and it is not closable".to_string(),
            ))
        }
    }

    /// `decide`: close the focused goal by evaluation, if it is a closed
    /// decidable proposition that evaluates TRUE (ground arithmetic,
    /// comparisons, Bool equalities, and ∧/∨/→ over them). Mirrors Lean's
    /// `decide`. A false or open goal is declined.
    pub fn decide(&mut self) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        match crate::decide::decide_expr(&g.target) {
            Some(tree) => {
                self.close(tree);
                Ok(self)
            }
            None => Err(TacticError::DoesNotApply(
                "decide: not a closed decidable goal (or it is false)".to_string(),
            )),
        }
    }

    /// `omega`: linear INTEGER arithmetic. When the in-scope `≤`/`<` hypotheses
    /// are jointly unsatisfiable over ℤ — using discreteness (`a < b ⟹ a+1 ≤ b`),
    /// the fact rational solvers lack — refute them and discharge ANY goal by
    /// ex-falso. Proves `x < y ∧ y < x+1 ⊢ Q`, which `linarith` cannot (that
    /// system is rationally satisfiable). Mirrors the refutation half of Lean's
    /// `omega`. Declines when the hypotheses have an integer model.
    pub fn omega(&mut self) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        if !crate::omega_solve::has_arith_facts(
            &g.hyps.iter().map(|h| (h.prop.clone(), h.proof.clone())).collect::<Vec<_>>(),
        ) {
            return Err(TacticError::DoesNotApply(
                "omega: no arithmetic (≤ / <) hypotheses in scope".to_string(),
            ));
        }
        let known: Vec<(ProofExpr, DerivationTree)> =
            g.hyps.iter().map(|h| (h.prop.clone(), h.proof.clone())).collect();
        match crate::omega_solve::omega_close(&known) {
            Some(bot) => {
                // ⊥ ⊢ goal, by ex-falso.
                let tree = DerivationTree::new(
                    g.target.clone(),
                    InferenceRule::ExFalso,
                    vec![bot],
                );
                self.close(tree);
                Ok(self)
            }
            None => Err(TacticError::DoesNotApply(
                "omega: the integer hypotheses are satisfiable (no refutation)".to_string(),
            )),
        }
    }

    /// `crush`: the grind-style closer. E-matches the in-scope `∀`-equality
    /// lemmas at the goal's ground terms, then discharges the goal by certified
    /// congruence closure. Proves goals plain `auto` cannot — e.g.
    /// `∀x. f(x)=g(x), a=b, P(g(b)) ⊢ P(f(a))` — and every step is kernel-checked.
    pub fn crush(&mut self) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        let premises: Vec<ProofExpr> = g.hyps.iter().map(|h| h.prop.clone()).collect();
        match crate::crush::crush_prove(&premises, &g.target) {
            Some(tree) => {
                self.close(tree);
                Ok(self)
            }
            None => Err(TacticError::DoesNotApply(
                "crush: could not close the goal by e-matching + congruence".to_string(),
            )),
        }
    }

    /// `auto`: discharge the focused goal with the backward chainer (the existing
    /// automation, hosted as one tactic). Its hypotheses and the premises are the
    /// available knowledge base.
    pub fn auto(&mut self) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        let mut engine = BackwardChainer::new();
        for h in &g.hyps {
            engine.add_axiom(h.prop.clone());
        }
        match engine.prove(g.target.clone()) {
            Ok(tree) => {
                self.close(tree);
                Ok(self)
            }
            Err(_) => Err(TacticError::AutoFailed),
        }
    }

    fn assemble(&self, idx: usize) -> Result<DerivationTree, TacticError> {
        match &self.nodes[idx] {
            Node::Done(tree) => Ok(tree.clone()),
            Node::Filled { conclusion, rule, children } => {
                let mut kids = children
                    .iter()
                    .map(|&c| self.assemble(c))
                    .collect::<Result<Vec<_>, _>>()?;
                // Generalize the step case: the eigenconstant the search ran under
                // becomes the `Variable` the certifier's `Match` binds.
                if let InferenceRule::StructuralInduction { step_var, .. } = rule {
                    if kids.len() == 2 {
                        kids[1] = eigen_to_var_tree(&kids[1], step_var);
                    }
                }
                // Generic induction: each case ran under its own rigid eigenconstants
                // (one per constructor argument); remap each back to the bound
                // `Variable` the certifier's `match` arm binds and the IH cites.
                if let InferenceRule::InductionScheme { cases, .. } = rule {
                    for (kid, case) in kids.iter_mut().zip(cases.iter()) {
                        for arg in &case.args {
                            *kid = eigen_to_var_tree(kid, &arg.name);
                        }
                    }
                }
                Ok(DerivationTree::new(conclusion.clone(), rule.clone(), kids))
            }
            Node::Hole(_) => Err(TacticError::GoalsRemain(1)),
        }
    }

    /// Assemble the current proof tree, if every goal is closed (for inspection/debug).
    pub fn assembled(&self) -> Option<DerivationTree> {
        if self.open.is_empty() {
            self.assemble(self.root).ok()
        } else {
            None
        }
    }

    /// Run a composed [`Tactic`] against this state, for fluent chaining with the
    /// primitive methods.
    pub fn run(&mut self, t: &Tactic) -> Result<&mut Self, TacticError> {
        t(self)?;
        Ok(self)
    }

    /// Finish the proof: every goal must be closed. Assembles the recorded tactic
    /// steps into one derivation and runs it through the kernel trust door — the
    /// returned [`VerifiedProof`] is `verified` only if the kernel accepts the term.
    pub fn qed(&self) -> Result<VerifiedProof, TacticError> {
        if !self.open.is_empty() {
            return Err(TacticError::GoalsRemain(self.open.len()));
        }
        let tree = self.assemble(self.root)?;
        Ok(check_derivation(&self.premises, &self.goal, tree))
    }
}

/// A first-class tactic: a transformation of the proof state that either makes
/// progress (`Ok`) or does not apply (`Err`). Tactics are values, so they compose
/// with the [`combinators`].
pub type Tactic = Box<dyn Fn(&mut ProofState) -> Result<(), TacticError>>;

/// Tactic *values* and the combinators that compose them — the language layer over
/// the primitive [`ProofState`] methods. The backtracking combinators (`first`,
/// `try_`, `repeat`) speculate on a clone of the state and commit only on success,
/// so a partial failure never corrupts the proof.
pub mod combinators {
    use super::{ProofState, ProofTerm, Tactic, TacticError};

    /// `intro name` as a tactic value.
    pub fn intro(name: &str) -> Tactic {
        let name = name.to_string();
        Box::new(move |st: &mut ProofState| st.intro(&name).map(|_| ()))
    }
    /// `assumption` as a tactic value.
    pub fn assumption() -> Tactic {
        Box::new(|st: &mut ProofState| st.assumption().map(|_| ()))
    }
    /// `simp` as a tactic value, its rule set drawn from everything in scope
    /// (premises, intro'd hypotheses, cited lemmas) — the script-level default.
    pub fn simp() -> Tactic {
        Box::new(|st: &mut ProofState| {
            let set = st.scope_simp_set();
            st.simp(&set).map(|_| ())
        })
    }
    /// `decide` as a tactic value.
    pub fn decide() -> Tactic {
        Box::new(|st: &mut ProofState| st.decide().map(|_| ()))
    }
    /// `omega` as a tactic value.
    pub fn omega() -> Tactic {
        Box::new(|st: &mut ProofState| st.omega().map(|_| ()))
    }
    /// `crush` as a tactic value.
    pub fn crush() -> Tactic {
        Box::new(|st: &mut ProofState| st.crush().map(|_| ()))
    }
    /// `exact name` as a tactic value.
    pub fn exact(name: &str) -> Tactic {
        let name = name.to_string();
        Box::new(move |st: &mut ProofState| st.exact(&name).map(|_| ()))
    }
    /// `split` (∧I) as a tactic value.
    pub fn split() -> Tactic {
        Box::new(|st: &mut ProofState| st.split().map(|_| ()))
    }
    /// `left` (∨I) as a tactic value.
    pub fn left() -> Tactic {
        Box::new(|st: &mut ProofState| st.left().map(|_| ()))
    }
    /// `right` (∨I) as a tactic value.
    pub fn right() -> Tactic {
        Box::new(|st: &mut ProofState| st.right().map(|_| ()))
    }
    /// `exists witness` (∃I) as a tactic value.
    pub fn exists(witness: ProofTerm) -> Tactic {
        Box::new(move |st: &mut ProofState| st.exists(witness.clone()).map(|_| ()))
    }
    /// `cases name` (∧/∨/∃ elimination) as a tactic value.
    pub fn cases(name: &str) -> Tactic {
        let name = name.to_string();
        Box::new(move |st: &mut ProofState| st.cases(&name).map(|_| ()))
    }
    /// `rewrite name` (Leibniz substitution by an equality) as a tactic value.
    pub fn rewrite(name: &str) -> Tactic {
        let name = name.to_string();
        Box::new(move |st: &mut ProofState| st.rewrite(&name).map(|_| ()))
    }
    /// `induction` (structural induction over `Nat`) as a tactic value.
    pub fn induction() -> Tactic {
        Box::new(|st: &mut ProofState| st.induction().map(|_| ()))
    }
    /// `induction_over` (generic structural induction over `ind_type`) as a tactic value.
    pub fn induction_over(ind_type: &str, ctors: Vec<super::CtorSpec>) -> Tactic {
        let ind_type = ind_type.to_string();
        Box::new(move |st: &mut ProofState| {
            st.induction_over(&ind_type, ctors.clone()).map(|_| ())
        })
    }
    /// `auto` (the backward chainer) as a tactic value.
    pub fn auto() -> Tactic {
        Box::new(|st: &mut ProofState| st.auto().map(|_| ()))
    }

    /// `t1; t2; …`: run each tactic in turn; fail (without committing the rest) at
    /// the first that does not apply.
    pub fn seq(tactics: Vec<Tactic>) -> Tactic {
        Box::new(move |st: &mut ProofState| {
            for t in &tactics {
                t(st)?;
            }
            Ok(())
        })
    }

    /// `first [t1, t2, …]`: try each tactic on a speculative copy and commit the
    /// first that succeeds; fail only if none apply.
    pub fn first(tactics: Vec<Tactic>) -> Tactic {
        Box::new(move |st: &mut ProofState| {
            for t in &tactics {
                let mut trial = st.clone();
                if t(&mut trial).is_ok() {
                    *st = trial;
                    return Ok(());
                }
            }
            Err(TacticError::DoesNotApply("first: no alternative applied".to_string()))
        })
    }

    /// `try t`: run `t` if it applies, otherwise leave the state unchanged. Always
    /// succeeds.
    pub fn try_(t: Tactic) -> Tactic {
        Box::new(move |st: &mut ProofState| {
            let mut trial = st.clone();
            if t(&mut trial).is_ok() {
                *st = trial;
            }
            Ok(())
        })
    }

    /// `repeat t`: apply `t` as long as it keeps applying (committing each success),
    /// then stop. Always succeeds. Bounded to avoid a non-progressing tactic looping
    /// forever.
    pub fn repeat(t: Tactic) -> Tactic {
        Box::new(move |st: &mut ProofState| {
            for _ in 0..100_000 {
                let mut trial = st.clone();
                if t(&mut trial).is_ok() {
                    *st = trial;
                } else {
                    break;
                }
                if st.open.is_empty() {
                    break;
                }
            }
            Ok(())
        })
    }

    /// `t1 <;> t2`-style: apply `t` to EVERY goal currently open (not just the
    /// focused one). Each goal is focused in turn; the subgoals `t` produces for it
    /// become the new open set, in original goal order.
    pub fn all_goals(t: Tactic) -> Tactic {
        Box::new(move |st: &mut ProofState| {
            let current: Vec<usize> = st.open.iter().copied().collect();
            let mut new_open = std::collections::VecDeque::new();
            for goal_idx in current {
                st.open = std::collections::VecDeque::from([goal_idx]);
                t(st)?;
                new_open.extend(st.open.iter().copied());
            }
            st.open = new_open;
            Ok(())
        })
    }

    /// `t1 <;> t2`: run `t1` on the focused goal, then `t2` on every goal it produces.
    pub fn then_all(t1: Tactic, t2: Tactic) -> Tactic {
        let t2 = std::rc::Rc::new(t2);
        Box::new(move |st: &mut ProofState| {
            t1(st)?;
            let t2 = t2.clone();
            all_goals(Box::new(move |s| t2(s)))(st)
        })
    }
}
