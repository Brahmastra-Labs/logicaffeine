//! Tactics over the kernel's own goals: a goal state of metavariables, outside the trusted base.
//!
//! A goal is a metavariable with the hypotheses in scope and the type it must inhabit. A tactic
//! assigns the focused goal a term that may mention new metavariables; those are the new goals,
//! front first, as Lean's tactic framework builds a proof. Nothing here is trusted: a tactic only
//! builds terms, and [`GoalState::qed`] instantiates the statement's proof, requires it closed, and
//! has the kernel check it against the statement exactly as a term written by hand. A tactic that
//! cannot do what it is asked leaves the state as it was.
//!
//! Hypotheses are locals (`Term::Var`). A term handed to a tactic reads a global named like a
//! hypothesis of the focused goal as that hypothesis ([`GoalState::read`]), the scoping a proof
//! script expects.

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use crate::context::{Context, SimpRule};
use crate::elaborate::{self, MetaCtx};
use crate::error::KernelError;
use crate::term::Term;
use crate::type_checker::{
    alpha_equivalent, def_eq, demanded_type_head, infer_sort, infer_type, is_subtype, substitute,
    TypeHead,
};

/// One open goal as a reader sees it: the hypotheses in scope, outermost first, and the target,
/// every solved metavariable substituted.
#[derive(Debug, Clone, PartialEq)]
pub struct Goal {
    pub hyps: Vec<(String, Term)>,
    pub target: Term,
}

impl fmt::Display for Goal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (name, ty) in &self.hyps {
            writeln!(f, "{name} : {}", Shown(ty))?;
        }
        write!(f, "⊢ {}", Shown(&self.target))
    }
}

/// Why a tactic did nothing, or why there is no proof yet.
#[derive(Debug)]
pub enum TacticError {
    /// Every goal is closed.
    NoGoals,
    /// The tactic does not apply to the focused goal.
    DoesNotApply {
        tactic: &'static str,
        goal: Goal,
        why: String,
    },
    /// `qed` with goals still open.
    GoalsRemain(Vec<Goal>),
    /// The kernel refused a term: the statement at [`GoalState::new`], the proof at
    /// [`GoalState::qed`].
    Kernel(KernelError),
}

impl fmt::Display for TacticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TacticError::NoGoals => write!(f, "no goals: the proof is complete"),
            TacticError::DoesNotApply { tactic, goal, why } => {
                write!(f, "`{tactic}` does not apply: {why}\nin the goal\n{goal}")
            }
            TacticError::GoalsRemain(goals) => {
                write!(f, "{} goal(s) remain:", goals.len())?;
                for goal in goals {
                    write!(f, "\n{goal}")?;
                }
                Ok(())
            }
            TacticError::Kernel(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TacticError {}

#[derive(Debug, Clone)]
struct Decl {
    hyps: Vec<(String, Term)>,
    target: Term,
}

/// One step of a tactic script (`by intro A x; exact (refl A x)`).
#[derive(Debug, Clone, PartialEq)]
pub enum TacticStep {
    Intro(Vec<String>),
    Exact(Term),
    Apply(Term),
    Assumption,
    Exfalso,
    Constructor,
    Left,
    Right,
    Exists(Term),
    Have(String, Term),
    Rfl,
    Change(Term),
    Rewrite(Term, bool),
    Induction(String, Vec<Vec<String>>),
    Cases(String, Vec<Vec<String>>),
    Refine(Term),
    Subst(String),
    Injection(String, Vec<String>),
    /// `calc [R₁ := p₁, R₂ := p₂, …]`: each step an equation and its proof.
    Calc(Vec<(Term, Term)>),
    Cc,
    Decide,
    /// `simp [l₁, …]`: rewrite by the lemmas (the context's `@[simp]` set when none is named).
    Simp(Vec<Term>),
}

impl fmt::Display for TacticStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let pattern = |cases: &[Vec<String>]| {
            let cases: Vec<String> = cases.iter().map(|c| c.join(" ")).collect();
            format!(" as [{}]", cases.join(" | "))
        };
        match self {
            TacticStep::Intro(names) => write!(f, "intro {}", names.join(" ")),
            TacticStep::Exact(t) => write!(f, "exact {}", Shown(t)),
            TacticStep::Apply(t) => write!(f, "apply {}", Shown(t)),
            TacticStep::Assumption => write!(f, "assumption"),
            TacticStep::Exfalso => write!(f, "exfalso"),
            TacticStep::Constructor => write!(f, "constructor"),
            TacticStep::Left => write!(f, "left"),
            TacticStep::Right => write!(f, "right"),
            TacticStep::Exists(t) => write!(f, "exists {}", Shown(t)),
            TacticStep::Have(n, t) => write!(f, "have {n} : {}", Shown(t)),
            TacticStep::Rfl => write!(f, "rfl"),
            TacticStep::Change(t) => write!(f, "change {}", Shown(t)),
            TacticStep::Rewrite(t, reverse) => {
                write!(f, "rw [{}{}]", if *reverse { "<- " } else { "" }, Shown(t))
            }
            TacticStep::Induction(x, cases) if cases.is_empty() => write!(f, "induction {x}"),
            TacticStep::Induction(x, cases) => write!(f, "induction {x}{}", pattern(cases)),
            TacticStep::Cases(x, cases) if cases.is_empty() => write!(f, "cases {x}"),
            TacticStep::Cases(x, cases) => write!(f, "cases {x}{}", pattern(cases)),
            TacticStep::Refine(t) => write!(f, "refine {}", Shown(t)),
            TacticStep::Subst(h) => write!(f, "subst {h}"),
            TacticStep::Injection(h, names) if names.is_empty() => write!(f, "injection {h}"),
            TacticStep::Injection(h, names) => write!(f, "injection {h} as [{}]", names.join(" ")),
            TacticStep::Calc(steps) => {
                let steps: Vec<String> = steps
                    .iter()
                    .map(|(r, p)| format!("{} := {}", Shown(r), Shown(p)))
                    .collect();
                write!(f, "calc [{}]", steps.join(", "))
            }
            TacticStep::Cc => write!(f, "cc"),
            TacticStep::Decide => write!(f, "decide"),
            TacticStep::Simp(lemmas) if lemmas.is_empty() => write!(f, "simp"),
            TacticStep::Simp(lemmas) => {
                let lemmas: Vec<String> = lemmas.iter().map(|l| Shown(l).to_string()).collect();
                write!(f, "simp [{}]", lemmas.join(", "))
            }
        }
    }
}

/// A script's failure: the step, by position and as written, and why; no step when the script ran
/// to its end and the proof is not done.
#[derive(Debug)]
pub struct ScriptError {
    pub step: usize,
    pub tactic: Option<TacticStep>,
    pub error: TacticError,
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.tactic {
            Some(tactic) => write!(f, "step {} (`{tactic}`): {}", self.step + 1, self.error),
            None => write!(f, "after the last step: {}", self.error),
        }
    }
}

impl std::error::Error for ScriptError {}

/// A proof under construction: the statement, the metavariables standing for its unproved
/// parts, and the goals still open.
#[derive(Debug, Clone)]
pub struct GoalState {
    ctx: Context,
    statement: Term,
    mctx: MetaCtx,
    decls: HashMap<String, Decl>,
    open: Vec<String>,
    root: String,
}

impl GoalState {
    /// A proof of `statement` in `ctx`, one goal open. `statement` must be a type.
    pub fn new(ctx: &Context, statement: &Term) -> Result<GoalState, TacticError> {
        infer_sort(ctx, statement).map_err(TacticError::Kernel)?;
        let mut st = GoalState {
            ctx: ctx.clone(),
            statement: statement.clone(),
            mctx: MetaCtx::new(),
            decls: HashMap::new(),
            open: Vec::new(),
            root: String::new(),
        };
        let root = st.declare(Vec::new(), statement.clone());
        st.root = root.clone();
        st.open.push(root);
        Ok(st)
    }

    /// The open goals, the focused one first.
    pub fn goals(&self) -> Vec<Goal> {
        self.open.iter().map(|m| self.view(m)).collect()
    }

    /// `term` as the focused goal reads it: a global named like one of its hypotheses is that
    /// hypothesis.
    pub fn read(&self, term: &Term) -> Term {
        match self.open.first() {
            Some(m) => localize(term, &self.decls[m].hyps),
            None => term.clone(),
        }
    }

    /// The finished proof: the statement's proof term with every goal's solution in place, closed
    /// and checked by the kernel against the statement.
    pub fn qed(&self) -> Result<Term, TacticError> {
        if !self.open.is_empty() {
            return Err(TacticError::GoalsRemain(self.goals()));
        }
        let proof = elaborate::instantiate(&Term::Var(self.root.clone()), &self.mctx);
        if elaborate::mentions_meta(&proof) {
            return Err(TacticError::Kernel(KernelError::CertificationError(
                format!(
                    "the proof still holds an unsolved metavariable: {}",
                    Shown(&proof)
                ),
            )));
        }
        let ty = infer_type(&self.ctx, &proof).map_err(TacticError::Kernel)?;
        if !is_subtype(&self.ctx, &ty, &self.statement) {
            return Err(TacticError::Kernel(KernelError::TypeMismatch {
                expected: Shown(&self.statement).to_string(),
                found: Shown(&ty).to_string(),
            }));
        }
        Ok(proof)
    }

    /// Run one step; a step that does not apply leaves the state as it found it, a multi-name
    /// `intro` included.
    pub fn step(&mut self, step: &TacticStep) -> Result<(), TacticError> {
        let saved = self.clone();
        let result = self.step_unguarded(step);
        if result.is_err() {
            *self = saved;
        }
        result
    }

    fn step_unguarded(&mut self, step: &TacticStep) -> Result<(), TacticError> {
        match step {
            TacticStep::Intro(xs) => {
                for x in xs {
                    self.intro(x)?;
                }
                Ok(())
            }
            TacticStep::Exact(t) => self.exact(t),
            TacticStep::Apply(t) => self.apply(t),
            TacticStep::Assumption => self.assumption(),
            TacticStep::Exfalso => self.exfalso(),
            TacticStep::Constructor => self.constructor(),
            TacticStep::Left => self.left(),
            TacticStep::Right => self.right(),
            TacticStep::Exists(t) => self.exists(t),
            TacticStep::Have(n, t) => self.have(n, t),
            TacticStep::Rfl => self.rfl(),
            TacticStep::Change(t) => self.change(t),
            TacticStep::Rewrite(t, reverse) => self.rewrite(t, *reverse),
            TacticStep::Induction(x, cases) => {
                let refs: Vec<Vec<&str>> = cases
                    .iter()
                    .map(|c| c.iter().map(String::as_str).collect())
                    .collect();
                let slices: Vec<&[&str]> = refs.iter().map(Vec::as_slice).collect();
                self.induction(x, &slices)
            }
            TacticStep::Cases(x, cases) => {
                let refs: Vec<Vec<&str>> = cases
                    .iter()
                    .map(|c| c.iter().map(String::as_str).collect())
                    .collect();
                let slices: Vec<&[&str]> = refs.iter().map(Vec::as_slice).collect();
                self.cases(x, &slices)
            }
            TacticStep::Refine(t) => self.refine(t),
            TacticStep::Subst(h) => self.subst(h),
            TacticStep::Injection(h, names) => {
                let names: Vec<&str> = names.iter().map(String::as_str).collect();
                self.injection(h, &names)
            }
            TacticStep::Calc(steps) => self.calc(steps),
            TacticStep::Cc => self.cc(),
            TacticStep::Decide => self.decide(),
            TacticStep::Simp(lemmas) => self.simp(lemmas),
        }
    }

    /// Run a script, step by step; the first step that does not apply stops it, the state left
    /// as that step found it.
    pub fn run(&mut self, script: &[TacticStep]) -> Result<(), ScriptError> {
        for (i, step) in script.iter().enumerate() {
            self.step(step).map_err(|error| ScriptError {
                step: i,
                tactic: Some(step.clone()),
                error,
            })?;
        }
        Ok(())
    }

    /// `intro x`: a goal `Π(y : A). B` becomes `B[y := x]` with the hypothesis `x : A`.
    pub fn intro(&mut self, name: &str) -> Result<(), TacticError> {
        let (m, goal) = self.focus()?;
        if goal.hyps.iter().any(|(n, _)| n == name) {
            return Err(refuse(
                "intro",
                &goal,
                format!("`{name}` already names a hypothesis"),
            ));
        }
        let ctx = self.scope(&goal.hyps);
        let Term::Pi {
            param,
            param_type,
            body_type,
        } = demanded_type_head(&ctx, goal.target.clone(), TypeHead::Function)
        else {
            return Err(refuse(
                "intro",
                &goal,
                "the goal is not a function type".to_string(),
            ));
        };
        let body = if param == name {
            *body_type
        } else {
            substitute(&body_type, &param, &Term::Var(name.to_string()))
        };
        let mut hyps = goal.hyps.clone();
        hyps.push((name.to_string(), (*param_type).clone()));
        let inner = self.declare(hyps, body);
        let proof = Term::Lambda {
            param: name.to_string(),
            param_type,
            body: Box::new(Term::Var(inner.clone())),
        };
        self.close(&m, proof, vec![inner]);
        Ok(())
    }

    /// `exact e`: close the goal with `e`, whose holes and implicit arguments unification fills.
    pub fn exact(&mut self, term: &Term) -> Result<(), TacticError> {
        let (m, goal) = self.focus()?;
        let ctx = self.scope(&goal.hyps);
        let term = localize(term, &goal.hyps);
        let mut mctx = self.mctx.clone();
        let (value, ty) =
            typed(&ctx, &mut mctx, &goal.hyps, &term, Some(&goal.target)).map_err(|e| {
                refuse(
                    "exact",
                    &goal,
                    format!("{} does not type: {e}", Shown(&term)),
                )
            })?;
        elaborate::unify_in(&ctx, &mut mctx, &goal.hyps, &ty, &goal.target);
        let (value, ty, target) = (
            elaborate::instantiate(&value, &mctx),
            elaborate::instantiate(&ty, &mctx),
            elaborate::instantiate(&goal.target, &mctx),
        );
        if elaborate::mentions_meta(&value) {
            return Err(refuse(
                "exact",
                &goal,
                format!("{} leaves a hole unsolved", Shown(&value)),
            ));
        }
        if !is_subtype(&ctx, &ty, &target) {
            return Err(refuse(
                "exact",
                &goal,
                format!(
                    "the goal is {}, and {} has type {}",
                    Shown(&target),
                    Shown(&value),
                    Shown(&ty)
                ),
            ));
        }
        self.mctx = mctx;
        self.close(&m, value, Vec::new());
        Ok(())
    }

    /// `apply f`: unify `f`'s conclusion with the goal; each premise of `f` unification does not
    /// determine becomes a goal, in order.
    pub fn apply(&mut self, term: &Term) -> Result<(), TacticError> {
        let term = self.read(term);
        self.apply_term("apply", term)
    }

    /// `assumption`: close the goal with a hypothesis whose type it is.
    pub fn assumption(&mut self) -> Result<(), TacticError> {
        let (m, goal) = self.focus()?;
        let ctx = self.scope(&goal.hyps);
        let Some((name, _)) = goal
            .hyps
            .iter()
            .rev()
            .find(|(_, ty)| is_subtype(&ctx, ty, &goal.target))
        else {
            return Err(refuse(
                "assumption",
                &goal,
                "no hypothesis states the goal".to_string(),
            ));
        };
        let proof = Term::Var(name.clone());
        self.close(&m, proof, Vec::new());
        Ok(())
    }

