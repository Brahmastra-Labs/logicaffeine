//! Term reduction for the Calculus of Constructions.
//!
//! This module implements the evaluation semantics of the kernel. Terms are
//! reduced to normal form using a call-by-name strategy.
//!
//! # Reduction Rules
//!
//! ## Beta Reduction
//! Function application: `(λx. body) arg → body[x := arg]`
//!
//! ## Iota Reduction
//! Pattern matching: `match (Cᵢ args) { ... caseᵢ ... } → caseᵢ(args)`
//!
//! ## Fix Unfolding (Guarded)
//! Recursive definitions: `(fix f. body) (Cᵢ args) → body[f := fix f. body] (Cᵢ args)`
//!
//! Fix unfolding is guarded by requiring the argument to be in constructor form.
//! This ensures termination by structural recursion.
//!
//! ## Delta Reduction
//! Global definitions are expanded when needed during normalization.
//!
//! # Primitive Operations
//!
//! The reducer handles built-in operations on literals:
//! - Arithmetic: `add`, `sub`, `mul`, `div`, `mod`
//! - Comparison: `lt`, `le`, `gt`, `ge`, `eq`
//! - Boolean: `ite` (if-then-else)
//!
//! # Reflection Builtins
//!
//! Special operations for the deep embedding (Syntax type):
//! - `syn_size`: Compute the size of a syntax tree
//! - `syn_max_var`: Find the maximum variable index
//! - `syn_step`, `syn_eval`: Bounded evaluation
//! - `syn_quote`, `syn_diag`: Reification and diagonalization
//!
//! # Divergence
//!
//! Normalization detects a CYCLING reduction (Brent's algorithm, O(1) memory) and falls back
//! on a large step budget for divergence that does not cycle. `normalize_checked` reports
//! which happened; `normalize` hands back how far it got, because its signature is public API.

use crate::context::Context;
use crate::omega;
use crate::term::{int_lit, lit_bigint, Literal, Term};
use crate::type_checker::substitute;

/// Why a reduction stopped before reaching a normal form.
///
/// `normalize` hands back a bare `Term`, so "this is the normal form" and "this is as far as I
/// got" are THE SAME ANSWER — and the caller cannot tell them apart. That is not a precision
/// loss, it is a soundness channel: a conversion check that compares two abandoned reductions
/// and finds them equal has certified nothing.
///
/// A δ-cycle (`a := b`, `b := a`) proves no fuel constant can close the gap, because that
/// reduction has no normal form at all. Only an error channel can.
///
/// It is nevertheless CONSERVATIVE rather than unsound, and the reason is worth stating because
/// it is what a future change can break: `reduce_step_opt` is a FUNCTION of the term, so two
/// reductions holding the same term after the same number of steps hold it forever after and
/// therefore have the same normal form. A conversion check built on "normalise both sides, then
/// compare" can be INCOMPLETE when a budget runs out — never wrong. The same determinism makes
/// the cycle answer safe: `normalize` hands the INPUT back, and the only term that can be
/// mistaken for is itself. Collapsing every non-normalising term to one answer instead — a
/// shared `Hole`, say — would identify two unrelated cycles and let `refl` prove an equation
/// between two well-typed terms that are not equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreduced {
    /// The reduction returned to a term it had already produced, so it cycles forever. The
    /// term has NO normal form.
    Cyclic,
    /// The step budget ran out. The term may or may not have a normal form.
    BudgetSpent,
    /// The term nests deeper than the kernel will descend, so the reducer never started.
    TooDeep,
}

thread_local! {
    /// The first RESOURCE stop — a spent step budget or a term too deep to descend — of any
    /// reduction run on this thread since the innermost [`observe_resource_stops`] began.
    /// Recorded where the kernel gives up, whether or not its caller then carries on with the
    /// partial term, because the caller that carries on is exactly the one that cannot tell.
    static RESOURCE_STOP: std::cell::Cell<Option<Unreduced>> = const { std::cell::Cell::new(None) };
}

/// Record that kernel work stopped for want of a resource rather than reaching its answer.
/// A cycle is not a resource stop: it is a proof that there is no normal form to reach.
pub(crate) fn record_resource_stop(reason: Unreduced) {
    if reason == Unreduced::Cyclic {
        return;
    }
    RESOURCE_STOP.with(|stop| {
        if stop.get().is_none() {
            stop.set(Some(reason))
        }
    });
}

/// The resource stop recorded on this thread, for `term_store`'s sized-stack threads to carry
/// back to the thread that started them.
pub(crate) fn resource_stop_on_this_thread() -> Option<Unreduced> {
    RESOURCE_STOP.with(|stop| stop.get())
}

/// Run `f` and report the first resource stop of any kernel work it did — a reduction that spent
/// its step budget, or a term too deep to descend — on this thread or on any sized-stack thread
/// the kernel started for it.
///
/// This is what makes "not equal" an answer. A conversion that returns `false` after one of its
/// reductions stopped short has compared a partial term, and a partial term can differ from a
/// normal form it never reached: that `false` is an UNDECIDED comparison, never evidence that the
/// two sides differ. Only the conversion that ran knows whether it depended on such a stop, so
/// the record is taken from inside it — never reconstructed afterwards by normalizing the whole
/// sides again, which costs more than the conversion it audits and cannot even see what the
/// conversion actually compared (`refute-cheap-first` refutes `(false, count 200)` against
/// `(true, count 201)` at the booleans without ever evaluating `count`).
///
/// Observations nest: the record of the enclosing observation is restored afterwards, and learns
/// of a stop inside this one too.
pub fn observe_resource_stops<R>(f: impl FnOnce() -> R) -> (R, Option<Unreduced>) {
    struct Enclosing(Option<Unreduced>);
    impl Drop for Enclosing {
        fn drop(&mut self) {
            RESOURCE_STOP.with(|stop| {
                let inner = stop.get();
                stop.set(self.0.or(inner));
            });
        }
    }
    let enclosing = Enclosing(RESOURCE_STOP.with(|stop| stop.replace(None)));
    let result = f();
    let stopped = RESOURCE_STOP.with(|stop| stop.get());
    drop(enclosing);
    (result, stopped)
}

/// Run `f` WITHOUT letting its resource stops reach the enclosing observation — for work whose
/// answer the verdict does not use (a debug differential that re-decides a pair already decided).
pub fn discard_resource_stops<R>(f: impl FnOnce() -> R) -> R {
    struct Enclosing(Option<Unreduced>);
    impl Drop for Enclosing {
        fn drop(&mut self) {
            RESOURCE_STOP.with(|stop| stop.set(self.0));
        }
    }
    let _enclosing = Enclosing(RESOURCE_STOP.with(|stop| stop.replace(None)));
    f()
}

/// The maximum number of whole-term reduction steps. Large enough that no TERMINATING
/// reduction the kernel is asked for hits it (a 20 000-step δ-chain has 50× headroom), because
/// a budget that stops a terminating reduction is indistinguishable from a normal form. Cyclic
/// divergence is caught by cycle detection below, long before this.
const MAX_REDUCTION_STEPS: usize = 1 << 20;

/// How much δ a reduction is allowed — Lean's `TransparencyMode`, narrowed to the axis that
/// actually changes the answer.
///
/// FOLLOWING LEAN, INCLUDING WHERE IT IS SUBTLE. Reducibility in Lean is an ELABORATOR
/// attribute: Lean's KERNEL unfolds every definition whatever its attribute, which is why
/// `example : secret = 5 := rfl` on an `@[irreducible] def secret : Nat := 5` is refused by the
/// elaborator and *accepted* by the kernel. So [`Transparency::All`] is the kernel's own
/// setting and stays what [`normalize`] uses, and [`Transparency::Excluding`] is the
/// elaborator's default transparency — the setting under which an `@[irreducible]` constant
/// keeps its body AND stays folded. Collapsing the two in either direction is a real loss:
/// always-transparent makes LOGOS prove by `rfl` what Lean's elaborator refuses (the
/// over-permissive direction), and routing irreducibility through `add_declaration` instead
/// throws the body away and manufactures a silent axiom.
///
/// The opacity set is supplied as a PREDICATE rather than read out of the `Context`, so the
/// reducer does not have to know how the attribute is stored: a reducibility map in `Context`
/// wires in as `Transparency::Excluding(&|n| ctx.is_irreducible(n))` and nothing here changes.
#[derive(Clone, Copy)]
pub enum Transparency<'a> {
    /// Unfold every definition. The kernel's setting.
    All,
    /// Keep every constant the predicate accepts FOLDED, body intact. The elaborator's default.
    ///
    /// `+ Sync` is not decoration: without it `Transparency` is not `Send`, and a normalisation
    /// entry point that cannot cross a thread boundary cannot be run on a SIZED STACK — which is
    /// what stops a deep term ABORTING the process instead of failing. The bound costs nothing:
    /// every predicate here is built from `Context::is_irreducible`, and `Context` is `Sync`.
    Excluding(&'a (dyn Fn(&str) -> bool + Sync)),
}

impl Transparency<'_> {
    /// Whether δ may unfold `name` at this transparency.
    fn unfolds(&self, name: &str) -> bool {
        match self {
            Transparency::All => true,
            Transparency::Excluding(opaque) => !(*opaque)(name),
        }
    }
}

/// Normalize a term, reporting honestly when the reduction did not finish.
///
/// `Err((partial, why))` carries the term the reducer had reached, so a caller that wants
/// best-effort behaviour can still have it — but it has to ask for it, which is the whole
/// point.
///
/// Divergence is detected with Brent's cycle-finding: one saved term (the "tortoise"), re-saved
/// at powers of two, compared against each new term. It needs O(1) memory, finds a cycle of any
/// period within about twice the cycle length, and — unlike a step budget — answers the δ-cycle
/// case CORRECTLY rather than merely late.
pub fn normalize_checked(ctx: &Context, term: &Term) -> Result<Term, (Term, Unreduced)> {
    normalize_checked_at(ctx, term, Transparency::All)
}

/// [`normalize_checked`] at a chosen transparency — the honest face of [`normalize_at`].
pub fn normalize_checked_at(
    ctx: &Context,
    term: &Term,
    tr: Transparency<'_>,
) -> Result<Term, (Term, Unreduced)> {
    if let Some(r) = crate::annotation::erased(term, |term| normalize_checked_at(ctx, term, tr)) {
        return r;
    }
    // A SIZED STACK, measured ONCE per outermost kernel call, and measured HERE because this is
    // the one funnel every normalisation entry point reaches — `normalize`, `normalize_at`,
    // `normalize_checked`, `normalize_elab` and `normalize_checked_elab` all end up in this
    // function, so wiring it once is wiring it five times without five chances to forget one.
    //
    // It matters because `reduce_step_opt` descends with NATIVE recursion: a term deeper than
    // the thread's stack does not fail, it ABORTS THE PROCESS, and a public entry point may
    // never do that. `deepen_stack` runs a shallow term inline and a deep one on a scoped
    // thread sized for it, and returns `None` on the re-entrant call inside its own closure —
    // so the depth measurement is paid exactly once however deep the recursion goes.
    let result =
        match crate::term_store::deepen_stack(term, || normalize_checked_at_sized(ctx, term, tr)) {
            Some(result) => result,
            None => normalize_checked_at_sized(ctx, term, tr),
        };
    if let Err((_, reason)) = &result {
        record_resource_stop(*reason);
    }
    result
}

/// [`normalize_checked`] within `steps` whole-term reduction steps rather than the kernel's full
/// allowance: exactly the same normal form whenever one is reached within them, `BudgetSpent`
/// sooner otherwise. A PROBE: its stop is not recorded as a resource stop of the enclosing
/// conversion, because the caller answers it with another method, not with a refusal.
pub(crate) fn normalize_checked_within(
    ctx: &Context,
    term: &Term,
    steps: usize,
) -> Result<Term, (Term, Unreduced)> {
    if let Some(r) =
        crate::annotation::erased(term, |term| normalize_checked_within(ctx, term, steps))
    {
        return r;
    }
    let limit = steps.min(MAX_REDUCTION_STEPS);
    discard_resource_stops(|| {
        match crate::term_store::deepen_stack(term, || {
            normalize_sized(ctx, term, Transparency::All, limit)
        }) {
            Some(result) => result,
            None => normalize_sized(ctx, term, Transparency::All, limit),
        }
    })
}

/// [`normalize_checked_at`]'s body, on a stack sized for `term`.
fn normalize_checked_at_sized(
    ctx: &Context,
    term: &Term,
    tr: Transparency<'_>,
) -> Result<Term, (Term, Unreduced)> {
    normalize_sized(ctx, term, tr, MAX_REDUCTION_STEPS)
}

/// The normalization pipeline within `limit` (at most `MAX_REDUCTION_STEPS`) whole-term steps.
fn normalize_sized(
    ctx: &Context,
    term: &Term,
    tr: Transparency<'_>,
    limit: usize,
) -> Result<Term, (Term, Unreduced)> {
    // A NESTED normalization may be handed a term deeper than the stack its outermost kernel call
    // was sized for: a reduct an earlier normalization in the same conversion built (and handed
    // back partial at the depth door). `deepen_stack` answers `None` for a nested call, so the
    // term is measured here and the stack grown for it, exactly as `normalize_progress` does for
    // the terms it builds — a public entry point may refuse a deep term, but never abort.
    let depth = crate::term_store::term_depth(term);
    let sized = depth.min(crate::type_checker::MAX_TERM_DEPTH);
    if sized > crate::term_store::INLINE_DEPTH
        && crate::term_store::stack_bytes_for_depth(sized) > crate::term_store::kernel_stack_bytes()
    {
        return crate::term_store::grow_stack_for_depth(sized, || {
            normalize_sized(ctx, term, tr, limit)
        });
    }
    // A term past the door is refused here and not by `enter_kernel`, which admits every term once
    // a kernel call is under way: a nested normalization can be handed the partial reduct an earlier
    // one stopped at, one level past the door, and descending it would exhaust the stack. The
    // refusal's partial term is cloned on the stack grown just above.
    if depth > crate::type_checker::MAX_TERM_DEPTH {
        return Err((term.clone(), Unreduced::TooDeep));
    }
    // `reduce_step_opt` descends into every subterm with native recursion, so a term deeper
    // than the kernel's door is refused BEFORE the first clone — cloning a `Term` is itself
    // recursive over its `Box`es, so even copying a deep enough term aborts the process.
    let Some(_door) = crate::type_checker::enter_kernel_or_refuse(term) else {
        return Err((term.clone(), Unreduced::TooDeep));
    };
    // `None` is the reducer SAYING there is no redex, not a structural comparison guessing it.
    // That is one fewer Θ(size) walk per step — and, unlike `reduced == current`, it is right
    // about a term holding a `NaN`, which is never equal to itself.
    //
    // The first question is asked on the BORROWED term, because the commonest input to a
    // conversion check is ALREADY a normal form: that answer now costs one walk and a single
    // clone (the one the return value needs), where the old shape cloned the whole term TWICE
    // before taking a single step.
    // The existing pure beta/zeta door gives the exact same normal form at
    // every transparency. Its conservative shape/name checks exclude all other
    // reduction rules. Use it where sharing repays the per-call store setup.
    if depth >= 64 {
        if let Some(nf) =
            crate::term_store::beta_zeta_normal_form_in_context(ctx, term, limit as u64)
        {
            return Ok(nf);
        }
    }
    // Spend from the existing budget; unsuccessful speculation does not reset it. A smaller limit
    // is the same budget with `MAX_REDUCTION_STEPS - limit` already spent.
    let mut spent = MAX_REDUCTION_STEPS - limit;
    if shared_head::natural_result_candidate(ctx, term, tr) {
        let allowance = limit as u64;
        let (answer, remaining) = shared_head::natural_normal_form(ctx, term, tr, allowance);
        spent += (allowance - remaining) as usize;
        if let shared_head::NatReadbackOutcome::Normal(normal) = answer {
            return Ok(normal);
        }
    }
    normalize_tree_with_spent(ctx, term, tr, spent)
}

// Resume ordinary reduction with prior speculative work charged to its budget.
fn normalize_tree_with_spent(
    ctx: &Context,
    term: &Term,
    tr: Transparency<'_>,
    spent: usize,
) -> Result<Term, (Term, Unreduced)> {
    if spent >= MAX_REDUCTION_STEPS {
        return Err((term.clone(), Unreduced::BudgetSpent));
    }
    let Some(current) = reduce_step_opt(ctx, term, tr) else {
        return Ok(term.clone());
    };
    normalize_progress(ctx, tr, current, term.clone(), 0, 1, spent + 1)
}

/// Resume with exactly the same fuel and cycle detector after stack growth.
fn normalize_progress(
    ctx: &Context,
    tr: Transparency<'_>,
    mut current: Term,
    mut tortoise: Term,
    mut since_save: usize,
    mut save_at: usize,
    mut steps: usize,
) -> Result<Term, (Term, Unreduced)> {
    let _probes = KProbeMemo::install();
    loop {
        let depth = crate::term_store::term_depth(&current);
        if depth > crate::type_checker::MAX_TERM_DEPTH {
            return Err((current, Unreduced::TooDeep));
        }
        if depth > crate::term_store::INLINE_DEPTH
            && crate::term_store::stack_bytes_for_depth(depth)
                > crate::term_store::kernel_stack_bytes()
        {
            return crate::term_store::grow_stack_for_depth(depth, || {
                normalize_progress(ctx, tr, current, tortoise, since_save, save_at, steps)
            });
        }
        if current == tortoise {
            return Err((current, Unreduced::Cyclic));
        }
        since_save += 1;
        if since_save == save_at {
            tortoise = current.clone();
            since_save = 0;
            save_at *= 2;
        }
        if steps == MAX_REDUCTION_STEPS {
            return Err((current, Unreduced::BudgetSpent));
        }
        if !reduce_step_in_place(ctx, &mut current, tr) {
            return Ok(current);
        }
        steps += 1;
    }
}

/// Normalize a term to its normal form.
///
/// Repeatedly applies reduction rules until no more reductions are possible.
/// This is a full normalization that reduces under binders.
///
/// This is the BEST-EFFORT face of [`normalize_checked`]: when the reduction cannot be
/// finished it hands back how far it got, which is exactly the answer a caller cannot
/// distinguish from success. It keeps this shape because `normalize` is re-exported from the
/// crate root and callers across the workspace depend on the signature; every caller INSIDE the
/// kernel that can carry an error should ask `normalize_checked` instead.
pub fn normalize(ctx: &Context, term: &Term) -> Term {
    normalize_at(ctx, term, Transparency::All)
}

/// [`normalize`] at a chosen transparency. At [`Transparency::Excluding`] the named constants
/// keep their bodies and stay FOLDED, which is what `@[irreducible]` means at Lean's default
/// transparency; `normalize` itself is the kernel's setting and unfolds everything.
pub fn normalize_at(ctx: &Context, term: &Term, tr: Transparency<'_>) -> Term {
    match normalize_checked_at(ctx, term, tr) {
        Ok(normal) => normal,
        // A CYCLING reduction has no normal form, and every term on the cycle is as good an
        // answer as any other — so handing back an arbitrary one makes `normalize`
        // non-idempotent (`normalize(a) = b`, `normalize(b) = a`) and every conversion check
        // built on "normalize both sides, then compare" inherits the instability. Handing the
        // term back unchanged is the one answer that is at least a FUNCTION.
        Err((_partial, Unreduced::Cyclic)) => term.clone(),
        Err((partial, _why)) => partial,
    }
}

/// [`normalize`] at the ELABORATOR's transparency, reading the context's own reducibility
/// registry — the door that makes `@[irreducible]` reachable without every caller inventing its
/// own opacity predicate.
///
/// [`Transparency::Excluding`] takes a PREDICATE so the reducer need not know how the attribute
/// is stored; the cost of that freedom is that a caller has to build the closure, and two
/// callers that build it differently would disagree about whether a constant unfolds — which is
/// `def_eq` answering differently depending on who asked. This is the one closure, built from
/// [`Context::is_irreducible`], so there is nothing to get wrong.
///
/// FOLLOWING LEAN, INCLUDING WHERE IT IS SUBTLE: this is NOT what the kernel uses. Lean's kernel
/// unfolds every definition whatever its attribute — `example : secret = 5 := rfl` on an
/// `@[irreducible] def secret := 5` is refused by the elaborator and accepted by the kernel —
/// so [`normalize`] stays [`Transparency::All`] and this is the elaborator's face.
pub fn normalize_elab(ctx: &Context, term: &Term) -> Term {
    let opaque = |n: &str| ctx.is_irreducible(n);
    normalize_at(ctx, term, Transparency::Excluding(&opaque))
}

/// [`normalize_elab`]'s honest face: the elaborator's transparency, and a reduction that did not
/// finish reported as such rather than handed back as a normal form.
pub fn normalize_checked_elab(ctx: &Context, term: &Term) -> Result<Term, (Term, Unreduced)> {
    let opaque = |n: &str| ctx.is_irreducible(n);
    normalize_checked_at(ctx, term, Transparency::Excluding(&opaque))
}

/// A rewrite that FIRED, reported as progress only if it actually changed the term.
///
/// Some rules are idempotent re-wrappings rather than genuine steps (a reflection builtin that
/// re-normalises an already-normal argument, say), and a reducer that claimed progress for one
/// of those would spin until the budget ran out. The comparison costs a walk, but it happens
/// only on the path where a named rule already matched — never on the search for one.
fn progressed(term: &Term, result: Term) -> Option<Term> {
    if result == *term {
        None
    } else {
        Some(result)
    }
}

thread_local! {
    /// The ι follow-up steps nested inside one another on this thread (see `reduce_step_opt`).
    static IOTA_FOLLOW_UPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many ι follow-up steps may nest. The stack a normalization runs on is sized at
/// `term_store::BYTES_PER_RUNG` (48 KiB) per level of the term it holds; a `reduce_step_opt` frame
/// is about 8.9 KB in a debug build (measured). Each nested follow-up adds one more frame per level
/// descended, so the descent and four follow-ups — five frames, ≈ 45 KB — are what one rung holds.
const IOTA_FOLLOW_UP_LIMIT: usize = 4;

struct IotaFollowUp(usize);

impl IotaFollowUp {
    fn enter() -> Option<IotaFollowUp> {
        let nested = IOTA_FOLLOW_UPS.with(|n| n.get());
        if nested >= IOTA_FOLLOW_UP_LIMIT {
            return None;
        }
        IOTA_FOLLOW_UPS.with(|n| n.set(nested + 1));
        Some(IotaFollowUp(nested))
    }
}

impl Drop for IotaFollowUp {
    fn drop(&mut self) {
        IOTA_FOLLOW_UPS.with(|n| n.set(self.0));
    }
}

/// Single-step reduction — `None` means THERE WAS NO REDEX, so the term is already normal.
///
/// Reduces the outermost redex first (call-by-name), then recurses into subterms for full
/// normalization.
///
/// **Why progress is a RETURN VALUE and not a comparison.** This used to answer with a `Term`
/// alone, and [`normalize_checked_at`] recovered "did anything happen?" by comparing the answer
/// to the input. That is wrong twice over. It costs a Θ(size) structural walk PER STEP, on top
/// of the Θ(size) walk the step itself already paid — and it is only correct if term equality is
/// REFLEXIVE, which it is not: `Literal::Float` compares with IEEE `==`, so a term holding a
/// `NaN` is never equal to itself, the fixpoint test can never fire, and `normalize` spent its
/// whole 2^20-step budget on a term that was ALREADY A NORMAL FORM — then reported
/// `Unreduced::BudgetSpent` about it. Progress is something the reducer knows for certain;
/// saying so is both exact and free, and it also stops every no-progress arm from cloning a
/// term it is handing straight back.
///
/// A rule that FIRED and led back to where it started (`a := a`, `fix f. f`) reports progress and
/// is caught by cycle detection as [`Unreduced::Cyclic`] — which is the honest answer, because
/// such a term has no normal form. "There is no redex" and "there is a redex that leads nowhere"
/// are different facts and must not share an answer.
fn reduce_step_opt(ctx: &Context, term: &Term, tr: Transparency<'_>) -> Option<Term> {
    #[cfg(test)]
    TREE_REDUCTION_CALLS.with(|n| n.set(n.get() + 1));
    // A registered `@[simp]` rule is a REWRITE the environment was told to perform, and it is
    // tried before β/δ/ι rather than after — see [`try_simp_rewrite`] for why "when everything
    // else is stuck" is the one placement that can never fire. In a context with no registered
    // rule (every context the prelude builds) this is one `is_empty` on a slice.
    if let Some(rewritten) = try_simp_rewrite(ctx, term) {
        return Some(rewritten);
    }
    match term {
        // Literals are in normal form
        Term::Lit(_) => None,

        // Delta for a universe-polymorphic reference: unfold its body with the universe
        // parameters instantiated by the level arguments.
        //
        // ONE KERNEL, ONE VERDICT. A `Const` whose level list does not match its declaration's
        // universe PARAMETERS is not a term of the calculus, and `infer_type` refuses it. This
        // arm used to fall through to `term.clone()` — the reducer answering "settled normal
        // form" about a term that denotes nothing, so a conversion check that normalises both
        // sides and compares would happily compare two meaningless terms and find them equal.
        // The honest answer is [`Term::Hole`], the kernel's own "nothing determinate here",
        // which `infer_type` refuses by construction (`CannotInferHole`). It is the one answer
        // that neither passes for a normal form nor LAUNDERS the term: zipping the level list
        // and dropping the extra would silently make `id.{0,1}` mean `id.{0}` and hand back
        // something well typed.
        //
        // Malformedness is a property of the TERM, so it is decided before the transparency
        // door — an `@[irreducible]` constant is still a constant; a mis-levelled one is not.
        Term::Const { name, levels } => match const_step(ctx, name, levels) {
            ConstStep::Unresolved => None,
            ConstStep::Illformed => Some(Term::Hole),
            ConstStep::Value => None,
            ConstStep::Delta(subst, body) => {
                if !tr.unfolds(name) {
                    return None;
                }
                Some(crate::term::instantiate_universes(&body, &subst))
            }
        },

        // Beta: (λx. body) arg → body[x := arg]
        Term::App(func, arg) => {
            // First try primitive reduction (add, sub, mul, etc.)
            if let Some(result) = try_primitive_reduce(ctx, func, arg, tr) {
                return progressed(term, result);
            }

            // An attested alias is a checked structural definition. First give
            // its operands one ordinary reduction step so nested compact
            // arithmetic does not lose the alias head and expand into Peano
            // recursion. Reuse the outer normalization budget and transparency.
            if let Term::App(head, first) = func.as_ref() {
                if let Term::Global(operation) = head.as_ref() {
                    if crate::int_inductive::integer_alias_dependencies(operation).is_some_and(
                        |dependencies| dependencies.iter().all(|name| tr.unfolds(name)),
                    ) && crate::int_inductive::accelerates_integer_alias(ctx, operation)
                    {
                        if let Some(reduced_first) = reduce_step_opt(ctx, first, tr) {
                            return Some(Term::App(
                                Box::new(Term::App(head.clone(), Box::new(reduced_first))),
                                arg.clone(),
                            ));
                        }
                        if let Some(reduced_last) = reduce_step_opt(ctx, arg, tr) {
                            return Some(Term::App(func.clone(), Box::new(reduced_last)));
                        }
                    }
                }
            }

            // Try reflection builtins (syn_size, syn_max_var)
            if let Some(result) = try_reflection_reduce(ctx, func, arg) {
                return progressed(term, result);
            }

            // Native reduction: `reduceBool t` evaluates `t` with the fast CBV evaluator
            // (the `native_decide` engine). This is the ONLY place the trusted evaluator
            // enters kernel reduction — gated behind the `reduceBool` primitive, which only
            // a `native_decide` proof (via the `ofReduceBool` axiom) ever mentions.
            //
            // THE SAME TWO GATES EVERY OTHER COMPUTATION RULE PASSES, and it is the rule that
            // needs them most, being the one whose answer an AXIOM turns into a proof. It used
            // to match a bare `Term::Global("reduceBool")` and ask neither:
            //
            //   * a DEFINITION of the name outranks the builtin — otherwise the reducer answers
            //     something the context it type-checked the application against says the
            //     constant does not mean, with no diagnostic anywhere;
            //   * the rule may not fire at a type the DECLARATION denies — the answer is always
            //     a `Bool` constructor, so a context declaring `reduceBool : Nat → Nat` got a
            //     kernel where `infer_type` said `Nat` and `normalize` produced a `Bool`, which
            //     is subject reduction failing, and it is the same divergence
            //     `declaration_agrees_with_the_rule` was written to close for the Int table.
            //
            // NOT gated on transparency: this is a computation rule, not δ — Lean runs its
            // `reduceNat?`/`reduceNative?` from `whnfImp` (`Lean/Meta/WHNF.lean:999-1004`)
            // before `unfoldDefinition?` and without consulting `TransparencyMode`.
            if let Term::Global(name) = func.as_ref() {
                if name == "reduceBool"
                    && ctx.get_definition_body(name).is_none()
                    && declaration_agrees_with_the_rule(ctx, name)
                {
                    if let Some(b) = crate::eval::eval_bool(ctx, arg) {
                        return progressed(
                            term,
                            Term::Global(if b { "true" } else { "false" }.to_string()),
                        );
                    }
                }
            }

            // Quotient computation: `Quot_lift A r B f h (Quot_mk A r a) ⤳ f a`, and `Quot_ind`'s.
            if let Some(result) = try_quot_lift_reduce(ctx, func, arg) {
                return progressed(term, result);
            }

            // Observational equality computes on its type (`observational`).
            if let Some(result) = try_obs_reduce(ctx, func, arg) {
                return progressed(term, result);
            }

            // ι for a fixpoint, at the column the termination guard admitted it on. Asked of
            // the WHOLE spine, because the structural argument need not be the innermost one.
            if let Some(result) = try_fix_unroll(ctx, term) {
                return progressed(term, result);
            }

            // Then try to reduce at the head
            match func.as_ref() {
                Term::Lambda { param, body, .. } => {
                    // Beta reduction. The redex is GONE from the answer — substituting into a
                    // body cannot reproduce the application that contained it — so this is
                    // progress without needing to be compared.
                    Some(substitute(body, param, arg))
                }
                // Fix application. UNROLLING IS NOT DECIDED HERE: `try_fix_unroll` above asked
                // the termination guard which column the recursion descends on and answered for
                // the whole spine. This arm only makes progress in the ARGUMENT, so a structural
                // argument that is not yet a constructor can become one.
                //
                // The test this arm used to make — "is the argument the innermost `App` happens
                // to carry a constructor?" — was wrong in both directions at once. It left
                // `Nat.add`, which recurses on its SECOND argument, stuck for every opaque
                // first argument; and it UNROLLED on a constructor sitting in a non-structural
                // column, where the guard promises no decreasing measure and the unrolling need
                // never stop.
                Term::Fix { .. } => {
                    let reduced_arg = reduce_step_opt(ctx, arg, tr)?;
                    Some(Term::App(func.clone(), Box::new(reduced_arg)))
                }
                // Mutual fix application: (mutualfix { fⱼ := bⱼ }.i) arg. When `arg` is a
                // constructor, unfold the i-th body, substituting EACH sibling name by its
                // own projection of the block, then apply — the mutual analog of the Fix
                // unfold above (so `Even.rec`'s call to `Odd.rec` computes).
                Term::MutualFix { defs, index } => {
                    if is_constructor_form(ctx, arg) {
                        let mut unfolded = defs[*index].1.clone();
                        for (j, (name_j, _)) in defs.iter().enumerate() {
                            let proj = Term::MutualFix {
                                defs: defs.clone(),
                                index: j,
                            };
                            unfolded = substitute(&unfolded, name_j, &proj);
                        }
                        Some(Term::App(Box::new(unfolded), arg.clone()))
                    } else {
                        let reduced_arg = reduce_step_opt(ctx, arg, tr)?;
                        Some(Term::App(func.clone(), Box::new(reduced_arg)))
                    }
                }
                // Nested application: ((f x) y) - reduce inner first, then the argument.
                // Other function forms take the same route.
                _ => {
                    if let Some(reduced_func) = reduce_step_opt(ctx, func, tr) {
                        return Some(Term::App(Box::new(reduced_func), arg.clone()));
                    }
                    let reduced_arg = reduce_step_opt(ctx, arg, tr)?;
                    Some(Term::App(func.clone(), Box::new(reduced_arg)))
                }
            }
        }

        // Iota: match (Cᵢ a₁...aₙ) with cases → caseᵢ a₁ ... aₙ
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            // `cases.get`, NOT `cases[…]`. The index comes from `extract_constructor`, whose Peano
            // bridge reports index 1 (`Succ`) for ANY positive `Lit(Nat(n))` regardless of which
            // inductive the `Match` was written against — so a `Match` with fewer cases than that
            // index used to panic and ABORT THE PROCESS out of `normalize`, a public entry point
            // that must be total. An out-of-range index means the `Match` is malformed and the type
            // checker will reject it; reduction's job is to make no progress, not to crash.
            if let Some(result) = iota_branch(ctx, discriminant, motive, cases) {
                // ι fired — the `Match` node is gone whatever happens next — then take one more
                // step on the selected branch if it has one. That step can be another ι whose
                // branch is stepped in turn, a recursion bounded by no term's depth, so only a few
                // follow-ups nest; past them the branch is handed back and the normalization loop
                // (which sizes its stack by the term it holds) takes the next step.
                let Some(_follow_up) = IotaFollowUp::enter() else {
                    return Some(result);
                };
                let stepped = reduce_step_opt(ctx, &result, tr);
                Some(stepped.unwrap_or(result))
            } else {
                // K and fieldless structure reduction require exactly one
                // constructor. Multiple branches cannot use either rule, so
                // do not recursively type-check their neutral scrutinees.
                if let Some(constructor) = (cases.len() == 1)
                    .then(|| k_constructor_form(ctx, discriminant, motive))
                    .flatten()
                {
                    return Some(Term::Match {
                        discriminant: Box::new(constructor),
                        motive: motive.clone(),
                        cases: cases.clone(),
                    });
                }
                // Try reducing the discriminant
                let reduced_disc = reduce_step_opt(ctx, discriminant, tr)?;
                Some(Term::Match {
                    discriminant: Box::new(reduced_disc),
                    motive: motive.clone(),
                    cases: cases.clone(),
                })
            }
        }

        // Reduce under lambdas (deep normalization)
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            let reduced_param_type = reduce_step_opt(ctx, param_type, tr);
            let reduced_body =
                reduce_step_opt(&ctx.extend(param, (**param_type).clone()), body, tr);
            if reduced_param_type.is_none() && reduced_body.is_none() {
                return None;
            }
            Some(Term::Lambda {
                param: param.clone(),
                param_type: Box::new(reduced_param_type.unwrap_or_else(|| (**param_type).clone())),
                body: Box::new(reduced_body.unwrap_or_else(|| (**body).clone())),
            })
        }

        // Reduce under Pi types
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            let reduced_param_type = reduce_step_opt(ctx, param_type, tr);
            let reduced_body_type =
                reduce_step_opt(&ctx.extend(param, (**param_type).clone()), body_type, tr);
            if reduced_param_type.is_none() && reduced_body_type.is_none() {
                return None;
            }
            Some(Term::Pi {
                param: param.clone(),
                param_type: Box::new(reduced_param_type.unwrap_or_else(|| (**param_type).clone())),
                body_type: Box::new(reduced_body_type.unwrap_or_else(|| (**body_type).clone())),
            })
        }

        // Reduce under Fix
        Term::Fix { name, body } => {
            let reduced_body = reduce_step_opt(ctx, body, tr)?;
            Some(Term::Fix {
                name: name.clone(),
                body: Box::new(reduced_body),
            })
        }

        // Reduce under a MutualFix (each body once; sibling names are free Vars here, so
        // no unfolding happens — this only normalizes the bodies' own redexes).
        Term::MutualFix { defs, index } => {
            let stepped: Vec<Option<Term>> = defs
                .iter()
                .map(|(_, b)| reduce_step_opt(ctx, b, tr))
                .collect();
            if stepped.iter().all(Option::is_none) {
                return None;
            }
            let new_defs = defs
                .iter()
                .zip(stepped)
                .map(|((n, b), rb)| (n.clone(), rb.unwrap_or_else(|| b.clone())))
                .collect();
            Some(Term::MutualFix {
                defs: new_defs,
                index: *index,
            })
        }

        // ζ, OCCURRENCE-GATED. `let x := v in b ⟶ b[x := v]` writes the value at EVERY
        // occurrence, so a value bound once and used n times is written n times — and the term
        // the kernel then checks is BIGGER than the one it was handed, which is the opposite of
        // what a sharing construct is for. Gated on the occurrence count the value is written
        // ONCE and the binding survives as the sharing node.
        //
        // A binder used at most once is substituted exactly as before, byte for byte, which is
        // what keeps every `let_zeta` case and the ζ-transparency cases green: all of them are
        // single-occurrence. The gate is re-read as reduction proceeds, because reduction
        // ERASES occurrences — a body with three uses before reduction and one after must still
        // expand, and does, on the next pass over the `Let` the previous pass rebuilt.
        Term::Let {
            name,
            ty,
            value,
            body,
        } => crate::term_store::zeta_or_share_opt(name, ty, value, body, &|t| {
            reduce_step_opt(ctx, t, tr)
        }),

        // Base cases: already in normal form
        Term::Sort(_) | Term::Var(_) | Term::Hole => None,

        // Delta reduction: unfold definitions (but not axioms, constructors, or inductives) —
        // and not an `@[irreducible]` one below the kernel's own transparency, which is the
        // whole point of the axis: the body is still THERE (so every equation it satisfies
        // stays provable and no silent axiom is manufactured), it simply does not unfold here.
        //
        // A definition whose body is its own name (`a := a`) UNFOLDS — the rule fired — and the
        // cycle detector then reports the reduction as `Cyclic`, which is true: the term has no
        // normal form. Calling that "no redex" would be `normalize` certifying a normal form
        // that does not exist.
        Term::Global(name) => {
            if !tr.unfolds(name) {
                return None;
            }
            // Axiom, constructor, or inductive: no reduction.
            Some(ctx.get_definition_body(name)?.clone())
        }

        // An annotation is consumed with the step its term takes, as Lean's `whnfCore` consumes
        // `mdata`, and never by a step of its own: a step that only dropped an annotation would
        // hand the cycle detector a term `==` to the one before it.
        Term::Annotated { term, .. } => reduce_step_opt(ctx, term, tr),
    }
}

