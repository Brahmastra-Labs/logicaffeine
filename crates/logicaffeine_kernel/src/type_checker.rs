//! Bidirectional type checker for the Calculus of Constructions.
//!
//! The type checker implements the typing rules of CoC:
//!
//! ```text
//! ─────────────────────── (Sort)
//!   Γ ⊢ Type n : Type (n+1)
//!
//!   Γ(x) = A
//! ─────────────── (Var)
//!   Γ ⊢ x : A
//!
//!   Γ ⊢ A : Type i    Γ, x:A ⊢ B : Type j
//! ─────────────────────────────────────────── (Pi)
//!          Γ ⊢ Π(x:A). B : Type max(i,j)
//!
//!   Γ ⊢ A : Type i    Γ, x:A ⊢ t : B
//! ─────────────────────────────────────── (Lambda)
//!      Γ ⊢ λ(x:A). t : Π(x:A). B
//!
//!   Γ ⊢ f : Π(x:A). B    Γ ⊢ a : A
//! ───────────────────────────────────── (App)
//!         Γ ⊢ f a : B[x := a]
//! ```

use crate::context::Context;
use crate::error::{KernelError, KernelResult};
use crate::reduction::normalize;
use crate::term::{Literal, Term, Universe};

// =================================================================================================
// The depth door
// =================================================================================================

/// The deepest term the kernel will descend.
///
/// A `Term` arrives from a translator, a file, or a network peer — it is UNTRUSTED INPUT, and
/// its nesting depth is chosen by whoever wrote it. `infer_type`, `reduce_step` and the
/// conversion checker all walk it with native recursion, so a deep enough term exhausts the
/// thread's stack and the process dies with `SIGABRT`: a denial of service with no error
/// channel, indistinguishable from a crash in the code that called the kernel. A checker whose
/// answer to a bad input is "the process is gone" cannot be part of a trusted base.
///
/// So the kernel REFUSES at the door what it cannot safely walk, and says so. The limit comes
/// from MEASUREMENT, not taste, and it is squeezed from both sides:
///
/// - It must be ABOVE everything the kernel is asked to do today. The deepest term the suite
///   builds is a 1500-deep β-spine (`lean_conformance_t00_terms`'s scaling measurement), which
///   normalises fine because its redex is at the head, so every step is shallow. A door below
///   that would refuse working code, and a guard that fails silently in the middle of a proof
///   is worse than the abort it replaced.
/// - It must be BELOW the depth at which `Term`'s DERIVED `Clone`/`Drop` abort. A single
///   clone-and-drop survives 2000 and aborts by 4000; a term that is cloned a few times over
///   (which checking one does) aborts by 2200. Those run before and after any kernel code, so a
///   term the kernel accepts must at least be one it can copy, repeatedly.
///
/// 1600 is the band between the two. The RESIDUE is real and is NOT closed: how deep the kernel
/// can actually descend depends on the SHAPE of a term, not only its depth — an irreducible
/// 400-deep spine aborts inside `reduce_step` (no head redex, so it is searched to the bottom)
/// while a 1500-deep β-spine does not. Closing that needs a larger kernel stack at the call
/// site, or an iterative `Drop`/`Clone` for `Term` plus an explicit-stack reducer — `term.rs`
/// and a reducer rewrite, not a constant.
pub(crate) const MAX_TERM_DEPTH: usize = 1600;

thread_local! {
    /// Whether a kernel entry point is already on this thread's stack. The door check runs
    /// once per outermost call, never on the recursive ones, so it costs one linear pass per
    /// kernel invocation rather than one per node.
    static INSIDE_KERNEL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Holds the "inside the kernel" flag for the outermost entry, clearing it on the way out —
/// including on a panic, so the flag can never be left stale.
pub(crate) struct DoorGuard(bool);

impl Drop for DoorGuard {
    fn drop(&mut self) {
        if self.0 {
            INSIDE_KERNEL.with(|f| f.set(false));
        }
    }
}

/// `t`'s nesting depth exceeds `limit`.
///
/// Measured with an EXPLICIT stack and an early exit, so asking the question about a
/// pathologically deep term cannot itself overflow — a recursive `depth()` helper would abort
/// on exactly the inputs this exists to catch.
pub(crate) fn depth_exceeds(t: &Term, limit: usize) -> bool {
    let mut work: Vec<(&Term, usize)> = vec![(t, 0)];
    while let Some((node, d)) = work.pop() {
        if d > limit {
            return true;
        }
        let deeper = d + 1;
        match node {
            Term::Sort(_)
            | Term::Var(_)
            | Term::Global(_)
            | Term::Const { .. }
            | Term::Lit(_)
            | Term::Hole => {}
            Term::App(f, a) => {
                work.push((f, deeper));
                work.push((a, deeper));
            }
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                work.push((param_type, deeper));
                work.push((body_type, deeper));
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                work.push((param_type, deeper));
                work.push((body, deeper));
            }
            Term::Let {
                ty, value, body, ..
            } => {
                work.push((ty, deeper));
                work.push((value, deeper));
                work.push((body, deeper));
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                work.push((discriminant, deeper));
                work.push((motive, deeper));
                for c in cases {
                    work.push((c, deeper));
                }
            }
            Term::Fix { body, .. } => work.push((body, deeper)),
            Term::MutualFix { defs, .. } => {
                for (_, b) in defs {
                    work.push((b, deeper));
                }
            }
            Term::Annotated { term, .. } => work.push((term, deeper)),
        }
    }
    false
}

/// Check `term` against the depth door if this is the OUTERMOST kernel call on this thread.
/// Returns the guard that restores the flag.
pub(crate) fn enter_kernel(term: &Term, what: &str) -> KernelResult<DoorGuard> {
    if INSIDE_KERNEL.with(|f| f.get()) {
        return Ok(DoorGuard(false));
    }
    if depth_exceeds(term, MAX_TERM_DEPTH) {
        crate::reduction::record_resource_stop(crate::reduction::Unreduced::TooDeep);
        return Err(KernelError::CertificationError(format!(
            "term nests deeper than the kernel's limit of {MAX_TERM_DEPTH} while running              {what}: this is a REFUSAL, not a verdict — the term is neither accepted nor              rejected, because descending it would exhaust the native stack"
        )));
    }
    INSIDE_KERNEL.with(|f| f.set(true));
    Ok(DoorGuard(true))
}

/// The same door for the entry points that cannot report an error. `None` means "refused".
pub(crate) fn enter_kernel_or_refuse(term: &Term) -> Option<DoorGuard> {
    enter_kernel(term, "reduction").ok()
}

/// Whether a kernel entry point is already on this thread's stack.
///
/// Read-only: it never opens or closes the door. It exists so an entry point can skip the
/// stack-SIZING measurement when the answer cannot matter — once a kernel call is under way the
/// native stack it is going to get has already been chosen, and measuring a subterm to ask for a
/// bigger one would only pay for a walk whose answer is ignored.
pub(crate) fn inside_kernel() -> bool {
    INSIDE_KERNEL.with(|f| f.get())
}

// App needs only a Pi head; Match needs only a registered inductive spine.
// A successful positive probe preserves the remaining arguments and annotations.
// Every inconclusive probe retains the original full-normalization fallback.
#[derive(Clone, Copy)]
pub(crate) enum TypeHead {
    Function,
    Inductive,
}

#[cfg(test)]
thread_local! {
    static TYPE_HEAD_FULL_NORMALIZATIONS: std::cell::Cell<[usize; 2]> =
        const { std::cell::Cell::new([0; 2]) };
}

pub(crate) fn demanded_type_head(ctx: &Context, ty: Term, requested: TypeHead) -> Term {
    probe_type_head(ctx, ty, requested).unwrap_or_else(|ty| {
        #[cfg(test)]
        TYPE_HEAD_FULL_NORMALIZATIONS.with(|count| {
            let mut values = count.get();
            values[match requested {
                TypeHead::Function => 0,
                TypeHead::Inductive => 1,
            }] += 1;
            count.set(values);
        });
        normalize(ctx, &ty)
    })
}

/// `Ok` with a type whose head is the requested one — the type itself, or its bounded head
/// reduction — and `Err` with the type handed back when only a full normalization could say.
fn probe_type_head(ctx: &Context, ty: Term, requested: TypeHead) -> Result<Term, Term> {
    // Preserve the existing syntactic Pi path, including custom-rule contexts.
    if matches!(requested, TypeHead::Function) && matches!(ty, Term::Pi { .. }) {
        return Ok(ty);
    }
    let has_requested_head = |candidate: &Term| match requested {
        TypeHead::Function => matches!(candidate, Term::Pi { .. }),
        TypeHead::Inductive => extract_inductive_name(ctx, candidate).is_some(),
    };
    // Registered rewrites may affect the exposed head. Their general reduction
    // path is unchanged; the bounded probe also refuses such contexts.
    if ctx.simp_rules().is_empty() {
        if has_requested_head(&ty) {
            return Ok(ty);
        }
        if let Some(exposed) = crate::reduction::bounded_conversion_head(ctx, &ty) {
            if has_requested_head(&exposed) {
                return Ok(exposed);
            }
        }
    }
    Err(ty)
}

/// The type of a projected value with its inductive family exposed at the head, as Lean's
/// `infer_proj` reads it (`whnf (infer_type e)`): the type itself when its head already names an
/// inductive family, else the same bounded head reduction the kernel's own `match` typing uses.
/// `Err` hands the type back when that bounded probe does not reach an inductive head; the caller
/// keeps its full-normalization fallback, so no type that normalizes to a family is refused here.
pub(crate) fn exposed_inductive_type(ctx: &Context, ty: Term) -> Result<Term, Term> {
    probe_type_head(ctx, ty, TypeHead::Inductive)
}

/// The type of an applied function with its `Π` exposed at the head, as Lean's `infer_app` reads
/// it (`ensure_pi (whnf (infer f))`) and as this kernel's own application rule does: the type
/// itself when it is a `Π`, else its bounded head reduction. `Err` hands the type back when that
/// probe does not reach a `Π`; the caller keeps its full-normalization fallback.
pub(crate) fn exposed_function_type(ctx: &Context, ty: Term) -> Result<Term, Term> {
    probe_type_head(ctx, ty, TypeHead::Function)
}

// Peel only the exact canonical Nat constructor typing rule. Every skipped
// application has domain Nat and codomain Nat; the residual tail still goes
// through ordinary checking. This removes recursive typing work, not limits.
fn infer_canonical_peano(ctx: &Context, term: &Term) -> Option<KernelResult<Term>> {
    let Term::App(head, _) = term else {
        return None;
    };
    if !matches!(head.as_ref(), Term::Global(name) if name == "Succ")
        || !ctx.simp_rules().is_empty()
        || !crate::int_inductive::canonical_nat_environment(ctx)
    {
        return None;
    }
    let mut tail = term;
    let mut prefix = 0usize;
    while let Term::App(head, argument) = tail {
        if !matches!(head.as_ref(), Term::Global(name) if name == "Succ") {
            break;
        }
        prefix += 1;
        if prefix > MAX_TERM_DEPTH {
            return Some(Err(KernelError::CertificationError(
                "computed Peano term exceeds the kernel term-depth limit".into(),
            )));
        }
        tail = argument;
    }
    // Computation may have built this prefix after the outer door ran. Preserve
    // that same ceiling across the entire result, including a non-Peano tail.
    if depth_exceeds(tail, MAX_TERM_DEPTH - prefix) {
        return Some(Err(KernelError::CertificationError(
            "computed Peano term exceeds the kernel term-depth limit".into(),
        )));
    }
    let natural = Term::Global("Nat".into());
    Some(check_type(ctx, tail, &natural).map(|()| natural))
}

#[cfg(test)]
thread_local! {
    static PEANO_INFERENCE_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Infer the type of a term in a context.
///
/// This is the main entry point for type checking. It implements bidirectional
/// type inference for the Calculus of Constructions.
///
/// # Type Rules
///
/// - `Type n : Type (n+1)` - Universes form a hierarchy
/// - `x : A` if `x : A` in context - Variable lookup
/// - `Π(x:A). B : Type max(i,j)` if `A : Type i` and `B : Type j`
/// - `λ(x:A). t : Π(x:A). B` if `t : B` in extended context
/// - `f a : B[x := a]` if `f : Π(x:A). B` and `a : A`
///
/// # Errors
///
/// Returns [`KernelError`] variants for:
/// - Unbound variables
/// - Type mismatches in applications
/// - Invalid match constructs
/// - Termination check failures for fixpoints
pub fn infer_type(ctx: &Context, term: &Term) -> KernelResult<Term> {
    if let Some(r) = crate::annotation::erased(term, |term| infer_type(ctx, term)) {
        return r;
    }
    // THE DOOR MUST MEAN WHAT IT SAYS. `MAX_TERM_DEPTH` advertises 1600 rungs, and this walk
    // dies at about 52 of a dependent telescope on the 2 MiB stack a spawned thread gets —
    // not with a refusal but with a `SIGABRT`, which is the one answer a kernel may never give.
    // `deepen_stack` sizes the stack from the term's OWN depth at the OUTERMOST entry: it runs
    // `f` inline for a shallow term and on a sized thread of the kernel's own for a deep one,
    // and always returns `None` on the re-entrant call inside the closure, which then falls
    // through to the body below. So the measurement is paid exactly once per outermost call —
    // the same budget `enter_kernel`'s own `depth_exceeds` pass already has — and it is skipped
    // entirely once a kernel call is under way, where a bigger stack can no longer be arranged.
    if !inside_kernel() {
        if let Some(r) = crate::term_store::deepen_stack(term, || infer_type(ctx, term)) {
            return r;
        }
    }
    let _door = enter_kernel(term, "infer_type")?;
    #[cfg(test)]
    PEANO_INFERENCE_ENTRIES.with(|count| count.set(count.get() + 1));
    if let Some(result) = infer_canonical_peano(ctx, term) {
        return result;
    }
    match term {
        // Sort: Type n : Type (n+1)
        Term::Sort(u) => Ok(Term::Sort(u.succ())),

        // Var: lookup in local context. A miss is not one event but two — an ordinary
        // unbound local, and an elaborator metavariable that was never solved — and they
        // need different repairs, so they get different diagnostics.
        Term::Var(name) => ctx.get(name).cloned().ok_or_else(|| classify_unbound(name)),

        // Global: lookup in global context (inductives, constructors, definitions, axioms).
        // The stored type is returned VERBATIM, level variables and all: a bare reference is
        // universe-polymorphic, and the levels are introduced as metavariables by the
        // APPLICATION rule (`implicit_levels_for_head`), where there is an argument to solve
        // them from. Introducing them here instead would rename the level variables of every
        // reference — including the ones a polymorphic definition's own body is being checked
        // against, where they are genuinely bound and must not be touched.
        Term::Global(name) => match ctx.get_global(name) {
            Some(ty) => Ok(ty.clone()),
            None => Err(classify_unbound(name)),
        },

        // Const: a universe-polymorphic global at explicit levels. The telescope, the closure
        // check and the positional zip are decided ONCE, in `term::resolve_const_levels`, which
        // the re-checker and the reducer call with their own lookups — so a reference that
        // typing refuses can never be a normal form to the reducer.
        Term::Const { name, levels } => {
            let (declared, ty) = lookup_const(ctx, name).ok_or_else(|| classify_unbound(name))?;
            let subst = crate::term::resolve_const_levels(name, declared.as_deref(), &ty, levels)
                .map_err(|e| KernelError::CertificationError(e.to_string()))?;
            Ok(crate::term::instantiate_universes(&ty, &subst))
        }

        // Pi: Π(x:A). B : Type max(sort(A), sort(B))
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            // A must be a type
            let a_sort = infer_sort(ctx, param_type)?;

            // B must be a type in the extended context
            let (param, body_type) = open_context_binder(ctx, param, param_type, body_type);
            let extended_ctx = ctx.extend(&param, (**param_type).clone());
            let b_sort = infer_sort(&extended_ctx, &body_type)?;

            // Product formation (CIC). `Prop` is **impredicative**: a Π whose
            // codomain is a proposition is itself a proposition, no matter the
            // domain's universe — so `∀x:Entity. P(x)` is a `Prop`, and FOL
            // formulas built from it (And/Or/Ex over universals) stay in `Prop`
            // where `And`/`Ex` require their arguments to live. `imax` is exactly
            // this rule: `imax(a, Prop) = Prop`, `imax(a, non-Prop) = max(a, b)`,
            // and it stays SYMBOLIC when the codomain level is a variable (whose
            // Prop-ness is not yet known) — the case the old `_ => max` got wrong.
            let pi_sort = a_sort.imax(&b_sort);
            Ok(Term::Sort(pi_sort))
        }

        // Lambda: λ(x:A). t : Π(x:A). T where t : T
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            // Check param_type is well-formed (is a type)
            let _ = infer_sort(ctx, param_type)?;

            // Infer body type in extended context
            let (param, body) = open_context_binder(ctx, param, param_type, body);
            let extended_ctx = ctx.extend(&param, (**param_type).clone());
            let body_type = infer_type(&extended_ctx, &body)?;

            // Keep the same fresh lexical binder in the inferred telescope.
            Ok(Term::Pi {
                param: param.into_owned(),
                param_type: param_type.clone(),
                body_type: Box::new(body_type),
            })
        }

        // Let: `let x : A := v in b`. Check `A` is a type and `v : A`, then type
        // the body with `x` bound TRANSPARENTLY — by zeta-substituting `v` for
        // `x` (so `b`'s type sees `x ≡ v`, not an opaque hypothesis). This is
        // checked by the substitution lemma when possible, with transparent
        // zeta-expansion retained for bodies that need the value for conversion.
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            let _ = infer_sort(ctx, ty)?;
            check_type(ctx, value, ty)?;
            // The substitution lemma lets us check the body with an opaque local
            // and substitute only its inferred type. A body whose conversions need
            // the value falls back to the full transparent rule below.
            if !ctx.local_name_in_use(name) {
                std::thread_local! { static LET_PROBE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
                struct Probe(bool);
                impl Drop for Probe {
                    fn drop(&mut self) {
                        LET_PROBE.with(|flag| flag.set(self.0));
                    }
                }
                let probe = Probe(LET_PROBE.with(|flag| flag.replace(true)));
                let nested = probe.0;
                let result = infer_type(&ctx.extend(name, (**ty).clone()), body);
                drop(probe);
                match result {
                    Ok(body_type) => return Ok(instantiate_codomain(name, ty, &body_type, value)),
                    // Abort the whole speculative pass at the first dependent
                    // failure; nested retries would otherwise grow exponentially.
                    Err(error) if nested => return Err(error),
                    Err(_) => {}
                }
            }
            let unfolded = substitute(body, name, value);
            infer_type(ctx, &unfolded)
        }

        // App: (f a) : B[x := a] where f : Π(x:A). B and a : A
        Term::App(func, arg) => {
            let func_type = infer_type(ctx, func)?;
            // Expose the Pi head without normalizing its domain or codomain.
            // Hidden/unsupported heads retain the full normalization fallback;
            // all argument checking and universe solving below still run.
            let func_type = demanded_type_head(ctx, func_type, TypeHead::Function);
            // A bare reference to a universe-polymorphic declaration carries no levels, so they
            // become fresh metavariables HERE — at the one place there is an argument to solve
            // them from.
            let func_type = implicit_levels_for_head(func, func_type);

            match func_type {
                Term::Pi {
                    param,
                    param_type,
                    body_type,
                } => {
                    // LEVEL SOLVING. When the head is a bare reference to a
                    // universe-polymorphic declaration, its domain carries level
                    // metavariables; solve them by matching the domain against the
                    // argument's ACTUAL type before checking anything. Choosing an
                    // instantiation is not accepting one — `check_type` below still runs
                    // against the substituted domain, so a bad choice refuses the
                    // application rather than admitting it.
                    // `None` means there was nothing to solve, and then the ORIGINALS are moved
                    // through untouched — the ordinary application must not pay a deep clone of
                    // its domain and codomain for a rule that did not fire.
                    let (param_type, body_type) =
                        match solve_domain_levels(ctx, &param_type, &body_type, arg) {
                            Some(solved) => solved,
                            None => (*param_type, *body_type),
                        };

                    // Check argument has expected type
                    check_type(ctx, arg, &param_type)?;

                    // Substitute argument into body type, SHARING it rather than copying it
                    // when copying would blow the type up (see `substitute_shared`).
                    Ok(expose_head(substitute_shared(
                        &body_type,
                        &param,
                        &param_type,
                        arg,
                    )))
                }
                _ => Err(KernelError::NotAFunction(format!("{}", func))),
            }
        }

        // Match: pattern matching on inductive types
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            // 1. Expose only a registered inductive head where possible.
            // Parameters and indices remain available for the exact conversion,
            // coverage, motive, branch and elimination checks below.
            let disc_type =
                demanded_type_head(ctx, infer_type(ctx, discriminant)?, TypeHead::Inductive);
            let inductive_name = extract_inductive_name(ctx, &disc_type)
                .ok_or_else(|| KernelError::NotAnInductive(format!("{}", disc_type)))?;

            // An empty `match` is `ex falso` and needs an UNINHABITED discriminant type. A
            // zero-constructor inductive the kernel's own literal rule inhabits is not one.
            if empty_match_would_be_a_lie(ctx, &inductive_name) {
                return Err(KernelError::InvalidMotive(format!(
                    "'{inductive_name}' is registered with no constructors, but the kernel's \
                     literal rule produces values of it — so a `match` on one is not `ex falso`: \
                     it would prove its motive from a value that demonstrably exists. Give \
                     '{inductive_name}' its constructors"
                )));
            }

            // Parameter/index split. `num_params` leading arguments of the inductive are
            // uniform PARAMETERS fixed by the discriminant; any remaining arguments are
            // INDICES that vary per constructor, over which the motive abstracts (`Eq`'s
            // `P : Π(y:A). Eq A x y → Sort`). When there are no indices this is the
            // ordinary eliminator, which takes the original path below byte-for-byte.
            let disc_args = extract_type_args(&disc_type);
            let num_params = ctx
                .inductive_num_params(&inductive_name)
                .min(disc_args.len());
            if disc_args.len() > num_params {
                return infer_indexed_match(
                    ctx,
                    discriminant,
                    motive,
                    cases,
                    &disc_type,
                    &inductive_name,
                    num_params,
                    &disc_args,
                );
            }

            // 2. Check motive is well-formed
            // The motive can be either:
            // - A function λ_:I. T (proper motive)
            // - A raw type T (constant motive, wrapped automatically)
            let motive_type = infer_type(ctx, motive)?;
            let effective_motive = match &motive_type {
                Term::Pi {
                    param,
                    param_type,
                    body_type,
                } => {
                    // A motive may spell the discriminant type through beta,
                    // delta, eta, or other kernel conversion rules. Cumulative
                    // subtyping is insufficient here: the domain is invariant.
                    if !def_eq(ctx, param_type, &disc_type) {
                        return Err(KernelError::InvalidMotive(format!(
                            "motive parameter {} doesn't match discriminant type {}",
                            param_type, disc_type
                        )));
                    }
                    // This is the CODOMAIN OF THE MOTIVE'S TYPE. It must
                    // itself reduce to Sort, not merely have a sort as its type:
                    // Nat has a sort too, but a Nat-valued motive is not a family.
                    let (param, body_type) = open_context_binder(ctx, param, param_type, body_type);
                    let motive_context = ctx.extend(&param, (**param_type).clone());
                    let codomain = crate::reduction::normalize_checked(&motive_context, &body_type)
                        .map_err(|reason| {
                            KernelError::CertificationError(format!(
                                "motive codomain normalization unfinished: {reason:?}"
                            ))
                        })?;
                    if !matches!(codomain, Term::Sort(_)) {
                        return Err(KernelError::InvalidMotive(format!(
                            "motive type codomain {} is not a sort",
                            body_type
                        )));
                    }
                    // Use motive as-is
                    (**motive).clone()
                }
                Term::Sort(_) => {
                    // Motive is a raw type - wrap in a constant function
                    // λ_:disc_type. motive
                    Term::Lambda {
                        param: "_".to_string(),
                        param_type: Box::new(disc_type.clone()),
                        body: motive.clone(),
                    }
                }
                _ => {
                    return Err(KernelError::InvalidMotive(format!(
                        "motive {} is not a function or type",
                        motive
                    )));
                }
            };

            // 3. Check case count matches constructor count
            let constructors = ctx.get_constructors(&inductive_name);
            if cases.len() != constructors.len() {
                return Err(KernelError::WrongNumberOfCases {
                    expected: constructors.len(),
                    found: cases.len(),
                });
            }

            // 4. Check each case has the correct type
            for (case, (ctor_name, _)) in cases.iter().zip(constructors.iter()) {
                let (ctor_head, ctor_type) =
                    constructor_at_discriminant(ctx, ctor_name, &disc_type)?;
                let expected_case_type =
                    compute_case_type(&effective_motive, &ctor_head, &ctor_type, &disc_type);
                check_type(ctx, case, &expected_case_type)?;
            }

            // 5. Return type is Motive(discriminant), beta-reduced
            // Without beta reduction, (λ_:T. R) x returns the un-reduced form
            // which causes type mismatches in nested matches.
            let return_type =
                beta_reduce(&Term::App(Box::new(effective_motive), discriminant.clone()));

            // 6. The elimination restrictions (`check_elimination`).
            let disc_sort = sort_of_type(ctx, &disc_type)?;
            check_elimination(ctx, &inductive_name, &disc_sort, || {
                sort_of_type(ctx, &return_type)
            })?;

            Ok(return_type)
        }

        // Literal: infer type based on literal kind
        Term::Lit(lit) => match lit {
            Literal::Int(_) | Literal::BigInt(_) => Ok(Term::Global("Int".to_string())),
            Literal::Nat(n) if *n < logicaffeine_base::BigInt::from_i64(0) => Err(
                KernelError::CertificationError("a `Nat` literal must be non-negative".to_string()),
            ),
            Literal::Nat(_) => Ok(Term::Global("Nat".to_string())),
            Literal::Float(_) => Ok(Term::Global("Float".to_string())),
            Literal::Text(_) => Ok(Term::Global("Text".to_string())),
            Literal::Duration(_) => Ok(Term::Global("Duration".to_string())),
            Literal::Date(_) => Ok(Term::Global("Date".to_string())),
            Literal::Moment(_) => Ok(Term::Global("Moment".to_string())),
        },

        // Hole: implicit argument, cannot infer type standalone
        // Holes are handled specially in check_type
        Term::Hole => Err(KernelError::CannotInferHole),

        // Fix: fix f. body
        // The type of (fix f. body) is the type of body when f is bound to that type.
        // This is a fixpoint equation: T = type_of(body) where f : T.
        //
        // For typical fixpoints, body is a lambda: fix f. λx:A. e
        // The type is Π(x:A). B where B is the type of e (with f : Π(x:A). B).
        Term::Fix { name, body } => {
            let (name, body) = open_context_binder(ctx, name, body, body);
            // For fix f. body, we need to handle the recursive reference to f.
            // We structurally infer the type from lambda structure.
            //
            // This works because:
            // 1. The body is typically nested lambdas with explicit parameter types
            // 2. The return type is determined by the innermost expression's motive
            // 3. Recursive calls have the same type as the fixpoint itself

            // Extract the structural type from nested lambdas and motive
            let structural_type = infer_fix_type_structurally(ctx, &body)
                .map_err(|e| name_the_unreadable_fixpoint(&name, e))?;
            if occurs_free(&structural_type, &name) {
                return Err(KernelError::CertificationError(format!(
                    "the fixpoint `{name}`'s type mentions `{name}` itself"
                )));
            }

            // *** THE GUARDIAN: TERMINATION CHECK ***
            // Verify that recursive calls decrease structurally.
            // Without this check, we could "prove" False by looping forever.
            crate::termination::check_termination(ctx, &name, &body)?;

            // Sanity check: verify the body is well-formed with f bound
            let extended = ctx.extend(&name, structural_type.clone());
            let _ = infer_type(&extended, &body)?;

            Ok(structural_type)
        }

        // MutualFix: a block of mutually-recursive definitions; this occurrence denotes
        // the `index`-th one. Each definition's type is inferred structurally (like the
        // single Fix), all names are put in scope, and the MUTUAL Giménez guard verifies
        // termination before the bodies are sanity-checked with every sibling bound.
        Term::MutualFix { defs, index } => {
            if defs.is_empty() || *index >= defs.len() {
                return Err(KernelError::CertificationError(
                    "mutual fixpoint with an empty block or out-of-range index".to_string(),
                ));
            }

            let opened_defs = open_context_mutual(ctx, defs);
            let defs = opened_defs.as_ref();
            // Structural type of each definition (independent of the others' bodies).
            let mut types = Vec::with_capacity(defs.len());
            for (name, body) in defs {
                let ty = infer_fix_type_structurally(ctx, body)
                    .map_err(|e| name_the_unreadable_fixpoint(name, e))?;
                if let Some((member, _)) = defs.iter().find(|(member, _)| occurs_free(&ty, member))
                {
                    return Err(KernelError::CertificationError(format!(
                        "the mutual fixpoint `{name}`'s type mentions the block's `{member}`"
                    )));
                }
                types.push(ty);
            }

            // *** THE GUARDIAN: MUTUAL TERMINATION CHECK ***
            crate::termination::check_termination_mutual(ctx, defs)?;

            // Sanity: every body is well-formed with ALL names bound to their structural
            // types (a sibling call `rec_Odd n' o` sees `rec_Odd`'s type this way).
            let mut extended = ctx.clone();
            for ((name, _), ty) in defs.iter().zip(types.iter()) {
                extended = extended.extend(name, ty.clone());
            }
            for (_, body) in defs {
                let _ = infer_type(&extended, body)?;
            }

            Ok(types[*index].clone())
        }

        Term::Annotated { term, .. } => infer_type(ctx, term),
    }
}