    /// `exfalso`: prove the goal from `False`, by `False`'s elimination with no case.
    pub fn exfalso(&mut self) -> Result<(), TacticError> {
        let (m, goal) = self.focus()?;
        let falsity = Term::Global("False".into());
        let inner = self.declare(goal.hyps.clone(), falsity.clone());
        let proof = Term::Match {
            discriminant: Box::new(Term::Var(inner.clone())),
            motive: Box::new(Term::Lambda {
                param: "_".into(),
                param_type: Box::new(falsity),
                body: Box::new(goal.target.clone()),
            }),
            cases: Vec::new(),
        };
        self.close(&m, proof, vec![inner]);
        Ok(())
    }

    /// `constructor`: apply the first constructor of the goal's family that applies.
    pub fn constructor(&mut self) -> Result<(), TacticError> {
        self.constructor_among("constructor", None)
    }

    /// `left`: apply the first of the two constructors of the goal's family.
    pub fn left(&mut self) -> Result<(), TacticError> {
        self.constructor_among("left", Some(0))
    }

    /// `right`: apply the second of the two constructors of the goal's family.
    pub fn right(&mut self) -> Result<(), TacticError> {
        self.constructor_among("right", Some(1))
    }

    /// `exists w`: the goal's family has one constructor, applied to the family's parameters and
    /// the witness `w`; its remaining premises are the goals (`Ex A P` leaves `P w`).
    pub fn exists(&mut self, witness: &Term) -> Result<(), TacticError> {
        let (_, goal) = self.focus()?;
        let ctx = self.scope(&goal.hyps);
        let w = localize(witness, &goal.hyps);
        let (ind, args) = self.goal_family("exists", &ctx, &goal)?;
        let constructors = ctx.get_constructors(&ind);
        let [(constructor, _)] = constructors.as_slice() else {
            return Err(refuse(
                "exists",
                &goal,
                format!("`{ind}` has {} constructors, not one", constructors.len()),
            ));
        };
        let np = ctx.inductive_num_params(&ind).min(args.len());
        let mut parts = args[..np].to_vec();
        parts.push(w);
        let term = apps(Term::Global(constructor.to_string()), &parts);
        self.apply_term("exists", term)
    }

    /// `have h : A`: prove `A` first, then the goal with the hypothesis `h : A`.
    pub fn have(&mut self, name: &str, fact: &Term) -> Result<(), TacticError> {
        let (m, goal) = self.focus()?;
        if goal.hyps.iter().any(|(n, _)| n == name) {
            return Err(refuse(
                "have",
                &goal,
                format!("`{name}` already names a hypothesis"),
            ));
        }
        let ctx = self.scope(&goal.hyps);
        let fact = localize(fact, &goal.hyps);
        infer_sort(&ctx, &fact).map_err(|e| {
            refuse(
                "have",
                &goal,
                format!("{} is not a type: {e}", Shown(&fact)),
            )
        })?;
        let first = self.declare(goal.hyps.clone(), fact.clone());
        let mut hyps = goal.hyps.clone();
        hyps.push((name.to_string(), fact.clone()));
        let then = self.declare(hyps, goal.target.clone());
        let proof = Term::App(
            Box::new(Term::Lambda {
                param: name.to_string(),
                param_type: Box::new(fact),
                body: Box::new(Term::Var(then.clone())),
            }),
            Box::new(Term::Var(first.clone())),
        );
        self.close(&m, proof, vec![first, then]);
        Ok(())
    }

