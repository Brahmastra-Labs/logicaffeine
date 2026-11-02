//! Weak-head reduction for lazy δ conversion (`type_checker::lazy_delta`).
//!
//! Lean's `whnf_core`: reduce only at the HEAD, with every rule the kernel's normalizer uses at the
//! head — β, ζ, ι on a constructor (after the discriminant or structural argument is itself put in
//! weak-head normal form, δ included), K for a one-case family, `Quot.lift` on `Quot.mk` (after
//! the major premise is put in weak-head normal form), and the registered literal computations
//! (after their arguments are) — but never δ at the head itself: unfolding the head constant is
//! the conversion loop's decision, one step at a time. So a term this leaves stuck is stuck for the
//! normalizer too, as far as its head is concerned: that is what lets the loop refute two stuck
//! terms (R4). Each rule is the reducer's own (`iota_branch`,
//! `k_constructor_form`, `try_primitive_reduce`, `const_step`, …); nothing here is a new reduction
//! rule, so every result is a reduct of its input and definitionally equal to it.
//!
//! Every step spends from one allowance, and no term deeper than the current stack supports is
//! ever produced; running out of either is `Exhausted`, never an answer.
use super::*;
use crate::reduction::{CertifiedProgram, ProgramShape};

/// The allowance or the stack-depth bound ran out; the caller keeps its ordinary path.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Exhausted;

/// One lazy-δ conversion's shared allowance: weak-head and δ steps, and the rungs of the stack it
/// runs on. A rung is the kernel's own unit of stack (`term_store::BYTES_PER_RUNG`, one level of
/// term depth); the stack a thread was sized for has `total` of them. Comparisons enclosing this
/// one have `taken` some, the weak-head reductions nested inside it (a discriminant's, a
/// fixpoint's major premise) take one each, and a term it holds takes one per level of its depth.
pub(crate) struct Allowance {
    steps: usize,
    total: usize,
    taken: usize,
    nesting: usize,
    wanted: Option<usize>,
}

impl Allowance {
    /// `steps` weak-head and δ steps on this thread's whole stack.
    pub(crate) fn new(steps: usize) -> Self {
        Allowance {
            steps,
            total: crate::term_store::kernel_stack_rungs(),
            taken: 0,
            nesting: 0,
            wanted: None,
        }
    }

    /// The same allowance for a computation nested `taken` rungs deep in enclosing ones.
    pub(crate) fn below(mut self, taken: usize) -> Self {
        self.taken = taken;
        self
    }

    /// The rungs a larger stack must have when only the stack this computation runs on was too
    /// small for it — a reduct, or a nesting of weak-head reductions, that the enclosing
    /// computations leave no room for, though the kernel's term-depth door admits it. `None`: the
    /// steps ran out, or the stack already has as many rungs as the door allows.
    pub(crate) fn wanted_depth(&self) -> Option<usize> {
        self.wanted
    }

    fn room(&self) -> usize {
        self.total.saturating_sub(self.taken + self.nesting)
    }

    /// Enter one nested weak-head reduction: one rung of this stack.
    fn nest(&mut self) -> Result<(), Exhausted> {
        if self.room() == 0 {
            if self.total < crate::type_checker::MAX_TERM_DEPTH {
                self.wanted = Some((self.total * 2).min(crate::type_checker::MAX_TERM_DEPTH));
            }
            return Err(Exhausted);
        }
        self.nesting += 1;
        Ok(())
    }

    fn spend(&mut self) -> Result<(), Exhausted> {
        self.steps = self.steps.checked_sub(1).ok_or(Exhausted)?;
        Ok(())
    }

    fn admit(&mut self, term: Term) -> Result<Term, Exhausted> {
        if !crate::type_checker::depth_exceeds(&term, self.room()) {
            return Ok(term);
        }
        if !crate::type_checker::depth_exceeds(&term, crate::type_checker::MAX_TERM_DEPTH) {
            let needed = self.taken + self.nesting + crate::term_store::term_depth(&term);
            if needed <= crate::type_checker::MAX_TERM_DEPTH
                && self.total < crate::type_checker::MAX_TERM_DEPTH
            {
                self.wanted = Some(
                    needed
                        .max(self.total * 2)
                        .min(crate::type_checker::MAX_TERM_DEPTH),
                );
            }
        }
        Err(Exhausted)
    }
}

fn reapply(head: Term, args: &[&Term]) -> Term {
    args.iter()
        .fold(head, |f, a| Term::App(Box::new(f), Box::new((*a).clone())))
}

