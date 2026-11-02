//! Lean's lazy δ-reduction for the primary kernel's conversion.
//!
//! Lean's kernel never fully normalizes to decide `a ≡ b`. It puts both sides in weak-head normal
//! form without δ, compares heads, and otherwise unfolds ONE side by a single δ step — the side whose
//! head constant has the greater definitional height, both when the heights are equal — trying
//! argument congruence first when both heads are the same constant. Theorems are unfolded last
//! (LF's lazy theorem delta, the 2026-09-23 ruling); `Opaque` declarations have no body and never
//! unfold. This module is that loop, placed in front of `def_eq_uncached`'s full-normalization
//! fallback.
//!
//! What it may answer, and why each answer is sound:
//! * `Some(true)`: from `alpha_eq`, a literal bridge, congruence (rigid, binder, same-constant,
//!   rigid-variable or fixpoint heads, and two stuck `match`es), η or structure η — each an
//!   existing rule of this kernel, applied to reducts of the inputs (every step is a rule the
//!   normalizer itself uses, so each side stays definitionally equal to where it started).
//! * `Some(true)` also from unit η on two stuck terms (the fallback's own rule, Lean's
//!   `is_def_eq_unit_like`).
//! * A literal met against a constructor application is compared as the constructor spelling the
//!   kernel's literal rule gives it (`0` is `Zero`, `n + 1` is `Succ n`, an integer literal its `Int`
//!   constructor) — the fallback's `nat_peano_equal` and `literal_constructor`, one constructor at
//!   a time — so every rule below then sees two constructor applications.
//! * Before a δ step, a side that applies a program the evaluator certifies to numerals is replaced
//!   by its value — Lean's `reduce_nat` (`reduction::certified_value`, also asked by `whnf`); the
//!   value is what the program's certified recurrence computes, so the side keeps its meaning.
//! * `Some(false)`: from `rigid_conversion` on two applications of the same inductive or
//!   constructor, the kernel's existing complete rule for rigid heads (the caller has already run
//!   `constructors_disagree` — and the loop runs it too, once, on a closed pair met inside the
//!   comparison before its first δ step), from `natural_literal_conversion` (a natural literal
//!   against a finished closed natural computation — the rule `def_eq_uncached` itself answers
//!   with), and — phase 2, approved 2026-09-24 — from Lean's closing
//!   `return false` in exactly four shapes: two binders whose domains or bodies differ (R3), two
//!   stuck terms whose rigid heads differ (R1), one local variable or inert constant (an axiom or
//!   opaque constant no rule computes with) applied to arguments that differ (R2), and — its own
//!   later step — stuck `match`es and fixpoints that are stuck for good on local variables or inert
//!   constants and that congruence cannot equate (R4; `whnf_core` is Lean-complete, so
//!   what it leaves stuck the normalizer leaves stuck). Each is decided only after every positive rule has failed and unit η has been
//!   asked; each head is one no reduction can change, so the fallback's normal forms keep it and
//!   would answer `false` too; every nested `false` comes from a complete `def_eq`, which records
//!   its own resource stops. A `false` can only ever refuse a proof, never admit one.
//! * `None`: undecided — the caller runs its unchanged normalization fallback.
//!
//! The differential below (`LOGOS_LAZY_DELTA_DIFFERENTIAL`) re-decides every answer by the fallback
//! and aborts on a disagreement, so a refutation that the fallback would overturn cannot pass a
//! run of the suite unnoticed.
//!
//! The loop spends one allowance of weak-head and δ steps. A reduct deeper than the stack the call
//! runs on is not an exhausted allowance: the pair is decided again on a stack sized for it, as the
//! normalizer grows its own stack (`term_store::grow_stack_for_depth`).
use super::*;
use crate::reduction::{
    certified_value, delta_head, natural_literal_conversion, unfold_head, whnf, whnf_core,
    Allowance, Exhausted,
};

/// The weak-head and δ steps one conversion may spend here before handing the pair back.
const LAZY_DELTA_STEPS: usize = 1 << 12;