    fn constructor_among(
        &mut self,
        tactic: &'static str,
        only: Option<usize>,
    ) -> Result<(), TacticError> {
        let (_, goal) = self.focus()?;
        let ctx = self.scope(&goal.hyps);
        let (ind, _) = self.goal_family(tactic, &ctx, &goal)?;
        let constructors: Vec<String> = ctx
            .get_constructors(&ind)
            .iter()
            .map(|(n, _)| n.to_string())
            .collect();
        let candidates = match only {
            Some(i) if constructors.len() == 2 => vec![constructors[i].clone()],
            Some(_) => {
                return Err(refuse(
                    tactic,
                    &goal,
                    format!("`{ind}` has {} constructors, not two", constructors.len()),
                ))
            }
            None => constructors,
        };
        let mut last = refuse(tactic, &goal, format!("`{ind}` has no constructor"));
        for c in candidates {
            match self.apply_term(tactic, Term::Global(c)) {
                Ok(()) => return Ok(()),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// The inductive family the goal is an instance of, and its arguments.
    fn goal_family(
        &self,
        tactic: &'static str,
        ctx: &Context,
        goal: &Goal,
    ) -> Result<(String, Vec<Term>), TacticError> {
        let family = demanded_type_head(ctx, goal.target.clone(), TypeHead::Inductive);
        let (head, args) = spine(&family);
        match head {
            Term::Global(n) | Term::Const { name: n, .. } if ctx.is_inductive(n) => {
                Ok((n.clone(), args))
            }
            _ => Err(refuse(
                tactic,
                goal,
                "the goal is not an instance of an inductive family".to_string(),
            )),
        }
    }

    fn apply_term(&mut self, tactic: &'static str, term: Term) -> Result<(), TacticError> {
        let (m, goal) = self.focus()?;
        let ctx = self.scope(&goal.hyps);
        let mut mctx = self.mctx.clone();
        let (f, f_ty) = typed(&ctx, &mut mctx, &goal.hyps, &term, None).map_err(|e| {
            refuse(
                tactic,
                &goal,
                format!("{} does not type: {e}", Shown(&term)),
            )
        })?;
        let f_ty = elaborate::instantiate(&f_ty, &mctx);
        let (f_arity, target_arity) = (pi_arity(&ctx, &f_ty), pi_arity(&ctx, &goal.target));
        let Some(n) = f_arity.checked_sub(target_arity) else {
            return Err(refuse(
                tactic,
                &goal,
                format!(
                    "{} : {} has fewer premises than the goal",
                    Shown(&f),
                    Shown(&f_ty)
                ),
            ));
        };
        let mut ty = f_ty;
        let mut args = Vec::with_capacity(n);
        let mut premises = Vec::with_capacity(n);
        for _ in 0..n {
            let Term::Pi {
                param,
                param_type,
                body_type,
            } = demanded_type_head(&ctx, ty, TypeHead::Function)
            else {
                unreachable!("`pi_arity` counted these premises");
            };
            let a = mctx.fresh();
            let Term::Var(name) = &a else {
                unreachable!("a metavariable is a `Var`")
            };
            premises.push((name.clone(), *param_type));
            ty = substitute(&body_type, &param, &a);
            args.push(a);
        }
        if !elaborate::unify_in(&ctx, &mut mctx, &goal.hyps, &ty, &goal.target) {
            let ty = elaborate::instantiate(&ty, &mctx);
            return Err(refuse(
                tactic,
                &goal,
                format!(
                    "{}'s conclusion {} does not unify with the goal",
                    Shown(&f),
                    Shown(&ty)
                ),
            ));
        }
        self.mctx = mctx;
        let mut subgoals = Vec::with_capacity(premises.len());
        for (name, target) in premises {
            self.decls.insert(
                name.clone(),
                Decl {
                    hyps: goal.hyps.clone(),
                    target,
                },
            );
            subgoals.push(name);
        }
        let proof = args
            .into_iter()
            .fold(f, |acc, a| Term::App(Box::new(acc), Box::new(a)));
        self.close(&m, proof, subgoals);
        Ok(())
    }

    /// `rfl`: close an equation whose two sides convert.
    pub fn rfl(&mut self) -> Result<(), TacticError> {
        let (m, goal) = self.focus()?;
        let ctx = self.scope(&goal.hyps);
        let Some(eq) = equation(&ctx, &goal.target) else {
            return Err(refuse(
                "rfl",
                &goal,
                "the goal is not an equation".to_string(),
            ));
        };
        let proof = apps(eq.refl(), &[eq.carrier.clone(), eq.lhs.clone()]);
        match infer_type(&ctx, &proof) {
            Ok(ty) if is_subtype(&ctx, &ty, &goal.target) => {}
            _ => {
                return Err(refuse(
                    "rfl",
                    &goal,
                    format!("{} and {} do not convert", Shown(&eq.lhs), Shown(&eq.rhs)),
                ))
            }
        }
        self.close(&m, proof, Vec::new());
        Ok(())
    }

    /// `change T`: replace the goal by a target it converts with.
    pub fn change(&mut self, target: &Term) -> Result<(), TacticError> {
        let (m, goal) = self.focus()?;
        let ctx = self.scope(&goal.hyps);
        let new = localize(target, &goal.hyps);
        infer_sort(&ctx, &new).map_err(|e| {
            refuse(
                "change",
                &goal,
                format!("{} is not a type: {e}", Shown(&new)),
            )
        })?;
        if !def_eq(&ctx, &new, &goal.target) {
            return Err(refuse(
                "change",
                &goal,
                format!("{} does not convert with the goal", Shown(&new)),
            ));
        }
        let inner = self.declare(goal.hyps.clone(), new);
        self.close(&m, Term::Var(inner.clone()), vec![inner]);
        Ok(())
    }

    /// `rw [h]` (`reverse`: `rw [← h]`): with `h : Eq A l r`, replace every occurrence of `l` (of
    /// `r`) in the goal by `r` (by `l`), occurrences under binders included, and close the goal by
    /// transport along `h`.
    pub fn rewrite(&mut self, eq_proof: &Term, reverse: bool) -> Result<(), TacticError> {
        let (m, goal) = self.focus()?;
        let ctx = self.scope(&goal.hyps);
        let h = localize(eq_proof, &goal.hyps);
        let h_ty = infer_type(&ctx, &h).map_err(|e| {
            refuse(
                "rewrite",
                &goal,
                format!("{} does not type: {e}", Shown(&h)),
            )
        })?;
        let Some(eq) = equation(&ctx, &h_ty) else {
            return Err(refuse(
                "rewrite",
                &goal,
                format!("{} : {} is not an equation", Shown(&h), Shown(&h_ty)),
            ));
        };
        let (from, to) = if reverse {
            (&eq.rhs, &eq.lhs)
        } else {
            (&eq.lhs, &eq.rhs)
        };
        let x = fresh_name("x", &ctx, &goal.hyps, &[&goal.target, from, to]);
        let (body, found) = abstract_occurrences(&goal.target, from, &x);
        if !found {
            return Err(refuse(
                "rewrite",
                &goal,
                format!("{} does not occur in the goal", Shown(from)),
            ));
        }
        let new_target = substitute(&body, &x, to);
        let motive = Term::Lambda {
            param: x,
            param_type: Box::new(eq.carrier.clone()),
            body: Box::new(body),
        };
        // `Eq_rec A to motive (_ : motive to) from (e : Eq A to from) : motive from`.
        let back = if reverse {
            h
        } else {
            apps(
                Term::Global("Eq_sym".into()),
                &[eq.carrier.clone(), eq.lhs.clone(), eq.rhs.clone(), h],
            )
        };
        let inner = self.declare(goal.hyps.clone(), new_target.clone());
        let proof = apps(
            Term::Global("Eq_rec".into()),
            &[
                eq.carrier.clone(),
                to.clone(),
                motive,
                Term::Var(inner.clone()),
                from.clone(),
                back,
            ],
        );
        // Checked with the new goal standing in as a local of its type, before anything commits.
        let probe = ctx.extend(&inner, new_target);
        match infer_type(&probe, &proof) {
            Ok(ty) if is_subtype(&probe, &ty, &goal.target) => {}
            Ok(ty) => {
                self.decls.remove(&inner);
                return Err(refuse(
                    "rewrite",
                    &goal,
                    format!("the transport proves {}, not the goal", Shown(&ty)),
                ));
            }
            Err(e) => {
                self.decls.remove(&inner);
                return Err(refuse(
                    "rewrite",
                    &goal,
                    format!("the transport does not type: {e}"),
                ));
            }
        }
        self.close(&m, proof, vec![inner]);
        Ok(())
    }

    /// `induction x with | c₁ a… | c₂ b… …`: `x : I p̄ ī`, a hypothesis whose indices are distinct
    /// hypotheses, is eliminated by `I_rec`. The motive abstracts `x`, its indices and every
    /// hypothesis that depends on them (reintroduced in each case under its own name); each
    /// constructor's case is a goal whose fields and induction hypotheses are named by `names[i]`,
    /// in order, the rest named fresh.
    pub fn induction(&mut self, var: &str, names: &[&[&str]]) -> Result<(), TacticError> {
        let (m, goal) = self.focus()?;
        let ctx = self.scope(&goal.hyps);
        let Some((_, var_ty)) = goal.hyps.iter().find(|(n, _)| n == var).cloned() else {
            return Err(refuse(
                "induction",
                &goal,
                format!("`{var}` is not a hypothesis"),
            ));
        };
        let family = demanded_type_head(&ctx, var_ty, TypeHead::Inductive);
        let (head, args) = spine(&family);
        let ind = match head {
            Term::Global(n) | Term::Const { name: n, .. } if ctx.is_inductive(n) => n.clone(),
            _ => {
                return Err(refuse(
                    "induction",
                    &goal,
                    format!("`{var}` : {} is not of an inductive family", Shown(&family)),
                ))
            }
        };
        let np = ctx.inductive_num_params(&ind);
        let (params, indices) = args.split_at(np.min(args.len()));
        let mut index_vars: Vec<String> = Vec::new();
        for ix in indices {
            match ix {
                Term::Var(n)
                    if goal.hyps.iter().any(|(h, _)| h == n)
                        && !index_vars.contains(n)
                        && !params
                            .iter()
                            .any(|p| crate::type_checker::occurs_free(p, n)) =>
                {
                    index_vars.push(n.clone())
                }
                other => {
                    return Err(refuse(
                        "induction",
                        &goal,
                        format!(
                            "the index {} of `{var}`'s type is not a hypothesis of its own",
                            Shown(other)
                        ),
                    ))
                }
            }
        }
        let rec_name = format!("{ind}_rec");
        if ctx.get_global(&rec_name).is_none() && ctx.get_universe_poly(&rec_name).is_none() {
            return Err(refuse(
                "induction",
                &goal,
                format!("`{ind}` has no recursor `{rec_name}`"),
            ));
        }
        // What the motive abstracts: `x`, its indices, and every hypothesis depending on them.
        let mut abstracted: BTreeSet<String> = index_vars.iter().cloned().collect();
        abstracted.insert(var.to_string());
        let mut dependents: Vec<(String, Term)> = Vec::new();
        for (n, ty) in &goal.hyps {
            if abstracted.contains(n) {
                continue;
            }
            if abstracted
                .iter()
                .any(|a| crate::type_checker::occurs_free(ty, a))
            {
                abstracted.insert(n.clone());
                dependents.push((n.clone(), ty.clone()));
            }
        }
        let kept: Vec<(String, Term)> = goal
            .hyps
            .iter()
            .filter(|(n, _)| !abstracted.contains(n))
            .cloned()
            .collect();
        let generalized = dependents
            .iter()
            .rev()
            .fold(goal.target.clone(), |t, (n, ty)| Term::Pi {
                param: n.clone(),
                param_type: Box::new(ty.clone()),
                body_type: Box::new(t),
            });
        // The motive's telescope, read off the recursor's motive binder at these parameters.
        let rec = apps(Term::Global(rec_name), params);
        let rec_ty = infer_type(&ctx, &rec).map_err(|e| {
            refuse(
                "induction",
                &goal,
                format!("the recursor does not type at these parameters: {e}"),
            )
        })?;
        let Term::Pi {
            param: motive_param,
            param_type: motive_ty,
            body_type: after_motive,
        } = demanded_type_head(&ctx, rec_ty, TypeHead::Function)
        else {
            return Err(refuse(
                "induction",
                &goal,
                "the recursor takes no motive".to_string(),
            ));
        };
        let telescope = pi_telescope(&ctx, &motive_ty, index_vars.len() + 1);
        if telescope.len() != index_vars.len() + 1 {
            return Err(refuse(
                "induction",
                &goal,
                format!(
                    "the motive {} does not range over the family",
                    Shown(&motive_ty)
                ),
            ));
        }
        let mut avoid_terms: Vec<&Term> = vec![&goal.target];
        avoid_terms.extend(goal.hyps.iter().map(|(_, t)| t));
        let mut motive_names: Vec<String> = Vec::new();
        let mut motive_types: Vec<Term> = Vec::new();
        for (i, (bname, bty)) in telescope.iter().enumerate() {
            let mut bty = bty.clone();
            for (j, prev) in motive_names.iter().enumerate() {
                bty = substitute(&bty, &telescope[j].0, &Term::Var(prev.clone()));
            }
            let base = if i == index_vars.len() {
                "y"
            } else {
                bname.as_str()
            };
            let fresh = fresh_name_avoiding(base, &ctx, &goal.hyps, &avoid_terms, &motive_names);
            motive_names.push(fresh);
            motive_types.push(bty);
        }
        let mut body = generalized;
        for (ix, fresh) in index_vars.iter().zip(&motive_names) {
            body = substitute(&body, ix, &Term::Var(fresh.clone()));
        }
        body = substitute(
            &body,
            var,
            &Term::Var(motive_names[index_vars.len()].clone()),
        );
        let motive = motive_names
            .iter()
            .zip(&motive_types)
            .rev()
            .fold(body, |b, (n, t)| Term::Lambda {
                param: n.clone(),
                param_type: Box::new(t.clone()),
                body: Box::new(b),
            });
        let mut ty = substitute(&after_motive, &motive_param, &motive);
        // One goal per constructor: its minor premise's binders introduced under their names,
        // then the dependents reintroduced.
        let constructors = ctx.get_constructors(&ind).len();
        let mut minors = Vec::with_capacity(constructors);
        let mut cases = Vec::with_capacity(constructors);
        let mut new_decls: Vec<(String, Decl)> = Vec::new();
        let mut taken: Vec<String> = kept.iter().map(|(n, _)| n.clone()).collect();
        for i in 0..constructors {
            let Term::Pi {
                param,
                param_type,
                body_type,
            } = demanded_type_head(&ctx, ty, TypeHead::Function)
            else {
                return Err(refuse(
                    "induction",
                    &goal,
                    "the recursor has fewer minor premises than constructors".to_string(),
                ));
            };
            let minor_ty = beta_normal(&param_type);
            let given: &[&str] = names.get(i).copied().unwrap_or(&[]);
            let mut case_hyps = kept.clone();
            let mut binders: Vec<(String, Term)> = Vec::new();
            let mut t = minor_ty;
            let mut j = 0;
            let case_ctx_base = self.scope(&kept);
            loop {
                let case_ctx = binders
                    .iter()
                    .fold(case_ctx_base.clone(), |c, (n, ty)| c.extend(n, ty.clone()));
                let Term::Pi {
                    param: b,
                    param_type: bt,
                    body_type: rest,
                } = &t
                else {
                    break;
                };
                let _ = &case_ctx;
                let name = match given.get(j) {
                    Some(n) => {
                        if taken.iter().any(|t| t == n) {
                            return Err(refuse(
                                "induction",
                                &goal,
                                format!("`{n}` already names a hypothesis"),
                            ));
                        }
                        n.to_string()
                    }
                    None => fresh_name_avoiding(b, &ctx, &case_hyps, &[&goal.target], &taken),
                };
                taken.push(name.clone());
                let rest = if *b == name {
                    (**rest).clone()
                } else {
                    substitute(rest, b, &Term::Var(name.clone()))
                };
                binders.push((name.clone(), (**bt).clone()));
                case_hyps.push((name, (**bt).clone()));
                t = rest;
                j += 1;
            }
            if given.len() > binders.len() {
                return Err(refuse(
                    "induction",
                    &goal,
                    format!(
                        "case {} names {} hypotheses and has {}",
                        i + 1,
                        given.len(),
                        binders.len()
                    ),
                ));
            }
            // Reintroduce the dependents under their own names.
            for (dn, _) in &dependents {
                let Term::Pi {
                    param: b,
                    param_type: bt,
                    body_type: rest,
                } = &t
                else {
                    unreachable!("the motive's body is a Π over the dependents");
                };
                let rest = if b == dn {
                    (**rest).clone()
                } else {
                    substitute(rest, b, &Term::Var(dn.clone()))
                };
                binders.push((dn.clone(), (**bt).clone()));
                case_hyps.push((dn.clone(), (**bt).clone()));
                t = rest;
            }
            let case = self.fresh_meta_name();
            new_decls.push((
                case.clone(),
                Decl {
                    hyps: case_hyps,
                    target: t,
                },
            ));
            let minor = binders
                .iter()
                .rev()
                .fold(Term::Var(case.clone()), |b, (n, ty)| Term::Lambda {
                    param: n.clone(),
                    param_type: Box::new(ty.clone()),
                    body: Box::new(b),
                });
            ty = substitute(&body_type, &param, &minor);
            minors.push(minor);
            cases.push(case);
        }
        let mut proof = apps(apps(rec, &[motive]), &minors);
        for ix in &index_vars {
            proof = Term::App(Box::new(proof), Box::new(Term::Var(ix.clone())));
        }
        proof = Term::App(Box::new(proof), Box::new(Term::Var(var.to_string())));
        for (dn, _) in &dependents {
            proof = Term::App(Box::new(proof), Box::new(Term::Var(dn.clone())));
        }
        for (name, decl) in new_decls {
            self.decls.insert(name, decl);
        }
        self.close(&m, proof, cases);
        Ok(())
    }

    /// `cases x with | c₁ a… | …`: `x : I p̄ ī` is eliminated by an indexed `match`, which owes a
    /// case only for the constructors `ī` allows (`type_checker::constructors_the_indices_allow`):
    /// `cases h` with `h : Eq Nat (Succ a) Zero` closes the goal with no case at all, as Lean's
    /// `noConfusion` does. With indices that are hypotheses of their own, the motive abstracts them,
    /// `x`, and every hypothesis depending on them, as [`GoalState::induction`] does, without
    /// induction hypotheses. An index that is not a hypothesis, at a family a constructor can still
    /// produce it in, needs equations between indices and is refused.
    pub fn cases(&mut self, var: &str, names: &[&[&str]]) -> Result<(), TacticError> {
        let (m, goal) = self.focus()?;
        let ctx = self.scope(&goal.hyps);
        let Some((_, var_ty)) = goal.hyps.iter().find(|(n, _)| n == var).cloned() else {
            return Err(refuse(
                "cases",
                &goal,
                format!("`{var}` is not a hypothesis"),
            ));
        };
        let family = demanded_type_head(&ctx, var_ty, TypeHead::Inductive);
        let (head, args) = spine(&family);
        let ind = match head {
            Term::Global(n) | Term::Const { name: n, .. } if ctx.is_inductive(n) => n.clone(),
            _ => {
                return Err(refuse(
                    "cases",
                    &goal,
                    format!("`{var}` : {} is not of an inductive family", Shown(&family)),
                ))
            }
        };
        let np = ctx.inductive_num_params(&ind).min(args.len());
        let (params, indices) = args.split_at(np);
        let allowed = crate::type_checker::constructors_the_indices_allow(&ctx, &ind, np, &args);
        let index_vars: Option<Vec<String>> = {
            let mut vars: Vec<String> = Vec::new();
            let mut ok = true;
            for ix in indices {
                match ix {
                    Term::Var(n)
                        if goal.hyps.iter().any(|(h, _)| h == n)
                            && !vars.contains(n)
                            && !params
                                .iter()
                                .any(|p| crate::type_checker::occurs_free(p, n)) =>
                    {
                        vars.push(n.clone())
                    }
                    _ => ok = false,
                }
            }
            ok.then_some(vars)
        };
        if index_vars.is_none() && !allowed.is_empty() {
            let head = head.clone();
            return self.cases_by_equations(
                var, names, &m, &goal, &ctx, &head, params, indices, &allowed,
            );
        }
        let index_vars = index_vars.unwrap_or_default();
        let mut abstracted: BTreeSet<String> = index_vars.iter().cloned().collect();
        abstracted.insert(var.to_string());
        let mut dependents: Vec<(String, Term)> = Vec::new();
        for (n, ty) in &goal.hyps {
            if abstracted.contains(n) {
                continue;
            }
            if abstracted
                .iter()
                .any(|a| crate::type_checker::occurs_free(ty, a))
            {
                abstracted.insert(n.clone());
                dependents.push((n.clone(), ty.clone()));
            }
        }
        let kept: Vec<(String, Term)> = goal
            .hyps
            .iter()
            .filter(|(n, _)| !abstracted.contains(n))
            .cloned()
            .collect();
        let generalized = dependents
            .iter()
            .rev()
            .fold(goal.target.clone(), |t, (n, ty)| Term::Pi {
                param: n.clone(),
                param_type: Box::new(ty.clone()),
                body_type: Box::new(t),
            });
        // The motive ranges over the family's indices at these parameters and the scrutinee.
        let arity = infer_type(&ctx, &apps(head.clone(), params)).map_err(|e| {
            refuse(
                "cases",
                &goal,
                format!("the family does not type at these parameters: {e}"),
            )
        })?;
        let index_telescope = pi_telescope(&ctx, &arity, usize::MAX);
        let mut avoid: Vec<&Term> = vec![&goal.target];
        avoid.extend(goal.hyps.iter().map(|(_, t)| t));
        let mut motive_names: Vec<String> = Vec::new();
        let mut motive_types: Vec<Term> = Vec::new();
        for (bname, bty) in &index_telescope {
            let mut bty = bty.clone();
            for (j, prev) in motive_names.iter().enumerate() {
                bty = substitute(&bty, &index_telescope[j].0, &Term::Var(prev.clone()));
            }
            let fresh = fresh_name_avoiding(bname, &ctx, &goal.hyps, &avoid, &motive_names);
            motive_names.push(fresh);
            motive_types.push(bty);
        }
        let scrutinee_name = fresh_name_avoiding("y", &ctx, &goal.hyps, &avoid, &motive_names);
        let scrutinee_ty = apps(
            apps(head.clone(), params),
            &motive_names
                .iter()
                .map(|n| Term::Var(n.clone()))
                .collect::<Vec<_>>(),
        );
        let mut body = generalized;
        for (ix, fresh) in index_vars.iter().zip(&motive_names) {
            body = substitute(&body, ix, &Term::Var(fresh.clone()));
        }
        body = substitute(&body, var, &Term::Var(scrutinee_name.clone()));
        let motive = motive_names.iter().zip(&motive_types).rev().fold(
            Term::Lambda {
                param: scrutinee_name,
                param_type: Box::new(scrutinee_ty),
                body: Box::new(body),
            },
            |b, (n, t)| Term::Lambda {
                param: n.clone(),
                param_type: Box::new(t.clone()),
                body: Box::new(b),
            },
        );
        // A case per allowed constructor: its fields, then the dependents, introduced by name.
        let constructors = ctx.get_constructors(&ind);
        let mut case_terms = Vec::with_capacity(allowed.len());
        let mut case_goals = Vec::with_capacity(allowed.len());
        let mut new_decls: Vec<(String, Decl)> = Vec::new();
        let mut taken: Vec<String> = kept.iter().map(|(n, _)| n.clone()).collect();
        for (slot, &ci) in allowed.iter().enumerate() {
            let cname = constructors[ci].0.to_string();
            let ctor = apps(
                match head {
                    Term::Const { levels, .. } if ctx.poly_declaration(&cname).is_some() => {
                        Term::Const {
                            name: cname.clone(),
                            levels: levels.clone(),
                        }
                    }
                    _ => Term::Global(cname.clone()),
                },
                params,
            );
            let ctor_ty = infer_type(&ctx, &ctor).map_err(|e| {
                refuse(
                    "cases",
                    &goal,
                    format!("`{cname}` does not type at these parameters: {e}"),
                )
            })?;
            let given: &[&str] = names.get(slot).copied().unwrap_or(&[]);
            let mut case_hyps = kept.clone();
            let mut fields: Vec<(String, Term)> = Vec::new();
            let mut t = ctor_ty;
            let mut j = 0;
            loop {
                let scope = fields
                    .iter()
                    .fold(self.scope(&kept), |c, (n, ty)| c.extend(n, ty.clone()));
                let Term::Pi {
                    param: b,
                    param_type: bt,
                    body_type: rest,
                } = demanded_type_head(&scope, t.clone(), TypeHead::Function)
                else {
                    break;
                };
                let name = match given.get(j) {
                    Some(n) => {
                        if taken.iter().any(|t| t == n) {
                            return Err(refuse(
                                "cases",
                                &goal,
                                format!("`{n}` already names a hypothesis"),
                            ));
                        }
                        n.to_string()
                    }
                    None => fresh_name_avoiding(&b, &ctx, &case_hyps, &[&goal.target], &taken),
                };
                taken.push(name.clone());
                t = if b == name {
                    *rest
                } else {
                    substitute(&rest, &b, &Term::Var(name.clone()))
                };
                fields.push((name.clone(), (*bt).clone()));
                case_hyps.push((name, *bt));
                j += 1;
            }
            if given.len() > fields.len() {
                return Err(refuse(
                    "cases",
                    &goal,
                    format!(
                        "case `{cname}` names {} hypotheses and has {}",
                        given.len(),
                        fields.len()
                    ),
                ));
            }
            // `t` is the family at the constructor's result indices.
            let (_, result) = spine(&t);
            let result_indices = &result[np.min(result.len())..];
            let value = apps(
                ctor.clone(),
                &fields
                    .iter()
                    .map(|(n, _)| Term::Var(n.clone()))
                    .collect::<Vec<_>>(),
            );
            let mut target = beta_normal(&apps(apps(motive.clone(), result_indices), &[value]));
            let mut binders = fields.clone();
            for (dn, _) in &dependents {
                let Term::Pi {
                    param: b,
                    param_type: bt,
                    body_type: rest,
                } = &target
                else {
                    unreachable!("the motive's body is a Π over the dependents");
                };
                let rest = if b == dn {
                    (**rest).clone()
                } else {
                    substitute(rest, b, &Term::Var(dn.clone()))
                };
                binders.push((dn.clone(), (**bt).clone()));
                case_hyps.push((dn.clone(), (**bt).clone()));
                target = rest;
            }
            let case = self.fresh_meta_name();
            new_decls.push((
                case.clone(),
                Decl {
                    hyps: case_hyps,
                    target,
                },
            ));
            case_terms.push(
                binders
                    .iter()
                    .rev()
                    .fold(Term::Var(case.clone()), |b, (n, ty)| Term::Lambda {
                        param: n.clone(),
                        param_type: Box::new(ty.clone()),
                        body: Box::new(b),
                    }),
            );
            case_goals.push(case);
        }
        let mut proof = Term::Match {
            discriminant: Box::new(Term::Var(var.to_string())),
            motive: Box::new(motive),
            cases: case_terms,
        };
        for (dn, _) in &dependents {
            proof = Term::App(Box::new(proof), Box::new(Term::Var(dn.clone())));
        }
        for (name, decl) in new_decls {
            self.decls.insert(name, decl);
        }
        self.close(&m, proof, case_goals);
        Ok(())
    }

    /// `cases x` where `x : I p̄ ī` and the indices `ī` are not hypotheses of their own. Each index
    /// is generalized with an equation `eₖ : Eq Uₖ īₖ jₖ`, and the goal is stated of `x` moved back
    /// to `I p̄ ī` along them, so it means what it meant. Each constructor's case then solves its
    /// equations: a hypothesis on either side is substituted, two applications of one constructor
    /// are injected, two different constructors close the case, and an equation whose sides convert
    /// is dropped; one none of these settles stays a hypothesis. Lean's `cases` unifies indices the
    /// same way. When an index's type depends on an earlier index, the equations would relate values
    /// of different types (heterogeneous equality), and the tactic says so.
    #[allow(clippy::too_many_arguments)]
    fn cases_by_equations(
        &mut self,
        var: &str,
        names: &[&[&str]],
        m: &str,
        goal: &Goal,
        ctx: &Context,
        head: &Term,
        params: &[Term],
        indices: &[Term],
        allowed: &[usize],
    ) -> Result<(), TacticError> {
        let family_at = |ix: &[Term]| apps(apps(head.clone(), params), ix);
        let arity = infer_type(ctx, &apps(head.clone(), params)).map_err(|e| {
            refuse(
                "cases",
                goal,
                format!("the family does not type at these parameters: {e}"),
            )
        })?;
        let telescope = pi_telescope(ctx, &arity, indices.len());
        for (k, (_, ty)) in telescope.iter().enumerate() {
            if let Some((earlier, _)) = telescope[..k]
                .iter()
                .find(|(n, _)| crate::type_checker::occurs_free(ty, n))
            {
                return Err(refuse(
                    "cases",
                    goal,
                    format!(
                        "the type {} of index {} depends on the index `{earlier}`: its equation would be heterogeneous",
                        Shown(ty),
                        k + 1
                    ),
                ));
            }
        }
        let index_types: Vec<Term> = telescope.into_iter().map(|(_, t)| t).collect();
        let n = indices.len();
        let mut avoid: Vec<&Term> = vec![&goal.target];
        avoid.extend(goal.hyps.iter().map(|(_, t)| t));
        avoid.extend(indices.iter());
        let mut taken: Vec<String> = goal.hyps.iter().map(|(h, _)| h.clone()).collect();
        let fresh = |base: &str, taken: &mut Vec<String>| {
            let name = fresh_name_avoiding(base, ctx, &goal.hyps, &avoid, taken);
            taken.push(name.clone());
            name
        };
        let js: Vec<String> = (0..n).map(|_| fresh("j", &mut taken)).collect();
        let es: Vec<String> = (0..n).map(|_| fresh("e", &mut taken)).collect();
        let zs: Vec<String> = (0..n).map(|_| fresh("z", &mut taken)).collect();
        let ws: Vec<String> = (0..n).map(|_| fresh("w", &mut taken)).collect();
        let y = fresh("y", &mut taken);
        let mut abstracted: BTreeSet<String> = BTreeSet::from([var.to_string()]);
        let mut dependents: Vec<(String, Term)> = Vec::new();
        for (h, ty) in &goal.hyps {
            if !abstracted.contains(h)
                && abstracted
                    .iter()
                    .any(|a| crate::type_checker::occurs_free(ty, a))
            {
                abstracted.insert(h.clone());
                dependents.push((h.clone(), ty.clone()));
            }
        }
        let kept: Vec<(String, Term)> = goal
            .hyps
            .iter()
            .filter(|(h, _)| !abstracted.contains(h))
            .cloned()
            .collect();
        // `y : I p̄ j̄` moved back to `I p̄ ī`, one index at a time, along `eₖ`.
        let cast = |value: Term| {
            (0..n).fold(value, |value, k| {
                let at = |z: Term| {
                    let mut ix: Vec<Term> = indices[..k].to_vec();
                    ix.push(z);
                    ix.extend(js[k + 1..].iter().map(|j| Term::Var(j.clone())));
                    family_at(&ix)
                };
                let landed = at(indices[k].clone());
                let motive = lam(
                    &zs[k],
                    index_types[k].clone(),
                    lam(
                        "_",
                        eq_term(&index_types[k], &indices[k], &Term::Var(zs[k].clone())),
                        pi("_", at(Term::Var(zs[k].clone())), landed.clone()),
                    ),
                );
                let transport = Term::Match {
                    discriminant: Box::new(Term::Var(es[k].clone())),
                    motive: Box::new(motive),
                    cases: vec![lam(&ws[k], landed, Term::Var(ws[k].clone()))],
                };
                Term::App(Box::new(transport), Box::new(value))
            })
        };
        let mentioned =
            crate::type_checker::occurs_free(&goal.target, var) || !dependents.is_empty();
        let moved = if mentioned {
            cast(Term::Var(y.clone()))
        } else {
            Term::Var(y.clone())
        };
        let mut body = dependents
            .iter()
            .rev()
            .fold(goal.target.clone(), |t, (h, ty)| pi(h, ty.clone(), t));
        body = substitute(&body, var, &moved);
        for k in (0..n).rev() {
            body = pi(
                &es[k],
                eq_term(&index_types[k], &indices[k], &Term::Var(js[k].clone())),
                body,
            );
        }
        let js_vars: Vec<Term> = js.iter().map(|j| Term::Var(j.clone())).collect();
        let mut motive = lam(&y, family_at(&js_vars), body);
        for k in (0..n).rev() {
            motive = lam(&js[k], index_types[k].clone(), motive);
        }
        // A case per allowed constructor: its fields introduced by name, its equations still to solve.
        // A given name may not be one a case's hypotheses already have (the kept hypotheses, the
        // equations, the reintroduced dependents); the motive's own binder names are no hypotheses.
        let constructors = ctx.get_constructors(&self.family_name(head));
        let mut case_terms = Vec::with_capacity(allowed.len());
        let mut case_goals = Vec::with_capacity(allowed.len());
        taken.extend(dependents.iter().map(|(h, _)| h.clone()));
        let hypotheses: Vec<String> = kept
            .iter()
            .map(|(h, _)| h.clone())
            .chain(es.iter().cloned())
            .chain(dependents.iter().map(|(h, _)| h.clone()))
            .collect();
        for (slot, &ci) in allowed.iter().enumerate() {
            let cname = constructors[ci].0.to_string();
            let ctor = apps(constructor_at(ctx, head, &cname), params);
            let ctor_ty = infer_type(ctx, &ctor).map_err(|e| {
                refuse(
                    "cases",
                    goal,
                    format!("`{cname}` does not type at these parameters: {e}"),
                )
            })?;
            let given: &[&str] = names.get(slot).copied().unwrap_or(&[]);
            let mut fields: Vec<(String, Term)> = Vec::new();
            let mut t = ctor_ty;
            loop {
                let scope = fields
                    .iter()
                    .fold(self.scope(&kept), |c, (h, ty)| c.extend(h, ty.clone()));
                let Term::Pi {
                    param: b,
                    param_type: bt,
                    body_type: rest,
                } = demanded_type_head(&scope, t.clone(), TypeHead::Function)
                else {
                    break;
                };
                let name = match given.get(fields.len()) {
                    Some(h)
                        if hypotheses
                            .iter()
                            .chain(fields.iter().map(|(f, _)| f))
                            .any(|t| t == h) =>
                    {
                        return Err(refuse(
                            "cases",
                            goal,
                            format!("`{h}` already names a hypothesis"),
                        ))
                    }
                    Some(h) => h.to_string(),
                    None => fresh_name_avoiding(&b, ctx, &kept, &[&goal.target], &taken),
                };
                taken.push(name.clone());
                t = if b == name {
                    *rest
                } else {
                    substitute(&rest, &b, &Term::Var(name.clone()))
                };
                fields.push((name, *bt));
            }
            if given.len() > fields.len() {
                return Err(refuse(
                    "cases",
                    goal,
                    format!(
                        "case `{cname}` names {} hypotheses and has {}",
                        given.len(),
                        fields.len()
                    ),
                ));
            }
            let (_, result) = spine(&t);
            let result_indices = &result[params.len().min(result.len())..];
            let value = apps(
                ctor.clone(),
                &fields
                    .iter()
                    .map(|(h, _)| Term::Var(h.clone()))
                    .collect::<Vec<_>>(),
            );
            let target = beta_normal(&apps(apps(motive.clone(), result_indices), &[value]));
            let mut case_hyps = kept.clone();
            case_hyps.extend(fields.iter().cloned());
            let case = self.declare(case_hyps, target);
            case_terms.push(
                fields
                    .iter()
                    .rev()
                    .fold(Term::Var(case.clone()), |b, (h, ty)| lam(h, ty.clone(), b)),
            );
            case_goals.push(case);
        }
        let mut proof = Term::Match {
            discriminant: Box::new(Term::Var(var.to_string())),
            motive: Box::new(motive),
            cases: case_terms,
        };
        for k in 0..n {
            proof = Term::App(
                Box::new(proof),
                Box::new(apps(
                    Term::Global("refl".into()),
                    &[index_types[k].clone(), indices[k].clone()],
                )),
            );
        }
        for (h, _) in &dependents {
            proof = Term::App(Box::new(proof), Box::new(Term::Var(h.clone())));
        }
        self.close(m, proof, case_goals.clone());
        for case in case_goals {
            let foreign: BTreeSet<String> =
                self.open.iter().filter(|g| **g != case).cloned().collect();
            self.on_goal(&case, |st| {
                for e in &es {
                    st.intro(e)?;
                }
                for (h, _) in &dependents {
                    st.intro(h)?;
                }
                st.solve_index_equations(es.clone(), &foreign)
            })?;
        }
        Ok(())
    }

    /// The inductive family a family head names.
    fn family_name(&self, head: &Term) -> String {
        match head {
            Term::Global(n) | Term::Const { name: n, .. } => n.clone(),
            other => unreachable!("a family head is a constant, not {other:?}"),
        }
    }

    /// Settle the index equations `equations` of the focused goal, one at a time, until the goal is
    /// closed (a goal in `foreign` is focused) or none is left that a step settles.
    fn solve_index_equations(
        &mut self,
        equations: Vec<String>,
        foreign: &BTreeSet<String>,
    ) -> Result<(), TacticError> {
        let mut queue: std::collections::VecDeque<String> = equations.into();
        let mut injected: BTreeSet<String> = BTreeSet::new();
        while let Some(e) = queue.pop_front() {
            match self.open.first() {
                Some(g) if !foreign.contains(g) => {}
                _ => return Ok(()),
            }
            let (_, goal) = self.focus()?;
            let Some((_, ty)) = goal.hyps.iter().find(|(h, _)| *h == e).cloned() else {
                continue;
            };
            let ctx = self.scope(&goal.hyps);
            let Some(eq) = equation(&ctx, &ty) else {
                continue;
            };
            if def_eq(&ctx, &eq.lhs, &eq.rhs) {
                let _ = self.clear(&e);
                continue;
            }
            if substitutable(&goal, &eq).is_some() {
                self.subst(&e)?;
                continue;
            }
            if injected.contains(&e) {
                continue;
            }
            let heads = (
                crate::type_checker::demanded_index_constructor_head(&ctx, &eq.lhs),
                crate::type_checker::demanded_index_constructor_head(&ctx, &eq.rhs),
            );
            if let (Some((cl, _)), Some((cr, _))) = heads {
                if ctx.constructor_inductive(&cl).is_some()
                    && ctx.constructor_inductive(&cl) == ctx.constructor_inductive(&cr)
                {
                    let produced = self.inject(&e, &[])?;
                    injected.insert(e.clone());
                    for h in produced.into_iter().rev() {
                        queue.push_front(h);
                    }
                    queue.push_back(e);
                }
            }
        }
        Ok(())
    }

    /// Run `f` with the open goal `m` focused; the goals it leaves in `m`'s place stay there.
    fn on_goal(
        &mut self,
        m: &str,
        f: impl FnOnce(&mut Self) -> Result<(), TacticError>,
    ) -> Result<(), TacticError> {
        let Some(pos) = self.open.iter().position(|g| g == m) else {
            return Ok(());
        };
        let before: Vec<String> = self.open.clone();
        let goal = self.open.remove(pos);
        self.open.insert(0, goal);
        let result = f(self);
        let produced: Vec<String> = self
            .open
            .iter()
            .filter(|g| !before.contains(g) || *g == m)
            .cloned()
            .collect();
        let mut order: Vec<String> = before
            .iter()
            .filter(|g| *g != m && self.open.contains(g))
            .cloned()
            .collect();
        let at = pos.min(order.len());
        order.splice(at..at, produced);
        self.open = order;
        result
    }

    /// `subst h`: `h : Eq A a b` where `b` (or else `a`) is a hypothesis that the other side and `A`
    /// do not mention; it is replaced by the other side everywhere and leaves the context with `h`.
    pub fn subst(&mut self, h: &str) -> Result<(), TacticError> {
        let (_, goal) = self.focus()?;
        let ctx = self.scope(&goal.hyps);
        let Some((_, h_ty)) = goal.hyps.iter().find(|(n, _)| n == h).cloned() else {
            return Err(refuse("subst", &goal, format!("`{h}` is not a hypothesis")));
        };
        let Some(eq) = equation(&ctx, &h_ty) else {
            return Err(refuse(
                "subst",
                &goal,
                format!("`{h}` : {} is not an equation", Shown(&h_ty)),
            ));
        };
        match substitutable(&goal, &eq) {
            Some(Side::Right) => self.cases(h, &[&[]]),
            Some(Side::Left) => {
                let flipped = fresh_name(h, &ctx, &goal.hyps, &[&goal.target]);
                let z = fresh_name("z", &ctx, &goal.hyps, &[&goal.target, &h_ty]);
                let symmetric = Term::Match {
                    discriminant: Box::new(Term::Var(h.to_string())),
                    motive: Box::new(lam(
                        &z,
                        eq.carrier.clone(),
                        lam(
                            "_",
                            eq_term(&eq.carrier, &eq.lhs, &Term::Var(z.clone())),
                            eq_term(&eq.carrier, &Term::Var(z.clone()), &eq.lhs),
                        ),
                    )),
                    cases: vec![apps(eq.refl(), &[eq.carrier.clone(), eq.lhs.clone()])],
                };
                self.assert_fact(&flipped, eq_term(&eq.carrier, &eq.rhs, &eq.lhs), symmetric)?;
                self.cases(&flipped, &[&[]])?;
                let _ = self.clear(h);
                Ok(())
            }
            None => Err(refuse(
                "subst",
                &goal,
                format!("neither side of `{h}` is a hypothesis the other side does not mention"),
            )),
        }
    }

    /// `injection h as e₁ …`: `h : Eq T (C ā) (C b̄)` gives `eᵢ : Eq Tᵢ aᵢ bᵢ` for each field of `C`
    /// whose type no earlier field decides; `h : Eq T (C ā) (D b̄)` with `C ≠ D` closes the goal.
    /// Each fact is proved by transport along `h` into a predicate that matches on the value, the
    /// construction Lean's `noConfusion` packages.
    pub fn injection(&mut self, h: &str, names: &[&str]) -> Result<(), TacticError> {
        self.inject(h, names).map(|_| ())
    }

    /// [`GoalState::injection`], with the names of the equations it introduced (none when it closed
    /// the goal).
    fn inject(&mut self, h: &str, names: &[&str]) -> Result<Vec<String>, TacticError> {
        let (m, goal) = self.focus()?;
        let ctx = self.scope(&goal.hyps);
        let Some((_, h_ty)) = goal.hyps.iter().find(|(n, _)| n == h).cloned() else {
            return Err(refuse(
                "injection",
                &goal,
                format!("`{h}` is not a hypothesis"),
            ));
        };
        let Some(eq) = equation(&ctx, &h_ty) else {
            return Err(refuse(
                "injection",
                &goal,
                format!("`{h}` : {} is not an equation", Shown(&h_ty)),
            ));
        };
        let family = demanded_type_head(&ctx, eq.carrier.clone(), TypeHead::Inductive);
        let (fhead, fargs) = spine(&family);
        let ind = match fhead {
            Term::Global(n) | Term::Const { name: n, .. } if ctx.is_inductive(n) => n.clone(),
            _ => {
                return Err(refuse(
                    "injection",
                    &goal,
                    format!("{} is not an inductive family", Shown(&eq.carrier)),
                ))
            }
        };
        let np = ctx.inductive_num_params(&ind).min(fargs.len());
        let read = |t: &Term| crate::type_checker::demanded_index_constructor_head(&ctx, t);
        let (Some((cl, al)), Some((cr, ar))) = (read(&eq.lhs), read(&eq.rhs)) else {
            return Err(refuse(
                "injection",
                &goal,
                format!(
                    "{} and {} are not both constructor applications",
                    Shown(&eq.lhs),
                    Shown(&eq.rhs)
                ),
            ));
        };
        let allowed = crate::type_checker::constructors_the_indices_allow(&ctx, &ind, np, &fargs);
        let constructors: Vec<String> = ctx
            .get_constructors(&ind)
            .iter()
            .map(|(c, _)| c.to_string())
            .collect();
        let mut avoid: Vec<&Term> = vec![&goal.target, &h_ty];
        avoid.extend(goal.hyps.iter().map(|(_, t)| t));
        // `P z := match z with | C x̄ => body C x̄ | … end`, a proposition about a value of the carrier.
        let index_types = {
            let arity = infer_type(&ctx, &apps(fhead.clone(), &fargs[..np])).map_err(|e| {
                refuse(
                    "injection",
                    &goal,
                    format!("the family does not type at these parameters: {e}"),
                )
            })?;
            pi_telescope(&ctx, &arity, fargs.len() - np)
        };
        let mut taken: Vec<String> = Vec::new();
        let fresh = |base: &str, taken: &mut Vec<String>| {
            let name = fresh_name_avoiding(base, &ctx, &goal.hyps, &avoid, taken);
            taken.push(name.clone());
            name
        };
        let jvars: Vec<String> = index_types.iter().map(|_| fresh("j", &mut taken)).collect();
        let zp = fresh("z", &mut taken);
        let z = fresh("z", &mut taken);
        let mut fields_of: Vec<Vec<(String, Term)>> = Vec::new();
        for &ci in &allowed {
            let ctor_ty = infer_type(
                &ctx,
                &apps(constructor_at(&ctx, fhead, &constructors[ci]), &fargs[..np]),
            )
            .map_err(|e| {
                refuse(
                    "injection",
                    &goal,
                    format!("`{}` does not type: {e}", constructors[ci]),
                )
            })?;
            // Each field renamed apart from everything in scope and from the telescope's own names,
            // later fields' types following the renaming.
            let telescope = pi_telescope(&ctx, &ctor_ty, usize::MAX);
            taken.extend(telescope.iter().map(|(b, _)| b.clone()));
            let mut fields: Vec<(String, Term)> = Vec::new();
            let mut renames: Vec<(String, String)> = Vec::new();
            for (b, bt) in telescope {
                let bt = renames.iter().fold(bt, |t, (from, to)| {
                    substitute(&t, from, &Term::Var(to.clone()))
                });
                let renamed = fresh(&b, &mut taken);
                renames.push((b, renamed.clone()));
                fields.push((renamed, bt));
            }
            fields_of.push(fields);
        }
        let predicate = |on: &dyn Fn(&str, &[(String, Term)]) -> Term, of: Term| -> Term {
            let mut motive = lam(
                &zp,
                apps(
                    apps(fhead.clone(), &fargs[..np]),
                    &jvars
                        .iter()
                        .map(|j| Term::Var(j.clone()))
                        .collect::<Vec<_>>(),
                ),
                Term::Sort(crate::term::Universe::Prop),
            );
            for (k, (_, t)) in index_types.iter().enumerate().rev() {
                motive = lam(&jvars[k], t.clone(), motive);
            }
            let cases = allowed
                .iter()
                .zip(&fields_of)
                .map(|(&ci, fields)| {
                    fields
                        .iter()
                        .rev()
                        .fold(on(&constructors[ci], fields), |b, (x, t)| {
                            lam(x, t.clone(), b)
                        })
                })
                .collect();
            Term::Match {
                discriminant: Box::new(of),
                motive: Box::new(motive),
                cases,
            }
        };
        let transport = |on: &dyn Fn(&str, &[(String, Term)]) -> Term, start: Term| -> Term {
            Term::Match {
                discriminant: Box::new(Term::Var(h.to_string())),
                motive: Box::new(lam(
                    &z,
                    eq.carrier.clone(),
                    lam(
                        "_",
                        eq_term(&eq.carrier, &eq.lhs, &Term::Var(z.clone())),
                        predicate(on, Term::Var(z.clone())),
                    ),
                )),
                cases: vec![start],
            }
        };
        if cl != cr {
            let falsity = transport(
                &|c: &str, _: &[(String, Term)]| {
                    Term::Global(if c == cl { "True" } else { "False" }.into())
                },
                Term::Global("I".into()),
            );
            let proof = Term::Match {
                discriminant: Box::new(falsity),
                motive: Box::new(lam("_", Term::Global("False".into()), goal.target.clone())),
                cases: Vec::new(),
            };
            match infer_type(&ctx, &proof) {
                Ok(ty) if is_subtype(&ctx, &ty, &goal.target) => {}
                Ok(ty) => {
                    return Err(refuse(
                        "injection",
                        &goal,
                        format!("the refutation proves {}", Shown(&ty)),
                    ))
                }
                Err(e) => {
                    return Err(refuse(
                        "injection",
                        &goal,
                        format!("the refutation does not type: {e}"),
                    ))
                }
            }
            self.close(&m, proof, Vec::new());
            return Ok(Vec::new());
        }
        let position = allowed
            .iter()
            .position(|&ci| constructors[ci] == cl)
            .ok_or_else(|| {
                refuse(
                    "injection",
                    &goal,
                    format!("`{cl}` cannot produce a value of {}", Shown(&eq.carrier)),
                )
            })?;
        let fields = &fields_of[position];
        let (lhs_fields, rhs_fields) = (&al[np.min(al.len())..], &ar[np.min(ar.len())..]);
        let mut facts: Vec<(String, Term, Term)> = Vec::new();
        for (k, (_, field_ty)) in fields.iter().enumerate() {
            if fields[..k]
                .iter()
                .any(|(x, _)| crate::type_checker::occurs_free(field_ty, x))
            {
                continue;
            }
            let (Some(a), Some(b)) = (lhs_fields.get(k), rhs_fields.get(k)) else {
                continue;
            };
            let on = |c: &str, xs: &[(String, Term)]| {
                if c == cl {
                    eq_term(field_ty, a, &Term::Var(xs[k].0.clone()))
                } else {
                    Term::Global("True".into())
                }
            };
            let proof = transport(
                &on,
                apps(Term::Global("refl".into()), &[field_ty.clone(), a.clone()]),
            );
            let fact = eq_term(field_ty, a, b);
            match infer_type(&ctx, &proof) {
                Ok(ty) if is_subtype(&ctx, &ty, &fact) => {}
                Ok(ty) => {
                    return Err(refuse(
                        "injection",
                        &goal,
                        format!("the field's transport proves {}", Shown(&ty)),
                    ))
                }
                Err(e) => {
                    return Err(refuse(
                        "injection",
                        &goal,
                        format!("the field's transport does not type: {e}"),
                    ))
                }
            }
            let name = match names.get(facts.len()) {
                Some(n) if goal.hyps.iter().any(|(h, _)| h == n) => {
                    return Err(refuse(
                        "injection",
                        &goal,
                        format!("`{n}` already names a hypothesis"),
                    ))
                }
                Some(n) => n.to_string(),
                None => fresh_name_avoiding(
                    "e",
                    &ctx,
                    &goal.hyps,
                    &avoid,
                    &facts.iter().map(|(n, _, _)| n.clone()).collect::<Vec<_>>(),
                ),
            };
            facts.push((name, fact, proof));
        }
        if facts.is_empty() {
            return Err(refuse(
                "injection",
                &goal,
                format!("`{cl}` has no field whose equation can be stated"),
            ));
        }
        if names.len() > facts.len() {
            return Err(refuse(
                "injection",
                &goal,
                format!("{} names for {} equations", names.len(), facts.len()),
            ));
        }
        let mut hyps = goal.hyps.clone();
        hyps.extend(facts.iter().map(|(n, t, _)| (n.clone(), t.clone())));
        let inner = self.declare(hyps, goal.target.clone());
        let lambda = facts
            .iter()
            .rev()
            .fold(Term::Var(inner.clone()), |b, (n, t, _)| {
                lam(n, t.clone(), b)
            });
        let proof = apps(
            lambda,
            &facts.iter().map(|(_, _, p)| p.clone()).collect::<Vec<_>>(),
        );
        self.close(&m, proof, vec![inner]);
        Ok(facts.into_iter().map(|(n, _, _)| n).collect())
    }

    /// `calc [R₁ := p₁, R₂ := p₂, …]`: a chain of equations `Eq A l r`, a step's left side `_` read as
    /// the previous step's right side, joined by transitivity into a proof that the first left side
    /// equals the last right side, which closes the goal. A step proved by `_` is left as a goal.
    pub fn calc(&mut self, steps: &[(Term, Term)]) -> Result<(), TacticError> {
        let (m, goal) = self.focus()?;
        let ctx = self.scope(&goal.hyps);
        if steps.is_empty() {
            return Err(refuse(
                "calc",
                &goal,
                "a calculation has at least one step".to_string(),
            ));
        }
        let mut mctx = self.mctx.clone();
        let mut carrier: Option<Term> = None;
        let mut first: Option<Term> = None;
        let mut last: Option<Term> = None;
        let mut proofs: Vec<(Term, Term)> = Vec::with_capacity(steps.len());
        let mut holes: Vec<(String, Term)> = Vec::new();
        for (i, (relation, proof)) in steps.iter().enumerate() {
            let step = i + 1;
            let relation = localize(relation, &goal.hyps);
            let (head, args) = spine(&relation);
            let (Term::Global(eq) | Term::Const { name: eq, .. }) = head else {
                return Err(refuse(
                    "calc",
                    &goal,
                    format!("step {step}, {}, is not an equation", Shown(&relation)),
                ));
            };
            let [a, l, r] = args.as_slice() else {
                return Err(refuse(
                    "calc",
                    &goal,
                    format!("step {step}, {}, is not an equation", Shown(&relation)),
                ));
            };
            if eq != "Eq" {
                return Err(refuse(
                    "calc",
                    &goal,
                    format!("step {step}, {}, is not an equation", Shown(&relation)),
                ));
            }
            let l = match (l, &last) {
                (Term::Hole, Some(previous)) => previous.clone(),
                (Term::Hole, None) => {
                    return Err(refuse(
                        "calc",
                        &goal,
                        "the first step's left side has no previous step to read".to_string(),
                    ))
                }
                (l, Some(previous)) if !def_eq(&ctx, previous, l) => {
                    return Err(refuse(
                        "calc",
                        &goal,
                        format!(
                            "step {step} starts at {}, and step {i} ends at {}",
                            Shown(l),
                            Shown(previous)
                        ),
                    ))
                }
                (l, _) => l.clone(),
            };
            if let Some(c) = &carrier {
                if !def_eq(&ctx, c, a) {
                    return Err(refuse(
                        "calc",
                        &goal,
                        format!(
                            "step {step} relates values of {}, not of {}",
                            Shown(a),
                            Shown(c)
                        ),
                    ));
                }
            }
            let stated = eq_term(a, &l, r);
            infer_sort(&ctx, &stated).map_err(|e| {
                refuse(
                    "calc",
                    &goal,
                    format!("step {step}, {}, is not a type: {e}", Shown(&stated)),
                )
            })?;
            let proof = localize(proof, &goal.hyps);
            if matches!(proof, Term::Hole) {
                let name = fresh_meta(&mut mctx);
                holes.push((name.clone(), stated.clone()));
                proofs.push((Term::Var(name), stated));
            } else {
                let (value, ty) = typed(&ctx, &mut mctx, &goal.hyps, &proof, Some(&stated))
                    .map_err(|e| {
                        refuse(
                            "calc",
                            &goal,
                            format!("step {step}'s proof {} does not type: {e}", Shown(&proof)),
                        )
                    })?;
                elaborate::unify_in(&ctx, &mut mctx, &goal.hyps, &ty, &stated);
                let (value, ty) = (
                    elaborate::instantiate(&value, &mctx),
                    elaborate::instantiate(&ty, &mctx),
                );
                if elaborate::mentions_meta(&value) {
                    return Err(refuse(
                        "calc",
                        &goal,
                        format!("step {step}'s proof leaves a hole; write `_` for the whole proof to leave it as a goal"),
                    ));
                }
                if !is_subtype(&ctx, &ty, &stated) {
                    return Err(refuse(
                        "calc",
                        &goal,
                        format!(
                            "step {step} states {}, and its proof proves {}",
                            Shown(&stated),
                            Shown(&ty)
                        ),
                    ));
                }
                proofs.push((value, stated));
            }
            carrier.get_or_insert_with(|| a.clone());
            first.get_or_insert_with(|| l.clone());
            last = Some(r.clone());
        }
        let (carrier, first, last) = (
            carrier.expect("a step"),
            first.expect("a step"),
            last.expect("a step"),
        );
        let conclusion = eq_term(&carrier, &first, &last);
        if !is_subtype(&ctx, &conclusion, &goal.target) {
            return Err(refuse(
                "calc",
                &goal,
                format!(
                    "the calculation proves {}, and the goal is {}",
                    Shown(&conclusion),
                    Shown(&goal.target)
                ),
            ));
        }
        // `trans p q := match q return (λ w _. Eq A x w) with p end`, folded over the steps.
        let mut avoid: Vec<&Term> = vec![&goal.target];
        avoid.extend(proofs.iter().map(|(p, _)| p));
        let w = fresh_name("w", &ctx, &goal.hyps, &avoid);
        let mut chained = proofs[0].0.clone();
        for (q, stated) in &proofs[1..] {
            let (_, args) = spine(stated);
            let y = args[1].clone();
            chained = Term::Match {
                discriminant: Box::new(q.clone()),
                motive: Box::new(lam(
                    &w,
                    carrier.clone(),
                    lam(
                        "_",
                        eq_term(&carrier, &y, &Term::Var(w.clone())),
                        eq_term(&carrier, &first, &Term::Var(w.clone())),
                    ),
                )),
                cases: vec![chained],
            };
        }
        // Checked with each step left open standing in as a local of its statement.
        let mut probe_ctx = ctx.clone();
        let mut probe = MetaCtx::new();
        let mut locals: Vec<String> = Vec::new();
        for (name, stated) in &holes {
            let local =
                fresh_name_avoiding("step", &ctx, &goal.hyps, &[&chained, &goal.target], &locals);
            locals.push(local.clone());
            probe_ctx = probe_ctx.extend(&local, stated.clone());
            probe.solve(name, Term::Var(local));
        }
        let checked = elaborate::instantiate(&chained, &probe);
        match infer_type(&probe_ctx, &checked) {
            Ok(t) if is_subtype(&probe_ctx, &t, &goal.target) => {}
            Ok(t) => {
                return Err(refuse(
                    "calc",
                    &goal,
                    format!("the chain proves {}, not the goal", Shown(&t)),
                ))
            }
            Err(e) => {
                return Err(refuse(
                    "calc",
                    &goal,
                    format!("the chain does not type: {e}"),
                ))
            }
        }
        self.mctx = mctx;
        let mut subgoals = Vec::with_capacity(holes.len());
        for (name, stated) in holes {
            self.decls.insert(
                name.clone(),
                Decl {
                    hyps: goal.hyps.clone(),
                    target: stated,
                },
            );
            subgoals.push(name);
        }
        self.close(&m, chained, subgoals);
        Ok(())
    }

    /// `cc`: close an equation the hypotheses entail by congruence: their equations, read in either
    /// direction and chained, and the rule that equal functions applied to equal arguments are equal.
    /// The proof is built from the hypotheses by symmetry, transitivity and congruence, each a `match`
    /// on an equation's proof, so nothing is trusted: `qed` checks the chain.
    pub fn cc(&mut self) -> Result<(), TacticError> {
        let (m, goal) = self.focus()?;
        let ctx = self.scope(&goal.hyps);
        let Some(target) = equation(&ctx, &goal.target) else {
            return Err(refuse(
                "cc",
                &goal,
                "the goal is not an equation".to_string(),
            ));
        };
        let mut closure = congruence::Closure::new(&ctx, &goal);
        for (h, ty) in &goal.hyps {
            if let Some(eq) = equation(&ctx, ty) {
                closure.assert_hypothesis(h, &eq.lhs, &eq.rhs);
            }
        }
        let (a, b) = (closure.term(&target.lhs), closure.term(&target.rhs));
        closure.saturate();
        let Some(proof) = closure.prove(a, b) else {
            return Err(refuse(
                "cc",
                &goal,
                format!(
                    "the hypotheses do not entail {} = {}",
                    Shown(&target.lhs),
                    Shown(&target.rhs)
                ),
            ));
        };
        match infer_type(&ctx, &proof) {
            Ok(ty) if is_subtype(&ctx, &ty, &goal.target) => {}
            Ok(ty) => {
                return Err(refuse(
                    "cc",
                    &goal,
                    format!("the chain proves {}, not the goal", Shown(&ty)),
                ))
            }
            Err(e) => return Err(refuse("cc", &goal, format!("the chain does not type: {e}"))),
        }
        self.close(&m, proof, Vec::new());
        Ok(())
    }

    /// `decide`: a goal `P` with a `Decidable P` instance the kernel evaluates to `isTrue`. The proof is
    /// `of_decide_eq_true P inst (refl Bool true)`, which the kernel accepts by computing
    /// `decide P inst` to `true`: a proved bridge and conversion, nothing assumed.
    pub fn decide(&mut self) -> Result<(), TacticError> {
        let (m, goal) = self.focus()?;
        let ctx = self.scope(&goal.hyps);
        let required = apps(Term::Global("Decidable".into()), &[goal.target.clone()]);
        let mut mctx = self.mctx.clone();
        let Some(instance) = elaborate::resolve_instance(&ctx, &mut mctx, &required) else {
            return Err(refuse(
                "decide",
                &goal,
                format!("no `Decidable` instance decides {}", Shown(&goal.target)),
            ));
        };
        let instance = elaborate::instantiate(&instance, &mctx);
        let verdict = crate::reduction::normalize(
            &ctx,
            &apps(
                Term::Global("decide".into()),
                &[goal.target.clone(), instance.clone()],
            ),
        );
        match &verdict {
            Term::Global(b) if b == "true" => {}
            Term::Global(b) if b == "false" => {
                return Err(refuse(
                    "decide",
                    &goal,
                    format!(
                        "{} is false: its instance decides it so",
                        Shown(&goal.target)
                    ),
                ))
            }
            other => {
                return Err(refuse(
                    "decide",
                    &goal,
                    format!(
                        "the instance does not compute to a verdict: `decide` reduces to {}",
                        Shown(other)
                    ),
                ))
            }
        }
        let proof = apps(
            Term::Global("of_decide_eq_true".into()),
            &[
                goal.target.clone(),
                instance,
                apps(
                    Term::Global("refl".into()),
                    &[Term::Global("Bool".into()), Term::Global("true".into())],
                ),
            ],
        );
        match infer_type(&ctx, &proof) {
            Ok(ty) if is_subtype(&ctx, &ty, &goal.target) => {}
            Ok(ty) => {
                return Err(refuse(
                    "decide",
                    &goal,
                    format!("the bridge proves {}, not the goal", Shown(&ty)),
                ))
            }
            Err(e) => {
                return Err(refuse(
                    "decide",
                    &goal,
                    format!("the bridge does not type: {e}"),
                ))
            }
        }
        self.close(&m, proof, Vec::new());
        Ok(())
    }

    /// `simp [l₁ …]`: rewrite the goal by each lemma (`Π x̄. Eq A l r`: a hypothesis, a constant or a
    /// term), left to right, wherever an instance of `l` stands outside every binder of the goal,
    /// until none applies, then close the goal by `rfl` when its sides agree. With no lemma it
    /// rewrites by the context's `@[simp]` theorems. Each rewrite is a `rewrite` along the lemma at
    /// the matched instance, so the proof is the chain of their transports with each lemma cited
    /// by name, and nothing is trusted. A run that rewrites nothing is refused, as is one still
    /// rewriting after [`SIMP_STEPS`] rewrites; either leaves the goal as it found it.
    pub fn simp(&mut self, lemmas: &[Term]) -> Result<(), TacticError> {
        let saved = self.clone();
        let result = self.simp_rewrites(lemmas);
        if result.is_err() {
            *self = saved;
        }
        result
    }

    fn simp_rewrites(&mut self, lemmas: &[Term]) -> Result<(), TacticError> {
        let (_, goal) = self.focus()?;
        let ctx = self.scope(&goal.hyps);
        let rules: Vec<(Term, SimpRule)> = if lemmas.is_empty() {
            ctx.simp_rules()
                .iter()
                .map(|r| (Term::Global(r.name.clone()), r.clone()))
                .collect()
        } else {
            lemmas
                .iter()
                .map(|lemma| {
                    let proof = localize(lemma, &goal.hyps);
                    let ty = infer_type(&ctx, &proof).map_err(|e| {
                        refuse(
                            "simp",
                            &goal,
                            format!("{} does not type: {e}", Shown(&proof)),
                        )
                    })?;
                    let rule = SimpRule::compile(&Shown(&proof).to_string(), &ty).map_err(|e| {
                        refuse(
                            "simp",
                            &goal,
                            format!("{} : {} is not a rewrite: {e}", Shown(&proof), Shown(&ty)),
                        )
                    })?;
                    Ok((proof, rule))
                })
                .collect::<Result<_, TacticError>>()?
        };
        if rules.is_empty() {
            return Err(refuse(
                "simp",
                &goal,
                "there is no lemma to rewrite by".to_string(),
            ));
        }
        let mut fired: Vec<String> = Vec::new();
        loop {
            let (_, goal) = self.focus()?;
            let Some((proof, name)) = simp_instance(&rules, &goal.target, &mut Vec::new()) else {
                break;
            };
            if fired.len() == SIMP_STEPS {
                let last: Vec<&str> = fired.iter().rev().take(4).map(String::as_str).collect();
                return Err(refuse(
                    "simp",
                    &goal,
                    format!(
                        "still rewriting after {SIMP_STEPS} rewrites (the last by {})",
                        last.join(", ")
                    ),
                ));
            }
            self.rewrite(&proof, false)?;
            fired.push(name);
        }
        if fired.is_empty() {
            let (_, goal) = self.focus()?;
            return Err(refuse(
                "simp",
                &goal,
                "no lemma rewrites the goal".to_string(),
            ));
        }
        if self
            .focus()
            .is_ok_and(|(_, goal)| equation(&self.scope(&goal.hyps), &goal.target).is_some())
        {
            let _ = self.rfl();
        }
        Ok(())
    }

    /// Add the hypothesis `name : fact`, proved by `proof` here and now.
    fn assert_fact(&mut self, name: &str, fact: Term, proof: Term) -> Result<(), TacticError> {
        let (m, goal) = self.focus()?;
        let ctx = self.scope(&goal.hyps);
        match infer_type(&ctx, &proof) {
            Ok(ty) if is_subtype(&ctx, &ty, &fact) => {}
            Ok(ty) => {
                return Err(refuse(
                    "have",
                    &goal,
                    format!("the proof proves {}, not {}", Shown(&ty), Shown(&fact)),
                ))
            }
            Err(e) => {
                return Err(refuse(
                    "have",
                    &goal,
                    format!("the proof of {} does not type: {e}", Shown(&fact)),
                ))
            }
        }
        let mut hyps = goal.hyps.clone();
        hyps.push((name.to_string(), fact.clone()));
        let inner = self.declare(hyps, goal.target.clone());
        self.close(
            &m,
            Term::App(
                Box::new(lam(name, fact, Term::Var(inner.clone()))),
                Box::new(proof),
            ),
            vec![inner],
        );
        Ok(())
    }

    /// Drop the hypothesis `h`, which nothing in the goal mentions.
    fn clear(&mut self, h: &str) -> Result<(), TacticError> {
        let (m, goal) = self.focus()?;
        if !goal.hyps.iter().any(|(n, _)| n == h) {
            return Err(refuse("clear", &goal, format!("`{h}` is not a hypothesis")));
        }
        if crate::type_checker::occurs_free(&goal.target, h)
            || goal
                .hyps
                .iter()
                .any(|(n, t)| n != h && crate::type_checker::occurs_free(t, h))
        {
            return Err(refuse(
                "clear",
                &goal,
                format!("the goal still mentions `{h}`"),
            ));
        }
        let hyps: Vec<(String, Term)> = goal.hyps.iter().filter(|(n, _)| n != h).cloned().collect();
        let inner = self.declare(hyps, goal.target.clone());
        self.close(&m, Term::Var(inner.clone()), vec![inner]);
        Ok(())
    }

    /// `refine e`: `e` with holes (`_`), elaborated against the goal; every hole unification leaves
    /// open becomes a goal, in the order the holes appear, with the binders around it as hypotheses.
    pub fn refine(&mut self, term: &Term) -> Result<(), TacticError> {
        let (m, goal) = self.focus()?;
        let ctx = self.scope(&goal.hyps);
        let term = localize(term, &goal.hyps);
        let mut mctx = self.mctx.clone();
        let first = mctx.holes().len();
        let (value, ty) =
            elaborate::elaborate_in(&ctx, &mut mctx, &goal.hyps, &term, Some(&goal.target))
                .map_err(|e| {
                    refuse(
                        "refine",
                        &goal,
                        format!(
                            "{} does not elaborate against the goal {}: {e}",
                            Shown(&term),
                            Shown(&goal.target)
                        ),
                    )
                })?;
        if !elaborate::unify_in(&ctx, &mut mctx, &goal.hyps, &ty, &goal.target) {
            let ty = elaborate::instantiate(&ty, &mctx);
            return Err(refuse(
                "refine",
                &goal,
                format!(
                    "the goal is {}, and {} has type {}",
                    Shown(&goal.target),
                    Shown(&term),
                    Shown(&ty)
                ),
            ));
        }
        let value = elaborate::instantiate(&value, &mctx);
        let holes: Vec<(String, Vec<(String, Term)>, Term)> = mctx.holes()[first..]
            .iter()
            .filter(|h| mctx.solution(&h.name).is_none())
            .map(|h| {
                let scope = h
                    .scope
                    .iter()
                    .map(|(n, t)| (n.clone(), elaborate::instantiate(t, &mctx)))
                    .collect();
                (h.name.clone(), scope, elaborate::instantiate(&h.ty, &mctx))
            })
            .collect();
        for (_, scope, ty) in &holes {
            if elaborate::mentions_meta(ty) {
                return Err(refuse(
                    "refine",
                    &goal,
                    format!("the type {} of a hole is not determined", Shown(ty)),
                ));
            }
            if let Some((n, _)) = scope
                .iter()
                .enumerate()
                .find(|(i, (n, _))| scope[..*i].iter().any(|(o, _)| o == n))
                .map(|(_, b)| b)
            {
                return Err(refuse(
                    "refine",
                    &goal,
                    format!("a binder around a hole shadows the hypothesis `{n}`; name it apart"),
                ));
            }
        }
        let open: BTreeSet<&str> = holes.iter().map(|(n, _, _)| n.as_str()).collect();
        let mut stray = BTreeSet::new();
        metas(&value, &mut stray);
        if let Some(meta) = stray.iter().find(|n| !open.contains(n.as_str())) {
            return Err(refuse(
                "refine",
                &goal,
                format!(
                    "{} leaves {meta}, an argument unification cannot determine",
                    Shown(&term)
                ),
            ));
        }
        // Checked with each hole standing in as a local function of the binders around it.
        let mut probe_ctx = ctx.clone();
        let mut probe = MetaCtx::new();
        let mut locals: Vec<String> = Vec::new();
        for (name, scope, ty) in &holes {
            let extra = &scope[goal.hyps.len().min(scope.len())..];
            let local =
                fresh_name_avoiding("hole", &ctx, &goal.hyps, &[&value, &goal.target], &locals);
            locals.push(local.clone());
            let local_ty = extra
                .iter()
                .rev()
                .fold(ty.clone(), |b, (n, t)| pi(n, t.clone(), b));
            probe_ctx = probe_ctx.extend(&local, local_ty);
            probe.solve(
                name,
                apps(
                    Term::Var(local),
                    &extra
                        .iter()
                        .map(|(n, _)| Term::Var(n.clone()))
                        .collect::<Vec<_>>(),
                ),
            );
        }
        let checked = elaborate::instantiate(&value, &probe);
        match infer_type(&probe_ctx, &checked) {
            Ok(t) if is_subtype(&probe_ctx, &t, &goal.target) => {}
            Ok(t) => {
                return Err(refuse(
                    "refine",
                    &goal,
                    format!("{} proves {}, not the goal", Shown(&term), Shown(&t)),
                ))
            }
            Err(e) => {
                return Err(refuse(
                    "refine",
                    &goal,
                    format!("{} does not type: {e}", Shown(&term)),
                ))
            }
        }
        self.mctx = mctx;
        let mut subgoals = Vec::with_capacity(holes.len());
        for (name, scope, ty) in holes {
            self.decls.insert(
                name.clone(),
                Decl {
                    hyps: scope,
                    target: ty,
                },
            );
            subgoals.push(name);
        }
        self.close(&m, value, subgoals);
        Ok(())
    }

    fn declare(&mut self, hyps: Vec<(String, Term)>, target: Term) -> String {
        let name = self.fresh_meta_name();
        self.decls.insert(name.clone(), Decl { hyps, target });
        name
    }

    fn fresh_meta_name(&mut self) -> String {
        match self.mctx.fresh() {
            Term::Var(name) => name,
            other => unreachable!("a metavariable is a `Var`, not {other:?}"),
        }
    }

    fn view(&self, m: &str) -> Goal {
        let decl = &self.decls[m];
        let mut hyps: Vec<(String, Term)> = Vec::with_capacity(decl.hyps.len());
        for (n, t) in &decl.hyps {
            let t = simplify(
                &self.scope(&hyps),
                &hyps,
                &elaborate::instantiate(t, &self.mctx),
            );
            hyps.push((n.clone(), t));
        }
        let target = simplify(
            &self.scope(&hyps),
            &hyps,
            &elaborate::instantiate(&decl.target, &self.mctx),
        );
        Goal { hyps, target }
    }

    fn focus(&self) -> Result<(String, Goal), TacticError> {
        let m = self.open.first().ok_or(TacticError::NoGoals)?;
        Ok((m.clone(), self.view(m)))
    }

    fn scope(&self, hyps: &[(String, Term)]) -> Context {
        hyps.iter()
            .fold(self.ctx.clone(), |c, (n, t)| c.extend(n, t.clone()))
    }

    /// Solve the focused goal `m` with `proof`; the goals among `subgoals` it leaves unsolved
    /// take its place, in order.
    fn close(&mut self, m: &str, proof: Term, subgoals: Vec<String>) {
        self.mctx.solve(m, proof);
        self.open.retain(|g| g != m);
        let fresh: Vec<String> = subgoals
            .into_iter()
            .filter(|g| self.mctx.solution(g).is_none())
            .collect();
        for (i, g) in fresh.into_iter().enumerate() {
            self.open.insert(i, g);
        }
        let solved: Vec<String> = self
            .open
            .iter()
            .filter(|g| self.mctx.solution(g).is_some())
            .cloned()
            .collect();
        self.open.retain(|g| !solved.contains(g));
    }
}

/// How many rewrites one `simp` makes before it is refused as not terminating.
pub const SIMP_STEPS: usize = 256;

/// The first instance of a rule in `t`, leftmost outermost and outside every binder of `t` (an
/// instance mentioning a bound variable is not a term of the goal's context): the rule's proof at
/// the matched instance, and the rule's name.
fn simp_instance(
    rules: &[(Term, SimpRule)],
    t: &Term,
    bound: &mut Vec<String>,
) -> Option<(Term, String)> {
    if !bound.iter().any(|b| crate::type_checker::occurs_free(t, b)) {
        for (proof, rule) in rules {
            if let Some(instance) = rule.matching(t) {
                return Some((apps(proof.clone(), &instance), rule.name.clone()));
            }
        }
    }
    let mut under = |name: &str, body: &Term, bound: &mut Vec<String>| {
        bound.push(name.to_string());
        let found = simp_instance(rules, body, bound);
        bound.pop();
        found
    };
    match t {
        Term::App(f, a) => {
            simp_instance(rules, f, bound).or_else(|| simp_instance(rules, a, bound))
        }
        Term::Pi {
            param,
            param_type,
            body_type,
        } => simp_instance(rules, param_type, bound).or_else(|| under(param, body_type, bound)),
        Term::Lambda {
            param,
            param_type,
            body,
        } => simp_instance(rules, param_type, bound).or_else(|| under(param, body, bound)),
        _ => None,
    }
}

fn fresh_meta(mctx: &mut MetaCtx) -> String {
    match mctx.fresh() {
        Term::Var(name) => name,
        other => unreachable!("a metavariable is a `Var`, not {other:?}"),
    }
}

fn lam(param: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: param.to_string(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}

fn pi(param: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: param.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}

fn eq_term(carrier: &Term, lhs: &Term, rhs: &Term) -> Term {
    apps(
        Term::Global("Eq".into()),
        &[carrier.clone(), lhs.clone(), rhs.clone()],
    )
}

/// The constructor `name` at the levels its family's head carries.
fn constructor_at(ctx: &Context, family_head: &Term, name: &str) -> Term {
    match family_head {
        Term::Const { levels, .. } if ctx.poly_declaration(name).is_some() => Term::Const {
            name: name.to_string(),
            levels: levels.clone(),
        },
        _ => Term::Global(name.to_string()),
    }
}

/// Which side of an equation is a hypothesis of `goal` that neither the other side nor the carrier
/// mentions, the right preferred.
#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
}

fn substitutable(goal: &Goal, eq: &Equation) -> Option<Side> {
    let free = |t: &Term, other: &Term| match t {
        Term::Var(v) => {
            goal.hyps.iter().any(|(n, _)| n == v)
                && !crate::type_checker::occurs_free(other, v)
                && !crate::type_checker::occurs_free(&eq.carrier, v)
        }
        _ => false,
    };
    if free(&eq.rhs, &eq.lhs) {
        Some(Side::Right)
    } else if free(&eq.lhs, &eq.rhs) {
        Some(Side::Left)
    } else {
        None
    }
}

/// The metavariables `t` mentions.
fn metas(t: &Term, out: &mut BTreeSet<String>) {
    match t {
        Term::Var(v) if elaborate::is_meta(v) => {
            out.insert(v.clone());
        }
        Term::Var(_)
        | Term::Global(_)
        | Term::Const { .. }
        | Term::Sort(_)
        | Term::Lit(_)
        | Term::Hole => {}
        Term::App(a, b) => {
            metas(a, out);
            metas(b, out);
        }
        Term::Pi {
            param_type: a,
            body_type: b,
            ..
        }
        | Term::Lambda {
            param_type: a,
            body: b,
            ..
        } => {
            metas(a, out);
            metas(b, out);
        }
        Term::Let {
            ty, value, body, ..
        } => {
            metas(ty, out);
            metas(value, out);
            metas(body, out);
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            metas(discriminant, out);
            metas(motive, out);
            cases.iter().for_each(|c| metas(c, out));
        }
        Term::Fix { body, .. } => metas(body, out),
        Term::MutualFix { defs, .. } => defs.iter().for_each(|(_, b)| metas(b, out)),
        Term::Annotated { term, .. } => metas(term, out),
    }
}

/// `t` as a goal reads it: β-redexes contracted, a `match` on a constructor application reduced to
/// its case (ι), and a `match` on a hypothesis proving an equation whose sides convert reduced as if
/// on `refl` (the kernel's K-like rule). Each step is a conversion, nothing is unfolded.
fn simplify(ctx: &Context, hyps: &[(String, Term)], t: &Term) -> Term {
    let go = |t: &Term| simplify(ctx, hyps, t);
    match t {
        Term::App(f, a) => match (go(f), go(a)) {
            (Term::Lambda { param, body, .. }, a) => go(&substitute(&body, &param, &a)),
            (f, a) => Term::App(Box::new(f), Box::new(a)),
        },
        Term::Pi {
            param,
            param_type,
            body_type,
        } => pi(param, go(param_type), go(body_type)),
        Term::Lambda {
            param,
            param_type,
            body,
        } => lam(param, go(param_type), go(body)),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            let discriminant = go(discriminant);
            let cases: Vec<Term> = cases.iter().map(go).collect();
            if let Some(reduced) = iota(ctx, hyps, &discriminant, &cases) {
                return go(&reduced);
            }
            Term::Match {
                discriminant: Box::new(discriminant),
                motive: Box::new(go(motive)),
                cases,
            }
        }
        Term::Annotated { term, .. } => go(term),
        _ => t.clone(),
    }
}

fn iota(
    ctx: &Context,
    hyps: &[(String, Term)],
    discriminant: &Term,
    cases: &[Term],
) -> Option<Term> {
    let (head, args) = spine(discriminant);
    match head {
        Term::Global(c) | Term::Const { name: c, .. } if ctx.is_constructor(c) => {
            let ind = ctx.constructor_inductive(c)?.to_string();
            let constructors = ctx.get_constructors(&ind);
            if cases.len() != constructors.len() {
                return None;
            }
            let position = constructors.iter().position(|(n, _)| n == c)?;
            let np = ctx.inductive_num_params(&ind);
            Some(apps(cases[position].clone(), args.get(np..)?))
        }
        Term::Var(h) if cases.len() == 1 => {
            let (_, ty) = hyps.iter().rev().find(|(n, _)| n == h)?;
            let eq = equation(ctx, ty)?;
            def_eq(ctx, &eq.lhs, &eq.rhs).then(|| cases[0].clone())
        }
        _ => None,
    }
}

/// A term handed to a tactic, with its type. A term with no hole is complete: the kernel types it,
/// its application rule solving the levels of a bare polymorphic constant (`refl A x`), which the
/// elaborator's unifier cannot (it has no level metavariables). A term with holes, or one the kernel
/// cannot type as written (an omitted implicit argument), is elaborated against `expected`.
fn typed(
    ctx: &Context,
    mctx: &mut MetaCtx,
    hyps: &[(String, Term)],
    term: &Term,
    expected: Option<&Term>,
) -> Result<(Term, Term), KernelError> {
    if !crate::type_checker::mentions_hole(term) {
        if let Ok(ty) = infer_type(ctx, term) {
            return Ok((term.clone(), ty));
        }
    }
    elaborate::elaborate_in(ctx, mctx, hyps, term, expected)
}

/// A term as the vernacular spells it (`Eq Nat Zero Zero`), its free variables (a goal's hypotheses)
/// by name, or the kernel's own rendering where the surface has no spelling (an unsolved
/// metavariable).
struct Shown<'a>(&'a Term);

impl fmt::Display for Shown<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let locals = crate::type_checker::free_vars(self.0);
        let locals: Vec<&str> = locals.iter().map(String::as_str).collect();
        match crate::interface::print_term_in(self.0, &locals) {
            Ok(text) => f.write_str(&text),
            Err(_) => write!(f, "{}", self.0),
        }
    }
}

