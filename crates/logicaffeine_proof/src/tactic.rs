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

use logicaffeine_kernel::prelude::StandardLibrary;

use crate::engine::BackwardChainer;
use crate::unify::{
    apply_subst_to_expr, apply_subst_to_term, instantiate_expr, unify_exprs, Substitution,
};
use crate::verify::{check_derivation, VerifiedProof};
use crate::{DerivationTree, InductionArg, InductionCase, InferenceRule, ProofExpr, ProofTerm};

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

/// A term as a proof SCRIPT writes it — the argument of `exact`, `have`, `refine`, `use`.
///
/// One syntax serves two languages, because Lean's does: the same `⟨a, b⟩` is a conjunction
/// proof against `p ∧ q` and a witness-and-proof pair against `∃x. φ`, and the same bare
/// name is a hypothesis in a proof position and a variable in a value position. Which
/// language a term is read in is decided by the position, never by the term
/// ([`ProofState::exact_term`] checks it as a proof, [`ProofState::value_term`] as a value).
#[derive(Debug, Clone, PartialEq)]
pub enum PTerm {
    /// A bare name: a hypothesis, a bound variable, or a constant.
    Ident(String),
    /// Juxtaposition — `f a b`. Modus ponens over an implication, instantiation over a
    /// universal, a function application in a value position.
    App(Box<PTerm>, Vec<PTerm>),
    /// The anonymous constructor `(a, b)` / `⟨a, b⟩`.
    Tuple(Vec<PTerm>),
    /// A `?_` / `?name` hole — a subgoal under `refine`, and an error anywhere else.
    Hole,
    /// An infix arithmetic operator, by its `ProofTerm` function name (`add`/`sub`/`mul`).
    Op(&'static str, Box<PTerm>, Box<PTerm>),
}

/// Elaborate a script term as a VALUE, resolving each bare name against `bound` — the
/// variables the surrounding proof has in scope.
fn value_of(
    term: &PTerm,
    bound: &std::collections::BTreeSet<String>,
) -> Result<ProofTerm, TacticError> {
    match term {
        PTerm::Ident(name) => Ok(if bound.contains(name) {
            ProofTerm::Variable(name.clone())
        } else {
            ProofTerm::Constant(name.clone())
        }),
        PTerm::App(head, args) => {
            let PTerm::Ident(name) = head.as_ref() else {
                return Err(TacticError::DoesNotApply(
                    "only a named function can be applied in a term position".to_string(),
                ));
            };
            let args = args
                .iter()
                .map(|a| value_of(a, bound))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ProofTerm::Function(name.clone(), args))
        }
        PTerm::Op(op, l, r) => Ok(ProofTerm::Function(
            (*op).to_string(),
            vec![value_of(l, bound)?, value_of(r, bound)?],
        )),
        PTerm::Tuple(items) => Ok(ProofTerm::Group(
            items
                .iter()
                .map(|a| value_of(a, bound))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        PTerm::Hole => Err(TacticError::DoesNotApply(
            "a hole is not a value — there is nothing for it to stand for".to_string(),
        )),
    }
}

/// Instantiate a `∀`-quantified proposition until it IS the target, extending `proof` with
/// the `UniversalInstTerm` chain that justifies each step.
///
/// This is what lets `exact h` close `P n` from `h : ∀m. P m`, which is ordinary Lean and
/// the reason a `have` need not restate what it proves.
fn instantiate_to(
    prop: &ProofExpr,
    target: &ProofExpr,
    proof: DerivationTree,
) -> Option<DerivationTree> {
    let (vars, body) = peel_foralls(prop);
    if vars.is_empty() {
        return None;
    }
    let mut subst = Substitution::new();
    if !match_expr_with(&body, target, &vars, &mut subst) {
        return None;
    }
    let mut concl = prop.clone();
    let mut tree = proof;
    for v in &vars {
        // A binder the body never pins down could be anything; refusing is the only honest
        // answer, since picking a witness would prove a different theorem.
        let witness = subst.get(v)?.clone();
        concl = instantiate_forall(&concl, &witness)?;
        tree = DerivationTree::new(
            concl.clone(),
            InferenceRule::UniversalInstTerm(witness),
            vec![tree],
        );
    }
    (concl == *target).then_some(tree)
}

/// One-sided match of a proposition `pattern` against `target`, where exactly `vars` are
/// pattern variables.
fn match_expr_with(
    pattern: &ProofExpr,
    target: &ProofExpr,
    vars: &[String],
    subst: &mut Substitution,
) -> bool {
    let term =
        |p: &ProofTerm, t: &ProofTerm, s: &mut Substitution| match_term_with(p, t, vars, &[], s);
    match (pattern, target) {
        (
            ProofExpr::Predicate {
                name: n1,
                args: a1,
                world: w1,
            },
            ProofExpr::Predicate {
                name: n2,
                args: a2,
                world: w2,
            },
        ) => {
            n1 == n2
                && w1 == w2
                && a1.len() == a2.len()
                && a1.iter().zip(a2).all(|(p, t)| term(p, t, subst))
        }
        (ProofExpr::Identity(l1, r1), ProofExpr::Identity(l2, r2)) => {
            term(l1, l2, subst) && term(r1, r2, subst)
        }
        (ProofExpr::And(a, b), ProofExpr::And(c, d))
        | (ProofExpr::Or(a, b), ProofExpr::Or(c, d))
        | (ProofExpr::Implies(a, b), ProofExpr::Implies(c, d))
        | (ProofExpr::Iff(a, b), ProofExpr::Iff(c, d)) => {
            match_expr_with(a, c, vars, subst) && match_expr_with(b, d, vars, subst)
        }
        (ProofExpr::Not(a), ProofExpr::Not(b)) => match_expr_with(a, b, vars, subst),
        (
            ProofExpr::ForAll {
                variable: v1,
                body: b1,
            },
            ProofExpr::ForAll {
                variable: v2,
                body: b2,
            },
        )
        | (
            ProofExpr::Exists {
                variable: v1,
                body: b1,
            },
            ProofExpr::Exists {
                variable: v2,
                body: b2,
            },
        ) => v1 == v2 && match_expr_with(b1, b2, vars, subst),
        (a, b) => a == b,
    }
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

/// Replace every occurrence of the term `from` with `to` throughout an expression,
/// IGNORING binding structure.
///
/// This is de-Skolemization, not substitution: [`eigen_to_var_tree`] uses it to turn a
/// freshly-generated eigen-`Constant` back into the `Variable` the certifier's `Match` arm
/// binds, across a whole derivation whose quantifier structure must survive untouched.
/// Substituting INTO a scoped proposition is [`substitute_in_expr`], which is
/// capture-avoiding; nothing else may use this one.
fn replace_in_expr_raw(e: &ProofExpr, from: &ProofTerm, to: &ProofTerm) -> ProofExpr {
    let re = |x: &ProofExpr| Box::new(replace_in_expr_raw(x, from, to));
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
        ProofExpr::ForAll { variable, body } => ProofExpr::ForAll {
            variable: variable.clone(),
            body: re(body),
        },
        ProofExpr::Exists { variable, body } => ProofExpr::Exists {
            variable: variable.clone(),
            body: re(body),
        },
        other => other.clone(),
    }
}

/// The variable leaves of a term. `Constant` is a GLOBAL name and binds to nothing; only
/// `Variable`/`BoundVarRef` are what a `ForAll`/`Exists` binder captures.
fn term_vars(t: &ProofTerm, out: &mut std::collections::BTreeSet<String>) {
    match t {
        ProofTerm::Variable(n) | ProofTerm::BoundVarRef(n) => {
            out.insert(n.clone());
        }
        ProofTerm::Function(_, args) | ProofTerm::Group(args) => {
            for a in args {
                term_vars(a, out);
            }
        }
        ProofTerm::Constant(_) => {}
    }
}

fn vars_of_term(t: &ProofTerm) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    term_vars(t, &mut out);
    out
}

/// The FREE variables of an expression — binders remove their own variable.
fn free_vars(e: &ProofExpr, out: &mut std::collections::BTreeSet<String>) {
    match e {
        ProofExpr::Predicate { args, .. } => {
            for a in args {
                term_vars(a, out);
            }
        }
        ProofExpr::Identity(l, r) => {
            term_vars(l, out);
            term_vars(r, out);
        }
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            free_vars(l, out);
            free_vars(r, out);
        }
        ProofExpr::Not(p) => free_vars(p, out),
        ProofExpr::ForAll { variable, body } | ProofExpr::Exists { variable, body } => {
            let mut inner = std::collections::BTreeSet::new();
            free_vars(body, &mut inner);
            inner.remove(variable);
            out.extend(inner);
        }
        ProofExpr::Term(t) => term_vars(t, out),
        _ => {}
    }
}

/// Every variable name the expression MENTIONS, bound ones included — the set a fresh
/// binder name must avoid.
fn all_vars(e: &ProofExpr, out: &mut std::collections::BTreeSet<String>) {
    match e {
        ProofExpr::ForAll { variable, body } | ProofExpr::Exists { variable, body } => {
            out.insert(variable.clone());
            all_vars(body, out);
        }
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            all_vars(l, out);
            all_vars(r, out);
        }
        ProofExpr::Not(p) => all_vars(p, out),
        other => free_vars(other, out),
    }
}

/// The CONSTANT names an expression mentions — the global symbols, as distinct from the
/// variables [`all_vars`] collects.
fn constants_of(e: &ProofExpr, out: &mut std::collections::BTreeSet<String>) {
    fn term(t: &ProofTerm, out: &mut std::collections::BTreeSet<String>) {
        match t {
            ProofTerm::Constant(n) => {
                out.insert(n.clone());
            }
            ProofTerm::Function(n, args) => {
                out.insert(n.clone());
                for a in args {
                    term(a, out);
                }
            }
            ProofTerm::Group(args) => {
                for a in args {
                    term(a, out);
                }
            }
            _ => {}
        }
    }
    match e {
        ProofExpr::Predicate { args, .. } => {
            for a in args {
                term(a, out);
            }
        }
        ProofExpr::Identity(l, r) => {
            term(l, out);
            term(r, out);
        }
        ProofExpr::Term(t) => term(t, out),
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            constants_of(l, out);
            constants_of(r, out);
        }
        ProofExpr::Not(p) => constants_of(p, out),
        ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => constants_of(body, out),
        _ => {}
    }
}

/// A name close to `base` that no name in `taken` uses.
fn fresh_name(base: &str, taken: &std::collections::BTreeSet<String>) -> String {
    for i in 1.. {
        let candidate = format!("{base}_{i}");
        if !taken.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!("the integers are not exhausted")
}

/// α-rename the free occurrences of the variable `old` to `new` inside `e`, stopping at any
/// binder that re-binds `old`. `new` must be fresh for `e` (see [`fresh_name`]), which is
/// what makes this rename itself capture-free.
fn rename_free_var(e: &ProofExpr, old: &str, new: &str) -> ProofExpr {
    match e {
        ProofExpr::ForAll { variable, .. } | ProofExpr::Exists { variable, .. }
            if variable == old =>
        {
            e.clone()
        }
        ProofExpr::ForAll { variable, body } => ProofExpr::ForAll {
            variable: variable.clone(),
            body: Box::new(rename_free_var(body, old, new)),
        },
        ProofExpr::Exists { variable, body } => ProofExpr::Exists {
            variable: variable.clone(),
            body: Box::new(rename_free_var(body, old, new)),
        },
        other => replace_in_expr_shallow(other, old, new),
    }
}

/// One non-quantifier layer of [`rename_free_var`].
fn replace_in_expr_shallow(e: &ProofExpr, old: &str, new: &str) -> ProofExpr {
    let from = ProofTerm::Variable(old.to_string());
    let to = ProofTerm::Variable(new.to_string());
    let rt = |x: &ProofTerm| replace_in_term(x, &from, &to);
    let re = |x: &ProofExpr| Box::new(rename_free_var(x, old, new));
    match e {
        ProofExpr::Predicate { name, args, world } => ProofExpr::Predicate {
            name: name.clone(),
            args: args.iter().map(rt).collect(),
            world: world.clone(),
        },
        ProofExpr::Identity(l, r) => ProofExpr::Identity(rt(l), rt(r)),
        ProofExpr::And(l, r) => ProofExpr::And(re(l), re(r)),
        ProofExpr::Or(l, r) => ProofExpr::Or(re(l), re(r)),
        ProofExpr::Implies(l, r) => ProofExpr::Implies(re(l), re(r)),
        ProofExpr::Iff(l, r) => ProofExpr::Iff(re(l), re(r)),
        ProofExpr::Not(p) => ProofExpr::Not(re(p)),
        ProofExpr::Term(t) => ProofExpr::Term(rt(t)),
        other => other.clone(),
    }
}

/// CAPTURE-AVOIDING substitution: replace every FREE occurrence of the term `from` with
/// `to` throughout a scoped expression. This is the Leibniz substitution `rewrite` and
/// `simp` perform on a goal, and it is the only substitution either may use.
///
/// Two hazards, both handled here rather than by a caller-side refusal:
///
/// * **Shadowing.** A binder that re-binds one of `from`'s free variables introduces a
///   DIFFERENT variable of the same spelling, so the pattern has no occurrence beneath it;
///   the quantifier is left alone and the free occurrences beside it are still rewritten. A
///   patch that refused the whole rewrite instead would silently under-rewrite.
/// * **Capture.** A binder that binds one of `to`'s free variables would capture the
///   replacement, turning a claim about one entity into a claim about the quantified one.
///   The binder is α-renamed away first, so the replacement's variable stays free and stays
///   distinct.
pub(crate) fn substitute_in_expr(e: &ProofExpr, from: &ProofTerm, to: &ProofTerm) -> ProofExpr {
    let re = |x: &ProofExpr| Box::new(substitute_in_expr(x, from, to));
    let rt = |x: &ProofTerm| replace_in_term(x, from, to);
    match e {
        ProofExpr::Predicate { name, args, world } => ProofExpr::Predicate {
            name: name.clone(),
            args: args.iter().map(rt).collect(),
            world: world.clone(),
        },
        ProofExpr::Identity(l, r) => ProofExpr::Identity(rt(l), rt(r)),
        ProofExpr::And(l, r) => ProofExpr::And(re(l), re(r)),
        ProofExpr::Or(l, r) => ProofExpr::Or(re(l), re(r)),
        ProofExpr::Implies(l, r) => ProofExpr::Implies(re(l), re(r)),
        ProofExpr::Iff(l, r) => ProofExpr::Iff(re(l), re(r)),
        ProofExpr::Not(p) => ProofExpr::Not(re(p)),
        ProofExpr::Term(t) => ProofExpr::Term(rt(t)),
        ProofExpr::ForAll { .. } | ProofExpr::Exists { .. } => substitute_under_binder(e, from, to),
        other => other.clone(),
    }
}

fn substitute_under_binder(e: &ProofExpr, from: &ProofTerm, to: &ProofTerm) -> ProofExpr {
    let (variable, body, is_forall) = match e {
        ProofExpr::ForAll { variable, body } => (variable.clone(), (**body).clone(), true),
        ProofExpr::Exists { variable, body } => (variable.clone(), (**body).clone(), false),
        other => return other.clone(),
    };
    // Shadowing: beneath this binder `variable` is a different variable, so the pattern
    // simply does not occur there.
    if vars_of_term(from).contains(&variable) {
        return e.clone();
    }
    // Capture: α-rename the binder out of the replacement's way first.
    let (variable, body) = if vars_of_term(to).contains(&variable) {
        let mut taken = vars_of_term(to);
        taken.extend(vars_of_term(from));
        all_vars(&body, &mut taken);
        let fresh = fresh_name(&variable, &taken);
        let renamed = rename_free_var(&body, &variable, &fresh);
        (fresh, renamed)
    } else {
        (variable, body)
    };
    let body = Box::new(substitute_in_expr(&body, from, to));
    if is_forall {
        ProofExpr::ForAll { variable, body }
    } else {
        ProofExpr::Exists { variable, body }
    }
}

/// POSITIONAL substitution: replace the `target`-th FREE occurrence of `from` with `to`,
/// leaving every other occurrence alone, and report how many free occurrences there were.
///
/// [`substitute_in_expr`] rewrites them all, which is why `rewrite` could only ever turn
/// `P(a, a)` into `P(b, b)` — and `P(b, a)` is a different goal, frequently the only provable
/// one. That is the whole reason Lean has `conv` and occurrence arguments.
///
/// The binder discipline is EXACTLY [`substitute_under_binder`]'s, and that is load-bearing
/// in both directions: an occurrence the goal BINDS is not an occurrence of the pattern (so
/// it is not counted either — the indices a caller passes must mean the same thing as the
/// ones this counts), and a binder that would capture the replacement's free variable is
/// α-renamed away first. Counting only free occurrences keeps the index stable under the
/// α-renaming the capture case performs.
struct OccurrenceSubst<'a> {
    from: &'a ProofTerm,
    to: &'a ProofTerm,
    target: usize,
    seen: usize,
}