struct Unfold {
    theorem: bool,
    height: u32,
}

impl Unfold {
    fn of(ctx: &Context, name: &str) -> Unfold {
        Unfold {
            theorem: is_transparent_theorem(ctx, name),
            height: ctx.definition_height(name),
        }
    }
}

/// Which side(s) to unfold: theorems last, then the greater height, both on a tie.
fn unfold_order(left: &Unfold, right: &Unfold) -> (bool, bool) {
    match (left.theorem, right.theorem) {
        (false, true) => (true, false),
        (true, false) => (false, true),
        _ => match left.height.cmp(&right.height) {
            std::cmp::Ordering::Greater => (true, false),
            std::cmp::Ordering::Less => (false, true),
            std::cmp::Ordering::Equal => (true, true),
        },
    }
}

thread_local! {
    /// The pair (by address) that a nested comparison hands to the full `def_eq` because this loop
    /// left it open. When `def_eq_uncached` reaches the loop for that very pair, the loop is not
    /// run again: it would leave it open again, after redoing — and, through its own nested
    /// comparisons, re-entering — all of its work, which doubles at every level of a term.
    static LEFT_OPEN: std::cell::Cell<Option<(usize, usize)>> = const { std::cell::Cell::new(None) };
}

fn address(a: &Term, b: &Term) -> (usize, usize) {
    (a as *const Term as usize, b as *const Term as usize)
}

/// The conversion of a pair met INSIDE the loop (an argument, a binder body, a branch): Lean's
/// `is_def_eq` on it — lazy δ first, so a pair the loop decides never pays for the probes and the
/// normalization fallback of a full `def_eq`. Its answer is `def_eq`'s: a `true` from the loop is a
/// proof; a `false` from the loop is one of its definite refutations, which no sound probe can
/// overturn except proof irrelevance — asked here, as Lean asks it before anything else; and a pair
/// the loop leaves open gets the full `def_eq`. Positive answers are remembered by the conversion
/// session exactly as `def_eq`'s are.
///
/// Each nesting takes one rung of this thread's stack, exactly as `def_eq`'s do
/// (`within_conversion_rungs`): a deeper nesting continues on a larger stack, and one past the
/// kernel's term-depth door is a recorded resource stop, never a crash.
fn nested_eq(ctx: &Context, a: &Term, b: &Term) -> bool {
    within_conversion_rungs(|| {
        crate::conversion_session::convert(ctx, a, b, || {
            if alpha_eq(a, b) {
                return true;
            }
            match lazy_delta_conversion(ctx, a, b) {
                Some(true) => true,
                Some(false) => proof_irrelevant(ctx, a, b),
                None => {
                    let outer = LEFT_OPEN.with(|open| open.replace(Some(address(a, b))));
                    let equal = def_eq(ctx, a, b);
                    LEFT_OPEN.with(|open| open.set(outer));
                    equal
                }
            }
        })
    })
}

/// Binder congruence with the loop's own nested conversion: the domains, then the bodies under a
/// fresh variable. Complete for two binders of the same kind.
fn binder_congruence(ctx: &Context, a: &Term, b: &Term) -> bool {
    let (p, pt, pb, q, qt, qb) = match (a, b) {
        (
            Term::Lambda {
                param: p,
                param_type: pt,
                body: pb,
            },
            Term::Lambda {
                param: q,
                param_type: qt,
                body: qb,
            },
        )
        | (
            Term::Pi {
                param: p,
                param_type: pt,
                body_type: pb,
            },
            Term::Pi {
                param: q,
                param_type: qt,
                body_type: qb,
            },
        ) => (p, pt, pb, q, qt, qb),
        _ => return false,
    };
    if !nested_eq(ctx, pt, qt) {
        return false;
    }
    let name = fresh_conversion_name(Some(ctx), &[pb, qb, pt, qt]);
    let variable = Term::Var(name.clone());
    let lhs = substitute(pb, p, &variable);
    let rhs = substitute(qb, q, &variable);
    nested_eq(&ctx.extend(&name, (**pt).clone()), &lhs, &rhs)
}

