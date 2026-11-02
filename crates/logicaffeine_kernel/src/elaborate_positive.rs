//! Positive-only small closed conversion for the elaborator. Unknown is fallback.
use crate::reduction::{self, Transparency};
use crate::{Context, Literal, Term, Universe};

// Admission is deliberately narrower than kernel intake. Every refusal merely
// retains ordinary elaborator normalization. Check before cloning any input or
// delta body, including constant level/name/literal payloads.
fn admitted(t: &Term) -> Option<usize> {
    let (nodes, depth, _) = reduction::bounded_conversion_metrics(t)?;
    if depth > 64 {
        return None;
    }
    let mut level_budget = 256usize;
    let mut level = |u: &Universe| {
        let mut work = vec![(u, 0usize)];
        let mut visited = 0usize;
        while let Some((u, depth)) = work.pop() {
            if depth > 16 || visited == 32 || level_budget == 0 {
                return false;
            }
            visited += 1;
            level_budget -= 1;
            match u {
                Universe::Prop | Universe::SProp | Universe::Type(_) => {}
                Universe::Var(n) => {
                    if n.len() > 256 {
                        return false;
                    }
                }
                Universe::Succ(a) => work.push((a.as_ref(), depth + 1)),
                Universe::Max(a, b) | Universe::IMax(a, b) => {
                    work.push((a.as_ref(), depth + 1));
                    work.push((b.as_ref(), depth + 1));
                }
            }
        }
        true
    };
    let mut work = vec![t];
    while let Some(t) = work.pop() {
        match t {
            Term::Hole => return None,
            Term::Var(n) | Term::Global(n) => {
                if n.len() > 256 {
                    return None;
                }
            }
            Term::Const { name, levels } => {
                if name.len() > 256 || levels.len() > 16 || !levels.iter().all(&mut level) {
                    return None;
                }
            }
            Term::Sort(u) => {
                if !level(u) {
                    return None;
                }
            }
            Term::Lit(Literal::Nat(n) | Literal::BigInt(n)) => {
                if n.to_i64().is_none() {
                    return None;
                }
            }
            Term::Lit(Literal::Text(s)) => {
                if s.len() > 256 {
                    return None;
                }
            }
            Term::Lit(_) => {}
            Term::App(f, a) => {
                work.push(f);
                work.push(a);
            }
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                if param.len() > 256 {
                    return None;
                }
                work.push(param_type);
                work.push(body_type);
            }
            Term::Lambda {
                param,
                param_type,
                body,
            } => {
                if param.len() > 256 {
                    return None;
                }
                work.push(param_type);
                work.push(body);
            }
            Term::Let {
                name,
                ty,
                value,
                body,
            } => {
                if name.len() > 256 {
                    return None;
                }
                work.push(ty);
                work.push(value);
                work.push(body);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                work.push(discriminant);
                work.push(motive);
                work.extend(cases);
            }
            Term::Fix { name, body } => {
                if name.len() > 256 {
                    return None;
                }
                work.push(body);
            }
            Term::MutualFix { defs, .. } => {
                for (n, b) in defs {
                    if n.len() > 256 {
                        return None;
                    }
                    work.push(b);
                }
            }
            Term::Annotated { term, .. } => work.push(term),
        }
    }
    Some(nodes)
}

// A single existing evaluator observation, only at a demanded nullary data
// or natural-literal endpoint. Partial function prefixes never trigger speculative evaluation.
fn compact_data(ctx: &Context, a: &Term, b: &Term, used: &mut bool, fuel: u64) -> bool {
    if *used
        || fuel == 0
        || !ctx.simp_rules().is_empty()
        || !ctx.elaborator_all_transparent()
        || admitted(a).is_none()
        || admitted(b).is_none()
        || !crate::term_store::is_closed_without_holes(a)
        || !crate::term_store::is_closed_without_holes(b)
    {
        return false;
    }
    let atomic = |t: &Term| match t {
        Term::Global(n) if ctx.is_constructor(n) && ctx.poly_declaration(n).is_none() => {
            matches!(ctx.get_global(n),Some(Term::Global(owner)) if ctx.is_inductive(owner)
                && matches!(ctx.get_global(owner),Some(Term::Sort(Universe::Type(_)))))
        }
        _ => false,
    };
    let natural_literal = |t: &Term| matches!(t,Term::Lit(Literal::Nat(n)) if !n.is_negative());
    if natural_literal(a) || natural_literal(b) {
        *used = true;
        // Positive evidence only. False/unknown leave ordinary unification in
        // control; no second evaluator budget is spent on this obligation.
        return matches!(
            reduction::natural_literal_conversion(ctx, a, b, fuel.min(1 << 18)),
            Some(true)
        );
    }
    if !atomic(a) && !atomic(b) {
        return false;
    }
    *used = true;
    crate::reduction::constructor_equivalent(ctx, a, b, fuel.min(1 << 18))
}