impl OccurrenceSubst<'_> {
    fn term(&mut self, t: &ProofTerm) -> ProofTerm {
        if t == self.from {
            let here = self.seen;
            self.seen += 1;
            return if here == self.target {
                self.to.clone()
            } else {
                t.clone()
            };
        }
        match t {
            ProofTerm::Function(n, args) => {
                let mut out = Vec::with_capacity(args.len());
                for a in args {
                    out.push(self.term(a));
                }
                ProofTerm::Function(n.clone(), out)
            }
            ProofTerm::Group(args) => {
                let mut out = Vec::with_capacity(args.len());
                for a in args {
                    out.push(self.term(a));
                }
                ProofTerm::Group(out)
            }
            other => other.clone(),
        }
    }

    fn expr(&mut self, e: &ProofExpr) -> ProofExpr {
        match e {
            ProofExpr::Predicate { name, args, world } => {
                let mut out = Vec::with_capacity(args.len());
                for a in args {
                    out.push(self.term(a));
                }
                ProofExpr::Predicate {
                    name: name.clone(),
                    args: out,
                    world: world.clone(),
                }
            }
            ProofExpr::Identity(l, r) => {
                let l2 = self.term(l);
                let r2 = self.term(r);
                ProofExpr::Identity(l2, r2)
            }
            ProofExpr::And(l, r) => {
                let l2 = self.expr(l);
                let r2 = self.expr(r);
                ProofExpr::And(Box::new(l2), Box::new(r2))
            }
            ProofExpr::Or(l, r) => {
                let l2 = self.expr(l);
                let r2 = self.expr(r);
                ProofExpr::Or(Box::new(l2), Box::new(r2))
            }
            ProofExpr::Implies(l, r) => {
                let l2 = self.expr(l);
                let r2 = self.expr(r);
                ProofExpr::Implies(Box::new(l2), Box::new(r2))
            }
            ProofExpr::Iff(l, r) => {
                let l2 = self.expr(l);
                let r2 = self.expr(r);
                ProofExpr::Iff(Box::new(l2), Box::new(r2))
            }
            ProofExpr::Not(p) => ProofExpr::Not(Box::new(self.expr(p))),
            ProofExpr::Term(t) => ProofExpr::Term(self.term(t)),
            ProofExpr::ForAll { .. } | ProofExpr::Exists { .. } => self.under_binder(e),
            other => other.clone(),
        }
    }

    fn under_binder(&mut self, e: &ProofExpr) -> ProofExpr {
        let (variable, body, is_forall) = match e {
            ProofExpr::ForAll { variable, body } => (variable.clone(), (**body).clone(), true),
            ProofExpr::Exists { variable, body } => (variable.clone(), (**body).clone(), false),
            other => return other.clone(),
        };
        // Shadowing: beneath this binder the pattern's variable is a DIFFERENT variable, so
        // the pattern does not occur here and nothing beneath is counted.
        if vars_of_term(self.from).contains(&variable) {
            return e.clone();
        }
        // Capture: α-rename the binder out of the replacement's way first.
        let (variable, body) = if vars_of_term(self.to).contains(&variable) {
            let mut taken = vars_of_term(self.to);
            taken.extend(vars_of_term(self.from));
            all_vars(&body, &mut taken);
            let fresh = fresh_name(&variable, &taken);
            let renamed = rename_free_var(&body, &variable, &fresh);
            (fresh, renamed)
        } else {
            (variable, body)
        };
        let body = Box::new(self.expr(&body));
        if is_forall {
            ProofExpr::ForAll { variable, body }
        } else {
            ProofExpr::Exists { variable, body }
        }
    }
}

/// `(rewritten, free occurrences seen)` for the `occurrence`-th free match of `from`.
/// `occurrence >= seen` means there is no such occurrence and nothing was replaced.
/// Pass `usize::MAX` to COUNT without replacing anything.
pub(crate) fn substitute_occurrence_in_expr(
    e: &ProofExpr,
    from: &ProofTerm,
    to: &ProofTerm,
    occurrence: usize,
) -> (ProofExpr, usize) {
    let mut subst = OccurrenceSubst {
        from,
        to,
        target: occurrence,
        seen: 0,
    };
    let out = subst.expr(e);
    (out, subst.seen)
}

/// Strip a `∀` prefix, returning the binder names in order and the body beneath them.
/// The binders are exactly the pattern variables of a quantified rule.
fn peel_foralls(e: &ProofExpr) -> (Vec<String>, ProofExpr) {
    let mut vars = Vec::new();
    let mut cur = e.clone();
    while let ProofExpr::ForAll { variable, body } = cur {
        vars.push(variable);
        cur = *body;
    }
    (vars, cur)
}

/// Move the `∀` binder named `variable` to the FRONT of the goal's quantifier prefix,
/// keeping the other binders in their relative order. `∀m n. Q m n` at `n` becomes
/// `∀n m. Q m n` — the same theorem (independent universals commute), stated so that the
/// variable to induct on is the one a structural-induction motive can abstract.
fn reorder_forall_prefix(e: &ProofExpr, variable: &str) -> Option<ProofExpr> {
    let (prefix, body) = peel_foralls(e);
    if !prefix.iter().any(|v| v == variable) {
        return None;
    }
    let mut order: Vec<&String> = prefix.iter().filter(|v| *v != variable).collect();
    let moved = variable.to_string();
    order.insert(0, &moved);
    let mut out = body;
    for v in order.into_iter().rev() {
        out = ProofExpr::ForAll {
            variable: v.clone(),
            body: Box::new(out),
        };
    }
    Some(out)
}

/// Recover the inductive type and its constructor set for a goal `∀x. body` from the
/// hypotheses in scope — the induction PRINCIPLE stated as premises, which is how a
/// structural-induction goal reaches the tactic layer in an untyped `ProofExpr`.
///
/// A hypothesis `body[x := C]` names the nullary constructor `C`; a hypothesis
/// `∀a₁…aₙ. body[x := aᵢ] → … → body[x := C(a₁…aₙ)]` names `C`, with a recursive position
/// at every `aᵢ` that carries an induction hypothesis. Constructors come back in the order
/// their hypotheses appear, which is the order the subgoals are then presented in. `None`
/// when the scope names no constructor at all, or names constructors of two different types.
fn infer_ctors(variable: &str, body: &ProofExpr, hyps: &[Hyp]) -> Option<(String, Vec<CtorSpec>)> {
    let at = |t: ProofTerm| -> ProofExpr { instantiate_expr(body, variable, &t) };
    let mut ind_type: Option<String> = None;
    let mut ctors: Vec<CtorSpec> = Vec::new();
    for h in hyps {
        let Some((ctor, recursive)) = constructor_case(&h.prop, &at) else {
            continue;
        };
        let owner = inductive_of_constructor(&ctor)?;
        match &ind_type {
            Some(t) if t != &owner => return None,
            _ => ind_type = Some(owner),
        }
        if ctors.iter().any(|c| c.constructor == ctor) {
            continue;
        }
        ctors.push(CtorSpec {
            constructor: ctor,
            recursive,
        });
    }
    match ind_type {
        Some(t) if !ctors.is_empty() => Some((t, ctors)),
        _ => None,
    }
}

/// Read one hypothesis as one constructor case of an induction principle: its constructor
/// and, per argument position, whether that argument carries an induction hypothesis.
///
/// How many of the hypothesis's leading `∀`s are the CONSTRUCTOR's arguments is not known in
/// advance — the motive may itself be quantified (`∀m. Q(m, Zero)` is the whole base case,
/// where `∀h t. P(t) → P(ECons h t)` binds two constructor arguments). So each prefix length
/// is tried in turn and the shortest that yields a confirmed constructor application wins.
fn constructor_case(
    prop: &ProofExpr,
    at: &dyn Fn(ProofTerm) -> ProofExpr,
) -> Option<(String, Vec<bool>)> {
    let (prefix, base) = peel_foralls(prop);
    for depth in 0..=prefix.len() {
        // Re-wrap the binders this reading does NOT treat as constructor arguments.
        let mut rest = base.clone();
        for v in prefix[depth..].iter().rev() {
            rest = ProofExpr::ForAll {
                variable: v.clone(),
                body: Box::new(rest),
            };
        }
        let mut antecedents = Vec::new();
        while let ProofExpr::Implies(a, b) = rest {
            antecedents.push(*a);
            rest = *b;
        }
        let Some((ctor, ctor_args)) = constructor_application(&rest, at) else {
            continue;
        };
        // Every argument must be one of the binders this reading claimed, or the conclusion
        // is some other fact that merely mentions the same symbol.
        let args = &prefix[..depth];
        if !ctor_args.iter().all(
            |a| matches!(a, ProofTerm::Variable(n) | ProofTerm::Constant(n) if args.contains(n)),
        ) {
            continue;
        }
        let recursive = ctor_args
            .iter()
            .map(|arg| antecedents.iter().any(|ant| *ant == at(arg.clone())))
            .collect();
        return Some((ctor, recursive));
    }
    None
}

/// If `conclusion` is `body[x := C]` or `body[x := C(ā)]` for some symbol `C`, return `C`
/// and its arguments. `at` re-substitutes so the guess is CONFIRMED rather than pattern-
/// matched out of the shape.
fn constructor_application(
    conclusion: &ProofExpr,
    at: &dyn Fn(ProofTerm) -> ProofExpr,
) -> Option<(String, Vec<ProofTerm>)> {
    let mut candidates = Vec::new();
    collect_head_terms(conclusion, &mut candidates);
    candidates.into_iter().find_map(|candidate| {
        let (name, args) = match &candidate {
            ProofTerm::Constant(n) => (n.clone(), Vec::new()),
            ProofTerm::Function(n, a) => (n.clone(), a.clone()),
            _ => return None,
        };
        (at(candidate) == *conclusion).then_some((name, args))
    })
}

/// Every term the expression carries, outermost first — the candidates for "the term the
/// induction variable was replaced by".
fn collect_head_terms(e: &ProofExpr, out: &mut Vec<ProofTerm>) {
    match e {
        ProofExpr::Predicate { args, .. } => out.extend(args.iter().cloned()),
        ProofExpr::Identity(l, r) => {
            out.push(l.clone());
            out.push(r.clone());
        }
        ProofExpr::Term(t) => out.push(t.clone()),
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            collect_head_terms(l, out);
            collect_head_terms(r, out);
        }
        ProofExpr::Not(p) => collect_head_terms(p, out),
        ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
            collect_head_terms(body, out)
        }
        _ => {}
    }
}

/// Instantiate the outermost `∀` of `e` at `witness`, or `None` if `e` is not quantified.
fn instantiate_forall(e: &ProofExpr, witness: &ProofTerm) -> Option<ProofExpr> {
    match e {
        ProofExpr::ForAll { variable, body } => {
            // Instantiating a binder AT ITSELF is the identity. It must be taken as one:
            // `apply_subst_to_term` chases `{z ↦ z}` forever, which is why `intro` skips the
            // rename in exactly this case.
            if matches!(
                witness,
                ProofTerm::Variable(n) | ProofTerm::BoundVarRef(n) if n == variable
            ) {
                return Some((**body).clone());
            }
            Some(instantiate_expr(body, variable, witness))
        }
        _ => None,
    }
}

/// One-sided match of `pattern` against `target`, where exactly the names in `vars` are
/// pattern variables. Every other `Variable` is rigid and matches only itself. A binding
/// that mentions a name in `rigid` (a variable the GOAL binds at the match site) is
/// rejected: the instantiation would escape its scope.
fn match_term_with(
    pattern: &ProofTerm,
    target: &ProofTerm,
    vars: &[String],
    rigid: &[String],
    subst: &mut Substitution,
) -> bool {
    match pattern {
        ProofTerm::Variable(n) if vars.iter().any(|v| v == n) => {
            if vars_of_term(target)
                .iter()
                .any(|t| rigid.iter().any(|r| r == t))
            {
                return false;
            }
            match subst.get(n) {
                Some(bound) => bound == target,
                None => {
                    subst.insert(n.clone(), target.clone());
                    true
                }
            }
        }
        ProofTerm::Function(name, args) => match target {
            ProofTerm::Function(n2, a2) if name == n2 && args.len() == a2.len() => args
                .iter()
                .zip(a2.iter())
                .all(|(p, t)| match_term_with(p, t, vars, rigid, subst)),
            _ => false,
        },
        ProofTerm::Group(args) => match target {
            ProofTerm::Group(a2) if args.len() == a2.len() => args
                .iter()
                .zip(a2.iter())
                .all(|(p, t)| match_term_with(p, t, vars, rigid, subst)),
            _ => false,
        },
        other => other == target,
    }
}

/// The first subterm of `t` (outermost, then left to right) that `pattern` matches.
fn first_match_in_term(
    t: &ProofTerm,
    pattern: &ProofTerm,
    vars: &[String],
    rigid: &[String],
) -> Option<Substitution> {
    let mut subst = Substitution::new();
    if match_term_with(pattern, t, vars, rigid, &mut subst) {
        return Some(subst);
    }
    match t {
        ProofTerm::Function(_, args) | ProofTerm::Group(args) => args
            .iter()
            .find_map(|a| first_match_in_term(a, pattern, vars, rigid)),
        _ => None,
    }
}