fn refuse(tactic: &'static str, goal: &Goal, why: String) -> TacticError {
    TacticError::DoesNotApply {
        tactic,
        goal: goal.clone(),
        why,
    }
}

fn apps(f: Term, args: &[Term]) -> Term {
    args.iter()
        .fold(f, |acc, a| Term::App(Box::new(acc), Box::new(a.clone())))
}

fn spine(t: &Term) -> (&Term, Vec<Term>) {
    let mut args = Vec::new();
    let mut head = t;
    while let Term::App(f, a) = head {
        args.push((**a).clone());
        head = f;
    }
    args.reverse();
    (head, args)
}

/// A global named like a hypothesis is that hypothesis.
fn localize(term: &Term, hyps: &[(String, Term)]) -> Term {
    if hyps.is_empty() {
        return term.clone();
    }
    let names: BTreeSet<&str> = hyps.iter().map(|(n, _)| n.as_str()).collect();
    map_globals(term, &|n| {
        names.contains(n).then(|| Term::Var(n.to_string()))
    })
}

fn map_globals(t: &Term, f: &dyn Fn(&str) -> Option<Term>) -> Term {
    let go = |t: &Term| Box::new(map_globals(t, f));
    match t {
        Term::Global(n) => f(n).unwrap_or_else(|| t.clone()),
        Term::Var(_) | Term::Const { .. } | Term::Sort(_) | Term::Lit(_) | Term::Hole => t.clone(),
        Term::App(a, b) => Term::App(go(a), go(b)),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: go(param_type),
            body_type: go(body_type),
        },
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: go(param_type),
            body: go(body),
        },
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: go(discriminant),
            motive: go(motive),
            cases: cases.iter().map(|c| map_globals(c, f)).collect(),
        },
        Term::Fix { name, body } => Term::Fix {
            name: name.clone(),
            body: go(body),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(n, b)| (n.clone(), map_globals(b, f)))
                .collect(),
            index: *index,
        },
        Term::Let {
            name,
            ty,
            value,
            body,
        } => Term::Let {
            name: name.clone(),
            ty: go(ty),
            value: go(value),
            body: go(body),
        },
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: go(term),
        },
    }
}