/// `rigid_conversion` with the loop's own nested conversion: two applications of the same
/// inductive or constructor are equal exactly when their arguments are (a constructor's proofs
/// are equal outright by proof irrelevance). `None` when the heads are not one such rigid head.
fn rigid_arguments(ctx: &Context, a: &Term, b: &Term) -> Option<bool> {
    let (ah, aa) = spine_of(a);
    let (bh, ba) = spine_of(b);
    if aa.is_empty() || aa.len() != ba.len() || !alpha_eq(ah, bh) {
        return None;
    }
    let name = match ah {
        Term::Global(n) | Term::Const { name: n, .. } => n,
        _ => return None,
    };
    if !ctx.is_inductive(name) && !ctx.is_constructor(name) {
        return None;
    }
    if ctx.is_constructor(name) && proof_irrelevant(ctx, a, b) {
        return Some(true);
    }
    Some(aa.iter().zip(&ba).all(|(x, y)| nested_eq(ctx, x, y)))
}

/// `f a₁ … aₙ ≡ f b₁ … bₙ` for the same head `f` when every `aᵢ ≡ bᵢ` (congruence; a positive
/// rule only).
fn same_head_arguments(ctx: &Context, a: &Term, b: &Term) -> bool {
    let (ah, aa) = spine_of(a);
    let (bh, ba) = spine_of(b);
    !aa.is_empty()
        && aa.len() == ba.len()
        && alpha_eq(ah, bh)
        && aa.iter().zip(&ba).all(|(x, y)| nested_eq(ctx, x, y))
}

/// Two stuck terms equal by congruence: the same rigid head (a variable, an axiom, a fixpoint)
/// applied to convertible arguments, or two stuck `match`es — convertible discriminants, motives
/// and branches — applied to convertible arguments. Lean compares stuck recursor applications
/// this way (`is_def_eq_app`). A positive rule only.
fn stuck_congruence(ctx: &Context, a: &Term, b: &Term) -> bool {
    let (ah, aa) = spine_of(a);
    let (bh, ba) = spine_of(b);
    if aa.len() != ba.len() {
        return false;
    }
    let heads = match (ah, bh) {
        (
            Term::Match {
                discriminant: d1,
                motive: m1,
                cases: c1,
            },
            Term::Match {
                discriminant: d2,
                motive: m2,
                cases: c2,
            },
        ) => {
            c1.len() == c2.len()
                && nested_eq(ctx, d1, d2)
                && nested_eq(ctx, m1, m2)
                && c1.iter().zip(c2).all(|(x, y)| nested_eq(ctx, x, y))
        }
        // As the fallback compares two fixpoints: their bodies, with the recursive name opened as
        // one fresh variable on both sides.
        (Term::Fix { name: n1, body: b1 }, Term::Fix { name: n2, body: b2 }) => {
            let (_, left, right) = open_conversion_binders(Some(ctx), n1, b1, n2, b2);
            nested_eq(ctx, &left, &right)
        }
        _ => !aa.is_empty() && alpha_eq(ah, bh),
    };
    heads && aa.iter().zip(&ba).all(|(x, y)| nested_eq(ctx, x, y))
}

/// A constant no rule of this kernel ever computes with — Lean's axioms and opaque constants: a
/// declared name with no body to unfold, not a constructor or an inductive type (rigid heads of
/// their own), and no rule that fires on the name ([`computes_by_name`]). An application of one is
/// neutral for good: every normal form keeps it, with its head.
fn inert(ctx: &Context, head: &Term) -> bool {
    match head {
        Term::Global(name) => {
            ctx.get_global(name).is_some()
                && ctx.get_definition_body(name).is_none()
                && ctx.get_universe_poly(name).is_none()
                && !ctx.is_constructor(name)
                && !ctx.is_inductive(name)
                && !computes_by_name(ctx, name)
        }
        Term::Const { name, levels } => {
            !ctx.is_constructor(name)
                && !ctx.is_inductive(name)
                && crate::reduction::const_is_value(ctx, name, levels)
                && !computes_by_name(ctx, name)
        }
        _ => false,
    }
}