/// The first subterm of the EXPRESSION `e` that `pattern` matches, in the reading order a
/// proof has — outermost before inner, left before right. Beneath a `ForAll`/`Exists` the
/// binder joins `rigid`, so a match may not bind a pattern variable to something the goal
/// binds (that instantiation would escape the quantifier).
fn first_match_in_expr(
    e: &ProofExpr,
    pattern: &ProofTerm,
    vars: &[String],
) -> Option<Substitution> {
    fn go(
        e: &ProofExpr,
        pattern: &ProofTerm,
        vars: &[String],
        rigid: &mut Vec<String>,
    ) -> Option<Substitution> {
        let term = |t: &ProofTerm| first_match_in_term(t, pattern, vars, rigid);
        match e {
            ProofExpr::Predicate { args, .. } => args.iter().find_map(term),
            ProofExpr::Identity(l, r) => term(l).or_else(|| term(r)),
            ProofExpr::Term(t) => term(t),
            ProofExpr::And(l, r)
            | ProofExpr::Or(l, r)
            | ProofExpr::Implies(l, r)
            | ProofExpr::Iff(l, r) => {
                go(l, pattern, vars, rigid).or_else(|| go(r, pattern, vars, rigid))
            }
            ProofExpr::Not(p) => go(p, pattern, vars, rigid),
            ProofExpr::ForAll { variable, body } | ProofExpr::Exists { variable, body } => {
                // A binder that re-binds one of the pattern's OWN rigid variables shadows it:
                // the pattern does not occur beneath it at all.
                if vars_of_term(pattern).contains(variable) && !vars.iter().any(|v| v == variable) {
                    return None;
                }
                rigid.push(variable.clone());
                let found = go(body, pattern, vars, rigid);
                rigid.pop();
                found
            }
            _ => None,
        }
    }
    go(e, pattern, vars, &mut Vec::new())
}

/// Rewrite the eigen-`Constant` `name` back to `Variable(name)` throughout a
/// derivation — the generalization step for `induction`'s step case, turning the
/// rigid search variable into the one the certifier's `Match` arm binds.
fn eigen_to_var_tree(tree: &DerivationTree, name: &str) -> DerivationTree {
    let from = ProofTerm::Constant(name.to_string());
    let to = ProofTerm::Variable(name.to_string());
    // Every payload this rule carries, through the ONE exhaustive mapper — a rule whose
    // witness or motive still named the eigenconstant while the conclusion around it was
    // renamed is a tree the certifier computes one proposition from and checks another.
    let rule = tree.rule.map_payload(
        &|n: &str| n.to_string(),
        &|t: &ProofTerm| replace_in_term(t, &from, &to),
        &|e: &ProofExpr| replace_in_expr_raw(e, &from, &to),
    );
    DerivationTree {
        conclusion: replace_in_expr_raw(&tree.conclusion, &from, &to),
        rule,
        premises: tree
            .premises
            .iter()
            .map(|p| eigen_to_var_tree(p, name))
            .collect(),
        depth: tree.depth,
        substitution: tree.substitution.clone(),
    }
}

/// A hypothesis directly in scope (premise / `intro` / `cases`-branch): its proof
/// is a `PremiseMatch` leaf the certifier resolves against the bound or registered
/// hypothesis of the same proposition.
/// The ∃-introduction rule for a witness: the NAME-carrying
/// [`InferenceRule::ExistentialIntro`] for a bare `Constant`, the TERM-carrying
/// [`InferenceRule::ExistentialIntroTerm`] for everything else.
///
/// The split is not a fast path, it is a FAITHFULNESS one. `ExistentialIntro`'s witness is a
/// `String` that the certifier resolves as a local `Var` or a global `Global` by asking
/// whether the surrounding traversal has bound that name — which is right for a constant and
/// wrong for a `Variable`, whose lowering is unconditionally `Term::Var`: a proof-level
/// variable free in the THEOREM (not bound by any `intro`) would resolve to a `Global` while
/// the goal type mentions the `Var`, and the two would not match. Carrying the term lowers it
/// once, the same way everywhere.
fn witness_rule(witness: &ProofTerm, witness_type: String) -> InferenceRule {
    match witness {
        ProofTerm::Constant(name) => InferenceRule::ExistentialIntro {
            witness: name.clone(),
            witness_type,
        },
        other => InferenceRule::ExistentialIntroTerm {
            witness: other.clone(),
            witness_type,
        },
    }
}

fn direct_hyp(name: String, prop: ProofExpr) -> Hyp {
    let proof = DerivationTree::leaf(prop.clone(), InferenceRule::PremiseMatch);
    Hyp { name, prop, proof }
}

/// `¬p` and `p → False` are the SAME proposition — `certifier::proof_expr_to_type` lowers both to
/// `Π(_:p). False`. The tactic layer compared `ProofExpr`s structurally, so it alone told them
/// apart: `intro` had no image on a negation goal, and `assumption` could not see a `¬p` hypothesis
/// discharging a `p → False` target or vice versa. Normalising at every comparison point keeps the
/// two layers agreeing about what a proposition IS, rather than patching each tactic separately.
fn unfold_not(e: &ProofExpr) -> ProofExpr {
    match e {
        ProofExpr::Not(p) => ProofExpr::Implies(
            Box::new(unfold_not(p)),
            Box::new(ProofExpr::Atom(FALSUM.to_string())),
        ),
        // The falsum has three spellings in the wild and `certifier.rs` accepts all three, so a
        // comparison that does not canonicalise them tells apart two propositions the kernel
        // cannot. Recurse into implications so `¬¬p` and `(p → ⊥) → False` agree too.
        ProofExpr::Atom(n) if is_falsum(n) => ProofExpr::Atom(FALSUM.to_string()),
        ProofExpr::Implies(a, b) => {
            ProofExpr::Implies(Box::new(unfold_not(a)), Box::new(unfold_not(b)))
        }
        other => other.clone(),
    }
}

/// The canonical spelling of falsum inside [`unfold_not`].
const FALSUM: &str = "⊥";

/// The three spellings `certifier.rs` treats as falsum.
fn is_falsum(name: &str) -> bool {
    name == "⊥" || name == "False" || name == "false"
}

/// Is this proposition the absurdity, under any of its spellings?
fn is_falsum_expr(e: &ProofExpr) -> bool {
    matches!(e, ProofExpr::Atom(n) if is_falsum(n))
}

/// Do `l` and `r` denote the same polynomial over the commutative-ring vocabulary?
///
/// Asks the arithmetic oracle that `InferenceRule::ArithDecision` certifies through, on a
/// context carrying the ring axioms — so the tactic's YES and the kernel's YES are the same
/// question, put to the same decision procedure, rather than two implementations that can
/// drift apart.
/// Is `l = r` a formal identity of the commutative ring — the question `ring` and
/// `ArithDecision` both answer? Exposed for [`crate::omega_solve`], whose goal-directed half
/// discharges an `Int` EQUALITY goal (`n + 0 = n`, an `omega` goal in Lean) through the same
/// oracle rather than a second copy of the decision.
pub(crate) fn int_identity_holds(l: &ProofTerm, r: &ProofTerm) -> bool {
    ring_identity_holds(l, r)
}

/// Is this term BOOL-valued — a comparison in the canonical `le(a, b) = true` encoding, or a
/// Bool literal — rather than an `Int` term?
///
/// [`crate::arith::prove_int_eq`] answers about VALUES, and `le(2, 3)` computes to `true`, so
/// it cheerfully agrees that `le(2, 3) = true` is an identity. The PROPOSITION is true; the
/// certificate `ring` would emit is not, because [`InferenceRule::ArithDecision`] lowers to an
/// `Eq Int` fact and the kernel then reports "expected Int, found Bool" about a correct
/// statement — a tactic reporting success and the trust door refusing it. Comparisons belong
/// to `decide` (ground) and `omega` (symbolic), both of which certify them properly.
fn is_bool_valued(t: &ProofTerm) -> bool {
    match t {
        ProofTerm::Constant(c) => c == "true" || c == "false",
        ProofTerm::Function(name, _) => {
            matches!(
                name.as_str(),
                "le" | "lt" | "ge" | "gt" | "eq" | "and" | "or" | "not"
            )
        }
        _ => false,
    }
}

fn ring_identity_holds(l: &ProofTerm, r: &ProofTerm) -> bool {
    let (Ok(kl), Ok(kr)) = (
        crate::certifier::proof_term_to_kernel_term(l),
        crate::certifier::proof_term_to_kernel_term(r),
    ) else {
        return false;
    };
    crate::arith::prove_int_eq(StandardLibrary::context(), &kl, &kr).is_some()
}

/// The first NULLARY constructor of a prelude inductive — its closed base point, read off the
/// registered constructor signatures rather than a table (`Nat ↦ Zero`, `EList ↦ ENil`).
///
/// Used by `ProofState::induction_generalizing` to land back on the original goal after a
/// VACUOUS generalization: when the induction variable occurs nowhere, `∀v:T. G ⊢ G` needs an
/// inhabitant of `T` and nothing else. An inductive with no nullary constructor has no such
/// closed point at this layer, and the caller says so instead of guessing one.
fn nullary_constructor(ind_type: &str) -> Option<String> {
    use logicaffeine_kernel::Term;
    StandardLibrary::context()
        .get_constructors(ind_type)
        .into_iter()
        .find(|(_, ty)| !matches!(ty, Term::Pi { .. }))
        .map(|(name, _)| name.to_string())
}