/// How many premises `ty` has, reading through definitions to each `Π`.
fn pi_arity(ctx: &Context, ty: &Term) -> usize {
    pi_telescope(ctx, ty, usize::MAX).len()
}

/// The first `limit` binders of `ty`'s `Π` telescope, each type in the scope of those before it.
fn pi_telescope(ctx: &Context, ty: &Term, limit: usize) -> Vec<(String, Term)> {
    let mut out = Vec::new();
    let mut scope = ctx.clone();
    let mut t = ty.clone();
    while out.len() < limit {
        match demanded_type_head(&scope, t, TypeHead::Function) {
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                scope = scope.extend(&param, (*param_type).clone());
                out.push((param, *param_type));
                t = *body_type;
            }
            _ => break,
        }
    }
    out
}

/// An equation `Eq A l r`, read through definitions to the `Eq` head.
struct Equation {
    head: Term,
    carrier: Term,
    lhs: Term,
    rhs: Term,
}

impl Equation {
    /// `refl` at the levels the equation's `Eq` carries.
    fn refl(&self) -> Term {
        match &self.head {
            Term::Const { levels, .. } => Term::Const {
                name: "refl".into(),
                levels: levels.clone(),
            },
            _ => Term::Global("refl".into()),
        }
    }
}

fn equation(ctx: &Context, ty: &Term) -> Option<Equation> {
    let read = |t: &Term| {
        let (head, args) = spine(t);
        match head {
            Term::Global(n) | Term::Const { name: n, .. } if n == "Eq" && args.len() == 3 => {
                Some(Equation {
                    head: head.clone(),
                    carrier: args[0].clone(),
                    lhs: args[1].clone(),
                    rhs: args[2].clone(),
                })
            }
            _ => None,
        }
    };
    read(ty).or_else(|| read(&demanded_type_head(ctx, ty.clone(), TypeHead::Inductive)))
}