/// A structural-inference failure that is really "this fixpoint's own name is not in scope yet",
/// renamed so it cannot be mistaken for a forward reference.
///
/// `infer_fix_type_structurally` runs in the ORIGINAL context — the fixpoint's own name is bound
/// only afterwards, for the sanity check — so a `Fix` whose result type can be read off no route
/// but one through the recursive call reports `UnboundVariable(<its own binder>)`. That is the
/// exact variant `Context::declare` keys its FORWARD-REFERENCE deferral on, so such a definition
/// used to be DEFERRED, and a deferral that can never resolve (the name is bound by the term
/// itself, not by any later declaration) leaves the definition registered with a body nothing
/// type-checked. Naming the situation refuses it loudly instead of admitting it silently.
///
/// Only the fixpoint's OWN binder is renamed. A genuine forward reference to another constant
/// keeps `UnboundVariable` and keeps being deferred, which is the behaviour the prelude's
/// registration order depends on.
fn name_the_unreadable_fixpoint(name: &str, e: KernelError) -> KernelError {
    match &e {
        KernelError::UnboundVariable(v) if v == name => KernelError::CertificationError(format!(
            "the fixpoint `{name}`'s result type cannot be read off its body: every route to it \
             passes through the recursive call, whose type is the very thing being inferred. \
             Give the recursion a `match` whose motive states the result type — that is the \
             route this kernel reads a fixpoint's type by."
        )),
        _ => e,
    }
}

/// Infer the type of a fixpoint body structurally.
///
/// For `λx:A. body`, returns `Π(x:A). <type of body>`.
/// This recursively handles nested lambdas.
///
/// The key insight is that for well-formed fixpoints, the body structure
/// determines the type: parameters have explicit types, and the return type
/// can be inferred from the innermost expression.
fn infer_fix_type_structurally(ctx: &Context, term: &Term) -> KernelResult<Term> {
    match term {
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            // Check param_type is well-formed
            let _ = infer_sort(ctx, param_type)?;

            // Extend context and recurse into body
            let (param, body) = open_context_binder(ctx, param, param_type, body);
            let extended = ctx.extend(&param, (**param_type).clone());
            let body_type = infer_fix_type_structurally(&extended, &body)?;

            // Retain the name used during structural inference.
            Ok(Term::Pi {
                param: param.into_owned(),
                param_type: param_type.clone(),
                body_type: Box::new(body_type),
            })
        }
        // For non-lambda bodies (the base case), we need to determine the return type.
        // This is typically a Match whose motive determines it: the return type is the
        // `motive` applied to the discriminant's INDEX arguments (for an indexed family) and
        // then the discriminant itself, β-normalized. Computing the application — rather
        // than just reading off the motive's body — is robust to the motive's binder names
        // (which need not match the fixpoint's) and to indexed families. The discriminant is
        // the fixpoint's structural binder, in scope, so its type is available without the
        // not-yet-bound recursive name.
        Term::Match {
            discriminant,
            motive,
            ..
        } => {
            // A constant motive `return T` — one whose own type is a Sort — IS the result
            // type and is not applied to the discriminant. Only a function motive `λx. P x`
            // is applied to the discriminant's index arguments and then the discriminant
            // itself. This mirrors the Term::Match inference rule, which wraps a Sort-typed
            // motive as the constant `λ_:I. T` rather than applying it; without this guard a
            // constant motive `Nat`/`List A` becomes the ill-formed `App(Nat, n)`.
            if let Ok(mt) = infer_type(ctx, motive) {
                if matches!(normalize(ctx, &mt), Term::Sort(_)) {
                    return Ok(normalize(ctx, motive));
                }
            }
            let mut applied = (**motive).clone();
            if let Ok(dt) = infer_type(ctx, discriminant) {
                let dt = normalize(ctx, &dt);
                if let Some(ind) = extract_inductive_name(ctx, &dt) {
                    let args = extract_type_args(&dt);
                    let p = ctx.inductive_num_params(&ind).min(args.len());
                    for idx in &args[p..] {
                        applied = Term::App(Box::new(applied), Box::new(idx.clone()));
                    }
                }
            }
            applied = Term::App(Box::new(applied), discriminant.clone());
            Ok(normalize(ctx, &applied))
        }
        // A convoy: a `match` applied to further arguments. Its type is the `match`'s, read as
        // above, applied to those arguments.
        Term::App(..) if matches!(spine_of(term).0, Term::Match { .. }) => {
            let (head, args) = spine_of(term);
            let mut ty = infer_fix_type_structurally(ctx, head)?;
            for arg in args {
                match demanded_type_head(ctx, ty, TypeHead::Function) {
                    Term::Pi {
                        param, body_type, ..
                    } => ty = substitute(&body_type, &param, arg),
                    other => return Err(KernelError::NotAFunction(format!("{other}"))),
                }
            }
            Ok(normalize(ctx, &ty))
        }
        // For other expressions, try normal inference
        _ => infer_type(ctx, term),
    }
}

/// Check that a term has the expected type (with subtyping/cumulativity).
///
/// Implements bidirectional type checking: when checking a Lambda against a Pi,
/// we can use the Pi's parameter type instead of the Lambda's (which may be a
/// placeholder from match case parsing).
pub(crate) fn check_type(ctx: &Context, term: &Term, expected: &Term) -> KernelResult<()> {
    // SPEC B — a `Term::Hole` needs no arm here, in either position. A hole TERM falls through
    // to `infer_type`, which reports `CannotInferHole`; a hole EXPECTED TYPE falls through to
    // `alpha_eq`/`is_subtype`, which refuse it. The two exemptions that used to sit here —
    // "a hole stands for a type, so accept it against any `Sort`" and "a hole expects
    // anything, so accept any well-typed term" — were the only reason `Eq _ (3:Nat) (true:Bool)`
    // type-checked: a heterogeneous equation this kernel deliberately cannot express.

    // A fixpoint checked against a type: bound at that type, a terminating fixpoint's body must
    // have it. It types a recursion whose result type no `match` motive states, at every position
    // that has an expected type (a declared type, a function's domain, a `let`'s type, a case's
    // type), each of which is a type wherever this is asked. A fixpoint this does not type falls to
    // inference, which reads the type off the body.
    if let Term::Fix { name, body } = term {
        let (name, body) = open_context_binder(ctx, name, body, body);
        if crate::termination::check_termination(ctx, &name, &body).is_ok() {
            let scoped = ctx.extend(&name, expected.clone());
            if matches!(infer_type(&scoped, &body), Ok(got) if is_subtype(&scoped, &got, expected))
            {
                return Ok(());
            }
        }
    }

    // Special case: Lambda with placeholder type checked against Pi
    // This handles match cases where binder types come from the expected type
    if let Term::Lambda {
        param,
        param_type,
        body,
    } = term
    {
        // Check if param_type is a placeholder ("_")
        if let Term::Global(name) = param_type.as_ref() {
            if name == "_" {
                // Bidirectional mode: get param type from expected
                if let Term::Pi {
                    param_type: expected_param_type,
                    body_type: expected_body_type,
                    param: expected_param,
                } = expected
                {
                    // Check body in extended context using expected param type
                    let fresh = fresh_context_name(
                        ctx,
                        param,
                        &[expected_param_type, expected_body_type, body],
                    );
                    let body = rename_var(body, param, &fresh);
                    let extended_ctx = ctx.extend(&fresh, (**expected_param_type).clone());
                    let body_expected =
                        substitute(expected_body_type, expected_param, &Term::Var(fresh));
                    return check_type(&extended_ctx, &body, &body_expected);
                }
            }
        }
    }

    let inferred = infer_type(ctx, term)?;
    if is_subtype(ctx, &inferred, expected) {
        Ok(())
    } else {
        Err(KernelError::TypeMismatch {
            expected: format!("{}", expected),
            found: format!("{}", inferred),
        })
    }
}

/// Normalize an already inferred non-Sort type for sort recognition.
fn normalized_inferred_sort(ctx: &Context, term: &Term, inferred: &Term) -> KernelResult<Universe> {
    let normalized = crate::reduction::normalize_checked(ctx, inferred).map_err(|reason| {
        KernelError::CertificationError(format!(
            "sort inference did not finish normalization: {reason:?}"
        ))
    })?;
    match normalized {
        Term::Sort(universe) => Ok(universe),
        _ => Err(KernelError::NotAType(format!("{}", term))),
    }
}

/// Infer a type expression once, verify its inferred type is a Sort, and retain
/// that original unnormalized inferred type for independent agreement checks.
pub(crate) fn infer_type_as_type(ctx: &Context, term: &Term) -> KernelResult<Term> {
    let inferred = infer_type(ctx, term)?;
    if !matches!(&inferred, Term::Sort(_)) {
        normalized_inferred_sort(ctx, term, &inferred)?;
    }
    Ok(inferred)
}

/// Infer the sort (universe) of a type.
///
/// A term is a type if its type is a Sort.
pub(crate) fn infer_sort(ctx: &Context, term: &Term) -> KernelResult<Universe> {
    let inferred = infer_type(ctx, term)?;
    match inferred {
        Term::Sort(universe) => Ok(universe),
        other => normalized_inferred_sort(ctx, term, &other),
    }
}

/// The RESULT sort of an inductive's arity — peel its leading `Π`s to the final `Sort`.
fn result_sort_universe(t: &Term) -> Option<Universe> {
    let mut cur = t;
    while let Term::Pi { body_type, .. } = cur {
        cur = body_type;
    }
    match cur {
        Term::Sort(u) => Some(u.clone()),
        _ => None,
    }
}

/// Extract the result universe from an already exposed inductive arity.
pub(crate) fn constructor_target_universe(
    ind: &str,
    normalized_arity: &Term,
) -> KernelResult<Universe> {
    result_sort_universe(normalized_arity).ok_or_else(|| {
        KernelError::InvalidDeclaration(format!(
            "inductive '{ind}' must have a telescope ending in Sort"
        ))
    })
}

/// Whether a family in `target` may store arguments of any sort: the impredicative sorts. Neither
/// gives a stored argument back as data — an `SProp` eliminates into `SProp` alone, and a `Prop`
/// into a relevant sort only as a subsingleton whose fields are all proofs — so a large argument
/// stored there opens no large elimination.
fn stores_any_sort(target: &Universe) -> bool {
    *target == Universe::SProp || target.equiv(&Universe::Prop)
}

/// Shared by the checked, freshened signature walk and the standalone universe gate.
pub(crate) fn check_constructor_field_universe(
    ctx: &Context,
    ind: &str,
    ctor: &str,
    field: &Term,
    target: &Universe,
) -> KernelResult<()> {
    if stores_any_sort(target) {
        return Ok(());
    }
    let sort = infer_sort(ctx, field)?;
    if !sort.is_subtype_of(target) {
        return Err(KernelError::InvalidDeclaration(format!(
            "universe inconsistency: constructor '{ctor}' stores an argument in sort \
             {sort}, which exceeds the sort {target} of its inductive '{ind}'"
        )));
    }
    Ok(())
}

/// Check the CIC UNIVERSE CONSTRAINT of an inductive constructor: every VALUE argument's
/// sort must be `≤` the inductive's result sort — so a `Type 0` inductive cannot store a
/// `Type 0`-typed field (which lives in `Type 1`), the universe inconsistency that opens
/// Girard/Hurkens paradoxes. A `Prop` or `SProp` inductive is exempt (both sorts are
/// impredicative and admit arguments of any sort), as in Rocq. The inductive (and any mutual
/// siblings a recursive field references) must already be registered in `ctx`.
pub fn check_constructor_universes(
    ctx: &Context,
    ind: &str,
    ctor: &str,
    ty: &Term,
) -> KernelResult<()> {
    let Some(ind_ty) = ctx.get_global(ind) else {
        return Ok(());
    };
    let normal = |term: &Term| {
        crate::reduction::normalize_checked(ctx, term).map_err(|error|
            KernelError::CertificationError(format!(
                "constructor '{ctor}' of '{ind}': universe telescope normalization unfinished: {error:?}"
            )))
    };
    let target = constructor_target_universe(ind, &normal(ind_ty)?)?;
    if stores_any_sort(&target) {
        return Ok(());
    }
    let mut cur = normal(ty)?;
    let num_params = ctx.inductive_num_params(ind);
    let mut ext = ctx.clone();
    let mut i = 0usize;
    while let Term::Pi {
        param,
        param_type,
        body_type,
    } = cur
    {
        if i >= num_params {
            check_constructor_field_universe(&ext, ind, ctor, &param_type, &target)?;
        }
        let mut fresh = format!("__constructor_universe_local_{i}");
        while ext.local_name_in_use(&fresh) {
            fresh.push('_');
        }
        cur = substitute(&body_type, &param, &Term::Var(fresh.clone()));
        ext = ext.extend(&fresh, *param_type);
        i += 1;
    }
    Ok(())
}

/// Beta-reduce a term (single step, at the head).
///
/// (λx.body) arg → body[x := arg]
fn beta_reduce(term: &Term) -> Term {
    match term {
        Term::App(func, arg) => {
            match func.as_ref() {
                Term::Lambda { param, body, .. } => {
                    // Beta reduction: (λx.body) arg → body[x := arg]
                    substitute(body, param, arg)
                }
                _ => term.clone(),
            }
        }
        _ => term.clone(),
    }
}

/// Instantiate both a constructor reference and its type at the scrutinee's levels.
fn constructor_at_discriminant(
    ctx: &Context,
    name: &str,
    disc_type: &Term,
) -> KernelResult<(Term, Term)> {
    let mut head = disc_type;
    while let Term::App(f, _) = head {
        head = f;
    }
    let constructor = match head {
        Term::Const { levels, .. } if ctx.poly_declaration(name).is_some() => Term::Const {
            name: name.to_string(),
            levels: levels.clone(),
        },
        _ => Term::Global(name.to_string()),
    };
    let ty = infer_type(ctx, &constructor)?;
    Ok((constructor, ty))
}