/// The inductive type a prelude CONSTRUCTOR belongs to, read off its registered kernel
/// signature (`Π…. Ind`) rather than a table this file would have to keep in step.
fn inductive_of_constructor(ctor: &str) -> Option<String> {
    use logicaffeine_kernel::Term;
    let mut ty = StandardLibrary::context().get_global(ctor)?.clone();
    while let Term::Pi { body_type, .. } = ty {
        ty = *body_type;
    }
    match ty {
        Term::Global(name) => Some(name),
        _ => None,
    }
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
        let root_goal = Goal {
            hyps,
            target: goal.clone(),
        };
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

    /// A [`crate::simp::SimpSet`] combining the in-scope hypotheses with every `@[simp]`
    /// lemma REGISTERED in a kernel context — the script-level `simp` of a development whose
    /// attribute set is available.
    ///
    /// The scope set alone is a per-goal accident: a lemma is a rule because it happens to be
    /// a hypothesis here. `@[simp]` is the opposite — a declaration, made once, that says this
    /// equation is how the simplifier should normalise from now on. Both feed the same
    /// rewriter, which is the point: one notion of a rule, two ways to reach it.
    pub fn context_simp_set(&self, ctx: &logicaffeine_kernel::Context) -> crate::simp::SimpSet {
        let mut set = self.scope_simp_set();
        set.register_context_lemmas(ctx);
        set
    }

    /// A [`crate::simp::SimpSet`] built from EXACTLY the named hypotheses — the rule set of
    /// Lean's `simp only [h₁, h₂]`.
    ///
    /// Selection is the whole point of `only`: the default set is every in-scope hypothesis
    /// that orients, so two `simp only` calls meant to stop at different normal forms
    /// collapse into one, and the collapse runs in the dangerous direction (MORE rules than
    /// the source asked for). An unknown name, or one that does not orient as a rewrite
    /// rule, is an error rather than a silently empty set.
    pub fn named_simp_set(&self, names: &[String]) -> Result<crate::simp::SimpSet, TacticError> {
        let g = self.focus().ok_or(TacticError::NoOpenGoals)?;
        let mut set = crate::simp::SimpSet::new();
        for name in names {
            let h = g
                .hyps
                .iter()
                .find(|h| &h.name == name)
                .ok_or_else(|| TacticError::NoSuchHypothesis(name.clone()))?;
            if !set.register_lemma(&h.name, &h.prop) {
                return Err(TacticError::DoesNotApply(format!(
                    "simp only [{name}]: `{name}` does not orient as a rewrite rule"
                )));
            }
        }
        Ok(set)
    }

    fn focused_goal(&self) -> Result<Goal, TacticError> {
        self.focus().cloned().ok_or(TacticError::NoOpenGoals)
    }

    /// Every NAME a goal is already committed to — hypothesis names, the variables the
    /// target and its scope mention (bound ones included), and the constants they name.
    /// A fresh eigen-constant must avoid all of them.
    fn scope_names(&self, g: &Goal) -> std::collections::BTreeSet<String> {
        let mut taken = std::collections::BTreeSet::new();
        let absorb = |e: &ProofExpr, taken: &mut std::collections::BTreeSet<String>| {
            all_vars(e, taken);
            constants_of(e, taken);
        };
        absorb(&g.target, &mut taken);
        for h in &g.hyps {
            taken.insert(h.name.clone());
            absorb(&h.prop, &mut taken);
        }
        taken
    }

    /// The names free in the focused goal that stand for TERM VARIABLES rather than global
    /// constants — what an `intro`-bound `n` is, and what a witness spelled `n` must BE.
    fn scope_free_vars(&self) -> std::collections::BTreeSet<String> {
        let mut vars = std::collections::BTreeSet::new();
        if let Some(g) = self.focus() {
            free_vars(&g.target, &mut vars);
            for h in &g.hyps {
                free_vars(&h.prop, &mut vars);
            }
        }
        vars
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
        self.nodes[idx] = Node::Filled {
            conclusion,
            rule,
            children: child_indices,
        };
    }

    /// Close the focused goal with an already-complete derivation.
    ///
    /// Every closer commits through here — `auto`'s engine tree, `crush`, the decision
    /// procedures, a hypothesis's proof — so this is where "prove it or decline it" is held:
    /// a derivation holding a node [`certifier::certify`](crate::certifier::certify) cannot
    /// build (an SMT oracle's verdict, a dead modal or temporal rule) is refused BEFORE the
    /// goal is consumed. The tactic declines with the offending rule and the reason, and the
    /// proof state is left exactly as it was, instead of reporting a success that `qed` would
    /// later refuse.
    fn close(&mut self, tree: DerivationTree) -> Result<(), TacticError> {
        if let Some((rule, why)) = tree.uncertifiable_rules().into_iter().next() {
            return Err(TacticError::DoesNotApply(format!(
                "the derivation for {} contains {rule}, which cannot be certified: {why}",
                tree.conclusion
            )));
        }
        let idx = self.open.pop_front().expect("a focused goal");
        self.nodes[idx] = Node::Done(tree);
        Ok(())
    }

    // === Tactics ===

    /// `intro name`: for a goal `P → Q`, assume `P` as hypothesis `name` and reduce
    /// to `Q`; for `∀x. φ(x)`, introduce the bound variable under `name` and reduce
    /// to `φ(name)`. Mirrors Lean's `intro`.
    pub fn intro(&mut self, name: &str) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        match &unfold_not(&g.target) {
            ProofExpr::Implies(p, q) => {
                let mut hyps = g.hyps.clone();
                hyps.push(direct_hyp(name.to_string(), (**p).clone()));
                let sub = Goal {
                    hyps,
                    target: (**q).clone(),
                };
                self.refine(
                    g.target.clone(),
                    InferenceRule::ImpliesIntro,
                    vec![Child::Sub(sub)],
                );
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
                    let renamed =
                        instantiate_expr(body, variable, &ProofTerm::Variable(name.to_string()));
                    let conclusion = ProofExpr::ForAll {
                        variable: name.to_string(),
                        body: Box::new(renamed.clone()),
                    };
                    (conclusion, renamed)
                };
                let sub = Goal {
                    hyps: g.hyps.clone(),
                    target: renamed,
                };
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
        let want = unfold_not(&g.target);
        if let Some(h) = g
            .hyps
            .iter()
            .find(|h| h.prop == g.target || unfold_not(&h.prop) == want)
        {
            let proof = h.proof.clone();
            self.close(proof)?;
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
            Some(h) if h.prop == g.target || unfold_not(&h.prop) == unfold_not(&g.target) => {
                let proof = h.proof.clone();
                self.close(proof)?;
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
                let ga = Goal {
                    hyps: g.hyps.clone(),
                    target: (**a).clone(),
                };
                let gb = Goal {
                    hyps: g.hyps.clone(),
                    target: (**b).clone(),
                };
                self.refine(
                    g.target.clone(),
                    InferenceRule::ConjunctionIntro,
                    vec![Child::Sub(ga), Child::Sub(gb)],
                );
                Ok(self)
            }
            // `A ↔ B` splits into its two implications. `InferenceRule::BicondIntro` and its
            // certifier arm (certifier.rs) have existed all along — no tactic ever CONSTRUCTED
            // one, so the rule was reachable only by hand-building a `DerivationTree`. Lean spells
            // this `constructor`, which is already one of `split`'s aliases.
            ProofExpr::Iff(a, b) => {
                let fwd = Goal {
                    hyps: g.hyps.clone(),
                    target: ProofExpr::Implies(a.clone(), b.clone()),
                };
                let bwd = Goal {
                    hyps: g.hyps.clone(),
                    target: ProofExpr::Implies(b.clone(), a.clone()),
                };
                self.refine(
                    g.target.clone(),
                    InferenceRule::BicondIntro,
                    vec![Child::Sub(fwd), Child::Sub(bwd)],
                );
                Ok(self)
            }
            other => Err(TacticError::DoesNotApply(format!(
                "split expects ∧ or ↔, got {other:?}"
            ))),
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
                let chosen = if take_left {
                    (**a).clone()
                } else {
                    (**b).clone()
                };
                let sub = Goal {
                    hyps: g.hyps.clone(),
                    target: chosen,
                };
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
    ///
    /// The witness is an arbitrary TERM — `use (n + 1)`, `use (f x)` — which is what Lean's
    /// `exists`/`use` takes. A compound witness used to be REFUSED here, because
    /// `InferenceRule::ExistentialIntro` carries its witness as a `String` that the certifier
    /// resolves as one name: `add(n, 1)` certified to a global literally called
    /// `"add(n, 1)"`, which no context declares, so the kernel rejected the artefact at
    /// `qed` — a failure arriving so far from the mistake that it read as "the theorem is
    /// false". The rule now carries the term ([`InferenceRule::ExistentialIntroTerm`],
    /// exactly as `UniversalInstTerm` sits beside `UniversalInst` for the dual quantifier),
    /// so the honest answer is to take the step.
    ///
    /// A bare NAME still emits `ExistentialIntro`, which is not a fast path but a
    /// COMPATIBILITY one: producers and consumers of that rule (including the eigenconstant
    /// renaming in `engine::rewrite_tree_const_to_var`) match on its `String` witness.
    ///
    /// The ∃-binder's DOMAIN is computed, not assumed: `certifier::existential_domain` reads
    /// it off the body, and `proof_expr_to_type` builds the goal type with the same function,
    /// so an arithmetic existential is `Ex Int (λm:Int. …)` on both sides of the trust door.
    /// Stamping the FOL default `Entity` on `∃m. m = n + 1` produced `Eq Int m` with
    /// `m : Entity`, which is not well-typed at all.
    pub fn exists(&mut self, witness: ProofTerm) -> Result<&mut Self, TacticError> {
        if matches!(witness, ProofTerm::Group(_)) {
            return Err(TacticError::DoesNotApply(format!(
                "exists {witness}: a tuple is not one witness — supply the components with \
                 the anonymous constructor `exact (w, h)`, which nests the ∃-introductions"
            )));
        }
        let g = self.focused_goal()?;
        match &g.target {
            ProofExpr::Exists { variable, body } => {
                let instantiated = instantiate_expr(body, variable, &witness);
                let sub = Goal {
                    hyps: g.hyps.clone(),
                    target: instantiated,
                };
                let witness_type = crate::certifier::existential_domain(variable, body);
                let rule = witness_rule(&witness, witness_type);
                self.refine(g.target.clone(), rule, vec![Child::Sub(sub)]);
                Ok(self)
            }
            other => Err(TacticError::DoesNotApply(format!(
                "exists expects ∃, got {other:?}"
            ))),
        }
    }

    /// `rfl`: close a goal `t = t` by reflexivity.
    ///
    /// The closer itself is not new — `simp` has always ended with it — but it had no name of its
    /// own, so a script could only reach reflexivity by asking for simplification and hoping. One
    /// closer, two names: `simp` now delegates here rather than carrying a second copy.
    pub fn rfl(&mut self) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        if let ProofExpr::Identity(l, r) = &g.target {
            if l == r {
                let target = g.target.clone();
                self.close(DerivationTree::leaf(target, InferenceRule::Reflexivity))?;
                return Ok(self);
            }
            return Err(TacticError::DoesNotApply(format!(
                "rfl: the two sides differ — {l:?} vs {r:?}"
            )));
        }
        Err(TacticError::DoesNotApply(format!(
            "rfl expects an equality goal, got {:?}",
            g.target
        )))
    }

    // === Proof TERMS ===

    /// `exact e`: close the goal with a proof TERM, not merely a hypothesis name.
    ///
    /// `exact (hpq hp)`, `exact (hp, hq)`, `exact h` where `h : ∀m. P m` against `P n` — all
    /// ordinary Lean, and all of them elaborate here against the goal. A term carrying a
    /// hole is refused: leaving goals open is [`refine`](Self::refine_term)'s job, and an
    /// `exact` that quietly did that would report a proof it had not finished.
    pub fn exact_term(&mut self, term: &PTerm) -> Result<&mut Self, TacticError> {
        self.build_from_term(term, false)
    }

    /// `refine e`: elaborate a proof term against the goal, and leave one SUBGOAL per hole,
    /// in the order the holes appear.
    ///
    /// The holes are proof-state goals (`Node::Hole`), never kernel term holes: the kernel's
    /// `types_equal` matches `Term::Hole` against ANYTHING, so a term with a hole in it
    /// typechecks and the resulting certificate is indistinguishable from a real one. A hole
    /// that is the WHOLE term is refused — it leaves the goal exactly as it was, so it is a
    /// step that claims to have done something and has not.
    pub fn refine_term(&mut self, term: &PTerm) -> Result<&mut Self, TacticError> {
        self.build_from_term(term, true)
    }

    /// `have h from e`: add `h`, proved by the term `e`, to the local scope.
    ///
    /// Nothing among the primitives introduced a hypothesis with an accompanying proof:
    /// `intro` takes one from the goal's antecedent, `cases` from an existing hypothesis's
    /// shape, and that was the complete set of ways a name entered a goal's scope. The
    /// hypothesis carries the elaborated derivation itself, so each USE emits that proof —
    /// no new rule, no lifted theorem, and a `have` whose statement mentions an
    /// `intro`-bound variable works like any other.
    pub fn have_term(&mut self, name: &str, term: &PTerm) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        let (prop, proof) = self.synthesize(term, &g.hyps)?;
        let mut hyps = g.hyps.clone();
        hyps.push(Hyp {
            name: name.to_string(),
            prop,
            proof,
        });
        self.set_focused_goal(Goal {
            hyps,
            target: g.target,
        });
        Ok(self)
    }

    fn build_from_term(
        &mut self,
        term: &PTerm,
        allow_holes: bool,
    ) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        if allow_holes && matches!(term, PTerm::Hole) {
            return Err(TacticError::DoesNotApply(
                "refine: a bare hole leaves the goal exactly as it was".to_string(),
            ));
        }
        let (node, holes) = self.elaborate(term, &g.hyps, &g.target, allow_holes)?;
        let idx = self.open.pop_front().expect("a focused goal");
        self.nodes[idx] = node;
        for &h in holes.iter().rev() {
            self.open.push_front(h);
        }
        Ok(self)
    }

    /// CHECK a proof term against an expected proposition, pushing its children into the
    /// node arena. Returns the root node (not yet placed) and the indices of the holes it
    /// spawned, in source order.
    fn elaborate(
        &mut self,
        term: &PTerm,
        hyps: &[Hyp],
        target: &ProofExpr,
        allow_holes: bool,
    ) -> Result<(Node, Vec<usize>), TacticError> {
        match term {
            PTerm::Hole => {
                if !allow_holes {
                    return Err(TacticError::DoesNotApply(
                        "a hole is only meaningful under `refine`".to_string(),
                    ));
                }
                Ok((
                    Node::Hole(Goal {
                        hyps: hyps.to_vec(),
                        target: target.clone(),
                    }),
                    Vec::new(),
                ))
            }
            // The anonymous constructor: which introduction rule it is comes from the
            // expected proposition, exactly as Lean's `⟨…⟩` does.
            PTerm::Tuple(items) => self.elaborate_anonymous(items, hyps, target, allow_holes),
            _ => {
                let (prop, tree) = self.synthesize(term, hyps)?;
                if prop == *target || unfold_not(&prop) == unfold_not(target) {
                    Ok((Node::Done(tree), Vec::new()))
                } else {
                    match instantiate_to(&prop, target, tree.clone()) {
                        Some(instantiated) => Ok((Node::Done(instantiated), Vec::new())),
                        None => Err(TacticError::DoesNotApply(format!(
                            "the term proves {prop:?}, which is not the goal {target:?}"
                        ))),
                    }
                }
            }
        }
    }

    fn elaborate_anonymous(
        &mut self,
        items: &[PTerm],
        hyps: &[Hyp],
        target: &ProofExpr,
        allow_holes: bool,
    ) -> Result<(Node, Vec<usize>), TacticError> {
        if items.len() < 2 {
            return Err(TacticError::DoesNotApply(
                "an anonymous constructor needs at least two components".to_string(),
            ));
        }
        // `⟨a, b, c⟩` against a right-nested shape is `⟨a, ⟨b, c⟩⟩`.
        let (head, rest) = items.split_first().expect("at least two components");
        let tail = if rest.len() == 1 {
            rest[0].clone()
        } else {
            PTerm::Tuple(rest.to_vec())
        };
        let (rule, left, right) = match target {
            ProofExpr::And(a, b) => (
                InferenceRule::ConjunctionIntro,
                (**a).clone(),
                (**b).clone(),
            ),
            ProofExpr::Iff(a, b) => (
                InferenceRule::BicondIntro,
                ProofExpr::Implies(a.clone(), b.clone()),
                ProofExpr::Implies(b.clone(), a.clone()),
            ),
            // `⟨w, hw⟩` for an existential: the first component is the WITNESS TERM, not a
            // proof, so it elaborates through the value language.
            ProofExpr::Exists { variable, body } => {
                let witness = self.value_term(head)?;
                let instantiated = instantiate_expr(body, variable, &witness);
                let (child, holes) =
                    self.elaborate_child(&tail, hyps, &instantiated, allow_holes)?;
                let witness_type = crate::certifier::existential_domain(variable, body);
                let rule = witness_rule(&witness, witness_type);
                return Ok((
                    Node::Filled {
                        conclusion: target.clone(),
                        rule,
                        children: vec![child],
                    },
                    holes,
                ));
            }
            other => {
                return Err(TacticError::DoesNotApply(format!(
                    "an anonymous constructor expects ∧, ↔ or ∃, got {other:?}"
                )))
            }
        };
        let (left_idx, mut holes) = self.elaborate_child(head, hyps, &left, allow_holes)?;
        let (right_idx, right_holes) = self.elaborate_child(&tail, hyps, &right, allow_holes)?;
        holes.extend(right_holes);
        Ok((
            Node::Filled {
                conclusion: target.clone(),
                rule,
                children: vec![left_idx, right_idx],
            },
            holes,
        ))
    }

    /// [`elaborate`](Self::elaborate) into a fresh arena slot, reporting its index.
    fn elaborate_child(
        &mut self,
        term: &PTerm,
        hyps: &[Hyp],
        target: &ProofExpr,
        allow_holes: bool,
    ) -> Result<(usize, Vec<usize>), TacticError> {
        let (node, holes) = self.elaborate(term, hyps, target, allow_holes)?;
        let is_hole = matches!(node, Node::Hole(_));
        let idx = self.nodes.len();
        self.nodes.push(node);
        Ok((idx, if is_hole { vec![idx] } else { holes }))
    }

    /// SYNTHESIZE a proof term's proposition. Terms whose type is determined by the term
    /// itself — a name, or an application of one — go this way; the anonymous constructor
    /// and the hole do not, because only the expected proposition decides what they mean.
    fn synthesize(
        &mut self,
        term: &PTerm,
        hyps: &[Hyp],
    ) -> Result<(ProofExpr, DerivationTree), TacticError> {
        match term {
            PTerm::Ident(name) => hyps
                .iter()
                .find(|h| &h.name == name)
                .map(|h| (h.prop.clone(), h.proof.clone()))
                .ok_or_else(|| TacticError::NoSuchHypothesis(name.clone())),
            PTerm::App(head, args) => {
                let (mut prop, mut proof) = self.synthesize(head, hyps)?;
                for arg in args {
                    let (next_prop, next_proof) = match unfold_not(&prop) {
                        // A universal applied to a term is INSTANTIATION; an implication
                        // applied to a proof is modus ponens. The head's own shape decides.
                        ProofExpr::Implies(ant, con) => {
                            let (node, holes) = self.elaborate(arg, hyps, &ant, false)?;
                            debug_assert!(holes.is_empty(), "checked without holes");
                            let Node::Done(arg_proof) = node else {
                                return Err(TacticError::DoesNotApply(
                                    "an applied argument must be a complete proof".to_string(),
                                ));
                            };
                            (
                                (*con).clone(),
                                DerivationTree::new(
                                    (*con).clone(),
                                    InferenceRule::ModusPonens,
                                    vec![proof, arg_proof],
                                ),
                            )
                        }
                        ProofExpr::ForAll { .. } => {
                            let witness = self.value_term(arg)?;
                            let instantiated = instantiate_forall(&prop, &witness)
                                .expect("the proposition is universally quantified");
                            (
                                instantiated.clone(),
                                DerivationTree::new(
                                    instantiated,
                                    InferenceRule::UniversalInstTerm(witness),
                                    vec![proof],
                                ),
                            )
                        }
                        other => {
                            return Err(TacticError::DoesNotApply(format!(
                                "{other:?} is neither an implication nor a universal, so it \
                                 cannot be applied"
                            )))
                        }
                    };
                    prop = next_prop;
                    proof = next_proof;
                }
                Ok((prop, proof))
            }
            PTerm::Hole => Err(TacticError::DoesNotApply(
                "a hole's proposition cannot be inferred — it is only meaningful where the \
                 goal supplies one"
                    .to_string(),
            )),
            PTerm::Tuple(_) => Err(TacticError::DoesNotApply(
                "an anonymous constructor's proposition cannot be inferred — it is only \
                 meaningful against a known goal"
                    .to_string(),
            )),
            PTerm::Op(op, _, _) => Err(TacticError::DoesNotApply(format!(
                "`{op}` builds a VALUE, not a proof"
            ))),
        }
    }

    /// Elaborate a script term as a VALUE — an `∃` witness, or the argument of a universal.
    ///
    /// A bare name is the `Variable` the surrounding proof bound if the goal's scope has one
    /// of that name, and a `Constant` otherwise. That distinction is invisible in every
    /// diagnostic a user sees (both `Display` as the bare name) and decides whether the
    /// instantiated goal matches its context at all, so it is READ OFF the scope rather than
    /// guessed: `intro n . exists n` must produce the `n` that `intro` bound.
    pub fn value_term(&self, term: &PTerm) -> Result<ProofTerm, TacticError> {
        let bound = self.scope_free_vars();
        value_of(term, &bound)
    }

    /// `by_contra h`: assume the goal is FALSE under the name `h` and leave `⊥` to derive.
    ///
    /// Lean's `by_contra` is a STEP, and that is the whole point: a proof shaped
    /// `intro x; by_contra h; <twenty steps>` needs the twenty steps to be writable. The
    /// classical reductio itself is not new — `auto`'s search reaches it — but only as one
    /// opaque call with no way to help it along. Certified as `ClassicalReductio`, which
    /// discharges through the single classical axiom `dne`, so the axiom footprint of the
    /// theorem still records that it went classical.
    pub fn by_contra(&mut self, name: &str) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        if is_falsum_expr(&g.target) {
            return Err(TacticError::DoesNotApply(
                "by_contra: the goal is already ⊥, so there is nothing to negate".to_string(),
            ));
        }
        let mut hyps = g.hyps.clone();
        hyps.push(direct_hyp(
            name.to_string(),
            ProofExpr::Not(Box::new(g.target.clone())),
        ));
        let sub = Goal {
            hyps,
            target: ProofExpr::Atom(FALSUM.to_string()),
        };
        self.refine(
            g.target.clone(),
            InferenceRule::ClassicalReductio,
            vec![Child::Sub(sub)],
        );
        Ok(self)
    }

    /// `ring`: close a polynomial identity over the commutative-ring vocabulary
    /// (`add`/`sub`/`mul` over literals and opaque atoms).
    ///
    /// The normaliser is not new — it is [`crate::arith::prove_int_eq`], the same
    /// proof-PRODUCING oracle `InferenceRule::ArithDecision` certifies through, whose
    /// trusted base is the seven registered ring axioms. What was missing was a name on the
    /// tactic-script surface: `ring` was reachable only from the literate kernel surface,
    /// and the three vernacular surfaces did not interoperate.
    ///
    /// The oracle is consulted HERE, not only at `qed`: its polynomial layer is a COMPLETE
    /// negative decision, so a goal that is not a formal ring identity is declined by the
    /// tactic rather than accepted and blown up at the trust door.
    pub fn ring(&mut self) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        let ProofExpr::Identity(l, r) = &g.target else {
            return Err(TacticError::DoesNotApply(format!(
                "ring expects an equality goal, got {:?}",
                g.target
            )));
        };
        if is_bool_valued(l) || is_bool_valued(r) {
            return Err(TacticError::DoesNotApply(format!(
                "ring proves a RING identity over `Int`; `{}` is a Bool fact — a ground one is                  `decide`'s and a symbolic one is `omega`'s",
                g.target
            )));
        }
        if !ring_identity_holds(l, r) {
            return Err(TacticError::DoesNotApply(
                "ring: the two sides are not the same polynomial".to_string(),
            ));
        }
        let target = g.target.clone();
        self.close(DerivationTree::leaf(target, InferenceRule::ArithDecision))?;
        Ok(self)
    }

    /// `apply h`: look the rule up by hypothesis NAME, then reduce the goal by modus ponens.
    ///
    /// [`ProofState::apply`] takes the rule as a `ProofExpr`, which a proof script has no way to
    /// write — that is the whole reason `apply` was unreachable from script text despite working.
    /// This is the name-taking door, mirroring [`ProofState::exact`].
    pub fn apply_named(&mut self, name: &str) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        let rule = match g.hyps.iter().find(|h| h.name == name) {
            Some(h) => h.prop.clone(),
            None => return Err(TacticError::NoSuchHypothesis(name.to_string())),
        };
        self.apply(&rule)
    }

    /// `apply rule`: `rule` is `P → Goal` (a hypothesis or premise). Reduce the goal
    /// to its antecedent `P` by modus ponens.
    pub fn apply(&mut self, rule: &ProofExpr) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        if let ProofExpr::Implies(ant, con) = rule {
            if unify_exprs(&g.target, con).is_ok() {
                let rule_leaf = DerivationTree::leaf(rule.clone(), InferenceRule::PremiseMatch);
                let sub = Goal {
                    hyps: g.hyps.clone(),
                    target: (**ant).clone(),
                };
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
        self.cases_with(name, &[])
    }

    /// `cases h with a b` / `obtain ⟨a, b⟩ := h`: [`cases`](Self::cases), with the components
    /// bound under the names the PROOF chose instead of the generated
    /// `h_1`/`h_2`/`h_l`/`h_r`/`h_w`.
    ///
    /// A Lean proof names what it destructures and every later step cites those names; a
    /// translator forced to renumber them is rewriting the proof. The arity is checked
    /// against the shape, because `cases h with hp` on a conjunction is a mistake about what
    /// `h` IS, not a shorthand.
    ///
    /// The ∃ branch takes `with w hw`: `w` names the fresh eigen-constant and `hw` the
    /// instantiated body. A chosen witness name is REFUSED when it is already in play in the
    /// goal or its scope — the eigenvariable condition is what makes ∃-elimination sound,
    /// and a name the surrounding proof already uses is not fresh.
    pub fn cases_with(&mut self, name: &str, names: &[String]) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        let hyp = g
            .hyps
            .iter()
            .find(|h| h.name == name)
            .cloned()
            .ok_or_else(|| TacticError::NoSuchHypothesis(name.to_string()))?;
        let want = |n: usize, shape: &str| -> Result<(), TacticError> {
            if names.is_empty() || names.len() == n {
                Ok(())
            } else {
                Err(TacticError::DoesNotApply(format!(
                    "cases {name} with …: {shape} binds {n} name(s), {} given",
                    names.len()
                )))
            }
        };
        let pick =
            |i: usize, fallback: String| -> String { names.get(i).cloned().unwrap_or(fallback) };
        match &hyp.prop {
            ProofExpr::And(a, b) => {
                want(2, "a conjunction")?;
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
                hyps.push(Hyp {
                    name: pick(0, format!("{name}_1")),
                    prop: (**a).clone(),
                    proof: proj_a,
                });
                hyps.push(Hyp {
                    name: pick(1, format!("{name}_2")),
                    prop: (**b).clone(),
                    proof: proj_b,
                });
                self.set_focused_goal(Goal {
                    hyps,
                    target: g.target.clone(),
                });
                Ok(self)
            }
            ProofExpr::Or(a, b) => {
                want(2, "a disjunction")?;
                let mut ha = g.hyps.clone();
                ha.push(direct_hyp(pick(0, format!("{name}_l")), (**a).clone()));
                let ga = Goal {
                    hyps: ha,
                    target: g.target.clone(),
                };
                let mut hb = g.hyps.clone();
                hb.push(direct_hyp(pick(1, format!("{name}_r")), (**b).clone()));
                let gb = Goal {
                    hyps: hb,
                    target: g.target.clone(),
                };
                self.refine(
                    g.target.clone(),
                    InferenceRule::DisjunctionCases,
                    vec![
                        Child::Closed(hyp.proof.clone()),
                        Child::Sub(ga),
                        Child::Sub(gb),
                    ],
                );
                Ok(self)
            }
            ProofExpr::Exists { variable, body } => {
                want(2, "an existential")?;
                let witness = match names.first() {
                    Some(chosen) => {
                        if self.scope_names(&g).contains(chosen) {
                            return Err(TacticError::DoesNotApply(format!(
                                "cases {name} with {chosen} …: `{chosen}` is already in scope, \
                                 so it is not a fresh witness"
                            )));
                        }
                        self.fresh += 1;
                        chosen.clone()
                    }
                    None => self.fresh_witness(),
                };
                let phi_c = instantiate_expr(body, variable, &ProofTerm::Constant(witness.clone()));
                let mut hyps = g.hyps.clone();
                hyps.push(direct_hyp(pick(1, format!("{name}_w")), phi_c));
                let gbody = Goal {
                    hyps,
                    target: g.target.clone(),
                };
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

    /// `induction`: structural induction on the goal's OUTERMOST `∀` binder.
    ///
    /// The scheme is READ OFF the scope rather than hard-wired: a hypothesis `P(C)` names a
    /// nullary constructor and a hypothesis `∀ā. (P(aᵢ) → …) → P(C ā)` names a constructor
    /// with induction hypotheses at the `aᵢ`, so a goal over a list whose ENil/ECons
    /// premises are in scope splits on ENil and ECons — where the hard-wired Peano scheme
    /// substituted `Zero`/`Succ k` into a list-valued position and produced syntactically
    /// well-formed nonsense that the automation would then happily "prove". Nat keeps the
    /// specialised path, whose `Fix`/`Match` certifies over the kernel's own `Nat` recursor;
    /// everything else routes to the dependent [`induction_over`](Self::induction_over).
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
        // A scheme recovered from the scope wins, unless it IS the Peano scheme — for which
        // the specialised Nat recursor below is the better certified object.
        if let Some((ind_type, ctors)) = infer_ctors(&variable, &body, &g.hyps) {
            if ind_type != "Nat" {
                return self.induction_over(&ind_type, ctors);
            }
        }
        self.fresh += 1;
        // Uppercase so the search treats the step variable as a rigid eigen-CONSTANT
        // (never a unifiable metavariable, which the backward chainer would either
        // ground to `Zero` or expand into an infinite `P(Succ^n k)` chain). It is
        // remapped back to a bound `Variable` when the proof is assembled (see
        // `assemble`), which is what the certifier's `Match` arm binds.
        let step_var = format!("K{}", self.fresh);
        let subst_to = |term: ProofTerm| instantiate_expr(&body, &variable, &term);
        let base_target = subst_to(ProofTerm::Constant("Zero".to_string()));
        let succ_k = ProofTerm::Function(
            "Succ".to_string(),
            vec![ProofTerm::Constant(step_var.clone())],
        );
        let step_target = subst_to(succ_k);
        let ih_prop = subst_to(ProofTerm::Constant(step_var.clone()));

        let base_goal = Goal {
            hyps: g.hyps.clone(),
            target: base_target,
        };
        // Put the induction hypothesis FIRST so `auto`/`assumption` discharge the step's
        // recursive obligation `P(k)` against the IH (keeping `k` symbolic) rather than
        // against a base fact like `P(Zero)` (which would unsoundly ground `k := Zero`).
        let mut step_hyps = vec![direct_hyp("ih".to_string(), ih_prop)];
        step_hyps.extend(g.hyps.clone());
        let step_goal = Goal {
            hyps: step_hyps,
            target: step_target,
        };

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

    /// `induction n`: structural induction on the NAMED variable, wherever it sits in the
    /// goal's `∀` prefix.
    ///
    /// The argumentless [`induction`](Self::induction) takes whatever binder happens to be
    /// outermost, so against `∀m n. Q m n` it and Lean's `induction n` prove DIFFERENT
    /// theorems — both certified, and nothing downstream can tell. Naming the variable is
    /// how a translator says which one it meant.
    ///
    /// For an inner variable the goal is first GENERALIZED: `∀ō. ∀v. body` is proved from
    /// `∀v. ∀ō. body` — the motive `λv. ∀ō. body`, which is the strengthened induction
    /// hypothesis a proof by induction on an inner variable needs. The bridge between the
    /// two orders is built from `UniversalIntro` and `UniversalInst` alone (introduce the
    /// prefix, then instantiate the reordered theorem at the introduced variables), so it
    /// certifies through rules that already existed — no new axiom, no new rule.
    ///
    /// When the prefix does not bind `v` at all the tactic does NOT refuse: it takes Lean's
    /// reading and generalizes — see `induction_generalizing`.
    pub fn induction_on(&mut self, variable: &str) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        let (prefix, _) = peel_foralls(&g.target);
        let Some(position) = prefix.iter().position(|v| v == variable) else {
            return self.induction_generalizing(variable);
        };
        if position == 0 {
            return self.induction();
        }
        // The reordered statement `∀v. ∀ō. body`, and the bridge down to the body.
        let outer: Vec<String> = prefix[..position].to_vec();
        let reordered = reorder_forall_prefix(&g.target, variable).ok_or_else(|| {
            TacticError::DoesNotApply(format!(
                "induction {variable}: the goal's ∀ prefix could not be reordered"
            ))
        })?;

        // The induction variable's DOMAIN: the structural-induction node below types its
        // binder as the inductive, so the λ that binds `v` above must agree or the
        // application the bridge builds does not typecheck.
        let reordered_body = match &reordered {
            ProofExpr::ForAll { body, .. } => (**body).clone(),
            _ => unreachable!("reorder_forall_prefix returns a universal"),
        };
        let ind_type = infer_ctors(variable, &reordered_body, &g.hyps)
            .map(|(t, _)| t)
            .unwrap_or_else(|| "Nat".to_string());

        // Downward: introduce ō, then v at the inductive's domain, landing on the bare body.
        for name in &outer {
            self.intro(name)?;
        }
        let focused = self.focused_goal()?;
        let ProofExpr::ForAll { body, .. } = &focused.target else {
            return Err(TacticError::DoesNotApply(format!(
                "induction {variable}: the prefix did not strip down to the binder"
            )));
        };
        let sub = Goal {
            hyps: focused.hyps.clone(),
            target: (**body).clone(),
        };
        self.refine(
            focused.target.clone(),
            InferenceRule::UniversalIntro {
                variable: variable.to_string(),
                var_type: ind_type,
            },
            vec![Child::Sub(sub)],
        );
        // Upward: the body follows from the reordered theorem instantiated at v, then at
        // each ō in turn. `witnesses[i]` justifies `conclusions[i]` from `conclusions[i-1]`,
        // with the reordered theorem itself below the bottom of the chain.
        let witnesses: Vec<String> = std::iter::once(variable.to_string())
            .chain(outer.iter().cloned())
            .collect();
        let mut conclusions = Vec::with_capacity(witnesses.len());
        let mut concl = reordered.clone();
        for name in &witnesses {
            concl = instantiate_forall(&concl, &ProofTerm::Variable(name.clone())).ok_or_else(
                || {
                    TacticError::DoesNotApply(
                        "induction: the reordered goal lost a binder".to_string(),
                    )
                },
            )?;
            conclusions.push(concl.clone());
        }
        // The derivation nests the other way round: the focused hole IS the last
        // conclusion, and each refinement exposes the one below it.
        let hyps = self.focused_goal()?.hyps;
        for i in (0..witnesses.len()).rev() {
            let below = if i == 0 {
                reordered.clone()
            } else {
                conclusions[i - 1].clone()
            };
            self.refine(
                conclusions[i].clone(),
                InferenceRule::UniversalInst(witnesses[i].clone()),
                vec![Child::Sub(Goal {
                    hyps: hyps.clone(),
                    target: below,
                })],
            );
        }
        self.induction()
    }

    /// `induction n` where the goal's `∀` prefix does not bind `n`.
    ///
    /// Three readings are possible and they are told apart by what `n` can be TALKING about —
    /// which is the same question Lean answers with its local context, and all an untyped
    /// `ProofExpr` can answer without one:
    ///
    /// * **`n` is a free local** of the goal or of a hypothesis. This is Lean's `induction n` on
    ///   a local: REVERT every hypothesis that mentions `n`, generalize the goal over `n`, apply
    ///   the eliminator, and re-introduce what was reverted in each branch.
    /// * **`n` is nothing at all, and the goal QUANTIFIES.** Then a binder was plainly meant and
    ///   this one is not it; taking a different binder is how two theorems get proved under one
    ///   name, so this REFUSES and says which binders exist.
    /// * **`n` is nothing at all, and the goal does not quantify.** There is no binder that could
    ///   have been meant, and the generalization is vacuous: `n ∉ FV(G)`, so `∀n:T. G ⊢ G` needs
    ///   only an inhabitant of `T`. This is the case our own `ProofStrategy::Induction` doc
    ///   comment teaches verbatim (`Proof: Induction on n.` against `Given: Bob is happy. Prove:
    ///   Bob is happy.`), and refusing it left the documentation teaching a spelling that does
    ///   not work — worse than an undocumented gap, because a producer copies it.
    ///
    /// Nothing new is assumed anywhere in this. The reverted hypotheses come back by one
    /// `ModusPonens` against the leaf each already carried, and the generalization descends by
    /// `UniversalInstTerm` — at `n` itself when `n` is a local, and at the inductive's closed
    /// base point when it is vacuous.
    ///
    /// The revert is not bookkeeping. A hypothesis stated about the LOCAL `n` left in scope would
    /// sit under the eliminator's own `λn`, which SHADOWS that local: the branch would be reading
    /// a hypothesis about a different `n` than the one it was stated about.
    ///
    /// The two live cases differ in what they must KNOW. A vacuous generalization binds a variable
    /// the motive never mentions, so any inhabited inductive serves and `Nat` is taken. A
    /// generalization over a local COMMITS the eliminator to that local's domain, and `ProofExpr`
    /// carries no types — so the domain has to come from the scope, via the induction principle
    /// stated as premises ([`infer_ctors`]). When the scope does not say, this refuses rather than
    /// guess `Nat`: the guess would build a tree naming a domain nothing supports, and a tactic
    /// that returns `Ok` with an object the kernel must refuse is a claim, not a tactic.
    fn induction_generalizing(&mut self, variable: &str) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        let mentions = |e: &ProofExpr| {
            let mut fv = std::collections::BTreeSet::new();
            free_vars(e, &mut fv);
            fv.contains(variable)
        };
        let (reverted, kept): (Vec<Hyp>, Vec<Hyp>) =
            g.hyps.iter().cloned().partition(|h| mentions(&h.prop));
        let is_local = mentions(&g.target) || !reverted.is_empty();
        if !is_local {
            let (prefix, _) = peel_foralls(&g.target);
            if !prefix.is_empty() {
                return Err(TacticError::DoesNotApply(format!(
                    "induction {variable}: the goal's ∀ prefix binds {prefix:?}, not `{variable}` \
                     — a goal that quantifies has a binder this tactic was meant for, and taking \
                     a different one proves a different theorem under the same name"
                )));
            }
        }
        // The scheme is read off the ORIGINAL goal: the reverted form below is an implication,
        // and a constructor case stated as a premise matches the goal's shape, not that one.
        let scheme = infer_ctors(variable, &g.target, &g.hyps);
        let ind_type = match (&scheme, is_local) {
            (Some((t, _)), _) => t.clone(),
            (None, false) => "Nat".to_string(),
            (None, true) => {
                return Err(TacticError::DoesNotApply(format!(
                    "induction {variable}: `{variable}` is a free local, not a bound variable, \
                     and nothing in scope says what it ranges over — state the induction \
                     principle for its type as premises (`P(C)` and `∀ā. P(aᵢ) → P(C ā)`) and \
                     the domain is read off them"
                )));
            }
        };

        // REVERT: `Γ, r̄ ⊢ G` becomes `Γ ⊢ r̄ → G`.
        let mut reverted_target = g.target.clone();
        for h in reverted.iter().rev() {
            reverted_target =
                ProofExpr::Implies(Box::new(h.prop.clone()), Box::new(reverted_target));
        }
        let generalized = ProofExpr::ForAll {
            variable: variable.to_string(),
            body: Box::new(reverted_target.clone()),
        };
        // Where to land back on the original goal. A local is instantiated at ITSELF; a vacuous
        // variable at the inductive's closed base point.
        let witness = if is_local {
            ProofTerm::Variable(variable.to_string())
        } else {
            let base = nullary_constructor(&ind_type).ok_or_else(|| {
                TacticError::DoesNotApply(format!(
                    "induction {variable}: `{variable}` occurs nowhere in the goal, and \
                     `{ind_type}` has no nullary constructor to instantiate the generalization \
                     back down at"
                ))
            })?;
            ProofTerm::Constant(base)
        };

        // Downward, innermost first: the focused hole is `G`, discharged by one `ModusPonens`
        // per reverted hypothesis (the implication is the SUBGOAL, the hypothesis is the leaf it
        // already carried), and the last of those holes is `r̄ → G`, discharged by instantiating
        // the generalization.
        let mut concl = g.target.clone();
        for h in reverted.iter().rev() {
            let imp = ProofExpr::Implies(Box::new(h.prop.clone()), Box::new(concl.clone()));
            self.refine(
                concl,
                InferenceRule::ModusPonens,
                vec![
                    Child::Sub(Goal {
                        hyps: kept.clone(),
                        target: imp.clone(),
                    }),
                    Child::Closed(h.proof.clone()),
                ],
            );
            concl = imp;
        }
        self.refine(
            concl,
            InferenceRule::UniversalInstTerm(witness),
            vec![Child::Sub(Goal {
                hyps: kept.clone(),
                target: generalized,
            })],
        );

        // The eliminator. `induction` re-reads the scheme from the goal it is handed, and the
        // reverted goal no longer has the shape a premise-stated constructor case matches, so a
        // non-Nat scheme is dispatched here from what was read off the original.
        let before: std::collections::BTreeSet<usize> = self.open.iter().copied().collect();
        match scheme {
            Some((t, ctors)) if t != "Nat" => self.induction_over(&t, ctors)?,
            _ => self.induction()?,
        };

        // …then every branch it produced gets its reverted hypotheses back, under their
        // ORIGINAL names, so a script written before the revert still reads.
        if reverted.is_empty() {
            return Ok(self);
        }
        let produced: VecDeque<usize> = self.open.clone();
        let mut new_open = VecDeque::new();
        for idx in produced {
            if before.contains(&idx) {
                new_open.push_back(idx);
                continue;
            }
            self.open = VecDeque::from([idx]);
            for h in &reverted {
                self.intro(&h.name)?;
            }
            new_open.extend(self.open.iter().copied());
        }
        self.open = new_open;
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
    /// backward chainer neither grounds nor unrolls them); `assemble`
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
        let subst_to = |term: ProofTerm| instantiate_expr(&body, &variable, &term);

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
                    arg_names
                        .iter()
                        .map(|n| ProofTerm::Constant(n.clone()))
                        .collect(),
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
            children.push(Child::Sub(Goal {
                hyps,
                target: case_target,
            }));

            cases.push(InductionCase {
                constructor: ctor.constructor.clone(),
                args: arg_names
                    .iter()
                    .zip(ctor.recursive.iter())
                    .map(|(name, &recursive)| InductionArg {
                        name: name.clone(),
                        recursive,
                    })
                    .collect(),
            });
        }

        self.refine(
            g.target.clone(),
            InferenceRule::InductionScheme {
                variable,
                ind_type: ind_type.to_string(),
                cases,
            },
            children,
        );
        Ok(self)
    }

    /// `rewrite h`: rewrite the goal left-to-right by the equation `h`.
    ///
    /// `h` may be GROUND (`lhs = rhs`) or ∀-QUANTIFIED (`∀x̄. lhs = rhs`), which is the
    /// commonest kind in real mathematics and had no image at all before. A quantified
    /// equation is instantiated the way Lean's `rw` instantiates it: the first (outermost,
    /// leftmost) subterm of the goal that matches `lhs` fixes the pattern variables, and the
    /// rewrite then replaces that INSTANTIATED left-hand side — so `∀x. f x = g x` against
    /// `f 1 = g 1 ∧ f 2 = g 2` touches the `f 1` and leaves the `f 2` alone.
    ///
    /// Substitution is capture-avoiding (`substitute_in_expr`), so an occurrence the goal
    /// BINDS is not an occurrence of the pattern and a replacement mentioning a free
    /// variable is not captured by a binder that happens to share its spelling.
    ///
    /// Certified by `Eq_rec` (Leibniz): the node concludes the original goal, its equality
    /// premise proves `rhs = lhs` (the ∀-instantiation chain over `h`, then symmetry), and
    /// its source premise is the rewritten subgoal. Fails if the pattern does not occur.
    ///
    /// ONE occurrence — the first. `rw` used to rewrite every structural match, so a goal
    /// `P(a, a)` with `h : a = b` could only become `P(b, b)`, and the frequently-provable
    /// `P(b, a)` was unreachable from any surface: `InferenceRule::Rewrite` carried no
    /// position either, so even a Rust caller had nowhere to say which one it meant. Use
    /// [`ProofState::rewrite_at`] to name another occurrence; `repeat (rewrite h)` recovers
    /// the all-occurrences sweep, one certified step per occurrence.
    pub fn rewrite(&mut self, name: &str) -> Result<&mut Self, TacticError> {
        self.rewrite_at(name, 0)
    }

    /// `rewrite h at k`: [`ProofState::rewrite`] aimed at the `k`-th (0-based) FREE
    /// occurrence of the equation's left-hand side, counted left to right. Lean's `conv`
    /// and its occurrence arguments exist for exactly this, and the occurrence travels all
    /// the way to the trust door as `InferenceRule::RewriteAt` — the `Eq_rec` motive
    /// abstracts that one position, so the kernel re-checks the choice rather than taking
    /// the tactic's word for it.
    pub fn rewrite_at(&mut self, name: &str, occurrence: usize) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        let hyp = g
            .hyps
            .iter()
            .find(|h| h.name == name)
            .cloned()
            .ok_or_else(|| TacticError::NoSuchHypothesis(name.to_string()))?;
        let (pattern_vars, equation) = peel_foralls(&hyp.prop);
        let (lhs, rhs) = match equation {
            ProofExpr::Identity(l, r) => (l, r),
            other => {
                return Err(TacticError::DoesNotApply(format!(
                    "rewrite expects an equality hypothesis, got {other:?}"
                )))
            }
        };

        // Instantiate a quantified equation at its first match in the goal.
        let mut instantiation = Vec::new();
        let (lhs, rhs) = if pattern_vars.is_empty() {
            (lhs, rhs)
        } else {
            let subst = first_match_in_expr(&g.target, &lhs, &pattern_vars).ok_or_else(|| {
                TacticError::DoesNotApply(format!("rewrite: no subterm of the goal matches {lhs}"))
            })?;
            for v in &pattern_vars {
                match subst.get(v) {
                    Some(t) => instantiation.push(t.clone()),
                    // A variable the left-hand side never mentions is undetermined, so the
                    // instantiated right-hand side would carry a free metavariable.
                    None => {
                        return Err(TacticError::DoesNotApply(format!(
                            "rewrite: the match does not determine `{v}`"
                        )))
                    }
                }
            }
            (
                apply_subst_to_term(&lhs, &subst),
                apply_subst_to_term(&rhs, &subst),
            )
        };

        let (subgoal_target, occurrences) =
            substitute_occurrence_in_expr(&g.target, &lhs, &rhs, occurrence);
        if occurrences == 0 {
            return Err(TacticError::DoesNotApply(format!(
                "rewrite: {lhs} does not occur FREE in the goal"
            )));
        }
        if occurrence >= occurrences {
            return Err(TacticError::DoesNotApply(format!(
                "rewrite: the goal has {occurrences} free occurrence(s) of {lhs}, so there is \
                 no occurrence {occurrence}"
            )));
        }
        if subgoal_target == g.target {
            return Err(TacticError::DoesNotApply(format!(
                "rewrite: replacing occurrence {occurrence} of {lhs} by {rhs} leaves the goal \
                 unchanged, so the step proves nothing"
            )));
        }

        // The equality premise: instantiate the rule at the matched terms, then symmetrise —
        // the premise must conclude `rhs = lhs`.
        let mut eq_proof = hyp.proof.clone();
        let mut concl = hyp.prop.clone();
        for witness in instantiation {
            concl = instantiate_forall(&concl, &witness).ok_or_else(|| {
                TacticError::DoesNotApply(
                    "rewrite: the equation has fewer binders than the match".to_string(),
                )
            })?;
            eq_proof = DerivationTree::new(
                concl.clone(),
                InferenceRule::UniversalInstTerm(witness),
                vec![eq_proof],
            );
        }
        let eq_sym = DerivationTree::new(
            ProofExpr::Identity(rhs.clone(), lhs.clone()),
            InferenceRule::EqualitySymmetry,
            vec![eq_proof],
        );
        let sub = Goal {
            hyps: g.hyps.clone(),
            target: subgoal_target,
        };
        self.refine(
            g.target.clone(),
            InferenceRule::RewriteAt {
                from: rhs,
                to: lhs,
                occurrence,
            },
            vec![Child::Closed(eq_sym), Child::Sub(sub)],
        );
        Ok(self)
    }

    /// `rewrite h at hp`: rewrite the HYPOTHESIS `hp` with the equation `h` — Lean's
    /// `rw [h] at hp`, and the LOCATION half of positional rewriting.
    ///
    /// [`ProofState::rewrite_at`] reads and writes the GOAL only, so a Lean proof that
    /// normalises a hypothesis before using it had no image at all. The corpus case for `conv`
    /// makes the point sharply: positional rewriting has two aspects, the POSITION and the
    /// LOCATION, and "a translator that fixes only one of them has fixed nothing".
    ///
    /// The step runs FORWARD, where the goal rewrite runs backward: the hypothesis's own
    /// derivation is the SOURCE premise and the rewritten proposition is the conclusion, so the
    /// goal is untouched and only the context moves. Nothing is ASSUMED — the new hypothesis
    /// carries a complete `Eq_rec` derivation built from the old one, which `assumption` and
    /// `exact` splice in and which `qed` re-checks like any other step. That is why no
    /// refinement node is needed: a hypothesis in this proof state is a proposition WITH its
    /// proof, and this replaces both at once.
    ///
    /// The motive's position is CHOSEN by `crate::engine::leibniz_step` rather than taken
    /// from the caller's index: the tactic layer counts occurrences in a `ProofExpr` and the
    /// kernel counts them in a `Term`, the two agree on the leftmost and can differ deeper, and
    /// the kernel is the one that decides.
    pub fn rewrite_hyp_at(
        &mut self,
        eq_name: &str,
        hyp_name: &str,
        occurrence: usize,
    ) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        if eq_name == hyp_name {
            return Err(TacticError::DoesNotApply(format!(
                "rewrite {eq_name} at {hyp_name}: an equation cannot rewrite itself"
            )));
        }
        let eq_hyp = g
            .hyps
            .iter()
            .find(|h| h.name == eq_name)
            .cloned()
            .ok_or_else(|| TacticError::NoSuchHypothesis(eq_name.to_string()))?;
        let target_hyp = g
            .hyps
            .iter()
            .find(|h| h.name == hyp_name)
            .cloned()
            .ok_or_else(|| TacticError::NoSuchHypothesis(hyp_name.to_string()))?;
        let (pattern_vars, equation) = peel_foralls(&eq_hyp.prop);
        let ProofExpr::Identity(lhs, rhs) = equation else {
            return Err(TacticError::DoesNotApply(format!(
                "rewrite expects an equality hypothesis, got {:?}",
                eq_hyp.prop
            )));
        };

        // Instantiate a quantified equation at its first match IN THE HYPOTHESIS — the
        // location decides where the pattern is looked for, not just where it is replaced.
        let mut instantiation = Vec::new();
        let (lhs, rhs) = if pattern_vars.is_empty() {
            (lhs, rhs)
        } else {
            let subst =
                first_match_in_expr(&target_hyp.prop, &lhs, &pattern_vars).ok_or_else(|| {
                    TacticError::DoesNotApply(format!(
                        "rewrite: no subterm of `{hyp_name}` matches {lhs}"
                    ))
                })?;
            for v in &pattern_vars {
                match subst.get(v) {
                    Some(t) => instantiation.push(t.clone()),
                    None => {
                        return Err(TacticError::DoesNotApply(format!(
                            "rewrite: the match does not determine `{v}`"
                        )))
                    }
                }
            }
            (
                apply_subst_to_term(&lhs, &subst),
                apply_subst_to_term(&rhs, &subst),
            )
        };

        let (new_prop, occurrences) =
            substitute_occurrence_in_expr(&target_hyp.prop, &lhs, &rhs, occurrence);
        if occurrences == 0 {
            return Err(TacticError::DoesNotApply(format!(
                "rewrite: {lhs} does not occur FREE in `{hyp_name}`"
            )));
        }
        if occurrence >= occurrences {
            return Err(TacticError::DoesNotApply(format!(
                "rewrite: `{hyp_name}` has {occurrences} free occurrence(s) of {lhs}, so there \
                 is no occurrence {occurrence}"
            )));
        }
        if new_prop == target_hyp.prop {
            return Err(TacticError::DoesNotApply(format!(
                "rewrite: replacing occurrence {occurrence} of {lhs} by {rhs} leaves \
                 `{hyp_name}` unchanged, so the step proves nothing"
            )));
        }

        // The equality premise, instantiated at the match. FORWARD, so — unlike the goal
        // rewrite — it is NOT symmetrised: the hypothesis travels from `lhs` to `rhs`.
        let mut eq_proof = eq_hyp.proof.clone();
        let mut concl = eq_hyp.prop.clone();
        for witness in instantiation {
            concl = instantiate_forall(&concl, &witness).ok_or_else(|| {
                TacticError::DoesNotApply(
                    "rewrite: the equation has fewer binders than the match".to_string(),
                )
            })?;
            eq_proof = DerivationTree::new(
                concl.clone(),
                InferenceRule::UniversalInstTerm(witness),
                vec![eq_proof],
            );
        }
        let proof = crate::engine::leibniz_step(
            new_prop.clone(),
            &lhs,
            &rhs,
            eq_proof,
            target_hyp.proof.clone(),
        )
        .ok_or_else(|| {
            TacticError::DoesNotApply(format!(
                "rewrite {eq_name} at {hyp_name}: no `Eq_rec` motive abstracts exactly the \
                 occurrence this step carried, so there is no certificate to build and the \
                 step is refused rather than left for the trust door"
            ))
        })?;

        let idx = *self.open.front().ok_or(TacticError::NoOpenGoals)?;
        if let Node::Hole(goal) = &mut self.nodes[idx] {
            if let Some(h) = goal.hyps.iter_mut().find(|h| h.name == hyp_name) {
                h.prop = new_prop;
                h.proof = proof;
            }
        }
        Ok(self)
    }

    /// `rewrite h at hp` at the FIRST occurrence — the spelling a script writes.
    pub fn rewrite_hyp(&mut self, eq_name: &str, hyp_name: &str) -> Result<&mut Self, TacticError> {
        self.rewrite_hyp_at(eq_name, hyp_name, 0)
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
                let new_target = substitute_in_expr(&g.target, &step.from, &step.to);
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
                let sub = Goal {
                    hyps: g.hyps.clone(),
                    target: new_target,
                };
                self.refine(
                    g.target.clone(),
                    InferenceRule::Rewrite {
                        from: step.to,
                        to: step.from,
                    },
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
                let sub = Goal {
                    hyps: g.hyps.clone(),
                    target: iff.rhs,
                };
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
        if self.rfl().is_ok() {
            return Ok(self);
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

    /// `decide`: close the focused goal by evaluation, if it is a closed decidable
    /// proposition that evaluates TRUE — ground arithmetic (at ARBITRARY precision, the
    /// kernel's own), comparisons, `Bool` equalities, a ground DISEQUALITY, and ∧/∨/→/¬ over
    /// them. Mirrors Lean's `decide`. A false or open goal is declined.
    ///
    /// The two ways this declines are now two different messages. They used to be one — "not
    /// a closed decidable goal (or it is false)" — and that conflation is dangerous in one
    /// direction: a caller that reads `DoesNotApply` as "try another tactic" keeps searching
    /// past a goal that is actually FALSE, and may eventually close it from an unrelated
    /// contradiction by ex falso, producing a certified proof of a statement nobody asserted.
    /// `decide` knows the difference; now it says it.
    pub fn decide(&mut self) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        match crate::decide::decide_verdict(&g.target) {
            crate::decide::Verdict::True(tree) => {
                self.close(tree)?;
                Ok(self)
            }
            crate::decide::Verdict::False => Err(TacticError::DoesNotApply(format!(
                "decide: the goal is closed and decidable, and it is FALSE — `{}` evaluates \
                 false, so no tactic can prove it and a search that keeps going is searching \
                 for a proof of something untrue",
                g.target
            ))),
            crate::decide::Verdict::Undecided => Err(TacticError::DoesNotApply(format!(
                "decide: `{}` is not a CLOSED goal in the decidable fragment (ground Int/Bool \
                 arithmetic, comparisons, and ¬/∧/∨/→ over them) — its truth was not \
                 determined either way",
                g.target
            ))),
        }
    }

    /// `omega`: linear INTEGER arithmetic. When the in-scope `≤`/`<` hypotheses
    /// are jointly unsatisfiable over ℤ — using discreteness (`a < b ⟹ a+1 ≤ b`),
    /// the fact rational solvers lack — refute them and discharge ANY goal by
    /// ex-falso. Proves `x < y ∧ y < x+1 ⊢ Q`, which `linarith` cannot (that
    /// system is rationally satisfiable). Mirrors the refutation half of Lean's
    /// `omega`. Declines when the hypotheses have an integer model.
    /// `linarith`: refute the arithmetic hypotheses over the RATIONALS, by Fourier-Motzkin with
    /// a Farkas certificate — and decline anything that needs integrality.
    ///
    /// `linarith` was an ALIAS for `omega`, which tightens `a < b` to `a + 1 ≤ b`. That is sound
    /// over ℤ and **unsound over an ordered field**: `ProofExpr` is untyped, so nothing records the
    /// carrier, and a goal false over ℚ could be certified because the integer step found a
    /// refutation the rationals do not have. Routing here — `engine::cert_farkas`, the same
    /// rational core the backward chainer already uses, *without* the discreteness step — makes
    /// `linarith` mean what its name means. `omega` keeps the integer reasoning under its own name.
    pub fn linarith(&mut self) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        let known: Vec<(ProofExpr, DerivationTree)> = g
            .hyps
            .iter()
            .map(|h| (h.prop.clone(), h.proof.clone()))
            .collect();
        if !crate::omega_solve::has_arith_facts(&known) {
            return Err(TacticError::DoesNotApply(
                "linarith: no arithmetic (≤ / <) hypotheses in scope".to_string(),
            ));
        }
        match crate::engine::cert_farkas(&known) {
            Some(bot) => {
                let tree =
                    DerivationTree::new(g.target.clone(), InferenceRule::ExFalso, vec![bot]);
                self.close(tree)?;
                Ok(self)
            }
            None => Err(TacticError::DoesNotApply(
                "linarith: no rational Farkas certificate — the hypotheses are satisfiable over an \
                 ordered field. If the refutation needs integrality, that is `omega`, not `linarith`"
                    .to_string(),
            )),
        }
    }

    /// A DECISION PROCEDURE for the goal, not only a refutation of the hypotheses.
    ///
    /// Lean's `omega` negates the goal, adds it to the hypothesis set, and refutes; ours
    /// only ever refuted `g.hyps`, so `x ≤ 3 ⊢ x ≤ 4` — the shape most real uses have —
    /// was DECLINED with a message ("the integer hypotheses are satisfiable") that described
    /// the hypotheses when the problem was that there was no way to attack the goal at all.
    /// Three halves now, in order:
    ///
    /// 1. **Refute the hypotheses.** Unchanged, and still first: contradictory bounds
    ///    discharge ANY goal by ex falso.
    /// 2. **Prove the goal directly.** A ground comparison by computation; a `≤` goal by
    ///    chaining the in-scope `≤` facts (`le_trans`/`le_refl`); an `Int` EQUALITY goal
    ///    through the ring oracle, which is how `n + 0 = n` — an `omega` goal in Lean with no
    ///    hypotheses whatever — gets proved.
    /// 3. **Split on totality and refute the other side.** `le_total a b` is already in the
    ///    trusted base, and its own comment names this use. For a `≤` goal, take the goal
    ///    itself in the left branch and refute `hyps ∪ {b ≤ a}` in the right.
    ///
    /// Half 3 is deliberately the SOUND, axiom-free approximation of "negate the goal":
    /// `¬(a ≤ b)` is `b + 1 ≤ a` over ℤ, one unit stronger than the `b ≤ a` totality gives,
    /// and the extra unit needs an axiom (`¬(a ≤ b) → b + 1 ≤ a`, or antisymmetry) that the
    /// trusted base does not have. Adding one to widen a tactic would trade a visible
    /// capability gap for an invisible trust cost, so it is NOT done here; the fragment this
    /// reaches is "every goal whose negation-by-totality is refutable", which covers every
    /// goal with slack. The gate is likewise relaxed: an arithmetic GOAL admits the tactic
    /// even with no arithmetic hypothesis in scope, which is the no-hypothesis Lean case.
    pub fn omega(&mut self) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        let known: Vec<(ProofExpr, DerivationTree)> = g
            .hyps
            .iter()
            .map(|h| (h.prop.clone(), h.proof.clone()))
            .collect();
        let arith_goal = crate::omega_solve::is_arith_goal(&g.target);
        if !crate::omega_solve::has_arith_facts(&known) && !arith_goal {
            return Err(TacticError::DoesNotApply(format!(
                "omega: no arithmetic (≤ / <) hypotheses in scope and `{}` is not an \
                 arithmetic goal either, so there is nothing linear to decide",
                g.target
            )));
        }
        // 1. The rational core FIRST. `omega_close` returns `None` unless it tightened at
        // least one STRICT hypothesis, on the assumption that its caller already tried the
        // rationals — so a PURELY non-strict contradiction (`5 ≤ x`, `x ≤ 3`) was declined
        // with the message "the integer hypotheses are satisfiable" about a system that is
        // not. Integrality is a strengthening of the rational theory, never a weakening:
        // anything Farkas refutes over ℚ is refuted over ℤ too.
        if let Some(bot) =
            crate::engine::cert_farkas(&known).or_else(|| crate::omega_solve::omega_close(&known))
        {
            let tree = DerivationTree::new(g.target.clone(), InferenceRule::ExFalso, vec![bot]);
            self.close(tree)?;
            return Ok(self);
        }
        // 2 + 3. Goal-directed.
        if let Some(tree) = crate::omega_solve::omega_prove(&known, &g.target) {
            self.close(tree)?;
            return Ok(self);
        }
        Err(TacticError::DoesNotApply(format!(
            "omega: the hypotheses have an integer model, and `{}` does not follow from them \
             — neither directly nor by refuting its negation-by-totality",
            g.target
        )))
    }

    /// `calc h₁ h₂ …`: compose the NAMED order/equality facts into the goal — Lean's `calc`,
    /// whose job is a chain whose RELATION CHANGES along it.
    ///
    /// `calc` existed on no LOGOS surface. The pure-`=` chain had an expansion (three
    /// `rewrite`s), but a MIXED chain did not: the order graph walked `≤` edges alone, with no
    /// arm that could consume an equality, so a single `=` step anywhere made the chain
    /// unreachable. The composition now lives in `engine::cert_le_chain` — `≤` edges,
    /// discreteness-tightened `<` edges, and equality edges including their CONGRUENT images
    /// (`a = b` bridging `a + 1` to `b + 1`) — and this is its name on the surface.
    ///
    /// Naming the facts is the point, exactly as `simp only [h]` names its rule set: `auto`
    /// searches everything in scope, while a `calc` block says which steps, in which order,
    /// and fails if they do not compose. A goal `a ≤ d` composes through the chain; an
    /// equality goal composes through congruence closure over the named equations.
    ///
    /// The relation the chain CONCLUDES is `=` or `≤`. A strict `<` conclusion is not
    /// certifiable at all: of the eight `Int` order axioms none concludes a `lt` (`lt_succ_le`
    /// and `lt_add1_le` both take one as a premise), so the composition can state its result
    /// only in the tightened form `a + 1 ≤ d`. The missing primitive is the converse
    /// `le_succ_lt : Π a b. (a + 1) ≤ b → a < b`, one new `Int` axiom — named here rather than
    /// assumed, because widening a tactic by growing the trusted base is the one trade this
    /// campaign forbids.
    pub fn calc(&mut self, names: &[String]) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        let mut steps: Vec<(ProofExpr, DerivationTree)> = Vec::new();
        for name in names {
            let hyp = g
                .hyps
                .iter()
                .find(|h| &h.name == name)
                .ok_or_else(|| TacticError::NoSuchHypothesis(name.clone()))?;
            steps.push((hyp.prop.clone(), hyp.proof.clone()));
        }
        if steps.is_empty() {
            return Err(TacticError::DoesNotApply(
                "calc: name the steps of the chain — `calc h1 h2 h3`".to_string(),
            ));
        }
        // A step may be a QUANTIFIED equation. Lean writes the application into the block
        // (`_ = g b := h₂ b`); a named block has to find it, so each `∀x̄. lhs = rhs` is
        // E-matched at the chain's own terms and travels on as its ground instances.
        let steps = crate::crush::instantiate_steps(&steps, &g.target);
        if let Some((a, b)) = crate::engine::as_le_pair(&g.target) {
            if let Some(tree) = crate::engine::cert_le_chain(&steps, &a, &b) {
                self.close(tree)?;
                return Ok(self);
            }
            return Err(TacticError::DoesNotApply(format!(
                "calc: the named steps do not compose into `{}`",
                g.target
            )));
        }
        if let Some((a, b)) = crate::omega_solve::as_lt_pair(&g.target) {
            if let Some(tree) = crate::engine::cert_lt_chain(&steps, &a, &b) {
                self.close(tree)?;
                return Ok(self);
            }
            if !crate::engine::strict_conclusions_available() {
                return Err(TacticError::DoesNotApply(format!(
                    "calc: a STRICT conclusion `{}` has no certifiable image — of the `Int` \
                     order axioms none concludes a `lt` (`lt_succ_le` and `lt_add1_le` both \
                     take one as a premise). The chain composes; only the last step is \
                     missing, and it is the axiom `le_succ_lt : Π a b. (a + 1) ≤ b → a < b` \
                     (or, better, DEFINING `lt a b := le (add a 1) b`, which removes two \
                     axioms instead of adding one). State the goal as `a + 1 ≤ b` meanwhile",
                    g.target
                )));
            }
            return Err(TacticError::DoesNotApply(format!(
                "calc: the named steps do not compose into `{}`",
                g.target
            )));
        }
        if let ProofExpr::Identity(l, r) = &g.target {
            if let Some(tree) = crate::engine::cert_equality_chain(&steps, l, r) {
                self.close(tree)?;
                return Ok(self);
            }
            return Err(TacticError::DoesNotApply(format!(
                "calc: the named equations do not chain into `{}`",
                g.target
            )));
        }
        Err(TacticError::DoesNotApply(format!(
            "calc composes an `=` or `≤` chain; `{}` is neither",
            g.target
        )))
    }

    /// `by_rule R`: discharge the focused goal in ONE step with the NAMED inference rule,
    /// from the facts already in scope.
    ///
    /// The language AST has carried a public `ProofStrategy::ByRule(name)` variant — with
    /// `Proof: ModusPonens.` as its own documented example — that nothing in the workspace
    /// constructs and no driver has an arm for, because the proof layer offered no way to
    /// apply a rule by its name. This is that door.
    ///
    /// It is a real tactic and not a label, and three things make it one:
    ///
    /// * A name that is not a rule is a NAME error, listing the rules that are — a translator
    ///   handing us a Lean rule name needs to know which of the two kinds of failure it hit.
    /// * A rule that is declared but has no certifier arm reports
    ///   [`InferenceRule::uncertifiable_reason`] — the list exists precisely so a PRODUCER can
    ///   consult it before emitting a tree, and this is the one place in the crate that does.
    /// * The node is certified BEFORE success is reported. `by_rule R` succeeds only when the
    ///   kernel agrees that R, applied to facts in scope, proves this goal — so it can never
    ///   hand `qed` a derivation the trust door must refuse.
    ///
    /// The premises are drawn from the hypotheses in scope and nowhere else: `by_rule` is one
    /// step, not a search. A goal needing a subproof is `auto`'s, or is written out.
    pub fn by_rule(&mut self, name: &str) -> Result<&mut Self, TacticError> {
        /// The widest rule in the enum takes three premises (`DisjunctionElim`: the
        /// disjunction and one implication per side), so a one-step application never needs
        /// a longer tuple.
        const BY_RULE_MAX_ARITY: usize = 3;
        let g = self.focused_goal()?;
        let Some(rule) = InferenceRule::from_name(name) else {
            let known: Vec<String> = InferenceRule::NAMEABLE
                .iter()
                .map(|r| r.canonical_name())
                .collect();
            return Err(TacticError::DoesNotApply(format!(
                "by_rule: `{name}` is not an inference rule that can be applied from a name \
                 alone. A rule carrying an ARGUMENT (a rewrite's equation, an instantiation's \
                 witness, an induction's variable) has its own tactic — `rewrite`, `exists`, \
                 `induction`. The nameable rules are: {}",
                known.join(", ")
            )));
        };
        if let Some(reason) = rule.uncertifiable_reason() {
            return Err(TacticError::DoesNotApply(format!(
                "by_rule {}: {reason}",
                rule.canonical_name()
            )));
        }
        let props: Vec<ProofExpr> = g.hyps.iter().map(|h| h.prop.clone()).collect();
        let pool: Vec<DerivationTree> = g.hyps.iter().map(|h| h.proof.clone()).collect();
        let checker = crate::verify::ScopedChecker::new(&props, &g.target);
        // Candidate premise tuples, shortest first: a leaf rule is one question, and a rule
        // that needs two facts is not asked about until the cheaper readings are exhausted.
        // The budget bounds the cubic tier, which is the only one that can grow.
        const BUDGET: usize = 512;
        let mut asked = 0usize;
        let mut arity_tuples: Vec<Vec<DerivationTree>> = vec![Vec::new()];
        for _ in 0..=BY_RULE_MAX_ARITY {
            for combo in &arity_tuples {
                if asked >= BUDGET {
                    break;
                }
                asked += 1;
                let tree = DerivationTree::new(g.target.clone(), rule.clone(), combo.clone());
                if checker.certifies(tree.clone()) {
                    self.close(tree)?;
                    return Ok(self);
                }
            }
            arity_tuples = arity_tuples
                .iter()
                .flat_map(|combo| {
                    pool.iter().map(move |p| {
                        let mut next = combo.clone();
                        next.push(p.clone());
                        next
                    })
                })
                .collect();
            if arity_tuples.len() > BUDGET {
                arity_tuples.truncate(BUDGET);
            }
        }
        Err(TacticError::DoesNotApply(format!(
            "by_rule {}: that rule does not discharge `{}` from the {} hypothes{} in scope",
            rule.canonical_name(),
            g.target,
            pool.len(),
            if pool.len() == 1 { "is" } else { "es" }
        )))
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
                self.close(tree)?;
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
    ///
    /// With the `verification` feature the chainer's last resort is the Z3 oracle, whose
    /// `OracleVerification` leaf attests a goal without a proof term. The chainer reaches
    /// it only after every certifiable strategy failed, so such a tree means `auto` found
    /// no proof: `close` refuses it and `auto` declines.
    pub fn auto(&mut self) -> Result<&mut Self, TacticError> {
        let g = self.focused_goal()?;
        let mut engine = BackwardChainer::new();
        // A tactic's hypotheses ARE kernel hypotheses: the trust door registers each as a
        // declaration whose type mentions the statement's free variables as context LOCALS.
        // So a step that used one AT AN INSTANCE would cite a proposition no context carries,
        // and `qed` would refuse a tree `auto` had already called a proof. Saying so here is
        // what makes the honest answer STRUCTURAL — in this reading the search cannot build
        // the step at all, so there is nothing for a later check to catch.
        engine.set_premise_variables(crate::PremiseVariables::Local);
        for h in &g.hyps {
            engine.add_axiom(h.prop.clone());
        }
        match engine.prove(g.target.clone()) {
            Ok(tree) => {
                self.close(tree)?;
                Ok(self)
            }
            Err(_) => Err(TacticError::AutoFailed),
        }
    }

    fn assemble(&self, idx: usize) -> Result<DerivationTree, TacticError> {
        match &self.nodes[idx] {
            Node::Done(tree) => Ok(tree.clone()),
            Node::Filled {
                conclusion,
                rule,
                children,
            } => {
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
    /// `linarith` (rational Fourier-Motzkin with a Farkas certificate) as a tactic value.
    pub fn linarith() -> Tactic {
        Box::new(|st: &mut ProofState| st.linarith().map(|_| ()))
    }
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
    /// `rewrite h at k` as a tactic value — the `k`-th occurrence, 0-based.
    pub fn rewrite_at(name: &str, occurrence: usize) -> Tactic {
        let name = name.to_string();
        Box::new(move |st: &mut ProofState| st.rewrite_at(&name, occurrence).map(|_| ()))
    }
    pub fn rewrite(name: &str) -> Tactic {
        let name = name.to_string();
        Box::new(move |st: &mut ProofState| st.rewrite(&name).map(|_| ()))
    }
    /// `rewrite h at hp` — the LOCATION form, rewriting a HYPOTHESIS rather than the goal.
    pub fn rewrite_hyp_at(eq: &str, hyp: &str, occurrence: usize) -> Tactic {
        let (eq, hyp) = (eq.to_string(), hyp.to_string());
        Box::new(move |st: &mut ProofState| st.rewrite_hyp_at(&eq, &hyp, occurrence).map(|_| ()))
    }
    /// `apply name` (modus ponens against a named rule) as a tactic value.
    pub fn apply(name: &str) -> Tactic {
        let name = name.to_string();
        Box::new(move |st: &mut ProofState| st.apply_named(&name).map(|_| ()))
    }
    /// `rfl` (close `t = t` by reflexivity) as a tactic value.
    pub fn rfl() -> Tactic {
        Box::new(|st: &mut ProofState| st.rfl().map(|_| ()))
    }
    /// `induction` (structural induction over `Nat`) as a tactic value.
    pub fn induction() -> Tactic {
        Box::new(|st: &mut ProofState| st.induction().map(|_| ()))
    }
    /// `induction_over` (generic structural induction over `ind_type`) as a tactic value.
    pub fn induction_over(ind_type: &str, ctors: Vec<super::CtorSpec>) -> Tactic {
        let ind_type = ind_type.to_string();
        Box::new(move |st: &mut ProofState| st.induction_over(&ind_type, ctors.clone()).map(|_| ()))
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
            Err(TacticError::DoesNotApply(
                "first: no alternative applied".to_string(),
            ))
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
    /// `any_goals t`: run `t` on every open goal, keeping the goals it left and the goals it
    /// declined alike, and succeed iff it succeeded on AT LEAST ONE.
    ///
    /// The permissive sibling of [`all_goals`], and the one Lean scripts actually reach for after a
    /// `constructor` splits a goal into branches only some of which `t` can discharge. Unlike
    /// `all_goals` it does not abort the whole combinator when `t` fails on one branch — that
    /// branch is simply carried forward unchanged.
    pub fn any_goals(t: Tactic) -> Tactic {
        Box::new(move |st: &mut ProofState| {
            let current: Vec<usize> = st.open.iter().copied().collect();
            let mut new_open = std::collections::VecDeque::new();
            let mut any_succeeded = false;
            for goal_idx in current {
                st.open = std::collections::VecDeque::from([goal_idx]);
                match t(st) {
                    Ok(()) => {
                        any_succeeded = true;
                        new_open.extend(st.open.iter().copied());
                    }
                    // `t` declined this branch: keep the goal exactly as it was and move on.
                    Err(_) => new_open.push_back(goal_idx),
                }
            }
            st.open = new_open;
            if any_succeeded {
                Ok(())
            } else {
                Err(TacticError::DoesNotApply(
                    "any_goals: the tactic applied to no open goal".to_string(),
                ))
            }
        })
    }

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

    /// `t1 <;> t2`: run `t1` on the focused goal, then `t2` on exactly the goals `t1`
    /// PRODUCED — not on every goal that happens to be open.
    ///
    /// The difference is invisible when `<;>` is the first branching step, which almost every
    /// textbook use is, and that is why running `all_goals` here survived a large test suite.
    /// It is not invisible after an earlier step left a goal waiting: `split . left <;>
    /// assumption` must leave the `split`'s right conjunct alone. When `t2` happened to
    /// discharge that goal too, the script succeeded one goal further along than the proof
    /// believed; when it did not, the failure was reported for a goal the source never asked
    /// it to look at. A goal `t1` produced is one that was not open before it ran — node
    /// indices only ever grow, so the two sets are told apart exactly.
    pub fn then_all(t1: Tactic, t2: Tactic) -> Tactic {
        Box::new(move |st: &mut ProofState| {
            let before: std::collections::HashSet<usize> = st.open.iter().copied().collect();
            t1(st)?;
            let after: Vec<usize> = st.open.iter().copied().collect();
            let mut new_open = std::collections::VecDeque::new();
            for goal_idx in after {
                if before.contains(&goal_idx) {
                    new_open.push_back(goal_idx);
                    continue;
                }
                st.open = std::collections::VecDeque::from([goal_idx]);
                t2(st)?;
                new_open.extend(st.open.iter().copied());
            }
            st.open = new_open;
            Ok(())
        })
    }

    /// `· t` / `focus t` / `next t`: run `t` on the focused goal ALONE, and make it an error
    /// for the block to leave that goal open.
    ///
    /// A script is otherwise a flat list of chains, and because subgoals are pushed to the
    /// front, stripping Lean's bullets and concatenating the blocks WORKS — right up until a
    /// block leaves a goal open or closes two, at which point the blocks silently misalign
    /// and, with generic enough tactics, the proof still succeeds having proved the goals in
    /// a pairing the source never specified. The bullet is what makes that impossible, so it
    /// is the ERROR, not the focusing, that is the capability here.
    pub fn focus(t: Tactic) -> Tactic {
        Box::new(move |st: &mut ProofState| {
            let original: std::collections::VecDeque<usize> = st.open.clone();
            let Some(&idx) = st.open.front() else {
                return Err(TacticError::NoOpenGoals);
            };
            let rest: std::collections::VecDeque<usize> = st.open.iter().skip(1).copied().collect();
            st.open = std::collections::VecDeque::from([idx]);
            match t(st) {
                Ok(()) if st.open.is_empty() => {
                    st.open = rest;
                    Ok(())
                }
                Ok(()) => {
                    let left = st.open.len();
                    st.open = original;
                    Err(TacticError::DoesNotApply(format!(
                        "focus: the block left {left} goal(s) open — a focused block must \
                         close exactly the goal it opened"
                    )))
                }
                Err(e) => {
                    st.open = original;
                    Err(e)
                }
            }
        })
    }

    /// `by_contra h` as a tactic value.
    pub fn by_contra(name: &str) -> Tactic {
        let name = name.to_string();
        Box::new(move |st: &mut ProofState| st.by_contra(&name).map(|_| ()))
    }
    /// `ring` (commutative-ring normalisation) as a tactic value.
    pub fn ring() -> Tactic {
        Box::new(|st: &mut ProofState| st.ring().map(|_| ()))
    }
    /// `exact <term>` as a tactic value.
    pub fn exact_term(term: super::PTerm) -> Tactic {
        Box::new(move |st: &mut ProofState| st.exact_term(&term).map(|_| ()))
    }
    /// `refine <term>` as a tactic value.
    pub fn refine(term: super::PTerm) -> Tactic {
        Box::new(move |st: &mut ProofState| st.refine_term(&term).map(|_| ()))
    }
    /// `have name from <term>` as a tactic value.
    pub fn have(name: &str, term: super::PTerm) -> Tactic {
        let name = name.to_string();
        Box::new(move |st: &mut ProofState| st.have_term(&name, &term).map(|_| ()))
    }
    /// `exists <term>` as a tactic value, with the witness resolved against the goal's scope.
    pub fn exists_term(term: super::PTerm) -> Tactic {
        Box::new(move |st: &mut ProofState| {
            let witness = st.value_term(&term)?;
            st.exists(witness).map(|_| ())
        })
    }
    /// `cases name with a b` as a tactic value.
    pub fn cases_with(name: &str, names: Vec<String>) -> Tactic {
        let name = name.to_string();
        Box::new(move |st: &mut ProofState| st.cases_with(&name, &names).map(|_| ()))
    }
    /// `by_rule R` as a tactic value — discharge the goal with the NAMED inference rule.
    pub fn by_rule(name: &str) -> Tactic {
        let name = name.to_string();
        Box::new(move |st: &mut ProofState| st.by_rule(&name).map(|_| ()))
    }

    /// `calc h1 h2 …` as a tactic value — compose the named chain.
    pub fn calc(names: Vec<String>) -> Tactic {
        Box::new(move |st: &mut ProofState| st.calc(&names).map(|_| ()))
    }
    /// `induction name` as a tactic value.
    pub fn induction_on(name: &str) -> Tactic {
        let name = name.to_string();
        Box::new(move |st: &mut ProofState| st.induction_on(&name).map(|_| ()))
    }
    /// `simp only [a, b]` as a tactic value — the rule set is exactly the named hypotheses.
    pub fn simp_only(names: Vec<String>) -> Tactic {
        Box::new(move |st: &mut ProofState| {
            let set = st.named_simp_set(&names)?;
            st.simp(&set).map(|_| ())
        })
    }
    /// `simp [a, b]` as a tactic value — Lean's "these AS WELL", which for a default set
    /// that is already the whole scope adds nothing. The names are still resolved, so a
    /// typo is an error rather than a silently ignored rule.
    pub fn simp_with(names: Vec<String>) -> Tactic {
        Box::new(move |st: &mut ProofState| {
            st.named_simp_set(&names)?;
            let set = st.scope_simp_set();
            st.simp(&set).map(|_| ())
        })
    }
}