/// Whether a `match` whose motive is `motive` may select a branch for `discriminant`.
///
/// Lean's recursor belongs to ONE inductive and fires only on that inductive's constructors
/// (`getRecRuleFor`). A LOGOS `match` names its inductive through its motive's last binder
/// (`λ indices… λ(x : I params indices). T`). When that binder's type is headed by a registered
/// inductive other than the discriminant's, the match is ill-typed and must not compute: the
/// `Nat` literal `3` is not the `Int` constructor `negSucc`, although both inductives have two
/// constructors. A motive that does not name an inductive (a raw sort, an alias) is not refused
/// here — the type checker is the authority on it, and this guard only removes wrong progress.
///
/// An ETA-SHORT motive (`λ y. tail y`, whose major-premise binder lives inside `tail`'s own
/// type) has fewer lambdas than the discriminant's index telescope plus the major, so its last
/// lambda binds an INDEX, not the major premise; there is no syntactic major domain to read, and
/// the guard defers to the type checker exactly as it does for a motive that names no inductive.
fn motive_admits(ctx: &Context, motive: &Term, discriminant: &Term) -> bool {
    let Some(actual) = constructor_inductive_of(ctx, discriminant) else {
        return true;
    };
    let mut domain = None;
    let mut binders = 0usize;
    let mut current = motive;
    while let Term::Lambda {
        param_type, body, ..
    } = current
    {
        domain = Some(param_type.as_ref());
        binders += 1;
        current = body;
    }
    if binders <= ctx.inductive_num_indices(&actual) {
        return true;
    }
    let Some(domain) = domain else { return true };
    let mut head = domain;
    while let Term::App(f, _) = head {
        head = f;
    }
    let named = match head {
        Term::Global(n) | Term::Const { name: n, .. } if ctx.is_inductive(n) => n,
        _ => return true,
    };
    actual == *named
}

/// Select an ordinary constructor branch, shared by full and lazy reduction.
/// Parameter removal and mutual-block case selection remain in the existing
/// constructor decoder and case-position calculation.
fn iota_branch(ctx: &Context, discriminant: &Term, motive: &Term, cases: &[Term]) -> Option<Term> {
    if !motive_admits(ctx, motive, discriminant) {
        return None;
    }
    let (i, args) = extract_constructor(ctx, discriminant)?;
    let p = case_position(ctx, discriminant, i, cases.len())?;
    let mut result = cases.get(p)?.clone();
    for arg in args {
        result = Term::App(Box::new(result), Box::new(arg));
    }
    Some(result)
}

#[cfg(test)]
thread_local! {
    static BOUNDED_SIZE_PENDING_PEAK: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

// Bound the work stack before extending wide case/definition vectors.
// The Boolean records a conservative monomorphic-level fragment for readback;
// ordinary conversion callers retain their previous Term-node admission.
pub(crate) fn bounded_conversion_metrics(term: &Term) -> Option<(usize, usize, bool)> {
    let mut pending = vec![(term, 0usize)];
    let mut nodes = 0usize;
    let mut depth = 0usize;
    let mut plain_levels = true;
    while let Some((t, d)) = pending.pop() {
        #[cfg(test)]
        BOUNDED_SIZE_PENDING_PEAK.with(|peak| peak.set(peak.get().max(pending.len() + 1)));
        nodes += 1;
        depth = depth.max(d);
        let children = match t {
            Term::App(..) | Term::Lambda { .. } | Term::Pi { .. } => 2,
            Term::Let { .. } => 3,
            Term::Match { cases, .. } => cases.len().checked_add(2)?,
            Term::Fix { .. } => 1,
            Term::MutualFix { defs, .. } => defs.len(),
            _ => 0,
        };
        if nodes.checked_add(pending.len())?.checked_add(children)? > 4096 {
            return None;
        }
        let next = d + 1;
        match t {
            Term::App(f, a) => {
                pending.push((f, next));
                pending.push((a, next));
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                pending.push((param_type, next));
                pending.push((body, next));
            }
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                pending.push((param_type, next));
                pending.push((body_type, next));
            }
            Term::Let {
                ty, value, body, ..
            } => {
                pending.push((ty, next));
                pending.push((value, next));
                pending.push((body, next));
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                pending.push((discriminant, next));
                pending.push((motive, next));
                pending.extend(cases.iter().map(|case| (case, next)));
            }
            Term::Fix { body, .. } => pending.push((body, next)),
            Term::MutualFix { defs, .. } => {
                pending.extend(defs.iter().map(|(_, body)| (body, next)))
            }
            Term::Const { .. } => plain_levels = false,
            Term::Sort(u) => {
                plain_levels &= matches!(
                    u,
                    crate::Universe::Prop | crate::Universe::SProp | crate::Universe::Type(_)
                )
            }
            _ => {}
        }
    }
    Some((nodes, depth, plain_levels))
}

/// Size check before cloning or substitution keeps the opportunistic path from
/// materializing a large tree even when a single beta step duplicates its input.
#[cfg(test)]
thread_local! {
    static BOUNDED_SIZE_CALLS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn bounded_conversion_size(term: &Term) -> Option<usize> {
    #[cfg(test)]
    BOUNDED_SIZE_CALLS.with(|count| {
        if let Some(n) = count.get() {
            count.set(Some(n + 1));
        }
    });
    bounded_conversion_metrics(term).map(|(nodes, _, _)| nodes)
}

fn bounded_conversion_substitute(body: &Term, name: &str, value: &Term) -> Option<Term> {
    let body_size = bounded_conversion_size(body)?;
    let occurrences = crate::term_store::occurrence_count_in_term(body, name) as usize;
    let value_size = bounded_conversion_size(value)?;
    let upper_bound = body_size.checked_add(occurrences.checked_mul(value_size)?)?;
    if upper_bound > 4096 {
        return None;
    }
    Some(substitute(body, name, value))
}

fn bounded_conversion_substitute_owned(body: Term, name: &str, value: Term) -> Option<Term> {
    let body_size = bounded_conversion_size(&body)?;
    let value_size = bounded_conversion_size(&value)?;
    let Some(occurrences) =
        crate::term_store::bounded_occurrence_count_in_term(&body, name, 64, 4096)
    else {
        // The optimization's bounded lexical scan is optional. Preserve the
        // existing behavior for deeper terms rather than adding a refusal.
        return bounded_conversion_substitute(&body, name, &value);
    };
    let upper_bound = body_size.checked_add((occurrences as usize).checked_mul(value_size)?)?;
    if upper_bound > 4096 {
        return None;
    }
    Some(crate::type_checker::substitute_owned_value_counted(
        body,
        name,
        value,
        occurrences as usize,
    ))
}

/// The shared admission boundary for small closed conversion bodies. This
/// checks annotations and every branch, including parts head reduction may erase.
pub(crate) fn bounded_closed_conversion_size(body: &Term) -> Option<usize> {
    let size = bounded_conversion_size(body)?;
    if !crate::term_store::is_closed_without_holes(body) {
        return None;
    }
    Some(size)
}

/// One closed delta step on the head path only. Its size is a scheduling hint,
/// not evidence of equality. Small wrappers can be exposed before a shared,
/// expensive function body. Match discriminants are the only other head path.
pub(crate) fn bounded_delta_head(ctx: &Context, term: &Term) -> Option<(usize, Term)> {
    bounded_delta_head_impl(ctx, term, Transparency::All, bounded_conversion_size, false)
}

/// Same head-delta rule with caller transparency and pre-copy admission. The
/// kernel wrapper above retains its original All policy and size admission.
pub(crate) fn bounded_delta_head_with(
    ctx: &Context,
    term: &Term,
    tr: Transparency<'_>,
    admit: fn(&Term) -> Option<usize>,
) -> Option<(usize, Term)> {
    bounded_delta_head_impl(ctx, term, tr, admit, true)
}

fn bounded_delta_head_impl(
    ctx: &Context,
    term: &Term,
    tr: Transparency<'_>,
    admit: fn(&Term) -> Option<usize>,
    bounded_telescope: bool,
) -> Option<(usize, Term)> {
    admit(term)?;
    match term {
        Term::Global(name) | Term::Const { name, .. } => {
            if !tr.unfolds(name) {
                return None;
            }
            let body = match term {
                Term::Global(_) => {
                    let body = ctx.get_definition_body(name)?;
                    // This immutable monomorphic body is unchanged by cloning.
                    // Retain its admission result and reject open/hole bodies
                    // while borrowed, before allocating an unused copy. Explicit
                    // universe instantiation below still needs its own recheck.
                    let size = admit(body)?;
                    if !crate::term_store::is_closed_without_holes(body) {
                        return None;
                    }
                    return Some((size, body.clone()));
                }
                Term::Const { levels, .. } => {
                    // const_step clones its header and body before resolving
                    // levels. Reject oversized candidates while still borrowed,
                    // using that resolver's exact lookup precedence.
                    match ctx.get_universe_poly(name) {
                        Some((params, ty, body)) => {
                            // const_step also copies explicit parameter names before
                            // resolving arity. Bound this borrowed metadata on the
                            // elaborator route, including unused parameters. The
                            // original kernel All wrapper retains its prior policy.
                            if bounded_telescope
                                && (params.len() > 16 || params.iter().any(|name| name.len() > 256))
                            {
                                return None;
                            }
                            admit(ty)?;
                            admit(body)?;
                        }
                        None => {
                            admit(ctx.get_global(name)?)?;
                            if let Some(body) = ctx.get_definition_body(name) {
                                admit(body)?;
                            }
                        }
                    }
                    match const_step(ctx, name, levels) {
                        ConstStep::Delta(subst, body) => {
                            crate::term::instantiate_universes(&body, &subst)
                        }
                        _ => return None,
                    }
                }
                _ => unreachable!(),
            };
            let size = admit(&body)?;
            if !crate::term_store::is_closed_without_holes(&body) {
                return None;
            }
            Some((size, body))
        }
        Term::App(f, a) => {
            let (size, f) = bounded_delta_head_impl(ctx, f, tr, admit, bounded_telescope)?;
            Some((size, Term::App(Box::new(f), a.clone())))
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            let (size, disc) =
                bounded_delta_head_impl(ctx, discriminant, tr, admit, bounded_telescope)?;
            Some((
                size,
                Term::Match {
                    discriminant: Box::new(disc),
                    motive: motive.clone(),
                    cases: cases.clone(),
                },
            ))
        }
        _ => None,
    }
}

/// Expose only a small head spine, leaving arguments, binder annotations and
/// unselected branches unevaluated. This is an opportunistic conversion step;
/// a resource boundary returns no result, never a claimed normal form.
pub(crate) fn bounded_conversion_head(ctx: &Context, term: &Term) -> Option<Term> {
    if !ctx.simp_rules().is_empty() {
        return None;
    }
    let mut budget = 256;
    let mut current = beta_iota_zeta_head(ctx, term, &mut budget)?;
    loop {
        if budget == 0 {
            return None;
        }
        budget -= 1;
        match bounded_delta_head(ctx, &current) {
            Some((_, next)) => current = beta_iota_zeta_head(ctx, &next, &mut budget)?,
            None => return Some(current),
        }
    }
}

#[cfg(test)]
thread_local! {
    static BOUNDED_HEAD_CLONE_NODES: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
    static BOUNDED_HEAD_VISITS: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
}
/// Enable/reset an isolated test counter and return the preceding sample.
#[cfg(test)]
pub(crate) fn take_bounded_head_clone_nodes() -> usize {
    BOUNDED_HEAD_CLONE_NODES.with(|count| count.replace(Some(0)).unwrap_or(0))
}

/// Count actual evaluator-loop entries, independently of copy and fuel totals.
#[cfg(test)]
pub(crate) fn take_bounded_head_visits() -> usize {
    BOUNDED_HEAD_VISITS.with(|count| count.replace(Some(0)).unwrap_or(0))
}

/// Weak-head beta/iota/zeta and guarded fixpoint reduction without delta,
/// primitives, or proof search. The caller shares one budget across an entire congruence proof.
/// A depleted budget is inconclusive. Annotations and unused arguments stay lazy.
pub(crate) fn beta_iota_zeta_head(ctx: &Context, term: &Term, budget: &mut usize) -> Option<Term> {
    let _clone_nodes = bounded_conversion_size(term)?;
    #[cfg(test)]
    BOUNDED_HEAD_CLONE_NODES.with(|count| {
        if let Some(previous) = count.get() {
            count.set(Some(previous + _clone_nodes));
        }
    });
    // Continuations own the siblings which the recursive evaluator used to
    // leave in its callers. Moving a demanded child avoids copying its prefix;
    // every evaluation visit still spends the original shared work allowance.
    enum Frame {
        Apply(Term),
        Match { motive: Term, cases: Vec<Term> },
    }
    let mut frames = Vec::new();
    let mut current = term.clone();
    let mut entering = false; // the original input was preflighted above
    'evaluate: loop {
        #[cfg(test)]
        BOUNDED_HEAD_VISITS.with(|count| {
            if let Some(previous) = count.get() {
                count.set(Some(previous + 1));
            }
        });
        if entering {
            bounded_conversion_size(&current)?;
        }
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        // A descended child was checked above, before spending its fuel.
        // Only rewritten terms need the post-fuel check; current is unchanged here.
        if !entering {
            bounded_conversion_size(&current)?;
        }
        match current {
            Term::Let {
                name, value, body, ..
            } => {
                current = bounded_conversion_substitute_owned(*body, &name, *value)?;
                entering = false;
                continue;
            }
            Term::App(function, argument) => {
                frames.push(Frame::Apply(*argument));
                current = *function;
                entering = true;
                continue;
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                frames.push(Frame::Match {
                    motive: *motive,
                    cases,
                });
                current = *discriminant;
                entering = true;
                continue;
            }
            _ => {}
        }
        // Returning through a continuation is not a fresh evaluation visit.
        // This exactly mirrors the old call return, including partial spines.
        while let Some(frame) = frames.pop() {
            match frame {
                Frame::Apply(argument) => {
                    if let Term::Lambda { param, body, .. } = current {
                        current = bounded_conversion_substitute_owned(*body, &param, argument)?;
                        entering = false;
                        continue 'evaluate;
                    }
                    let applied = Term::App(Box::new(current), Box::new(argument));
                    let (head, args) = app_spine(&applied);
                    if let Term::Fix { name, body } = head {
                        let column = (args.iter().any(|a| is_constructor_form(ctx, a))
                            && crate::termination::check_termination(ctx, name, body).is_ok())
                        .then(|| crate::termination::structural_columns(ctx, body))
                        .and_then(|columns| columns.first().copied());
                        if column
                            .and_then(|i| args.get(i))
                            .is_some_and(|arg| is_constructor_form(ctx, arg))
                        {
                            let unfolded = bounded_conversion_substitute(body, name, head)?;
                            current = args.iter().fold(unfolded, |f, a| {
                                Term::App(Box::new(f), Box::new((*a).clone()))
                            });
                            entering = false;
                            continue 'evaluate;
                        }
                    }
                    current = applied;
                }
                Frame::Match { motive, cases } => {
                    let full_inventory = constructor_inductive_of(ctx, &current)
                        .is_some_and(|ind| ctx.get_constructors(&ind).len() == cases.len());
                    if let Some(branch) = full_inventory
                        .then(|| iota_branch(ctx, &current, &motive, &cases))
                        .flatten()
                    {
                        current = branch;
                        entering = false;
                        continue 'evaluate;
                    }
                    current = Term::Match {
                        discriminant: Box::new(current),
                        motive: Box::new(motive),
                        cases,
                    };
                }
            }
        }
        return Some(current);
    }
}

/// Read the result of a declared application spine without validating its
/// arguments. Only a negative eligibility filter may consume this hint. Opaque
/// type aliases and oversized substitutions return no hint and use full typing.
fn declared_spine_type(ctx: &Context, term: &Term) -> Option<Term> {
    bounded_conversion_size(term)?;
    let mut head = term;
    let mut args = Vec::new();
    while let Term::App(f, a) = head {
        args.push(a.as_ref());
        head = f;
    }
    if !matches!(head, Term::Var(_) | Term::Global(_) | Term::Const { .. }) {
        return None;
    }
    let mut ty = crate::infer_type(ctx, head).ok()?;
    bounded_conversion_size(&ty)?;
    for arg in args.into_iter().rev() {
        let Term::Pi {
            param, body_type, ..
        } = ty
        else {
            return None;
        };
        ty = bounded_conversion_substitute(&body_type, &param, arg)?;
    }
    Some(ty)
}

/// Proof irrelevance permits replacing a K-like proof by its nullary constructor
/// only when their complete types, including every index, are convertible.
/// K-probe answers remembered for the rest of ONE normalization.
///
/// Full normalization walks the whole term on every step, so a single-case match whose
/// discriminant is a stuck proof is probed again on every step while other redexes are
/// contracted — Init's `WellFounded.Nat.fix_eq` spent almost all of its time re-deciding the
/// same failed probe. The probe proper ([`k_constructor_checked`]) is a pure function of the
/// proof and the context it is probed in. Each `normalize_progress` run has one root context and
/// the in-place walker extends it only through the λ/Π binders it descends; `Context::extend`
/// replaces a name's binding, so that context is the root plus, for every name bound on the
/// walker's path, that name's current type. The key records exactly those types (read from the
/// probed context itself) and the proof, by structural identity. A nested normalization (the
/// probe type-checks and converts) runs with its own memo, and the probe runs with none, so no
/// answer crosses a different root context. The cheap motive filter runs before any lookup.
struct KProbeMemo {
    /// Created on the first probe that passes the motive filter: most normalizations probe
    /// nothing, and must not pay for a store they never use.
    store: Option<crate::term_store::TermStore>,
    path: Vec<String>,
    answers: std::collections::HashMap<
        (
            Vec<(String, crate::term_store::TermId)>,
            crate::term_store::TermId,
        ),
        Option<Term>,
    >,
}
/// Memory bound for one normalization's remembered probes; beyond it the memo is dropped and
/// every further probe is computed afresh, exactly as without the memo.
const K_PROBE_MEMO_ENTRIES: usize = 256;
const K_PROBE_MEMO_NODES: usize = 1 << 16;
thread_local! {
    static K_PROBE_MEMO: std::cell::RefCell<Option<KProbeMemo>> = const { std::cell::RefCell::new(None) };
}
struct KProbeScope(Option<KProbeMemo>);
impl Drop for KProbeScope {
    fn drop(&mut self) {
        K_PROBE_MEMO.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}
impl KProbeMemo {
    /// A fresh memo for this normalization; the enclosing one is restored on exit or unwind.
    fn install() -> KProbeScope {
        let fresh = KProbeMemo {
            store: None,
            path: Vec::new(),
            answers: std::collections::HashMap::new(),
        };
        KProbeScope(K_PROBE_MEMO.with(|slot| slot.borrow_mut().replace(fresh)))
    }
    /// Record that the walker descends under a binder named `param`.
    fn enter(param: &str) -> bool {
        K_PROBE_MEMO.with(|slot| match slot.borrow_mut().as_mut() {
            Some(memo) => {
                memo.path.push(param.to_owned());
                true
            }
            None => false,
        })
    }
    fn leave(pushed: bool) {
        if !pushed {
            return;
        }
        K_PROBE_MEMO.with(|slot| {
            if let Some(memo) = slot.borrow_mut().as_mut() {
                memo.path.pop();
            }
        });
    }
    fn probe(ctx: &Context, discriminant: &Term, motive: &Term) -> Option<Term> {
        if !k_motive_admits(ctx, motive) {
            return None;
        }
        let key = K_PROBE_MEMO.with(|slot| {
            let mut slot = slot.borrow_mut();
            let memo = slot.as_mut()?;
            let mut names = memo.path.clone();
            names.sort();
            names.dedup();
            let store = memo
                .store
                .get_or_insert_with(crate::term_store::TermStore::new);
            let mut locals = Vec::with_capacity(names.len());
            for name in names {
                // A path name is bound in the probed context by construction.
                let ty = ctx.get(&name)?;
                let id = store.intern(ty);
                locals.push((name, id));
            }
            let key = (locals, store.intern(discriminant));
            Some(match memo.answers.get(&key) {
                Some(answer) => Err(answer.clone()),
                None => Ok(key),
            })
        });
        let key = match key {
            Some(Err(answer)) => return answer,
            Some(Ok(key)) => Some(key),
            None => None,
        };
        // The probe type-checks and converts; it must neither see nor extend this memo.
        let suspended = K_PROBE_MEMO.with(|slot| slot.borrow_mut().take());
        let answer = k_constructor_checked(ctx, discriminant);
        K_PROBE_MEMO.with(|slot| *slot.borrow_mut() = suspended);
        if let Some(key) = key {
            K_PROBE_MEMO.with(|slot| {
                let mut slot = slot.borrow_mut();
                let Some(memo) = slot.as_mut() else {
                    return;
                };
                if memo.answers.len() >= K_PROBE_MEMO_ENTRIES
                    || memo
                        .store
                        .as_ref()
                        .is_some_and(|store| store.len() >= K_PROBE_MEMO_NODES)
                {
                    *slot = None;
                    return;
                }
                memo.answers.insert(key, answer.clone());
            });
        }
        answer
    }
}

fn k_constructor_form(ctx: &Context, proof: &Term, motive: &Term) -> Option<Term> {
    if !k_motive_admits(ctx, motive) {
        return None;
    }
    k_constructor_checked(ctx, proof)
}

/// The cheap negative filter: `false` only when the motive's visible major-premise type is an
/// inductive family with neither a K nor a unit constructor.
fn k_motive_admits(ctx: &Context, motive: &Term) -> bool {
    // Indexed motives abstract over indices before the major premise. Only
    // use the final domain when the lambda telescope visibly ends in a type;
    // an eta-short motive may still hide more binders in its final application.
    // This negative filter never establishes a reduction. Candidates still
    // undergo full proof inference, sort, constructor and index checks.
    let mut current = motive;
    let mut major_type = None;
    while let Term::Lambda {
        param_type, body, ..
    } = current
    {
        major_type = Some(param_type.as_ref());
        current = body;
    }
    let exposed = beta_iota_zeta_head(ctx, current, &mut 64);
    let terminal = exposed.as_ref().unwrap_or(current);
    let terminal_type = matches!(terminal, Term::Sort(_) | Term::Pi { .. })
        || matches!(declared_spine_type(ctx, terminal), Some(Term::Sort(_)));
    if let Some(ty) = major_type.filter(|_| terminal_type) {
        let mut head = ty;
        while let Term::App(f, _) = head {
            head = f;
        }
        if let Term::Global(name) | Term::Const { name, .. } = head {
            if ctx.is_inductive(name)
                && ctx.k_constructor(name).is_none()
                && ctx.unit_constructor(name).is_none()
            {
                return false;
            }
        }
    }
    true
}

/// The K probe proper: infer the proof's type and check it against the constructor's. Its
/// answer depends only on `proof` and `ctx`.
fn k_constructor_checked(ctx: &Context, proof: &Term) -> Option<Term> {
    #[cfg(test)]
    K_PROBES.with(|count| count.set(count.get() + 1));
    let proof_type = crate::infer_type(ctx, proof).ok()?;
    let proof_type = normalize_checked(ctx, &proof_type).ok()?;
    let mut head = &proof_type;
    while let Term::App(f, _) = head {
        head = f;
    }
    let (inductive, levels) = match head {
        Term::Global(n) => (n.as_str(), None),
        Term::Const { name, levels } => (name.as_str(), Some(levels)),
        _ => return None,
    };
    let unit = ctx.unit_constructor(inductive);
    let constructor = ctx.k_constructor(inductive).or(unit)?;
    let sort = crate::type_checker::infer_sort(ctx, &proof_type).ok()?;
    if unit.is_none() && (sort == crate::Universe::SProp || !sort.equiv(&crate::Universe::Prop)) {
        return None;
    }
    let mut value = match levels {
        Some(levels) => Term::Const {
            name: constructor.into(),
            levels: levels.clone(),
        },
        None => Term::Global(constructor.into()),
    };
    for arg in spine_args(&proof_type)
        .into_iter()
        .take(ctx.inductive_num_params(inductive))
    {
        value = Term::App(Box::new(value), Box::new(arg));
    }
    let value_type = crate::infer_type(ctx, &value).ok()?;
    crate::type_checker::def_eq(ctx, &proof_type, &value_type).then_some(value)
}

#[cfg(test)]
thread_local! {
    static K_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// What the reducer may do with a `Term::Const { name, levels }` reference.
///
/// ONE WELL-FORMEDNESS TEST, NOT TWO. Whether `name.{levels}` is a term of the calculus is
/// decided by [`crate::term::resolve_const_levels`], which is the same question `infer_type`
/// asks and the same one `recheck::const_body` asks — so a reference typing refuses can never
/// be a normal form to the reducer. The arm this replaces asked only HALF of it: it compared
/// `params.len()` against `levels.len()` itself, and only for a name in the
/// universe-polymorphic map. The consequence was live, and it is a typing/reduction divergence
/// of exactly the kind this codebase calls conformance defect 3: a declaration whose TYPE is
/// not level-CLOSED — a level variable outside its telescope — can never be instantiated, so
/// `infer_type` refuses every reference to it, while the reducer's arity test passed (`0 == 0`)
/// and it UNFOLDED THE BODY. `normalize` then made an ill-formed reference definitionally equal
/// to a well-typed term, which is laundering rather than mere disagreement.
///
/// [`ConstStep::Unresolved`] is the one case that stays inert, and the line is drawn where it is
/// on purpose. A reference that does not RESOLVE — no declaration at all, or a declaration with
/// no level telescope, which is reached as a `Term::Global` and never as a `Const` — is an
/// UNBOUND reference, and reduction has always left an unbound `Term::Global` alone: whether a
/// name is bound is the type checker's business. A reference that DOES resolve and is ill-formed
/// against the declaration it resolved to is a different fact, and it is the one that laundered.
enum ConstStep {
    /// The reference does not resolve — nothing is declared under the name, or what is declared
    /// has no level telescope and is reachable only as a `Term::Global`. Inert, exactly as an
    /// unbound `Term::Global` is inert.
    Unresolved,
    /// The reference is not a term of the calculus. `infer_type` refuses it, and so must the
    /// reducer's answer: see the `Term::Hole` note on the `Term::Const` arm.
    Illformed,
    /// Well formed, and the declaration has no body — an axiom, inductive or constructor is a
    /// VALUE, so there is no redex.
    Value,
    /// Well formed, with a body to unfold under this universe substitution.
    Delta(
        std::collections::HashMap<String, crate::term::Universe>,
        Term,
    ),
}

#[cfg(test)]
thread_local! {
    // Actual type/body clone events at polymorphic delta resolution.
    static CONST_STEP_CLONES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Resolve a `Const` reference against the environment — the reducer's own lookup, written
/// from the rule rather than shared, with the DECISION itself delegated to the one resolver.
fn const_step(ctx: &Context, name: &str, levels: &[crate::term::Universe]) -> ConstStep {
    // A `Const` is the UNIVERSE-POLYMORPHIC reference form. A declaration carries a telescope
    // either by declaring one (`universe_polys`) or by mentioning a level variable in its type;
    // a declaration with neither is reachable only as a `Global`.
    let (declared, ty, body) = match ctx.get_universe_poly(name) {
        Some((params, ty, body)) => {
            #[cfg(test)]
            CONST_STEP_CLONES.with(|count| count.set(count.get() + 2));
            (Some(params.clone()), ty.clone(), Some(body.clone()))
        }
        None => match ctx.get_global(name) {
            None => return ConstStep::Unresolved,
            Some(ty) if !crate::term::mentions_level_var(ty) => return ConstStep::Unresolved,
            Some(ty) => {
                let body = ctx.get_definition_body(name);
                #[cfg(test)]
                CONST_STEP_CLONES
                    .with(|count| count.set(count.get() + 1 + usize::from(body.is_some())));
                (None, ty.clone(), body.cloned())
            }
        },
    };
    match crate::term::resolve_const_levels(name, declared.as_deref(), &ty, levels) {
        Err(_) => ConstStep::Illformed,
        Ok(subst) => match body {
            Some(body) => ConstStep::Delta(subst, body),
            None => ConstStep::Value,
        },
    }
}

// -------------------------------------------------------------------------
// The `@[simp]` registry, CONSULTED
// -------------------------------------------------------------------------

/// The head symbol of an application spine — the `@[simp]` registry's DISPATCH KEY.
///
/// Written here rather than borrowed because the reducer must ask the question the registry
/// answered: `SimpRule::compile` keys a rule by its left-hand side's head, and a rewriter that
/// computed the head differently would look in the wrong bucket and silently never fire.
fn simp_dispatch_head(t: &Term) -> Option<&str> {
    let mut cur = t;
    while let Term::App(f, _) = cur {
        cur = f;
    }
    match cur {
        Term::Global(n) => Some(n.as_str()),
        Term::Const { name, .. } => Some(name.as_str()),
        _ => None,
    }
}

/// Try to rewrite `term` at the TOP with a registered `@[simp]` rule.
///
/// **Why this is tried BEFORE δ and not after.** The obvious placement — "at every subterm
/// still stuck after β/δ/ι" — cannot work for the shape simp lemmas exist for. A rule's
/// left-hand side is headed by a DEFINED constant (`size n ⟶ n`), and δ unfolds that constant
/// on the very first step, so by the time a term is stuck the pattern is gone: `size (size n)`
/// becomes `plus (plus n 0) 0`, which no `size` rule can ever match again. Lean's `simp`
/// rewrites in preference to unfolding for precisely this reason, and so does this.
///
/// **What it costs when nobody registers anything**, which is every context the prelude
/// builds: one `is_empty` on a slice. The dispatch beyond that is one spine walk and one hash
/// lookup per node — never a linear scan of [`Context::simp_rules`], which is what the
/// head-keyed registry exists to prevent.
///
/// **Why it cannot loop.** `SimpRule::compile` refuses `lhs = lhs` and a bare-variable
/// left-hand side at the door, so no single rule rewrites a term to itself or matches
/// everywhere; [`progressed`] refuses a rewrite that did not change the term; and a CYCLE
/// between two rules is caught by [`normalize_checked_at`]'s Brent detection as
/// [`Unreduced::Cyclic`], which is the honest answer — such a term has no normal form.
///
/// **What it does NOT do.** Matching is first-order and syntactic, exactly as
/// [`crate::SimpRule::rewrite`] performs it — the rule and its matcher live together so a
/// rewriter cannot accept what `add_simp_lemma` refused. It never matches up to conversion:
/// a rule about `bulk` must not fire on a definitionally-equal `size` application, or one
/// registered lemma would rewrite every term in the environment.
/// **The SECOND kernel's door.** `recheck.rs::whnf` must apply the same rules or a proof that
/// leans on a rewrite is accepted by one kernel and refused by the other. The matching itself
/// is `SimpRule::rewrite`, which lives with the rule so the two rewriters cannot disagree about
/// what a registered lemma means; only the WALK is each kernel's own.
pub(crate) fn try_simp_rewrite(ctx: &Context, term: &Term) -> Option<Term> {
    if ctx.simp_rules().is_empty() {
        return None;
    }
    let head = simp_dispatch_head(term)?;
    ctx.simp_rules_for(head).find_map(|rule| {
        let rewritten = rule.rewrite(term)?;
        if captures_a_free_name(term, &rewritten) {
            return None;
        }
        progressed(term, rewritten)
    })
}

/// Whether a rewrite would CAPTURE — turn a variable that was free in `term` into one bound by
/// a binder the rule's right-hand side brought with it.
///
/// **This is a real hazard and it is the price of wiring a rewriter into a NAMED-variable
/// kernel.** `SimpRule::rewrite` substitutes the matched terms into the rule's right-hand side
/// with a substitution that tracks SHADOWING of the pattern variables — which is the right check
/// for the pattern variable's own name and no check at all for the free names of what it was
/// bound to. So a lemma whose right-hand side BINDS a name, `∀x. g x = (λn. x)`, rewrites `g n`
/// in a scope holding a local `n` to `λn. n` — the identity where the lemma says the constant
/// function. That is a wrong ANSWER inside `normalize`, therefore inside `def_eq`, therefore
/// inside every `rfl`.
///
/// The rule that would fire is DECLINED rather than α-renamed, deliberately: renaming is the
/// matcher's job and belongs with the matcher (`SimpRule::rewrite`, which owns the substitution
/// and is the only place that can rename correctly). Declining is the conservative half — it can
/// only make the simplifier weaker, never wrong — and it costs nothing in practice, because a
/// right-hand side with no binder in it can never capture and that is almost every simp lemma.
fn captures_a_free_name(term: &Term, rewritten: &Term) -> bool {
    let mut free: Vec<&str> = Vec::new();
    collect_free_names(term, &mut Vec::new(), &mut free);
    if free.is_empty() {
        return false;
    }
    binds_any_of(rewritten, &free)
}

/// The FREE variable names of `t`, first-occurrence order, duplicates removed.
fn collect_free_names<'a>(t: &'a Term, bound: &mut Vec<&'a str>, out: &mut Vec<&'a str>) {
    match t {
        Term::Var(n) => {
            if !bound.iter().any(|b| *b == n.as_str()) && !out.iter().any(|o| *o == n.as_str()) {
                out.push(n.as_str());
            }
        }
        Term::Sort(_) | Term::Global(_) | Term::Const { .. } | Term::Lit(_) | Term::Hole => {}
        Term::App(f, a) => {
            collect_free_names(f, bound, out);
            collect_free_names(a, bound, out);
        }
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            collect_free_names(param_type, bound, out);
            bound.push(param.as_str());
            collect_free_names(body_type, bound, out);
            bound.pop();
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            collect_free_names(param_type, bound, out);
            bound.push(param.as_str());
            collect_free_names(body, bound, out);
            bound.pop();
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            collect_free_names(ty, bound, out);
            collect_free_names(value, bound, out);
            bound.push(name.as_str());
            collect_free_names(body, bound, out);
            bound.pop();
        }
        Term::Fix { name, body } => {
            bound.push(name.as_str());
            collect_free_names(body, bound, out);
            bound.pop();
        }
        Term::MutualFix { defs, .. } => {
            for (n, _) in defs {
                bound.push(n.as_str());
            }
            for (_, b) in defs {
                collect_free_names(b, bound, out);
            }
            for _ in defs {
                bound.pop();
            }
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            collect_free_names(discriminant, bound, out);
            collect_free_names(motive, bound, out);
            for c in cases {
                collect_free_names(c, bound, out);
            }
        }
        Term::Annotated { term, .. } => collect_free_names(term, bound, out),
    }
}

/// Whether `t` binds any of `names` anywhere inside it.
fn binds_any_of(t: &Term, names: &[&str]) -> bool {
    let binds = |n: &String| names.iter().any(|x| *x == n.as_str());
    match t {
        Term::Sort(_)
        | Term::Var(_)
        | Term::Global(_)
        | Term::Const { .. }
        | Term::Lit(_)
        | Term::Hole => false,
        Term::App(f, a) => binds_any_of(f, names) || binds_any_of(a, names),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => binds(param) || binds_any_of(param_type, names) || binds_any_of(body_type, names),
        Term::Lambda {
            param,
            param_type,
            body,
        } => binds(param) || binds_any_of(param_type, names) || binds_any_of(body, names),
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            binds(name)
                || binds_any_of(ty, names)
                || binds_any_of(value, names)
                || binds_any_of(body, names)
        }
        Term::Fix { name, body } => binds(name) || binds_any_of(body, names),
        Term::MutualFix { defs, .. } => {
            defs.iter().any(|(n, b)| binds(n) || binds_any_of(b, names))
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            binds_any_of(discriminant, names)
                || binds_any_of(motive, names)
                || cases.iter().any(|c| binds_any_of(c, names))
        }
        Term::Annotated { term, .. } => binds_any_of(term, names),
    }
}

/// Check if a term is a constructor (possibly applied to arguments).
fn is_constructor_form(ctx: &Context, term: &Term) -> bool {
    // Integer literals retain their existing canonical-signature bridge. All
    // other forms use borrowed metadata and never construct discarded fields.
    if matches!(term, Term::Lit(Literal::Int(_) | Literal::BigInt(_))) {
        return extract_constructor(ctx, term).is_some();
    }
    constructor_view(ctx, term).is_some()
}

/// Which entry of a `Match`'s case list belongs to the discriminant's constructor.
///
/// Usually that is the constructor's own position in its inductive's registration order. But an
/// INDEXED family's match may be PRUNED — a constructor whose declared result index cannot equal
/// the scrutinee's owes no branch (SPEC F, `type_checker::constructors_the_indices_allow`) — and
/// then the case list is SHORTER than the constructor list: the second of two constructors is the
/// FIRST of a one-case list. A `Term::Match` does not record which constructors its cases stand
/// for, so the mapping is recomputed here, from the scrutinee's own indices.
///
/// This is where the ι-rule would otherwise go quietly wrong rather than merely get stuck:
/// indexing a pruned list by the full position applies the WRONG branch to the right arguments.
/// `None` means "cannot be determined" and no reduction happens — sound, and the type checker
/// will have refused any match this cannot resolve.
fn case_position(
    ctx: &Context,
    discriminant: &Term,
    ctor_index: usize,
    num_cases: usize,
) -> Option<usize> {
    // The overwhelming case: a full case list indexes directly.
    let ind = constructor_inductive_of(ctx, discriminant)?;
    let ctors = ctx.get_constructors(&ind);
    if num_cases == ctors.len() {
        return Some(ctor_index);
    }
    // A pruned list. Recover the scrutinee's indices from the constructor's declared result type
    // with its ACTUAL arguments substituted, then ask which constructors those indices allow.
    let num_params = ctx.inductive_num_params(&ind);
    let (_, ctor_type) = *ctors.get(ctor_index)?;
    let actual = spine_args(discriminant);
    let mut binders: Vec<String> = Vec::new();
    let mut cur = ctor_type;
    while let Term::Pi {
        param, body_type, ..
    } = cur
    {
        binders.push(param.clone());
        cur = body_type;
    }
    let mut result = cur.clone();
    for (binder, value) in binders.iter().zip(actual.iter()) {
        result = substitute(&result, binder, value);
    }
    let result_args = spine_args(&result);
    let allowed =
        crate::type_checker::constructors_the_indices_allow(ctx, &ind, num_params, &result_args);
    if allowed.len() != num_cases {
        return None;
    }
    allowed.iter().position(|&i| i == ctor_index)
}

/// The inductive a constructor-headed term belongs to, bridging a `Nat` literal to `Nat`.
fn constructor_inductive_of(ctx: &Context, t: &Term) -> Option<String> {
    if crate::int_inductive::literal_constructor(ctx, t).is_some() {
        return Some("Int".to_string());
    }
    if matches!(t, Term::Lit(Literal::Nat(_))) {
        return Some("Nat".to_string());
    }
    let mut head = t;
    while let Term::App(f, _) = head {
        head = f;
    }
    match head {
        Term::Global(n) | Term::Const { name: n, .. } => {
            ctx.constructor_inductive(n).map(|s| s.to_string())
        }
        _ => None,
    }
}

/// An application spine's arguments, in source order.
fn spine_args(t: &Term) -> Vec<Term> {
    let mut args = Vec::new();
    let mut cur = t;
    while let Term::App(f, a) = cur {
        args.push((**a).clone());
        cur = f;
    }
    args.reverse();
    args
}