/// `t` with every occurrence of `from` replaced by the variable `x`, and whether there was one.
/// An occurrence under a binder of one of `from`'s free variables is a different term and is
/// left alone.
fn abstract_occurrences(t: &Term, from: &Term, x: &str) -> (Term, bool) {
    if alpha_equivalent(t, from) {
        return (Term::Var(x.to_string()), true);
    }
    let captured = |name: &str| crate::type_checker::occurs_free(from, name);
    let mut found = false;
    let mut go = |t: &Term| {
        let (r, f) = abstract_occurrences(t, from, x);
        found |= f;
        Box::new(r)
    };
    let r = match t {
        Term::App(a, b) => Term::App(go(a), go(b)),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            let pt = go(param_type);
            let bt = if captured(param) {
                body_type.clone()
            } else {
                go(body_type)
            };
            Term::Pi {
                param: param.clone(),
                param_type: pt,
                body_type: bt,
            }
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            let pt = go(param_type);
            let b = if captured(param) {
                body.clone()
            } else {
                go(body)
            };
            Term::Lambda {
                param: param.clone(),
                param_type: pt,
                body: b,
            }
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            let (ty2, v2) = (go(ty), go(value));
            let b = if captured(name) {
                body.clone()
            } else {
                go(body)
            };
            Term::Let {
                name: name.clone(),
                ty: ty2,
                value: v2,
                body: b,
            }
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            let d = go(discriminant);
            let mo = go(motive);
            let cs = cases.iter().map(|c| *go(c)).collect();
            Term::Match {
                discriminant: d,
                motive: mo,
                cases: cs,
            }
        }
        Term::Fix { name, body } => {
            let b = if captured(name) {
                body.clone()
            } else {
                go(body)
            };
            Term::Fix {
                name: name.clone(),
                body: b,
            }
        }
        Term::Annotated { term, .. } => *go(term),
        _ => t.clone(),
    };
    (r, found)
}