/// An indexed motive must abstract over the family's GENERIC index telescope,
/// followed by a major premise at those same indices, and return a sort. Validate
/// its inferred type rather than beta-reducing applications: a constant motive
/// body can otherwise erase ill-typed arguments before anyone checks them.
fn validate_indexed_motive(
    ctx: &Context,
    motive: &Term,
    disc_type: &Term,
    num_params: usize,
    disc_args: &[Term],
) -> KernelResult<()> {
    fn head(ctx: &Context, ty: Term) -> KernelResult<Term> {
        if matches!(ty, Term::Pi { .. } | Term::Sort(_)) {
            return Ok(ty);
        }
        crate::reduction::normalize_checked(ctx, &ty).map_err(|reason| {
            KernelError::CertificationError(format!(
                "indexed motive telescope normalization unfinished: {reason:?}"
            ))
        })
    }
    let error = |message: &str| KernelError::InvalidMotive(message.to_string());
    let (family_head, _) = spine_of(disc_type);
    let mut family = family_head.clone();
    let mut arity = infer_type(ctx, family_head)?;
    for parameter in &disc_args[..num_params] {
        let Term::Pi {
            param, body_type, ..
        } = head(ctx, arity)?
        else {
            return Err(error(
                "inductive has fewer parameters than its discriminant",
            ));
        };
        arity = substitute(&body_type, &param, parameter);
        family = Term::App(Box::new(family), Box::new(parameter.clone()));
    }
    let mut motive_type = infer_type(ctx, motive)?;
    let mut inner = ctx.clone();
    let mut avoid = free_vars(motive);
    avoid.extend(free_vars(disc_type));
    for index in 0..disc_args.len() - num_params {
        let Term::Pi {
            param,
            param_type,
            body_type,
        } = head(&inner, arity)?
        else {
            return Err(error(
                "inductive index telescope is shorter than its discriminant",
            ));
        };
        let Term::Pi {
            param: actual_param,
            param_type: actual_domain,
            body_type: actual_body,
        } = head(&inner, motive_type)?
        else {
            return Err(error("indexed motive lacks an index binder"));
        };
        if !def_eq(&inner, &actual_domain, &param_type) {
            return Err(error(
                "indexed motive index domain differs from the inductive telescope",
            ));
        }
        let mut name = fresh_name(&format!("__motive_index{index}"), &avoid);
        while inner.local_name_in_use(&name) {
            name.push('_');
        }
        avoid.insert(name.clone());
        let variable = Term::Var(name.clone());
        arity = substitute(&body_type, &param, &variable);
        motive_type = substitute(&actual_body, &actual_param, &variable);
        family = Term::App(Box::new(family), Box::new(variable));
        inner = inner.extend(&name, *param_type);
    }
    if !matches!(head(&inner, arity)?, Term::Sort(_)) {
        return Err(error(
            "discriminant does not instantiate the full inductive index telescope",
        ));
    }
    let Term::Pi {
        param,
        param_type,
        body_type,
    } = head(&inner, motive_type)?
    else {
        return Err(error("indexed motive lacks its major-premise binder"));
    };
    if !def_eq(&inner, &param_type, &family) {
        return Err(error(
            "indexed motive major domain differs from the generic indexed family",
        ));
    }
    let mut major = fresh_name("__motive_major", &avoid);
    while inner.local_name_in_use(&major) {
        major.push('_');
    }
    let codomain = substitute(&body_type, &param, &Term::Var(major.clone()));
    inner = inner.extend(&major, family);
    if !matches!(head(&inner, codomain)?, Term::Sort(_)) {
        return Err(error(
            "indexed motive must return a sort after its major-premise binder",
        ));
    }
    Ok(())
}

/// Type an indexed match after validating its generic index/major telescope.
/// Each branch specializes that checked motive to its constructor's indices;
/// the result specializes it to the discriminant. Validation must precede beta
/// reduction, which can erase malformed motive applications.
#[allow(clippy::too_many_arguments)]
fn infer_indexed_match(
    ctx: &Context,
    discriminant: &Term,
    motive: &Term,
    cases: &[Term],
    disc_type: &Term,
    inductive_name: &str,
    num_params: usize,
    disc_args: &[Term],
) -> KernelResult<Term> {
    // The discriminant's own parameter args (fixed) and index args (what the motive is
    // instantiated at for the *result* type).
    let disc_params = &disc_args[0..num_params];
    let disc_indices = &disc_args[num_params..];

    // Validate the generic telescope before applications of the motive are
    // beta-reduced below. Checking only their reduced result can erase a
    // mismatched index or major-premise argument.
    validate_indexed_motive(ctx, motive, disc_type, num_params, disc_args)?;

    // SPEC F — coverage is over the constructors the INDICES still allow, not over all of them.
    // A branch is owed only for a constructor that could have produced this scrutinee; one
    // whose declared result index provably clashes with the discriminant's is discharged here,
    // exactly as Lean discharges it by index unification. Supplying every constructor's branch
    // anyway stays legal, so nothing written against the old rule stops working.
    let constructors = ctx.get_constructors(inductive_name);
    let owed: Vec<(&str, &Term)> = if cases.len() == constructors.len() {
        constructors.iter().map(|(n, t)| (*n, *t)).collect()
    } else {
        let allowed = constructors_the_indices_allow(ctx, inductive_name, num_params, disc_args);
        if cases.len() != allowed.len() {
            return Err(KernelError::WrongNumberOfCases {
                expected: allowed.len(),
                found: cases.len(),
            });
        }
        allowed
            .iter()
            .map(|&i| (constructors[i].0, constructors[i].1))
            .collect()
    };

    // Each case against its indexed constructor type.
    for (case, (ctor_name, _)) in cases.iter().zip(owed.iter()) {
        let (ctor_head, ctor_type) = constructor_at_discriminant(ctx, ctor_name, disc_type)?;
        let expected =
            compute_indexed_case_type(motive, &ctor_head, &ctor_type, num_params, disc_params);
        check_type(ctx, case, &expected)?;
    }

    // Return type: `motive disc_index₁ … disc_indexₖ discriminant`, normalized.
    let mut ret = motive.clone();
    for idx in disc_indices {
        ret = Term::App(Box::new(ret), Box::new(idx.clone()));
    }
    ret = Term::App(Box::new(ret), Box::new(discriminant.clone()));
    let ret = normalize(ctx, &ret);

    // The result must be a type — this is what certifies the motive is a well-formed
    // family into a sort (a non-family motive would not infer to a `Sort` here).
    match normalize(ctx, &infer_type(ctx, &ret)?) {
        Term::Sort(_) => {}
        other => {
            return Err(KernelError::InvalidMotive(format!(
                "indexed match on '{}' has non-type result {} — the motive is not a family into a sort",
                inductive_name, other
            )));
        }
    }

    let disc_sort = sort_of_type(ctx, disc_type)?;
    check_elimination(ctx, inductive_name, &disc_sort, || sort_of_type(ctx, &ret))?;

    Ok(ret)
}

/// The universe a type lives in: the sort its type normalizes to.
fn sort_of_type(ctx: &Context, ty: &Term) -> KernelResult<Universe> {
    match normalize(ctx, &infer_type(ctx, ty)?) {
        Term::Sort(u) => Ok(u),
        other => Err(KernelError::InvalidMotive(format!(
            "{ty} is not a type: its type is {other}"
        ))),
    }
}

/// The elimination restrictions of `match`, for a scrutinee whose family lives in `Sort disc` and
/// a result whose sort `ret` reads, read only for a family that may be a proposition; the plain and
/// the indexed `match` rules both ask here.
///
/// * An `SProp` family eliminates only into `SProp`, with no subsingleton exception: definitional
///   irrelevance identifies any two inhabitants of an `SProp` type, so a match out of one into a
///   relevant sort equates its branches (two constructors, a match into `Nat`, and `Zero` is
///   definitionally `Succ Zero`), and into `Prop` it equates `True` with `False`.
/// * A family that is a proposition eliminates into a larger sort only when it is a subsingleton:
///   no constructors (`ex falso`), or one whose non-parameter arguments are all proofs (`And`,
///   `Eq`). Otherwise a proof would pick a value, and proof irrelevance would equate the values it
///   picks. The rule must hold at EVERY instance of the family's universe levels: `Sort u` is
///   `Prop` at `u := 0`, so for a family whose level may be `Prop` the result's level must be at
///   most the family's (`SProp`, `Prop`, or `Sort u` again). Checked with `u` abstract, an
///   elimination of a two-constructor `Two.{u} : Sort u` into `Nat` would otherwise pass, and at
///   `u := 0` be the `0 = 1` of a large elimination of a proposition. Lean makes such a family's
///   recursor eliminate into `Prop` only; this keeps the elimination at the family's own level too.
fn check_elimination(
    ctx: &Context,
    inductive_name: &str,
    disc: &Universe,
    ret: impl FnOnce() -> KernelResult<Universe>,
) -> KernelResult<()> {
    if disc.is_never_prop() {
        return Ok(());
    }
    let ret = &ret()?;
    if matches!(disc, Universe::SProp) && !matches!(ret, Universe::SProp) {
        return Err(KernelError::InvalidMotive(format!(
            "elimination of the definitionally-irrelevant proposition '{inductive_name}' \
             into a relevant sort is not allowed: every inhabitant of an SProp is \
             definitionally equal, so a match on one would equate its branches"
        )));
    }
    let (large, instance) = if disc.equiv(&Universe::Prop) {
        (!ret.equiv(&Universe::Prop), "")
    } else {
        (
            !ret.is_subtype_of(disc),
            " (a proposition at some instance of its universe levels)",
        )
    };
    if large && !is_subsingleton_prop(ctx, inductive_name)? {
        return Err(KernelError::InvalidMotive(format!(
            "large elimination of proposition '{inductive_name}'{instance} into a larger sort is not \
             allowed: only subsingleton propositions (empty, or one constructor with propositional \
             arguments — e.g. False, And, eq) may be eliminated into Type"
        )));
    }
    Ok(())
}

/// The expected type of one constructor's case in an INDEXED match: the constructor's
/// leading `num_params` parameters are instantiated by the discriminant's `disc_params`;
/// its remaining value arguments become the case's `Π` binders; and the codomain is the
/// `motive` applied to the constructor's RESULT indices (as they appear in its declared
/// return type) and then the constructor value itself.
fn compute_indexed_case_type(
    motive: &Term,
    ctor_head: &Term,
    ctor_type: &Term,
    num_params: usize,
    disc_params: &[Term],
) -> Term {
    // Consume binders while their scope is intact. Substituting independently
    // into stripped field types can rewrite variables introduced by an earlier
    // parameter substitution, even when each individual substitution is safe.
    let mut current = ctor_type.clone();
    for arg in disc_params.iter().take(num_params) {
        if let Term::Pi {
            param, body_type, ..
        } = current
        {
            current = substitute(&body_type, &param, arg);
        } else {
            break;
        }
    }
    let mut avoid = free_vars(motive);
    avoid.extend(free_vars(&current));
    for arg in disc_params {
        avoid.extend(free_vars(arg));
    }
    let mut fields = Vec::new();
    while let Term::Pi {
        param,
        param_type,
        body_type,
    } = current
    {
        let fresh = fresh_name(&format!("__arg{}", fields.len()), &avoid);
        avoid.insert(fresh.clone());
        current = substitute(&body_type, &param, &Term::Var(fresh.clone()));
        fields.push((fresh, *param_type));
    }
    let mut ctor_applied = ctor_head.clone();
    for arg in disc_params {
        ctor_applied = Term::App(Box::new(ctor_applied), Box::new(arg.clone()));
    }
    for (field, _) in &fields {
        ctor_applied = Term::App(Box::new(ctor_applied), Box::new(Term::Var(field.clone())));
    }
    let mut result = motive.clone();
    for index in extract_type_args(&current).into_iter().skip(num_params) {
        result = Term::App(Box::new(result), Box::new(index));
    }
    result = beta_reduce(&Term::App(Box::new(result), Box::new(ctor_applied)));
    for (param, domain) in fields.into_iter().rev() {
        result = Term::Pi {
            param,
            param_type: Box::new(beta_reduce(&domain)),
            body_type: Box::new(result),
        };
    }
    result
}

/// Non-indexed cases use the same capture-avoiding telescope instantiation.
fn compute_case_type(motive: &Term, ctor_head: &Term, ctor_type: &Term, disc_type: &Term) -> Term {
    let parameters = extract_type_args(disc_type);
    compute_indexed_case_type(motive, ctor_head, ctor_type, parameters.len(), &parameters)
}

/// Extract type arguments from a type application.
///
/// - `List A` → `[A]`
/// - `Either A B` → `[A, B]`
/// - `Nat` → `[]`
fn extract_type_args(ty: &Term) -> Vec<Term> {
    let mut args = Vec::new();
    let mut current = ty;

    while let Term::App(func, arg) = current {
        args.push((**arg).clone());
        current = func;
    }

    args.reverse();
    args
}

/// Substitute a term for a variable: `body[var := replacement]`.
///
/// Performs capture-avoiding substitution. Variables bound by lambda,
/// pi, or fix that shadow `var` are not substituted into.
///
/// # Capture Avoidance
///
/// Given `substitute(λx. y, "y", x)`, the result is `λx. x` (not `λx. x`
/// with the inner x captured). This implementation relies on unique
/// variable names from parsing.
///
/// # Term Forms
///
/// - `Sort`, `Lit`, `Hole`, `Global` - Unchanged (no variables)
/// - `Var(name)` - Replaced if `name == var`, unchanged otherwise
/// - `Pi`, `Lambda`, `Fix` - Substitute in components, respecting shadowing
/// - `App`, `Match` - Substitute recursively in all subterms
pub fn substitute(body: &Term, var: &str, replacement: &Term) -> Term {
    // Fast path: if `var` does not occur free in `body`, the substitution is the
    // identity. Crucially this lets us skip `free_vars(replacement)` — and the
    // `replacement` is, in `App`/`Match` type inference, the WHOLE argument/discriminant
    // proof term. A propositional implication's codomain never mentions the bound proof
    // variable (non-dependent), so this is the common case and turns quadratic checking
    // (walk the giant argument at every application) into linear.
    if !occurs_free(body, var) {
        return body.clone();
    }
    // Compute the free variables of the replacement once; a binder in `body`
    // that captures any of them must be alpha-renamed before we descend.
    let replacement_fvs = free_vars(replacement);
    substitute_avoiding(body, var, replacement, &replacement_fvs)
}

// =================================================================================================
// Universe-polymorphic references, level solving, and sharing (main kernel side)
// =================================================================================================

/// The axioms Lean's kernel carries that LOGOS's trusted base deliberately does NOT.
///
/// A proof minted against a PATCHED prelude cites one of these and then fails re-check against
/// the rebuilt trusted context. Reported as an `UnboundVariable` that reads like a typo, and the
/// obvious repair — add the name to `StandardLibrary::register` — is the catastrophic one: the
/// trusted base grows while every certificate ever minted still claims the old one. So these
/// names get their OWN diagnosis, and every other missing name stays an unbound variable.
///
/// The list is deliberately CLOSED and small. Widening it to "any unknown global is an axiom"
/// is the wrong fix: an unknown name is overwhelmingly a typo or a missing import, and calling
/// that a trusted-base event trains the reader to ignore the message that matters.
const UNTRUSTED_AXIOMS: &[&str] = &[
    "Classical_choice",
    "propext",
    "Quot_sound",
    "sorryAx",
    "em",
    "choice",
];

/// Diagnose a name that resolved nowhere. One `KernelError` variant used to cover three
/// completely different events; they need three different repairs, so they get three different
/// messages.
pub(crate) fn unbound_diagnosis(name: &str) -> Option<String> {
    if crate::elaborate::is_meta(name) {
        return Some(format!(
            "E_UNINSTANTIATED_METAVARIABLE: the metavariable '{name}' reached the kernel \
             without a solution. Elaboration must instantiate every metavariable before the \
             term is checked; a `?`-named variable is not a name the kernel can look up, and \
             guessing one is a soundness decision disguised as a convenience",
        ));
    }
    if UNTRUSTED_AXIOMS.contains(&name) {
        let admission = matches!(name, "sorryAx");
        return Some(format!(
            "{}: '{name}' is not in LOGOS's trusted base. {}",
            if admission {
                "E_INCOMPLETE_PROOF"
            } else {
                "E_UNTRUSTED_AXIOM"
            },
            if admission {
                "This proof is INCOMPLETE — it rests on an admission (`sorry`), which is an \
                 axiom that proves everything. Adding the axiom to the prelude would make the \
                 admission invisible to every certificate minted afterwards"
            } else {
                "Checking this term would enlarge the trusted base. Adding the axiom to \
                 `StandardLibrary::register` is the one repair that must not be made: the \
                 prelude version would still describe the axiom set the certificate was NOT \
                 checked against"
            },
        ));
    }
    None
}

/// The error for a name that resolved nowhere (see [`unbound_diagnosis`]).
fn classify_unbound(name: &str) -> KernelError {
    match unbound_diagnosis(name) {
        Some(msg) => KernelError::CertificationError(msg),
        None => KernelError::UnboundVariable(name.to_string()),
    }
}

/// Look up a name as a universe-polymorphic reference: its DECLARED parameter list (present
/// only where a declaration STATED one) and its stored type. `None` when the name resolves
/// nowhere.
///
/// The registries stay disjoint on purpose — `get_global` must not see a universe-polymorphic
/// definition, because one is δ-transparent and has no constructors — so the lookup is ordered
/// rather than merged. [`Context::poly_declaration`] is that order for the two registries that
/// carry a LEVEL TELESCOPE: a polymorphic TYPE declaration (a body-less inductive, structure or
/// class) outranks a polymorphic DEFINITION, exactly as a header outranks a body in
/// `get_global`.
///
/// ★ READING THE STATED TELESCOPE IS NOT BOOKKEEPING. Where no list is stored the telescope is
/// DERIVED from the free level variables of the stored type in first-occurrence order
/// (`term::level_telescope`), and the two orders coincide for every declaration whose arity
/// happens to mention its parameters in declaration order — which is almost all of them, and is
/// why reading only the derived order survived this long. `Swapped.{u,v} : Sort v → Sort u →
/// Sort (max u v)` derives `[v, u]`, so `Swapped.{0,3}` came out with its two binders EXCHANGED:
/// a well-typed term at a type its declaration does not have, with no diagnostic anywhere, and
/// with the second kernel deriving the same wrong order and reporting `Agreed`. Lean cannot
/// reach that state: `ConstantInfo.levelParams` is stored on every declaration and
/// `Level.instantiateParams` (`Lean/Level.lean:594`) zips THAT list positionally, never one
/// recovered from the type.
fn lookup_const(ctx: &Context, name: &str) -> Option<(Option<Vec<String>>, Term)> {
    if let Some((params, ty)) = ctx.poly_declaration(name) {
        return Some((Some(params.to_vec()), ty.clone()));
    }
    let ty = ctx.get_global(name)?;
    // A `Const` is the UNIVERSE-POLYMORPHIC reference form, and `name.{}` is not a second
    // spelling of `Global(name)`: a monomorphic declaration has no level telescope, so a
    // reference that carries one is a statement about a declaration that does not exist. Lean
    // conflates the two (`Expr.const n []` IS the constant); LOGOS keeps them apart on purpose,
    // and `lean_conformance_t00_terms::a_monomorphic_reference_resolves_as_a_global_and_a_const_at_no_levels_does_not`
    // is the lock. The telescope is what makes a declaration reachable here — which is exactly
    // what lets an AXIOM, an INDUCTIVE or a STRUCTURE be universe-polymorphic without entering
    // the definition-only `universe_polys` map.
    if !crate::term::mentions_level_var(ty) {
        return None;
    }
    Some((None, ty.clone()))
}

/// Turn the level variables of an APPLICATION HEAD's type into fresh metavariables, so the
/// application rule can solve them from the argument.
///
/// Leaving them as ordinary variables is what pinned a `Sort u` declaration to one
/// instantiation: `Type 0 ⋠ u` (a variable may be `Prop`), so `Box Nat` was refused while
/// `Box True` was accepted — one declaration, one usable level. Defaulting them to `Type 0` is
/// the opposite error, and the worse one, because cumulativity then absorbs the difference
/// silently.
///
/// The rewrite fires ONLY for a head that is a bare `Term::Global`. That restriction is what
/// keeps it honest: a level variable anywhere else — in the body of the polymorphic definition
/// currently being checked, say — is genuinely BOUND by its own declaration and a use site has
/// no business solving it. A `Term::Const` head already carries explicit levels and is left
/// alone; a partially-applied head keeps whatever metavariables the earlier application left
/// unsolved, so a spine solves its levels left to right.
pub(crate) fn implicit_levels_for_head(func: &Term, func_type: Term) -> Term {
    let Term::Global(name) = func else {
        return func_type;
    };
    // The guard is allocation-free and short-circuiting: this question is asked at EVERY
    // application, and almost every declaration is monomorphic, so learning "no" must cost
    // nothing. Collecting the telescope only happens for the declarations that have one.
    if !crate::term::mentions_level_var(&func_type) {
        return func_type;
    }
    let params = crate::term::free_level_vars(&func_type);
    if params.is_empty() {
        return func_type;
    }
    let subst = crate::term::fresh_level_metas(name, &params);
    crate::term::instantiate_universes(&func_type, &subst)
}

/// Solve the level metavariables of an application's DOMAIN from the argument's actual type,
/// and propagate the solution into the codomain.
///
/// This chooses an instantiation; it does not accept one. The caller still runs `check_type`
/// against the substituted domain, so an unsolvable or unhelpful choice makes the application
/// FAIL. A domain with no metavariables is returned untouched, so the ordinary path never pays
/// for this.
fn solve_domain_levels(
    ctx: &Context,
    param_type: &Term,
    body_type: &Term,
    arg: &Term,
) -> Option<(Term, Term)> {
    let subst = domain_level_solution(ctx, param_type, arg)?;
    Some((
        crate::term::instantiate_universes(param_type, &subst),
        crate::term::instantiate_universes(body_type, &subst),
    ))
}

/// The levels `arg` solves for the metavariables of the domain `param_type` — the application
/// rule's one decision. [`solve_domain_levels`] applies it; `implicit_instance_levels` records it,
/// so conversion reads a bare spine's instance from exactly the rule that typed the spine. `None`
/// when the domain has no metavariables or nothing was solved.
fn domain_level_solution(
    ctx: &Context,
    param_type: &Term,
    arg: &Term,
) -> Option<std::collections::HashMap<String, Universe>> {
    if !crate::term::mentions_level_meta(param_type) {
        return None;
    }
    // The argument's type is only a HINT here; if it cannot be inferred (a `Hole`, a term that
    // is checkable but not inferrable) the metavariables stay unsolved and `check_type` reports
    // the real failure.
    let arg_type = infer_type(ctx, arg).ok()?;
    let arg_type = match arg_type {
        Term::Sort(_) => arg_type,
        other => normalize(ctx, &other),
    };
    let mut subst = std::collections::HashMap::new();
    crate::term::unify_levels(param_type, &arg_type, &mut subst);
    (!subst.is_empty()).then_some(subst)
}