/// Extract constructor index and VALUE arguments from a term (skipping type arguments).
///
/// Returns `Some((constructor_index, [value_arg1, value_arg2, ...]))` if term is `Cᵢ(type_args..., value_args...)`
/// where `Cᵢ` is the i-th constructor of some inductive type.
///
/// For polymorphic constructors like `Cons A h t`, this returns only [h, t], not [A, h, t].
/// Borrowed constructor metadata. Field materialization belongs to eliminators,
/// not to the boolean recognition performed while looking for a fixpoint redex.
enum ConstructorView<'a> {
    Named {
        index: usize,
        parameters: usize,
        term: &'a Term,
    },
    Natural {
        index: usize,
        value: &'a logicaffeine_base::BigInt,
    },
}
fn constructor_view<'a>(ctx: &Context, term: &'a Term) -> Option<ConstructorView<'a>> {
    if let Term::Lit(Literal::Nat(value)) = term {
        if !crate::int_inductive::reads_nat_literal(ctx, value) {
            return None;
        }
        let constructor = if *value <= logicaffeine_base::BigInt::from_i64(0) {
            "Zero"
        } else {
            "Succ"
        };
        let index = ctx
            .constructor_headers("Nat")
            .position(|(name, _)| name == constructor)?;
        return Some(ConstructorView::Natural { index, value });
    }
    let mut head = term;
    while let Term::App(f, _) = head {
        head = f;
    }
    let (Term::Global(name) | Term::Const { name, .. }) = head else {
        return None;
    };
    let owner = ctx.constructor_inductive(name)?;
    let (index, (_, ty)) = ctx
        .constructor_headers(owner)
        .enumerate()
        .find(|(_, (candidate, _))| *candidate == name)?;
    let parameters = ctx
        .inductive_declared_params(owner)
        .unwrap_or_else(|| count_type_params(ty));
    Some(ConstructorView::Named {
        index,
        parameters,
        term,
    })
}
/// `term` as an explicit constructor application: itself when its head is a constructor, and one
/// constructor layer of a numeral (`Zero`, `Succ n`, `Int_ofNat n`, `Int_negSucc n`).
pub(crate) fn constructor_form(ctx: &Context, term: &Term) -> Option<Term> {
    if let Some(layer) = crate::int_inductive::literal_constructor(ctx, term) {
        return Some(layer);
    }
    match constructor_view(ctx, term)? {
        ConstructorView::Natural { value, .. }
            if *value <= logicaffeine_base::BigInt::from_i64(0) =>
        {
            Some(Term::Global("Zero".to_string()))
        }
        ConstructorView::Natural { value, .. } => Some(Term::App(
            Box::new(Term::Global("Succ".to_string())),
            Box::new(Term::Lit(Literal::Nat(
                value.sub(&logicaffeine_base::BigInt::from_i64(1)),
            ))),
        )),
        ConstructorView::Named { .. } => Some(term.clone()),
    }
}

fn extract_constructor(ctx: &Context, term: &Term) -> Option<(usize, Vec<Term>)> {
    #[cfg(test)]
    CONSTRUCTOR_MATERIALIZATIONS.with(|n| n.set(n.get() + 1));
    if let Some(constructor) = crate::int_inductive::literal_constructor(ctx, term) {
        return extract_constructor(ctx, &constructor);
    }
    match constructor_view(ctx, term)? {
        ConstructorView::Natural { index, value } => {
            let fields = if *value <= logicaffeine_base::BigInt::from_i64(0) {
                Vec::new()
            } else {
                vec![Term::Lit(Literal::Nat(
                    value.sub(&logicaffeine_base::BigInt::from_i64(1)),
                ))]
            };
            Some((index, fields))
        }
        ConstructorView::Named {
            index,
            parameters,
            term,
        } => {
            let (_, args) = app_spine(term);
            Some((index, args.into_iter().skip(parameters).cloned().collect()))
        }
    }
}

/// Count leading type parameters in a constructor type.
///
/// Type parameters are Pis where the param_type is a Sort (Type n or Prop).
/// For `Π(A:Type). Π(_:A). Π(_:List A). List A`, this returns 1.
fn count_type_params(ty: &Term) -> usize {
    let mut count = 0;
    let mut current = ty;

    while let Term::Pi {
        param_type,
        body_type,
        ..
    } = current
    {
        if is_sort(param_type) {
            count += 1;
            current = body_type;
        } else {
            break;
        }
    }

    count
}

/// Check if a term is a Sort (Type n or Prop).
fn is_sort(term: &Term) -> bool {
    matches!(term, Term::Sort(_))
}

/// The head and argument spine of an application, by reference (`f a b c` → `(f, [a,b,c])`).
fn app_spine(t: &Term) -> (&Term, Vec<&Term>) {
    let mut args = Vec::new();
    let mut cur = t;
    while let Term::App(f, a) = cur {
        args.push(a.as_ref());
        cur = f.as_ref();
    }
    args.reverse();
    (cur, args)
}

/// The certified observational equality's computation rules (`observational::reduce`), offered
/// the application `func arg`.
fn try_obs_reduce(ctx: &Context, func: &Term, arg: &Term) -> Option<Term> {
    ctx.observational.as_ref()?;
    let (head, mut args) = app_spine(func);
    args.push(arg);
    crate::observational::reduce(ctx, head, &args)
}

/// Quotient elimination applies the function to a constructed representative.
/// Legacy lift spines have a respect proof; certified polymorphic induction
/// has a Prop-valued motive and one fewer argument. Imported identities come
/// from checked certificates, and representative universes must agree.
fn try_quot_lift_reduce(ctx: &Context, func: &Term, arg: &Term) -> Option<Term> {
    let (fh, fargs) = app_spine(func);
    let (arity, universe) = match fh {
        Term::Global(n) => {
            let arity = match crate::quotient::prelude_kind(ctx, n)? {
                crate::quotient::PreludeKind::Lift => 5,
                crate::quotient::PreludeKind::Induction => 4,
                _ => return None,
            };
            (arity, None)
        }
        Term::Const { name, levels } => {
            let arity = match crate::quotient::kind(ctx, name, levels)? {
                crate::quotient::Kind::Lift => 5,
                crate::quotient::Kind::Induction => 4,
                _ => return None,
            };
            (arity, Some(&levels[0]))
        }
        _ => return None,
    };
    if fargs.len() != arity {
        return None;
    }
    let (qh, qargs) = app_spine(arg);
    let constructor = match (qh, universe) {
        (Term::Global(n), None) => {
            crate::quotient::prelude_kind(ctx, n) == Some(crate::quotient::PreludeKind::Constructor)
        }
        (Term::Const { name, levels }, Some(u)) => {
            crate::quotient::kind(ctx, name, levels) == Some(crate::quotient::Kind::Constructor)
                && levels[0].equiv(u)
        }
        _ => false,
    };
    if !constructor || qargs.len() != 3 {
        return None;
    }
    Some(Term::App(
        Box::new(fargs[3].clone()),
        Box::new(qargs[2].clone()),
    ))
}

/// ι FOR A FIXPOINT, at the column the TERMINATION GUARD admitted it on.
///
/// The specification is [`crate::termination::structural_columns`]'s, quoted there in full:
///
/// > `(fix f. λx₀ … λxₙ. e) a₀ … aₙ` unrolls exactly when the argument at the STRUCTURAL
/// > position — `structural_columns(ctx, body)[0]` — is in constructor form.
///
/// The arm this stands beside asks the question of whatever argument the INNERMOST `App`
/// happens to carry, which is always `a₀`. That is a strictly smaller rule, and the definition
/// it leaves stuck is the commonest one there is: `Nat.add` recurses on its SECOND argument
/// (`fix f. λm. λn. match n with …`), so the guard admits it with the structural column at
/// position 1 and the reducer never unrolled it for a non-constructor `m`. `n + 0 = n` is
/// `rfl` in Lean and was not `rfl` here — and `normalize` handed back a term still holding a
/// δι-redex, which is the shape a conversion check cannot see past.
///
/// WHICH COLUMN is the guard's own answer and is asked of the guard, never guessed: a reducer
/// that picked the column itself would compute a function the guard never checked. WHAT TO DO
/// with the answer is this kernel's own code, and `recheck.rs` implements the same
/// specification independently over de Bruijn levels.
///
/// Both halves are needed for `double_check` to mean anything here, and neither may be a copy
/// of the other: today BOTH kernels are argument-0-only, so they agree by being wrong in the
/// same way and the second kernel cannot see it.
fn try_fix_unroll(ctx: &Context, term: &Term) -> Option<Term> {
    let (head, args) = app_spine(term);
    let Term::Fix { name, body } = head else {
        return None;
    };
    // The guard's analysis walks the body, so it is asked only once something could possibly
    // unroll. No argument in constructor form means no column can be satisfied, whichever one
    // the guard names — and that is the shape every STUCK application has, which is the shape
    // reduction spends most of its steps on.
    if !args.iter().any(|a| is_constructor_form(ctx, a)) {
        return None;
    }
    // An empty column list means the guard would REFUSE this body: there is no structural
    // position, so there is no ι rule and the fixpoint is a value.
    let column = *crate::termination::structural_columns(ctx, body).first()?;
    if !is_constructor_form(ctx, args.get(column)?) {
        return None;
    }
    let unfolded = substitute(body, name, head);
    Some(args.iter().fold(unfolded, |f, a| {
        Term::App(Box::new(f), Box::new((*a).clone()))
    }))
}

// -------------------------------------------------------------------------
// Primitive computation rules
// -------------------------------------------------------------------------

/// Lean 4's `Int.ediv` / `Int.emod` — the EUCLIDEAN convention, and TOTAL.
///
/// THE SPECIFICATION. Every engine that folds `div`/`mod` on literals implements THIS, each
/// from the text, never by copying another engine's code — `reduction::try_primitive_reduce`
/// (here), `recheck::try_builtin` (the second kernel), `eval::prim_arith` (the `native_decide`
/// evaluator) and `simp::try_arithmetic` (the constant folder that certifies `simp` goals):
///
/// ```text
///   b ≠ 0  ⟹  a = b·q + r   ∧   0 ≤ r < |b|      (the unique such pair)
///   b = 0  ⟹  q = 0  ∧  r = a                     (`a = 0·0 + a`, so the identity survives)
/// ```
///
/// Rust's `/` and `%` are TRUNCATED — the quotient rounds toward zero and the remainder
/// carries the DIVIDEND's sign — so they agree with the spec above on every non-negative
/// dividend and disagree on every negative one. That is exactly why a truncating
/// implementation survives casual testing and then answers `(-7) / 2 = -3` where Lean answers
/// `-4`: a silent wrong answer, the worst outcome there is.
///
/// This derivation corrects the truncated pair by one; `eval.rs` derives the same pair a
/// different way (recovering `q` by exact division once `r` is known), so a slip in one
/// derivation cannot hide in the other.
fn ediv_emod_i64(a: i64, b: i64) -> Option<(i64, i64)> {
    if b == 0 {
        return Some((0, a));
    }
    // `None` only for `i64::MIN / -1`, which overflows the machine quotient; the caller
    // promotes to the exact path rather than wrapping.
    let q = a.checked_div(b)?;
    let r = a.checked_rem(b)?;
    if r >= 0 {
        return Some((q, r));
    }
    if b > 0 {
        Some((q.checked_sub(1)?, r.checked_add(b)?))
    } else {
        Some((q.checked_add(1)?, r.checked_sub(b)?))
    }
}

/// The arbitrary-precision half of the same specification. Total in the same way, so the wide
/// path and the narrow path answer identically on every value both can represent.
fn ediv_emod_big(
    a: &logicaffeine_base::BigInt,
    b: &logicaffeine_base::BigInt,
) -> (logicaffeine_base::BigInt, logicaffeine_base::BigInt) {
    use logicaffeine_base::BigInt;
    let Some((q, r)) = a.div_rem(b) else {
        return (BigInt::from_i64(0), a.clone());
    };
    if !r.is_negative() {
        return (q, r);
    }
    let one = BigInt::from_i64(1);
    if !b.is_negative() {
        (q.sub(&one), r.add(b))
    } else {
        (q.add(&one), r.sub(b))
    }
}

fn bool_term(b: bool) -> Term {
    Term::Global(if b { "true" } else { "false" }.to_string())
}

/// The `Int` computation rules: arithmetic on a machine fast path that PROMOTES on overflow,
/// and order decided directly. `None` means "no rule for this name at these literal kinds",
/// which is how a `Nat`, `Text` or `Float` operand keeps the term stuck instead of being
/// silently read as an integer.
fn int_binary(op: &str, xl: &Literal, yl: &Literal) -> Option<Term> {
    // FAST PATH: both operands fit `i64`. Only an OVERFLOWING arithmetic op falls through —
    // to be redone exactly in BigInt below.
    if let (Literal::Int(a), Literal::Int(b)) = (xl, yl) {
        let fast = match op {
            "add" => a.checked_add(*b),
            "sub" => a.checked_sub(*b),
            "mul" => a.checked_mul(*b),
            "div" => ediv_emod_i64(*a, *b).map(|(q, _)| q),
            "mod" => ediv_emod_i64(*a, *b).map(|(_, r)| r),
            _ => None,
        };
        if let Some(r) = fast {
            return Some(Term::Lit(Literal::Int(r)));
        }
        // Bool-valued comparison primitives: ground order decided by computation, so
        // `Eq Bool (le m n) true` holds by `refl` exactly when `m ≤ n` — the sound
        // substrate for linear-arithmetic certificates.
        let fast_bool = match op {
            "le" => Some(a <= b),
            "lt" => Some(a < b),
            "ge" => Some(a >= b),
            "gt" => Some(a > b),
            _ => None,
        };
        if let Some(bl) = fast_bool {
            return Some(bool_term(bl));
        }
    }

    // EXACT PATH (K6): arbitrary-precision integer arithmetic, reached when an operand is a
    // `BigInt` or an `i64` op overflowed. Results are canonicalised by `int_lit` (demote to
    // `Int` when they fit), so a value has ONE literal and definitional equality of literals
    // stays sound.
    let (Some(xb), Some(yb)) = (lit_bigint(xl), lit_bigint(yl)) else {
        return None;
    };
    let big = match op {
        "add" => Some(xb.add(&yb)),
        "sub" => Some(xb.sub(&yb)),
        "mul" => Some(xb.mul(&yb)),
        "div" => Some(ediv_emod_big(&xb, &yb).0),
        "mod" => Some(ediv_emod_big(&xb, &yb).1),
        _ => None,
    };
    if let Some(r) = big {
        return Some(Term::Lit(int_lit(r)));
    }
    match op {
        "le" => Some(bool_term(xb <= yb)),
        "lt" => Some(bool_term(xb < yb)),
        "ge" => Some(bool_term(xb >= yb)),
        "gt" => Some(bool_term(xb > yb)),
        _ => None,
    }
}

/// The `Text` computation rules. Lean's `String` operations are recursions over
/// `String.data`; `Text` is an opaque carrier, so they are primitive here — but they must
/// COMPUTE, or `example : "ab" ++ "cd" = "abcd" := rfl` has no LOGOS form at all and an
/// axiomatised `Text_append` makes the equation FALSE rather than unproved.
///
/// `Text_length` counts CHARACTERS, as Lean's `String.length` does — not bytes.
fn text_binary(op: &str, xl: &Literal, yl: &Literal) -> Option<Term> {
    let (Literal::Text(a), Literal::Text(b)) = (xl, yl) else {
        return None;
    };
    match op {
        "Text_append" => Some(Term::Lit(Literal::Text(format!("{a}{b}")))),
        "Text_beq" => Some(bool_term(a == b)),
        "Text_isPrefixOf" => Some(bool_term(b.starts_with(a.as_str()))),
        _ => None,
    }
}

/// Every binary computation rule, keyed by name and by the literal kinds it accepts. A new
/// primitive type is a new ROW here, not another arm grafted onto the `Int` dispatch — which
/// is what kept every `Text` operation unstatable.
fn binary_prim(op: &str, xl: &Literal, yl: &Literal) -> Option<Term> {
    int_binary(op, xl, yl).or_else(|| text_binary(op, xl, yl))
}

/// Every UNARY computation rule, same shape.
fn unary_prim(op: &str, xl: &Literal) -> Option<Term> {
    match (op, xl) {
        ("Text_length", Literal::Text(s)) => Some(Term::Lit(Literal::Nat(
            logicaffeine_base::BigInt::from_u64(s.chars().count() as u64),
        ))),
        _ => None,
    }
}

// -------------------------------------------------------------------------
// The accelerated `Nat` order — Lean's GMP `Nat.ble`/`Nat.blt`, with a check Lean does not do
// -------------------------------------------------------------------------

/// THE SPECIFICATION the acceleration stands in for, written as a program because that is what
/// it is checked against:
///
/// ```text
///   natLe : Nat → Nat → Bool
///   natLe := fix f. λm. λn. match m { Zero   => true
///                                   | Succ m✧ => match n { Zero   => false
///                                                        | Succ n✧ => f m✧ n✧ } }
///   natLt : Nat → Nat → Bool
///   natLt := λm. λn. natLe (Succ m) n
/// ```
///
/// A `Literal::Nat(k)` enters that recursion through the Peano bridge in
/// [`extract_constructor`], which peels ONE `Succ` per unit — so deciding `m ≤ n` costs
/// `min(m, n)` whole-term reduction steps. `Char`'s validity bound is `< 0x110000`, which is
/// 1 114 112 steps against a 2^20 budget: the verdict is not slow, it is UNREACHABLE, and the
/// bound gets refused for want of arithmetic rather than for being false.
///
/// Lean solves this by overriding `Nat.ble`/`Nat.blt` in the kernel with GMP, keyed on the
/// constant's NAME and trusted to match the reference definition. **We do the same thing and
/// check it.** The override below fires only when the constant is bound to exactly the program
/// above — same skeleton, same case order, same recursive call, same declared type — so the
/// fast answer IS the slow answer by construction, and a context that rebinds `natLe` silently
/// loses the acceleration instead of silently getting the wrong value. That is a strictly
/// smaller act of trust than Lean's, for one small structural comparison per fired redex.
///
/// The clamp is the boundary a fast path gets wrong: the Peano bridge sends a NEGATIVE
/// `Literal::Nat` to `Zero` (a well-formed `Nat` is never negative, but the kernel can be
/// HANDED one), so the literal comparison has to clamp the same way or the two spellings of one
/// reduction disagree.
fn nat_order_value(op: &str, xl: &Literal, yl: &Literal) -> Option<bool> {
    let (Literal::Nat(a), Literal::Nat(b)) = (xl, yl) else {
        return None;
    };
    let zero = logicaffeine_base::BigInt::from_i64(0);
    let a = if a.is_negative() { &zero } else { a };
    let b = if b.is_negative() { &zero } else { b };
    match op {
        "natLe" => Some(a <= b),
        "natLt" => Some(a < b),
        _ => None,
    }
}

fn is_var_named(t: &Term, name: &str) -> bool {
    matches!(t, Term::Var(n) if n == name)
}

fn is_global_named(t: &Term, name: &str) -> bool {
    matches!(t, Term::Global(n) if n == name)
}

/// `Nat → Nat → Bool`, whatever the binders are called. Checked as well as the body, because a
/// program with the right shape at the wrong type is a different function.
fn is_nat_nat_bool(ty: &Term) -> bool {
    let Term::Pi {
        param_type: first,
        body_type: rest,
        ..
    } = ty
    else {
        return false;
    };
    let Term::Pi {
        param_type: second,
        body_type: result,
        ..
    } = rest.as_ref()
    else {
        return false;
    };
    is_global_named(first, "Nat")
        && is_global_named(second, "Nat")
        && is_global_named(result, "Bool")
}

/// Whether `body` is the `natLe` program in the specification above, up to the names of its
/// binders. The five binders must be pairwise DISTINCT: with shadowing, a term of this shape
/// can compute something else entirely, and a matcher that read the shape without the scoping
/// would accept it.
fn is_min_recursion_order(body: &Term) -> bool {
    let Term::Fix { name: f, body: abs } = body else {
        return false;
    };
    let Term::Lambda {
        param: m,
        body: abs_n,
        ..
    } = abs.as_ref()
    else {
        return false;
    };
    let Term::Lambda {
        param: n,
        body: outer,
        ..
    } = abs_n.as_ref()
    else {
        return false;
    };
    let Term::Match {
        discriminant: on_m,
        cases: m_cases,
        ..
    } = outer.as_ref()
    else {
        return false;
    };
    if m_cases.len() != 2 || !is_var_named(on_m, m) || !is_global_named(&m_cases[0], "true") {
        return false;
    }
    let Term::Lambda {
        param: m_pred,
        body: inner,
        ..
    } = &m_cases[1]
    else {
        return false;
    };
    let Term::Match {
        discriminant: on_n,
        cases: n_cases,
        ..
    } = inner.as_ref()
    else {
        return false;
    };
    if n_cases.len() != 2 || !is_var_named(on_n, n) || !is_global_named(&n_cases[0], "false") {
        return false;
    }
    let Term::Lambda {
        param: n_pred,
        body: call,
        ..
    } = &n_cases[1]
    else {
        return false;
    };
    let Term::App(partial, second) = call.as_ref() else {
        return false;
    };
    let Term::App(head, first) = partial.as_ref() else {
        return false;
    };
    let names = [f, m, n, m_pred, n_pred];
    let distinct = names
        .iter()
        .enumerate()
        .all(|(i, a)| names[i + 1..].iter().all(|b| a != b));
    distinct && is_var_named(head, f) && is_var_named(first, m_pred) && is_var_named(second, n_pred)
}

/// Whether `body` is `λm. λn. <le> (Succ m) n` — the `natLt` program, which is only the right
/// function if `<le>` is itself the accelerated order.
fn is_successor_shifted_order(body: &Term, le: &str) -> bool {
    let Term::Lambda {
        param: m,
        body: abs_n,
        ..
    } = body
    else {
        return false;
    };
    let Term::Lambda {
        param: n,
        body: call,
        ..
    } = abs_n.as_ref()
    else {
        return false;
    };
    let Term::App(partial, second) = call.as_ref() else {
        return false;
    };
    let Term::App(head, first) = partial.as_ref() else {
        return false;
    };
    let Term::App(succ, shifted) = first.as_ref() else {
        return false;
    };
    m != n
        && is_global_named(head, le)
        && is_global_named(succ, "Succ")
        && is_var_named(shifted, m)
        && is_var_named(second, n)
}

/// Whether `natLe` in this context IS the program the acceleration computes.
fn accelerates_nat_le(ctx: &Context) -> bool {
    ctx.get_global("natLe").is_some_and(is_nat_nat_bool)
        && ctx
            .get_definition_body("natLe")
            .is_some_and(is_min_recursion_order)
}

/// The shortcut reads compact Nat literals as Zero/Succ and returns a constructor
/// named true/false. Those are semantic dependencies, even when both order bodies
/// are unchanged: a checked alias true : Bool := false must evaluate to false,
/// and a legacy mutation of Nat's positional inventory must use the actual body.
fn has_nat_order_carriers(ctx: &Context) -> bool {
    if !crate::int_inductive::canonical_nat_environment(ctx)
        || !ctx.is_inductive("Bool")
        || ctx.get_global("Bool") != Some(&Term::Sort(crate::term::Universe::Type(0)))
        || ctx.inductive_num_params("Bool") != 0
        || ctx.poly_declaration("Bool").is_some()
        || ctx.get_definition_body("Bool").is_some()
        || ctx.get_constructors("Bool").len() != 2
    {
        return false;
    }
    ["false", "true"].into_iter().all(|name| {
        ctx.constructor_inductive(name) == Some("Bool")
            && ctx.get_global(name) == Some(&Term::Global("Bool".into()))
            && ctx.poly_declaration(name).is_none()
            && ctx.get_definition_body(name).is_none()
    })
}

/// Whether op may be decided on two Nat literals in this context: the shared
/// structural gate. The named, de Bruijn and two evaluation engines keep their
/// own computations; all must refuse an override whose dependencies changed.
pub(crate) fn accelerates_nat_order(ctx: &Context, op: &str) -> bool {
    if !has_nat_order_carriers(ctx) {
        return false;
    }
    match op {
        "natLe" => accelerates_nat_le(ctx),
        "natLt" => {
            ctx.get_global("natLt").is_some_and(is_nat_nat_bool)
                && ctx
                    .get_definition_body("natLt")
                    .is_some_and(|b| is_successor_shifted_order(b, "natLe"))
                && accelerates_nat_le(ctx)
        }
        _ => false,
    }
}

/// The non-dependent arrow spine of a type, as bare global names: `A → B → C` is
/// `["A", "B", "C"]`. `None` for anything else, which is already a disagreement with every
/// primitive's signature and so needs no further analysis. Binder NAMES are ignored, because a
/// declaration is free to spell them however it likes; a domain or codomain that is not a bare
/// global cannot be one of these signatures at all.
fn arrow_spine_of_globals(ty: &Term) -> Option<Vec<&str>> {
    let mut spine = Vec::new();
    let mut cur = ty;
    while let Term::Pi {
        param_type,
        body_type,
        ..
    } = cur
    {
        let Term::Global(name) = param_type.as_ref() else {
            return None;
        };
        spine.push(name.as_str());
        cur = body_type;
    }
    let Term::Global(name) = cur else {
        return None;
    };
    spine.push(name);
    Some(spine)
}

/// The type every computation rule below promises. ONE row per rule, because a rule without one
/// is a rule nothing holds to a type — `R_reduction_extra`'s
/// `every_primitive_the_reducer_computes_has_a_signature_it_is_held_to` fails if a new primitive
/// arrives without its row.
fn primitive_signature(op: &str) -> Option<&'static [&'static str]> {
    Some(match op {
        "add" | "sub" | "mul" | "div" | "mod" => &["Int", "Int", "Int"],
        "le" | "lt" | "ge" | "gt" => &["Int", "Int", "Bool"],
        "Text_append" => &["Text", "Text", "Text"],
        "Text_beq" | "Text_isPrefixOf" => &["Text", "Text", "Bool"],
        "Text_length" => &["Text", "Nat"],
        // The NATIVE-REDUCTION hook. It computes no literal, so `binary_prim`/`unary_prim` never
        // match it and it can never be fired BY this table — its row is here so the hook can ask
        // `declaration_agrees_with_the_rule` the same question every other rule asks. Lean's own
        // `Lean.reduceBool` is `Bool → Bool`.
        "reduceBool" => &["Bool", "Bool"],
        _ => return None,
    })
}

/// Whether a computation rule of this kernel fires on the name `op` itself — a registered primitive
/// operation, the native-reduction hook or a reflection rule — so that an application of `op` may
/// compute although `op` has no body.
pub(crate) fn computes_by_name(op: &str) -> bool {
    primitive_signature(op).is_some() || reflection_signature(op).is_some()
}

/// Whether `name.{levels}` is a well-formed reference with no body to unfold (an axiom, an opaque
/// constant, an inductive type or a constructor).
pub(crate) fn const_is_value(ctx: &Context, name: &str, levels: &[crate::term::Universe]) -> bool {
    matches!(const_step(ctx, name, levels), ConstStep::Value)
}

/// Whether the context's declaration of `op` — if it has one — AGREES with the type the
/// computation rule's answer inhabits.
///
/// SUBJECT REDUCTION. A primitive rule fires on a NAME and a pair of literal kinds, and nothing
/// used to make it look at the declared TYPE. So a context declaring
/// `Text_append : Text → Text → Nat` got a kernel where `infer_type` answered `Nat` and
/// `normalize` produced a `Text` literal — a term and its reduct with different types, which is
/// the one thing a conversion check may not have, since every use of a reduct in place of its
/// term assumes exactly that they are interchangeable. It is the same typing-versus-reduction
/// divergence the codebase calls conformance defect 3, reached through the primitive table
/// instead of through `get_global`'s resolution order.
///
/// A name the context does not declare at all is left alone: the application cannot be typed in
/// that context either way, so there is no verdict to disagree with, and refusing here would
/// take away the only route a `Text` operation currently has before the prelude declares one.
pub(crate) fn declaration_agrees_with_the_rule(ctx: &Context, op: &str) -> bool {
    let Some(signature) = primitive_signature(op) else {
        return false;
    };
    match ctx.get_global(op) {
        None => true,
        Some(declared) => arrow_spine_of_globals(declared).is_some_and(|s| s == signature),
    }
}

#[derive(Clone, Copy)]
enum ReflectionType {
    Syntax,
    Derivation,
    Int,
    Nat,
    Tactic,
    TacticList,
}

/// Ground type schemas for the reflected computation rules, including higher-order tactics.
fn reflection_signature(op: &str) -> Option<&'static [ReflectionType]> {
    use ReflectionType::{
        Derivation as D, Int as I, Nat as N, Syntax as S, Tactic as T, TacticList as L,
    };
    Some(match op {
        "syn_size" | "syn_max_var" => &[S, I],
        "syn_quote" | "syn_diag" | "syn_step" => &[S, S],
        "syn_beta" | "induction_base_goal" => &[S, S, S],
        "syn_eval" => &[I, S, S],
        "syn_lift" => &[I, I, S, S],
        "syn_subst" => &[S, I, S, S],
        "concludes" => &[D, S],
        "try_refl" | "try_compute" | "try_ring" | "try_lia" | "try_cc" | "try_simp"
        | "try_omega" | "try_auto" | "try_decide" | "try_assumption" | "try_split" | "try_left"
        | "try_right" | "try_bitblast" | "try_tabulate" | "try_hw_auto" | "try_inversion"
        | "tact_fail" => &[S, D],
        "induction_num_cases" => &[S, N],
        "try_cong" => &[S, D, D],
        "try_rewrite" | "try_rewrite_rev" => &[D, S, D],
        "try_script" => &[S, S, D],
        "tact_try" | "tact_repeat" | "tact_solve" => &[T, S, D],
        "tact_first" => &[L, S, D],
        "tact_orelse" | "tact_then" => &[T, T, S, D],
        "induction_step_goal" => &[S, S, N, S],
        "try_induction" | "try_destruct" => &[S, S, D, D],
        "try_apply" => &[S, D, S, D],
        _ => return None,
    })
}

fn matches_reflection_type(ty: &Term, shape: ReflectionType) -> bool {
    use ReflectionType::*;
    match shape {
        Tactic => matches_reflection_signature(ty, &[Syntax, Derivation]),
        TacticList => matches!(ty, Term::App(head, elem)
            if matches!(head.as_ref(), Term::Global(n) if n == "TList")
                && matches_reflection_type(elem, Tactic)),
        _ => {
            let expected = match shape {
                Syntax => "Syntax",
                Derivation => "Derivation",
                Int => "Int",
                Nat => "Nat",
                _ => unreachable!(),
            };
            matches!(ty, Term::Global(n) if n == expected)
        }
    }
}

fn matches_reflection_signature(mut ty: &Term, signature: &[ReflectionType]) -> bool {
    for shape in &signature[..signature.len() - 1] {
        let Term::Pi {
            param_type,
            body_type,
            ..
        } = ty
        else {
            return false;
        };
        if !matches_reflection_type(param_type, *shape) {
            return false;
        }
        ty = body_type;
    }
    matches_reflection_type(ty, signature[signature.len() - 1])
}

/// Both kernels gate the same declared primitive interface; their computations remain separate.
pub(crate) fn reflection_declaration_agrees(ctx: &Context, op: &str) -> bool {
    let Some(signature) = reflection_signature(op) else {
        return false;
    };
    ctx.get_global(op)
        .is_none_or(|ty| matches_reflection_signature(ty, signature))
}

/// Try to reduce a primitive operation.
///
/// Returns `Some(result)` for a fully applied primitive like `(add 3 4)` or `(Text_length "ab")`.
///
/// A primitive rule fires on a NAME, so it must never outrank a DEFINITION of that name: if
/// the context binds `op` to a body, the kernel has to compute what the body says, or it
/// disagrees with the very definition it type-checked the application against — a silent wrong
/// answer with no diagnostic anywhere. Bodyless primitives retain their declared
/// computation rules; checked definitions require attestation of the exact body
/// and its dependencies before literal arithmetic can replace delta reduction.
///
/// An ACCELERATION is the one thing allowed through that gate, because it is checked against
/// the body it is standing in for rather than merely named after it — see [`nat_order_value`].
/// It is δ wearing a faster coat, so it asks the transparency door the same question δ asks:
/// an `@[irreducible]` constant stays folded here too.
fn try_primitive_reduce(
    ctx: &Context,
    func: &Term,
    arg: &Term,
    tr: Transparency<'_>,
) -> Option<Term> {
    let owned_y;
    let yl = if let Term::Lit(literal) = arg {
        literal
    } else {
        owned_y = crate::int_inductive::constructor_literal(ctx, arg)?;
        &owned_y
    };
    match func {
        // Unary: `op x`
        Term::Global(op) => {
            let result = unary_prim(op, yl)?;
            if ctx.get_definition_body(op).is_some() || !declaration_agrees_with_the_rule(ctx, op) {
                return None;
            }
            Some(result)
        }
        // Binary: `(op x) y`
        Term::App(op_term, x) => {
            let Term::Global(op) = op_term.as_ref() else {
                return None;
            };
            let owned_x;
            let xl = if let Term::Lit(literal) = x.as_ref() {
                literal
            } else {
                owned_x = crate::int_inductive::constructor_literal(ctx, x)?;
                &owned_x
            };
            if let Some(decided) = nat_order_value(op, xl, yl) {
                // natLt's delta body calls natLe. An elaborator transparency
                // that keeps that dependency folded must not bypass it either.
                if tr.unfolds(op)
                    && (op != "natLt" || tr.unfolds("natLe"))
                    && accelerates_nat_order(ctx, op)
                {
                    return Some(bool_term(decided));
                }
                return None;
            }
            let result = binary_prim(op, xl, yl)?;
            if ctx.get_definition_body(op).is_some() {
                let dependencies = crate::int_inductive::integer_alias_dependencies(op)?;
                if !dependencies.iter().all(|name| tr.unfolds(name))
                    || !crate::int_inductive::accelerates_integer_alias(ctx, op)
                {
                    return None;
                }
            } else if !declaration_agrees_with_the_rule(ctx, op) {
                return None;
            }
            Some(result)
        }
        _ => None,
    }
}

// -------------------------------------------------------------------------
// Reflection Builtins
// -------------------------------------------------------------------------