/// One weak-head step at the head, WITHOUT δ at the head. `Ok(None)`: no head redex.
fn core_step(
    ctx: &Context,
    term: &Term,
    allowance: &mut Allowance,
) -> Result<Option<Term>, Exhausted> {
    allowance.spend()?;
    match term {
        Term::Let {
            name, value, body, ..
        } => Ok(Some(substitute(body, name, value))),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            let exposed = whnf(ctx, discriminant, allowance)?;
            if let Some(branch) = iota_branch(ctx, &exposed, motive, cases) {
                return Ok(Some(branch));
            }
            if cases.len() == 1 {
                if let Some(constructor) = k_constructor_form(ctx, &exposed, motive) {
                    if let Some(branch) = iota_branch(ctx, &constructor, motive, cases) {
                        return Ok(Some(branch));
                    }
                }
            }
            Ok(None)
        }
        Term::App(func, arg) => {
            // The computation rules the reducer tries at an application, in its order.
            if let Some(result) = try_primitive_reduce(ctx, func, arg, Transparency::All) {
                if result != *term {
                    return Ok(Some(result));
                }
            }
            if let Some(result) = try_reflection_reduce(ctx, func, arg) {
                if result != *term {
                    return Ok(Some(result));
                }
            }
            if let Term::Global(name) = func.as_ref() {
                if name == "reduceBool"
                    && ctx.get_definition_body(name).is_none()
                    && declaration_agrees_with_the_rule(ctx, name)
                {
                    if let Some(value) = crate::eval::eval_bool(ctx, arg) {
                        return Ok(Some(Term::Global(
                            if value { "true" } else { "false" }.into(),
                        )));
                    }
                }
            }
            if let Some(result) = try_quot_lift_reduce(ctx, func, arg) {
                if result != *term {
                    return Ok(Some(result));
                }
            }
            if let Some(result) = primitive_on_weak_head_arguments(ctx, term, allowance)? {
                return Ok(Some(result));
            }
            if let Some(result) = quotient_on_weak_head_major(ctx, func, arg, allowance)? {
                return Ok(Some(result));
            }
            let (head, args) = app_spine(term);
            match head {
                Term::Lambda { param, body, .. } => {
                    Ok(Some(reapply(substitute(body, param, args[0]), &args[1..])))
                }
                Term::Let {
                    name, value, body, ..
                } => Ok(Some(reapply(substitute(body, name, value), &args))),
                Term::Match { .. } => {
                    Ok(core_step(ctx, head, allowance)?.map(|head| reapply(head, &args)))
                }
                Term::Fix { name, body } => {
                    // ι for a fixpoint, at the column the termination guard admits.
                    let Some(&column) = crate::termination::structural_columns(ctx, body).first()
                    else {
                        return Ok(None);
                    };
                    let Some(argument) = args.get(column) else {
                        return Ok(None);
                    };
                    let major = whnf(ctx, argument, allowance)?;
                    if !is_constructor_form(ctx, &major) {
                        return Ok(None);
                    }
                    let unfolded = substitute(body, name, head);
                    let mut spine: Vec<&Term> = args.clone();
                    spine[column] = &major;
                    Ok(Some(reapply(unfolded, &spine)))
                }
                Term::MutualFix { defs, index } => {
                    let major = whnf(ctx, args[0], allowance)?;
                    if !is_constructor_form(ctx, &major) {
                        return Ok(None);
                    }
                    let mut unfolded = defs[*index].1.clone();
                    for (j, (sibling, _)) in defs.iter().enumerate() {
                        let projection = Term::MutualFix {
                            defs: defs.clone(),
                            index: j,
                        };
                        unfolded = substitute(&unfolded, sibling, &projection);
                    }
                    let mut spine: Vec<&Term> = args.clone();
                    spine[0] = &major;
                    Ok(Some(reapply(unfolded, &spine)))
                }
                _ => Ok(None),
            }
        }
        _ => Ok(None),
    }
}

/// Lean's `reduce_nat`: a body-less primitive operation computes on the weak-head normal forms of
/// its arguments, not only on arguments already written as literals — the normalizer reaches the
/// same literals by normalizing the arguments first. `Ok(None)`: not such an operation, or its
/// arguments do not reach the literals it computes on.
fn primitive_on_weak_head_arguments(
    ctx: &Context,
    term: &Term,
    allowance: &mut Allowance,
) -> Result<Option<Term>, Exhausted> {
    let (head, args) = app_spine(term);
    let Term::Global(op) = head else {
        return Ok(None);
    };
    let Some(signature) = primitive_signature(op) else {
        return Ok(None);
    };
    if ctx.get_definition_body(op).is_some() || signature.len() != args.len() + 1 {
        return Ok(None);
    }
    let mut exposed = Vec::with_capacity(args.len());
    for argument in &args {
        exposed.push(whnf(ctx, argument, allowance)?);
    }
    if exposed
        .iter()
        .zip(&args)
        .all(|(after, before)| after == *before)
    {
        return Ok(None);
    }
    let rebuilt = reapply(head.clone(), &exposed.iter().collect::<Vec<_>>());
    let Term::App(func, arg) = &rebuilt else {
        return Ok(None);
    };
    Ok(try_primitive_reduce(ctx, func, arg, Transparency::All).filter(|result| *result != rebuilt))
}