fn same(
    ctx: &Context,
    a: &Term,
    b: &Term,
    tr: Transparency<'_>,
    fuel: &mut usize,
    depth: usize,
) -> bool {
    let allowed = ctx.elaborator_all_transparent();
    let mut used = false;
    let result = same_inner(ctx, a, b, tr, fuel, depth, allowed, &mut used);
    // Opt-in bounded diagnostic carries no term/source payload and does not
    // change admission or evaluator budgets. Observe actual corpus eligibility.
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    if (used || !allowed)
        && *TRACE.get_or_init(|| std::env::var_os("LOGOS_ELAB_COMPACT_TRACE").is_some())
    {
        let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if n < 32 {
            eprintln!("LOGOS_ELAB_COMPACT sequence={n} full_transparency={allowed} attempted={used} positive={result}");
        }
    }
    result
}

fn same_inner(
    ctx: &Context,
    a: &Term,
    b: &Term,
    tr: Transparency<'_>,
    fuel: &mut usize,
    depth: usize,
    compact_allowed: bool,
    compact_used: &mut bool,
) -> bool {
    // A structural child is a new equality obligation, with its own originals.
    same_from(
        ctx,
        a,
        b,
        a,
        b,
        tr,
        fuel,
        depth,
        compact_allowed,
        compact_used,
    )
}

// Invariant: current a/b derive from origin_a/origin_b only through the checked
// head reductions below. Origins are borrowed, never normalized or cloned here.
fn same_from(
    ctx: &Context,
    a: &Term,
    b: &Term,
    origin_a: &Term,
    origin_b: &Term,
    tr: Transparency<'_>,
    fuel: &mut usize,
    depth: usize,
    compact_allowed: bool,
    compact_used: &mut bool,
) -> bool {
    if depth >= 64 || *fuel == 0 {
        return false;
    }
    *fuel -= 1;
    if admitted(a).is_none() || admitted(b).is_none() {
        return false;
    }
    if crate::type_checker::alpha_equivalent(a, b) {
        return true;
    }
    // A literal reached by this obligation's checked head chain can be observed
    // against the opposite ORIGINAL. Keep its checked arithmetic head available
    // rather than feeding an already unfolded Fix into recurrence certification.
    let natural = |t: &Term| matches!(t,Term::Lit(Literal::Nat(n)) if !n.is_negative());
    let (observed_a, observed_b) = if natural(a) {
        (a, origin_b)
    } else if natural(b) {
        (origin_a, b)
    } else {
        (a, b)
    };
    if compact_allowed && compact_data(ctx, observed_a, observed_b, compact_used, 1 << 18) {
        return true;
    }
    // Congruence BEFORE head exposure: a common expensive argument is compared
    // structurally and never unfolded merely because a surrounding wrapper is.
    if let (Term::App(af, aa), Term::App(bf, ba)) = (a, b) {
        if same_inner(
            ctx,
            af,
            bf,
            tr,
            fuel,
            depth + 1,
            compact_allowed,
            compact_used,
        ) && same_inner(
            ctx,
            aa,
            ba,
            tr,
            fuel,
            depth + 1,
            compact_allowed,
            compact_used,
        ) {
            return true;
        }
    }
    let Some(ah) = reduction::beta_iota_zeta_head(ctx, a, fuel) else {
        return false;
    };
    let Some(bh) = reduction::beta_iota_zeta_head(ctx, b, fuel) else {
        return false;
    };
    if admitted(&ah).is_none() || admitted(&bh).is_none() {
        return false;
    }
    if !crate::type_checker::alpha_equivalent(a, &ah)
        || !crate::type_checker::alpha_equivalent(b, &bh)
    {
        return same_from(
            ctx,
            &ah,
            &bh,
            origin_a,
            origin_b,
            tr,
            fuel,
            depth + 1,
            compact_allowed,
            compact_used,
        );
    }
    if *fuel == 0 {
        return false;
    }
    *fuel -= 1;
    // One delta step per side, only on the head path. The existing helper
    // preserves original beta/iota/fix guards and checks each body before copy.
    let da = reduction::bounded_delta_head_with(ctx, &ah, tr, admitted);
    // Preserve an already identical opposite operand before unfolding its head.
    // The candidate is a checked stored body admitted by the existing delta gate.
    if da
        .as_ref()
        .is_some_and(|(_, a)| crate::type_checker::alpha_equivalent(a, &bh))
    {
        return true;
    }
    let db = reduction::bounded_delta_head_with(ctx, &bh, tr, admitted);
    if db
        .as_ref()
        .is_some_and(|(_, b)| crate::type_checker::alpha_equivalent(&ah, b))
    {
        return true;
    }
    match (da, db) {
        (None, None) => false,
        (Some((_, a)), None) => same_from(
            ctx,
            &a,
            &bh,
            origin_a,
            origin_b,
            tr,
            fuel,
            depth + 1,
            compact_allowed,
            compact_used,
        ),
        (None, Some((_, b))) => same_from(
            ctx,
            &ah,
            &b,
            origin_a,
            origin_b,
            tr,
            fuel,
            depth + 1,
            compact_allowed,
            compact_used,
        ),
        (Some((_, a)), Some((_, b))) => same_from(
            ctx,
            &a,
            &b,
            origin_a,
            origin_b,
            tr,
            fuel,
            depth + 1,
            compact_allowed,
            compact_used,
        ),
    }
}