/// Try to reduce reflection builtins (syn_size, syn_max_var, syn_lift, syn_subst, syn_beta, syn_step).
///
/// Pattern: (syn_size arg) or (syn_max_var arg) or (syn_step arg) where arg is a Syntax constructor.
/// Pattern: ((syn_beta body) arg) for two-argument builtins.
/// Pattern: (((syn_lift amount) cutoff) term) for three-argument builtins.
/// Pattern: (((syn_subst replacement) index) term) for substitution.
///
/// An operation `reflection` defines runs natively only where its definition is attested
/// (`reflection::attested`), and δ unfolds it everywhere else; the rest are assumptions whose rule
/// fires on their declared signature and never past a definition of the name.
fn try_reflection_reduce(ctx: &Context, func: &Term, arg: &Term) -> Option<Term> {
    let mut head = func;
    while let Term::App(f, _) = head {
        head = f;
    }
    let Term::Global(name) = head else {
        return None;
    };
    let gate = if crate::reflection::DEFINED.contains(&name.as_str()) {
        crate::reflection::attested(ctx, name)
    } else {
        !ctx.is_definition(name) && reflection_declaration_agrees(ctx, name)
    };
    if !gate {
        return None;
    }
    // First, check for single-argument builtins
    if let Term::Global(op_name) = func {
        match op_name.as_str() {
            "syn_size" => {
                // Normalize the argument first
                let norm_arg = normalize(ctx, arg);
                return try_syn_size_reduce(ctx, &norm_arg);
            }
            "syn_max_var" => {
                // Normalize the argument first
                let norm_arg = normalize(ctx, arg);
                return try_syn_max_var_reduce(ctx, &norm_arg, 0);
            }
            "syn_step" => {
                // Normalize the argument first
                let norm_arg = normalize(ctx, arg);
                return try_syn_step_reduce(ctx, &norm_arg);
            }
            "syn_quote" => {
                // Normalize the argument first
                let norm_arg = normalize(ctx, arg);
                return try_syn_quote_reduce(ctx, &norm_arg);
            }
            "syn_diag" => {
                // Normalize the argument first
                let norm_arg = normalize(ctx, arg);
                return try_syn_diag_reduce(ctx, &norm_arg);
            }
            "concludes" => {
                // Normalize the argument first
                let norm_arg = normalize(ctx, arg);
                return try_concludes_reduce(ctx, &norm_arg);
            }
            "try_refl" => {
                // Normalize the argument first
                let norm_arg = normalize(ctx, arg);
                return try_try_refl_reduce(ctx, &norm_arg);
            }
            "tact_fail" => {
                // tact_fail always returns error derivation
                return Some(make_error_derivation());
            }
            "try_compute" => {
                // try_compute goal = DCompute goal
                // The validation happens in concludes
                let norm_arg = normalize(ctx, arg);
                return Some(Term::App(
                    Box::new(Term::Global("DCompute".to_string())),
                    Box::new(norm_arg),
                ));
            }
            "try_ring" => {
                // Ring tactic: prove polynomial equalities by normalization
                let norm_arg = normalize(ctx, arg);
                return try_try_ring_reduce(ctx, &norm_arg);
            }
            "try_lia" => {
                // LIA tactic: prove linear inequalities by Fourier-Motzkin
                let norm_arg = normalize(ctx, arg);
                return try_try_lia_reduce(ctx, &norm_arg);
            }
            "try_cc" => {
                // CC tactic: prove equalities by congruence closure
                let norm_arg = normalize(ctx, arg);
                return try_try_cc_reduce(ctx, &norm_arg);
            }
            "try_simp" => {
                // Simp tactic: prove equalities by simplification and arithmetic
                let norm_arg = normalize(ctx, arg);
                return try_try_simp_reduce(ctx, &norm_arg);
            }
            "try_omega" => {
                // Omega tactic: prove integer inequalities with proper rounding
                let norm_arg = normalize(ctx, arg);
                return try_try_omega_reduce(ctx, &norm_arg);
            }
            "try_auto" => {
                // Auto tactic: tries all tactics in sequence
                let norm_arg = normalize(ctx, arg);
                return try_try_auto_reduce(ctx, &norm_arg);
            }
            "try_decide" => {
                // Decide tactic: decide a decidable proposition by reflection + computation
                let norm_arg = normalize(ctx, arg);
                return try_try_decide_reduce(ctx, &norm_arg);
            }
            "try_assumption" => {
                // Assumption tactic: close the goal with a hypothesis (after intro)
                let norm_arg = normalize(ctx, arg);
                return try_try_assumption_reduce(ctx, &norm_arg);
            }
            "try_split" => {
                // Split tactic: ∧-introduction (prove both conjuncts)
                let norm_arg = normalize(ctx, arg);
                return try_try_split_reduce(ctx, &norm_arg);
            }
            "try_left" => {
                // Left tactic: ∨-introduction-left (prove the left disjunct)
                let norm_arg = normalize(ctx, arg);
                return try_try_left_reduce(ctx, &norm_arg);
            }
            "try_right" => {
                // Right tactic: ∨-introduction-right (prove the right disjunct)
                let norm_arg = normalize(ctx, arg);
                return try_try_right_reduce(ctx, &norm_arg);
            }
            "try_bitblast" => {
                // Bitblast tactic: prove Bit equalities by normalization
                let norm_arg = normalize(ctx, arg);
                return try_try_bitblast_reduce(ctx, &norm_arg);
            }
            "try_tabulate" => {
                // Tabulate tactic: prove universal Bit goals by exhaustion
                let norm_arg = normalize(ctx, arg);
                return try_try_tabulate_reduce(ctx, &norm_arg);
            }
            "try_hw_auto" => {
                // Hardware auto: tries bitblast, then tabulate, then auto
                let norm_arg = normalize(ctx, arg);
                return try_try_hw_auto_reduce(ctx, &norm_arg);
            }
            "try_inversion" => {
                // Inversion tactic: derives False if no constructor can match
                let norm_arg = normalize(ctx, arg);
                return try_try_inversion_reduce(ctx, &norm_arg);
            }
            "induction_num_cases" => {
                // Returns number of constructors for an inductive type
                let norm_arg = normalize(ctx, arg);
                return try_induction_num_cases_reduce(ctx, &norm_arg);
            }
            _ => {}
        }
    }

    // For multi-argument builtins like syn_lift and syn_subst,
    // we need to check if func is a partial application

    // syn_lift amount cutoff term
    // Structure: (((syn_lift amount) cutoff) term) = App(App(App(syn_lift, amount), cutoff), term)
    // When reduced: func = App(App(syn_lift, amount), cutoff), arg = term
    if let Term::App(partial2, cutoff) = func {
        if let Term::App(partial1, amount) = partial2.as_ref() {
            if let Term::Global(op_name) = partial1.as_ref() {
                if op_name == "syn_lift" {
                    if let (Term::Lit(Literal::Int(amt)), Term::Lit(Literal::Int(cut))) =
                        (amount.as_ref(), cutoff.as_ref())
                    {
                        let norm_term = normalize(ctx, arg);
                        return try_syn_lift_reduce(ctx, *amt, *cut, &norm_term);
                    }
                }
            }
        }
    }

    // syn_subst replacement index term
    // Structure: (((syn_subst replacement) index) term)
    if let Term::App(partial2, index) = func {
        if let Term::App(partial1, replacement) = partial2.as_ref() {
            if let Term::Global(op_name) = partial1.as_ref() {
                if op_name == "syn_subst" {
                    if let Term::Lit(Literal::Int(idx)) = index.as_ref() {
                        let norm_replacement = normalize(ctx, replacement);
                        let norm_term = normalize(ctx, arg);
                        return try_syn_subst_reduce(ctx, &norm_replacement, *idx, &norm_term);
                    }
                }
            }
        }
    }

    // syn_beta body arg (2 arguments)
    // Structure: ((syn_beta body) arg)
    if let Term::App(partial1, body) = func {
        if let Term::Global(op_name) = partial1.as_ref() {
            if op_name == "syn_beta" {
                let norm_body = normalize(ctx, body);
                let norm_arg = normalize(ctx, arg);
                return try_syn_beta_reduce(ctx, &norm_body, &norm_arg);
            }
        }
    }

    // try_cong context eq_proof (2 arguments)
    // Structure: ((try_cong context) eq_proof)
    // Returns: DCong context eq_proof
    if let Term::App(partial1, context) = func {
        if let Term::Global(op_name) = partial1.as_ref() {
            if op_name == "try_cong" {
                let norm_context = normalize(ctx, context);
                let norm_proof = normalize(ctx, arg);
                return Some(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global("DCong".to_string())),
                        Box::new(norm_context),
                    )),
                    Box::new(norm_proof),
                ));
            }
        }
    }

    // try_rewrite eq_proof goal (2 arguments)
    // Structure: ((try_rewrite eq_proof) goal)
    // Given eq_proof : Eq A x y, rewrites goal by replacing x with y
    if let Term::App(partial1, eq_proof) = func {
        if let Term::Global(op_name) = partial1.as_ref() {
            if op_name == "try_rewrite" {
                let norm_eq_proof = normalize(ctx, eq_proof);
                let norm_goal = normalize(ctx, arg);
                return try_try_rewrite_reduce(ctx, &norm_eq_proof, &norm_goal, false);
            }
            if op_name == "try_rewrite_rev" {
                let norm_eq_proof = normalize(ctx, eq_proof);
                let norm_goal = normalize(ctx, arg);
                return try_try_rewrite_reduce(ctx, &norm_eq_proof, &norm_goal, true);
            }
        }
    }

    // syn_eval fuel term (2 arguments)
    // Structure: ((syn_eval fuel) term)
    if let Term::App(partial1, fuel_term) = func {
        if let Term::Global(op_name) = partial1.as_ref() {
            if op_name == "syn_eval" {
                if let Term::Lit(Literal::Int(fuel)) = fuel_term.as_ref() {
                    let norm_term = normalize(ctx, arg);
                    return try_syn_eval_reduce(ctx, *fuel, &norm_term);
                }
            }
        }
    }

    // induction_base_goal ind_type motive (2 arguments)
    // Structure: ((induction_base_goal ind_type) motive)
    if let Term::App(partial1, ind_type) = func {
        if let Term::Global(op_name) = partial1.as_ref() {
            if op_name == "induction_base_goal" {
                let norm_ind_type = normalize(ctx, ind_type);
                let norm_motive = normalize(ctx, arg);
                return try_induction_base_goal_reduce(ctx, &norm_ind_type, &norm_motive);
            }
        }
    }

    // tact_orelse t1 t2 goal (3 arguments)
    // Structure: (((tact_orelse t1) t2) goal)
    // Here: func = App(App(tact_orelse, t1), t2), arg = goal
    if let Term::App(partial1, t2) = func {
        if let Term::App(combinator, t1) = partial1.as_ref() {
            if let Term::Global(name) = combinator.as_ref() {
                if name == "tact_orelse" {
                    return try_tact_orelse_reduce(ctx, t1, t2, arg);
                }
                // tact_then t1 t2 goal (3 arguments)
                if name == "tact_then" {
                    return try_tact_then_reduce(ctx, t1, t2, arg);
                }
            }
        }
    }

    // tact_try t goal (2 arguments)
    // Structure: ((tact_try t) goal)
    if let Term::App(combinator, t) = func {
        if let Term::Global(name) = combinator.as_ref() {
            if name == "tact_try" {
                return try_tact_try_reduce(ctx, t, arg);
            }
            // tact_repeat t goal (2 arguments)
            if name == "tact_repeat" {
                return try_tact_repeat_reduce(ctx, t, arg);
            }
            // tact_solve t goal (2 arguments)
            if name == "tact_solve" {
                return try_tact_solve_reduce(ctx, t, arg);
            }
            // tact_first tactics goal (2 arguments)
            if name == "tact_first" {
                return try_tact_first_reduce(ctx, t, arg);
            }
        }
    }

    // induction_step_goal ind_type motive ctor_idx (3 arguments)
    // Structure: (((induction_step_goal ind_type) motive) ctor_idx)
    // Returns the goal for the given constructor index
    if let Term::App(partial1, motive) = func {
        if let Term::App(combinator, ind_type) = partial1.as_ref() {
            if let Term::Global(name) = combinator.as_ref() {
                if name == "induction_step_goal" {
                    let norm_ind_type = normalize(ctx, ind_type);
                    let norm_motive = normalize(ctx, motive);
                    let norm_idx = normalize(ctx, arg);
                    return try_induction_step_goal_reduce(
                        ctx,
                        &norm_ind_type,
                        &norm_motive,
                        &norm_idx,
                    );
                }
            }
        }
    }

    // try_induction ind_type motive cases (3 arguments)
    // Structure: (((try_induction ind_type) motive) cases)
    // Returns DElim ind_type motive cases (delegating to existing infrastructure)
    if let Term::App(partial1, motive) = func {
        if let Term::App(combinator, ind_type) = partial1.as_ref() {
            if let Term::Global(name) = combinator.as_ref() {
                if name == "try_induction" {
                    let norm_ind_type = normalize(ctx, ind_type);
                    let norm_motive = normalize(ctx, motive);
                    let norm_cases = normalize(ctx, arg);
                    return try_try_induction_reduce(
                        ctx,
                        &norm_ind_type,
                        &norm_motive,
                        &norm_cases,
                    );
                }
            }
        }
    }

    // try_destruct ind_type motive cases (3 arguments)
    // Structure: (((try_destruct ind_type) motive) cases)
    // Case analysis without induction hypotheses
    if let Term::App(partial1, motive) = func {
        if let Term::App(combinator, ind_type) = partial1.as_ref() {
            if let Term::Global(name) = combinator.as_ref() {
                if name == "try_destruct" {
                    let norm_ind_type = normalize(ctx, ind_type);
                    let norm_motive = normalize(ctx, motive);
                    let norm_cases = normalize(ctx, arg);
                    return try_try_destruct_reduce(&norm_ind_type, &norm_motive, &norm_cases);
                }
            }
        }
    }

    // try_apply hyp_name hyp_proof goal (3 arguments)
    // Structure: (((try_apply hyp_name) hyp_proof) goal)
    // Manual backward chaining
    if let Term::App(partial1, hyp_proof) = func {
        if let Term::App(combinator, hyp_name) = partial1.as_ref() {
            if let Term::Global(name) = combinator.as_ref() {
                if name == "try_apply" {
                    let norm_hyp_name = normalize(ctx, hyp_name);
                    let norm_hyp_proof = normalize(ctx, hyp_proof);
                    let norm_goal = normalize(ctx, arg);
                    return try_try_apply_reduce(ctx, &norm_hyp_name, &norm_hyp_proof, &norm_goal);
                }
            }
        }
    }

    None
}

/// Compute size of a Syntax term.
///
/// SVar n -> 1
/// SGlobal n -> 1
/// SSort u -> 1
/// SApp f x -> 1 + size(f) + size(x)
/// SLam A b -> 1 + size(A) + size(b)
/// SPi A B -> 1 + size(A) + size(B)
fn try_syn_size_reduce(ctx: &Context, term: &Term) -> Option<Term> {
    // Match on the constructor form
    // Unary constructors: (SVar n), (SGlobal n), (SSort u)
    // Binary constructors: ((SApp f) x), ((SLam A) b), ((SPi A) B)

    // Check for unary constructor: (Ctor arg)
    if let Term::App(ctor_term, _inner_arg) = term {
        if let Term::Global(ctor_name) = ctor_term.as_ref() {
            match ctor_name.as_str() {
                "SVar" | "SGlobal" | "SSort" | "SLit" | "SName" => {
                    return Some(Term::Lit(Literal::Int(1)));
                }
                _ => {}
            }
        }

        // Check for binary constructor: ((Ctor a) b)
        if let Term::App(inner, a) = ctor_term.as_ref() {
            if let Term::Global(ctor_name) = inner.as_ref() {
                match ctor_name.as_str() {
                    "SApp" | "SLam" | "SPi" => {
                        // Get sizes of both children
                        let a_size = try_syn_size_reduce(ctx, a)?;
                        let b_size = try_syn_size_reduce(ctx, _inner_arg)?;

                        if let (Term::Lit(Literal::Int(a_n)), Term::Lit(Literal::Int(b_n))) =
                            (a_size, b_size)
                        {
                            return Some(Term::Lit(Literal::Int(1 + a_n + b_n)));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    None
}

/// Compute maximum free variable index in a Syntax term.
///
/// The `depth` parameter tracks how many binders we're under.
/// SVar k -> k - depth (returns -1 if bound, k - depth if free)
/// SGlobal _ -> -1 (no free variables)
/// SSort _ -> -1 (no free variables)
/// SApp f x -> max(max_var(f), max_var(x))
/// SLam A b -> max(max_var(A), max_var(b, depth+1))
/// SPi A B -> max(max_var(A), max_var(B, depth+1))
fn try_syn_max_var_reduce(ctx: &Context, term: &Term, depth: i64) -> Option<Term> {
    // Check for unary constructor: (Ctor arg)
    if let Term::App(ctor_term, inner_arg) = term {
        if let Term::Global(ctor_name) = ctor_term.as_ref() {
            match ctor_name.as_str() {
                "SVar" => {
                    // SVar k -> if k >= depth then k - depth else -1
                    if let Term::Lit(Literal::Int(k)) = inner_arg.as_ref() {
                        if *k >= depth {
                            // Free variable with adjusted index
                            return Some(Term::Lit(Literal::Int(*k - depth)));
                        } else {
                            // Bound variable
                            return Some(Term::Lit(Literal::Int(-1)));
                        }
                    }
                }
                "SGlobal" | "SSort" | "SLit" | "SName" => {
                    // No free variables
                    return Some(Term::Lit(Literal::Int(-1)));
                }
                _ => {}
            }
        }

        // Check for binary constructor: ((Ctor a) b)
        if let Term::App(inner, a) = ctor_term.as_ref() {
            if let Term::Global(ctor_name) = inner.as_ref() {
                match ctor_name.as_str() {
                    "SApp" => {
                        // SApp f x -> max(max_var(f), max_var(x))
                        let a_max = try_syn_max_var_reduce(ctx, a, depth)?;
                        let b_max = try_syn_max_var_reduce(ctx, inner_arg, depth)?;

                        if let (Term::Lit(Literal::Int(a_n)), Term::Lit(Literal::Int(b_n))) =
                            (a_max, b_max)
                        {
                            return Some(Term::Lit(Literal::Int(a_n.max(b_n))));
                        }
                    }
                    "SLam" | "SPi" => {
                        // SLam A b -> max(max_var(A, depth), max_var(b, depth+1))
                        // The body 'b' is under one additional binder
                        let a_max = try_syn_max_var_reduce(ctx, a, depth)?;
                        let b_max = try_syn_max_var_reduce(ctx, inner_arg, depth + 1)?;

                        if let (Term::Lit(Literal::Int(a_n)), Term::Lit(Literal::Int(b_n))) =
                            (a_max, b_max)
                        {
                            return Some(Term::Lit(Literal::Int(a_n.max(b_n))));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    None
}

// -------------------------------------------------------------------------
// Substitution Builtins
// -------------------------------------------------------------------------

/// Lift De Bruijn indices in a Syntax term.
///
/// syn_lift amount cutoff term:
/// - Variables with index < cutoff are bound -> unchanged
/// - Variables with index >= cutoff are free -> add amount
fn try_syn_lift_reduce(ctx: &Context, amount: i64, cutoff: i64, term: &Term) -> Option<Term> {
    // Check for unary constructor: (Ctor arg)
    if let Term::App(ctor_term, inner_arg) = term {
        if let Term::Global(ctor_name) = ctor_term.as_ref() {
            match ctor_name.as_str() {
                "SVar" => {
                    if let Term::Lit(Literal::Int(k)) = inner_arg.as_ref() {
                        if *k >= cutoff {
                            // Free variable, shift
                            return Some(Term::App(
                                Box::new(Term::Global("SVar".to_string())),
                                Box::new(Term::Lit(Literal::Int(k.checked_add(amount)?))),
                            ));
                        } else {
                            // Bound variable, no shift
                            return Some(term.clone());
                        }
                    }
                }
                "SGlobal" | "SSort" | "SLit" | "SName" => {
                    // No free variables
                    return Some(term.clone());
                }
                _ => {}
            }
        }

        // Check for binary constructor: ((Ctor a) b)
        if let Term::App(inner, a) = ctor_term.as_ref() {
            if let Term::Global(ctor_name) = inner.as_ref() {
                match ctor_name.as_str() {
                    "SApp" => {
                        // No binding, same cutoff for both
                        let a_lifted = try_syn_lift_reduce(ctx, amount, cutoff, a)?;
                        let b_lifted = try_syn_lift_reduce(ctx, amount, cutoff, inner_arg)?;
                        return Some(Term::App(
                            Box::new(Term::App(
                                Box::new(Term::Global("SApp".to_string())),
                                Box::new(a_lifted),
                            )),
                            Box::new(b_lifted),
                        ));
                    }
                    "SLam" | "SPi" => {
                        // Binder: param type at current cutoff, body at cutoff+1
                        let param_lifted = try_syn_lift_reduce(ctx, amount, cutoff, a)?;
                        let body_lifted = try_syn_lift_reduce(ctx, amount, cutoff + 1, inner_arg)?;
                        return Some(Term::App(
                            Box::new(Term::App(
                                Box::new(Term::Global(ctor_name.clone())),
                                Box::new(param_lifted),
                            )),
                            Box::new(body_lifted),
                        ));
                    }
                    _ => {}
                }
            }
        }
    }

    None
}

/// Substitute a term for a variable in a Syntax term.
///
/// syn_subst replacement index term:
/// - If term is SVar k and k == index, return replacement
/// - If term is SVar k and k != index, return term unchanged
/// - For binders, increment index and lift replacement
fn try_syn_subst_reduce(
    ctx: &Context,
    replacement: &Term,
    index: i64,
    term: &Term,
) -> Option<Term> {
    // Plain substitution (no de Bruijn decrement): used by recursor / induction
    // handling where the binder is NOT eliminated.
    syn_subst_impl(ctx, replacement, index, term, false)
}

/// Substitute `replacement` for de Bruijn variable `index` in a Syntax term.
///
/// When `decrement` is true this implements the variable part of a BETA step:
/// because the binder is eliminated, surviving free variables `k` with
/// `k > index` are lowered to `k - 1`. With `decrement` false it is a plain
/// substitution that leaves non-matching variables untouched (the semantics the
/// recursor/induction call sites rely on).
fn syn_subst_impl(
    ctx: &Context,
    replacement: &Term,
    index: i64,
    term: &Term,
    decrement: bool,
) -> Option<Term> {
    // Check for unary constructor: (Ctor arg)
    if let Term::App(ctor_term, inner_arg) = term {
        if let Term::Global(ctor_name) = ctor_term.as_ref() {
            match ctor_name.as_str() {
                "SVar" => {
                    if let Term::Lit(Literal::Int(k)) = inner_arg.as_ref() {
                        if *k == index {
                            // Match! Return replacement (already lifted to this depth).
                            return Some(replacement.clone());
                        } else if decrement && *k > index {
                            // Beta: the binder was removed, so a surviving free
                            // variable above it drops by one level.
                            return Some(Term::App(
                                Box::new(Term::Global("SVar".to_string())),
                                Box::new(Term::Lit(Literal::Int(*k - 1))),
                            ));
                        } else {
                            // No match, return unchanged
                            return Some(term.clone());
                        }
                    }
                }
                "SGlobal" | "SSort" | "SLit" | "SName" => {
                    // No variables to substitute
                    return Some(term.clone());
                }
                _ => {}
            }
        }

        // Check for binary constructor: ((Ctor a) b)
        if let Term::App(inner, a) = ctor_term.as_ref() {
            if let Term::Global(ctor_name) = inner.as_ref() {
                match ctor_name.as_str() {
                    "SApp" => {
                        // No binding, same index and replacement
                        let a_subst = syn_subst_impl(ctx, replacement, index, a, decrement)?;
                        let b_subst =
                            syn_subst_impl(ctx, replacement, index, inner_arg, decrement)?;
                        return Some(Term::App(
                            Box::new(Term::App(
                                Box::new(Term::Global("SApp".to_string())),
                                Box::new(a_subst),
                            )),
                            Box::new(b_subst),
                        ));
                    }
                    "SLam" | "SPi" => {
                        // Binder: param type at current index, body at index+1
                        // Lift replacement when going under binder
                        let param_subst = syn_subst_impl(ctx, replacement, index, a, decrement)?;

                        // Lift the replacement by 1 when going under the binder
                        let lifted_replacement = try_syn_lift_reduce(ctx, 1, 0, replacement)?;
                        let body_subst = syn_subst_impl(
                            ctx,
                            &lifted_replacement,
                            index + 1,
                            inner_arg,
                            decrement,
                        )?;

                        return Some(Term::App(
                            Box::new(Term::App(
                                Box::new(Term::Global(ctor_name.clone())),
                                Box::new(param_subst),
                            )),
                            Box::new(body_subst),
                        ));
                    }
                    _ => {}
                }
            }
        }
    }

    None
}

// -------------------------------------------------------------------------
// Computation Builtins
// -------------------------------------------------------------------------

/// Beta reduction: substitute arg for variable 0 in body, with the de Bruijn
/// decrement that removing the binder requires.
///
/// `syn_beta body arg` reflects the kernel's real beta step: substitute `arg`
/// for index 0 in `body`, AND shift every surviving free variable `k > 0` down
/// to `k - 1` (the λ binder is gone, so references that pointed PAST it must
/// drop one level). Without the decrement the reflection would be unfaithful —
/// `reflected_beta_decrements_surviving_free_vars` pins it: `(λT. SVar 1)(SVar 7)
/// → SVar 0`. (`syn_subst` itself is the raw, non-shifting substitution
/// primitive — `syn_beta` is `syn_subst` at index 0 composed with that shift.)
fn try_syn_beta_reduce(ctx: &Context, body: &Term, arg: &Term) -> Option<Term> {
    syn_subst_impl(ctx, arg, 0, body, true)
}

/// Try to reduce arithmetic on Syntax literals.
///
/// Handles: SApp (SApp (SName op) (SLit n)) (SLit m) → SLit result
/// where op is one of: add, sub, mul, div, mod
fn try_syn_arith_reduce(func: &Term, arg: &Term) -> Option<Term> {
    // func should be: SApp (SName op) (SLit n)
    // Pattern: App(App(Global("SApp"), App(Global("SName"), Lit(Text(op)))), App(Global("SLit"), Lit(Int(n))))
    if let Term::App(inner_ctor, n_term) = func {
        if let Term::App(sapp_ctor, op_term) = inner_ctor.as_ref() {
            // Check for SApp constructor
            if let Term::Global(ctor_name) = sapp_ctor.as_ref() {
                if ctor_name != "SApp" {
                    return None;
                }
            } else {
                return None;
            }

            // Extract op from SName op
            let op = if let Term::App(sname_ctor, op_str) = op_term.as_ref() {
                if let Term::Global(name) = sname_ctor.as_ref() {
                    if name == "SName" {
                        if let Term::Lit(Literal::Text(op)) = op_str.as_ref() {
                            op.clone()
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            } else {
                return None;
            };

            // Extract n from SLit n
            let n = if let Term::App(slit_ctor, n_val) = n_term.as_ref() {
                if let Term::Global(name) = slit_ctor.as_ref() {
                    if name == "SLit" {
                        if let Term::Lit(Literal::Int(n)) = n_val.as_ref() {
                            *n
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            } else {
                return None;
            };

            // Extract m from SLit m (the arg)
            let m = if let Term::App(slit_ctor, m_val) = arg {
                if let Term::Global(name) = slit_ctor.as_ref() {
                    if name == "SLit" {
                        if let Term::Lit(Literal::Int(m)) = m_val.as_ref() {
                            *m
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            } else {
                return None;
            };

            // Compute result based on operator. The reflected object language models the
            // kernel's OWN arithmetic, so `div`/`mod` here are the same TOTAL Euclidean pair
            // `try_primitive_reduce` computes: a reflected `div` that truncated while the
            // kernel's rounded the other way would let `try_dcompute_conclude` certify
            // `Eq Int (div (-7) 2) (-3)` — a FALSE equation, accepted by reflection.
            let result = match op.as_str() {
                "add" => n.checked_add(m),
                "sub" => n.checked_sub(m),
                "mul" => n.checked_mul(m),
                "div" => ediv_emod_i64(n, m).map(|(q, _)| q),
                "mod" => ediv_emod_i64(n, m).map(|(_, r)| r),
                _ => None,
            };

            // Build SLit result
            if let Some(r) = result {
                return Some(Term::App(
                    Box::new(Term::Global("SLit".to_string())),
                    Box::new(Term::Lit(Literal::Int(r))),
                ));
            }
        }
    }

    None
}

/// Single-step head reduction.
///
/// Looks for the leftmost-outermost beta redex and reduces it.
/// - If SApp (SLam T body) arg: perform beta reduction
/// - If SApp f x where f is reducible: reduce f
/// - Otherwise: return unchanged (stuck or value)
fn try_syn_step_reduce(ctx: &Context, term: &Term) -> Option<Term> {
    // Check for SApp (binary constructor)
    if let Term::App(ctor_term, arg) = term {
        if let Term::App(inner, func) = ctor_term.as_ref() {
            if let Term::Global(ctor_name) = inner.as_ref() {
                if ctor_name == "SApp" {
                    // We have SApp func arg
                    // First, check for arithmetic primitives: SApp (SApp (SName op) (SLit n)) (SLit m)
                    if let Some(result) = try_syn_arith_reduce(func.as_ref(), arg.as_ref()) {
                        return Some(result);
                    }

                    // Check if func is SLam (beta redex)
                    if let Term::App(lam_inner, body) = func.as_ref() {
                        if let Term::App(lam_ctor, _param_type) = lam_inner.as_ref() {
                            if let Term::Global(lam_name) = lam_ctor.as_ref() {
                                if lam_name == "SLam" {
                                    // Beta redex! (SApp (SLam T body) arg) → syn_beta body arg
                                    return try_syn_beta_reduce(ctx, body.as_ref(), arg.as_ref());
                                }
                            }
                        }
                    }

                    // Delta reduction via kernel normalization:
                    // When the entire SApp tree represents a reducible expression
                    // (e.g., SApp(SApp(SName("bit_and"), SName("B1")), SName("B0"))),
                    // reify to kernel Term, normalize, and convert back to Syntax.
                    // This leverages the kernel's full reduction machinery (delta + iota + beta)
                    // without needing Match support in the Syntax representation.
                    {
                        let full_app = term; // The entire SApp(func, arg) Syntax term
                        if let Some(kernel_term) = syntax_to_term(full_app) {
                            let normalized = normalize(ctx, &kernel_term);
                            if normalized != kernel_term {
                                if let Some(result_syntax) = term_to_syntax(&normalized) {
                                    return Some(result_syntax);
                                }
                            }
                        }
                    }

                    // Not a beta redex or reducible application. Try to step the function.
                    if let Some(stepped_func) = try_syn_step_reduce(ctx, func.as_ref()) {
                        // Check if func actually changed
                        if &stepped_func != func.as_ref() {
                            // func reduced, reconstruct SApp stepped_func arg
                            return Some(Term::App(
                                Box::new(Term::App(
                                    Box::new(Term::Global("SApp".to_string())),
                                    Box::new(stepped_func),
                                )),
                                Box::new(arg.as_ref().clone()),
                            ));
                        }
                    }

                    // func is stuck. Try to step the argument.
                    if let Some(stepped_arg) = try_syn_step_reduce(ctx, arg.as_ref()) {
                        if &stepped_arg != arg.as_ref() {
                            // arg reduced, reconstruct SApp func stepped_arg
                            return Some(Term::App(
                                Box::new(Term::App(
                                    Box::new(Term::Global("SApp".to_string())),
                                    Box::new(func.as_ref().clone()),
                                )),
                                Box::new(stepped_arg),
                            ));
                        }
                    }

                    // Both func and arg are stuck, return original term
                    return Some(term.clone());
                }
            }
        }
    }

    // Not an application - it's a value or stuck term
    // Return unchanged
    Some(term.clone())
}

// -------------------------------------------------------------------------
// Bounded Evaluation
// -------------------------------------------------------------------------

/// Bounded evaluation: reduce for up to N steps.
///
/// syn_eval fuel term:
/// - If fuel <= 0: return term unchanged
/// - Otherwise: step and repeat until normal form or fuel exhausted
fn try_syn_eval_reduce(ctx: &Context, fuel: i64, term: &Term) -> Option<Term> {
    match syn_eval_checked(ctx, fuel, term) {
        // A fuelled evaluator's exhausted result is a legitimate answer for the OBJECT-language
        // `syn_eval fuel t`, because the fuel is written in the term and the caller can see it.
        // It is NOT a legitimate answer for a proof door, which is why the honest channel exists.
        Ok(value) | Err(Some(value)) => Some(value),
        Err(None) => None,
    }
}

/// Bounded evaluation of a reflected term, reporting whether it FINISHED.
///
/// `Ok(v)` — reached a normal form (or a stuck term) with fuel to spare. `Err(Some(v))` — the
/// fuel ran out at `v`, which is as far as it got and NOT the value of the expression.
/// `Err(None)` — a step could not be taken at all.
///
/// Iterative on purpose. The recursive shape this replaces recursed ONCE PER STEP, so the fuel
/// literal in `syn_eval <fuel> t` — a number the user writes in the term — set the native stack
/// depth: a large enough one overflowed the stack and ABORTED THE PROCESS out of `normalize`, a
/// total public entry point. Depth is now bounded by the heap, and the budget can be raised
/// without buying a crash.
fn syn_eval_checked(ctx: &Context, fuel: i64, term: &Term) -> Result<Term, Option<Term>> {
    let mut current = term.clone();
    let mut left = fuel;
    while left > 0 {
        let stepped = match try_syn_step_reduce(ctx, &current) {
            Some(s) => s,
            None => return Err(None),
        };
        if stepped == current {
            return Ok(current);
        }
        current = stepped;
        left -= 1;
    }
    Err(Some(current))
}

// -------------------------------------------------------------------------
// Reification (Quote)
// -------------------------------------------------------------------------

/// Quote a Syntax value: produce Syntax that constructs it.
///
/// syn_quote term:
/// - SVar n → SApp (SName "SVar") (SLit n)
/// - SGlobal n → SApp (SName "SGlobal") (SLit n)
/// - SSort u → SApp (SName "SSort") (quote_univ u)
/// - SApp f x → SApp (SApp (SName "SApp") (quote f)) (quote x)
/// - SLam T b → SApp (SApp (SName "SLam") (quote T)) (quote b)
/// - SPi T B → SApp (SApp (SName "SPi") (quote T)) (quote B)
/// - SLit n → SApp (SName "SLit") (SLit n)
/// - SName s → SName s (self-quoting)
#[allow(dead_code)]
fn try_syn_quote_reduce(ctx: &Context, term: &Term) -> Option<Term> {
    // Helper to build SApp (SName name) arg
    fn sname_app(name: &str, arg: Term) -> Term {
        Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SApp".to_string())),
                Box::new(Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(Term::Lit(Literal::Text(name.to_string()))),
                )),
            )),
            Box::new(arg),
        )
    }

    // Helper to build SLit n
    fn slit(n: i64) -> Term {
        Term::App(
            Box::new(Term::Global("SLit".to_string())),
            Box::new(Term::Lit(Literal::Int(n))),
        )
    }

    // Helper to build SName s
    fn sname(s: &str) -> Term {
        Term::App(
            Box::new(Term::Global("SName".to_string())),
            Box::new(Term::Lit(Literal::Text(s.to_string()))),
        )
    }

    // Helper to build SApp (SApp (SName name) a) b
    fn sname_app2(name: &str, a: Term, b: Term) -> Term {
        Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SApp".to_string())),
                Box::new(sname_app(name, a)),
            )),
            Box::new(b),
        )
    }

    // Match on Syntax constructors
    if let Term::App(ctor_term, inner_arg) = term {
        if let Term::Global(ctor_name) = ctor_term.as_ref() {
            match ctor_name.as_str() {
                "SVar" => {
                    if let Term::Lit(Literal::Int(n)) = inner_arg.as_ref() {
                        return Some(sname_app("SVar", slit(*n)));
                    }
                }
                "SGlobal" => {
                    if let Term::Lit(Literal::Int(n)) = inner_arg.as_ref() {
                        return Some(sname_app("SGlobal", slit(*n)));
                    }
                }
                "SSort" => {
                    // Quote the universe
                    let quoted_univ = quote_univ(inner_arg)?;
                    return Some(sname_app("SSort", quoted_univ));
                }
                "SLit" => {
                    if let Term::Lit(Literal::Int(n)) = inner_arg.as_ref() {
                        return Some(sname_app("SLit", slit(*n)));
                    }
                }
                "SName" => {
                    // SName is self-quoting
                    return Some(term.clone());
                }
                _ => {}
            }
        }

        // Binary constructors: ((Ctor a) b)
        if let Term::App(inner, a) = ctor_term.as_ref() {
            if let Term::Global(ctor_name) = inner.as_ref() {
                match ctor_name.as_str() {
                    "SApp" | "SLam" | "SPi" => {
                        let quoted_a = try_syn_quote_reduce(ctx, a)?;
                        let quoted_b = try_syn_quote_reduce(ctx, inner_arg)?;
                        return Some(sname_app2(ctor_name, quoted_a, quoted_b));
                    }
                    _ => {}
                }
            }
        }
    }

    None
}

/// Quote a Univ value.
///
/// UProp → SName "UProp"
/// UType n → SApp (SName "UType") (SLit n)
fn quote_univ(term: &Term) -> Option<Term> {
    fn sname(s: &str) -> Term {
        Term::App(
            Box::new(Term::Global("SName".to_string())),
            Box::new(Term::Lit(Literal::Text(s.to_string()))),
        )
    }

    fn slit(n: i64) -> Term {
        Term::App(
            Box::new(Term::Global("SLit".to_string())),
            Box::new(Term::Lit(Literal::Int(n))),
        )
    }

    fn sname_app(name: &str, arg: Term) -> Term {
        Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SApp".to_string())),
                Box::new(sname(name)),
            )),
            Box::new(arg),
        )
    }

    if let Term::Global(name) = term {
        if name == "UProp" {
            return Some(sname("UProp"));
        }
    }

    if let Term::App(ctor, arg) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "UType" {
                if let Term::Lit(Literal::Int(n)) = arg.as_ref() {
                    return Some(sname_app("UType", slit(*n)));
                }
            }
        }
    }

    None
}

// -------------------------------------------------------------------------
// Diagonalization (Self-Reference)
// -------------------------------------------------------------------------

/// The diagonal function: syn_diag x = syn_subst (syn_quote x) 0 x
///
/// This takes a term x with free variable 0, quotes x to get its construction
/// code, then substitutes that code for variable 0 in x.
fn try_syn_diag_reduce(ctx: &Context, term: &Term) -> Option<Term> {
    // Step 1: Quote the term
    let quoted = try_syn_quote_reduce(ctx, term)?;

    // Step 2: Substitute the quoted form for variable 0 in the original term
    try_syn_subst_reduce(ctx, &quoted, 0, term)
}

// -------------------------------------------------------------------------
// Inference Rules
// -------------------------------------------------------------------------

/// Extract the conclusion from a derivation.
///
/// concludes d:
/// - DAxiom P → P
/// - DModusPonens d_impl d_ant → extract B from (Implies A B) if A matches antecedent
/// - DUnivIntro d → wrap conclusion in Forall
/// - DUnivElim d term → substitute term into forall body
/// - DRefl T a → Eq T a a
fn try_concludes_reduce(ctx: &Context, deriv: &Term) -> Option<Term> {
    // A proof SCRIPT `try_script names goal` concludes its goal iff evaluating the script proves it.
    // A failed or malformed script concludes the error sentinel, so the soundness gate rejects it.
    if let Some((list, goal)) = extract_try_script(deriv) {
        return match extract_script_names(&list) {
            // The script proved the goal — but the CONCLUSION this door reports is compared
            // against the stated goal by the REPL's soundness gate, so it must be a finished
            // normal form. An abandoned reduction reported as the conclusion would let the gate
            // match two partial terms and accept a theorem nothing certified.
            Some(names) if eval_script_names(ctx, &names, &[], &goal) => {
                match normalize_checked(ctx, &goal) {
                    Ok(normal) => Some(normal),
                    Err(_) => Some(make_sname_error()),
                }
            }
            _ => Some(make_sname_error()),
        };
    }

    // DAxiom P → P
    if let Term::App(ctor_term, p) = deriv {
        if let Term::Global(ctor_name) = ctor_term.as_ref() {
            if ctor_name == "DAxiom" {
                return Some(p.as_ref().clone());
            }
            if ctor_name == "DUnivIntro" {
                // Get conclusion of inner derivation
                let inner_conc = try_concludes_reduce(ctx, p)?;
                // Lift it by 1 and wrap in Forall
                let lifted = try_syn_lift_reduce(ctx, 1, 0, &inner_conc)?;
                return Some(make_forall_syntax(&lifted));
            }
            if ctor_name == "DCompute" {
                // DCompute goal: verify goal is Eq T A B and eval(A) == eval(B)
                return try_dcompute_conclude(ctx, p);
            }
            if ctor_name == "DRingSolve" {
                // DRingSolve goal: verify goal is Eq T A B and polynomials are equal
                return try_dring_solve_conclude(ctx, p);
            }
            if ctor_name == "DLiaSolve" {
                // DLiaSolve goal: verify goal is an inequality and LIA proves it
                return try_dlia_solve_conclude(ctx, p);
            }
            if ctor_name == "DccSolve" {
                // DccSolve goal: verify goal by congruence closure
                return try_dcc_solve_conclude(ctx, p);
            }
            if ctor_name == "DSimpSolve" {
                // DSimpSolve goal: verify goal by simplification
                return try_dsimp_solve_conclude(ctx, p);
            }
            if ctor_name == "DOmegaSolve" {
                // DOmegaSolve goal: verify goal by integer arithmetic
                return try_domega_solve_conclude(ctx, p);
            }
            if ctor_name == "DAutoSolve" {
                // DAutoSolve goal: verify goal by trying all tactics
                return try_dauto_solve_conclude(ctx, p);
            }
            if ctor_name == "DBitblastSolve" {
                // DBitblastSolve goal: verify Eq Bit by normalization
                return try_dbitblast_solve_conclude(ctx, p);
            }
            if ctor_name == "DTabulateSolve" {
                // DTabulateSolve goal: verify universal Bit by exhaustion
                return try_dtabulate_solve_conclude(ctx, p);
            }
            if ctor_name == "DHwAutoSolve" {
                // DHwAutoSolve goal: verify by hw tactic chain
                return try_dhw_auto_solve_conclude(ctx, p);
            }
            if ctor_name == "DInversion" {
                // DInversion hyp_type: verify no constructor can match, return False
                return try_dinversion_conclude(ctx, p);
            }
        }
    }

    // DRewrite eq_proof old_goal new_goal → new_goal (if verified)
    // Pattern: App(App(App(DRewrite, eq_proof), old_goal), new_goal)
    if let Term::App(partial1, new_goal) = deriv {
        if let Term::App(partial2, old_goal) = partial1.as_ref() {
            if let Term::App(ctor_term, eq_proof) = partial2.as_ref() {
                if let Term::Global(ctor_name) = ctor_term.as_ref() {
                    if ctor_name == "DRewrite" {
                        return try_drewrite_conclude(ctx, eq_proof, old_goal, new_goal);
                    }
                }
            }
        }
    }

    // DDestruct ind_type motive cases → Forall ind_type motive (if verified)
    // Pattern: App(App(App(DDestruct, ind_type), motive), cases)
    if let Term::App(partial1, cases) = deriv {
        if let Term::App(partial2, motive) = partial1.as_ref() {
            if let Term::App(ctor_term, ind_type) = partial2.as_ref() {
                if let Term::Global(ctor_name) = ctor_term.as_ref() {
                    if ctor_name == "DDestruct" {
                        return try_ddestruct_conclude(ctx, ind_type, motive, cases);
                    }
                }
            }
        }
    }

    // DApply hyp_name hyp_proof old_goal new_goal → new_goal (if verified)
    // Pattern: App(App(App(App(DApply, hyp_name), hyp_proof), old_goal), new_goal)
    if let Term::App(partial1, new_goal) = deriv {
        if let Term::App(partial2, old_goal) = partial1.as_ref() {
            if let Term::App(partial3, hyp_proof) = partial2.as_ref() {
                if let Term::App(ctor_term, _) = partial3.as_ref() {
                    if let Term::Global(ctor_name) = ctor_term.as_ref() {
                        if ctor_name == "DApply" {
                            return try_dapply_conclude(ctx, hyp_proof, old_goal, new_goal);
                        }
                    }
                }
            }
        }
    }

    // DRefl T a → Eq T a a
    if let Term::App(partial, a) = deriv {
        if let Term::App(ctor_term, t) = partial.as_ref() {
            if let Term::Global(ctor_name) = ctor_term.as_ref() {
                if ctor_name == "DRefl" {
                    return Some(make_eq_syntax(t.as_ref(), a.as_ref()));
                }
                if ctor_name == "DCong" {
                    // DCong context eq_proof → Eq T (f a) (f b)
                    // where context = SLam T body, eq_proof proves Eq T a b
                    return try_dcong_conclude(ctx, t, a);
                }
            }
        }
    }

    // DModusPonens d_impl d_ant
    if let Term::App(partial, d_ant) = deriv {
        if let Term::App(ctor_term, d_impl) = partial.as_ref() {
            if let Term::Global(ctor_name) = ctor_term.as_ref() {
                if ctor_name == "DModusPonens" {
                    // Get conclusions of both derivations
                    let impl_conc = try_concludes_reduce(ctx, d_impl)?;
                    let ant_conc = try_concludes_reduce(ctx, d_ant)?;

                    // Check if impl_conc = SApp (SApp (SName "Implies") A) B
                    if let Some((a, b)) = extract_implication(&impl_conc) {
                        // Check if ant_conc equals A
                        if syntax_equal(&ant_conc, &a) {
                            return Some(b);
                        }
                    }
                    // Invalid modus ponens
                    return Some(make_sname_error());
                }
                if ctor_name == "DUnivElim" {
                    // d_impl is the derivation, d_ant is the term to substitute
                    let conc = try_concludes_reduce(ctx, d_impl)?;
                    if let Some(body) = extract_forall_body(&conc) {
                        // Substitute d_ant for var 0 in body
                        return try_syn_subst_reduce(ctx, d_ant, 0, &body);
                    }
                    // Invalid universal elimination
                    return Some(make_sname_error());
                }
            }
        }
    }

    // DInduction motive base step → Forall Nat motive (if verified)
    // Pattern: App(App(App(DInduction, motive), base), step)
    if let Term::App(partial1, step) = deriv {
        if let Term::App(partial2, base) = partial1.as_ref() {
            if let Term::App(ctor_term, motive) = partial2.as_ref() {
                if let Term::Global(ctor_name) = ctor_term.as_ref() {
                    if ctor_name == "DInduction" {
                        return try_dinduction_reduce(ctx, motive, base, step);
                    }
                }
            }
        }
    }

    // DElim ind_type motive cases → Forall ind_type motive (if verified)
    // Pattern: App(App(App(DElim, ind_type), motive), cases)
    if let Term::App(partial1, cases) = deriv {
        if let Term::App(partial2, motive) = partial1.as_ref() {
            if let Term::App(ctor_term, ind_type) = partial2.as_ref() {
                if let Term::Global(ctor_name) = ctor_term.as_ref() {
                    if ctor_name == "DElim" {
                        return try_delim_conclude(ctx, ind_type, motive, cases);
                    }
                }
            }
        }
    }

    None
}

/// **THE ONE READER OF AN IMPLICATION**, whatever it happens to be spelled as.
///
/// The reflected surface has THREE spellings of `A → B`, and until this function they never met:
///
/// * `SApp (SApp (SName "implies") A) B` — lower case, what `intro`, `decide`, `assumption` and
///   the simp/cc goal readers build and match;
/// * `SApp (SApp (SName "Implies") A) B` — capitalised, what `make_implies_syntax` builds, so
///   every induction STEP case was stated in a dialect no script step could read;
/// * a NON-DEPENDENT `SPi A B` — what the literate `Statement:` surface produces, because the
///   infix `implies` desugars to a `Pi { param: "_" }` and `term_to_syntax` reflects a `Pi`
///   faithfully as an `SPi`.
///
/// One spelling per producer and a different one per consumer is a seam, not a feature, and it
/// was observable: `## Theorem: P1 Statement: (implies True True). Proof: auto.` certified while
/// `## Theorem: P2 Statement: True implies True. Proof: auto.` — the SAME proposition, written
/// the way the documentation teaches — did not, and `intro. assumption.` failed on the infix one
/// too.
///
/// A DEPENDENT `SPi` is a QUANTIFIER and is refused here: its body mentions the bound variable,
/// so discharging the antecedent does not hand you the conclusion. The test is conservative on
/// purpose — a body mentioning ANY de Bruijn index is declined rather than renumbered, because
/// re-indexing it wrongly would hand a tactic the wrong proposition and it would prove THAT.
fn as_implication(term: &Term) -> Option<(Term, Term)> {
    if let Some(pair) = extract_sapp2(term, "implies") {
        return Some(pair);
    }
    if let Some(pair) = extract_sapp2(term, "Implies") {
        return Some(pair);
    }
    let (antecedent, consequent) = extract_spi(term)?;
    if syntax_mentions_svar(&consequent) {
        return None;
    }
    Some((antecedent, consequent))
}

/// Whether a reflected term mentions any de Bruijn index — `SVar n` for any `n`.
fn syntax_mentions_svar(term: &Term) -> bool {
    match term {
        Term::App(f, x) => {
            if let Term::Global(ctor) = f.as_ref() {
                if ctor == "SVar" {
                    return true;
                }
            }
            syntax_mentions_svar(f) || syntax_mentions_svar(x)
        }
        _ => false,
    }
}

/// Extract (A, B) from an implication, in any of its three spellings — see [`as_implication`],
/// which this is now a name for. Kept because `DModusPonens` and the congruence-closure reader
/// both call it by this name.
fn extract_implication(term: &Term) -> Option<(Term, Term)> {
    as_implication(term)
}

/// Extract all hypotheses from a chain of nested implications.
///
/// For `A -> B -> C`, returns `([A, B], C)`.
/// For non-implications, returns `([], term)`.
fn extract_implications(term: &Term) -> Option<(Vec<Term>, Term)> {
    let mut hyps = Vec::new();
    let mut current = term.clone();

    while let Some((hyp, rest)) = extract_implication(&current) {
        hyps.push(hyp);
        current = rest;
    }

    if hyps.is_empty() {
        None
    } else {
        Some((hyps, current))
    }
}

/// Extract body from SApp (SApp (SName "Forall") T) (SLam T body)
///
/// In kernel representation:
/// Extract body from Forall syntax (two forms supported):
/// Form 1: SApp (SName "Forall") (SLam T body)
/// Form 2: SApp (SApp (SName "Forall") T) (SLam T body)
fn extract_forall_body(term: &Term) -> Option<Term> {
    // term = App(App(SApp, X), lam)
    if let Term::App(outer, lam) = term {
        if let Term::App(sapp_outer, x) = outer.as_ref() {
            if let Term::Global(ctor) = sapp_outer.as_ref() {
                if ctor == "SApp" {
                    // Form 1: X = App(SName, "Forall")
                    if let Term::App(sname, text) = x.as_ref() {
                        if let Term::Global(sname_ctor) = sname.as_ref() {
                            if sname_ctor == "SName" {
                                if let Term::Lit(Literal::Text(s)) = text.as_ref() {
                                    if s == "Forall" {
                                        // Extract body from SLam
                                        return extract_slam_body(lam);
                                    }
                                }
                            }
                        }
                    }

                    // Form 2: X = App(App(SApp, App(SName, "Forall")), T)
                    if let Term::App(inner, _t) = x.as_ref() {
                        if let Term::App(sapp_inner, sname_forall) = inner.as_ref() {
                            if let Term::Global(ctor2) = sapp_inner.as_ref() {
                                if ctor2 == "SApp" {
                                    if let Term::App(sname, text) = sname_forall.as_ref() {
                                        if let Term::Global(sname_ctor) = sname.as_ref() {
                                            if sname_ctor == "SName" {
                                                if let Term::Lit(Literal::Text(s)) = text.as_ref() {
                                                    if s == "Forall" {
                                                        return extract_slam_body(lam);
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
        }
    }
    None
}

/// Extract body from ((SLam T) body)
fn extract_slam_body(term: &Term) -> Option<Term> {
    if let Term::App(inner, body) = term {
        if let Term::App(slam, _t) = inner.as_ref() {
            if let Term::Global(name) = slam.as_ref() {
                if name == "SLam" {
                    return Some(body.as_ref().clone());
                }
            }
        }
    }
    None
}

/// Equality of reflected `Syntax` propositions: structural, except that the two spellings of
/// `Forall` are one proposition.
///
/// The encoding spells `Forall` two ways, and [`extract_forall_body`] reads both: Form 1,
/// `Forall (SLam T b)`, and Form 2, `Forall T (SLam T b)`, whose domain repeats the one the λ
/// carries. Every certifying door asks "is this the proposition that was owed?" through this one
/// comparison, and owed formulas are built in Form 2 ([`make_forall_syntax_with_type`]), so a
/// proof spelled in Form 1 compares equal. It is not a relaxation: a domain that differs, in
/// either spelling, still compares unequal.
///
/// A reflected `Syntax` value is a tree of constructor applications, so the walk descends through
/// `App` and compares every other node structurally. It allocates nothing, and the `a == b` fast
/// path keeps the common case at one structural comparison.
fn syntax_equal(a: &Term, b: &Term) -> bool {
    a == b || syntax_equal_modulo_forall_spelling(a, b)
}

fn syntax_equal_modulo_forall_spelling(a: &Term, b: &Term) -> bool {
    match (reflected_forall(a), reflected_forall(b)) {
        (Some((domain_a, lam_a)), Some((domain_b, lam_b))) => {
            syntax_equal_modulo_forall_spelling(domain_a, domain_b)
                && syntax_equal_modulo_forall_spelling(lam_a, lam_b)
        }
        (None, None) => match (a, b) {
            (Term::App(f_a, x_a), Term::App(f_b, x_b)) => {
                syntax_equal_modulo_forall_spelling(f_a, f_b)
                    && syntax_equal_modulo_forall_spelling(x_a, x_b)
            }
            _ => a == b,
        },
        _ => false,
    }
}

/// The domain and λ of a reflected `Forall` in either spelling: Form 2 states its domain, Form 1
/// reads it off its `SLam`. A Form-1 `Forall` applied to anything but an `SLam` has no domain to
/// read and is not recognised.
fn reflected_forall(term: &Term) -> Option<(&Term, &Term)> {
    fn sapp_parts(t: &Term) -> Option<(&Term, &Term)> {
        let Term::App(outer, arg) = t else {
            return None;
        };
        let Term::App(ctor, func) = outer.as_ref() else {
            return None;
        };
        match ctor.as_ref() {
            Term::Global(name) if name == "SApp" => Some((func.as_ref(), arg.as_ref())),
            _ => None,
        }
    }
    fn slam_domain(t: &Term) -> Option<&Term> {
        let Term::App(outer, _body) = t else {
            return None;
        };
        let Term::App(ctor, domain) = outer.as_ref() else {
            return None;
        };
        match ctor.as_ref() {
            Term::Global(name) if name == "SLam" => Some(domain.as_ref()),
            _ => None,
        }
    }

    let (head, lam) = sapp_parts(term)?;
    if is_sname_with_value(head, "Forall") {
        return Some((slam_domain(lam)?, lam));
    }
    let (forall, domain) = sapp_parts(head)?;
    is_sname_with_value(forall, "Forall").then_some((domain, lam))
}

/// The goal a `*_conclude` door is about to CERTIFY, normalised through the HONEST face.
///
/// `None` means the goal's own reduction did not finish, and the door must refuse.
///
/// Every certifying door began `let norm_goal = normalize(ctx, goal);` — the BEST-EFFORT face,
/// which hands back HOW FAR IT GOT, applied to the very term whose truth is about to be
/// certified. So a goal whose normalisation was abandoned reached the checker as a PARTIAL
/// goal, was decided about, and was then returned as the concluded statement: the door
/// certified something other than what it was asked about, with nothing anywhere saying so.
/// That is DEFECT 9 at the door layer, and `simp_compute_check` and `try_dcompute_conclude`
/// already make exactly this argument about the two SIDES of the equality while taking the
/// goal itself from the unchecked face.
///
/// ONE helper rather than sixteen open-coded `normalize_checked` calls, because a door added
/// later copies the line above it: the pit of success is that the only thing there is to copy
/// is the honest one.
fn goal_to_certify(ctx: &Context, goal: &Term) -> Option<Term> {
    normalize_checked(ctx, goal).ok()
}

/// Build SName "Error"
fn make_sname_error() -> Term {
    Term::App(
        Box::new(Term::Global("SName".to_string())),
        Box::new(Term::Lit(Literal::Text("Error".to_string()))),
    )
}

/// Build SName from a string
fn make_sname(s: &str) -> Term {
    Term::App(
        Box::new(Term::Global("SName".to_string())),
        Box::new(Term::Lit(Literal::Text(s.to_string()))),
    )
}

/// Build SLit from an integer
fn make_slit(n: i64) -> Term {
    Term::App(
        Box::new(Term::Global("SLit".to_string())),
        Box::new(Term::Lit(Literal::Int(n))),
    )
}

/// Build SApp f x
fn make_sapp(f: Term, x: Term) -> Term {
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(f),
        )),
        Box::new(x),
    )
}

/// Build SPi A B
fn make_spi(a: Term, b: Term) -> Term {
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SPi".to_string())),
            Box::new(a),
        )),
        Box::new(b),
    )
}

/// Build SLam A b
fn make_slam(a: Term, b: Term) -> Term {
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SLam".to_string())),
            Box::new(a),
        )),
        Box::new(b),
    )
}

/// Build SSort u
fn make_ssort(u: Term) -> Term {
    Term::App(Box::new(Term::Global("SSort".to_string())), Box::new(u))
}

/// Build SVar n
fn make_svar(n: i64) -> Term {
    Term::App(
        Box::new(Term::Global("SVar".to_string())),
        Box::new(Term::Lit(Literal::Int(n))),
    )
}

/// Convert a kernel Term to Syntax encoding (S* form).
///
/// This converts semantic Terms to their syntactic representation:
/// - Global(n) → SName n
/// - Var(n) → SName n (for named variables)
/// - App(f, x) → SApp (convert f) (convert x)
/// - Pi { param, param_type, body_type } → SPi (convert param_type) (convert body_type)
/// - Lambda { param, param_type, body } → SLam (convert param_type) (convert body)
/// - Sort(Type n) → SSort (UType n)
/// - Sort(Prop) → SSort UProp
/// - Lit(Int n) → SLit n
/// - Lit(Text s) → SName s
fn term_to_syntax(term: &Term) -> Option<Term> {
    match term {
        Term::Global(name) => Some(make_sname(name)),

        Term::Const { name, .. } => Some(make_sname(name)),

        Term::Var(name) => {
            // Named variables become SName in syntax representation
            Some(make_sname(name))
        }

        Term::App(f, x) => {
            let sf = term_to_syntax(f)?;
            let sx = term_to_syntax(x)?;
            Some(make_sapp(sf, sx))
        }

        Term::Pi { param_type, body_type, .. } => {
            let sp = term_to_syntax(param_type)?;
            let sb = term_to_syntax(body_type)?;
            Some(make_spi(sp, sb))
        }

        Term::Lambda { param_type, body, .. } => {
            let sp = term_to_syntax(param_type)?;
            let sb = term_to_syntax(body)?;
            Some(make_slam(sp, sb))
        }

        Term::Sort(univ) => Some(make_ssort(univ_to_syntax(univ))),

        Term::Lit(Literal::Int(n)) => Some(make_slit(*n)),

        Term::Lit(Literal::Text(s)) => Some(make_sname(s)),

        // Float, Duration, Date, and Moment literals - skip for now as they're rarely used in proofs
        Term::Lit(Literal::Float(_))
        | Term::Lit(Literal::Duration(_))
        | Term::Lit(Literal::Date(_))
        | Term::Lit(Literal::Moment(_))
        // Neither a BigInt nor a Nat literal fits the syntax layer's machine-int `SLit`.
        | Term::Lit(Literal::BigInt(_))
        | Term::Lit(Literal::Nat(_)) => None,

        // Match expressions, Fix, Let, and Hole are complex - skip for now
        Term::Match { .. } | Term::Fix { .. } | Term::MutualFix { .. } | Term::Let { .. }
        | Term::Hole => None,

        // The syntax layer encodes kernel meaning; an annotation is not part of it.
        Term::Annotated { term, .. } => term_to_syntax(term),
    }
}

/// Build DHint derivation that references a hint by name.
///
/// Uses DAxiom (SName "hint_name") to indicate the proof uses this hint.
fn make_hint_derivation(goal: &Term) -> Term {
    // DAutoSolve goal: auto succeeded; which hint was used shows when tracing.
    Term::App(
        Box::new(Term::Global("DAutoSolve".to_string())),
        Box::new(goal.clone()),
    )
}

/// Try to apply a hint to prove a goal.
///
/// Returns Some(derivation) if the hint's type matches the goal,
/// otherwise returns None.
fn try_apply_hint(ctx: &Context, hint_type: &Term, goal: &Term) -> Option<Term> {
    // Convert hint type to syntax form
    let hint_syntax = term_to_syntax(hint_type)?;

    // Normalize both for comparison
    let norm_hint = normalize(ctx, &hint_syntax);
    let norm_goal = normalize(ctx, goal);

    // Direct match: hint type equals goal
    if syntax_equal(&norm_hint, &norm_goal) {
        return Some(make_hint_derivation(goal));
    }

    // Try backward chaining for implications: if hint is P → Q and goal is Q,
    // try to prove P using auto and then apply the hint
    if let Term::App(outer, q) = &hint_syntax {
        if let Term::App(pi_ctor, _p) = outer.as_ref() {
            if let Term::Global(name) = pi_ctor.as_ref() {
                if name == "SPi" {
                    // hint_type is SPi P Q, goal might be Q
                    let norm_q = normalize(ctx, q);
                    if syntax_equal(&norm_q, &norm_goal) {
                        // Need to prove P first, then apply hint
                        // This would require recursive auto call - skip for now
                        // to avoid infinite recursion
                    }
                }
            }
        }
    }

    None
}

/// Build Forall Type0 (SLam Type0 body)
fn make_forall_syntax(body: &Term) -> Term {
    let type0 = Term::App(
        Box::new(Term::Global("SSort".to_string())),
        Box::new(Term::App(
            Box::new(Term::Global("UType".to_string())),
            Box::new(Term::Lit(Literal::Int(0))),
        )),
    );

    // SLam Type0 body
    let slam = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SLam".to_string())),
            Box::new(type0.clone()),
        )),
        Box::new(body.clone()),
    );

    // SApp (SApp (SName "Forall") Type0) slam
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("SApp".to_string())),
                    Box::new(Term::App(
                        Box::new(Term::Global("SName".to_string())),
                        Box::new(Term::Lit(Literal::Text("Forall".to_string()))),
                    )),
                )),
                Box::new(type0),
            )),
        )),
        Box::new(slam),
    )
}