/// Lean's quotient reduction: `Quot.lift f h q` (and `Quot.ind`) put the major premise `q` in weak-head
/// normal form before looking for `Quot.mk`. `Ok(None)`: not a saturated quotient eliminator, or
/// its major premise does not reach `Quot.mk`.
fn quotient_on_weak_head_major(
    ctx: &Context,
    func: &Term,
    arg: &Term,
    allowance: &mut Allowance,
) -> Result<Option<Term>, Exhausted> {
    let (head, fargs) = app_spine(func);
    let arity = match head {
        Term::Global(name) => match crate::quotient::prelude_kind(ctx, name) {
            Some(crate::quotient::PreludeKind::Lift) => 5,
            Some(crate::quotient::PreludeKind::Induction) => 4,
            _ => return Ok(None),
        },
        Term::Const { name, levels } => match crate::quotient::kind(ctx, name, levels) {
            Some(crate::quotient::Kind::Lift) => 5,
            Some(crate::quotient::Kind::Induction) => 4,
            _ => return Ok(None),
        },
        _ => return Ok(None),
    };
    if fargs.len() != arity {
        return Ok(None);
    }
    let major = whnf(ctx, arg, allowance)?;
    if major == *arg {
        return Ok(None);
    }
    Ok(try_quot_lift_reduce(ctx, func, &major))
}

/// Weak-head normal form WITHOUT δ at the head.
pub(crate) fn whnf_core(
    ctx: &Context,
    term: Term,
    allowance: &mut Allowance,
) -> Result<Term, Exhausted> {
    let mut current = term;
    while let Some(next) = core_step(ctx, &current, allowance)? {
        current = allowance.admit(next)?;
    }
    Ok(current)
}

/// Full weak-head normal form: [`whnf_core`], then one δ step at the head, until neither applies.
pub(crate) fn whnf(
    ctx: &Context,
    term: &Term,
    allowance: &mut Allowance,
) -> Result<Term, Exhausted> {
    allowance.nest()?;
    let result = whnf_nested(ctx, term, allowance);
    allowance.nesting -= 1;
    result
}

fn whnf_nested(ctx: &Context, term: &Term, allowance: &mut Allowance) -> Result<Term, Exhausted> {
    let mut current = whnf_core(ctx, term.clone(), allowance)?;
    loop {
        if let Some(value) = certified_value(ctx, &current, allowance)? {
            return Ok(value);
        }
        match unfold_head(ctx, &current, allowance)? {
            Some(next) => current = next,
            None => return Ok(current),
        }
    }
}

/// The natural number a weak-head value spells: a literal, `Zero`, or successors of one.
fn numeral(term: &Term) -> Option<crate::BigInt> {
    let mut successors = 0u64;
    let mut current = term;
    loop {
        match current {
            Term::Lit(Literal::Nat(n)) if !n.is_negative() => {
                return Some(n.add(&crate::BigInt::from_u64(successors)))
            }
            Term::Global(name) if name == "Zero" => {
                return Some(crate::BigInt::from_u64(successors))
            }
            Term::App(f, a) if matches!(f.as_ref(), Term::Global(name) if name == "Succ") => {
                successors += 1;
                current = a;
            }
            _ => return None,
        }
    }
}

/// Lean's `reduce_nat` (its kernel's `whnf` and `lazy_delta_reduction` both ask it before a δ step):
/// a program the evaluator certifies — an arithmetic operation, a natural order or equality, a
/// division program on a pair — applied to operands whose weak-head normal forms are numerals is
/// its value, computed rather than unfolded. The certificate is the evaluator's own
/// ([`crate::reduction::certified_program`]), so no name grants anything; `Ok(None)`: not such an
/// application.
pub(crate) fn certified_value(
    ctx: &Context,
    term: &Term,
    allowance: &mut Allowance,
) -> Result<Option<Term>, Exhausted> {
    let (head, args) = app_spine(term);
    let Term::Global(name) = head else {
        return Ok(None);
    };
    let Some(shape) = crate::reduction::program_shape(ctx, name, args.len()) else {
        return Ok(None);
    };
    let Some(program) = crate::reduction::certified_program(ctx, name) else {
        return Ok(None);
    };
    let division = matches!(program, CertifiedProgram::Division { .. });
    let (x, y) = match shape {
        ProgramShape::Binary if !division => (
            whnf(ctx, args[0], allowance)?,
            whnf(ctx, args[1], allowance)?,
        ),
        ProgramShape::Pair { pair, parameters } if division => {
            let observed = whnf(ctx, args[0], allowance)?;
            let (constructor, fields) = app_spine(&observed);
            if !matches!(constructor, Term::Global(n) | Term::Const { name: n, .. } if *n == pair)
                || fields.len() != parameters + 2
            {
                return Ok(None);
            }
            (
                whnf(ctx, fields[parameters], allowance)?,
                whnf(ctx, fields[parameters + 1], allowance)?,
            )
        }
        _ => return Ok(None),
    };
    let (Some(x), Some(y)) = (numeral(&x), numeral(&y)) else {
        return Ok(None);
    };
    Ok(crate::reduction::certified_application(&program, &x, &y))
}