/// A term with no proper subterms — copying one is free, so sharing it would only add noise.
fn is_atomic(t: &Term) -> bool {
    matches!(
        t,
        Term::Var(_)
            | Term::Global(_)
            | Term::Sort(_)
            | Term::Lit(_)
            | Term::Const { .. }
            | Term::Hole
    )
}

/// The smallest argument [`substitute_shared`] will consider sharing.
///
/// Below this, the two walks the sharing decision costs are more expensive than the copies they
/// would save, so the argument is simply copied — which keeps the overwhelmingly common
/// application byte-identical to eager substitution and costs it nothing to find out.
const SHARE_MIN_ARGUMENT_NODES: usize = 16;

/// Whether `t` has more than `limit` nodes.
///
/// Recursive with an early exit and no allocation: the App rule asks this at every node, so a
/// `Vec` here would be a malloc per application. The recursion is bounded by the kernel's own
/// depth door, and it stops the moment the budget is spent — so a huge argument costs `limit`
/// nodes to reject, not its own size.
fn size_exceeds(t: &Term, limit: usize) -> bool {
    fn walk(t: &Term, left: &mut usize) -> bool {
        if *left == 0 {
            return true;
        }
        *left -= 1;
        match t {
            Term::Var(_) | Term::Global(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole => false,
            Term::Const { .. } => false,
            Term::App(f, a) => walk(f, left) || walk(a, left),
            Term::Pi {
                param_type,
                body_type,
                ..
            } => walk(param_type, left) || walk(body_type, left),
            Term::Lambda {
                param_type, body, ..
            } => walk(param_type, left) || walk(body, left),
            Term::Fix { body, .. } => walk(body, left),
            Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| walk(b, left)),
            Term::Let {
                ty, value, body, ..
            } => walk(ty, left) || walk(value, left) || walk(body, left),
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                walk(discriminant, left)
                    || walk(motive, left)
                    || cases.iter().any(|c| walk(c, left))
            }
            Term::Annotated { term, .. } => {
                *left += 1;
                walk(term, left)
            }
        }
    }
    let mut left = limit;
    walk(t, &mut left)
}

/// The node count of a term — the cost model behind [`substitute_shared`]'s decision.
fn node_count(t: &Term) -> usize {
    match t {
        Term::Var(_) | Term::Global(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole => 1,
        Term::Const { .. } => 1,
        Term::App(f, a) => 1 + node_count(f) + node_count(a),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => 1 + node_count(param_type) + node_count(body_type),
        Term::Lambda {
            param_type, body, ..
        } => 1 + node_count(param_type) + node_count(body),
        Term::Fix { body, .. } => 1 + node_count(body),
        Term::MutualFix { defs, .. } => 1 + defs.iter().map(|(_, b)| node_count(b)).sum::<usize>(),
        Term::Let {
            ty, value, body, ..
        } => 1 + node_count(ty) + node_count(value) + node_count(body),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            1 + node_count(discriminant)
                + node_count(motive)
                + cases.iter().map(node_count).sum::<usize>()
        }
        Term::Annotated { term, .. } => node_count(term),
    }
}

/// How many FREE occurrences of `var` a term has, plus the term's own node count — one walk,
/// because the sharing decision needs both.
fn body_stats(term: &Term, var: &str) -> (usize, usize) {
    match term {
        Term::Var(name) => (usize::from(name == var), 1),
        Term::Global(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole | Term::Const { .. } => (0, 1),
        Term::App(f, a) => {
            let (o1, s1) = body_stats(f, var);
            let (o2, s2) = body_stats(a, var);
            (o1 + o2, 1 + s1 + s2)
        }
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            let (o1, s1) = body_stats(param_type, var);
            let (o2, s2) = body_stats(body_type, var);
            (o1 + if param == var { 0 } else { o2 }, 1 + s1 + s2)
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            let (o1, s1) = body_stats(param_type, var);
            let (o2, s2) = body_stats(body, var);
            (o1 + if param == var { 0 } else { o2 }, 1 + s1 + s2)
        }
        Term::Fix { name, body } => {
            let (o, s) = body_stats(body, var);
            (if name == var { 0 } else { o }, 1 + s)
        }
        Term::MutualFix { defs, .. } => {
            let shadowed = defs.iter().any(|(n, _)| n == var);
            let mut occ = 0;
            let mut size = 1;
            for (_, b) in defs {
                let (o, s) = body_stats(b, var);
                if !shadowed {
                    occ += o;
                }
                size += s;
            }
            (occ, size)
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            let (o1, s1) = body_stats(ty, var);
            let (o2, s2) = body_stats(value, var);
            let (o3, s3) = body_stats(body, var);
            (o1 + o2 + if name == var { 0 } else { o3 }, 1 + s1 + s2 + s3)
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            let (o1, s1) = body_stats(discriminant, var);
            let (o2, s2) = body_stats(motive, var);
            let mut occ = o1 + o2;
            let mut size = 1 + s1 + s2;
            for c in cases {
                let (o, s) = body_stats(c, var);
                occ += o;
                size += s;
            }
            (occ, size)
        }
        Term::Annotated { term, .. } => body_stats(term, var),
    }
}

/// Substitute `replacement` for `var` in `body`, WRITING IT ONCE when copying it would blow the
/// result up.
///
/// `Term` is a named tree with no sharing, so the ordinary substitution materialises a full
/// clone of the argument at every occurrence. In a DEPENDENT telescope — a `variable`-heavy
/// section, a bundled algebraic structure, Mathlib's daily shape — the bound variable occurs
/// once or twice per binder, so one argument becomes 2n copies and the inferred type is
/// superlinear in the telescope depth. Lean pays nothing for this: its `Expr` is a hash-consed
/// DAG over de Bruijn indices.
///
/// LOGOS's equivalent is ζ: `let x : A := a in B` is definitionally `B[x := a]`, and it writes
/// `a` ONCE. The decision is a COST MODEL rather than a threshold — share exactly when the
/// duplication would more than double the result — so the common case (a small argument, or one
/// occurrence) is byte-identical to eager substitution and nothing downstream sees a change.
/// Instantiate a Pi codomain while retaining the kernel's sharing strategy.
/// This constructs a type expression only; callers must separately establish
/// that the function has this Pi type and the argument has its domain.
pub fn instantiate_codomain(param: &str, domain: &Term, body: &Term, argument: &Term) -> Term {
    expose_head(substitute_shared(body, param, domain, argument))
}

fn substitute_shared(body: &Term, var: &str, var_type: &Term, replacement: &Term) -> Term {
    // THREE CHEAP GATES BEFORE THE ACCOUNTING. The App rule runs this at every node of every
    // application, so the accounting below — two full walks — may only be paid where it can pay
    // for itself. An atomic argument is free to copy; an argument the body does not mention is
    // not copied at all (`substitute`'s own short-circuit); and an argument small enough that
    // the accounting costs more than the copies it would save is copied. Skipping these made
    // registering the standard library two orders of magnitude slower.
    if is_atomic(replacement) {
        return substitute(body, var, replacement);
    }
    if !occurs_free(body, var) {
        return body.clone();
    }
    if !size_exceeds(replacement, SHARE_MIN_ARGUMENT_NODES) {
        return substitute(body, var, replacement);
    }
    let (occurrences, body_size) = body_stats(body, var);
    let repl_size = node_count(replacement);
    if (occurrences - 1) * repl_size <= body_size + repl_size {
        return substitute(body, var, replacement);
    }
    Term::Let {
        name: var.to_string(),
        ty: Box::new(var_type.clone()),
        value: Box::new(replacement.clone()),
        body: Box::new(body.clone()),
    }
}

/// Peel shared bindings off the HEAD of a type so `infer_type` never hands back a `Let`-headed
/// one.
///
/// Every consumer of an inferred type matches on its head — `Pi` to apply another argument,
/// `Sort` to read a level, a `Global` spine to find an inductive. A shared binding at the head
/// would break all of them, so it is pushed inward instead: the immediate DOMAIN is substituted
/// (the caller needs it concrete to check the next argument against), and the binding is
/// re-established over the codomain, where it is still written once. Each application therefore
/// costs one copy of the argument, not one per remaining binder.
///
/// A binding whose variable the body does not use is DROPPED rather than carried, so a fully
/// applied type reports `Prop`, not `let A := … in Prop`.
///
/// A β-redex at the head breaks those consumers the same way, and instantiating a codomain
/// makes one whenever the argument substituted was a λ heading an application — a motive `P`
/// in `P x` is the everyday case. It is contracted here ([`head_beta`]), as Lean's instantiation
/// contracts it: `Nat.rec (motive := fun _ => Nat) z s n` infers `Nat`, not `(fun _ => Nat) n`.
fn expose_head(t: Term) -> Term {
    let Term::Let {
        name,
        ty,
        value,
        body,
    } = t
    else {
        // The overwhelmingly common shape: nothing shared and no redex at the head, so the type
        // is handed straight back by MOVE.
        return head_beta(t);
    };
    match expose_head(*body) {
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            // Pushing the binding under the Π moves `value` under `param`'s binder, so a
            // `param` that occurs free in `value` (or shadows `name`) must be renamed first.
            let mut value_fvs = free_vars(&value);
            // The shared variable itself must be avoided too: the renamed binder lands in the
            // same scope as the ζ-binding, so a collision there would capture it.
            value_fvs.insert(name.clone());
            let (param, body_type) = if param == name || value_fvs.contains(&param) {
                let fresh = fresh_name(&param, &value_fvs);
                let renamed = substitute(&body_type, &param, &Term::Var(fresh.clone()));
                (fresh, renamed)
            } else {
                (param, *body_type)
            };
            let new_domain = substitute(&param_type, &name, &value);
            let new_codomain = if occurs_free(&body_type, &name) {
                Term::Let {
                    name,
                    ty,
                    value,
                    body: Box::new(body_type),
                }
            } else {
                body_type
            };
            Term::Pi {
                param,
                param_type: Box::new(new_domain),
                body_type: Box::new(new_codomain),
            }
        }
        other => head_beta(substitute(&other, &name, &value)),
    }
}

/// The most β-steps [`head_beta`] takes before it hands the type back as it stands. A type
/// reaching it was built from checked parts, so its head reduction is short; the bound exists
/// so that no input can make the kernel spin, and stopping early is harmless — the result is
/// still convertible to the type, merely less exposed.
const MAX_HEAD_BETA_STEPS: usize = 4096;

/// Contract the β-redexes at the HEAD of `t`: `(λx. B) a ā ⤳ B[x := a] ā`, repeated while the
/// head is a λ with an argument waiting. Only the head spine is touched; anything else, and a
/// term with no redex at its head, is returned by move.
fn head_beta(t: Term) -> Term {
    fn head_is_lambda(mut t: &Term) -> bool {
        while let Term::App(f, _) = t {
            t = f;
        }
        matches!(t, Term::Lambda { .. })
    }
    if !matches!(t, Term::App(..)) || !head_is_lambda(&t) {
        return t;
    }
    // The spine's arguments, LAST first, so the next argument to consume is `pop`.
    let mut pending: Vec<Term> = Vec::new();
    let mut head = t;
    while let Term::App(f, a) = head {
        pending.push(*a);
        head = *f;
    }
    for _ in 0..MAX_HEAD_BETA_STEPS {
        let Term::Lambda {
            param,
            param_type,
            body,
        } = head
        else {
            break;
        };
        let Some(argument) = pending.pop() else {
            head = Term::Lambda {
                param,
                param_type,
                body,
            };
            break;
        };
        head = substitute(&body, &param, &argument);
        while let Term::App(f, a) = head {
            pending.push(*a);
            head = *f;
        }
    }
    pending
        .into_iter()
        .rev()
        .fold(head, |f, a| Term::App(Box::new(f), Box::new(a)))
}

/// Whether `var` occurs free in `term` (binder-aware, short-circuiting).
pub(crate) fn occurs_free(term: &Term, var: &str) -> bool {
    match term {
        Term::Var(name) => name == var,
        Term::Sort(_) | Term::Lit(_) | Term::Hole | Term::Global(_) | Term::Const { .. } => false,
        Term::App(func, arg) => occurs_free(func, var) || occurs_free(arg, var),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => occurs_free(param_type, var) || (param != var && occurs_free(body_type, var)),
        Term::Lambda {
            param,
            param_type,
            body,
        } => occurs_free(param_type, var) || (param != var && occurs_free(body, var)),
        Term::Fix { name, body } => name != var && occurs_free(body, var),
        Term::MutualFix { defs, .. } => {
            // `var` is free only if it is not one of the (all-binding) def names AND
            // occurs free in some body.
            !defs.iter().any(|(n, _)| n == var) && defs.iter().any(|(_, b)| occurs_free(b, var))
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            occurs_free(ty, var)
                || occurs_free(value, var)
                || (name != var && occurs_free(body, var))
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            occurs_free(discriminant, var)
                || occurs_free(motive, var)
                || cases.iter().any(|c| occurs_free(c, var))
        }
        Term::Annotated { term, .. } => occurs_free(term, var),
    }
}

/// Collect the free variables of a term (named representation).
pub(crate) fn free_vars(term: &Term) -> std::collections::HashSet<String> {
    fn go(term: &Term, bound: &mut Vec<String>, acc: &mut std::collections::HashSet<String>) {
        match term {
            Term::Var(name) => {
                if !bound.iter().any(|b| b == name) {
                    acc.insert(name.clone());
                }
            }
            Term::Sort(_) | Term::Lit(_) | Term::Hole | Term::Global(_) | Term::Const { .. } => {}
            Term::App(func, arg) => {
                go(func, bound, acc);
                go(arg, bound, acc);
            }
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                go(param_type, bound, acc);
                bound.push(param.clone());
                go(body_type, bound, acc);
                bound.pop();
            }
            Term::Lambda {
                param,
                param_type,
                body,
            } => {
                go(param_type, bound, acc);
                bound.push(param.clone());
                go(body, bound, acc);
                bound.pop();
            }
            Term::Fix { name, body } => {
                bound.push(name.clone());
                go(body, bound, acc);
                bound.pop();
            }
            Term::MutualFix { defs, .. } => {
                for (n, _) in defs {
                    bound.push(n.clone());
                }
                for (_, b) in defs {
                    go(b, bound, acc);
                }
                for _ in defs {
                    bound.pop();
                }
            }
            Term::Let {
                name,
                ty,
                value,
                body,
            } => {
                go(ty, bound, acc);
                go(value, bound, acc);
                bound.push(name.clone());
                go(body, bound, acc);
                bound.pop();
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                go(discriminant, bound, acc);
                go(motive, bound, acc);
                for c in cases {
                    go(c, bound, acc);
                }
            }
            Term::Annotated { term, .. } => go(term, bound, acc),
        }
    }
    let mut acc = std::collections::HashSet::new();
    let mut bound = Vec::new();
    go(term, &mut bound, &mut acc);
    acc
}

/// Collect every variable name appearing in a term (bound or free).
fn all_var_names(term: &Term, acc: &mut std::collections::HashSet<String>) {
    match term {
        Term::Var(name) => {
            acc.insert(name.clone());
        }
        Term::Sort(_) | Term::Lit(_) | Term::Hole | Term::Global(_) | Term::Const { .. } => {}
        Term::App(func, arg) => {
            all_var_names(func, acc);
            all_var_names(arg, acc);
        }
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            acc.insert(param.clone());
            all_var_names(param_type, acc);
            all_var_names(body_type, acc);
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            acc.insert(param.clone());
            all_var_names(param_type, acc);
            all_var_names(body, acc);
        }
        Term::Fix { name, body } => {
            acc.insert(name.clone());
            all_var_names(body, acc);
        }
        Term::MutualFix { defs, .. } => {
            for (n, b) in defs {
                acc.insert(n.clone());
                all_var_names(b, acc);
            }
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            acc.insert(name.clone());
            all_var_names(ty, acc);
            all_var_names(value, acc);
            all_var_names(body, acc);
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            all_var_names(discriminant, acc);
            all_var_names(motive, acc);
            for c in cases {
                all_var_names(c, acc);
            }
        }
        Term::Annotated { term, .. } => all_var_names(term, acc),
    }
}

/// Pick a binder name derived from `base` that collides with nothing in `avoid`.
fn fresh_name(base: &str, avoid: &std::collections::HashSet<String>) -> String {
    let mut candidate = format!("{}'", base);
    let mut counter: u32 = 0;
    while avoid.contains(&candidate) {
        counter += 1;
        candidate = format!("{}'{}", base, counter);
    }
    candidate
}

/// Fresh for scoped syntax and active local keys AND stored local types.
fn fresh_context_name(ctx: &Context, base: &str, terms: &[&Term]) -> String {
    let mut avoid = std::collections::HashSet::new();
    for term in terms {
        all_var_names(term, &mut avoid);
    }
    let mut name = fresh_name(base, &avoid);
    while avoid.contains(&name) || ctx.local_name_in_use(&name) {
        name.push('_');
    }
    name
}

/// A lexical binder may not overwrite a name needed by a stored local type.
/// Preserve borrowed syntax when unused; its own annotation is never renamed.
fn open_context_binder<'a>(
    ctx: &Context,
    name: &'a str,
    domain: &Term,
    body: &'a Term,
) -> (std::borrow::Cow<'a, str>, std::borrow::Cow<'a, Term>) {
    use std::borrow::Cow;
    if !binder_would_capture(ctx, name, domain) {
        return (Cow::Borrowed(name), Cow::Borrowed(body));
    }
    let fresh = fresh_context_name(ctx, name, &[domain, body]);
    let opened = rename_var(body, name, &fresh);
    (Cow::Owned(fresh), Cow::Owned(opened))
}

/// Whether entering a binder named `name` over `domain` would capture an outer occurrence.
///
/// Two occurrences can be captured: one free in a stored local type, which would re-point at the
/// new binder, and one free in the binder's own domain, which is read in the OUTER scope — but
/// only while an outer local of that name exists, since otherwise the domain is unbound anyway.
/// Merely shadowing an outer local that nothing mentions captures nothing: the outer binding is
/// unreachable under the binder, exactly as with de Bruijn indices, so the author's binder name
/// is kept — the inferred type of `λ(x:A). λ(x:A). x` is `Π(x:A). Π(x:A). A`, as in Lean.
fn binder_would_capture(ctx: &Context, name: &str, domain: &Term) -> bool {
    ctx.local_types_mention(name) || (ctx.has_local(name) && occurs_free(domain, name))
}

/// Choose all targets before renaming mutually scoped recursive bodies.
fn open_context_mutual<'a>(
    ctx: &Context,
    defs: &'a [(String, Term)],
) -> std::borrow::Cow<'a, [(String, Term)]> {
    use std::borrow::Cow;
    if !defs.iter().any(|(name, _)| ctx.local_name_in_use(name)) {
        return Cow::Borrowed(defs);
    }
    let mut avoid = std::collections::HashSet::new();
    for (name, body) in defs {
        avoid.insert(name.clone());
        all_var_names(body, &mut avoid);
    }
    let mut replacements = Vec::new();
    for (name, _) in defs {
        if ctx.local_name_in_use(name) && !replacements.iter().any(|(old, _)| old == name) {
            let mut fresh = fresh_name(name, &avoid);
            while avoid.contains(&fresh) || ctx.local_name_in_use(&fresh) {
                fresh.push('_');
            }
            avoid.insert(fresh.clone());
            replacements.push((name.clone(), fresh));
        }
    }
    Cow::Owned(
        defs.iter()
            .map(|(name, body)| {
                let mut opened = body.clone();
                for (old, fresh) in &replacements {
                    opened = rename_var(&opened, old, fresh);
                }
                let name = replacements
                    .iter()
                    .find(|(old, _)| old == name)
                    .map_or_else(|| name.clone(), |(_, fresh)| fresh.clone());
                (name, opened)
            })
            .collect(),
    )
}

/// Choose a fresh binder name for `param`, avoiding the replacement's free vars,
/// every name occurring in `body`, and the variable being substituted.
fn freshen(
    param: &str,
    body: &Term,
    replacement_fvs: &std::collections::HashSet<String>,
    var: &str,
) -> String {
    let mut avoid = replacement_fvs.clone();
    all_var_names(body, &mut avoid);
    avoid.insert(var.to_string());
    fresh_name(param, &avoid)
}

/// Rename free occurrences of `from` to `to` in `term`. `to` must be globally
/// fresh in `term`, so this rename cannot itself capture.
fn rename_var(term: &Term, from: &str, to: &str) -> Term {
    let repl = Term::Var(to.to_string());
    let mut fvs = std::collections::HashSet::new();
    fvs.insert(to.to_string());
    substitute_avoiding(term, from, &repl, &fvs)
}

/// Capture-avoiding substitution `body[var := replacement]`, where
/// `replacement_fvs` is the precomputed free-variable set of `replacement`.
///
/// When a binder (`Pi`/`Lambda`/`Fix`) would capture a free variable of
/// `replacement`, the binder is alpha-renamed to a fresh name before the
/// substitution descends into its body.
/// Consume the body and final replacement occurrence using the same capture-
/// avoiding binder traversal as borrowed substitution. The caller supplies the
/// exact lexical occurrence count after checking its own output-size bound.
pub(crate) fn substitute_owned_value_counted(
    body: Term,
    var: &str,
    replacement: Term,
    occurrences: usize,
) -> Term {
    if occurrences == 0 {
        return body;
    }
    let free = free_vars(&replacement);
    let mut value = SubstitutionValue::Owned {
        term: Some(replacement),
        remaining: occurrences,
    };
    let result = substitute_input(SubstitutionInput::Owned(body), var, &mut value, &free);
    debug_assert!(matches!(
        value,
        SubstitutionValue::Owned { remaining: 0, .. }
    ));
    result
}