// -------------------------------------------------------------------------
// Core Tactics
// -------------------------------------------------------------------------

/// Build SApp (SApp (SApp (SName "Eq") type_s) term) term
///
/// Constructs the Syntax representation of (Eq type_s term term)
fn make_eq_syntax(type_s: &Term, term: &Term) -> Term {
    let eq_name = Term::App(
        Box::new(Term::Global("SName".to_string())),
        Box::new(Term::Lit(Literal::Text("Eq".to_string()))),
    );

    // SApp (SName "Eq") type_s
    let app1 = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(eq_name),
        )),
        Box::new(type_s.clone()),
    );

    // SApp (SApp (SName "Eq") type_s) term
    let app2 = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(app1),
        )),
        Box::new(term.clone()),
    );

    // SApp (SApp (SApp (SName "Eq") type_s) term) term
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(app2),
        )),
        Box::new(term.clone()),
    )
}

/// Extract (T, a, b) from SApp (SApp (SApp (SName "Eq") T) a) b
///
/// Pattern matches against the Syntax representation of (Eq T a b)
fn extract_eq(term: &Term) -> Option<(Term, Term, Term)> {
    // term = App(App(SApp, X), b)
    if let Term::App(outer, b) = term {
        if let Term::App(sapp_outer, x) = outer.as_ref() {
            if let Term::Global(ctor) = sapp_outer.as_ref() {
                if ctor == "SApp" {
                    // x = App(App(SApp, Y), a)
                    if let Term::App(inner, a) = x.as_ref() {
                        if let Term::App(sapp_inner, y) = inner.as_ref() {
                            if let Term::Global(ctor2) = sapp_inner.as_ref() {
                                if ctor2 == "SApp" {
                                    // y = App(App(SApp, eq_name), t)
                                    if let Term::App(inner2, t) = y.as_ref() {
                                        if let Term::App(sapp_inner2, sname_eq) = inner2.as_ref() {
                                            if let Term::Global(ctor3) = sapp_inner2.as_ref() {
                                                if ctor3 == "SApp" {
                                                    // sname_eq = App(SName, "Eq")
                                                    if let Term::App(sname, text) =
                                                        sname_eq.as_ref()
                                                    {
                                                        if let Term::Global(sname_ctor) =
                                                            sname.as_ref()
                                                        {
                                                            if sname_ctor == "SName" {
                                                                if let Term::Lit(Literal::Text(s)) =
                                                                    text.as_ref()
                                                                {
                                                                    if s == "Eq" {
                                                                        return Some((
                                                                            t.as_ref().clone(),
                                                                            a.as_ref().clone(),
                                                                            b.as_ref().clone(),
                                                                        ));
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
                        }
                    }
                }
            }
        }
    }
    None
}

/// Build DRefl type_s term
fn make_drefl(type_s: &Term, term: &Term) -> Term {
    let drefl = Term::Global("DRefl".to_string());
    let app1 = Term::App(Box::new(drefl), Box::new(type_s.clone()));
    Term::App(Box::new(app1), Box::new(term.clone()))
}

/// Build DAxiom (SName "Error")
fn make_error_derivation() -> Term {
    let daxiom = Term::Global("DAxiom".to_string());
    let error = make_sname_error();
    Term::App(Box::new(daxiom), Box::new(error))
}

/// Build DAxiom goal (identity derivation)
fn make_daxiom(goal: &Term) -> Term {
    let daxiom = Term::Global("DAxiom".to_string());
    Term::App(Box::new(daxiom), Box::new(goal.clone()))
}

/// Reflexivity tactic: try to prove a goal by reflexivity.
///
/// try_refl goal:
/// - If goal matches (Eq T a b) and a == b, return DRefl T a
/// - Otherwise return DAxiom (SName "Error")
fn try_try_refl_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    // Normalize the goal first
    let norm_goal = normalize(ctx, goal);

    // Pattern match: SApp (SApp (SApp (SName "Eq") T) a) b
    if let Some((type_s, left, right)) = extract_eq(&norm_goal) {
        // Check if left == right (structural equality)
        if syntax_equal(&left, &right) {
            // Success! Return DRefl T a
            return Some(make_drefl(&type_s, &left));
        }
    }

    // Failure: return error derivation
    Some(make_error_derivation())
}

// =============================================================================
// RING TACTIC (POLYNOMIAL EQUALITY)
// =============================================================================

use crate::cc;
use crate::lia;
use crate::ring;
use crate::simp;

/// Ring tactic: try to prove a goal by polynomial normalization.
///
/// try_ring goal:
/// - If goal matches (Eq T a b) where T is Int or Nat
/// - And poly(a) == poly(b) after normalization
/// - Returns DRingSolve goal
/// - Otherwise returns DAxiom (SName "Error")
fn try_try_ring_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    // Normalize the goal first
    let norm_goal = normalize(ctx, goal);

    // Pattern match: SApp (SApp (SApp (SName "Eq") T) a) b
    if let Some((type_s, left, right)) = extract_eq(&norm_goal) {
        // Check type is Int or Nat (ring types)
        if !is_ring_type(&type_s) {
            return Some(make_error_derivation());
        }

        // Reify both sides to polynomials (one interner: both sides must
        // agree on which named global is which variable)
        let mut vars = crate::reify::VarInterner::new();
        let poly_left = match ring::reify(&left, &mut vars) {
            Ok(p) => p,
            Err(_) => return Some(make_error_derivation()),
        };
        let poly_right = match ring::reify(&right, &mut vars) {
            Ok(p) => p,
            Err(_) => return Some(make_error_derivation()),
        };

        // Check canonical equality
        if poly_left.canonical_eq(&poly_right) {
            // Success! Return DRingSolve goal
            return Some(Term::App(
                Box::new(Term::Global("DRingSolve".to_string())),
                Box::new(norm_goal),
            ));
        }
    }

    // Failure: return error derivation
    Some(make_error_derivation())
}

/// Verify DRingSolve proof.
///
/// DRingSolve goal → goal (if verified)
fn try_dring_solve_conclude(ctx: &Context, goal: &Term) -> Option<Term> {
    let Some(norm_goal) = goal_to_certify(ctx, goal) else {
        return Some(make_sname_error());
    };

    // Extract T, lhs, rhs from Eq T lhs rhs
    if let Some((type_s, left, right)) = extract_eq(&norm_goal) {
        // Verify type is a ring type
        if !is_ring_type(&type_s) {
            return Some(make_sname_error());
        }

        // Reify and verify (one interner across both sides)
        let mut vars = crate::reify::VarInterner::new();
        let poly_left = match ring::reify(&left, &mut vars) {
            Ok(p) => p,
            Err(_) => return Some(make_sname_error()),
        };
        let poly_right = match ring::reify(&right, &mut vars) {
            Ok(p) => p,
            Err(_) => return Some(make_sname_error()),
        };

        if poly_left.canonical_eq(&poly_right) {
            return Some(norm_goal);
        }
    }

    Some(make_sname_error())
}

/// Check if a Syntax type is a ring type (Int or Nat)
fn is_ring_type(type_term: &Term) -> bool {
    if let Some(name) = extract_sname_from_syntax(type_term) {
        return name == "Int" || name == "Nat";
    }
    false
}

/// Extract name from SName term
fn extract_sname_from_syntax(term: &Term) -> Option<String> {
    if let Term::App(ctor, arg) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SName" {
                if let Term::Lit(Literal::Text(s)) = arg.as_ref() {
                    return Some(s.clone());
                }
            }
        }
    }
    None
}

// =============================================================================
// LIA TACTIC (LINEAR INTEGER ARITHMETIC)
// =============================================================================

/// LIA tactic: try to prove a goal by Fourier-Motzkin elimination.
///
/// try_lia goal:
/// - If goal matches (Lt/Le/Gt/Ge a b)
/// - And Fourier-Motzkin shows the negation is unsatisfiable
/// - Returns DLiaSolve goal
/// - Otherwise returns DAxiom (SName "Error")
fn try_try_lia_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    // Normalize the goal first
    let norm_goal = normalize(ctx, goal);

    // Extract comparison: (SApp (SApp (SName "Lt"|"Le"|etc) a) b)
    if let Some((rel, lhs_term, rhs_term)) = lia::extract_comparison(&norm_goal) {
        // Reify both sides to linear expressions (one interner across both)
        let mut vars = crate::reify::VarInterner::new();
        let lhs = match lia::reify_linear(&lhs_term, &mut vars) {
            Ok(l) => l,
            Err(_) => return Some(make_error_derivation()),
        };
        let rhs = match lia::reify_linear(&rhs_term, &mut vars) {
            Ok(r) => r,
            Err(_) => return Some(make_error_derivation()),
        };

        // Convert to negated constraint for validity checking
        if let Some(negated) = lia::goal_to_negated_constraint(&rel, &lhs, &rhs) {
            // If the negation is unsatisfiable, the goal is valid
            if lia::fourier_motzkin_unsat(&[negated]) {
                // Success! Return DLiaSolve goal
                return Some(Term::App(
                    Box::new(Term::Global("DLiaSolve".to_string())),
                    Box::new(norm_goal),
                ));
            }
        }
    }

    // Failure: return error derivation
    Some(make_error_derivation())
}

/// Verify DLiaSolve proof.
///
/// DLiaSolve goal → goal (if verified)
fn try_dlia_solve_conclude(ctx: &Context, goal: &Term) -> Option<Term> {
    let Some(norm_goal) = goal_to_certify(ctx, goal) else {
        return Some(make_sname_error());
    };

    // Extract comparison and verify
    if let Some((rel, lhs_term, rhs_term)) = lia::extract_comparison(&norm_goal) {
        // Reify both sides (one interner across both)
        let mut vars = crate::reify::VarInterner::new();
        let lhs = match lia::reify_linear(&lhs_term, &mut vars) {
            Ok(l) => l,
            Err(_) => return Some(make_sname_error()),
        };
        let rhs = match lia::reify_linear(&rhs_term, &mut vars) {
            Ok(r) => r,
            Err(_) => return Some(make_sname_error()),
        };

        // Verify via Fourier-Motzkin
        if let Some(negated) = lia::goal_to_negated_constraint(&rel, &lhs, &rhs) {
            if lia::fourier_motzkin_unsat(&[negated]) {
                return Some(norm_goal);
            }
        }
    }

    Some(make_sname_error())
}

// =============================================================================
// CONGRUENCE CLOSURE TACTIC
// =============================================================================

/// CC tactic: try to prove a goal by congruence closure.
///
/// try_cc goal:
/// - If goal is a direct equality (Eq a b) or an implication with hypotheses
/// - And congruence closure shows the conclusion follows
/// - Returns DccSolve goal
/// - Otherwise returns DAxiom (SName "Error")
fn try_try_cc_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    // Normalize the goal first
    let norm_goal = normalize(ctx, goal);

    // Use cc::check_goal which handles both:
    // - Direct equalities: (Eq a b)
    // - Implications: (implies (Eq x y) (Eq (f x) (f y)))
    if cc::check_goal(&norm_goal) {
        // Success! Return DccSolve goal
        return Some(Term::App(
            Box::new(Term::Global("DccSolve".to_string())),
            Box::new(norm_goal),
        ));
    }

    // Failure: return error derivation
    Some(make_error_derivation())
}

/// Verify DccSolve proof.
///
/// DccSolve goal → goal (if verified)
fn try_dcc_solve_conclude(ctx: &Context, goal: &Term) -> Option<Term> {
    let Some(norm_goal) = goal_to_certify(ctx, goal) else {
        return Some(make_sname_error());
    };

    // Re-verify using congruence closure
    if cc::check_goal(&norm_goal) {
        return Some(norm_goal);
    }

    Some(make_sname_error())
}

// =============================================================================
// SIMP TACTIC (SIMPLIFICATION)
// =============================================================================

/// try_simp goal:
/// - Prove equalities by simplification and arithmetic evaluation
/// - Handles hypotheses via implications: (implies (Eq x 0) (Eq (add x 1) 1))
/// - Returns DSimpSolve goal on success
/// - Otherwise returns DAxiom (SName "Error")
fn try_try_simp_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    // Normalize the goal first
    let norm_goal = normalize(ctx, goal);

    // Use simp::check_goal (congruence closure) which handles:
    // - Reflexive equalities: (Eq a a)
    // - Constant folding: (Eq (add 2 3) 5)
    // - Hypothesis substitution: (implies (Eq x 0) (Eq (add x 1) 1))
    // …or decide the equality by REFLECTION + kernel normalization (unfolds `## To` user
    // definitions and evaluates arithmetic), so `(double 3) equals 6` is proven by computation.
    if simp::check_goal(&norm_goal) || simp_compute_check(ctx, &norm_goal) {
        // Success! Return DSimpSolve goal
        return Some(Term::App(
            Box::new(Term::Global("DSimpSolve".to_string())),
            Box::new(norm_goal),
        ));
    }

    // Failure: return error derivation
    Some(make_error_derivation())
}

/// Match a reflected 2-argument application `SApp (SApp (SName name) a) b`, returning `(a, b)`.
/// (`extract_eq` is the 3-argument `Eq T a b` counterpart.)
fn extract_sapp2(term: &Term, name: &str) -> Option<(Term, Term)> {
    if let Term::App(outer, b) = term {
        if let Term::App(sapp_outer, x) = outer.as_ref() {
            if matches!(sapp_outer.as_ref(), Term::Global(c) if c == "SApp") {
                if let Term::App(inner, a) = x.as_ref() {
                    if let Term::App(sapp_inner, sname_app) = inner.as_ref() {
                        if matches!(sapp_inner.as_ref(), Term::Global(c) if c == "SApp") {
                            if let Term::App(sname, text) = sname_app.as_ref() {
                                if matches!(sname.as_ref(), Term::Global(c) if c == "SName")
                                    && matches!(text.as_ref(), Term::Lit(Literal::Text(s)) if s == name)
                                {
                                    return Some((a.as_ref().clone(), b.as_ref().clone()));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Match a reflected 1-argument application `SApp (SName name) a`, returning `a` (e.g. `not P`).
fn extract_sapp1(term: &Term, name: &str) -> Option<Term> {
    if let Term::App(outer, arg) = term {
        if let Term::App(sapp, sname_app) = outer.as_ref() {
            if matches!(sapp.as_ref(), Term::Global(c) if c == "SApp") {
                if let Term::App(sname, text) = sname_app.as_ref() {
                    if matches!(sname.as_ref(), Term::Global(c) if c == "SName")
                        && matches!(text.as_ref(), Term::Lit(Literal::Text(s)) if s == name)
                    {
                        return Some(arg.as_ref().clone());
                    }
                }
            }
        }
    }
    None
}

/// Reflect a `Syntax` term to a real term, normalize it, and read off an integer literal — the
/// ground value of a closed arithmetic expression, or `None` if it does not reduce to a literal.
fn eval_int(ctx: &Context, syntax: &Term) -> Option<i64> {
    let t = syntax_to_term(syntax)?;
    // `normalize_checked`, not `normalize`: a literal read out of an ABANDONED reduction is
    // not the value of the expression, and this feeds tactic decisions.
    if let Ok(Term::Lit(Literal::Int(n))) = normalize_checked(ctx, &t) {
        Some(n)
    } else {
        None
    }
}

/// Structural substitution: replace every subterm equal to `from` with `to`. Used to apply an
/// equality hypothesis (`x = 0`) as a rewrite. First-order — recurses through applications, which
/// is all the reflected arithmetic goals contain.
fn subst_structural(t: &Term, from: &Term, to: &Term) -> Term {
    if t == from {
        return to.clone();
    }
    match t {
        Term::App(f, x) => Term::App(
            Box::new(subst_structural(f, from, to)),
            Box::new(subst_structural(x, from, to)),
        ),
        _ => t.clone(),
    }
}

fn is_atom_var(t: &Term) -> bool {
    matches!(t, Term::Global(_) | Term::Var(_))
}

/// Decide an (optionally hypothesis-guarded) equality goal by REFLECTION + kernel normalization.
///
/// Peels `implies H1 (implies H2 … conclusion)`, uses each equality hypothesis `a = b` as a rewrite
/// (substituting the variable side by the other), reflects the conclusion's sides back to real terms
/// and checks they share a normal form — i.e. definitional equality in the kernel. This unfolds user
/// `## To` definitions and EVALUATES arithmetic, which the congruence-closure [`simp::check_goal`]
/// cannot: `(double 3) equals 6` and `x = 0 → x + 1 = 1` both decide here.
///
/// When the variable rewrites do not close it, every equality hypothesis is used as a rewrite
/// between normal forms instead ([`rewrite_by_hypotheses`]), which is how an induction step
/// consumes its hypothesis.
///
/// Sound: same normal form ⟺ `refl`-inhabited; a false equality (`48 = 6·9` → `48` vs `54`) is
/// rejected, and a hypothesis neither pass can use is conservatively DROPPED (so we only ever
/// prove the conclusion when it holds under the rewrites we applied). Symbolic-only goals
/// (`add a b = add b a`) return `false` and are left to `ring`.
fn simp_compute_check(ctx: &Context, norm_goal: &Term) -> bool {
    // Peel hypotheses off nested implications.
    let mut hyps: Vec<Term> = Vec::new();
    let mut current = norm_goal.clone();
    while let Some((h, rest)) = as_implication(&current) {
        hyps.push(h);
        current = rest;
    }

    // The conclusion must be an equality.
    let (_ty, left_s, right_s) = match extract_eq(&current) {
        Some(t) => t,
        None => return false,
    };
    let (mut l, mut r) = match (syntax_to_term(&left_s), syntax_to_term(&right_s)) {
        (Some(l), Some(r)) => (l, r),
        _ => return false,
    };

    // A pending declaration is unavailable, even if reduction would erase it
    // or leave the same unresolved name on both sides.
    if ctx.has_pending_dependency(&l)
        || ctx.has_pending_dependency(&r)
        || syntax_to_term(&_ty).is_some_and(|ty| ctx.has_pending_dependency(&ty))
    {
        return false;
    }

    let (conclusion_left, conclusion_right) = (l.clone(), r.clone());

    // Apply each equality hypothesis as a rewrite (variable side → other side).
    for h in &hyps {
        let (hl, hr) = match extract_eq(h) {
            Some((_, hl_s, hr_s)) => match (syntax_to_term(&hl_s), syntax_to_term(&hr_s)) {
                (Some(hl), Some(hr)) => (hl, hr),
                _ => continue,
            },
            None => continue,
        };
        let (from, to) = if is_atom_var(&hl) {
            (hl, hr)
        } else if is_atom_var(&hr) {
            (hr, hl)
        } else {
            continue;
        };
        if ctx.has_pending_dependency(&from) || ctx.has_pending_dependency(&to) {
            return false;
        }
        l = subst_structural(&l, &from, &to);
        r = subst_structural(&r, &from, &to);
    }

    // THE HONEST COMPARISON. `normalize` hands back how far it got, so "these two reached the
    // same normal form" and "these two were abandoned at the same place" are the same answer —
    // and this is a door that ACCEPTS PROOFS. Two ABANDONED reductions comparing equal
    // certifies nothing, so both sides must have actually FINISHED. `normalize_checked` is the
    // only way to know.
    if let (Ok(ln), Ok(rn)) = (normalize_checked(ctx, &l), normalize_checked(ctx, &r)) {
        if ln == rn {
            return true;
        }
    }
    rewrite_by_hypotheses(ctx, &hyps, conclusion_left, conclusion_right)
}

/// How many rounds of rewrite-then-normalize [`rewrite_by_hypotheses`] runs before it gives up.
const MAX_HYPOTHESIS_REWRITE_ROUNDS: usize = 16;

/// Decide `l = r` under the equality hypotheses by rewriting with them BETWEEN NORMAL FORMS —
/// what an induction step needs and a variable substitution cannot give.
///
/// The succ case of `0 + n = n` is `(0 + k = k) → 0 + (k+1) = k+1`. Substituting the variable
/// side of the hypothesis (`k := 0 + k`) only moves the problem; the step is closed by FOLDING:
/// `0 + (k+1)` computes to `Succ (0 + k)`, and the hypothesis turns that `0 + k` back into `k`.
/// Both sides of every hypothesis are normalized first, so the rule meets the goal in the one
/// shape reduction leaves it in — an unfolded `Nat_add` stuck on the open `k` is the same term
/// in the hypothesis and in the goal. A hypothesis with an atom on one side rewrites the other
/// side to that atom; any other hypothesis rewrites left to right.
///
/// Sound: every rewrite replaces a subterm `a` by `b` under the hypothesis `a = b`, and only at
/// a position reached through APPLICATIONS from the root — never under a binder, a match or a
/// fixpoint — so no variable of `a` or `b` is ever captured and the context around the rewritten
/// subterm is a plain first-order application, where `a = b` gives `C[a] = C[b]`. A hypothesis
/// whose sides cannot be normalized is dropped, which only proves less, and a pending
/// declaration anywhere refuses the whole goal, as the variable rewrites above do. The answer is
/// again two FINISHED normal forms compared, so an abandoned reduction certifies nothing, and a
/// hypothesis that cannot be oriented toward termination (`f k = g (f k)`) spends at most
/// [`MAX_HYPOTHESIS_REWRITE_ROUNDS`] rounds.
fn rewrite_by_hypotheses(ctx: &Context, hyps: &[Term], mut l: Term, mut r: Term) -> bool {
    let mut rules: Vec<(Term, Term)> = Vec::new();
    for h in hyps {
        let Some((_, a_s, b_s)) = extract_eq(h) else {
            continue;
        };
        let (Some(a), Some(b)) = (syntax_to_term(&a_s), syntax_to_term(&b_s)) else {
            continue;
        };
        if ctx.has_pending_dependency(&a) || ctx.has_pending_dependency(&b) {
            return false;
        }
        let (Ok(a), Ok(b)) = (normalize_checked(ctx, &a), normalize_checked(ctx, &b)) else {
            continue;
        };
        if a == b {
            continue;
        }
        rules.push(if is_atom_var(&a) && !is_atom_var(&b) {
            (b, a)
        } else {
            (a, b)
        });
    }
    if rules.is_empty() {
        return false;
    }
    for _ in 0..MAX_HYPOTHESIS_REWRITE_ROUNDS {
        let (Ok(ln), Ok(rn)) = (normalize_checked(ctx, &l), normalize_checked(ctx, &r)) else {
            return false;
        };
        if ln == rn {
            return true;
        }
        let (next_l, rewrote_l) = rewrite_application_spine(&ln, &rules);
        let (next_r, rewrote_r) = rewrite_application_spine(&rn, &rules);
        if !rewrote_l && !rewrote_r {
            return false;
        }
        l = next_l;
        r = next_r;
    }
    false
}

/// Replace every outermost occurrence of a rule's left side by its right side, descending only
/// through applications (see [`rewrite_by_hypotheses`] for why that is the sound frontier), and
/// report whether anything was replaced.
fn rewrite_application_spine(t: &Term, rules: &[(Term, Term)]) -> (Term, bool) {
    if let Some((_, to)) = rules.iter().find(|(from, _)| from == t) {
        return (to.clone(), true);
    }
    match t {
        Term::App(f, x) => {
            let (f, rewrote_f) = rewrite_application_spine(f, rules);
            let (x, rewrote_x) = rewrite_application_spine(x, rules);
            (Term::App(Box::new(f), Box::new(x)), rewrote_f || rewrote_x)
        }
        _ => (t.clone(), false),
    }
}

/// Verify DSimpSolve proof.
///
/// DSimpSolve goal → goal (if verified)
fn try_dsimp_solve_conclude(ctx: &Context, goal: &Term) -> Option<Term> {
    let Some(norm_goal) = goal_to_certify(ctx, goal) else {
        return Some(make_sname_error());
    };

    // Re-verify using simplification OR reflection+normalization (same check as the reduce path).
    if simp::check_goal(&norm_goal) || simp_compute_check(ctx, &norm_goal) {
        return Some(norm_goal);
    }

    Some(make_sname_error())
}

// =============================================================================
// OMEGA TACTIC (TRUE INTEGER ARITHMETIC)
// =============================================================================

/// try_omega goal:
///
/// Omega tactic using integer-aware Fourier-Motzkin elimination.
/// Unlike lia (which uses rationals), omega handles integers properly:
/// - x > 1 means x >= 2 (strict-to-nonstrict conversion)
/// - 3x <= 10 means x <= 3 (floor division)
///
/// - Extracts comparison (Lt, Le, Gt, Ge) from goal
/// - Reifies to integer linear expressions
/// - Converts to negated constraint (validity = negation is unsat)
/// - Applies omega test
/// - Returns DOmegaSolve goal on success
fn try_try_omega_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Handle implications: extract hypotheses and check conclusion
    if let Some((hyps, conclusion)) = extract_implications(&norm_goal) {
        // Convert hypotheses to constraints. ONE interner spans hypotheses
        // and conclusion: a named global must be the same variable everywhere.
        let mut vars = crate::reify::VarInterner::new();
        let constraints = omega_hyp_constraints(&hyps, &mut vars);

        // Now check if the conclusion is provable given the constraints
        if let Some((rel, lhs_term, rhs_term)) = omega::extract_comparison(&conclusion) {
            if let (Some(lhs), Some(rhs)) = (
                omega::reify_int_linear(&lhs_term, &mut vars),
                omega::reify_int_linear(&rhs_term, &mut vars),
            ) {
                // To prove the conclusion, check if its negation is unsat
                if let Some(neg_constraint) = omega::goal_to_negated_constraint(&rel, &lhs, &rhs) {
                    let mut all_constraints = constraints;
                    all_constraints.push(neg_constraint);

                    if omega::omega_unsat(&all_constraints) {
                        return Some(Term::App(
                            Box::new(Term::Global("DOmegaSolve".to_string())),
                            Box::new(norm_goal),
                        ));
                    }
                }
            }
        }

        // Failure
        return Some(make_error_derivation());
    }

    // Direct comparison (no hypotheses)
    if let Some((rel, lhs_term, rhs_term)) = omega::extract_comparison(&norm_goal) {
        let mut vars = crate::reify::VarInterner::new();
        if let (Some(lhs), Some(rhs)) = (
            omega::reify_int_linear(&lhs_term, &mut vars),
            omega::reify_int_linear(&rhs_term, &mut vars),
        ) {
            if let Some(constraint) = omega::goal_to_negated_constraint(&rel, &lhs, &rhs) {
                if omega::omega_unsat(&[constraint]) {
                    return Some(Term::App(
                        Box::new(Term::Global("DOmegaSolve".to_string())),
                        Box::new(norm_goal),
                    ));
                }
            }
        }
    }

    Some(make_error_derivation())
}

/// Convert comparison hypotheses to omega constraints (facts).
///
/// A hypothesis is given, so it enters the system directly with integer
/// strict-to-nonstrict tightening: `a < b` becomes `a - b + 1 <= 0`.
fn omega_hyp_constraints(
    hyps: &[Term],
    vars: &mut crate::reify::VarInterner,
) -> Vec<omega::IntConstraint> {
    let one = omega::IntExpr::constant(1);
    let mut constraints = Vec::new();
    for hyp in hyps {
        if let Some((rel, lhs_term, rhs_term)) = omega::extract_comparison(hyp) {
            if let (Some(lhs), Some(rhs)) = (
                omega::reify_int_linear(&lhs_term, vars),
                omega::reify_int_linear(&rhs_term, vars),
            ) {
                match rel.as_str() {
                    "Lt" | "lt" => {
                        // a < b means a - b <= -1, i.e., (a - b + 1) <= 0
                        constraints.push(omega::IntConstraint {
                            expr: lhs.sub(&rhs).add(&one),
                            strict: false,
                        });
                    }
                    "Le" | "le" => {
                        // a <= b means a - b <= 0
                        constraints.push(omega::IntConstraint {
                            expr: lhs.sub(&rhs),
                            strict: false,
                        });
                    }
                    "Gt" | "gt" => {
                        // a > b means a - b >= 1, i.e., (b - a + 1) <= 0
                        constraints.push(omega::IntConstraint {
                            expr: rhs.sub(&lhs).add(&one),
                            strict: false,
                        });
                    }
                    "Ge" | "ge" => {
                        // a >= b means b - a <= 0
                        constraints.push(omega::IntConstraint {
                            expr: rhs.sub(&lhs),
                            strict: false,
                        });
                    }
                    _ => {}
                }
            }
        }
    }
    constraints
}

/// Verify DOmegaSolve proof.
///
/// DOmegaSolve goal → goal (if verified)
fn try_domega_solve_conclude(ctx: &Context, goal: &Term) -> Option<Term> {
    let Some(norm_goal) = goal_to_certify(ctx, goal) else {
        return Some(make_sname_error());
    };

    // Re-verify using omega test
    // Handle implications
    if let Some((hyps, conclusion)) = extract_implications(&norm_goal) {
        let mut vars = crate::reify::VarInterner::new();
        let constraints = omega_hyp_constraints(&hyps, &mut vars);

        if let Some((rel, lhs_term, rhs_term)) = omega::extract_comparison(&conclusion) {
            if let (Some(lhs), Some(rhs)) = (
                omega::reify_int_linear(&lhs_term, &mut vars),
                omega::reify_int_linear(&rhs_term, &mut vars),
            ) {
                if let Some(neg_constraint) = omega::goal_to_negated_constraint(&rel, &lhs, &rhs) {
                    let mut all_constraints = constraints;
                    all_constraints.push(neg_constraint);

                    if omega::omega_unsat(&all_constraints) {
                        return Some(norm_goal);
                    }
                }
            }
        }

        return Some(make_sname_error());
    }

    // Direct comparison
    if let Some((rel, lhs_term, rhs_term)) = omega::extract_comparison(&norm_goal) {
        let mut vars = crate::reify::VarInterner::new();
        if let (Some(lhs), Some(rhs)) = (
            omega::reify_int_linear(&lhs_term, &mut vars),
            omega::reify_int_linear(&rhs_term, &mut vars),
        ) {
            if let Some(constraint) = omega::goal_to_negated_constraint(&rel, &lhs, &rhs) {
                if omega::omega_unsat(&[constraint]) {
                    return Some(norm_goal);
                }
            }
        }
    }

    Some(make_sname_error())
}

// =============================================================================
// AUTO TACTIC (THE INFINITY GAUNTLET)
// =============================================================================

/// Check if a derivation is an error derivation.
///
/// Error derivations have the pattern: DAxiom (SApp (SName "Error") ...)
fn is_error_derivation(term: &Term) -> bool {
    // Check for DAxiom constructor
    if let Term::App(ctor, arg) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "DAxiom" {
                // Check if arg is SName "Error" or SApp (SName "Error") ...
                if let Term::App(sname, inner) = arg.as_ref() {
                    if let Term::Global(sn) = sname.as_ref() {
                        if sn == "SName" {
                            if let Term::Lit(Literal::Text(s)) = inner.as_ref() {
                                return s == "Error";
                            }
                        }
                        if sn == "SApp" {
                            // Recursively check - could be SApp (SName "Error") ...
                            return true; // For now, treat any DAxiom as potential error
                        }
                    }
                }
                // DAxiom with SName "Error"
                if let Term::Global(sn) = arg.as_ref() {
                    // This shouldn't happen but just in case
                    return sn == "Error";
                }
                return true; // Any DAxiom is treated as error for auto purposes
            }
        }
    }
    false
}

/// Auto tactic: tries each decision procedure in sequence.
///
/// Order: True/False → simp → ring → cc → omega → lia
/// Returns the first successful derivation, or error if all fail.
// =============================================================================
// HARDWARE TACTICS (bitblast, tabulate, hw_auto)
// =============================================================================

/// Bitblast tactic: prove Eq Bit lhs rhs by normalizing both sides.
///
/// Operates on Syntax terms: SApp(SApp(SApp(SName("Eq"), SName("Bit")), lhs), rhs)
/// Normalizes lhs and rhs to concrete Bit values (B0/B1) and checks equality.
fn try_try_bitblast_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Extract Eq T lhs rhs from Syntax
    if let Some((type_s, left, right)) = extract_eq_syntax_parts(&norm_goal) {
        // Check type is Bit (or BVec — for future extension)
        if !is_bit_type(&type_s) {
            return Some(make_error_derivation());
        }

        // Evaluate both sides using syn_eval with delta reduction support.
        // syn_step now handles definition unfolding (delta reduction),
        // so syn_eval correctly reduces expressions like
        // SApp(SApp(SName("bit_and"), SName("B1")), SName("B0")) → SName("B0")
        let fuel = 1000;
        let left_eval = try_syn_eval_reduce(ctx, fuel, &left)?;
        let right_eval = try_syn_eval_reduce(ctx, fuel, &right)?;

        if syntax_equal(&left_eval, &right_eval) {
            return Some(Term::App(
                Box::new(Term::Global("DBitblastSolve".to_string())),
                Box::new(norm_goal),
            ));
        }
    }

    Some(make_error_derivation())
}

/// Convert a Syntax deep embedding back to a kernel Term.
///
/// This is the proper bridge between the reflected Syntax representation
/// and the kernel's native Term language. The kernel normalizer handles
/// all reduction strategies (delta, iota, beta, fix), so reifying to Term
/// and normalizing there gives correct results for hardware gate expressions.
///
/// Handles: SName, SApp, SLit, SVar, SLam, SPi, SSort, SMatch
fn syntax_to_term(syntax: &Term) -> Option<Term> {
    if let Term::App(ctor, inner) = syntax {
        if let Term::Global(name) = ctor.as_ref() {
            match name.as_str() {
                "SName" => {
                    if let Term::Lit(Literal::Text(s)) = inner.as_ref() {
                        return Some(Term::Global(s.clone()));
                    }
                }
                "SLit" => {
                    if let Term::Lit(Literal::Int(n)) = inner.as_ref() {
                        return Some(Term::Lit(Literal::Int(*n)));
                    }
                }
                "SVar" => {
                    if let Term::Lit(Literal::Int(n)) = inner.as_ref() {
                        return Some(Term::Var(format!("v{}", n)));
                    }
                }
                "SGlobal" => {
                    if let Term::Lit(Literal::Int(n)) = inner.as_ref() {
                        return Some(Term::Var(format!("g{}", n)));
                    }
                }
                _ => {}
            }
        }
    }

    // Two-argument constructors: SApp, SLam, SPi
    if let Term::App(outer, second) = syntax {
        if let Term::App(sctor, first) = outer.as_ref() {
            if let Term::Global(name) = sctor.as_ref() {
                match name.as_str() {
                    "SApp" => {
                        let f = syntax_to_term(first)?;
                        let x = syntax_to_term(second)?;
                        return Some(Term::App(Box::new(f), Box::new(x)));
                    }
                    "SLam" => {
                        let param_type = syntax_to_term(first)?;
                        let body = syntax_to_term(second)?;
                        return Some(Term::Lambda {
                            param: "_".to_string(),
                            param_type: Box::new(param_type),
                            body: Box::new(body),
                        });
                    }
                    "SPi" => {
                        let param_type = syntax_to_term(first)?;
                        let body_type = syntax_to_term(second)?;
                        return Some(Term::Pi {
                            param: "_".to_string(),
                            param_type: Box::new(param_type),
                            body_type: Box::new(body_type),
                        });
                    }
                    _ => {}
                }
            }
        }
    }

    // Three-argument: SMatch discriminant motive cases
    // (SApp (SApp (SApp (SName "SMatch") disc) motive) cases)
    if let Term::App(outer, third) = syntax {
        if let Term::App(mid, second) = outer.as_ref() {
            if let Term::App(inner, first) = mid.as_ref() {
                if let Term::Global(name) = inner.as_ref() {
                    if name == "SMatch" {
                        let disc = syntax_to_term(first)?;
                        let motive = syntax_to_term(second)?;
                        // cases would need list handling — skip for now
                        let _ = third;
                        return Some(Term::Match {
                            discriminant: Box::new(disc),
                            motive: Box::new(motive),
                            cases: vec![],
                        });
                    }
                }
            }
        }
    }

    None
}

/// Check if a Syntax type term represents Bit.
fn is_bit_type(term: &Term) -> bool {
    // SName "Bit"
    if let Term::App(ctor, inner) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SName" {
                if let Term::Lit(Literal::Text(s)) = inner.as_ref() {
                    return s == "Bit";
                }
            }
        }
    }
    false
}

/// Substitute SVar(idx) with `replacement` in a Syntax term, shifting de Bruijn
/// indices under binders (SPi, SLam).
fn syntax_subst_var(term: &Term, idx: i64, replacement: &Term) -> Term {
    // Check for SVar n
    if let Term::App(ctor, inner) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SVar" {
                if let Term::Lit(Literal::Int(n)) = inner.as_ref() {
                    if *n == idx {
                        return replacement.clone();
                    } else if *n > idx {
                        // Free variable above the binder — shift down
                        return make_svar(*n - 1);
                    } else {
                        return term.clone();
                    }
                }
            }
        }
    }

    // Two-argument constructors: SApp, SLam, SPi
    if let Term::App(outer, second) = term {
        if let Term::App(sctor, first) = outer.as_ref() {
            if let Term::Global(name) = sctor.as_ref() {
                match name.as_str() {
                    "SApp" => {
                        let f = syntax_subst_var(first, idx, replacement);
                        let x = syntax_subst_var(second, idx, replacement);
                        return make_sapp(f, x);
                    }
                    "SPi" => {
                        // Under a binder, shift the index
                        let param_t = syntax_subst_var(first, idx, replacement);
                        let body = syntax_subst_var(second, idx + 1, replacement);
                        return make_spi(param_t, body);
                    }
                    "SLam" => {
                        let param_t = syntax_subst_var(first, idx, replacement);
                        let body = syntax_subst_var(second, idx + 1, replacement);
                        return make_slam(param_t, body);
                    }
                    _ => {}
                }
            }
        }
    }

    // One-argument constructors that don't bind: SName, SLit, SSort — return as-is
    term.clone()
}

/// Tabulate tactic: prove universally quantified Bit goals by exhaustive enumeration.
///
/// For a goal of the form SPi(SName("Bit"), body):
/// 1. Substitute SVar 0 with SName("B0") in body → body_b0
/// 2. Substitute SVar 0 with SName("B1") in body → body_b1
/// 3. Recursively verify both substituted bodies
/// 4. If both succeed, return DTabulateSolve(goal)
///
/// Handles nested Pi binders (e.g., Pi(a:Bit). Pi(b:Bit). P(a,b)) by recursion.
/// Rejects non-Bit quantification.
fn try_try_tabulate_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Try to extract SPi(param_type, body)
    if let Some((param_type, body)) = extract_spi(&norm_goal) {
        // Only handle Bit quantification
        if !is_bit_type(&param_type) {
            return Some(make_error_derivation());
        }

        // Substitute SVar 0 with B0 and B1
        let body_b0 = syntax_subst_var(&body, 0, &make_sname("B0"));
        let body_b1 = syntax_subst_var(&body, 0, &make_sname("B1"));

        // Recursively verify both cases
        let ok_b0 = tabulate_verify_case(ctx, &body_b0);
        let ok_b1 = tabulate_verify_case(ctx, &body_b1);

        if ok_b0 && ok_b1 {
            return Some(Term::App(
                Box::new(Term::Global("DTabulateSolve".to_string())),
                Box::new(norm_goal),
            ));
        }

        return Some(make_error_derivation());
    }

    // Not a Pi — not a quantified goal
    Some(make_error_derivation())
}

/// Verify a single case for tabulation. The case is either:
/// - An Eq(Bit, lhs, rhs) — verify by normalizing both sides
/// - Another SPi(Bit, ...) — recurse (nested quantification)
/// - A formula that reduces to SName("True") — trivially true
fn tabulate_verify_case(ctx: &Context, case: &Term) -> bool {
    let norm = normalize(ctx, case);

    // If it's another Pi over Bit, recurse
    if let Some((param_type, _)) = extract_spi(&norm) {
        if is_bit_type(&param_type) {
            if let Some(result) = try_try_tabulate_reduce(ctx, &norm) {
                return !is_error_derivation(&result);
            }
        }
        return false;
    }

    // Try to verify as an Eq
    if let Some((_type_s, left, right)) = extract_eq_syntax_parts(&norm) {
        let fuel = 1000;
        if let (Some(left_eval), Some(right_eval)) = (
            try_syn_eval_reduce(ctx, fuel, &left),
            try_syn_eval_reduce(ctx, fuel, &right),
        ) {
            return syntax_equal(&left_eval, &right_eval);
        }
        // Try evaluating the whole equality — might reduce to True
        if let Some(full_eval) = try_syn_eval_reduce(ctx, fuel, &norm) {
            if is_sname_with_value(&full_eval, "True") {
                return true;
            }
        }
        return false;
    }

    // Try evaluating the whole formula — might be SName("True")
    let fuel = 1000;
    if let Some(eval) = try_syn_eval_reduce(ctx, fuel, &norm) {
        if is_sname_with_value(&eval, "True") {
            return true;
        }
    }

    false
}

/// Check if a Syntax term is SName(value)
fn is_sname_with_value(term: &Term, value: &str) -> bool {
    if let Term::App(ctor, inner) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SName" {
                if let Term::Lit(Literal::Text(s)) = inner.as_ref() {
                    return s == value;
                }
            }
        }
    }
    false
}