/// Whether a rule fires on the name itself: a registered primitive, the native-reduction hook, a
/// reflection rule, or a quotient primitive (the prelude's or a registered one, at any levels).
fn computes_by_name(ctx: &Context, name: &str) -> bool {
    crate::reduction::computes_by_name(name)
        || crate::quotient::prelude_kind(ctx, name).is_some()
        || ctx.quotient_primitives.contains_key(name)
}

/// Stuck for good on a local variable or an inert constant: the variable itself (applied or not),
/// an application of an inert constant, or a `match` whose discriminant — or a fixpoint application
/// whose structural argument — is, after weak-head reduction with δ, itself stuck for good. No
/// reduction can ever move the head of such a term, so the normalizer's normal form keeps the same
/// `match`, fixpoint, variable or constant at its head.
fn stuck_on_a_local(
    ctx: &Context,
    term: &Term,
    allowance: &mut Allowance,
) -> Result<bool, Exhausted> {
    let mut current = term.clone();
    loop {
        let (head, args) = spine_of(&current);
        let blocking = match head {
            Term::Var(_) => return Ok(true),
            head if inert(ctx, head) => return Ok(true),
            Term::Match { discriminant, .. } => (**discriminant).clone(),
            Term::Fix { body, .. } => {
                let Some(&column) = crate::termination::structural_columns(ctx, body).first()
                else {
                    return Ok(false);
                };
                let Some(argument) = args.get(column) else {
                    return Ok(false);
                };
                (*argument).clone()
            }
            _ => return Ok(false),
        };
        current = whnf(ctx, &blocking, allowance)?;
    }
}