/// With a closed replacement, the final lexical occurrence can take its
/// already-owned storage. Earlier occurrences still receive distinct clones.
/// The same binder traversal serves borrowed, body-owned and fully-owned input.
pub(crate) fn substitute_owned_closed_value_counted(
    body: Term,
    var: &str,
    replacement: Term,
    occurrences: usize,
) -> Term {
    let remaining = occurrences;
    let mut value = SubstitutionValue::Owned {
        term: Some(replacement),
        remaining,
    };
    let result = substitute_input(
        SubstitutionInput::Owned(body),
        var,
        &mut value,
        &std::collections::HashSet::new(),
    );
    debug_assert!(matches!(
        value,
        SubstitutionValue::Owned { remaining: 0, .. }
    ));
    result
}

enum SubstitutionValue<'a> {
    Borrowed(&'a Term),
    Owned {
        term: Option<Term>,
        remaining: usize,
    },
}
impl SubstitutionValue<'_> {
    fn materialize(&mut self) -> Term {
        match self {
            Self::Borrowed(term) => {
                #[cfg(test)]
                record_substitution_tree(term);
                (*term).clone()
            }
            Self::Owned { term, remaining } => {
                assert!(
                    *remaining > 0,
                    "lexical occurrence count must cover substitution"
                );
                *remaining -= 1;
                if *remaining == 0 {
                    term.take().expect("owned replacement available")
                } else {
                    let term = term.as_ref().expect("owned replacement retained");
                    #[cfg(test)]
                    record_substitution_tree(term);
                    term.clone()
                }
            }
        }
    }
}
fn substitute_avoiding(
    body: &Term,
    var: &str,
    replacement: &Term,
    replacement_fvs: &std::collections::HashSet<String>,
) -> Term {
    substitute_input(
        SubstitutionInput::Borrowed(body),
        var,
        &mut SubstitutionValue::Borrowed(replacement),
        replacement_fvs,
    )
}

/// Capture-avoiding substitution that reuses the owned body's containers.
/// The borrowed replacement follows the same freshening and occurrence rules
/// as public substitution, including when the target variable is absent.
pub(crate) fn substitute_owned(body: Term, var: &str, replacement: &Term) -> Term {
    if !occurs_free(&body, var) {
        return body;
    }
    let free = free_vars(replacement);
    substitute_input(
        SubstitutionInput::Owned(body),
        var,
        &mut SubstitutionValue::Borrowed(replacement),
        &free,
    )
}

/// The caller has already established lexical closedness of the replacement.
/// This owns the body and retains its existing containers during substitution.
pub(crate) fn substitute_owned_closed(body: Term, var: &str, replacement: &Term) -> Term {
    substitute_input(
        SubstitutionInput::Owned(body),
        var,
        &mut SubstitutionValue::Borrowed(replacement),
        &std::collections::HashSet::new(),
    )
}

enum SubstitutionInput<'a> {
    Borrowed(&'a Term),
    Owned(Term),
}

enum SubstitutionChild<'a> {
    Borrowed(&'a Term),
    Owned(Box<Term>),
}

enum SubstitutionCases<'a> {
    Borrowed(&'a [Term]),
    Owned(Vec<Term>),
}

enum SubstitutionDefinitions<'a> {
    Borrowed(&'a [(String, Term)]),
    Owned(Vec<(String, Term)>),
}

// This view separates storage ownership from the single binder algorithm below.
enum SubstitutionView<'a> {
    Atom(SubstitutionInput<'a>),
    Binder {
        pi: bool,
        name: std::borrow::Cow<'a, str>,
        domain: SubstitutionChild<'a>,
        body: SubstitutionChild<'a>,
    },
    App(SubstitutionChild<'a>, SubstitutionChild<'a>),
    Match {
        discriminant: SubstitutionChild<'a>,
        motive: SubstitutionChild<'a>,
        cases: SubstitutionCases<'a>,
    },
    Fix {
        name: std::borrow::Cow<'a, str>,
        body: SubstitutionChild<'a>,
    },
    MutualFix {
        defs: SubstitutionDefinitions<'a>,
        index: usize,
    },
    Let {
        name: std::borrow::Cow<'a, str>,
        ty: SubstitutionChild<'a>,
        value: SubstitutionChild<'a>,
        body: SubstitutionChild<'a>,
    },
    Annotated {
        annotation: std::borrow::Cow<'a, crate::Annotation>,
        term: SubstitutionChild<'a>,
    },
}

impl<'a> SubstitutionInput<'a> {
    fn view(self) -> SubstitutionView<'a> {
        use std::borrow::Cow;
        use SubstitutionChild::{Borrowed as B, Owned as O};
        match self {
            Self::Borrowed(term) => match term {
                Term::Pi {
                    param,
                    param_type,
                    body_type,
                } => SubstitutionView::Binder {
                    pi: true,
                    name: Cow::Borrowed(param),
                    domain: B(param_type),
                    body: B(body_type),
                },
                Term::Lambda {
                    param,
                    param_type,
                    body,
                } => SubstitutionView::Binder {
                    pi: false,
                    name: Cow::Borrowed(param),
                    domain: B(param_type),
                    body: B(body),
                },
                Term::App(f, a) => SubstitutionView::App(B(f), B(a)),
                Term::Match {
                    discriminant,
                    motive,
                    cases,
                } => SubstitutionView::Match {
                    discriminant: B(discriminant),
                    motive: B(motive),
                    cases: SubstitutionCases::Borrowed(cases),
                },
                Term::Fix { name, body } => SubstitutionView::Fix {
                    name: Cow::Borrowed(name),
                    body: B(body),
                },
                Term::MutualFix { defs, index } => SubstitutionView::MutualFix {
                    defs: SubstitutionDefinitions::Borrowed(defs),
                    index: *index,
                },
                Term::Let {
                    name,
                    ty,
                    value,
                    body,
                } => SubstitutionView::Let {
                    name: Cow::Borrowed(name),
                    ty: B(ty),
                    value: B(value),
                    body: B(body),
                },
                Term::Annotated { annotation, term } => SubstitutionView::Annotated {
                    annotation: Cow::Borrowed(annotation),
                    term: B(term),
                },
                _ => SubstitutionView::Atom(Self::Borrowed(term)),
            },
            Self::Owned(term) => match term {
                Term::Pi {
                    param,
                    param_type,
                    body_type,
                } => SubstitutionView::Binder {
                    pi: true,
                    name: Cow::Owned(param),
                    domain: O(param_type),
                    body: O(body_type),
                },
                Term::Lambda {
                    param,
                    param_type,
                    body,
                } => SubstitutionView::Binder {
                    pi: false,
                    name: Cow::Owned(param),
                    domain: O(param_type),
                    body: O(body),
                },
                Term::App(f, a) => SubstitutionView::App(O(f), O(a)),
                Term::Match {
                    discriminant,
                    motive,
                    cases,
                } => SubstitutionView::Match {
                    discriminant: O(discriminant),
                    motive: O(motive),
                    cases: SubstitutionCases::Owned(cases),
                },
                Term::Fix { name, body } => SubstitutionView::Fix {
                    name: Cow::Owned(name),
                    body: O(body),
                },
                Term::MutualFix { defs, index } => SubstitutionView::MutualFix {
                    defs: SubstitutionDefinitions::Owned(defs),
                    index,
                },
                Term::Let {
                    name,
                    ty,
                    value,
                    body,
                } => SubstitutionView::Let {
                    name: Cow::Owned(name),
                    ty: O(ty),
                    value: O(value),
                    body: O(body),
                },
                Term::Annotated { annotation, term } => SubstitutionView::Annotated {
                    annotation: Cow::Owned(annotation),
                    term: O(term),
                },
                _ => SubstitutionView::Atom(Self::Owned(term)),
            },
        }
    }
}

impl<'a> SubstitutionChild<'a> {
    fn as_term(&self) -> &Term {
        match self {
            Self::Borrowed(t) => t,
            Self::Owned(t) => t,
        }
    }

    fn keep(self) -> Box<Term> {
        match self {
            Self::Borrowed(t) => {
                #[cfg(test)]
                record_substitution_tree(t);
                Box::new(t.clone())
            }
            Self::Owned(t) => t,
        }
    }

    fn apply(
        self,
        var: &str,
        value: &mut SubstitutionValue<'_>,
        fvs: &std::collections::HashSet<String>,
    ) -> Box<Term> {
        match self {
            Self::Borrowed(t) => Box::new(substitute_input(
                SubstitutionInput::Borrowed(t),
                var,
                value,
                fvs,
            )),
            Self::Owned(mut t) => {
                let input = std::mem::replace(t.as_mut(), Term::Hole);
                *t = substitute_input(SubstitutionInput::Owned(input), var, value, fvs);
                t
            }
        }
    }

    fn rename(self, old: &str, new: &str) -> Self {
        match self {
            Self::Borrowed(t) => Self::Owned(Box::new(rename_var(t, old, new))),
            Self::Owned(mut t) => {
                *t = rename_var(&t, old, new);
                Self::Owned(t)
            }
        }
    }
}

fn substitute_binder<'a>(
    mut name: std::borrow::Cow<'a, str>,
    mut body: SubstitutionChild<'a>,
    var: &str,
    value: &mut SubstitutionValue<'_>,
    fvs: &std::collections::HashSet<String>,
) -> (String, Box<Term>) {
    if name.as_ref() == var {
        return (name.into_owned(), body.keep());
    }
    if fvs.contains(name.as_ref()) {
        let fresh = freshen(&name, body.as_term(), fvs, var);
        body = body.rename(&name, &fresh);
        name = std::borrow::Cow::Owned(fresh);
    }
    (name.into_owned(), body.apply(var, value, fvs))
}

#[cfg(test)]
thread_local! {
    static SUBSTITUTION_COPY_WORK: std::cell::Cell<(usize, usize)> =
        const { std::cell::Cell::new((0, 0)) };
}

#[cfg(test)]
pub(crate) fn take_substitution_copy_work() -> (usize, usize) {
    SUBSTITUTION_COPY_WORK.with(|work| work.replace((0, 0)))
}

#[cfg(test)]
fn record_substitution_node(term: &Term) {
    let bytes = match term {
        Term::Var(name)
        | Term::Global(name)
        | Term::Const { name, .. }
        | Term::Fix { name, .. }
        | Term::Let { name, .. } => name.len(),
        Term::Pi { param, .. } | Term::Lambda { param, .. } => param.len(),
        Term::MutualFix { defs, .. } => defs.iter().map(|(name, _)| name.len()).sum(),
        _ => 0,
    };
    SUBSTITUTION_COPY_WORK.with(|work| {
        let (nodes, names) = work.get();
        work.set((nodes + 1, names + bytes));
    });
}

#[cfg(test)]
fn record_substitution_tree(term: &Term) {
    record_substitution_node(term);
    match term {
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            record_substitution_tree(param_type);
            record_substitution_tree(body_type);
        }
        Term::Lambda {
            param_type, body, ..
        } => {
            record_substitution_tree(param_type);
            record_substitution_tree(body);
        }
        Term::App(f, a) => {
            record_substitution_tree(f);
            record_substitution_tree(a);
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            record_substitution_tree(discriminant);
            record_substitution_tree(motive);
            for case in cases {
                record_substitution_tree(case);
            }
        }
        Term::Fix { body, .. } => record_substitution_tree(body),
        Term::MutualFix { defs, .. } => {
            for (_, body) in defs {
                record_substitution_tree(body);
            }
        }
        Term::Let {
            ty, value, body, ..
        } => {
            record_substitution_tree(ty);
            record_substitution_tree(value);
            record_substitution_tree(body);
        }
        Term::Annotated { term, .. } => record_substitution_tree(term),
        Term::Sort(_)
        | Term::Var(_)
        | Term::Global(_)
        | Term::Const { .. }
        | Term::Lit(_)
        | Term::Hole => {}
    }
}

fn substitute_input(
    input: SubstitutionInput<'_>,
    var: &str,
    value: &mut SubstitutionValue<'_>,
    fvs: &std::collections::HashSet<String>,
) -> Term {
    // Counters observe actual borrowed-node reconstruction and replacement
    // clones. Identifier bytes exclude literal and universe payloads.
    #[cfg(test)]
    if let SubstitutionInput::Borrowed(term) = &input {
        if !matches!(term, Term::Var(name) if name == var) {
            record_substitution_node(term);
        }
    }
    match input.view() {
        SubstitutionView::Atom(input) => {
            let term = match &input {
                SubstitutionInput::Borrowed(t) => *t,
                SubstitutionInput::Owned(t) => t,
            };
            if matches!(term, Term::Var(name) if name == var) {
                value.materialize()
            } else {
                match input {
                    SubstitutionInput::Borrowed(t) => t.clone(),
                    SubstitutionInput::Owned(t) => t,
                }
            }
        }
        SubstitutionView::Binder {
            pi,
            name,
            domain,
            body,
        } => {
            let param_type = domain.apply(var, value, fvs);
            let (param, body) = substitute_binder(name, body, var, value, fvs);
            if pi {
                Term::Pi {
                    param,
                    param_type,
                    body_type: body,
                }
            } else {
                Term::Lambda {
                    param,
                    param_type,
                    body,
                }
            }
        }
        SubstitutionView::App(f, a) => {
            Term::App(f.apply(var, value, fvs), a.apply(var, value, fvs))
        }
        SubstitutionView::Match {
            discriminant,
            motive,
            cases,
        } => {
            let discriminant = discriminant.apply(var, value, fvs);
            let motive = motive.apply(var, value, fvs);
            let cases = match cases {
                SubstitutionCases::Borrowed(cases) => cases
                    .iter()
                    .map(|case| {
                        substitute_input(SubstitutionInput::Borrowed(case), var, value, fvs)
                    })
                    .collect(),
                SubstitutionCases::Owned(mut cases) => {
                    for case in &mut cases {
                        let input = std::mem::replace(case, Term::Hole);
                        *case = substitute_input(SubstitutionInput::Owned(input), var, value, fvs);
                    }
                    cases
                }
            };
            Term::Match {
                discriminant,
                motive,
                cases,
            }
        }
        SubstitutionView::Fix { name, body } => {
            let (name, body) = substitute_binder(name, body, var, value, fvs);
            Term::Fix { name, body }
        }
        SubstitutionView::MutualFix { defs, index } => {
            let mut defs = match defs {
                SubstitutionDefinitions::Borrowed(defs) => {
                    #[cfg(test)]
                    for (_, body) in defs {
                        record_substitution_tree(body);
                    }
                    defs.to_vec()
                }
                SubstitutionDefinitions::Owned(defs) => defs,
            };
            if defs.iter().any(|(name, _)| name == var) {
                return Term::MutualFix { defs, index };
            }
            // All mutual names bind in every body. Freshen each conflicting
            // name consistently across the block before substituting any body.
            for i in 0..defs.len() {
                if fvs.contains(&defs[i].0) {
                    let mut avoid = fvs.clone();
                    for (name, body) in &defs {
                        all_var_names(body, &mut avoid);
                        avoid.insert(name.clone());
                    }
                    avoid.insert(var.to_string());
                    let fresh = fresh_name(&defs[i].0, &avoid);
                    for j in 0..defs.len() {
                        defs[j].1 = rename_var(&defs[j].1, &defs[i].0, &fresh);
                    }
                    defs[i].0 = fresh;
                }
            }
            for (_, body) in &mut defs {
                let input = std::mem::replace(body, Term::Hole);
                *body = substitute_input(SubstitutionInput::Owned(input), var, value, fvs);
            }
            Term::MutualFix { defs, index }
        }
        SubstitutionView::Let {
            name,
            ty,
            value: binding,
            body,
        } => {
            let ty = ty.apply(var, value, fvs);
            let binding = binding.apply(var, value, fvs);
            let (name, body) = substitute_binder(name, body, var, value, fvs);
            Term::Let {
                name,
                ty,
                value: binding,
                body,
            }
        }
        SubstitutionView::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.into_owned(),
            term: term.apply(var, value, fvs),
        },
    }
}

/// Check if type `a` is a subtype of type `b` (cumulativity).
///
/// Implements the subtyping relation for the Calculus of Constructions
/// with cumulative universes.
///
/// # Subtyping Rules
///
/// - **Universe cumulativity**: `Type i <= Type j` if `i <= j`
/// - **Pi contravariance**: `Π(x:A). B <= Π(x:A'). B'` if `A' <= A` and `B <= B'`
/// - **Structural equality**: Other terms are compared after normalization
///
/// # Normalization
///
/// Both types are normalized before comparison, ensuring that definitionally
/// equal types are recognized as subtypes.
///
/// # Cumulativity Examples
///
/// - `Type 0 <= Type 1` (lower universe is subtype of higher)
/// - `Nat -> Type 0 <= Nat -> Type 1` (covariant in return type)
/// - `Type 1 -> Nat <= Type 0 -> Nat` (contravariant in parameter type)
pub fn is_subtype(ctx: &Context, a: &Term, b: &Term) -> bool {
    if let Some(r) = crate::annotation::erased_pair(a, b, |a, b| is_subtype(ctx, a, b)) {
        return r;
    }
    // The sized stack, BEFORE the α fast path: `types_equal` descends both terms with real
    // frames, so a deep pair aborts the process inside the shortcut rather than inside the
    // comparison it was shortcutting. `deepen_stack_all` sizes for the DEEPER of the two, in
    // one pass, and short-circuits once a kernel call is already under way.
    if !inside_kernel() {
        if let Some(r) = crate::term_store::deepen_stack_all(&[a, b], || is_subtype(ctx, a, b)) {
            return r;
        }
    }
    // Fast path: already structurally (definitionally) equal — the overwhelming case
    // in propositional/FOL proofs, where types are atoms and ∧/∨/¬/→ in normal form.
    // Skipping the two `normalize` walks here is the difference between linear and
    // pathological checking on a large certified grid proof.
    if alpha_eq(a, b) {
        return true;
    }
    // A term too deep to descend is REFUSED, and a refused conversion is "not equal" — never
    // "equal", which would certify a comparison the kernel never actually made.
    if enter_kernel(a, "is_subtype").is_err() || enter_kernel(b, "is_subtype").is_err() {
        return false;
    }
    if crate::conversion_session::known(ctx, a, b) {
        return true;
    }
    if let Some(equal) = rigid_conversion(ctx, a, b) {
        return equal;
    }
    if conversion_by_congruence(ctx, a, b) {
        return true;
    }
    if conversion_by_unfolded_head(ctx, a, b) {
        return true;
    }
    // Otherwise normalize both terms before comparison — this ensures that e.g.
    // `ReachesOne (collatzStep 2)` equals `ReachesOne 1`.
    if crate::term_store::beta_zeta_equivalent(ctx, a, b, 1 << 16) {
        return true;
    }
    if crate::term_store::beta_delta_equivalent(ctx, a, b, 1 << 16) {
        return true;
    }
    let a_norm = normalize(ctx, a);
    let b_norm = normalize(ctx, b);

    is_subtype_normalized(ctx, &a_norm, &b_norm)
}

/// Check subtyping on already-normalized terms. Cumulativity lives ONLY here: universes
/// follow `Prop ≤ Type i ≤ Type j`, and a `Π` is contravariant in its domain and covariant
/// in its codomain. Every other position is INVARIANT and delegates to [`def_eq_normalized`]
/// (definitional equality: reduction + η + proof irrelevance) — using cumulative subtyping
/// in an invariant position (e.g. a function argument) would be unsound.
fn is_subtype_normalized(ctx: &Context, a: &Term, b: &Term) -> bool {
    // ζ ON DEMAND. `reduction`'s ζ rule now KEEPS a `Let` whose variable occurs more than once
    // — the binding IS the sharing node, and writing the value out at every occurrence is the
    // quadratic blow-up it exists to avoid — so a `Let` survives into a normal form and
    // conversion has to be able to see through one. Without this step two convertible terms,
    // one of which kept its binding, compare UNEQUAL, and sharing would change what a type
    // MEANS, which is the one thing an optimisation may never do. Expanding is always sound:
    // `let x := v in b` and `b[x := v]` are the SAME term by ζ, in every calculus that has it.
    // Each expansion removes one binder from the head, so the recursion terminates.
    if let Some(expanded) = crate::term_store::zeta_expand_head_term(a) {
        return is_subtype_normalized(ctx, &normalize(ctx, &expanded), b);
    }
    if let Some(expanded) = crate::term_store::zeta_expand_head_term(b) {
        return is_subtype_normalized(ctx, a, &normalize(ctx, &expanded));
    }
    match (a, b) {
        // Universe subtyping
        (Term::Sort(u1), Term::Sort(u2)) => u1.is_subtype_of(u2),

        // Pi subtyping (contravariant in param, covariant in body)
        (
            Term::Pi {
                param: p1,
                param_type: t1,
                body_type: b1,
            },
            Term::Pi {
                param: p2,
                param_type: t2,
                body_type: b2,
            },
        ) => {
            // Contravariant: t2 ≤ t1 (the expected param can be more specific)
            is_subtype_normalized(ctx, t2, t1) && {
                // Covariant: b1 ≤ b2 (alpha-rename to compare bodies, under the binder)
                let (name, lhs, rhs) = open_conversion_binders(Some(ctx), p1, b1, p2, b2);
                let ext = ctx.extend(&name, (**t1).clone());
                is_subtype_normalized(&ext, &lhs, &rhs)
            }
        }

        // Everything else is invariant: definitional equality.
        _ => def_eq_normalized(ctx, a, b),
    }
}