/// Whether an ATOMIC (non-connective) goal is decided by the arithmetic/equality tactics. Used by
/// [`auto_prop_check`] at the leaves of a propositional goal.
fn auto_atom_provable(ctx: &Context, goal: &Term) -> bool {
    if simp::check_goal(goal) || simp_compute_check(ctx, goal) {
        return true;
    }
    for reduce in [
        try_try_ring_reduce,
        try_try_omega_reduce,
        try_try_lia_reduce,
    ] {
        if let Some(r) = reduce(ctx, goal) {
            if !is_error_derivation(&r) {
                return true;
            }
        }
    }
    false
}

/// Decide a proposition by REFLECTION + kernel computation, returning `Some(true)` if it is
/// decidably TRUE, `Some(false)` if decidably FALSE, and `None` if undecided (left to other tactics).
///
/// Handles: `True`/`False`; equalities and equality-hypothesis implications (via
/// [`simp_compute_check`], which unfolds `## To` defs + evaluates arithmetic); `not P` (refute `P`);
/// `And`/`Or`/`implies` over sub-decisions; and integer comparisons `Lt`/`le`/`Gt`/`ge` over closed
/// literals (both directions — so `not (Lt 5 2)` and `not (Eq 2 3)` decide). Sound: every `Some(b)`
/// follows from computation or an intro/refutation rule; symbolic goals return `None`.
fn decide_prop(ctx: &Context, goal: &Term) -> Option<bool> {
    let ng = normalize(ctx, goal);

    if is_sname_with_value(&ng, "True") {
        return Some(true);
    }
    if is_sname_with_value(&ng, "False") {
        return Some(false);
    }
    // Equalities + implication-with-equality-hypotheses: a positive decision only.
    if simp_compute_check(ctx, &ng) {
        return Some(true);
    }
    if let Some(p) = extract_sapp1(&ng, "not") {
        return decide_prop(ctx, &p).map(|b| !b);
    }
    if let Some((p, q)) = extract_sapp2(&ng, "And") {
        return match (decide_prop(ctx, &p), decide_prop(ctx, &q)) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        };
    }
    if let Some((p, q)) = extract_sapp2(&ng, "Or") {
        return match (decide_prop(ctx, &p), decide_prop(ctx, &q)) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        };
    }
    if as_implication(&ng).is_some() {
        return decide_implication(ctx, &ng);
    }
    // Integer comparisons over closed literals (both directions).
    let comparisons: [(&str, fn(i64, i64) -> bool); 4] = [
        ("Lt", |a, b| a < b),
        ("le", |a, b| a <= b),
        ("Gt", |a, b| a > b),
        ("ge", |a, b| a >= b),
    ];
    for (name, cmp) in comparisons {
        if let Some((a_s, b_s)) = extract_sapp2(&ng, name) {
            if let (Some(a), Some(b)) = (eval_int(ctx, &a_s), eval_int(ctx, &b_s)) {
                return Some(cmp(a, b));
            }
            // Symbolic comparison (e.g. `x < x + 1`): defer the positive case to lia/omega.
            return if auto_atom_provable(ctx, &ng) {
                Some(true)
            } else {
                None
            };
        }
    }
    // A disequality survives to here only when both sides are distinct ground literals.
    if let Some((_, a_s, b_s)) = extract_eq(&ng) {
        if let (Some(a), Some(b)) = (eval_int(ctx, &a_s), eval_int(ctx, &b_s)) {
            return Some(a == b);
        }
        return None;
    }
    // Atomic fallback: provable by the arithmetic tactics?
    if auto_atom_provable(ctx, &ng) {
        Some(true)
    } else {
        None
    }
}

/// Decide an implication goal `H1 -> H2 -> … -> C` by →-INTRODUCTION: assume the hypotheses and
/// decide the conclusion. Handles a refutable hypothesis (the implication is vacuously true), the
/// conclusion being one of the hypotheses (ASSUMPTION), an equality hypothesis used to REWRITE the
/// conclusion — including a non-equality conclusion like `x = 5 -> x < 10`, which the equality-only
/// `simp_compute_check` cannot — and a conclusion decidable on its own (weakening). Sound: assuming
/// `P` to prove `Q` is exactly →-intro; a false `Q[hyps]` means the implication fails at that
/// assignment, so it is not a theorem. Returns `None` when undecided.
fn decide_implication(ctx: &Context, goal: &Term) -> Option<bool> {
    let mut hyps: Vec<Term> = Vec::new();
    let mut concl = normalize(ctx, goal);
    while let Some((h, rest)) = as_implication(&concl) {
        hyps.push(normalize(ctx, &h));
        concl = normalize(ctx, &rest);
    }
    if hyps.is_empty() {
        return None;
    }

    // Process the hypotheses left-to-right, threading the substitutions the equality hypotheses
    // license. If any hypothesis becomes decidably FALSE once earlier equalities are applied, the
    // hypothesis set is jointly UNSATISFIABLE, so the implication is vacuously true for ANY
    // conclusion (this catches both a single false hypothesis and a contradictory pair like
    // `x = 5` ∧ `x = 6`). This is sound: no assignment satisfies all antecedents.
    let mut subst: Vec<(Term, Term)> = Vec::new();
    for h in &hyps {
        let mut hh = h.clone();
        for (from, to) in &subst {
            hh = subst_structural(&hh, from, to);
        }
        let hh = normalize(ctx, &hh);
        if decide_prop(ctx, &hh) == Some(false) {
            return Some(true); // contradictory hypotheses ⇒ vacuously true
        }
        if let Some((_, hl, hr)) = extract_eq(&hh) {
            match (syntax_to_term(&hl), syntax_to_term(&hr)) {
                (Some(l), Some(_)) if is_atom_var(&l) => subst.push((hl, hr)),
                (Some(_), Some(r)) if is_atom_var(&r) => subst.push((hr, hl)),
                _ => {}
            }
        }
    }

    // Assumption: the conclusion is (syntactically) one of the hypotheses.
    if hyps.iter().any(|h| syntax_equal(h, &concl)) {
        return Some(true);
    }

    // Rewrite the conclusion with the gathered equality substitutions, then decide it.
    let mut c = concl;
    for (from, to) in &subst {
        c = subst_structural(&c, from, to);
    }
    decide_prop(ctx, &c)
}

/// Whether a propositional/decidable goal is provable — `decide_prop` says `Some(true)`. This is
/// what lets `auto` (and the `decide` tactic) conclude `(And True True)`, `(not (Eq Int 2 3))`,
/// `(not (Lt 5 2))`, `(implies True True)` — which the flat tactic list cannot.
fn auto_prop_check(ctx: &Context, goal: &Term) -> bool {
    decide_prop(ctx, goal) == Some(true)
}

// =============================================================================
// TACTIC SEQUENCING — multi-step proof scripts (`intro. <closer>.`)
//
// A script is carried as `try_script <name-list> <goal>`, where the name-list is a reflected,
// INERT list of tactic names (`SApp (SApp (SName "step") (SName "intro")) … (SName "end")`). Keeping
// the steps as names (not `try_<kind> goal` terms) is essential: `try_<kind> goal` would REDUCE to
// its derivation against the ORIGINAL goal before the evaluator runs, so a structural closer like
// `left`/`split` (which needs the residual after `intro`, not the whole implication) could never be
// applied correctly. The evaluator threads the residual goal + hypotheses itself.
// =============================================================================

/// The reflected `SName "s"` as a string.
fn extract_sname_string(t: &Term) -> Option<String> {
    if let Term::App(f, inner) = t {
        if matches!(f.as_ref(), Term::Global(n) if n == "SName") {
            if let Term::Lit(Literal::Text(s)) = inner.as_ref() {
                return Some(s.clone());
            }
        }
    }
    None
}

/// Match `try_script list goal` = `App(App(Global "try_script", list), goal)`.
fn extract_try_script(term: &Term) -> Option<(Term, Term)> {
    if let Term::App(outer, goal) = term {
        if let Term::App(g, list) = outer.as_ref() {
            if matches!(g.as_ref(), Term::Global(n) if n == "try_script") {
                return Some(((**list).clone(), (**goal).clone()));
            }
        }
    }
    None
}

/// Decode the reflected script list into tactic names, or `None` if it is malformed.
fn extract_script_names(list: &Term) -> Option<Vec<String>> {
    let mut names = Vec::new();
    let mut cur = list.clone();
    loop {
        if is_sname_with_value(&cur, "end") {
            return Some(names);
        }
        let (name_s, rest) = extract_sapp2(&cur, "step")?;
        names.push(extract_sname_string(&name_s)?);
        cur = rest;
    }
}

/// Build the reflected 2-argument application `SApp (SApp (SName name) a) b`.
fn build_sapp2(name: &str, a: Term, b: Term) -> Term {
    let sname = Term::App(
        Box::new(Term::Global("SName".to_string())),
        Box::new(Term::Lit(Literal::Text(name.to_string()))),
    );
    let sapp = |x: Term, y: Term| {
        Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SApp".to_string())),
                Box::new(x),
            )),
            Box::new(y),
        )
    };
    sapp(sapp(sname, a), b)
}

/// Rebuild the implication `h1 -> h2 -> … -> goal` from a hypothesis list and a conclusion.
fn wrap_implication(hyps: &[Term], goal: &Term) -> Term {
    let mut result = goal.clone();
    for h in hyps.iter().rev() {
        result = build_sapp2("implies", h.clone(), result);
    }
    result
}

/// Run the reduce of the named single-shot tactic on `goal` (already normalized).
fn run_named_tactic_reduce(ctx: &Context, kind: &str, goal: &Term) -> Option<Term> {
    match kind {
        "ring" => try_try_ring_reduce(ctx, goal),
        "lia" => try_try_lia_reduce(ctx, goal),
        "cc" => try_try_cc_reduce(ctx, goal),
        "simp" => try_try_simp_reduce(ctx, goal),
        "omega" => try_try_omega_reduce(ctx, goal),
        "auto" => try_try_auto_reduce(ctx, goal),
        "decide" => try_try_decide_reduce(ctx, goal),
        "refl" => try_try_refl_reduce(ctx, goal),
        "assumption" => try_try_assumption_reduce(ctx, goal),
        "split" => try_try_split_reduce(ctx, goal),
        "left" => try_try_left_reduce(ctx, goal),
        "right" => try_try_right_reduce(ctx, goal),
        _ => None,
    }
}

/// Whether the named tactic proves `goal` under `hyps`. Tries it two sound ways: on the re-wrapped
/// `hyps -> goal` (so a hypothesis-aware tactic like `auto`/`assumption`/`cc` uses the hypotheses)
/// AND on the bare residual `goal` (so a structural tactic like `left`/`split`/`ring` — which cannot
/// consume implications — proves it directly, valid by weakening). Either success discharges it.
fn close_under_hyps(ctx: &Context, kind: &str, hyps: &[Term], goal: &Term) -> bool {
    let ng = normalize(ctx, goal);
    let non_error = |d: Option<Term>| d.is_some_and(|t| !is_error_derivation(&t));
    if !hyps.is_empty() {
        let wrapped = normalize(ctx, &wrap_implication(hyps, &ng));
        if non_error(run_named_tactic_reduce(ctx, kind, &wrapped)) {
            return true;
        }
    }
    non_error(run_named_tactic_reduce(ctx, kind, &ng))
}

/// Evaluate a proof SCRIPT (list of tactic names) against a hypothesis context and current goal.
/// Leading `intro`s open universal binders or move implication antecedents into the hypotheses.
/// Universal variables retain their de Bruijn indices in the open goal. Lifting the previous
/// hypotheses makes room for index zero without capturing a free variable or consulting a global
/// name. The final closer must verify the residual proposition under those hypotheses.
fn eval_script_names(ctx: &Context, names: &[String], hyps: &[Term], goal: &Term) -> bool {
    match names.split_first() {
        None => false, // empty script proves nothing
        Some((first, rest)) if first == "intro" => {
            let Ok(ng) = normalize_checked(ctx, goal) else {
                return false;
            };
            if let Some(body) = extract_forall_body(&ng) {
                let Some(lifted_hyps) = hyps
                    .iter()
                    .map(|hyp| try_syn_lift_reduce(ctx, 1, 0, hyp))
                    .collect::<Option<Vec<_>>>()
                else {
                    return false;
                };
                return eval_script_names(ctx, rest, &lifted_hyps, &body);
            }
            match as_implication(&ng) {
                Some((p, q)) => {
                    let mut h2 = hyps.to_vec();
                    h2.push(normalize(ctx, &p));
                    eval_script_names(ctx, rest, &h2, &q)
                }
                None => false, // no universal or implication binder to introduce
            }
        }
        Some((closer, rest)) if rest.is_empty() => close_under_hyps(ctx, closer, hyps, goal),
        _ => false, // a non-`intro` step before the final closer
    }
}

/// A `DAutoSolve goal` derivation over the normalized goal — the shared proof object for the
/// decision-procedure tactics (`decide`, `split`, `left`, `right`), verified by `try_dauto_solve_conclude`.
fn dauto_solve(ctx: &Context, goal: &Term) -> Term {
    Term::App(
        Box::new(Term::Global("DAutoSolve".to_string())),
        Box::new(normalize(ctx, goal)),
    )
}

/// The `decide` tactic: prove a decidable proposition by reflection + computation. Succeeds exactly
/// when [`decide_prop`] returns `Some(true)`. The derivation reuses `DAutoSolve` (whose conclusion
/// re-runs the same decision through [`auto_prop_check`]), so a false or undecided goal yields an
/// error derivation that the soundness gate rejects.
fn try_try_decide_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    if decide_prop(ctx, goal) == Some(true) {
        Some(dauto_solve(ctx, goal))
    } else {
        Some(make_error_derivation())
    }
}

/// Whether `goal` follows from the hypotheses by →-introduction and ASSUMPTION alone: introduce each
/// antecedent of an implication chain, then require the conclusion to be (syntactically) one of the
/// gathered hypotheses. Directed — it does NOT invoke the decision procedure, so `assumption` proves
/// exactly the goals that close by a hypothesis (`P -> P`, `P -> Q -> P`).
fn prove_by_assumption(ctx: &Context, hyps: &[Term], goal: &Term) -> bool {
    let ng = normalize(ctx, goal);
    if hyps.iter().any(|h| syntax_equal(h, &ng)) {
        return true;
    }
    if let Some((p, q)) = as_implication(&ng) {
        let mut h2 = hyps.to_vec();
        h2.push(normalize(ctx, &p));
        return prove_by_assumption(ctx, &h2, &q);
    }
    false
}

/// The `assumption` tactic: close the goal with a hypothesis (after introducing implication
/// antecedents). Reuses the `DAutoSolve` proof object.
fn try_try_assumption_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    if prove_by_assumption(ctx, &[], goal) {
        Some(dauto_solve(ctx, goal))
    } else {
        Some(make_error_derivation())
    }
}

/// The `split` tactic: the ∧-introduction rule — prove a conjunction `And P Q` by proving BOTH
/// conjuncts. Fails on a non-conjunction (directed, unlike `auto`).
fn try_try_split_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    let ng = normalize(ctx, goal);
    if let Some((p, q)) = extract_sapp2(&ng, "And") {
        if decide_prop(ctx, &p) == Some(true) && decide_prop(ctx, &q) == Some(true) {
            return Some(dauto_solve(ctx, &ng));
        }
    }
    Some(make_error_derivation())
}

/// The `left` tactic: the ∨-introduction-left rule — prove a disjunction `Or P Q` by proving the
/// LEFT disjunct `P`. Fails if `P` is not provable, even when the whole `Or` is (that is `right`).
fn try_try_left_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    let ng = normalize(ctx, goal);
    if let Some((p, _q)) = extract_sapp2(&ng, "Or") {
        if decide_prop(ctx, &p) == Some(true) {
            return Some(dauto_solve(ctx, &ng));
        }
    }
    Some(make_error_derivation())
}

/// The `right` tactic: the ∨-introduction-right rule — prove `Or P Q` by proving the RIGHT disjunct `Q`.
fn try_try_right_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    let ng = normalize(ctx, goal);
    if let Some((_p, q)) = extract_sapp2(&ng, "Or") {
        if decide_prop(ctx, &q) == Some(true) {
            return Some(dauto_solve(ctx, &ng));
        }
    }
    Some(make_error_derivation())
}