/// One δ step at the head, then [`whnf_core`]. `Ok(None)`: the head is not δ-reducible.
pub(crate) fn unfold_head(
    ctx: &Context,
    term: &Term,
    allowance: &mut Allowance,
) -> Result<Option<Term>, Exhausted> {
    let Some(unfolded) = delta_step(ctx, term) else {
        return Ok(None);
    };
    allowance.spend()?;
    let unfolded = allowance.admit(unfolded)?;
    Ok(Some(whnf_core(ctx, unfolded, allowance)?))
}

/// The β-normal form of `term`: every `(λx. b) a` anywhere in it, under binders and in every
/// branch, contracted — and nothing else (no δ, ι or ζ), so it never unfolds a definition. `None`
/// when `steps` contractions or the kernel's depth door do not suffice.
pub(crate) fn beta_normal_form(term: &Term, steps: &mut usize) -> Option<Term> {
    fn go(t: &Term, steps: &mut usize) -> Option<Term> {
        let b = |t: &Term, steps: &mut usize| go(t, steps).map(Box::new);
        Some(match t {
            Term::App(f, a) => {
                let f = go(f, steps)?;
                let a = go(a, steps)?;
                if let Term::Lambda { param, body, .. } = f {
                    *steps = steps.checked_sub(1)?;
                    let contracted = substitute(&body, &param, &a);
                    if crate::type_checker::depth_exceeds(
                        &contracted,
                        crate::type_checker::MAX_TERM_DEPTH,
                    ) {
                        return None;
                    }
                    go(&contracted, steps)?
                } else {
                    Term::App(Box::new(f), Box::new(a))
                }
            }
            Term::Pi {
                param,
                param_type,
                body_type,
            } => Term::Pi {
                param: param.clone(),
                param_type: b(param_type, steps)?,
                body_type: b(body_type, steps)?,
            },
            Term::Lambda {
                param,
                param_type,
                body,
            } => Term::Lambda {
                param: param.clone(),
                param_type: b(param_type, steps)?,
                body: b(body, steps)?,
            },
            Term::Let {
                name,
                ty,
                value,
                body,
            } => Term::Let {
                name: name.clone(),
                ty: b(ty, steps)?,
                value: b(value, steps)?,
                body: b(body, steps)?,
            },
            Term::Match {
                discriminant,
                motive,
                cases,
            } => Term::Match {
                discriminant: b(discriminant, steps)?,
                motive: b(motive, steps)?,
                cases: cases.iter().map(|c| go(c, steps)).collect::<Option<_>>()?,
            },
            Term::Fix { name, body } => Term::Fix {
                name: name.clone(),
                body: b(body, steps)?,
            },
            Term::MutualFix { defs, index } => Term::MutualFix {
                defs: defs
                    .iter()
                    .map(|(n, d)| Some((n.clone(), go(d, steps)?)))
                    .collect::<Option<_>>()?,
                index: *index,
            },
            other => other.clone(),
        })
    }
    go(term, steps)
}

/// The name of the δ-reducible constant at the head of `term`'s application spine, if any.
pub(crate) fn delta_head<'t>(ctx: &Context, term: &'t Term) -> Option<&'t str> {
    let (head, _) = app_spine(term);
    match head {
        Term::Global(name) if ctx.get_definition_body(name).is_some() => Some(name),
        Term::Const { name, levels } => match const_step(ctx, name, levels) {
            ConstStep::Delta(..) => Some(name),
            _ => None,
        },
        _ => None,
    }
}

/// One δ step at the head of `term`'s application spine: the reducer's own unfolding of a `Global`
/// (its registered body) or a `Const` (its body at the given levels). `None`: no δ at the head.
pub(crate) fn delta_step(ctx: &Context, term: &Term) -> Option<Term> {
    let (head, args) = app_spine(term);
    let unfolded = match head {
        Term::Global(name) => ctx.get_definition_body(name)?.clone(),
        Term::Const { name, levels } => match const_step(ctx, name, levels) {
            ConstStep::Delta(substitution, body) => {
                crate::term::instantiate_universes(&body, &substitution)
            }
            _ => return None,
        },
        _ => return None,
    };
    Some(reapply(unfolded, &args))
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/whnf_core/controls.rs"]
mod controls;