#[cfg(test)]
thread_local! {
    static CONVERSION_FULL_NORMALIZATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static CONVERSION_PROOF_TYPE_PROBES: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Every full-normalization fallback this process has entered, on any thread. The per-thread
/// count cannot see a Lean import: the importer and the kernel's sized stacks run on threads of
/// their own. A control that reads this runs its import alone, in a child process.
#[cfg(test)]
static PROCESS_FULL_NORMALIZATIONS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Definitional equality (symmetric): reduction, congruence, η, and proof irrelevance. This
/// is the conversion used in INVARIANT positions (function arguments, `Lambda`/`Match`/`Fix`
/// subterms). `is_subtype` layers cumulativity on top of it.
pub(crate) fn def_eq(ctx: &Context, a: &Term, b: &Term) -> bool {
    if let Some(r) = crate::annotation::erased_pair(a, b, |a, b| def_eq(ctx, a, b)) {
        return r;
    }
    within_conversion_rungs(|| {
        crate::conversion_session::convert(ctx, a, b, || def_eq_uncached(ctx, a, b))
    })
}

thread_local! {
    /// The conversions nested inside one another on this thread, one rung of stack each.
    static CONVERSION_NESTING: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The conversions enclosing the current one.
pub(crate) fn conversion_nesting() -> usize {
    CONVERSION_NESTING.with(|n| n.get())
}

/// One nesting of conversion, returned even when the conversion unwinds.
struct ConversionRung(usize);

impl Drop for ConversionRung {
    fn drop(&mut self) {
        CONVERSION_NESTING.with(|n| n.set(self.0));
    }
}

/// Run one conversion nested in its callers. Conversions recurse through their arguments and
/// through the reducts their probes expose, so their nesting is bounded by neither input's depth:
/// comparing `Nat.below m n` against `Nat.below m k` for two large literals descends once per
/// `Succ`. Each nesting takes one rung of this thread's stack (the kernel's own unit,
/// `term_store::BYTES_PER_RUNG`). When the rungs run out the conversion continues on a stack twice
/// the size, as the normalizer grows its own, carrying the count along; past the kernel's
/// term-depth door (`MAX_TERM_DEPTH` nestings) it stops as a recorded resource stop — "not
/// shown equal", which `exact` reports as a `ResourceLimit` decline — never a crash.
pub(crate) fn within_conversion_rungs(f: impl FnOnce() -> bool + Send) -> bool {
    let taken = conversion_nesting();
    if taken >= MAX_TERM_DEPTH {
        crate::reduction::record_resource_stop(crate::Unreduced::TooDeep);
        return false;
    }
    let rungs = crate::term_store::kernel_stack_rungs();
    if taken + 1 >= rungs && rungs < MAX_TERM_DEPTH {
        return crate::term_store::grow_stack_for_depth(
            (rungs * 2).min(MAX_TERM_DEPTH),
            move || {
                CONVERSION_NESTING.with(|n| n.set(taken));
                within_conversion_rungs(f)
            },
        );
    }
    let _rung = ConversionRung(taken);
    CONVERSION_NESTING.with(|n| n.set(taken + 1));
    f()
}

fn def_eq_uncached(ctx: &Context, a: &Term, b: &Term) -> bool {
    if alpha_eq(a, b) {
        return true;
    }
    if enter_kernel(a, "def_eq").is_err() || enter_kernel(b, "def_eq").is_err() {
        return false;
    }
    // Matching binder heads can compare domains and capture-safely opened
    // bodies directly. Do this before allocating stores for every telescope
    // suffix; it is the same positive-only congruence rule used below.
    let matching_binders = matches!(
        (a, b),
        (Term::Pi { .. }, Term::Pi { .. }) | (Term::Lambda { .. }, Term::Lambda { .. })
    );
    if matching_binders && conversion_by_congruence(ctx, a, b) {
        return true;
    }
    if let Some(equal) = rigid_conversion(ctx, a, b) {
        return equal;
    }
    if let Some(equal) = crate::reduction::natural_literal_conversion(ctx, a, b, 1 << 18) {
        return equal;
    }
    if crate::reduction::constructor_equivalent(ctx, a, b, 1 << 26) {
        return true;
    }
    if crate::term_store::beta_zeta_equivalent(ctx, a, b, 1 << 16) {
        return true;
    }
    if crate::term_store::beta_delta_equivalent(ctx, a, b, 1 << 16) {
        return true;
    }
    if !matching_binders && conversion_by_congruence(ctx, a, b) {
        return true;
    }
    // A bounded common reduct can settle open recursor-iota comparisons without
    // recursively typing both endpoints merely to rule out proof irrelevance.
    // Keep the result: the same probe must not run again after an inconclusive
    // attempt, and an ineligible pair must retain the head fallback's old gate.
    let bounded_head = bounded_head_conversion(ctx, a, b);
    if bounded_head == Some(true) {
        return true;
    }
    // Lean's `is_def_eq_proof_irrel`: two proofs are equal exactly when their propositions are —
    // by irrelevance when the propositions convert, and never when they differ, since convertible
    // terms have convertible types. Neither proof is ever unfolded.
    if let Some(equal) = proof_irrelevance(ctx, a, b) {
        return equal;
    }
    if bounded_head == Some(false) && conversion_by_exposed_heads(ctx, a, b) {
        return true;
    }
    // Closed data whose weak-head values differ at a constructor is refuted as Lean refutes it,
    // never by normalizing both sides (LG; `constructor_refutation_controls`).
    if crate::reduction::constructors_disagree(ctx, a, b, 1 << 26) {
        return false;
    }
    // LAZY δ (Lean's `lazy_delta_reduction`, LG): weak-head reduce, compare heads, and unfold one
    // side at a time by definitional height, theorems last. Undecided pairs fall to the
    // normalization below, unchanged.
    if let Some(answer) = lazy_delta::lazy_delta_after_the_probes(ctx, a, b) {
        #[cfg(debug_assertions)]
        lazy_delta::differential(ctx, a, b, answer);
        return answer;
    }
    // LAZY THEOREM DELTA (Lean's `lazy_delta_reduction`: a theorem has the lowest unfolding
    // priority). Checked theorems are published unfoldably, as Lean publishes them, but a
    // conversion almost never needs to look inside one — proofs are compared by irrelevance —
    // and unfolding every theorem inside every proof argument makes normalization explode.
    // So normalize first with theorems FOLDED and compare; unfold them only if that fails,
    // continuing from the folded normal forms. Sound by construction: the folded forms are
    // reducts of the originals (a subset of the same reductions), so an equality found there
    // is an equality of the originals, and fully normalizing a reduct reaches the same normal
    // form as normalizing the original — no work is repeated and no equality is invented.
    #[cfg(test)]
    {
        CONVERSION_FULL_NORMALIZATIONS.with(|count| count.set(count.get() + 1));
        PROCESS_FULL_NORMALIZATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    let folded = |name: &str| is_transparent_theorem(ctx, name);
    let a_folded =
        crate::reduction::normalize_at(ctx, a, crate::reduction::Transparency::Excluding(&folded));
    let b_folded =
        crate::reduction::normalize_at(ctx, b, crate::reduction::Transparency::Excluding(&folded));
    if def_eq_normalized(ctx, &a_folded, &b_folded) {
        return true;
    }
    let a_norm = normalize(ctx, &a_folded);
    let b_norm = normalize(ctx, &b_folded);
    def_eq_normalized(ctx, &a_norm, &b_norm)
}

/// Whether `name` is a checked theorem published with its proof as a transparent body.
pub(crate) fn is_transparent_theorem(ctx: &Context, name: &str) -> bool {
    ctx.checked_opaque_dependencies(name).is_some() && ctx.get_definition_body(name).is_some()
}

// Transparent aliases can conceal a common inductive head. Expose that head
// before normalizing its arguments, so invariant fields use ordinary conversion
// (including the shared computation path) rather than eager tree evaluation.
fn conversion_by_unfolded_head(ctx: &Context, a: &Term, b: &Term) -> bool {
    match bounded_head_conversion(ctx, a, b) {
        Some(true) => true,
        Some(false) => conversion_by_exposed_heads(ctx, a, b),
        None => false,
    }
}

// None preserves the existing hole/rewrite admission gate. Some(false) means
// only that the single bounded attempt was inconclusive, never inequality.
fn bounded_head_conversion(ctx: &Context, a: &Term, b: &Term) -> Option<bool> {
    // This positive-only gateway must not certify an erased Hole or run
    // outside the head probe's rewrite policy. Literal identity is unchanged.
    if !ctx.simp_rules().is_empty() || mentions_hole(a) || mentions_hole(b) {
        return None;
    }
    // A beta application or a closed wrapper can hide the common function.
    // Compare after each bounded step before unfolding that common function:
    // eager head exposure can otherwise expand all of its shared arguments.
    let mut budget = 256;
    Some(bounded_conversion(ctx, a, b, &mut budget))
}

// Call only after an eligible, inconclusive bounded_head_conversion. This
// broader fallback may recursively invoke full conversion through rigid heads;
// def_eq therefore keeps it after proof irrelevance, unlike the bounded probe.
fn conversion_by_exposed_heads(ctx: &Context, a: &Term, b: &Term) -> bool {
    let Some(lhs) = crate::reduction::bounded_conversion_head(ctx, a) else {
        return false;
    };
    let Some(rhs) = crate::reduction::bounded_conversion_head(ctx, b) else {
        return false;
    };
    if alpha_eq(a, &lhs) && alpha_eq(b, &rhs) {
        return false;
    }
    alpha_eq(&lhs, &rhs)
        || rigid_conversion(ctx, &lhs, &rhs) == Some(true)
        || conversion_by_congruence(ctx, &lhs, &rhs)
}

/// Binder congruence can establish equality without evaluating annotations.
/// Failure is inconclusive: eta, proof irrelevance and reduction may still
/// equate the terms. Arbitrary function arguments are deliberately left lazy;
/// a function may discard an argument whose evaluation is very expensive.
fn conversion_by_congruence(ctx: &Context, a: &Term, b: &Term) -> bool {
    let binders = match (a, b) {
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
        ) => Some((p, pt, pb, q, qt, qb)),
        _ => None,
    };
    if let Some((p, pt, pb, q, qt, qb)) = binders {
        if !def_eq(ctx, pt, qt) {
            return false;
        }
        let name = fresh_context_name(ctx, "__conversion", &[pb, qb, pt, qt]);
        let variable = Term::Var(name.clone());
        let lhs = substitute(pb, p, &variable);
        let rhs = substitute(qb, q, &variable);
        return def_eq(&ctx.extend(&name, (**pt).clone()), &lhs, &rhs);
    }
    // Congruence over a defined function is only an opportunistic proof:
    // its arguments may be discarded after unfolding the head. Never invoke
    // unrestricted conversion on those arguments before reducing the function.
    let (ah, aa) = spine_of(a);
    let (bh, ba) = spine_of(b);
    if !aa.is_empty() && aa.len() == ba.len() && alpha_eq(ah, bh) {
        let mut budget = 256;
        return aa
            .iter()
            .zip(ba)
            .all(|(x, y)| bounded_conversion(ctx, x, y, &mut budget));
    }
    false
}

/// Predict only a fixed global head after syntactic beta/zeta prefixes. This is
/// a scheduling hint, never conversion evidence: substitution cannot alter a
/// Global/Const, whereas a variable head is deliberately inconclusive. Every
/// inspected node uses the caller's existing conversion fuel.
#[cfg(test)]
fn delta_exposes_head(candidate: &Term, other: &Term, budget: &mut usize) -> bool {
    delta_exposes_head_through(None, candidate, other, budget)
}

fn delta_exposes_head_through<'a>(
    ctx: Option<&'a Context>,
    candidate: &'a Term,
    other: &Term,
    budget: &mut usize,
) -> bool {
    let mut target = other;
    loop {
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        match target {
            Term::App(f, _) => target = f,
            Term::Global(_) | Term::Const { .. } => break,
            _ => return false,
        }
    }
    let mut term = candidate;
    let mut applied = 0usize;
    loop {
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        match term {
            Term::App(f, _) => {
                applied += 1;
                term = f;
            }
            Term::Lambda { body, .. } if applied > 0 => {
                applied -= 1;
                term = body;
            }
            Term::Let { body, .. } => term = body,
            Term::Global(name) => {
                if alpha_eq(term, target) {
                    return true;
                }
                // Follow only the same borrowed monomorphic body lookup used by
                // bounded delta. Pending/opaque names have no visible body.
                // No substitution is needed to predict a fixed global head;
                // the actual conversion still validates every unfolding.
                let Some(body) = ctx.and_then(|ctx| ctx.get_definition_body(name)) else {
                    return false;
                };
                term = body;
            }
            // Polymorphic delta requires universe instantiation; leave it to
            // the existing reducer instead of guessing a substituted head.
            Term::Const { .. } => return alpha_eq(term, target),
            _ => return false,
        }
    }
}

/// Compare closed helper bodies before specializing their arguments. This is
/// only a positive identity check, with the same admission as bounded delta.
/// Kernel conversion uses full transparency; pending/opaque bodies are hidden
/// by get_definition_body. Explicit universe constants keep the resolver path.
fn bounded_closed_head_identity(ctx: &Context, a: &Term, b: &Term, budget: &mut usize) -> bool {
    if !matches!(
        (a, b),
        (Term::Global(_), Term::Global(_) | Term::Lambda { .. })
            | (Term::Lambda { .. }, Term::Global(_))
    ) {
        return false;
    }
    fn observe<'a>(ctx: &'a Context, term: &'a Term, budget: &mut usize) -> Option<&'a Term> {
        let body = match term {
            Term::Global(name) => ctx.get_definition_body(name)?,
            Term::Lambda { .. } => term,
            _ => return None,
        };
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        crate::reduction::bounded_closed_conversion_size(body)?;
        Some(body)
    }
    let Some(a) = observe(ctx, a, budget) else {
        return false;
    };
    let Some(b) = observe(ctx, b, budget) else {
        return false;
    };
    // alpha_eq preserves the kernel's binder, universe and literal semantics.
    // Its bounded body traversal may freshen binders; it is not allocation-free.
    alpha_eq(a, b)
}

struct BorrowedCongruence {
    equal: bool,
    fixed_heads: (bool, bool),
}

/// Borrowed congruence leaves common heads folded. A failed argument comparison
/// is inconclusive: the caller may still prove equality by erasing it in beta/delta.
#[cfg(test)]
thread_local! {
    static BORROWED_STRUCTURAL_VISITS: std::cell::Cell<Option<usize>> =
        const {std::cell::Cell::new(None)};
}

fn bounded_structural_congruence(
    ctx: &Context,
    a: &Term,
    b: &Term,
    budget: &mut usize,
) -> BorrowedCongruence {
    #[cfg(test)]
    BORROWED_STRUCTURAL_VISITS.with(|count| {
        if let Some(n) = count.get() {
            count.set(Some(n + 1));
        }
    });
    // Preserve only observations already made by this structural probe. A
    // Global/Const-rooted App spine is inert under beta/iota/zeta, even when
    // this comparison fails; it may still require delta and argument checking.
    let fixed = |head: &Term| matches!(head, Term::Global(_) | Term::Const { .. });
    let mut fixed_heads = (fixed(a), fixed(b));
    let equal = match (a, b) {
        (Term::App(..), Term::App(..)) => {
            let (ah, aa) = spine_of(a);
            let (bh, ba) = spine_of(b);
            fixed_heads = (fixed(ah), fixed(bh));
            aa.len() == ba.len()
                && (alpha_eq(ah, bh) || bounded_closed_head_identity(ctx, ah, bh, budget))
                && aa
                    .iter()
                    .zip(ba)
                    .all(|(x, y)| bounded_conversion(ctx, x, y, budget))
        }
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
        ) => {
            if bounded_conversion(ctx, pt, qt, budget) {
                let (name, lhs, rhs) = open_conversion_binders(Some(ctx), p, pb, q, qb);
                bounded_conversion(&ctx.extend(&name, (**pt).clone()), &lhs, &rhs, budget)
            } else {
                false
            }
        }
        _ => bounded_closed_head_identity(ctx, a, b, budget),
    };
    BorrowedCongruence { equal, fixed_heads }
}

/// Positive-only congruence using bounded beta/iota/zeta and closed head delta.
/// Exhaustion or an unsupported rule is inconclusive. Prefer exposing a common
/// fixed head; otherwise expose the smaller definition before a costly body.
pub(crate) fn bounded_conversion(ctx: &Context, a: &Term, b: &Term, budget: &mut usize) -> bool {
    if *budget == 0 {
        return false;
    }
    *budget -= 1;
    if alpha_eq(a, b) {
        return true;
    }
    let observed = bounded_structural_congruence(ctx, a, b, budget);
    if observed.equal {
        return true;
    }
    // The fixedness observation comes from the spine already visited above.
    // Retain the same whole-input admission before avoiding the reducer's
    // provably unchanged copy/traversal. This is not evidence of equality.
    fn expose<'a>(
        ctx: &Context,
        term: &'a Term,
        fixed: bool,
        budget: &mut usize,
    ) -> Option<std::borrow::Cow<'a, Term>> {
        if fixed {
            crate::reduction::bounded_conversion_size(term)?;
            Some(std::borrow::Cow::Borrowed(term))
        } else {
            crate::reduction::beta_iota_zeta_head(ctx, term, budget).map(std::borrow::Cow::Owned)
        }
    }
    let Some(a) = expose(ctx, a, observed.fixed_heads.0, budget) else {
        return false;
    };
    let Some(b) = expose(ctx, b, observed.fixed_heads.1, budget) else {
        return false;
    };
    // Two borrowed exposures preserve both terms. Repeating the failed
    // structural probe would revisit the same obligations with less fuel.
    // Either newly reduced term still needs fresh equality/congruence checks.
    if !matches!(
        (&a, &b),
        (std::borrow::Cow::Borrowed(_), std::borrow::Cow::Borrowed(_))
    ) {
        if alpha_eq(&a, &b) {
            return true;
        }
        if bounded_structural_congruence(ctx, &a, &b, budget).equal {
            return true;
        }
    }
    if *budget == 0 {
        return false;
    }
    match (
        crate::reduction::bounded_delta_head(ctx, &a),
        crate::reduction::bounded_delta_head(ctx, &b),
    ) {
        (Some((ac, ua)), Some((bc, ub))) => {
            // Inspect only the candidate deferred by the size heuristic, without
            // substituting or evaluating either definition speculatively. The
            // recursive conversion still checks every resulting obligation.
            if ac <= bc {
                if delta_exposes_head_through(Some(ctx), &ub, &a, budget) {
                    bounded_conversion(ctx, &a, &ub, budget)
                } else {
                    bounded_conversion(ctx, &ua, &b, budget)
                }
            } else if delta_exposes_head_through(Some(ctx), &ua, &b, budget) {
                bounded_conversion(ctx, &ua, &b, budget)
            } else {
                bounded_conversion(ctx, &a, &ub, budget)
            }
        }
        (Some((_, unfolded)), None) => bounded_conversion(ctx, &unfolded, &b, budget),
        (None, Some((_, unfolded))) => bounded_conversion(ctx, &a, &unfolded, budget),
        (None, None) => false,
    }
}

/// Injective inductive heads and data constructors need only compare arguments.
/// Trying shallow arguments first can refute a pair without evaluating its other
/// field. Proof constructors are handled by proof irrelevance before this rule.
fn rigid_conversion(ctx: &Context, a: &Term, b: &Term) -> Option<bool> {
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
    let mut order: Vec<usize> = (0..aa.len()).collect();
    order.sort_by_key(|&i| {
        crate::term_store::term_depth(aa[i]) + crate::term_store::term_depth(ba[i])
    });
    Some(order.into_iter().all(|i| def_eq(ctx, aa[i], ba[i])))
}

/// Decompose a term into its head and argument spine (`f a b c` → `(f, [a,b,c])`).
fn spine_of(t: &Term) -> (&Term, Vec<&Term>) {
    let mut args = Vec::new();
    let mut head = t;
    while let Term::App(f, a) = head {
        args.push(a.as_ref());
        head = f;
    }
    args.reverse();
    (head, args)
}

/// Structure η in ONE direction: if `mk_term` is a fully-applied constructor of a
/// registered structure `S` (`S_mk p̄ ā`) and `other` is not itself constructor-
/// headed, return `Some(eq)` where `eq` decides `mk_term ≡ other` by comparing each
/// field `aᵢ` against `S_projᵢ p̄ other`. `None` when the shape does not apply, so the
/// caller falls through to ordinary congruence.
fn try_structure_eta(ctx: &Context, mk_term: &Term, other: &Term) -> Option<bool> {
    let (head, args) = spine_of(mk_term);
    let (hname, levels) = match head {
        Term::Global(n) => (n, None),
        Term::Const { name, levels } => (name, Some(levels)),
        _ => return None,
    };
    let (_sname, info) = ctx.struct_of_constructor(hname)?;
    let nfields = info.projections.len();
    // Must be fully applied: params + one argument per field.
    if args.len() != info.num_params + nfields {
        return None;
    }
    // Do not eta when `other` is ALSO this constructor — that is ordinary
    // congruence (and avoids a pointless expansion loop).
    let (ohead, _) = spine_of(other);
    if matches!(ohead, Term::Global(n) | Term::Const {name:n, ..} if n == hname) {
        return None;
    }
    let params = &args[..info.num_params];
    let field_args = &args[info.num_params..];
    // Each field argument must equal the projection of `other`.
    Some(info.projections.iter().enumerate().all(|(i, proj)| {
        let mut proj_applied = match levels {
            Some(levels) => Term::Const {
                name: proj.clone(),
                levels: levels.clone(),
            },
            None => Term::Global(proj.clone()),
        };
        for p in params {
            proj_applied = Term::App(Box::new(proj_applied), Box::new((*p).clone()));
        }
        proj_applied = Term::App(Box::new(proj_applied), Box::new(other.clone()));
        def_eq(ctx, field_args[i], &proj_applied)
    }))
}

/// `n a₁ … aₖ ≡ n.{ℓ̄} b₁ … bₖ` for a bare reference `n` whose declaration carries a level
/// telescope: exactly when the levels the application rule solves for the bare spine are
/// equivalent, position by position, to `ℓ̄`, and `aᵢ ≡ bᵢ`. A level the spine does not determine
/// leaves the bare reference unsolved, and then nothing is concluded — the conservative direction.
/// A DIFFERENT explicit level never converts: `Eq.{u+2} X` is well formed by cumulativity, but it
/// is not the constant `refl.{u+1} X x` proves an instance of. With no arguments the bare
/// reference stands at its declaration's own levels (`term`'s rule 4), so `n ≡ n.{ℓ̄}` exactly
/// when `ℓ̄` is that telescope; an application's head never reaches this case, because
/// congruence does not compare a bare head with an explicit one ([`bare_against_explicit`]).
fn implicit_instance_conversion(ctx: &Context, a: &Term, b: &Term) -> bool {
    if !bare_against_explicit(a, b) {
        return false;
    }
    let (ah, aa) = spine_of(a);
    let (bh, ba) = spine_of(b);
    if aa.len() != ba.len() {
        return false;
    }
    let (name, levels, bare_args) = match (ah, bh) {
        (Term::Global(n), Term::Const { name, levels }) if n == name => (name, levels, &aa),
        (Term::Const { name, levels }, Term::Global(n)) if n == name => (name, levels, &ba),
        _ => return false,
    };
    if bare_args.is_empty() {
        return lookup_const(ctx, name).is_some_and(|(declared, const_type)| {
            let own = crate::term::level_telescope(declared.as_deref(), &const_type);
            own.len() == levels.len()
                && own
                    .iter()
                    .zip(levels)
                    .all(|(p, l)| l.equiv(&Universe::Var(p.clone())))
        });
    }
    implicit_instance_levels(ctx, name, bare_args).is_some_and(|solved| {
        solved.len() == levels.len() && solved.iter().zip(levels).all(|(x, y)| x.equiv(y))
    }) && aa.iter().zip(ba.iter()).all(|(x, y)| def_eq(ctx, x, y))
}