/// Every redex `(λx. b) a` contracted, nothing unfolded.
fn beta_normal(t: &Term) -> Term {
    match t {
        Term::App(f, a) => {
            let (f, a) = (beta_normal(f), beta_normal(a));
            match f {
                Term::Lambda { param, body, .. } => beta_normal(&substitute(&body, &param, &a)),
                f => Term::App(Box::new(f), Box::new(a)),
            }
        }
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(beta_normal(param_type)),
            body_type: Box::new(beta_normal(body_type)),
        },
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(beta_normal(param_type)),
            body: Box::new(beta_normal(body)),
        },
        Term::Annotated { term, .. } => beta_normal(term),
        _ => t.clone(),
    }
}

/// A name for a new local: `base`, or `base` with a numeral, mentioned nowhere in `terms`, naming
/// no hypothesis and no global.
fn fresh_name(base: &str, ctx: &Context, hyps: &[(String, Term)], terms: &[&Term]) -> String {
    fresh_name_avoiding(base, ctx, hyps, terms, &[])
}

fn fresh_name_avoiding(
    base: &str,
    ctx: &Context,
    hyps: &[(String, Term)],
    terms: &[&Term],
    also: &[String],
) -> String {
    let base = if base.is_empty() || base == "_" {
        "x"
    } else {
        base
    };
    let clashes = |n: &str| {
        hyps.iter().any(|(h, _)| h == n)
            || also.iter().any(|a| a == n)
            || ctx.get_global(n).is_some()
            || terms.iter().any(|t| mentions_name(t, n))
    };
    if !clashes(base) {
        return base.to_string();
    }
    (1..)
        .map(|i| format!("{base}{i}"))
        .find(|n| !clashes(n))
        .expect("a fresh name exists")
}