/// Verify DBitblastSolve proof by re-evaluating via syn_eval.
fn try_dbitblast_solve_conclude(ctx: &Context, goal: &Term) -> Option<Term> {
    let Some(norm_goal) = goal_to_certify(ctx, goal) else {
        return Some(make_sname_error());
    };
    if let Some((type_s, left, right)) = extract_eq_syntax_parts(&norm_goal) {
        if is_bit_type(&type_s) {
            let fuel = 1000;
            let left_eval = try_syn_eval_reduce(ctx, fuel, &left)?;
            let right_eval = try_syn_eval_reduce(ctx, fuel, &right)?;
            if syntax_equal(&left_eval, &right_eval) {
                return Some(norm_goal);
            }
        }
    }
    None
}

/// Verify DTabulateSolve proof by re-running exhaustive enumeration.
fn try_dtabulate_solve_conclude(ctx: &Context, goal: &Term) -> Option<Term> {
    let Some(norm_goal) = goal_to_certify(ctx, goal) else {
        return Some(make_sname_error());
    };
    // Re-verify: the goal must be a provable Pi-over-Bit
    if let Some((param_type, _)) = extract_spi(&norm_goal) {
        if is_bit_type(&param_type) {
            if let Some(result) = try_try_tabulate_reduce(ctx, &norm_goal) {
                if !is_error_derivation(&result) {
                    return Some(norm_goal);
                }
            }
        }
    }
    None
}

/// Verify DHwAutoSolve proof by re-running hw tactic chain.
fn try_dhw_auto_solve_conclude(ctx: &Context, goal: &Term) -> Option<Term> {
    // Try bitblast conclude first
    if let Some(result) = try_dbitblast_solve_conclude(ctx, goal) {
        return Some(result);
    }
    // Fall back to auto conclude
    try_dauto_solve_conclude(ctx, goal)
}

/// Hardware auto: tries bitblast, then tabulate, then falls back to auto.
fn try_try_hw_auto_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Try bitblast first (fast, for concrete Bit equalities)
    if let Some(result) = try_try_bitblast_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(result);
        }
    }

    // Try tabulate (for universal Bit quantification)
    if let Some(result) = try_try_tabulate_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(result);
        }
    }

    // Fall back to auto
    try_try_auto_reduce(ctx, &norm_goal)
}

fn try_try_auto_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Handle trivial cases: True and False
    // SName "True" is trivially provable
    if let Term::App(ctor, inner) = &norm_goal {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SName" {
                if let Term::Lit(Literal::Text(s)) = inner.as_ref() {
                    if s == "True" {
                        // True is always provable - use DAutoSolve
                        return Some(Term::App(
                            Box::new(Term::Global("DAutoSolve".to_string())),
                            Box::new(norm_goal),
                        ));
                    }
                    if s == "False" {
                        // False is never provable
                        return Some(make_error_derivation());
                    }
                }
            }
        }
    }

    // Try simp (handles equalities with simplification)
    if let Some(result) = try_try_simp_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(result);
        }
    }

    // Try ring (polynomial equalities)
    if let Some(result) = try_try_ring_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(result);
        }
    }

    // Try cc (congruence closure)
    if let Some(result) = try_try_cc_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(result);
        }
    }

    // Try omega (integer arithmetic - most precise)
    if let Some(result) = try_try_omega_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(result);
        }
    }

    // Try lia (linear arithmetic - fallback)
    if let Some(result) = try_try_lia_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(result);
        }
    }

    // Try registered hints
    for hint_name in ctx.get_hints() {
        if let Some(hint_type) = ctx.get_global(hint_name) {
            if let Some(result) = try_apply_hint(ctx, hint_type, &norm_goal) {
                return Some(result);
            }
        }
    }

    // Propositional structure (And / Or / implies over decidable atoms) — the flat tactic list
    // above only decides atomic goals.
    if auto_prop_check(ctx, &norm_goal) {
        return Some(Term::App(
            Box::new(Term::Global("DAutoSolve".to_string())),
            Box::new(norm_goal),
        ));
    }

    // All tactics failed
    Some(make_error_derivation())
}

/// Verify DAutoSolve proof by re-running tactic search.
fn try_dauto_solve_conclude(ctx: &Context, goal: &Term) -> Option<Term> {
    let Some(norm_goal) = goal_to_certify(ctx, goal) else {
        return Some(make_sname_error());
    };

    // Handle trivial cases: True
    if let Term::App(ctor, inner) = &norm_goal {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SName" {
                if let Term::Lit(Literal::Text(s)) = inner.as_ref() {
                    if s == "True" {
                        return Some(norm_goal.clone());
                    }
                }
            }
        }
    }

    // Try each tactic - if any succeeds, the proof is valid
    if let Some(result) = try_try_simp_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(norm_goal.clone());
        }
    }
    if let Some(result) = try_try_ring_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(norm_goal.clone());
        }
    }
    if let Some(result) = try_try_cc_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(norm_goal.clone());
        }
    }
    if let Some(result) = try_try_omega_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(norm_goal.clone());
        }
    }
    if let Some(result) = try_try_lia_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(norm_goal.clone());
        }
    }

    // Try registered hints
    for hint_name in ctx.get_hints() {
        if let Some(hint_type) = ctx.get_global(hint_name) {
            if try_apply_hint(ctx, hint_type, &norm_goal).is_some() {
                return Some(norm_goal);
            }
        }
    }

    // Propositional structure — must mirror the reduce path exactly, or the soundness gate rejects.
    if auto_prop_check(ctx, &norm_goal) {
        return Some(norm_goal);
    }

    Some(make_sname_error())
}

// =============================================================================
// GENERIC INDUCTION HELPERS (induction_num_cases, induction_base_goal, etc.)
// =============================================================================

/// Returns the number of constructors for an inductive type.
///
/// induction_num_cases (SName "Nat") → Succ (Succ Zero) = 2
/// induction_num_cases (SName "Bool") → Succ (Succ Zero) = 2
/// induction_num_cases (SApp (SName "List") A) → Succ (Succ Zero) = 2
fn try_induction_num_cases_reduce(ctx: &Context, ind_type: &Term) -> Option<Term> {
    // Extract inductive name from Syntax
    let ind_name = match extract_inductive_name_from_syntax(ind_type) {
        Some(name) => name,
        None => {
            // Not a valid inductive type syntax
            return Some(Term::Global("Zero".to_string()));
        }
    };

    // Look up constructors
    let constructors = ctx.get_constructors(&ind_name);
    let num_ctors = constructors.len();

    // Build Nat representation: Succ (Succ (... Zero))
    let mut result = Term::Global("Zero".to_string());
    for _ in 0..num_ctors {
        result = Term::App(Box::new(Term::Global("Succ".to_string())), Box::new(result));
    }

    Some(result)
}

/// Returns the base case goal for induction (first constructor).
///
/// Given ind_type and motive (SLam T body), returns motive[ctor0/var].
fn try_induction_base_goal_reduce(ctx: &Context, ind_type: &Term, motive: &Term) -> Option<Term> {
    // Extract inductive name
    let ind_name = match extract_inductive_name_from_syntax(ind_type) {
        Some(name) => name,
        None => return Some(make_sname_error()),
    };

    // Get constructors
    let constructors = ctx.get_constructors(&ind_name);
    if constructors.is_empty() {
        return Some(make_sname_error());
    }

    // Extract motive body
    let motive_body = match extract_slam_body(motive) {
        Some(body) => body,
        None => return Some(make_sname_error()),
    };

    // Build goal for first constructor (base case)
    let (ctor_name, _) = constructors[0];
    build_case_expected(ctx, ctor_name, &constructors, &motive_body, ind_type, true)
}

/// Returns the goal for a specific constructor index.
///
/// Given ind_type, motive, and constructor index (as Nat), returns the case goal.
fn try_induction_step_goal_reduce(
    ctx: &Context,
    ind_type: &Term,
    motive: &Term,
    ctor_idx: &Term,
) -> Option<Term> {
    // Extract inductive name
    let ind_name = match extract_inductive_name_from_syntax(ind_type) {
        Some(name) => name,
        None => return Some(make_sname_error()),
    };

    // Get constructors
    let constructors = ctx.get_constructors(&ind_name);
    if constructors.is_empty() {
        return Some(make_sname_error());
    }

    // Convert Nat to index
    let idx = nat_to_usize(ctor_idx)?;
    if idx >= constructors.len() {
        return Some(make_sname_error());
    }

    // Extract motive body
    let motive_body = match extract_slam_body(motive) {
        Some(body) => body,
        None => return Some(make_sname_error()),
    };

    // Build goal for the specified constructor
    let (ctor_name, _) = constructors[idx];
    build_case_expected(ctx, ctor_name, &constructors, &motive_body, ind_type, true)
}

/// Convert a Nat term to usize.
///
/// Zero → 0
/// Succ Zero → 1
/// Succ (Succ Zero) → 2
fn nat_to_usize(term: &Term) -> Option<usize> {
    match term {
        Term::Global(name) if name == "Zero" => Some(0),
        Term::App(succ, inner) => {
            if let Term::Global(name) = succ.as_ref() {
                if name == "Succ" {
                    return nat_to_usize(inner).map(|n| n + 1);
                }
            }
            None
        }
        _ => None,
    }
}

/// Generic induction tactic.
///
/// try_induction ind_type motive cases → DElim ind_type motive cases
///
/// Delegates to existing DElim infrastructure after basic validation.
fn try_try_induction_reduce(
    ctx: &Context,
    ind_type: &Term,
    motive: &Term,
    cases: &Term,
) -> Option<Term> {
    // Extract inductive name to validate
    let ind_name = match extract_inductive_name_from_syntax(ind_type) {
        Some(name) => name,
        None => return Some(make_error_derivation()),
    };

    // Get constructors to validate count
    let constructors = ctx.get_constructors(&ind_name);
    if constructors.is_empty() {
        return Some(make_error_derivation());
    }

    // Extract case proofs to validate count
    let case_proofs = match extract_case_proofs(cases) {
        Some(proofs) => proofs,
        None => return Some(make_error_derivation()),
    };

    // Verify case count matches constructor count
    if case_proofs.len() != constructors.len() {
        return Some(make_error_derivation());
    }

    // Build DElim term (delegates verification to existing infrastructure)
    Some(Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("DElim".to_string())),
                Box::new(ind_type.clone()),
            )),
            Box::new(motive.clone()),
        )),
        Box::new(cases.clone()),
    ))
}

// -------------------------------------------------------------------------
// Deep Induction
// -------------------------------------------------------------------------

/// DInduction reduction with verification.
///
/// DInduction motive base step → Forall Nat motive (if verified)
///
/// Verification:
/// 1. Extract motive body from SLam Nat body
/// 2. Check that concludes(base) = motive[Zero/0]
/// 3. Check that concludes(step) = ∀k:Nat. P(k) → P(Succ k)
/// 4. If all checks pass, return Forall Nat motive
/// 5. Otherwise, return Error
fn try_dinduction_reduce(ctx: &Context, motive: &Term, base: &Term, step: &Term) -> Option<Term> {
    // Normalize all inputs
    let norm_motive = normalize(ctx, motive);
    let norm_base = normalize(ctx, base);
    let norm_step = normalize(ctx, step);

    // 1. Extract motive body (should be SLam (SName "Nat") body)
    let motive_body = match extract_slam_body(&norm_motive) {
        Some(body) => body,
        None => return Some(make_sname_error()),
    };

    // 2. Compute expected base: motive body with Zero substituted for SVar 0
    let zero = make_sname("Zero");
    let expected_base = match try_syn_subst_reduce(ctx, &zero, 0, &motive_body) {
        Some(b) => b,
        None => return Some(make_sname_error()),
    };

    // 3. Get actual base conclusion
    let base_conc = match try_concludes_reduce(ctx, &norm_base) {
        Some(c) => c,
        None => return Some(make_sname_error()),
    };

    // 4. Verify base matches expected
    if !syntax_equal(&base_conc, &expected_base) {
        return Some(make_sname_error());
    }

    // 5. Build expected step formula: ∀k:Nat. P(k) → P(Succ k)
    let expected_step = match build_induction_step_formula(ctx, &motive_body) {
        Some(s) => s,
        None => return Some(make_sname_error()),
    };

    // 6. Get actual step conclusion
    let step_conc = match try_concludes_reduce(ctx, &norm_step) {
        Some(c) => c,
        None => return Some(make_sname_error()),
    };

    // 7. Verify step matches expected
    if !syntax_equal(&step_conc, &expected_step) {
        return Some(make_sname_error());
    }

    // 8. Return conclusion: Forall Nat motive
    Some(make_forall_nat_syntax(&norm_motive))
}

/// Build step formula: ∀k:Nat. P(k) → P(Succ k)
///
/// Given motive body P (which uses SVar 0 for k), builds:
/// Forall (SName "Nat") (SLam (SName "Nat") (Implies P P[Succ(SVar 0)/SVar 0]))
fn build_induction_step_formula(ctx: &Context, motive_body: &Term) -> Option<Term> {
    // P(k) = motive_body (uses SVar 0 for k)
    let p_k = motive_body.clone();

    // P(Succ k) = motive_body with (SApp (SName "Succ") (SVar 0)) substituted
    let succ_var = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(make_sname("Succ")),
        )),
        Box::new(Term::App(
            Box::new(Term::Global("SVar".to_string())),
            Box::new(Term::Lit(Literal::Int(0))),
        )),
    );
    let p_succ_k = try_syn_subst_reduce(ctx, &succ_var, 0, motive_body)?;

    // Implies P(k) P(Succ k)
    let implies_body = make_implies_syntax(&p_k, &p_succ_k);

    // SLam (SName "Nat") implies_body
    let slam = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SLam".to_string())),
            Box::new(make_sname("Nat")),
        )),
        Box::new(implies_body),
    );

    // Forall (SName "Nat") slam
    Some(make_forall_syntax_with_type(&make_sname("Nat"), &slam))
}

/// Build SApp (SApp (SName "Implies") a) b
fn make_implies_syntax(a: &Term, b: &Term) -> Term {
    // SApp (SName "Implies") a
    let app1 = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(make_sname("Implies")),
        )),
        Box::new(a.clone()),
    );

    // SApp (SApp (SName "Implies") a) b
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(app1),
        )),
        Box::new(b.clone()),
    )
}

/// Build SApp (SApp (SName "Forall") (SName "Nat")) motive
fn make_forall_nat_syntax(motive: &Term) -> Term {
    make_forall_syntax_with_type(&make_sname("Nat"), motive)
}

/// Build SApp (SApp (SName "Forall") type_s) body
fn make_forall_syntax_with_type(type_s: &Term, body: &Term) -> Term {
    // SApp (SName "Forall") type_s
    let app1 = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(make_sname("Forall")),
        )),
        Box::new(type_s.clone()),
    );

    // SApp (SApp (SName "Forall") type_s) body
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(app1),
        )),
        Box::new(body.clone()),
    )
}

// -------------------------------------------------------------------------
// Solver (Computational Reflection)
// -------------------------------------------------------------------------

/// DCompute reduction with verification.
///
/// DCompute goal → goal (if verified by computation)
///
/// Verification:
/// 1. Check that goal is (Eq T A B) as Syntax
/// 2. Evaluate A and B using syn_eval with bounded fuel
/// 3. If eval(A) == eval(B), return goal
/// 4. Otherwise, return Error
fn try_dcompute_conclude(ctx: &Context, goal: &Term) -> Option<Term> {
    let Some(norm_goal) = goal_to_certify(ctx, goal) else {
        return Some(make_sname_error());
    };

    // Extract T, A, B from Eq T A B (as Syntax)
    // Pattern: SApp (SApp (SApp (SName "Eq") T) A) B
    let parts = extract_eq_syntax_parts(&norm_goal);
    if parts.is_none() {
        // Goal is not an equality - return Error
        return Some(make_sname_error());
    }
    let (_, a, b) = parts.unwrap();

    // Evaluate A and B, and INSIST both evaluations finished. This door accepts proofs: two
    // reductions abandoned at the same place compare equal and certify nothing, and the caller
    // of `try_syn_eval_reduce` cannot tell that from success.
    //
    // The budget stays at the original 1000, MEASURED rather than guessed: raising it to 1<<16
    // cost `tactic_qa_adversarial`'s two batteries 2.0-2.4× (3.7s→9.1s, 4.9s→10.1s) and bought
    // nothing, because `syn_eval_checked` stops the instant the reflected term stops CHANGING —
    // the only way to spend the budget is a genuinely diverging reflected reduction, and a
    // ground arithmetic equality (all `DCompute` is ever asked) finishes in a handful of steps.
    // 1000 is already two orders of magnitude of headroom.
    const DCOMPUTE_BUDGET: i64 = 1000;
    let (Ok(a_eval), Ok(b_eval)) = (
        syn_eval_checked(ctx, DCOMPUTE_BUDGET, &a),
        syn_eval_checked(ctx, DCOMPUTE_BUDGET, &b),
    ) else {
        return Some(make_sname_error());
    };

    // Compare normalized results
    if syntax_equal(&a_eval, &b_eval) {
        Some(norm_goal)
    } else {
        Some(make_sname_error())
    }
}

/// Extract T, A, B from Eq T A B (as Syntax)
///
/// Pattern: SApp (SApp (SApp (SName "Eq") T) A) B
fn extract_eq_syntax_parts(term: &Term) -> Option<(Term, Term, Term)> {
    // term = SApp X B where X = SApp Y A where Y = SApp (SName "Eq") T
    // Structure: App(App(SApp, App(App(SApp, App(App(SApp, App(SName, "Eq")), T)), A)), B)
    if let Term::App(partial2, b) = term {
        if let Term::App(sapp2, inner2) = partial2.as_ref() {
            if let Term::Global(sapp2_name) = sapp2.as_ref() {
                if sapp2_name != "SApp" {
                    return None;
                }
            } else {
                return None;
            }

            if let Term::App(partial1, a) = inner2.as_ref() {
                if let Term::App(sapp1, inner1) = partial1.as_ref() {
                    if let Term::Global(sapp1_name) = sapp1.as_ref() {
                        if sapp1_name != "SApp" {
                            return None;
                        }
                    } else {
                        return None;
                    }

                    if let Term::App(eq_t, t) = inner1.as_ref() {
                        if let Term::App(sapp0, eq_sname) = eq_t.as_ref() {
                            if let Term::Global(sapp0_name) = sapp0.as_ref() {
                                if sapp0_name != "SApp" {
                                    return None;
                                }
                            } else {
                                return None;
                            }

                            // Check if eq_sname is SName "Eq"
                            if let Term::App(sname_ctor, eq_str) = eq_sname.as_ref() {
                                if let Term::Global(ctor) = sname_ctor.as_ref() {
                                    if ctor == "SName" {
                                        if let Term::Lit(Literal::Text(s)) = eq_str.as_ref() {
                                            if s == "Eq" {
                                                return Some((
                                                    t.as_ref().clone(),
                                                    a.as_ref().clone(),
                                                    b.as_ref().clone(),
                                                ));
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
    None
}

// -------------------------------------------------------------------------
// Tactic Combinators
// -------------------------------------------------------------------------

/// Reduce tact_orelse t1 t2 goal
///
/// - Apply t1 to goal
/// - If concludes returns Error, apply t2 to goal
/// - Otherwise return t1's result
fn try_tact_orelse_reduce(ctx: &Context, t1: &Term, t2: &Term, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Apply t1 to goal
    let d1_app = Term::App(Box::new(t1.clone()), Box::new(norm_goal.clone()));
    let d1 = normalize(ctx, &d1_app);

    // Check if t1 succeeded by looking at concludes
    if let Some(conc1) = try_concludes_reduce(ctx, &d1) {
        if is_error_syntax(&conc1) {
            // t1 failed, try t2
            let d2_app = Term::App(Box::new(t2.clone()), Box::new(norm_goal));
            return Some(normalize(ctx, &d2_app));
        } else {
            // t1 succeeded
            return Some(d1);
        }
    }

    // Couldn't evaluate concludes - return error
    Some(make_error_derivation())
}

/// Check if a Syntax term is SName "Error"
fn is_error_syntax(term: &Term) -> bool {
    // Pattern: SName "Error" = App(Global("SName"), Lit(Text("Error")))
    if let Term::App(ctor, arg) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SName" {
                if let Term::Lit(Literal::Text(s)) = arg.as_ref() {
                    return s == "Error";
                }
            }
        }
    }
    false
}

/// Reduce tact_try t goal
///
/// - Apply t to goal
/// - If concludes returns Error, return identity (DAxiom goal)
/// - Otherwise return t's result
fn try_tact_try_reduce(ctx: &Context, t: &Term, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Apply t to goal
    let d_app = Term::App(Box::new(t.clone()), Box::new(norm_goal.clone()));
    let d = normalize(ctx, &d_app);

    // Check if t succeeded by looking at concludes
    if let Some(conc) = try_concludes_reduce(ctx, &d) {
        if is_error_syntax(&conc) {
            // t failed, return identity (DAxiom goal)
            return Some(make_daxiom(&norm_goal));
        } else {
            // t succeeded
            return Some(d);
        }
    }

    // Couldn't evaluate concludes - return identity
    Some(make_daxiom(&norm_goal))
}

/// Reduce tact_repeat t goal
///
/// Apply t repeatedly until it fails or makes no progress.
/// Returns the accumulated derivation or identity if first application fails.
fn try_tact_repeat_reduce(ctx: &Context, t: &Term, goal: &Term) -> Option<Term> {
    const MAX_ITERATIONS: usize = 100;

    let norm_goal = normalize(ctx, goal);
    let mut current_goal = norm_goal.clone();
    let mut last_successful_deriv: Option<Term> = None;

    for _ in 0..MAX_ITERATIONS {
        // Apply t to current goal
        let d_app = Term::App(Box::new(t.clone()), Box::new(current_goal.clone()));
        let d = normalize(ctx, &d_app);

        // Check result
        if let Some(conc) = try_concludes_reduce(ctx, &d) {
            if is_error_syntax(&conc) {
                // Tactic failed - stop and return what we have
                break;
            }

            // Check for no-progress (fixed point)
            if syntax_equal(&conc, &current_goal) {
                // No progress made - stop
                break;
            }

            // Progress made - continue
            current_goal = conc;
            last_successful_deriv = Some(d);
        } else {
            // Couldn't evaluate concludes - stop
            break;
        }
    }

    // Return final derivation or identity
    last_successful_deriv.or_else(|| Some(make_daxiom(&norm_goal)))
}

/// Reduce tact_then t1 t2 goal
///
/// - Apply t1 to goal
/// - If t1 fails, return Error
/// - Otherwise apply t2 to the result of t1
fn try_tact_then_reduce(ctx: &Context, t1: &Term, t2: &Term, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Apply t1 to goal
    let d1_app = Term::App(Box::new(t1.clone()), Box::new(norm_goal.clone()));
    let d1 = normalize(ctx, &d1_app);

    // Check if t1 succeeded
    if let Some(conc1) = try_concludes_reduce(ctx, &d1) {
        if is_error_syntax(&conc1) {
            // t1 failed
            return Some(make_error_derivation());
        }

        // t1 succeeded - apply t2 to the new goal (conc1)
        let d2_app = Term::App(Box::new(t2.clone()), Box::new(conc1));
        let d2 = normalize(ctx, &d2_app);

        // The result is d2 (which may succeed or fail)
        return Some(d2);
    }

    // Couldn't evaluate concludes - return error
    Some(make_error_derivation())
}

/// Reduce tact_first tactics goal
///
/// Try each tactic in the list until one succeeds.
/// Returns Error if all fail or list is empty.
fn try_tact_first_reduce(ctx: &Context, tactics: &Term, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Extract tactics from TList
    let tactic_vec = extract_tlist(tactics)?;

    for tactic in tactic_vec {
        // Apply this tactic to goal
        let d_app = Term::App(Box::new(tactic), Box::new(norm_goal.clone()));
        let d = normalize(ctx, &d_app);

        // Check if it succeeded
        if let Some(conc) = try_concludes_reduce(ctx, &d) {
            if !is_error_syntax(&conc) {
                // Success!
                return Some(d);
            }
            // Failed - try next
        }
    }

    // All failed
    Some(make_error_derivation())
}

/// Extract elements from a TList term
///
/// TList is polymorphic: TNil A and TCons A h t
/// So the structure is:
/// - TNil A = App(Global("TNil"), type)
/// - TCons A h t = App(App(App(Global("TCons"), type), head), tail)
fn extract_tlist(term: &Term) -> Option<Vec<Term>> {
    let mut result = Vec::new();
    let mut current = term.clone();

    loop {
        match &current {
            // TNil A = App(Global("TNil"), type)
            Term::App(tnil, _type) => {
                if let Term::Global(name) = tnil.as_ref() {
                    if name == "TNil" {
                        // Empty list
                        break;
                    }
                }
                // Try TCons A h t = App(App(App(Global("TCons"), type), head), tail)
                if let Term::App(partial2, tail) = &current {
                    if let Term::App(partial1, head) = partial2.as_ref() {
                        if let Term::App(tcons, _type) = partial1.as_ref() {
                            if let Term::Global(name) = tcons.as_ref() {
                                if name == "TCons" {
                                    result.push(head.as_ref().clone());
                                    current = tail.as_ref().clone();
                                    continue;
                                }
                            }
                        }
                    }
                }
                // Not a valid TList structure
                return None;
            }
            // Bare Global("TNil") without type argument - also valid
            Term::Global(name) if name == "TNil" => {
                break;
            }
            _ => {
                // Not a valid TList
                return None;
            }
        }
    }

    Some(result)
}

/// Reduce tact_solve t goal
///
/// - Apply t to goal
/// - If t fails (Error), return Error
/// - If t succeeds, return its result
fn try_tact_solve_reduce(ctx: &Context, t: &Term, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Apply t to goal
    let d_app = Term::App(Box::new(t.clone()), Box::new(norm_goal.clone()));
    let d = normalize(ctx, &d_app);

    // Check if t succeeded
    if let Some(conc) = try_concludes_reduce(ctx, &d) {
        if is_error_syntax(&conc) {
            // t failed
            return Some(make_error_derivation());
        }
        // t succeeded - return its derivation
        return Some(d);
    }

    // Couldn't evaluate concludes - return error
    Some(make_error_derivation())
}

// -------------------------------------------------------------------------
// Congruence Closure
// -------------------------------------------------------------------------

/// Validate DCong proof by congruence.
///
/// DCong context eq_proof where:
/// - context is SLam param_type body
/// - eq_proof proves Eq T a b
/// Returns: Eq param_type (body[0:=a]) (body[0:=b])
fn try_dcong_conclude(ctx: &Context, context: &Term, eq_proof: &Term) -> Option<Term> {
    // Get the conclusion of the equality proof
    let eq_conc = try_concludes_reduce(ctx, eq_proof)?;

    // Extract T, a, b from Eq T a b
    let parts = extract_eq_syntax_parts(&eq_conc);
    if parts.is_none() {
        // Not an equality proof
        return Some(make_sname_error());
    }
    let (_type_term, lhs, rhs) = parts.unwrap();

    // Normalize context and check it's a lambda
    let norm_context = normalize(ctx, context);

    // Extract (param_type, body) from SLam param_type body
    let slam_parts = extract_slam_parts(&norm_context);
    if slam_parts.is_none() {
        // Not a lambda context
        return Some(make_sname_error());
    }
    let (param_type, body) = slam_parts.unwrap();

    // Substitute lhs and rhs into body at index 0
    let fa = try_syn_subst_reduce(ctx, &lhs, 0, &body)?;
    let fb = try_syn_subst_reduce(ctx, &rhs, 0, &body)?;

    // Build result: Eq param_type fa fb
    Some(make_eq_syntax_three(&param_type, &fa, &fb))
}

/// Extract (param_type, body) from SLam param_type body
///
/// Pattern: App(App(Global("SLam"), param_type), body)
fn extract_slam_parts(term: &Term) -> Option<(Term, Term)> {
    if let Term::App(inner, body) = term {
        if let Term::App(slam_ctor, param_type) = inner.as_ref() {
            if let Term::Global(name) = slam_ctor.as_ref() {
                if name == "SLam" {
                    return Some((param_type.as_ref().clone(), body.as_ref().clone()));
                }
            }
        }
    }
    None
}

/// Build SApp (SApp (SApp (SName "Eq") type_s) a) b
///
/// Constructs the Syntax representation of (Eq type_s a b)
fn make_eq_syntax_three(type_s: &Term, a: &Term, b: &Term) -> Term {
    let eq_name = Term::App(
        Box::new(Term::Global("SName".to_string())),
        Box::new(Term::Lit(Literal::Text("Eq".to_string()))),
    );

    // SApp (SName "Eq") type_s
    let app1 = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(eq_name),
        )),
        Box::new(type_s.clone()),
    );

    // SApp (SApp (SName "Eq") type_s) a
    let app2 = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(app1),
        )),
        Box::new(a.clone()),
    );

    // SApp (SApp (SApp (SName "Eq") type_s) a) b
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(app2),
        )),
        Box::new(b.clone()),
    )
}

// -------------------------------------------------------------------------
// Generic Elimination
// -------------------------------------------------------------------------

/// DElim reduction with verification.
///
/// DElim ind_type motive cases → Forall ind_type motive (if verified)
///
/// Verification:
/// 1. Extract inductive name from ind_type Syntax
/// 2. Look up constructors for that inductive
/// 3. Extract case proofs from DCase chain
/// 4. Verify case count matches constructor count
/// 5. For each constructor, verify case conclusion matches expected
/// 6. Return Forall ind_type motive
fn try_delim_conclude(ctx: &Context, ind_type: &Term, motive: &Term, cases: &Term) -> Option<Term> {
    // Normalize inputs
    let norm_ind_type = normalize(ctx, ind_type);
    let norm_motive = normalize(ctx, motive);
    let norm_cases = normalize(ctx, cases);

    // 1. Extract inductive name from Syntax
    let ind_name = match extract_inductive_name_from_syntax(&norm_ind_type) {
        Some(name) => name,
        None => return Some(make_sname_error()),
    };

    // 2. Look up constructors for this inductive
    let constructors = ctx.get_constructors(&ind_name);
    if constructors.is_empty() {
        // Unknown inductive type
        return Some(make_sname_error());
    }

    // 3. Extract case proofs from DCase chain
    let case_proofs = match extract_case_proofs(&norm_cases) {
        Some(proofs) => proofs,
        None => return Some(make_sname_error()),
    };

    // 4. Verify case count matches constructor count
    if case_proofs.len() != constructors.len() {
        return Some(make_sname_error());
    }

    // 5. Extract motive body (should be SLam param_type body)
    let motive_body = match extract_slam_body(&norm_motive) {
        Some(body) => body,
        None => return Some(make_sname_error()),
    };

    // 6. For each constructor, verify case conclusion matches expected
    for (i, (ctor_name, _ctor_type)) in constructors.iter().enumerate() {
        let case_proof = &case_proofs[i];

        // Get actual conclusion of this case proof
        let case_conc = match try_concludes_reduce(ctx, case_proof) {
            Some(c) => c,
            None => return Some(make_sname_error()),
        };

        // Build expected conclusion based on constructor
        // For base case (0-ary constructor): motive[ctor/var]
        // For step case (recursive constructor): requires IH pattern
        let expected = match build_case_expected(
            ctx,
            ctor_name,
            &constructors,
            &motive_body,
            &norm_ind_type,
            true,
        ) {
            Some(e) => e,
            None => return Some(make_sname_error()),
        };

        // Verify conclusion matches expected
        if !syntax_equal(&case_conc, &expected) {
            return Some(make_sname_error());
        }
    }

    // 7. Return conclusion: Forall ind_type motive
    Some(make_forall_syntax_generic(&norm_ind_type, &norm_motive))
}

/// Extract inductive name from Syntax term.
///
/// Handles:
/// - SName "Nat" → "Nat"
/// - SApp (SName "List") A → "List"
fn extract_inductive_name_from_syntax(term: &Term) -> Option<String> {
    // Case 1: SName "X"
    if let Term::App(sname, text) = term {
        if let Term::Global(ctor) = sname.as_ref() {
            if ctor == "SName" {
                if let Term::Lit(Literal::Text(name)) = text.as_ref() {
                    return Some(name.clone());
                }
            }
        }
    }

    // Case 2: SApp (SName "X") args → extract "X" from the function position
    if let Term::App(inner, _arg) = term {
        if let Term::App(sapp, func) = inner.as_ref() {
            if let Term::Global(ctor) = sapp.as_ref() {
                if ctor == "SApp" {
                    // Recursively extract from the function
                    return extract_inductive_name_from_syntax(func);
                }
            }
        }
    }

    None
}

/// Extract case proofs from DCase chain.
///
/// DCase p1 (DCase p2 DCaseEnd) → [p1, p2]
fn extract_case_proofs(term: &Term) -> Option<Vec<Term>> {
    let mut proofs = Vec::new();
    let mut current = term;

    loop {
        // DCaseEnd - end of list
        if let Term::Global(name) = current {
            if name == "DCaseEnd" {
                return Some(proofs);
            }
        }

        // DCase head tail - Pattern: App(App(DCase, head), tail)
        if let Term::App(inner, tail) = current {
            if let Term::App(dcase, head) = inner.as_ref() {
                if let Term::Global(name) = dcase.as_ref() {
                    if name == "DCase" {
                        proofs.push(head.as_ref().clone());
                        current = tail.as_ref();
                        continue;
                    }
                }
            }
        }

        // Unrecognized structure
        return None;
    }
}

/// THE CASE FORMULA for one constructor — the proposition a bullet must prove.
///
/// For `C : Π(f₁:T₁) … (fₙ:Tₙ). Ind` the formula is
///
/// ```text
///   ∀f₁ … ∀fₙ.  (IH for each RECURSIVE fᵢ) →  P (C f₁ … fₙ)
/// ```
///
/// and the induction hypotheses are included only when `include_ih` — `induction` has them,
/// `destruct`/`cases` does not, and that is the ENTIRE difference between the two eliminators.
///
/// **ONE BUILDER, NOT THREE.** This used to branch three ways, and two of the branches were
/// wrong in the same direction — they checked a WEAKER proposition than the eliminator then
/// conceded:
///
/// * a hard-coded `ind_name == "Nat"` arm, which is `build_recursive_case_formula` written a
///   second time and therefore a second place for the formula to drift;
/// * a `is_recursive_constructor` gate whose false branch substituted the BARE constructor —
///   no field arguments and no quantifier. For `A Tag is either Zed or a Val with n: Nat.`
///   the `Val` case was checked as `P Val`, a proposition about a partially applied
///   constructor, while `DElim` went on to conclude `∀t:Tag, P t` — which includes every
///   `P (Val n)` that was never proved. `∀t:Tag, t = t` with two `refl` bullets certified,
///   while the same shape over a RECURSIVE constructor was correctly refused: the two
///   answers differed only by which branch of the gate the constructor fell into.
///
/// `build_recursive_case_formula` degrades correctly when a constructor has no value fields
/// (it substitutes the applied constructor, which is the base case), so the gate bought
/// nothing it did not also get wrong.
fn build_case_expected(
    ctx: &Context,
    ctor_name: &str,
    _constructors: &[(&str, &Term)],
    motive_body: &Term,
    ind_type: &Term,
    include_ih: bool,
) -> Option<Term> {
    let ind_name = extract_inductive_name_from_syntax(ind_type)?;
    match ctx.get_global(ctor_name) {
        Some(ctor_ty) => build_recursive_case_formula(
            ctx,
            ctor_name,
            ctor_ty,
            motive_body,
            ind_type,
            &ind_name,
            include_ih,
        ),
        // A constructor the context does not declare has no fields the kernel can see, so the
        // only formula available is the one about the constructor itself.
        None => {
            let ctor_applied = apply_type_args_to_ctor(&make_sname(ctor_name), ind_type);
            try_syn_subst_reduce(ctx, &ctor_applied, 0, motive_body)
        }
    }
}

/// Apply type arguments from ind_type to a constructor.
///
/// If ind_type = SApp (SName "List") A, and ctor = SName "Nil",
/// result = SApp (SName "Nil") A
fn apply_type_args_to_ctor(ctor: &Term, ind_type: &Term) -> Term {
    // Extract type arguments from ind_type
    let args = extract_type_args(ind_type);

    if args.is_empty() {
        return ctor.clone();
    }

    // Apply each arg: SApp (... (SApp ctor arg1) ...) argN
    args.iter().fold(ctor.clone(), |acc, arg| {
        Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SApp".to_string())),
                Box::new(acc),
            )),
            Box::new(arg.clone()),
        )
    })
}

/// Extract type arguments from polymorphic Syntax.
///
/// SApp (SApp (SName "Either") A) B → [A, B]
/// SApp (SName "List") A → [A]
/// SName "Nat" → []
fn extract_type_args(term: &Term) -> Vec<Term> {
    let mut args = Vec::new();
    let mut current = term;

    // Traverse SApp chain from outside in
    loop {
        if let Term::App(inner, arg) = current {
            if let Term::App(sapp, func) = inner.as_ref() {
                if let Term::Global(ctor) = sapp.as_ref() {
                    if ctor == "SApp" {
                        args.push(arg.as_ref().clone());
                        current = func.as_ref();
                        continue;
                    }
                }
            }
        }
        break;
    }

    // Reverse because we collected outside-in but want inside-out
    args.reverse();
    args
}

/// Build Forall Syntax for generic inductive type.
///
/// Forall ind_type motive = SApp (SApp (SName "Forall") ind_type) motive
fn make_forall_syntax_generic(ind_type: &Term, motive: &Term) -> Term {
    // SApp (SName "Forall") ind_type
    let forall_type = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(make_sname("Forall")),
        )),
        Box::new(ind_type.clone()),
    );

    // SApp forall_type motive
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(forall_type),
        )),
        Box::new(motive.clone()),
    )
}