/// Whether the spines of `a` and `b` are headed by the same name, bare in one and explicit in the
/// other — allocation-free, since conversion asks it of every pair it reaches. Such a pair converts
/// only by [`implicit_instance_conversion`], which reads the bare head's levels from its arguments.
fn bare_against_explicit(a: &Term, b: &Term) -> bool {
    fn head(t: &Term) -> &Term {
        let mut h = t;
        while let Term::App(f, _) = h {
            h = f;
        }
        h
    }
    matches!(
        (head(a), head(b)),
        (Term::Global(n), Term::Const { name, .. }) | (Term::Const { name, .. }, Term::Global(n))
            if n == name
    )
}

/// The levels the application rule solves for the bare spine `name args`, in the positional
/// order a `Term::Const` reference to `name` zips them in (`term::resolve_const_levels`). `None`
/// when `name` has no telescope or some level stays unsolved.
///
/// The spine is walked with the application rule's own steps — the head's type exposed by
/// `demanded_type_head`, its levels turned into metavariables by `implicit_levels_for_head`, each
/// argument's solution decided by `domain_level_solution` — so the instance read here is the one
/// the typing of the same spine used, and the two cannot drift apart.
fn implicit_instance_levels(ctx: &Context, name: &str, args: &[&Term]) -> Option<Vec<Universe>> {
    let (declared, const_type) = lookup_const(ctx, name)?;
    let positional = crate::term::level_telescope(declared.as_deref(), &const_type);
    let head = Term::Global(name.to_string());
    let mut current = infer_type(ctx, &head).ok()?;
    let mut solution = std::collections::HashMap::new();
    for (i, arg) in args.iter().enumerate() {
        let exposed = demanded_type_head(ctx, current, TypeHead::Function);
        let exposed = if i == 0 {
            implicit_levels_for_head(&head, exposed)
        } else {
            exposed
        };
        let Term::Pi {
            param,
            param_type,
            body_type,
        } = exposed
        else {
            return None;
        };
        let body_type = match domain_level_solution(ctx, &param_type, arg) {
            Some(subst) => {
                let body_type = crate::term::instantiate_universes(&body_type, &subst);
                solution.extend(subst);
                body_type
            }
            None => *body_type,
        };
        current = substitute(&body_type, &param, arg);
    }
    let metas = crate::term::fresh_level_metas(name, &positional);
    positional
        .iter()
        .map(|p| {
            let solved = metas.get(p)?.substitute(&solution);
            (!crate::term::universe_mentions(&solved, &crate::term::is_level_meta))
                .then_some(solved)
        })
        .collect()
}

/// True if `t` is the head of a `Nat` Peano value — `Zero` or `Succ _` — the shape a
/// `Nat` literal bridges against (K6).
fn is_nat_peano_headed(t: &Term) -> bool {
    match t {
        Term::Global(n) => n == "Zero",
        Term::App(f, _) => matches!(f.as_ref(), Term::Global(n) if n == "Succ"),
        _ => false,
    }
}

/// One Peano-unfolding step of a `Nat` literal: `Nat(0) → Zero`, `Nat(n) → Succ (Nat(n-1))`.
fn nat_peano_step(t: &Term) -> Term {
    match t {
        // `n ≤ 0` collapses to `Zero`, so peeling always TERMINATES even on a malformed
        // negative literal (a well-formed Nat is non-negative).
        Term::Lit(Literal::Nat(n)) if *n <= logicaffeine_base::BigInt::from_i64(0) => {
            Term::Global("Zero".to_string())
        }
        Term::Lit(Literal::Nat(n)) => Term::App(
            Box::new(Term::Global("Succ".to_string())),
            Box::new(Term::Lit(Literal::Nat(
                n.sub(&logicaffeine_base::BigInt::from_i64(1)),
            ))),
        ),
        other => other.clone(),
    }
}

/// Compare a compact natural with a Peano spine without expanding the literal.
/// A mixed spine may end in another literal or an open neutral term.
/// The Peano bridge's answer for the literal `number` against the `Zero`/`Succ`-headed `other`,
/// where the literal readings it relies on hold (`int_inductive::reads_nat_literal`): all of them
/// while `Zero` and `Succ` are `Nat`'s constructors, and `0 ≡ Zero` alone while only `Zero` is.
fn nat_bridge(ctx: &Context, number: &logicaffeine_base::BigInt, other: &Term) -> Option<bool> {
    let numerals = ctx.canonical_numerals();
    if numerals.succ {
        return Some(nat_peano_equal(ctx, number, other));
    }
    (numerals.zero
        && *number <= logicaffeine_base::BigInt::from_i64(0)
        && matches!(other, Term::Global(zero) if zero == "Zero"))
    .then_some(true)
}

fn nat_peano_equal(ctx: &Context, number: &logicaffeine_base::BigInt, mut term: &Term) -> bool {
    let zero = logicaffeine_base::BigInt::from_i64(0);
    let one = logicaffeine_base::BigInt::from_i64(1);
    let mut remaining = number.clone();
    loop {
        match term {
            Term::Global(name) if name == "Zero" => return remaining == zero,
            Term::Lit(Literal::Nat(other)) => return remaining == *other,
            Term::App(head, predecessor) if matches!(head.as_ref(), Term::Global(name) if name == "Succ") =>
            {
                if remaining <= zero {
                    return false;
                }
                remaining = remaining.sub(&one);
                term = predecessor;
            }
            _ => {
                return def_eq_normalized(ctx, &Term::Lit(Literal::Nat(remaining)), term);
            }
        }
    }
}

/// Definitional equality on already-normalized terms.
fn def_eq_normalized(ctx: &Context, a: &Term, b: &Term) -> bool {
    if alpha_eq(a, b) {
        return true;
    }

    // Literal Int values are the two ordinary constructor forms, not a second carrier.
    if let Some(a_ctor) = crate::int_inductive::literal_constructor(ctx, a) {
        let b_ctor = crate::int_inductive::literal_constructor(ctx, b);
        return def_eq_normalized(ctx, &a_ctor, b_ctor.as_ref().unwrap_or(b));
    }
    if let Some(b_ctor) = crate::int_inductive::literal_constructor(ctx, b) {
        return def_eq_normalized(ctx, a, &b_ctor);
    }

    // ζ ON DEMAND. `reduction`'s ζ rule now KEEPS a `Let` whose variable occurs more than once
    // — the binding IS the sharing node, and writing the value out at every occurrence is the
    // quadratic blow-up it exists to avoid — so a `Let` survives into a normal form and
    // conversion has to be able to see through one. Without this step two convertible terms,
    // one of which kept its binding, compare UNEQUAL, and sharing would change what a type
    // MEANS, which is the one thing an optimisation may never do. Expanding is always sound:
    // `let x := v in b` and `b[x := v]` are the SAME term by ζ, in every calculus that has it.
    // Each expansion removes one binder from the head, so the recursion terminates.
    if let Some(expanded) = crate::term_store::zeta_expand_head_term(a) {
        return def_eq_normalized(ctx, &normalize(ctx, &expanded), b);
    }
    if let Some(expanded) = crate::term_store::zeta_expand_head_term(b) {
        return def_eq_normalized(ctx, a, &normalize(ctx, &expanded));
    }

    // η-conversion: `f ≡ λx. f x`. When exactly one side is a λ, compare its body against
    // the other side applied to the bound variable, under the binder.
    if let Term::Lambda {
        param,
        param_type,
        body,
    } = a
    {
        if !matches!(b, Term::Lambda { .. }) {
            let name = fresh_conversion_name(Some(ctx), &[body, b, param_type]);
            let variable = Term::Var(name.clone());
            let body = substitute(body, param, &variable);
            let ext = ctx.extend(&name, (**param_type).clone());
            let bx = normalize(&ext, &Term::App(Box::new(b.clone()), Box::new(variable)));
            return def_eq_normalized(&ext, &body, &bx);
        }
    }
    if let Term::Lambda {
        param,
        param_type,
        body,
    } = b
    {
        if !matches!(a, Term::Lambda { .. }) {
            let name = fresh_conversion_name(Some(ctx), &[body, a, param_type]);
            let variable = Term::Var(name.clone());
            let body = substitute(body, param, &variable);
            let ext = ctx.extend(&name, (**param_type).clone());
            let ax = normalize(&ext, &Term::App(Box::new(a.clone()), Box::new(variable)));
            return def_eq_normalized(&ext, &ax, &body);
        }
    }

    // Structure η: `⟨p.1, …, p.n⟩ ≡ p`. When one side is a fully-applied
    // constructor of a REGISTERED structure and the other is not constructor-
    // headed, compare each field argument against the matching projection of the
    // other side. Keyed on the structure registry, so it never fires for an
    // ordinary inductive.
    if let Some(eq) = try_structure_eta(ctx, a, b) {
        return eq;
    }
    if let Some(eq) = try_structure_eta(ctx, b, a) {
        return eq;
    }

    // A bare reference is its own implicit instantiation: `Eq X x y` denotes `Eq.{ℓ} X x y` for
    // the ℓ the application rule solves from `X` (`implicit_levels_for_head`). `refl.{ℓ}`'s
    // instantiated type mentions the bare form, a statement may spell the explicit one, and
    // Lean's `@rfl.{ℓ} α a : @Eq.{ℓ} α a a` needs the two to meet.
    if implicit_instance_conversion(ctx, a, b) {
        return true;
    }

    // Peano bridge (K6): a `Nat(n)` literal is definitionally `Succ^n Zero`. Two Nat
    // literals are equal iff their counts are; a Nat literal and a `Zero`/`Succ`-headed
    // Peano term are compared by peeling one `Succ` at a time (terminating at
    // `Nat(0) ≡ Zero`), as far as the readings it needs hold (`nat_bridge`).
    match (a, b) {
        (Term::Lit(Literal::Nat(x)), Term::Lit(Literal::Nat(y))) => return x == y,
        (Term::Lit(Literal::Nat(n)), _) if is_nat_peano_headed(b) => {
            if let Some(equal) = nat_bridge(ctx, n, b) {
                return equal;
            }
        }
        (_, Term::Lit(Literal::Nat(n))) if is_nat_peano_headed(a) => {
            if let Some(equal) = nat_bridge(ctx, n, a) {
                return equal;
            }
        }
        _ => {}
    }

    let congruent = match (a, b) {
        (Term::Sort(u1), Term::Sort(u2)) => u1.equiv(u2),
        (
            Term::Pi {
                param: p1,
                param_type: t1,
                body_type: b1,
            },
            Term::Pi {
                param: p2,
                param_type: t2,
                body_type: b2,
            },
        ) => {
            def_eq_normalized(ctx, t1, t2) && {
                let (name, lhs, rhs) = open_conversion_binders(Some(ctx), p1, b1, p2, b2);
                let ext = ctx.extend(&name, (**t1).clone());
                def_eq_normalized(&ext, &lhs, &rhs)
            }
        }
        (
            Term::Lambda {
                param: p1,
                param_type: t1,
                body: b1,
            },
            Term::Lambda {
                param: p2,
                param_type: t2,
                body: b2,
            },
        ) => {
            def_eq_normalized(ctx, t1, t2) && {
                let (name, lhs, rhs) = open_conversion_binders(Some(ctx), p1, b1, p2, b2);
                let ext = ctx.extend(&name, (**t1).clone());
                def_eq_normalized(&ext, &lhs, &rhs)
            }
        }
        (Term::App(f1, a1), Term::App(f2, a2)) => {
            !bare_against_explicit(a, b)
                && def_eq_normalized(ctx, f1, f2)
                && def_eq_normalized(ctx, a1, a2)
        }
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
            // A neutral match reduces only its discriminant. Its motive and
            // unselected branches remain unevaluated, so compare those through
            // full conversion rather than assuming they are normal forms.
            def_eq_normalized(ctx, d1, d2)
                && def_eq(ctx, m1, m2)
                && c1.len() == c2.len()
                && c1.iter().zip(c2.iter()).all(|(x, y)| def_eq(ctx, x, y))
        }
        (Term::Fix { name: n1, body: b1 }, Term::Fix { name: n2, body: b2 }) => {
            let (_, lhs, rhs) = open_conversion_binders(Some(ctx), n1, b1, n2, b2);
            // Applied fixpoints stay opaque when their structural argument is
            // neutral. Their bodies may therefore still contain redexes even
            // after the enclosing application has reached its normal form.
            def_eq(ctx, &lhs, &rhs)
        }
        _ => false,
    };
    if congruent {
        return true;
    }

    // Unit eta also compares two neutral inhabitants, neither constructor-headed.
    // A known constructor of a nonunit family cannot use this rule; avoid
    // recursively typing expanded data merely to rediscover its family.
    if can_have_unit_type(ctx, a) && can_have_unit_type(ctx, b) {
        if let Ok(a_type) = infer_type(ctx, a) {
            let a_type = normalize(ctx, &a_type);
            let (head, _) = spine_of(&a_type);
            let name = match head {
                Term::Global(n) | Term::Const { name: n, .. } => Some(n),
                _ => None,
            };
            if name.is_some_and(|n| ctx.unit_constructor(n).is_some()) {
                if let Ok(b_type) = infer_type(ctx, b) {
                    if def_eq(ctx, &a_type, &b_type) {
                        return true;
                    }
                }
            }
        }
    }

    // Proof irrelevance: any two proofs of the same proposition are equal. Fires only when
    // structural comparison fails (so it never costs on the common path).
    proof_irrelevant(ctx, a, b)
}

// Read an exact monomorphic constructor family without typing its fields.
// Unknown and polymorphic references retain the ordinary conversion probes.
fn monomorphic_constructor_parent<'a>(ctx: &'a Context, term: &Term) -> Option<&'a str> {
    // Registered conversion rules can change the interpretation of a family.
    // Keep the original inference-based probes in every such context.
    if !ctx.simp_rules().is_empty() {
        return None;
    }
    let mut remaining = 256usize;
    let mut tick = || {
        remaining = remaining.checked_sub(1)?;
        Some(())
    };
    let mut head = term;
    let mut supplied = 0usize;
    while let Term::App(function, _) = head {
        tick()?;
        supplied += 1;
        head = function;
    }
    let Term::Global(name) = head else {
        return None;
    };
    let parent = ctx.constructor_inductive(name)?;
    if !ctx.is_inductive(parent)
        || ctx.is_inductive(name)
        || ctx.is_constructor(parent)
        || ctx.is_definition(name)
        || ctx.is_definition(parent)
        || ctx.poly_declaration(name).is_some()
        || ctx.poly_declaration(parent).is_some()
    {
        return None;
    }
    // The legacy raw constructor API records an owner independently of its
    // actual type. Require a saturated visible header with that rigid result;
    // otherwise a claimed data constructor can in fact be a proof or unit value.
    let mut result = ctx.get_global(name)?;
    let mut arity = 0usize;
    while let Term::Pi { body_type, .. } = result {
        tick()?;
        arity += 1;
        result = body_type;
    }
    if arity != supplied {
        return None;
    }
    let mut result_arguments = 0usize;
    while let Term::App(function, _) = result {
        tick()?;
        result_arguments += 1;
        result = function;
    }
    if !matches!(result, Term::Global(family) if family == parent) {
        return None;
    }
    let mut family_type = ctx.get_global(parent)?;
    let mut family_arity = 0usize;
    while let Term::Pi { body_type, .. } = family_type {
        tick()?;
        family_arity += 1;
        family_type = body_type;
    }
    if family_arity != result_arguments || !matches!(family_type, Term::Sort(_)) {
        return None;
    }
    Some(parent)
}

fn is_monomorphic_data_constructor(ctx: &Context, term: &Term) -> bool {
    let Some(parent) = monomorphic_constructor_parent(ctx, term) else {
        return false;
    };
    matches!(
        ctx.get_global(parent).and_then(result_sort_universe),
        Some(Universe::Type(_))
    )
}

// Necessary result shape only: this cannot certify an application or equality.
// If this syntactic Fix application successfully infers, its structural result
// is the visible nullary data family below. No arguments or branches are typed.
fn fixed_applied_match_result_family<'a>(ctx: &Context, term: &'a Term) -> Option<&'a str> {
    if !ctx.simp_rules().is_empty() {
        return None;
    }
    let mut steps = 256usize;
    let mut tick = || {
        steps = steps.checked_sub(1)?;
        Some(())
    };
    let mut head = term;
    // Lambda wrappers turn this data result into Pi codomains. A fixed Type
    // codomain remains in Type, so the wrappers cannot turn it into a proof.
    while let Term::Lambda { body, .. } = head {
        tick()?;
        head = body;
    }
    let mut supplied = 0usize;
    while let Term::App(function, _) = head {
        tick()?;
        supplied += 1;
        head = function;
    }
    let Term::Fix { body, .. } = head else {
        return None;
    };
    let mut inner = body.as_ref();
    let mut binders = Vec::new();
    while let Term::Lambda {
        param,
        param_type,
        body,
    } = inner
    {
        tick()?;
        binders.push((param.as_str(), param_type.as_ref()));
        inner = body;
    }
    if supplied == 0 || supplied != binders.len() {
        return None;
    }
    let Term::Match {
        discriminant,
        motive,
        ..
    } = inner
    else {
        return None;
    };
    tick()?;
    // Restrict the first version to a directly bound, nullary inductive
    // discriminant. Its structural typing has no hidden index applications.
    let Term::Var(variable) = discriminant.as_ref() else {
        return None;
    };
    let mut found_domain = None;
    for (name, domain) in binders.iter().rev() {
        tick()?;
        if *name == variable {
            found_domain = Some(*domain);
            break;
        }
    }
    let domain = found_domain?;
    let Term::Global(domain_name) = domain else {
        return None;
    };
    let rigid_sort = |name: &str| {
        if !ctx.is_inductive(name)
            || ctx.is_definition(name)
            || ctx.is_constructor(name)
            || ctx.poly_declaration(name).is_some()
        {
            return None;
        }
        match ctx.get_global(name) {
            Some(Term::Sort(level)) => Some(level),
            _ => None,
        }
    };
    rigid_sort(domain_name)?;
    let result = match motive.as_ref() {
        Term::Lambda {
            param_type, body, ..
        } if matches!(param_type.as_ref(), Term::Global(name) if name == domain_name) => {
            tick()?;
            body.as_ref()
        }
        other => other,
    };
    let Term::Global(result_name) = result else {
        return None;
    };
    if !matches!(rigid_sort(result_name), Some(Universe::Type(_))) {
        return None;
    }
    Some(result_name.as_str())
}

fn can_have_unit_type(ctx: &Context, term: &Term) -> bool {
    // Successful Lambda inference has a Pi type, never an inductive unit type.
    // Registered conversion rules retain the complete inference-based path.
    if ctx.simp_rules().is_empty() && matches!(term, Term::Lambda { .. }) {
        return false;
    }
    match monomorphic_constructor_parent(ctx, term)
        .or_else(|| fixed_applied_match_result_family(ctx, term))
    {
        Some(parent) => ctx.unit_constructor(parent).is_some(),
        None => true,
    }
}

/// Proof irrelevance: `a ≡ b` when both are proofs of the same proposition — `a : A`, `b : B`,
/// `A ≡ B`, and that common type is a `Prop` (or the definitionally-irrelevant `SProp`). Sound
/// because `Prop` is a universe of proof-irrelevant propositions.
///
/// The rule is stated SYMMETRICALLY on purpose. `A ≡ B` makes `A` propositional exactly when
/// `B` is, so asking either side is the same question — but only if the kernel asks. Asking
/// only the LEFT makes the answer depend on which side of `def_eq` a term was written on
/// whenever one side's type infers and the other's does not, and `def_eq`'s own contract is
/// that it is symmetric. A conversion check that accepts a proof written one way and refuses
/// the identical proof written the other is a bug the call site cannot see.
fn proof_irrelevant(ctx: &Context, a: &Term, b: &Term) -> bool {
    proof_irrelevance(ctx, a, b) == Some(true)
}

/// Proof irrelevance as Lean's `is_def_eq_proof_irrel` decides it: `None` when neither term is a
/// proof (or a type does not infer), else whether the two propositions convert — two proofs of one
/// proposition are equal, and a proof is never equal to a term of a different type.
fn proof_irrelevance(ctx: &Context, a: &Term, b: &Term) -> Option<bool> {
    if is_monomorphic_data_constructor(ctx, a)
        || is_monomorphic_data_constructor(ctx, b)
        || fixed_applied_match_result_family(ctx, a).is_some()
        || fixed_applied_match_result_family(ctx, b).is_some()
    {
        return None;
    }
    #[cfg(test)]
    CONVERSION_PROOF_TYPE_PROBES.with(|count| {
        if let Some(n) = count.get() {
            count.set(Some(n + 1));
        }
    });
    let (Ok(ta), Ok(tb)) = (infer_type(ctx, a), infer_type(ctx, b)) else {
        return None;
    };
    let is_propositional = |t: &Term| {
        matches!(
            infer_type(ctx, t).map(|s| normalize(ctx, &s)),
            Ok(Term::Sort(level)) if level == Universe::SProp || level.equiv(&Universe::Prop)
        )
    };
    if !is_propositional(&ta) && !is_propositional(&tb) {
        return None;
    }
    Some(def_eq(ctx, &ta, &tb))
}