/// R4 (approved 2026-09-24, its own step): two terms that can never move, at least one of them a
/// stuck `match` or fixpoint, that congruence could not equate — Lean's closing `return false` on
/// two stuck recursor applications. Each side is stuck for good on a local variable or an inert
/// constant ([`stuck_on_a_local`]) or has a rigid head (R1's list; a registered structure's constructor only
/// when structure η already decided the pair). The caller has asked unit η first.
fn stuck_apart(
    ctx: &Context,
    a: &Term,
    b: &Term,
    structure_eta_decided: bool,
    allowance: &mut Allowance,
) -> Result<bool, Exhausted> {
    let moving = |t: &Term| matches!(spine_of(t).0, Term::Match { .. } | Term::Fix { .. });
    if !moving(a) && !moving(b) {
        return Ok(false);
    }
    for side in [a, b] {
        match rigid_head(ctx, side) {
            Some((Rigid::Constructor(name), _))
                if !structure_eta_decided && ctx.struct_of_constructor(name).is_some() =>
            {
                return Ok(false);
            }
            Some(_) => {}
            None => {
                if !stuck_on_a_local(ctx, side, allowance)? {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

/// A head no reduction rule can ever change: a local variable, an inert constant ([`inert`]: an
/// axiom or opaque constant no rule computes with), a constructor, an inductive type former, a sort,
/// a Π, or a literal. Normalizing a term keeps its rigid head, so two terms whose rigid heads differ
/// have normal forms that differ at the head. A body-less primitive operation is not listed: it
/// computes once its arguments become literals.
#[derive(PartialEq)]
enum Rigid<'t> {
    Local(&'t str),
    Inert(&'t str),
    Constructor(&'t str),
    Inductive(&'t str),
    Sort(&'t Universe),
    Pi,
    Literal,
}

fn rigid_head<'t>(ctx: &Context, term: &'t Term) -> Option<(Rigid<'t>, usize)> {
    let (head, arguments) = spine_of(term);
    let rigid = match head {
        Term::Var(name) => Rigid::Local(name),
        // A bare name only: two universe instances of one constant are told apart by their levels,
        // which congruence compares, not by the name.
        Term::Global(name) if inert(ctx, head) => Rigid::Inert(name),
        Term::Global(name) | Term::Const { name, .. } if ctx.is_constructor(name) => {
            Rigid::Constructor(name)
        }
        Term::Global(name) | Term::Const { name, .. } if ctx.is_inductive(name) => {
            Rigid::Inductive(name)
        }
        Term::Sort(level) if arguments.is_empty() => Rigid::Sort(level),
        Term::Pi { .. } if arguments.is_empty() => Rigid::Pi,
        Term::Lit(_) if arguments.is_empty() => Rigid::Literal,
        _ => return None,
    };
    Some((rigid, arguments.len()))
}

/// R1 and R2 (phase 2, approved 2026-09-24): two stuck terms, neither with a δ-redex at its head,
/// that no positive rule could equate, whose rigid heads can never meet — two different rigid
/// heads (R1), or the same local variable or inert constant over arguments already found unequal (R2; a
/// constructor's and an inductive's are `rigid_conversion`'s). Lean ends `is_def_eq_core` with
/// `return false` exactly here. Excluded, because Lean decides them by other rules:
/// * a literal against a constructor application, or against another literal — the literal
///   bridges (`Nat`/`Int` constructors, `String.mk`, `Char`) expand the literal;
/// * a registered structure's constructor, unless structure η already decided the pair;
/// * two sorts whose levels are equivalent.
/// The caller asks unit η (Lean's `is_def_eq_unit_like`) first, and refutes only when the rule
/// is known not to apply.
fn rigid_mismatch(ctx: &Context, a: &Term, b: &Term, structure_eta_decided: bool) -> bool {
    let (Some((left, left_arity)), Some((right, right_arity))) =
        (rigid_head(ctx, a), rigid_head(ctx, b))
    else {
        return false;
    };
    let structure_constructor = |rigid: &Rigid| matches!(rigid, Rigid::Constructor(name) if ctx.struct_of_constructor(name).is_some());
    match (&left, &right) {
        (Rigid::Literal, Rigid::Literal | Rigid::Constructor(_))
        | (Rigid::Constructor(_), Rigid::Literal) => false,
        _ if !structure_eta_decided
            && (structure_constructor(&left) || structure_constructor(&right)) =>
        {
            false
        }
        (Rigid::Sort(u), Rigid::Sort(v)) => !u.equiv(v),
        (Rigid::Local(x), Rigid::Local(y)) | (Rigid::Inert(x), Rigid::Inert(y)) if x == y => {
            left_arity == right_arity
        }
        _ => left != right,
    }
}

/// Unit η, as the fallback's `def_eq_normalized` and Lean's `is_def_eq_unit_like` decide it: two
/// terms of one type whose family is a non-indexed, fieldless singleton (`PUnit`, `Unit`) are
/// equal. A term whose type does not infer is outside the rule, as the fallback's own unit-η step
/// has it (it asks `infer_type` and skips the rule on a refusal) — a comparison of two fixpoint
/// bodies names the recursive function without a type. `Some(false)`: the rule does not apply.
/// `None`: the type inferred but its head could not be exposed within the allowance, so nothing
/// is decided.
fn unit_eta(ctx: &Context, a: &Term, b: &Term) -> Option<bool> {
    if !can_have_unit_type(ctx, a) || !can_have_unit_type(ctx, b) {
        return Some(false);
    }
    let Ok(a_type) = infer_type(ctx, a) else {
        return Some(false);
    };
    let exposed = whnf(
        ctx,
        &a_type,
        &mut Allowance::new(LAZY_DELTA_STEPS).below(conversion_nesting()),
    )
    .ok()?;
    let (head, _) = spine_of(&exposed);
    let unit = matches!(head, Term::Global(name) | Term::Const { name, .. } if ctx.unit_constructor(name).is_some());
    if !unit {
        return Some(false);
    }
    let Ok(b_type) = infer_type(ctx, b) else {
        return Some(false);
    };
    Some(nested_eq(ctx, &a_type, &b_type))
}

/// `literal` as the constructor application the kernel's literal rule makes it, when `other` is a
/// constructor application: a natural literal `0` is `Zero` and `n + 1` is `Succ n` (K6), an
/// integer literal is its `Int` constructor. The spelling is the literal itself by that rule, so
/// comparing it is comparing the literal.
fn literal_spelled_for(ctx: &Context, literal: &Term, other: &Term) -> Option<Term> {
    if !matches!(rigid_head(ctx, other), Some((Rigid::Constructor(_), _))) {
        return None;
    }
    match literal {
        Term::Lit(Literal::Nat(n))
            if !n.is_negative() && crate::int_inductive::reads_nat_literal(ctx, n) =>
        {
            Some(if n.is_zero() {
                Term::Global("Zero".into())
            } else {
                Term::App(
                    Box::new(Term::Global("Succ".into())),
                    Box::new(Term::Lit(Literal::Nat(n.sub(&crate::BigInt::from_i64(1))))),
                )
            })
        }
        Term::Lit(_) => crate::int_inductive::literal_constructor(ctx, literal),
        _ => None,
    }
}

/// η: `λx. body ≡ other` when `body ≡ other x` under a fresh `x` (positive only).
fn eta(ctx: &Context, lambda: &Term, other: &Term) -> bool {
    let Term::Lambda {
        param,
        param_type,
        body,
    } = lambda
    else {
        return false;
    };
    let name = fresh_conversion_name(Some(ctx), &[body, other, param_type]);
    let variable = Term::Var(name.clone());
    let body = substitute(body, param, &variable);
    let extended = ctx.extend(&name, (**param_type).clone());
    nested_eq(
        &extended,
        &body,
        &Term::App(Box::new(other.clone()), Box::new(variable)),
    )
}

/// Lean's lazy δ conversion. See the module documentation for what each answer means.
///
/// With registered rewrite rules (`@[simp]`) the loop still answers `true`, never `false`: every
/// step it takes is a β/δ/ι/ζ reduction the rules leave valid — adding rules only adds equalities —
/// so an equality found without them holds with them, and it never depended on a rule, so no rule
/// outlives its scope through it. A refutation, though, could be overturned by a rule, so with
/// rules every `false` is left to the fallback, which rewrites (KB's finding: one unrelated rule
/// shut the loop off entirely and made registering a δ-chain quadratic).
pub(super) fn lazy_delta_conversion(ctx: &Context, a: &Term, b: &Term) -> Option<bool> {
    lazy(ctx, a, b, Observe::ClosedData)
}

/// [`lazy_delta_conversion`] for `def_eq_uncached`, which has already observed the pair as closed
/// data (`constructor_equivalent`, `constructors_disagree`) before reaching the loop.
pub(super) fn lazy_delta_after_the_probes(ctx: &Context, a: &Term, b: &Term) -> Option<bool> {
    lazy(ctx, a, b, Observe::Nothing)
}

/// Whether the loop evaluates a closed pair with the certified lazy evaluator before its first δ
/// step — Lean's `reduce_nat` and its constructor comparison of closed values: `Fin.mk 4 _`
/// against `Fin.mk 0 _` is refuted at once, never by unfolding `Nat.mod`'s well-founded recursion.
#[derive(Clone, Copy, PartialEq)]
enum Observe {
    ClosedData,
    Nothing,
}

/// The certified evaluator's allowance for one closed-data observation inside the loop.
const CLOSED_DATA_FUEL: u64 = 1 << 18;

fn lazy(ctx: &Context, a: &Term, b: &Term, observe: Observe) -> Option<bool> {
    if mentions_hole(a) || mentions_hole(b) {
        return None;
    }
    if LEFT_OPEN.with(|open| open.get()) == Some(address(a, b)) {
        LEFT_OPEN.with(|open| open.set(None));
        return None;
    }
    let answer = decide(ctx, a, b, observe);
    if ctx.simp_rules().is_empty() {
        answer
    } else {
        answer.filter(|&equal| equal)
    }
}

fn decide(ctx: &Context, a: &Term, b: &Term, observe: Observe) -> Option<bool> {
    let mut allowance = Allowance::new(LAZY_DELTA_STEPS).below(conversion_nesting());
    match converge(ctx, a, b, &mut allowance, observe) {
        Ok(answer) => answer,
        // A reduct — or a nesting of reductions — outgrew the room the stack this call runs on has
        // left, not the allowance: decide the pair again, from the start, on a stack sized for it.
        // The stack at least doubles each time and stops at the one the kernel's term-depth door
        // allows, where nothing is refused for the stack alone any more, so this ends.
        Err(Exhausted) => match allowance.wanted_depth() {
            Some(depth) => {
                crate::term_store::grow_stack_for_depth(depth, || decide(ctx, a, b, observe))
            }
            None => None,
        },
    }
}

fn converge(
    ctx: &Context,
    a: &Term,
    b: &Term,
    allowance: &mut Allowance,
    mut observe: Observe,
) -> Result<Option<bool>, Exhausted> {
    let mut a = whnf_core(ctx, a.clone(), allowance)?;
    let mut b = whnf_core(ctx, b.clone(), allowance)?;
    loop {
        if alpha_eq(&a, &b) {
            return Ok(Some(true));
        }
        // A natural literal against a closed natural computation the certified evaluator finished:
        // the kernel's own complete rule (`def_eq_uncached` answers with it outright), so its
        // `false` is a definite refutation too.
        if let Some(equal) = natural_literal_conversion(ctx, &a, &b, 1 << 18) {
            return Ok(Some(equal));
        }
        // The literal bridge: a literal against a constructor application is compared as the
        // constructor spelling the kernel's literal rule gives it — a natural `0` is `Zero` and
        // `n + 1` is `Succ n`, an integer literal its `Int` constructor — one constructor at a
        // time, exactly as the fallback's `nat_peano_equal` and `literal_constructor` spell it.
        if let Some(spelled) = literal_spelled_for(ctx, &a, &b) {
            a = spelled;
            continue;
        }
        if let Some(spelled) = literal_spelled_for(ctx, &b, &a) {
            b = spelled;
            continue;
        }
        if let Some(equal) = rigid_arguments(ctx, &a, &b) {
            return Ok(Some(equal));
        }
        match (&a, &b) {
            // R3: two binders are equal exactly when their domains and bodies are.
            (Term::Pi { .. }, Term::Pi { .. }) | (Term::Lambda { .. }, Term::Lambda { .. }) => {
                return Ok(Some(binder_congruence(ctx, &a, &b)));
            }
            (Term::Lambda { .. }, _) => return Ok(eta(ctx, &a, &b).then_some(true)),
            (_, Term::Lambda { .. }) => return Ok(eta(ctx, &b, &a).then_some(true)),
            _ => {}
        }
        let structure_eta = (
            try_structure_eta(ctx, &a, &b),
            try_structure_eta(ctx, &b, &a),
        );
        if structure_eta.0 == Some(true) || structure_eta.1 == Some(true) {
            return Ok(Some(true));
        }
        let left = delta_head(ctx, &a).map(|name| (name.to_owned(), Unfold::of(ctx, name)));
        let right = delta_head(ctx, &b).map(|name| (name.to_owned(), Unfold::of(ctx, name)));
        // Lean's `reduce_nat` before a δ step: a certified program at numerals is its value.
        if left.is_some() {
            if let Some(value) = certified_value(ctx, &a, allowance)? {
                a = value;
                continue;
            }
        }
        if right.is_some() {
            if let Some(value) = certified_value(ctx, &b, allowance)? {
                b = value;
                continue;
            }
        }
        if observe == Observe::ClosedData && (left.is_some() || right.is_some()) {
            observe = Observe::Nothing;
            if crate::reduction::constructor_equivalent(ctx, &a, &b, CLOSED_DATA_FUEL) {
                return Ok(Some(true));
            }
            if crate::reduction::constructors_disagree(ctx, &a, &b, CLOSED_DATA_FUEL) {
                return Ok(Some(false));
            }
        }
        let (unfold_left, unfold_right) = match (&left, &right) {
            // Two stuck heads: congruence, then Lean's closing rules — unit η, and then the
            // refutation of heads that can never meet (R1, R2, R4). Anything else is the
            // fallback's.
            (None, None) => {
                if stuck_congruence(ctx, &a, &b) {
                    return Ok(Some(true));
                }
                let unit = unit_eta(ctx, &a, &b);
                if unit == Some(true) {
                    return Ok(Some(true));
                }
                if unit != Some(false) {
                    return Ok(None);
                }
                let structure_eta_decided = structure_eta.0.is_some() || structure_eta.1.is_some();
                let apart = rigid_mismatch(ctx, &a, &b, structure_eta_decided)
                    || stuck_apart(ctx, &a, &b, structure_eta_decided, allowance)?;
                return Ok(apart.then_some(false));
            }
            (Some(_), None) => (true, false),
            (None, Some(_)) => (false, true),
            (Some((left_name, left)), Some((right_name, right))) => {
                if left_name == right_name
                    && left.height == right.height
                    && same_head_arguments(ctx, &a, &b)
                {
                    return Ok(Some(true));
                }
                unfold_order(left, right)
            }
        };
        if unfold_left {
            let Some(unfolded) = unfold_head(ctx, &a, allowance)? else {
                return Ok(None);
            };
            a = unfolded;
        }
        if unfold_right {
            let Some(unfolded) = unfold_head(ctx, &b, allowance)? else {
                return Ok(None);
            };
            b = unfolded;
        }
    }
}

/// The normalization fallback's own verdict on `a ≡ b`, `None` when either normalization did not
/// finish (and so decided nothing). Used only by the differential below.
#[cfg(debug_assertions)]
fn decided_by_normalization(ctx: &Context, a: &Term, b: &Term) -> Option<bool> {
    let folded = |name: &str| is_transparent_theorem(ctx, name);
    let tr = crate::reduction::Transparency::Excluding(&folded);
    let a_folded = crate::reduction::normalize_checked_at(ctx, a, tr).ok()?;
    let b_folded = crate::reduction::normalize_checked_at(ctx, b, tr).ok()?;
    if def_eq_normalized(ctx, &a_folded, &b_folded) {
        return Some(true);
    }
    let a_norm = crate::reduction::normalize_checked(ctx, &a_folded).ok()?;
    let b_norm = crate::reduction::normalize_checked(ctx, &b_folded).ok()?;
    Some(def_eq_normalized(ctx, &a_norm, &b_norm))
}

/// With `LOGOS_LAZY_DELTA_DIFFERENTIAL` set, a debug build re-decides every pair the lazy loop
/// answered by the normalization fallback and aborts on any pair the fallback DECIDES differently
/// (a fallback that runs out of budget decides nothing and is not compared). This is the lock that
/// `Some(x)` from the loop implies the fallback's `x` across every conversion a test run performs.
#[cfg(debug_assertions)]
pub(super) fn differential(ctx: &Context, a: &Term, b: &Term, answer: bool) {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ENABLED.get_or_init(|| std::env::var_os("LOGOS_LAZY_DELTA_DIFFERENTIAL").is_some()) {
        return;
    }
    // The re-decision is audit work the verdict never uses: its stops must not reach `exact`.
    if let Some(fallback) =
        crate::reduction::discard_resource_stops(|| decided_by_normalization(ctx, a, b))
    {
        assert_eq!(answer, fallback,
            "LAZY-DELTA-DISAGREEMENT: lazy δ answered {answer}, the normalization fallback {fallback}\n  a = {a}\n  b = {b}");
    }
}