/// Build the case formula for a recursive constructor.
///
/// For Cons : Π(A:Type). A -> List A -> List A
/// with motive P : List A -> Prop
/// Expected case: ∀x:A. ∀xs:List A. P(xs) -> P(Cons A x xs)
fn build_recursive_case_formula(
    ctx: &Context,
    ctor_name: &str,
    ctor_ty: &Term,
    motive_body: &Term,
    ind_type: &Term,
    ind_name: &str,
    include_ih: bool,
) -> Option<Term> {
    // Extract type args from ind_type for matching
    let type_args = extract_type_args(ind_type);

    // Collect constructor arguments (skipping type parameters)
    let args = collect_ctor_args(ctor_ty, ind_name, &type_args);

    if args.is_empty() {
        // No non-type arguments, treat as base case
        let ctor_applied = apply_type_args_to_ctor(&make_sname(ctor_name), ind_type);
        return try_syn_subst_reduce(ctx, &ctor_applied, 0, motive_body);
    }

    // Build from inside out:
    // 1. Build ctor application: Cons A x xs (with de Bruijn indices for args)
    // 2. Build P(ctor args): motive_body[ctor args/var]
    // 3. For each recursive arg, wrap with IH: P(xs) ->
    // 4. For each arg, wrap with forall: ∀xs:List A.

    // Build constructor application with de Bruijn indices
    let mut ctor_app = apply_type_args_to_ctor(&make_sname(ctor_name), ind_type);
    for (i, _) in args.iter().enumerate() {
        // Index from end: last arg is index 0, second-to-last is 1, etc.
        let idx = (args.len() - 1 - i) as i64;
        let var = Term::App(
            Box::new(Term::Global("SVar".to_string())),
            Box::new(Term::Lit(Literal::Int(idx))),
        );
        ctor_app = Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SApp".to_string())),
                Box::new(ctor_app),
            )),
            Box::new(var),
        );
    }

    // P(ctor args) - substitute ctor_app into motive
    let p_ctor = try_syn_subst_reduce(ctx, &ctor_app, 0, motive_body)?;

    // Build implications from inside out (for recursive args)
    let mut body = p_ctor;
    for (i, (arg_ty, is_recursive)) in args.iter().enumerate().rev() {
        if *is_recursive && include_ih {
            // Add IH: P(arg) -> body
            // arg is at index (args.len() - 1 - i)
            let idx = (args.len() - 1 - i) as i64;
            let var = Term::App(
                Box::new(Term::Global("SVar".to_string())),
                Box::new(Term::Lit(Literal::Int(idx))),
            );
            let p_arg = try_syn_subst_reduce(ctx, &var, 0, motive_body)?;
            body = make_implies_syntax(&p_arg, &body);
        }
        // Skip non-recursive args in the implication chain
        let _ = (i, arg_ty); // suppress unused warning
    }

    // Wrap with foralls from inside out
    for (arg_ty, _) in args.iter().rev() {
        // SLam arg_ty body
        let slam = Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SLam".to_string())),
                Box::new(arg_ty.clone()),
            )),
            Box::new(body.clone()),
        );
        // Forall arg_ty slam
        body = make_forall_syntax_with_type(arg_ty, &slam);
    }

    Some(body)
}

/// Collect constructor arguments, skipping type parameters.
/// Returns (arg_type, is_recursive) pairs.
fn collect_ctor_args(ctor_ty: &Term, ind_name: &str, type_args: &[Term]) -> Vec<(Term, bool)> {
    let mut args = Vec::new();
    let mut current = ctor_ty;
    let mut skip_count = type_args.len();

    loop {
        match current {
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                if skip_count > 0 {
                    // Skip type parameter
                    skip_count -= 1;
                } else {
                    // Regular argument
                    let is_recursive = contains_inductive_term(param_type, ind_name);
                    // Convert kernel type to Syntax representation
                    let arg_ty_syntax = kernel_type_to_syntax(param_type);
                    args.push((arg_ty_syntax, is_recursive));
                }
                current = body_type;
            }
            _ => break,
        }
    }

    args
}

/// Check if a kernel Term contains the inductive type.
fn contains_inductive_term(term: &Term, ind_name: &str) -> bool {
    match term {
        Term::Global(name) => name == ind_name,
        Term::App(f, a) => {
            contains_inductive_term(f, ind_name) || contains_inductive_term(a, ind_name)
        }
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            contains_inductive_term(param_type, ind_name)
                || contains_inductive_term(body_type, ind_name)
        }
        Term::Lambda {
            param_type, body, ..
        } => {
            contains_inductive_term(param_type, ind_name) || contains_inductive_term(body, ind_name)
        }
        _ => false,
    }
}

/// Convert a kernel Term (type) to its Syntax representation.
fn kernel_type_to_syntax(term: &Term) -> Term {
    match term {
        Term::Global(name) => make_sname(name),
        Term::Var(name) => make_sname(name), // Named variable
        Term::App(f, a) => {
            let f_syn = kernel_type_to_syntax(f);
            let a_syn = kernel_type_to_syntax(a);
            // SApp f_syn a_syn
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("SApp".to_string())),
                    Box::new(f_syn),
                )),
                Box::new(a_syn),
            )
        }
        Term::Pi {
            param: _,
            param_type,
            body_type,
        } => {
            let pt_syn = kernel_type_to_syntax(param_type);
            let bt_syn = kernel_type_to_syntax(body_type);
            // SPi pt_syn bt_syn
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("SPi".to_string())),
                    Box::new(pt_syn),
                )),
                Box::new(bt_syn),
            )
        }
        Term::Sort(univ) => {
            // SSort univ
            Term::App(
                Box::new(Term::Global("SSort".to_string())),
                Box::new(univ_to_syntax(univ)),
            )
        }
        Term::Lit(lit) => {
            // SLit lit
            Term::App(
                Box::new(Term::Global("SLit".to_string())),
                Box::new(Term::Lit(lit.clone())),
            )
        }
        _ => {
            // Fallback for complex terms
            make_sname("Unknown")
        }
    }
}

/// Convert a Universe to Syntax.
fn univ_to_syntax(univ: &crate::term::Universe) -> Term {
    use crate::term::Universe;
    match univ {
        Universe::SProp => Term::Global("USProp".to_string()),
        Universe::Prop => Term::Global("UProp".to_string()),
        Universe::Type(n) => Term::App(
            Box::new(Term::Global("UType".to_string())),
            Box::new(Term::Lit(Literal::Int(*n as i64))),
        ),
        // Universe-polymorphic levels: deeply-embedded so reflection stays total.
        Universe::Var(v) => Term::App(
            Box::new(Term::Global("UVar".to_string())),
            Box::new(Term::Global(v.clone())),
        ),
        Universe::Succ(l) => Term::App(
            Box::new(Term::Global("USucc".to_string())),
            Box::new(univ_to_syntax(l)),
        ),
        Universe::Max(a, b) => Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("UMax".to_string())),
                Box::new(univ_to_syntax(a)),
            )),
            Box::new(univ_to_syntax(b)),
        ),
        Universe::IMax(a, b) => Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("UIMax".to_string())),
                Box::new(univ_to_syntax(a)),
            )),
            Box::new(univ_to_syntax(b)),
        ),
    }
}

// -------------------------------------------------------------------------
// Inversion Tactic
// -------------------------------------------------------------------------

/// Inversion tactic: analyze hypothesis to derive False if no constructor matches.
///
/// Given hypothesis H of form `SApp (SName "IndName") args`, check if any constructor
/// of IndName can produce those args. If no constructor can match, return DInversion H.
fn try_try_inversion_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    // Extract inductive name and arguments from the hypothesis type
    let (ind_name, hyp_args) = match extract_applied_inductive_from_syntax(goal) {
        Some((name, args)) => (name, args),
        None => return Some(make_error_derivation()),
    };

    // Check if the inductive type actually exists
    if !ctx.is_inductive(&ind_name) {
        // Unknown inductive type - cannot derive anything
        return Some(make_error_derivation());
    }

    // Get constructors for this inductive
    let constructors = ctx.get_constructors(&ind_name);

    // Check each constructor to see if it can match
    let mut any_possible = false;
    for (_ctor_name, ctor_type) in constructors.iter() {
        if can_constructor_match_args(ctx, ctor_type, &hyp_args, &ind_name) {
            any_possible = true;
            break;
        }
    }

    if any_possible {
        // Cannot derive False - some constructor could match
        return Some(make_error_derivation());
    }

    // All constructors impossible → build DInversion
    Some(Term::App(
        Box::new(Term::Global("DInversion".to_string())),
        Box::new(goal.clone()),
    ))
}

/// Verify DInversion proof: check that no constructor can match the hypothesis.
fn try_dinversion_conclude(ctx: &Context, hyp_type: &Term) -> Option<Term> {
    let norm_hyp = normalize(ctx, hyp_type);

    let (ind_name, hyp_args) = match extract_applied_inductive_from_syntax(&norm_hyp) {
        Some((name, args)) => (name, args),
        None => return Some(make_sname_error()),
    };

    // Check if the inductive type actually exists
    if !ctx.is_inductive(&ind_name) {
        return Some(make_sname_error());
    }

    let constructors = ctx.get_constructors(&ind_name);

    // Verify ALL constructors are impossible
    for (_ctor_name, ctor_type) in constructors.iter() {
        if can_constructor_match_args(ctx, ctor_type, &hyp_args, &ind_name) {
            return Some(make_sname_error());
        }
    }

    // All impossible → concludes False
    Some(make_sname("False"))
}

/// Extract inductive name and arguments from Syntax.
///
/// SApp (SApp (SName "Even") x) y → ("Even", [x, y])
/// SName "False" → ("False", [])
fn extract_applied_inductive_from_syntax(term: &Term) -> Option<(String, Vec<Term>)> {
    // Base case: SName "X"
    if let Term::App(ctor, text) = term {
        if let Term::Global(ctor_name) = ctor.as_ref() {
            if ctor_name == "SName" {
                if let Term::Lit(Literal::Text(name)) = text.as_ref() {
                    return Some((name.clone(), vec![]));
                }
            }
        }
    }

    // Recursive case: SApp f x
    if let Term::App(inner, arg) = term {
        if let Term::App(sapp_ctor, func) = inner.as_ref() {
            if let Term::Global(ctor_name) = sapp_ctor.as_ref() {
                if ctor_name == "SApp" {
                    // Recursively extract from the function
                    let (name, mut args) = extract_applied_inductive_from_syntax(func)?;
                    args.push(arg.as_ref().clone());
                    return Some((name, args));
                }
            }
        }
    }

    None
}

/// Check if a constructor can possibly match the given arguments.
///
/// For constructor `even_succ : ∀n. Even n → Even (Succ (Succ n))`:
/// - The constructor's result pattern is `Even (Succ (Succ n))`
/// - If hyp_args is `[Succ (Succ (Succ Zero))]` (representing 3):
///   - Unify `Succ (Succ n)` with `Succ (Succ (Succ Zero))`
///   - This succeeds with n = Succ Zero = 1
///   - But then we need to check if `Even 1` is constructible (recursive)
fn can_constructor_match_args(
    ctx: &Context,
    ctor_type: &Term,
    hyp_args: &[Term],
    ind_name: &str,
) -> bool {
    // Decompose constructor type to get result pattern and bound variable names
    let (result, pattern_vars) = decompose_ctor_type_with_vars(ctor_type);

    // Extract result's arguments (what the constructor produces)
    let result_args = match extract_applied_inductive_from_syntax(&kernel_type_to_syntax(&result)) {
        Some((name, args)) if name == *ind_name => args,
        _ => return false,
    };

    // If argument counts don't match, can't unify
    if result_args.len() != hyp_args.len() {
        return false;
    }

    // Try syntactic unification of all arguments together (tracking bindings across args)
    let mut bindings: std::collections::HashMap<String, Term> = std::collections::HashMap::new();

    for (pattern, concrete) in result_args.iter().zip(hyp_args.iter()) {
        if !can_unify_syntax_terms_with_bindings(
            ctx,
            pattern,
            concrete,
            &pattern_vars,
            &mut bindings,
        ) {
            return false;
        }
    }

    // If we get here, the constructor could match
    // (We don't check recursive hypotheses for simplicity - that would require
    // full inversion with backtracking)
    true
}

/// Decompose a constructor type to get the result type and bound variable names.
///
/// `∀n:Nat. Even n → Even (Succ (Succ n))` → (`Even (Succ (Succ n))`, ["n"])
/// `∀A:Type. ∀x:A. Eq A x x` → (`Eq A x x`, ["A", "x"])
/// `Bool` → (`Bool`, [])
fn decompose_ctor_type_with_vars(ty: &Term) -> (Term, Vec<String>) {
    let mut vars = Vec::new();
    let mut current = ty;
    loop {
        match current {
            Term::Pi {
                param, body_type, ..
            } => {
                vars.push(param.clone());
                current = body_type;
            }
            _ => break,
        }
    }
    (current.clone(), vars)
}

/// Check if two Syntax terms can unify, tracking variable bindings.
///
/// Pattern variables (names in `pattern_vars`) can bind to any concrete value,
/// but must bind consistently (same variable must bind to same value).
/// Other SNames must match exactly.
/// SApp recurses on function and argument.
fn can_unify_syntax_terms_with_bindings(
    ctx: &Context,
    pattern: &Term,
    concrete: &Term,
    pattern_vars: &[String],
    bindings: &mut std::collections::HashMap<String, Term>,
) -> bool {
    // SVar can match anything (explicit unification variable)
    if let Term::App(ctor, _idx) = pattern {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SVar" {
                return true;
            }
        }
    }

    // SName: check if it's a pattern variable or a constant
    if let Term::App(ctor1, text1) = pattern {
        if let Term::Global(n1) = ctor1.as_ref() {
            if n1 == "SName" {
                if let Term::Lit(Literal::Text(var_name)) = text1.as_ref() {
                    // Check if this is a pattern variable
                    if pattern_vars.contains(var_name) {
                        // Pattern variable: check existing binding or create new one
                        if let Some(existing) = bindings.get(var_name) {
                            // Already bound: concrete must match existing binding
                            return syntax_terms_equal(existing, concrete);
                        } else {
                            // Not yet bound: bind to concrete value
                            bindings.insert(var_name.clone(), concrete.clone());
                            return true;
                        }
                    }
                }
                // Not a pattern variable: must match exactly
                if let Term::App(ctor2, text2) = concrete {
                    if let Term::Global(n2) = ctor2.as_ref() {
                        if n2 == "SName" {
                            return text1 == text2;
                        }
                    }
                }
                return false;
            }
        }
    }

    // SApp: recurse on both function and argument
    if let (Term::App(inner1, arg1), Term::App(inner2, arg2)) = (pattern, concrete) {
        if let (Term::App(sapp1, func1), Term::App(sapp2, func2)) =
            (inner1.as_ref(), inner2.as_ref())
        {
            if let (Term::Global(n1), Term::Global(n2)) = (sapp1.as_ref(), sapp2.as_ref()) {
                if n1 == "SApp" && n2 == "SApp" {
                    return can_unify_syntax_terms_with_bindings(
                        ctx,
                        func1,
                        func2,
                        pattern_vars,
                        bindings,
                    ) && can_unify_syntax_terms_with_bindings(
                        ctx,
                        arg1.as_ref(),
                        arg2.as_ref(),
                        pattern_vars,
                        bindings,
                    );
                }
            }
        }
    }

    // SLit: compare literal values
    if let (Term::App(ctor1, lit1), Term::App(ctor2, lit2)) = (pattern, concrete) {
        if let (Term::Global(n1), Term::Global(n2)) = (ctor1.as_ref(), ctor2.as_ref()) {
            if n1 == "SLit" && n2 == "SLit" {
                return lit1 == lit2;
            }
        }
    }

    // Fall back to exact structural equality
    pattern == concrete
}

/// Check if two Syntax terms are structurally equal.
fn syntax_terms_equal(a: &Term, b: &Term) -> bool {
    match (a, b) {
        (Term::App(f1, x1), Term::App(f2, x2)) => {
            syntax_terms_equal(f1, f2) && syntax_terms_equal(x1, x2)
        }
        (Term::Global(n1), Term::Global(n2)) => n1 == n2,
        (Term::Lit(l1), Term::Lit(l2)) => l1 == l2,
        _ => a == b,
    }
}

// -------------------------------------------------------------------------
// Operator Tactics (rewrite, destruct, apply)
// -------------------------------------------------------------------------

/// Extract Eq A x y components from a Syntax term.
///
/// SApp (SApp (SApp (SName "Eq") A) x) y → Some((A, x, y))
fn extract_eq_components_from_syntax(term: &Term) -> Option<(Term, Term, Term)> {
    // term = SApp (SApp (SApp (SName "Eq") A) x) y
    // In kernel representation: App(App(SApp, App(App(SApp, App(App(SApp, SName "Eq"), A)), x)), y)

    // Peel off outermost SApp to get ((SApp (SApp (SName "Eq") A) x), y)
    let (eq_a_x, y) = extract_sapp(term)?;

    // Peel off next SApp to get ((SApp (SName "Eq") A), x)
    let (eq_a, x) = extract_sapp(&eq_a_x)?;

    // Peel off next SApp to get ((SName "Eq"), A)
    let (eq, a) = extract_sapp(&eq_a)?;

    // Verify it's SName "Eq"
    let eq_name = extract_sname(&eq)?;
    if eq_name != "Eq" {
        return None;
    }

    Some((a, x, y))
}

/// Extract (f, x) from SApp f x (in kernel representation).
fn extract_sapp(term: &Term) -> Option<(Term, Term)> {
    // SApp f x = App(App(Global("SApp"), f), x)
    if let Term::App(inner, x) = term {
        if let Term::App(sapp_ctor, f) = inner.as_ref() {
            if let Term::Global(ctor_name) = sapp_ctor.as_ref() {
                if ctor_name == "SApp" {
                    return Some((f.as_ref().clone(), x.as_ref().clone()));
                }
            }
        }
    }
    None
}

/// Extract name from SName "name".
fn extract_sname(term: &Term) -> Option<String> {
    if let Term::App(ctor, text) = term {
        if let Term::Global(ctor_name) = ctor.as_ref() {
            if ctor_name == "SName" {
                if let Term::Lit(Literal::Text(name)) = text.as_ref() {
                    return Some(name.clone());
                }
            }
        }
    }
    None
}

/// Check if a Syntax term contains a specific subterm.
fn contains_subterm_syntax(term: &Term, target: &Term) -> bool {
    if syntax_equal(term, target) {
        return true;
    }

    // Check SApp f x
    if let Some((f, x)) = extract_sapp(term) {
        if contains_subterm_syntax(&f, target) || contains_subterm_syntax(&x, target) {
            return true;
        }
    }

    // Check SLam T body
    if let Some((t, body)) = extract_slam(term) {
        if contains_subterm_syntax(&t, target) || contains_subterm_syntax(&body, target) {
            return true;
        }
    }

    // Check SPi T body
    if let Some((t, body)) = extract_spi(term) {
        if contains_subterm_syntax(&t, target) || contains_subterm_syntax(&body, target) {
            return true;
        }
    }

    false
}

/// Extract (T, body) from SLam T body.
fn extract_slam(term: &Term) -> Option<(Term, Term)> {
    if let Term::App(inner, body) = term {
        if let Term::App(slam_ctor, t) = inner.as_ref() {
            if let Term::Global(ctor_name) = slam_ctor.as_ref() {
                if ctor_name == "SLam" {
                    return Some((t.as_ref().clone(), body.as_ref().clone()));
                }
            }
        }
    }
    None
}

/// Extract (T, body) from SPi T body.
fn extract_spi(term: &Term) -> Option<(Term, Term)> {
    if let Term::App(inner, body) = term {
        if let Term::App(spi_ctor, t) = inner.as_ref() {
            if let Term::Global(ctor_name) = spi_ctor.as_ref() {
                if ctor_name == "SPi" {
                    return Some((t.as_ref().clone(), body.as_ref().clone()));
                }
            }
        }
    }
    None
}

/// Replace first occurrence of target with replacement in a Syntax term.
fn replace_first_subterm_syntax(term: &Term, target: &Term, replacement: &Term) -> Option<Term> {
    // If term equals target, return replacement
    if syntax_equal(term, target) {
        return Some(replacement.clone());
    }

    // Try to replace in SApp f x
    if let Some((f, x)) = extract_sapp(term) {
        // First try to replace in f
        if let Some(new_f) = replace_first_subterm_syntax(&f, target, replacement) {
            return Some(make_sapp(new_f, x));
        }
        // Then try to replace in x
        if let Some(new_x) = replace_first_subterm_syntax(&x, target, replacement) {
            return Some(make_sapp(f, new_x));
        }
    }

    // Try to replace in SLam T body
    if let Some((t, body)) = extract_slam(term) {
        if let Some(new_t) = replace_first_subterm_syntax(&t, target, replacement) {
            return Some(make_slam(new_t, body));
        }
        if let Some(new_body) = replace_first_subterm_syntax(&body, target, replacement) {
            return Some(make_slam(t, new_body));
        }
    }

    // Try to replace in SPi T body
    if let Some((t, body)) = extract_spi(term) {
        if let Some(new_t) = replace_first_subterm_syntax(&t, target, replacement) {
            return Some(make_spi(new_t, body));
        }
        if let Some(new_body) = replace_first_subterm_syntax(&body, target, replacement) {
            return Some(make_spi(t, new_body));
        }
    }

    // No replacement found
    None
}

/// Rewrite tactic: given eq_proof (concluding Eq A x y) and goal,
/// replaces x with y (or y with x if reverse=true) in goal.
fn try_try_rewrite_reduce(
    ctx: &Context,
    eq_proof: &Term,
    goal: &Term,
    reverse: bool,
) -> Option<Term> {
    // Get the conclusion of eq_proof
    let eq_conclusion = try_concludes_reduce(ctx, eq_proof)?;

    // Extract Eq A x y components
    let (_, lhs, rhs) = match extract_eq_components_from_syntax(&eq_conclusion) {
        Some(components) => components,
        None => return Some(make_error_derivation()),
    };

    // Determine what to replace based on direction
    let (target, replacement) = if reverse { (rhs, lhs) } else { (lhs, rhs) };

    // Check if target exists in goal
    if !contains_subterm_syntax(goal, &target) {
        return Some(make_error_derivation());
    }

    // Replace target with replacement in goal
    let new_goal = match replace_first_subterm_syntax(goal, &target, &replacement) {
        Some(ng) => ng,
        None => return Some(make_error_derivation()),
    };

    // Build DRewrite eq_proof goal new_goal
    Some(Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("DRewrite".to_string())),
                Box::new(eq_proof.clone()),
            )),
            Box::new(goal.clone()),
        )),
        Box::new(new_goal),
    ))
}

/// Verify DRewrite derivation and return the new goal.
fn try_drewrite_conclude(
    ctx: &Context,
    eq_proof: &Term,
    old_goal: &Term,
    new_goal: &Term,
) -> Option<Term> {
    // Get the conclusion of eq_proof
    let eq_conclusion = try_concludes_reduce(ctx, eq_proof)?;

    // Extract Eq A x y components
    let (_ty, lhs, rhs) = match extract_eq_components_from_syntax(&eq_conclusion) {
        Some(components) => components,
        None => return Some(make_sname_error()),
    };

    // Verify: new_goal = old_goal[lhs := rhs] OR new_goal = old_goal[rhs := lhs]
    // Check forward direction first
    if let Some(computed_new) = replace_first_subterm_syntax(old_goal, &lhs, &rhs) {
        if syntax_equal(&computed_new, new_goal) {
            return Some(new_goal.clone());
        }
    }

    // Check reverse direction
    if let Some(computed_new) = replace_first_subterm_syntax(old_goal, &rhs, &lhs) {
        if syntax_equal(&computed_new, new_goal) {
            return Some(new_goal.clone());
        }
    }

    // Verification failed
    Some(make_sname_error())
}

/// Destruct tactic: case analysis without induction hypotheses.
fn try_try_destruct_reduce(
    ind_type: &Term,
    motive: &Term,
    cases: &Term,
) -> Option<Term> {
    // For now, destruct is essentially the same as induction
    // The key difference is in what goals are expected for each case
    // (no IH for recursive constructors)
    //
    // We simply build a DDestruct and let verification check case proofs

    Some(Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("DDestruct".to_string())),
                Box::new(ind_type.clone()),
            )),
            Box::new(motive.clone()),
        )),
        Box::new(cases.clone()),
    ))
}

/// Verify a `DDestruct` derivation — case analysis WITHOUT induction hypotheses.
///
/// Identical to [`try_delim_conclude`] but for the induction hypotheses, which is the entire
/// difference between `cases` and `induction`, so the two share the case-formula builder and
/// differ only in the `include_ih` flag they pass it.
///
/// **This used to check the case COUNT and nothing else** — its own comment said so — and then
/// concede `∀x:Ind, P x`. `DDestruct` is a declared `Derivation` constructor and `try_destruct`
/// is a declared tactic that builds one, so
///
/// ```text
///   Definition fake : Derivation :=
///     try_destruct (SName "Bool") (SLam (SName "Bool") (SName "False"))
///       (DCase (DAxiom (SName "True")) (DCase (DAxiom (SName "True")) DCaseEnd)).
///   Eval concludes fake.
/// ```
///
/// answered `Forall Bool (λb. False)` — a derivation of `∀b:Bool, False` from two case proofs
/// that were never looked at, and one `DAxiom` away from a proof of anything at all. Nothing in
/// the literate surface reached it only because `destruct` is absent from that vocabulary, and
/// a hole whose only fence is "no caller has been written yet" is a hole.
fn try_ddestruct_conclude(
    ctx: &Context,
    ind_type: &Term,
    motive: &Term,
    cases: &Term,
) -> Option<Term> {
    let norm_ind_type = normalize(ctx, ind_type);
    let norm_motive = normalize(ctx, motive);
    let norm_cases = normalize(ctx, cases);

    let ind_name = extract_inductive_name_from_syntax(&norm_ind_type)?;
    if !ctx.is_inductive(&ind_name) {
        return Some(make_sname_error());
    }
    let constructors = ctx.get_constructors(&ind_name);
    if constructors.is_empty() {
        return Some(make_sname_error());
    }

    let case_proofs = match extract_case_proofs(&norm_cases) {
        Some(proofs) => proofs,
        None => return Some(make_sname_error()),
    };
    if case_proofs.len() != constructors.len() {
        return Some(make_sname_error());
    }

    let motive_body = match extract_slam_body(&norm_motive) {
        Some(body) => body,
        None => return Some(make_sname_error()),
    };

    // EVERY case is checked against the proposition its constructor owes. `include_ih` is
    // false: a `destruct` bullet is handed no induction hypothesis, so demanding one would
    // refuse correct case proofs, and offering one would concede a hypothesis nobody proved.
    for (i, (ctor_name, _ctor_type)) in constructors.iter().enumerate() {
        let case_conc = match try_concludes_reduce(ctx, &case_proofs[i]) {
            Some(c) => c,
            None => return Some(make_sname_error()),
        };
        let expected = match build_case_expected(
            ctx,
            ctor_name,
            &constructors,
            &motive_body,
            &norm_ind_type,
            false,
        ) {
            Some(e) => e,
            None => return Some(make_sname_error()),
        };
        if !syntax_equal(&case_conc, &expected) {
            return Some(make_sname_error());
        }
    }

    Some(make_forall_syntax_generic(&norm_ind_type, &norm_motive))
}

/// Apply tactic: manual backward chaining.
fn try_try_apply_reduce(
    ctx: &Context,
    hyp_name: &Term,
    hyp_proof: &Term,
    goal: &Term,
) -> Option<Term> {
    // Get the conclusion of hyp_proof
    let hyp_conclusion = try_concludes_reduce(ctx, hyp_proof)?;

    // Check if it's an implication: SPi A B where B doesn't use the bound var
    if let Some((antecedent, consequent)) = extract_spi(&hyp_conclusion) {
        // Check if consequent matches goal
        if syntax_equal(&consequent, goal) {
            // Build DApply hyp_name hyp_proof goal antecedent
            return Some(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::Global("DApply".to_string())),
                            Box::new(hyp_name.clone()),
                        )),
                        Box::new(hyp_proof.clone()),
                    )),
                    Box::new(goal.clone()),
                )),
                Box::new(antecedent),
            ));
        }
    }

    // Check if it's a forall that could match
    if let Some(_forall_body) = extract_forall_body(&hyp_conclusion) {
        // Try to match goal with forall body (simple syntactic check)
        // For now, if goal appears to be an instance of the forall body, accept it
        // Full implementation would do proper unification

        // Build DApply with new goal being True (trivially satisfied)
        return Some(Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global("DApply".to_string())),
                        Box::new(hyp_name.clone()),
                    )),
                    Box::new(hyp_proof.clone()),
                )),
                Box::new(goal.clone()),
            )),
            Box::new(make_sname("True")),
        ));
    }

    // If hypothesis directly matches goal, we're done
    if syntax_equal(&hyp_conclusion, goal) {
        return Some(Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global("DApply".to_string())),
                        Box::new(hyp_name.clone()),
                    )),
                    Box::new(hyp_proof.clone()),
                )),
                Box::new(goal.clone()),
            )),
            Box::new(make_sname("True")),
        ));
    }

    // Cannot apply this hypothesis to this goal
    Some(make_error_derivation())
}

/// Verify DApply derivation.
fn try_dapply_conclude(
    ctx: &Context,
    hyp_proof: &Term,
    old_goal: &Term,
    new_goal: &Term,
) -> Option<Term> {
    // Get the conclusion of hyp_proof
    let hyp_conclusion = try_concludes_reduce(ctx, hyp_proof)?;

    // If hypothesis is an implication A -> B and old_goal is B
    // then new_goal should be A
    if let Some((antecedent, consequent)) = extract_spi(&hyp_conclusion) {
        if syntax_equal(&consequent, old_goal) {
            if syntax_equal(&antecedent, new_goal)
                || extract_sname(new_goal) == Some("True".to_string())
            {
                return Some(new_goal.clone());
            }
        }
    }

    // If hypothesis is a forall and goal matches instantiation
    if let Some(_forall_body) = extract_forall_body(&hyp_conclusion) {
        // For forall application, the new goal is typically True or the instantiated body
        if extract_sname(new_goal) == Some("True".to_string()) {
            return Some(new_goal.clone());
        }
    }

    // If hypothesis directly matches old_goal
    if syntax_equal(&hyp_conclusion, old_goal) {
        if extract_sname(new_goal) == Some("True".to_string()) {
            return Some(new_goal.clone());
        }
    }

    // Verification failed
    Some(make_sname_error())
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/normalize_tests.rs"]
mod normalize_tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/reflection_signature_tests.rs"]
mod reflection_signature_tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/lazy_head_tests.rs"]
mod lazy_head_tests;

mod shared_head;
pub(crate) use shared_head::{
    certified_application, certified_program, program_shape, CertifiedProgram, ProgramShape,
};
pub(crate) use shared_head::{
    constructor_equivalent, constructors_disagree, natural_literal_conversion,
};
mod whnf_core;
pub(crate) use whnf_core::{
    beta_normal_form, certified_value, delta_head, unfold_head, whnf, whnf_core, Allowance,
    Exhausted,
};

/// Optional inventory of real machine operations during atomic observation.
/// These counters do not replenish or alter evaluator fuel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DagAtomWork {
    pub eval: u64,
    pub compute: u64,
    pub force: u64,
    pub returned: u64,
    pub force_ready: u64,
    pub force_suspended: u64,
    pub force_blocked: u64,
    pub return_update: u64,
    pub return_apply: u64,
    pub return_unroll: u64,
    pub return_match: u64,
    pub return_finish_unary: u64,
    pub return_complete: u64,
    pub return_unary_demand: u64,
    pub thunk_allocations: u64,
    pub capture_key_entries: u64,
    pub ready_value_rewrites: u64,
    pub lambda_values: u64,
    pub eval_global: u64,
    pub compute_global: u64,
    pub global_cache_hits: u64,
    pub delta: u64,
    pub beta: u64,
    pub zeta: u64,
    pub iota: u64,
    pub guarded_unroll: u64,
    pub suspended_constructions: u64,
    pub demanded_fresh_suspended: u64,
    pub demanded_fresh_running: u64,
    pub demanded_cached_suspended: u64,
    pub demanded_ready: u64,
    pub suspended_lambda_constructions: u64,
    pub forced_lambda_suspensions: u64,
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/normalization_growth_controls.rs"]
mod normalization_growth_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/bounded_fix_controls.rs"]
mod bounded_fix_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/bounded_polymorphic_delta_work.rs"]
mod bounded_polymorphic_delta_work;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/owned_bounded_spine_controls.rs"]
mod owned_bounded_spine_controls;

#[cfg(test)]
thread_local! { static TREE_REDUCTION_CALLS: std::cell::Cell<usize> = const {std::cell::Cell::new(0)}; }

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/bounded_size_scratch_controls.rs"]
mod bounded_size_scratch_controls;

#[cfg(test)]
thread_local! {static CONSTRUCTOR_MATERIALIZATIONS:std::cell::Cell<usize>=const {std::cell::Cell::new(0)};}
#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/borrowed_constructor_controls.rs"]
mod borrowed_constructor_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/owned_conversion_integration_controls.rs"]
mod owned_conversion_integration_controls;

fn assign_progress_in_place(term: &mut Term, result: Term) -> bool {
    if result == *term {
        return false;
    }
    *term = result;
    true
}

/// The full reducer's existing one-step order, consuming only changed nodes.
/// The outer normalizer retains its cycle detector, depth checks and step fuel.
fn reduce_step_in_place(ctx: &Context, term: &mut Term, tr: Transparency<'_>) -> bool {
    #[cfg(test)]
    TREE_REDUCTION_CALLS.with(|n| n.set(n.get() + 1));
    if let Some(result) = try_simp_rewrite(ctx, term) {
        *term = result;
        return true;
    }
    if let Term::App(func, arg) = term {
        if let Some(result) = try_primitive_reduce(ctx, func, arg, tr) {
            return assign_progress_in_place(term, result);
        }
        if let Term::App(head, first) = func.as_mut() {
            if let Term::Global(operation) = head.as_ref() {
                if crate::int_inductive::integer_alias_dependencies(operation)
                    .is_some_and(|dependencies| dependencies.iter().all(|name| tr.unfolds(name)))
                    && crate::int_inductive::accelerates_integer_alias(ctx, operation)
                {
                    if reduce_step_in_place(ctx, first, tr) {
                        return true;
                    }
                    if reduce_step_in_place(ctx, arg, tr) {
                        return true;
                    }
                }
            }
        }
        if let Some(result) = try_reflection_reduce(ctx, func, arg) {
            return assign_progress_in_place(term, result);
        }
        if let Term::Global(name) = func.as_ref() {
            if name == "reduceBool"
                && ctx.get_definition_body(name).is_none()
                && declaration_agrees_with_the_rule(ctx, name)
            {
                if let Some(value) = crate::eval::eval_bool(ctx, arg) {
                    return assign_progress_in_place(
                        term,
                        Term::Global(if value { "true" } else { "false" }.into()),
                    );
                }
            }
        }
        if let Some(result) = try_quot_lift_reduce(ctx, func, arg) {
            return assign_progress_in_place(term, result);
        }
        if let Some(result) = try_obs_reduce(ctx, func, arg) {
            return assign_progress_in_place(term, result);
        }
    }
    if matches!(term, Term::App(..)) {
        if let Some(result) = try_fix_unroll(ctx, term) {
            return assign_progress_in_place(term, result);
        }
    }
    match term {
        Term::Lit(_) | Term::Sort(_) | Term::Var(_) | Term::Hole => false,
        Term::Const { name, levels } => {
            let result = match const_step(ctx, name, levels) {
                ConstStep::Unresolved | ConstStep::Value => return false,
                ConstStep::Illformed => Term::Hole,
                ConstStep::Delta(subst, body) => {
                    if !tr.unfolds(name) {
                        return false;
                    }
                    crate::term::instantiate_universes(&body, &subst)
                }
            };
            *term = result;
            true
        }
        Term::App(func, arg) => match func.as_mut() {
            Term::Lambda { .. } => {
                let Term::App(func, arg) = std::mem::replace(term, Term::Hole) else {
                    unreachable!()
                };
                let Term::Lambda { param, body, .. } = *func else {
                    unreachable!()
                };
                *term = crate::type_checker::substitute_owned(*body, &param, &arg);
                true
            }
            Term::Fix { .. } => reduce_step_in_place(ctx, arg, tr),
            Term::MutualFix { defs, index } => {
                if is_constructor_form(ctx, arg) {
                    let mut unfolded = defs[*index].1.clone();
                    for (j, (name, _)) in defs.iter().enumerate() {
                        let projection = Term::MutualFix {
                            defs: defs.clone(),
                            index: j,
                        };
                        unfolded = substitute(&unfolded, name, &projection);
                    }
                    **func = unfolded;
                    true
                } else {
                    reduce_step_in_place(ctx, arg, tr)
                }
            }
            _ => {
                if reduce_step_in_place(ctx, func, tr) {
                    return true;
                }
                reduce_step_in_place(ctx, arg, tr)
            }
        },
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            if let Some(mut result) = iota_branch(ctx, discriminant, motive, cases) {
                reduce_step_in_place(ctx, &mut result, tr);
                *term = result;
                return true;
            }
            if let Some(constructor) = (cases.len() == 1)
                .then(|| KProbeMemo::probe(ctx, discriminant, motive))
                .flatten()
            {
                **discriminant = constructor;
                return true;
            }
            reduce_step_in_place(ctx, discriminant, tr)
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            let original_type = (**param_type).clone();
            let domain_changed = reduce_step_in_place(ctx, param_type, tr);
            let scope = KProbeMemo::enter(param);
            let body_changed = reduce_step_in_place(&ctx.extend(param, original_type), body, tr);
            KProbeMemo::leave(scope);
            domain_changed || body_changed
        }
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            let original_type = (**param_type).clone();
            let domain_changed = reduce_step_in_place(ctx, param_type, tr);
            let scope = KProbeMemo::enter(param);
            let body_changed =
                reduce_step_in_place(&ctx.extend(param, original_type), body_type, tr);
            KProbeMemo::leave(scope);
            domain_changed || body_changed
        }
        Term::Fix { body, .. } => reduce_step_in_place(ctx, body, tr),
        Term::MutualFix { defs, .. } => {
            let mut changed = false;
            for (_, body) in defs {
                changed |= reduce_step_in_place(ctx, body, tr);
            }
            changed
        }
        Term::Let { .. } => {
            // The occurrence-gated sharing policy and its order, applied where the parts lie.
            crate::term_store::zeta_or_share_in_place(
                term,
                &|t| reduce_step_in_place(ctx, t, tr),
                &|t| reduce_step_opt(ctx, t, tr),
            )
        }
        Term::Global(name) => {
            if !tr.unfolds(name) {
                return false;
            }
            let Some(body) = ctx.get_definition_body(name) else {
                return false;
            };
            *term = body.clone();
            true
        }
        // Consumed with its term's step, never by one of its own (see `reduce_step_opt`).
        Term::Annotated { term: inner, .. } => {
            if !reduce_step_in_place(ctx, inner, tr) {
                return false;
            }
            let reduct = std::mem::replace(inner.as_mut(), Term::Hole);
            *term = reduct;
            true
        }
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/owned_normalize_controls.rs"]
mod owned_normalize_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/k_probe_controls.rs"]
mod k_probe_controls;
#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/nested_depth_controls.rs"]
mod nested_depth_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/pending_compute_controls.rs"]
mod pending_compute_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/borrowed_closed_controls.rs"]
mod borrowed_closed_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/lazy_telescope_admission.rs"]
mod lazy_telescope_admission;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/head_size_scan_controls.rs"]
mod head_size_scan_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/delta_borrowed_admission_controls.rs"]
mod delta_borrowed_admission_controls;