fn mentions_name(t: &Term, n: &str) -> bool {
    match t {
        Term::Var(v) | Term::Global(v) => v == n,
        Term::Const { name, .. } => name == n,
        Term::Sort(_) | Term::Lit(_) | Term::Hole => false,
        Term::App(a, b) => mentions_name(a, n) || mentions_name(b, n),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => param == n || mentions_name(param_type, n) || mentions_name(body_type, n),
        Term::Lambda {
            param,
            param_type,
            body,
        } => param == n || mentions_name(param_type, n) || mentions_name(body, n),
        Term::Let {
            name,
            ty,
            value,
            body,
        } => name == n || mentions_name(ty, n) || mentions_name(value, n) || mentions_name(body, n),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            mentions_name(discriminant, n)
                || mentions_name(motive, n)
                || cases.iter().any(|c| mentions_name(c, n))
        }
        Term::Fix { name, body } => name == n || mentions_name(body, n),
        Term::MutualFix { defs, .. } => defs.iter().any(|(m, b)| m == n || mentions_name(b, n)),
        Term::Annotated { term, .. } => mentions_name(term, n),
    }
}

/// Congruence closure with proof production (Nieuwenhuis and Oliveras): a union-find over the goal's
/// terms, every application read curried (`f a` is the pair `f`, `a`), and a proof forest recording
/// why each merge happened, from which an equation's proof is read back.
mod congruence {
    use super::{apps, eq_term, lam, Goal};
    use crate::context::Context;
    use crate::term::Term;
    use crate::type_checker::{demanded_type_head, infer_type, occurs_free, TypeHead};
    use std::collections::HashMap;

    /// Why two terms were merged.
    #[derive(Clone)]
    enum Reason {
        /// The hypothesis `name : Eq T lhs rhs`, the merged pair being its two sides.
        Hypothesis { name: String, lhs: usize },
        /// Two applications whose functions and arguments are already equal.
        Congruence { left: usize, right: usize },
    }

    pub(super) struct Closure<'a> {
        ctx: &'a Context,
        avoid: Vec<String>,
        terms: Vec<Term>,
        types: Vec<Option<Term>>,
        index: HashMap<String, usize>,
        /// For an application `f a` whose function is not dependent: `f` and `a`.
        app: Vec<Option<(usize, usize)>>,
        parent: Vec<usize>,
        /// The proof forest: each term's edge toward its tree's root.
        edge: Vec<Option<(usize, Reason)>>,
        pending: Vec<(usize, usize, Reason)>,
    }

    impl<'a> Closure<'a> {
        pub(super) fn new(ctx: &'a Context, goal: &Goal) -> Closure<'a> {
            let mut avoid: Vec<String> = goal.hyps.iter().map(|(h, _)| h.clone()).collect();
            avoid.push("w".to_string());
            Closure {
                ctx,
                avoid,
                terms: Vec::new(),
                types: Vec::new(),
                index: HashMap::new(),
                app: Vec::new(),
                parent: Vec::new(),
                edge: Vec::new(),
                pending: Vec::new(),
            }
        }

        /// The id of `t`, interned with its subterms.
        pub(super) fn term(&mut self, t: &Term) -> usize {
            let key = format!("{t:?}");
            if let Some(&id) = self.index.get(&key) {
                return id;
            }
            let app = match t {
                Term::App(f, a) if !self.dependent(f) => {
                    let (f, a) = (self.term(f), self.term(a));
                    Some((f, a))
                }
                _ => None,
            };
            let id = self.terms.len();
            self.terms.push(t.clone());
            self.types.push(infer_type(self.ctx, t).ok());
            self.index.insert(key, id);
            self.app.push(app);
            self.parent.push(id);
            self.edge.push(None);
            id
        }

        /// Whether `f`'s result type depends on its argument (then `f a` and `f b` have different
        /// types, and no equation relates them).
        fn dependent(&self, f: &Term) -> bool {
            let Ok(ty) = infer_type(self.ctx, f) else {
                return true;
            };
            match demanded_type_head(self.ctx, ty, TypeHead::Function) {
                Term::Pi {
                    param, body_type, ..
                } => occurs_free(&body_type, &param),
                _ => true,
            }
        }

        pub(super) fn assert_hypothesis(&mut self, name: &str, lhs: &Term, rhs: &Term) {
            let (l, r) = (self.term(lhs), self.term(rhs));
            self.pending.push((
                l,
                r,
                Reason::Hypothesis {
                    name: name.to_string(),
                    lhs: l,
                },
            ));
        }

        fn find(&mut self, mut x: usize) -> usize {
            while self.parent[x] != x {
                self.parent[x] = self.parent[self.parent[x]];
                x = self.parent[x];
            }
            x
        }

        /// Merge every pending pair, and every pair of applications the merges make congruent,
        /// until nothing changes.
        pub(super) fn saturate(&mut self) {
            loop {
                while let Some((a, b, why)) = self.pending.pop() {
                    let (ra, rb) = (self.find(a), self.find(b));
                    if ra == rb {
                        continue;
                    }
                    self.parent[ra] = rb;
                    self.reroot(a);
                    self.edge[a] = Some((b, why));
                }
                // One pass of the signature table: applications whose function and argument classes
                // agree are congruent.
                let mut signatures: HashMap<(usize, usize), usize> = HashMap::new();
                for x in 0..self.terms.len() {
                    let Some((f, a)) = self.app[x] else { continue };
                    let key = (self.find(f), self.find(a));
                    match signatures.get(&key) {
                        Some(&y) if self.find(x) != self.find(y) => {
                            self.pending
                                .push((y, x, Reason::Congruence { left: y, right: x }));
                        }
                        Some(_) => {}
                        None => {
                            signatures.insert(key, x);
                        }
                    }
                }
                if self.pending.is_empty() {
                    return;
                }
            }
        }

        /// Make `x` the root of its proof tree by reversing the edges on its path to the root.
        fn reroot(&mut self, x: usize) {
            let mut path = Vec::new();
            let mut at = x;
            while let Some((next, why)) = self.edge[at].clone() {
                path.push((at, next, why));
                at = next;
            }
            self.edge[x] = None;
            for (from, to, why) in path {
                self.edge[to] = Some((from, why));
            }
        }

        /// A proof that `a` equals `b`, when they are in one class.
        pub(super) fn prove(&mut self, a: usize, b: usize) -> Option<Term> {
            if self.find(a) != self.find(b) {
                return None;
            }
            self.explain(a, b)
        }

        /// The path between `a` and `b` in the proof forest, each step's proof chained.
        fn explain(&self, a: usize, b: usize) -> Option<Term> {
            let up = |mut x: usize| {
                let mut path = vec![x];
                while let Some((next, _)) = &self.edge[x] {
                    x = *next;
                    path.push(x);
                }
                path
            };
            let (pa, pb) = (up(a), up(b));
            let meet = *pa.iter().find(|x| pb.contains(x))?;
            let mut proof = self.refl(a)?;
            let mut at = a;
            while at != meet {
                let (next, why) = self.edge[at].clone()?;
                let step = self.step(at, next, &why)?;
                proof = self.trans(at, a, proof, step)?;
                at = next;
            }
            let back: Vec<usize> = pb.iter().take_while(|x| **x != meet).copied().collect();
            for &x in back.iter().rev() {
                let (next, why) = self.edge[x].clone()?;
                let step = self.step(next, x, &why)?;
                proof = self.trans(next, a, proof, step)?;
            }
            Some(proof)
        }

        /// A proof that `from` equals `to`, the two ends of one forest edge.
        fn step(&self, from: usize, to: usize, why: &Reason) -> Option<Term> {
            match why {
                Reason::Hypothesis { name, lhs } => {
                    let h = Term::Var(name.clone());
                    if *lhs == from {
                        Some(h)
                    } else {
                        self.symm(to, h)
                    }
                }
                Reason::Congruence { left, right } => {
                    let (l, r) = if *left == from {
                        (*left, *right)
                    } else {
                        (*right, *left)
                    };
                    let ((fl, al), (fr, ar)) = (self.app[l]?, self.app[r]?);
                    // `f a = g b` as `f a = f b` (the argument) then `f b = g b` (the function).
                    let args = self.explain(al, ar)?;
                    let by_arg = self.congr_arg(fl, al, args)?;
                    if fl == fr {
                        return Some(by_arg);
                    }
                    let funs = self.explain(fl, fr)?;
                    let by_fun = self.congr_fun(fl, ar, funs)?;
                    let mid = Term::App(
                        Box::new(self.terms[fl].clone()),
                        Box::new(self.terms[ar].clone()),
                    );
                    Some(self.trans_terms(
                        &self.types[l].clone()?,
                        &self.terms[l],
                        &mid,
                        by_arg,
                        by_fun,
                    ))
                }
            }
        }

        fn refl(&self, x: usize) -> Option<Term> {
            Some(apps(
                Term::Global("refl".into()),
                &[self.types[x].clone()?, self.terms[x].clone()],
            ))
        }

        fn fresh(&self, base: &str) -> String {
            let taken = |n: &str| {
                self.avoid.iter().any(|a| a == n)
                    || self.terms.iter().any(|t| super::mentions_name(t, n))
            };
            if !taken(base) {
                return base.to_string();
            }
            (1..)
                .map(|i| format!("{base}{i}"))
                .find(|n| !taken(n))
                .expect("a fresh name")
        }

        /// From `p : Eq T x _`, the same equation the other way round.
        fn symm(&self, x: usize, p: Term) -> Option<Term> {
            let (t, xs) = (self.types[x].clone()?, self.terms[x].clone());
            let z = self.fresh("z");
            Some(Term::Match {
                discriminant: Box::new(p),
                motive: Box::new(lam(
                    &z,
                    t.clone(),
                    lam(
                        "_",
                        eq_term(&t, &xs, &Term::Var(z.clone())),
                        eq_term(&t, &Term::Var(z.clone()), &xs),
                    ),
                )),
                cases: vec![apps(Term::Global("refl".into()), &[t, xs])],
            })
        }

        /// From `p : Eq T a y` and `q : Eq T y _` (`a` and `y` by id), `Eq T a _`.
        fn trans(&self, y: usize, a: usize, p: Term, q: Term) -> Option<Term> {
            let t = self.types[a].clone()?;
            Some(self.trans_terms(&t, &self.terms[a], &self.terms[y], p, q))
        }

        /// From `p : Eq t a y` and `q : Eq t y _`, `Eq t a _`: transport `p` along `q`.
        fn trans_terms(&self, t: &Term, a: &Term, y: &Term, p: Term, q: Term) -> Term {
            let w = self.fresh("w");
            Term::Match {
                discriminant: Box::new(q),
                motive: Box::new(lam(
                    &w,
                    t.clone(),
                    lam(
                        "_",
                        eq_term(t, y, &Term::Var(w.clone())),
                        eq_term(t, a, &Term::Var(w.clone())),
                    ),
                )),
                cases: vec![p],
            }
        }

        /// From `p : Eq A a _`, `Eq B (f a) (f _)`.
        fn congr_arg(&self, f: usize, a: usize, p: Term) -> Option<Term> {
            let fa = Term::App(
                Box::new(self.terms[f].clone()),
                Box::new(self.terms[a].clone()),
            );
            let ta = self.types[a].clone()?;
            let tb = infer_type(self.ctx, &fa).ok()?;
            let w = self.fresh("w");
            let at_w = Term::App(
                Box::new(self.terms[f].clone()),
                Box::new(Term::Var(w.clone())),
            );
            Some(Term::Match {
                discriminant: Box::new(p),
                motive: Box::new(lam(
                    &w,
                    ta.clone(),
                    lam(
                        "_",
                        eq_term(&ta, &self.terms[a], &Term::Var(w.clone())),
                        eq_term(&tb, &fa, &at_w),
                    ),
                )),
                cases: vec![apps(Term::Global("refl".into()), &[tb, fa.clone()])],
            })
        }

        /// From `p : Eq (A → B) f _`, `Eq B (f b) (_ b)`.
        fn congr_fun(&self, f: usize, b: usize, p: Term) -> Option<Term> {
            let tf = self.types[f].clone()?;
            let fb = Term::App(
                Box::new(self.terms[f].clone()),
                Box::new(self.terms[b].clone()),
            );
            let tb = infer_type(self.ctx, &fb).ok()?;
            let w = self.fresh("w");
            let w_b = Term::App(
                Box::new(Term::Var(w.clone())),
                Box::new(self.terms[b].clone()),
            );
            Some(Term::Match {
                discriminant: Box::new(p),
                motive: Box::new(lam(
                    &w,
                    tf.clone(),
                    lam(
                        "_",
                        eq_term(&tf, &self.terms[f], &Term::Var(w.clone())),
                        eq_term(&tb, &fb, &w_b),
                    ),
                )),
                cases: vec![apps(Term::Global("refl".into()), &[tb, fb.clone()])],
            })
        }
    }
}