/// Extract the inductive type name from a type.
///
/// Handles both:
/// - Simple inductives: `Nat` → Some("Nat")
/// - Polymorphic inductives: `List A` → Some("List")
///
/// Returns None if the type is not an inductive type.
fn extract_inductive_name(ctx: &Context, ty: &Term) -> Option<String> {
    match ty {
        // Simple case: Global("Nat")
        Term::Global(name) | Term::Const { name, .. } if ctx.is_inductive(name) => {
            Some(name.clone())
        }

        // Polymorphic case: App(App(...App(Global("List"), _)...), _)
        // Recursively unwrap App to find the base Global
        Term::App(func, _) => extract_inductive_name(ctx, func),

        _ => None,
    }
}

/// Whether `t` contains a `Term::Hole` anywhere. Short-circuits on the first one.
pub(crate) fn mentions_hole(t: &Term) -> bool {
    match t {
        Term::Hole => true,
        Term::Sort(_) | Term::Var(_) | Term::Global(_) | Term::Const { .. } | Term::Lit(_) => false,
        Term::App(f, a) => mentions_hole(f) || mentions_hole(a),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => mentions_hole(param_type) || mentions_hole(body_type),
        Term::Lambda {
            param_type, body, ..
        } => mentions_hole(param_type) || mentions_hole(body),
        Term::Let {
            ty, value, body, ..
        } => mentions_hole(ty) || mentions_hole(value) || mentions_hole(body),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            mentions_hole(discriminant) || mentions_hole(motive) || cases.iter().any(mentions_hole)
        }
        Term::Fix { body, .. } => mentions_hole(body),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| mentions_hole(b)),
        Term::Annotated { term, .. } => mentions_hole(term),
    }
}

/// α-equivalence **as a kernel verdict** — the syntactic fast path of every conversion check.
///
/// It is [`types_equal`] with SPEC B applied: a `Term::Hole` is an UNSOLVED ELABORATION
/// PROBLEM, not a term of the theory, so no judgement about it may succeed — not against a
/// type, not against a sort, not against another `Hole`. `types_equal` is a MATCHER, and a
/// matcher may sensibly treat a hole as a wildcard; a matcher's leniency must never become the
/// kernel's answer. One hole surviving into a checked statement otherwise makes every
/// comparison against that statement succeed, so a theorem is recorded as verified while
/// nothing proves it.
///
/// The hole walk runs only when the structural match already SUCCEEDED, so it never costs more
/// than the comparison that has just been paid for.
#[cfg(test)]
pub(crate) fn bounded_probe_alpha_eq_for_test(a: &Term, b: &Term) -> bool {
    alpha_eq(a, b)
}

fn alpha_eq(a: &Term, b: &Term) -> bool {
    types_equal(a, b) && !mentions_hole(a) && !mentions_hole(b)
}

/// α-equivalence with no hole on either side: the two terms differ at most in the names of their
/// bound variables, and so mean the same thing.
pub(crate) fn alpha_equivalent(a: &Term, b: &Term) -> bool {
    alpha_eq(a, b)
}

#[inline(never)]
fn fresh_conversion_name(ctx: Option<&Context>, terms: &[&Term]) -> String {
    let mut avoid = std::collections::HashSet::new();
    for term in terms {
        avoid.extend(free_vars(term));
    }
    let mut name = fresh_name("__conversion", &avoid);
    while avoid.contains(&name) || ctx.is_some_and(|c| c.local_name_in_use(&name)) {
        name.push('_');
    }
    name
}

#[inline(never)]
fn open_conversion_binders(
    ctx: Option<&Context>,
    p: &str,
    a: &Term,
    q: &str,
    b: &Term,
) -> (String, Term, Term) {
    let name = fresh_conversion_name(ctx, &[a, b]);
    let variable = Term::Var(name.clone());
    (
        name,
        substitute(a, p, &variable),
        substitute(b, q, &variable),
    )
}

#[inline(never)]
fn bound_types_equal(p: &str, a: &Term, q: &str, b: &Term) -> bool {
    if p == q {
        return types_equal(a, b);
    }
    let (_, a, b) = open_conversion_binders(None, p, a, q, b);
    types_equal(&a, &b)
}

/// Structural α-equivalence AS A MATCHER: the same term up to renaming of bound variables,
/// with a `Term::Hole` acting as a WILDCARD that matches any term.
///
/// The wildcard is why this is not the kernel's conversion check. Every verdict goes through
/// [`alpha_eq`], which refuses a hole outright.
fn types_equal(a: &Term, b: &Term) -> bool {
    // Hole matches anything (it's a type wildcard)
    if matches!(a, Term::Hole) || matches!(b, Term::Hole) {
        return true;
    }

    match (a, b) {
        (Term::Sort(u1), Term::Sort(u2)) => u1.equiv(u2),

        (Term::Lit(l1), Term::Lit(l2)) => l1 == l2,

        (Term::Var(n1), Term::Var(n2)) => n1 == n2,

        (Term::Global(n1), Term::Global(n2)) => n1 == n2,
        (
            Term::Const {
                name: n1,
                levels: l1,
            },
            Term::Const {
                name: n2,
                levels: l2,
            },
        ) => n1 == n2 && l1.len() == l2.len() && l1.iter().zip(l2).all(|(a, b)| a.equiv(b)),

        (
            Term::Pi {
                param: p1,
                param_type: t1,
                body_type: b1,
            },
            Term::Pi {
                param: p2,
                param_type: t2,
                body_type: b2,
            },
        ) => types_equal(t1, t2) && { bound_types_equal(p1, b1, p2, b2) },

        (
            Term::Lambda {
                param: p1,
                param_type: t1,
                body: b1,
            },
            Term::Lambda {
                param: p2,
                param_type: t2,
                body: b2,
            },
        ) => types_equal(t1, t2) && { bound_types_equal(p1, b1, p2, b2) },

        (Term::App(f1, a1), Term::App(f2, a2)) => types_equal(f1, f2) && types_equal(a1, a2),

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
            types_equal(d1, d2)
                && types_equal(m1, m2)
                && c1.len() == c2.len()
                && c1.iter().zip(c2.iter()).all(|(a, b)| types_equal(a, b))
        }

        (Term::Fix { name: n1, body: b1 }, Term::Fix { name: n2, body: b2 }) => {
            bound_types_equal(n1, b1, n2, b2)
        }

        // A ζ-binding, up to renaming. Without this arm two structurally identical SHARED types
        // fall past `is_subtype`'s fast path and are compared by normalising both — which
        // re-materialises exactly the copies the sharing avoided.
        //
        // The comparison itself lives OUT OF LINE. `types_equal` recurses once per node of a
        // term the depth door admits — up to 1600 frames — so every local this arm would add
        // (an α-renaming, a substituted body) is paid by all 1600 of them whether or not a `Let`
        // is ever seen. Inlining it cost ~180 KB of stack on a 1800-deep spine and turned
        // `kernel_core_extra::a_term_past_the_depth_door_is_refused_and_does_not_abort` from a
        // refusal into an abort.
        (Term::Let { .. }, Term::Let { .. }) => let_types_equal(a, b),

        // All recursive names bind every body simultaneously. Reuse the term
        // store's scope-aware canonicalizer, which retains member order/index.
        (Term::MutualFix { .. }, Term::MutualFix { .. }) => mutual_types_equal(a, b),

        _ => false,
    }
}

// Keep arena allocation out of the recursive structural comparator's stack frame.
#[inline(never)]
fn mutual_types_equal(a: &Term, b: &Term) -> bool {
    let mut store = crate::term_store::TermStore::new();
    let lhs = store.intern(a);
    let rhs = store.intern(b);
    store.alpha_same(lhs, rhs)
}

/// The `Let`/`Let` case of [`types_equal`], kept out of line so its locals do not enlarge the
/// frame of the recursion that calls it once per node. See the call site for why that matters.
#[inline(never)]
fn let_types_equal(a: &Term, b: &Term) -> bool {
    let (
        Term::Let {
            name: n1,
            ty: t1,
            value: v1,
            body: b1,
        },
        Term::Let {
            name: n2,
            ty: t2,
            value: v2,
            body: b2,
        },
    ) = (a, b)
    else {
        return false;
    };
    types_equal(t1, t2) && types_equal(v1, v2) && { bound_types_equal(n1, b1, n2, b2) }
}

/// The carriers the kernel's own `Term::Lit` rule inhabits DIRECTLY, with no constructor.
///
/// `ex falso` — an empty `match`, legal because a zero-constructor inductive has no inhabitants
/// — is a claim about INHABITATION, not about a constructor count. These carriers are registered
/// with no constructors and are inhabited anyway, by the `Term::Lit` arm of `infer_type` a few
/// hundred lines up. Without this list, `match (5 : Int) return False with []` type-checks: a
/// closed proof of `False` assembled from nothing but the standard library, corroborated by the
/// second kernel because it counts constructors the same way.
///
/// The list belongs HERE, next to the rule that needs it, and must be extended whenever the
/// literal rule learns a new carrier — the two are the same question asked twice ("what can the
/// kernel hand me a value of?"). The real repair for the six below is to give them their
/// constructors (`Int` is `ofNat`/`negSucc` in Lean); until then an empty match on one is
/// refused rather than believed.
fn is_inhabited_by_a_literal(name: &str) -> bool {
    matches!(
        name,
        "Int" | "Nat" | "Float" | "Text" | "Duration" | "Date" | "Moment"
    )
}

/// Whether a `match` on `inductive_name` is `ex falso` in name only — the inductive has no
/// constructors, so the empty case list passes coverage, but the kernel can produce a value of
/// it regardless.
fn empty_match_would_be_a_lie(ctx: &Context, inductive_name: &str) -> bool {
    ctx.get_constructors(inductive_name).is_empty() && is_inhabited_by_a_literal(inductive_name)
}

/// Decompose `t` into a CONSTRUCTOR head and its arguments, bridging a `Nat` literal to its
/// Peano spelling (`0` is `Zero`, `n+1` is `Succ (n-1)`) so that `Vect A 0` and `Vect A Zero`
/// are the same index to the unifier below.
fn constructor_head(ctx: &Context, t: &Term) -> Option<(String, Vec<Term>)> {
    if let Some(constructor) = crate::int_inductive::literal_constructor(ctx, t) {
        return constructor_head(ctx, &constructor);
    }
    if let Term::Lit(Literal::Nat(value)) = t {
        if !crate::int_inductive::reads_nat_literal(ctx, value) {
            return None;
        }
        return match nat_peano_step(t) {
            Term::Global(z) => Some((z, Vec::new())),
            Term::App(s, pred) => match *s {
                Term::Global(succ) => Some((succ, vec![*pred])),
                _ => None,
            },
            _ => None,
        };
    }
    let mut args = Vec::new();
    let mut head = t;
    while let Term::App(f, a) = head {
        args.push((**a).clone());
        head = f;
    }
    args.reverse();
    match head {
        Term::Global(name) | Term::Const { name, .. } if ctx.is_constructor(name) => {
            Some((name.clone(), args))
        }
        _ => None,
    }
}

/// Expose only the discriminant index currently needed for constructor disjointness.
/// Its local variables belong to ctx; the constructor's declared index lives in
/// another telescope and must not be reduced under this context.
pub(crate) fn demanded_index_constructor_head(
    ctx: &Context,
    t: &Term,
) -> Option<(String, Vec<Term>)> {
    if let Some(head) = constructor_head(ctx, t) {
        return Some(head);
    }
    if let Some(exposed) = crate::reduction::bounded_conversion_head(ctx, t) {
        if let Some(head) = constructor_head(ctx, &exposed) {
            return Some(head);
        }
    }
    // A probe boundary is not a semantic refusal. Retain the general checker's
    // existing capability, but demand this index alone instead of the whole family.
    // An unfinished normalization is never evidence that a case is impossible.
    let exposed = crate::reduction::normalize_checked(ctx, t).ok()?;
    constructor_head(ctx, &exposed)
}

/// Whether constructor disjointness is available for `ctor`'s inductive.
///
/// It is a `Type`-level phenomenon ONLY. In `Prop`, proof irrelevance makes any two inhabitants
/// of a proposition definitionally equal, so two distinct constructors of a propositional
/// inductive are EQUAL and nothing may be concluded from their difference. Using a clash there
/// would let the kernel discharge a branch that is in fact reachable. A family in `Sort u` is a
/// proposition at `u := 0`, so the family's arity must end in a sort that is `Type 0` or above at
/// every instance of its levels ([`Universe::is_never_prop`]); an arity that ends in anything but
/// a written sort grants nothing.
fn disjointness_available(ctx: &Context, ctor: &str) -> bool {
    let Some(ind) = ctx.constructor_inductive(ctor) else {
        return false;
    };
    let Some(ind_ty) = ctx.get_global(&ind) else {
        return false;
    };
    let mut cur = ind_ty;
    while let Term::Pi { body_type, .. } = cur {
        cur = body_type;
    }
    matches!(cur, Term::Sort(level) if level.is_never_prop())
}

/// Whether the two index terms PROVABLY cannot be made equal.
///
/// `scrutinee` is rigid — it comes from the discriminant's own type. `declared` is the
/// constructor's declared result index, in the constructor's scope, where `flexible` names the
/// binders that are still free to be anything.
///
/// The only reason ever given is CONSTRUCTOR DISJOINTNESS: two distinct constructors of the
/// same `Type`-level inductive can never produce equal values. Everything else — a flexible
/// variable on either side, a rigid variable, or an index whose head cannot be exposed — is
/// treated as possibly-equal, because refusing to rule a branch out is always safe and ruling one out
/// wrongly deletes a reachable case.
fn indices_clash(ctx: &Context, scrutinee: &Term, declared: &Term, flexible: &[String]) -> bool {
    if let Term::Var(v) = declared {
        if flexible.iter().any(|f| f == v) {
            return false;
        }
    }
    // Without a declared constructor head no clash is possible; do not evaluate
    // the scrutinee index merely to rediscover that fact.
    let Some((cd, args_d)) = constructor_head(ctx, declared) else {
        return false;
    };
    let Some((cs, args_s)) = demanded_index_constructor_head(ctx, scrutinee) else {
        return false;
    };
    if !disjointness_available(ctx, &cs) || !disjointness_available(ctx, &cd) {
        return false;
    }
    if cs != cd {
        return ctx.constructor_inductive(&cs) == ctx.constructor_inductive(&cd);
    }
    args_s
        .iter()
        .zip(args_d.iter())
        .any(|(a, b)| indices_clash(ctx, a, b, flexible))
}

/// **SPEC F** — which of `inductive_name`'s constructors the discriminant's INDICES still allow,
/// as positions in registration order. `disc_args` are the discriminant's type arguments, its
/// `num_params` parameters first.
///
/// A `Match` on an indexed family owes a branch only for the constructors that could actually
/// have produced the scrutinee. `VCons`'s result index is `Succ n`, which can never be `Zero`,
/// so a match on `Vect α 0` owes no `VCons` branch — and demanding one forces the caller to
/// invent an inhabitant of a type that may have none (`vtail`'s impossible branch owes a
/// `Vect A n` with no value in scope). A constructor's parameters are the scrutinee's own, so its
/// declared result indices are read with its parameter binders at the scrutinee's parameters:
/// `refl`'s index is its parameter `x`, which for `Eq Nat (Succ a) Zero` is `Succ a`, clashing
/// with `Zero`.
///
/// Conservative in the safe direction: a constructor is ruled out only on a definite clash, so
/// the worst this can do is ask for a branch that is unreachable — never omit one that is not.
pub(crate) fn constructors_the_indices_allow(
    ctx: &Context,
    inductive_name: &str,
    num_params: usize,
    disc_args: &[Term],
) -> Vec<usize> {
    let (params, disc_indices) = disc_args.split_at(num_params.min(disc_args.len()));
    ctx.get_constructors(inductive_name)
        .iter()
        .enumerate()
        .filter(|(_, (_, ctor_type))| {
            let mut opened: Term = (*ctor_type).clone();
            for value in params {
                match opened {
                    Term::Pi {
                        param, body_type, ..
                    } => opened = substitute(&body_type, &param, value),
                    _ => return true,
                }
            }
            let mut flexible: Vec<String> = Vec::new();
            let mut cur = &opened;
            while let Term::Pi {
                param, body_type, ..
            } = cur
            {
                flexible.push(param.clone());
                cur = body_type;
            }
            let result_args = extract_type_args(cur);
            let declared: &[Term] = if result_args.len() > num_params {
                &result_args[num_params..]
            } else {
                &[]
            };
            !disc_indices
                .iter()
                .zip(declared.iter())
                .any(|(s, d)| indices_clash(ctx, s, d, &flexible))
        })
        .map(|(i, _)| i)
        .collect()
}

/// Whether a `Prop` inductive may be **large-eliminated** (into `Type`).
///
/// The CIC criterion, stated once and implemented independently here and in
/// [`crate::recheck`]: `I` is large-eliminable iff
///
/// - it has **zero** constructors (`False` — ex falso reaches every sort), or
/// - it has **exactly one**, and every binder of that constructor's telescope beyond `I`'s
///   **parameters** is either
///   - **propositional** (its type's sort is `Prop`), or
///   - **an index**: it occurs, as itself, among the arguments of the constructor's result
///     type `I p̄ ī` past the parameters.
///
/// Two telescopes meet here and they have DIFFERENT lengths. `I`'s arity is params + indices;
/// the constructor binds the params and then its own fields — it does not bind the indices, it
/// COMPUTES them in its result. Counting the skip off the arity walks past real data fields
/// (`Tag : Bool → Prop` with `Tag_mk : Π(n:Nat). Tag (decide …)` would hand back the `n` a
/// proof was built from); counting it off the params WITHOUT the index clause amputates `Acc`,
/// whose `x : A` field is admissible only because the result is `Acc A R x` — and with `Acc`
/// goes every definition by well-founded recursion.
///
/// The index clause is what makes a data field harmless: if the field IS an index, it is
/// already recoverable from the proposition's own type, so eliminating into `Type` reveals
/// nothing the type did not already carry.
pub(crate) fn is_subsingleton_prop(ctx: &Context, inductive_name: &str) -> KernelResult<bool> {
    let ctors = ctx.get_constructors(inductive_name);
    match ctors.len() {
        0 => Ok(true),
        1 => {
            let num_params = ctx.inductive_num_params(inductive_name);
            let mut local = ctx.clone();
            let mut t = ctors[0].1.clone();
            let mut i = 0usize;
            // Field binders that are NOT propositional; each is admissible only if it
            // reappears among the result's indices.
            let mut owed: Vec<(String, usize)> = Vec::new();
            // Every binder name, to detect shadowing: if a name is bound twice, an occurrence
            // in the result refers to the INNER binder, so crediting the outer one would be
            // unsound. A repeat is refused rather than resolved.
            let mut bound: Vec<(String, String)> = Vec::new();
            while let Term::Pi {
                param,
                param_type,
                body_type,
            } = t
            {
                if i >= num_params && !infer_sort(&local, &param_type)?.equiv(&Universe::Prop) {
                    owed.push((param.clone(), bound.len()));
                }
                let original = param.clone();
                let (param, body_type) =
                    open_context_binder(&local, &param, &param_type, &body_type);
                let opened = param.to_string();
                t = body_type.into_owned();
                local = local.extend(&opened, (*param_type).clone());
                bound.push((original, opened));
                i += 1;
            }
            if owed.is_empty() {
                return Ok(true);
            }
            // `t` is now the constructor's result `I p̄ ī`; its arguments past the parameters
            // are the indices.
            let args = extract_type_args(&t);
            let indices: &[Term] = if args.len() > num_params {
                &args[num_params..]
            } else {
                &[]
            };
            Ok(owed.iter().all(|(original, position)| {
                let name = &bound[*position].1;
                bound.iter().filter(|(old, _)| old == original).count() == 1
                    && indices
                        .iter()
                        .any(|ix| matches!(ix, Term::Var(v) if v == name))
            }))
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/def_eq_and_freshness_mutation_kills.rs"]
mod def_eq_and_freshness_mutation_kills;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/large_elim_tests.rs"]
mod large_elim_tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/constructor_universe_controls.rs"]
mod constructor_universe_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/native_mutual_alpha_regressions.rs"]
mod native_mutual_alpha_regressions;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/early_binder_conversion_controls.rs"]
mod early_binder_conversion_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/type_head_work_controls.rs"]
mod type_head_work_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/head_alignment_conversion_controls.rs"]
mod head_alignment_conversion_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/head_alignment_exactness_controls.rs"]
mod head_alignment_exactness_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/head_alignment_float_contract_controls.rs"]
mod head_alignment_float_contract_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/delta_head_alignment_controls.rs"]
mod delta_head_alignment_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/delta_head_hint_boundaries.rs"]
mod delta_head_hint_boundaries;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/delta_head_chain_controls.rs"]
mod delta_head_chain_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/borrowed_common_head_work.rs"]
mod borrowed_common_head_work;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/closed_helper_body_controls.rs"]
mod closed_helper_body_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/closed_helper_body_admission_controls.rs"]
mod closed_helper_body_admission_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/borrowed_fixed_head_controls.rs"]
mod borrowed_fixed_head_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/constructor_probe_guard_controls.rs"]
mod constructor_probe_guard_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/fix_result_eligibility_controls.rs"]
mod fix_result_eligibility_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/fix_lambda_probe_controls.rs"]
mod fix_lambda_probe_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/peano_inference_controls.rs"]
mod peano_inference_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/peano_inference_boundary_controls.rs"]
mod peano_inference_boundary_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/peano_explicit_poly_control.rs"]
mod peano_explicit_poly_control;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/opaque_sort_reuse_controls.rs"]
mod opaque_sort_reuse_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/binder_entry_scope_controls.rs"]
mod binder_entry_scope_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/owned_substitution_controls.rs"]
mod owned_substitution_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/owned_replacement_controls.rs"]
mod owned_replacement_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/bounded_conversion_controls.rs"]
mod bounded_conversion_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/borrowed_retry_controls.rs"]
mod borrowed_retry_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/probe_scheduling_controls.rs"]
mod probe_scheduling_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/constructor_refutation_controls.rs"]
mod constructor_refutation_controls;

#[path = "lazy_delta.rs"]
mod lazy_delta;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/lazy_delta_controls.rs"]
mod lazy_delta_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/type_checker/recursor_probe_scheduling_controls.rs"]
mod recursor_probe_scheduling_controls;