pub(super) fn equivalent(ctx: &Context, a: &Term, b: &Term) -> bool {
    if !ctx.simp_rules().is_empty()
        || admitted(a).is_none()
        || admitted(b).is_none()
        || !crate::term_store::is_closed_without_holes(a)
        || !crate::term_store::is_closed_without_holes(b)
    {
        return false;
    }
    let opaque = |n: &str| ctx.is_irreducible(n);
    same(ctx, a, b, Transparency::Excluding(&opaque), &mut 4096, 0)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/elaborate/positive/guards.rs"]
mod guards;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/elaborate/positive/compact_data_guard_controls.rs"]
mod compact_data_guard_controls;

// Open obligations decompose only application congruence. No binder body is
// entered and no open beta/delta evaluator is added. Closed leaves reuse exactly
// the existing reduction engine and share its depth, fuel and compact attempt.
fn same_scoped(
    ctx: &Context,
    a: &Term,
    b: &Term,
    tr: Transparency<'_>,
    fuel: &mut usize,
    depth: usize,
    compact_allowed: bool,
    compact_used: &mut bool,
) -> bool {
    if depth >= 64 || *fuel == 0 {
        return false;
    }
    *fuel -= 1;
    if crate::type_checker::alpha_equivalent(a, b) {
        return true;
    }
    if crate::term_store::is_closed_without_holes(a)
        && crate::term_store::is_closed_without_holes(b)
    {
        return same_inner(ctx, a, b, tr, fuel, depth, compact_allowed, compact_used);
    }
    match (a, b) {
        (Term::App(af, aa), Term::App(bf, ba)) => {
            same_scoped(
                ctx,
                af,
                bf,
                tr,
                fuel,
                depth + 1,
                compact_allowed,
                compact_used,
            ) && same_scoped(
                ctx,
                aa,
                ba,
                tr,
                fuel,
                depth + 1,
                compact_allowed,
                compact_used,
            )
        }
        _ => false,
    }
}

/// Scoped positive evidence for the elaborator only. Context keys authorize
/// free-name scope, never their stored type or a local proposition's truth.
pub(super) fn equivalent_scoped(ctx: &Context, a: &Term, b: &Term) -> bool {
    if !ctx.simp_rules().is_empty()
        || admitted(a).is_none()
        || admitted(b).is_none()
        || super::mentions_meta(a)
        || super::mentions_meta(b)
        || !crate::term_store::free_names_in_term(a)
            .iter()
            .all(|n| !super::is_meta(n) && ctx.has_local(n))
        || !crate::term_store::free_names_in_term(b)
            .iter()
            .all(|n| !super::is_meta(n) && ctx.has_local(n))
    {
        return false;
    }
    if crate::term_store::is_closed_without_holes(a)
        && crate::term_store::is_closed_without_holes(b)
    {
        return equivalent(ctx, a, b);
    }
    let opaque = |n: &str| ctx.is_irreducible(n);
    same_scoped(
        ctx,
        a,
        b,
        Transparency::Excluding(&opaque),
        &mut 4096,
        0,
        ctx.elaborator_all_transparent(),
        &mut false,
    )
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/elaborate/positive/scoped_positive_guards.rs"]
mod scoped_positive_guards;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/elaborate/positive/natural_observation_guards.rs"]
mod natural_observation_guards;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/elaborate/positive/natural_origin_controls.rs"]
mod natural_origin_controls;
