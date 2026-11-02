//! Positive-only application congruence before unfolding a common global head.
use super::*;

thread_local! { static ACTIVE:std::cell::Cell<bool> = const {std::cell::Cell::new(false)}; }
const STEPS: usize = 4096;
const SPINE: usize = 64;

pub(super) fn exhausted() -> bool {
    ACTIVE.with(|v| v.get()) && WHNF_EXHAUSTED.with(|v| v.get())
}

// One allowance bounds reduction speculation, including nested conversions.
// It is not a byte bound on the ordinary checker's inference or term copying.
struct Saved {
    fuel: usize,
    limit: usize,
    owned: bool,
    depth: usize,
    exhausted: bool,
}
struct Scope(Option<Saved>);
impl Scope {
    fn enter() -> Option<Self> {
        if WHNF_EXHAUSTED.with(|v| v.get()) {
            return None;
        }
        if ACTIVE.with(|v| v.get()) {
            return Some(Self(None));
        }
        let owned = recheck_budget_active();
        let fuel = WHNF_FUEL.with(|v| v.get());
        let limit = if owned { (fuel / 4).min(STEPS) } else { STEPS };
        if limit == 0 {
            return None;
        }
        let depth = LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.get());
        let exhausted = WHNF_EXHAUSTED.with(|v| v.get());
        ACTIVE.with(|v| v.set(true));
        LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.set(depth + 1));
        WHNF_FUEL.with(|v| v.set(limit));
        Some(Self(Some(Saved {
            fuel,
            limit,
            owned,
            depth,
            exhausted,
        })))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        let Some(s) = &self.0 else {
            return;
        };
        let used = s.limit.saturating_sub(WHNF_FUEL.with(|v| v.get()));
        WHNF_FUEL.with(|v| {
            v.set(if s.owned {
                s.fuel.saturating_sub(used)
            } else {
                s.fuel
            })
        });
        WHNF_EXHAUSTED.with(|v| v.set(s.exhausted));
        LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.set(s.depth));
        ACTIVE.with(|v| v.set(false));
    }
}
// Header admission bounds direct inspection and universe substitution before
// inference. Transitive type inference still uses the ordinary checker; Scope
// meters reduction speculation, not a total allocation or inference allowance.
fn proof_spend(work: usize) -> bool {
    WHNF_FUEL.with(|v| {
        let available = v.get();
        v.set(available.saturating_sub(work));
        work <= available
    })
}

fn admit_proof_reference(ctx: &Context, term: &Db) -> bool {
    let Some(endpoint) = prefix_metrics(term) else {
        return false;
    };
    let (ty, parameters) = match term {
        Db::Global(name) => match ctx.get_global(name) {
            Some(ty) => (ty, None),
            None => return false,
        },
        Db::Const { name, levels } => {
            if let Some((params, ty)) = ctx.poly_declaration(name) {
                if params.len() != levels.len() || params.len() > 256 {
                    return false;
                }
                (ty, Some(params))
            } else {
                let Some(ty) = ctx.get_global(name) else {
                    return false;
                };
                (ty, None)
            }
        }
        _ => return false,
    };
    if let Some(parameters) = parameters {
        let mut bytes = 0usize;
        for name in parameters {
            let Some(next) = bytes.checked_add(name.len()) else {
                return false;
            };
            bytes = next;
            if bytes > 4096 || !proof_spend(name.len().saturating_add(1)) {
                return false;
            }
        }
    }
    // Reuse the conversion cache's borrowed payload admission, independently
    // of any cached equality. This visits syntax only and copies no payload.
    let Some((nodes, bytes)) = crate::conversion_session::admission_metrics(ty, ty) else {
        proof_spend(WHNF_FUEL.with(|v| v.get()));
        return false;
    };
    if !proof_spend(nodes) || bytes > 4096 {
        return false;
    }
    let limit = WHNF_FUEL.with(|v| v.get()).min(256);
    let mut admission = LocalAdmission::new(limit);
    let closed = admission.free_names(ty).is_ok_and(|names| names.is_empty());
    proof_spend(limit - admission.remaining);
    if !closed || admission.max_depth > 64 {
        return false;
    }
    // Explicit universes can be copied at every header occurrence. The
    // measured pair includes both header copies and all identifier payloads.
    let reserve = nodes.checked_add(bytes).and_then(|header| {
        endpoint
            .levels
            .checked_add(endpoint.payload)?
            .checked_add(1)?
            .checked_mul(header)
    });
    reserve.is_some_and(proof_spend)
}

pub(super) fn proof_references(ctx: &Context, locals: &dyn LocalTypes, a: &Db, b: &Db) -> bool {
    if !matches!(a, Db::Global(_) | Db::Const { .. })
        || !matches!(b, Db::Global(_) | Db::Const { .. })
    {
        return false;
    }
    let short_name = |term: &Db| match term {
        Db::Global(name) | Db::Const { name, .. } => name.len() <= 256,
        _ => false,
    };
    if !short_name(a) || !short_name(b) {
        return false;
    }
    let has_body = |term: &Db| match term {
        Db::Global(name) => ctx.get_definition_body(name).is_some(),
        Db::Const { name, .. } => {
            ctx.get_universe_poly(name).is_some() || ctx.get_definition_body(name).is_some()
        }
        _ => false,
    };
    if !has_body(a) && !has_body(b) {
        return false;
    }
    let Some(_scope) = Scope::enter() else {
        return false;
    };
    if !admit_proof_reference(ctx, a) || !admit_proof_reference(ctx, b) {
        return false;
    }
    proof_irrel(ctx, locals, a, b) && !WHNF_EXHAUSTED.with(|v| v.get())
}

fn head(t: &Db) -> Option<(&Db, usize)> {
    let mut t = t;
    let mut arity = 0;
    while let Db::App(f, _) = t {
        if arity == SPINE {
            return None;
        }
        arity += 1;
        t = f;
    }
    (arity != 0).then_some((t, arity))
}
pub(super) fn equivalent(ctx: &Context, locals: &dyn LocalTypes, a: &Db, b: &Db) -> bool {
    arguments(ctx, locals, a, b) == Arguments::Equal
}

/// What comparing two applications of one global head argument by argument concluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arguments {
    /// Every argument pair converts.
    Equal,
    /// An argument pair does not convert, and no reduction under the comparison was cut short: this
    /// kernel's conversion refutes that pair, and comparing the same pairs again refutes it again.
    Differ,
    /// Not two applications of one global head, or a reduction under the comparison was cut short.
    Unknown,
}

fn arguments(ctx: &Context, locals: &dyn LocalTypes, a: &Db, b: &Db) -> Arguments {
    if WHNF_EXHAUSTED.with(|v| v.get()) {
        return Arguments::Unknown;
    }
    let Some((f, n)) = head(a) else {
        return Arguments::Unknown;
    };
    let Some((g, m)) = head(b) else {
        return Arguments::Unknown;
    };
    if n != m {
        return Arguments::Unknown;
    }
    let same = match (f, g) {
        (Db::Global(f), Db::Global(g)) => f == g,
        (Db::Const { name: f, levels: u }, Db::Const { name: g, levels: v }) => {
            f == g && u.len() == v.len() && u.iter().zip(v).all(|(u, v)| u.equiv(v))
        }
        _ => false,
    };
    if !same {
        return Arguments::Unknown;
    }
    if super::conversion_memo::arguments_refuted(ctx, locals, a, b) {
        return Arguments::Differ;
    }
    let Some(_scope) = Scope::enter() else {
        return Arguments::Unknown;
    };
    // Every argument is compared in the same unchanged local context. Walking
    // from the final argument inward needs no temporary spine allocation.
    let whole = (a, b);
    let (mut a, mut b) = (a, b);
    while let (Db::App(f, x), Db::App(g, y)) = (a, b) {
        if WHNF_EXHAUSTED.with(|v| v.get()) {
            return Arguments::Unknown;
        }
        if !def_eq(ctx, locals, x, y) {
            // Read before the scope restores the caller's exhaustion record.
            if WHNF_EXHAUSTED.with(|v| v.get()) {
                return Arguments::Unknown;
            }
            super::conversion_memo::record_refuted_arguments(ctx, locals, whole.0, whole.1);
            return Arguments::Differ;
        }
        a = f;
        b = g;
    }
    if WHNF_EXHAUSTED.with(|v| v.get()) {
        Arguments::Unknown
    } else {
        Arguments::Equal
    }
}

// Reserve the existing speculative allowance, including failed probes. Lowering
// cannot borrow it again through optional FixBody sharing. Restore only unused
// work, including during unwinding.
struct ReservedFuel(usize);
impl Drop for ReservedFuel {
    fn drop(&mut self) {
        WHNF_FUEL.with(|v| v.set(self.0));
    }
}
fn function_head(ctx: &Context, t: &Db) -> bool {
    let mut t = t;
    for _ in 0..=SPINE {
        match t {
            Db::App(f, _) => t = f,
            Db::Global(name) => {
                return matches!(ctx.get_definition_body(name), Some(Term::Lambda { .. }))
            }
            Db::Const { name, .. } => {
                return ctx
                    .get_universe_poly(name)
                    .map(|(_, _, body)| matches!(body, Term::Lambda { .. }))
                    .unwrap_or_else(|| {
                        matches!(ctx.get_definition_body(name), Some(Term::Lambda { .. }))
                    })
            }
            _ => return false,
        }
    }
    false
}
// Eligibility alone peels matching binders; the actual closure proof still
// checks the complete endpoints, including every domain and capture. This
// spends the existing allowance and never searches arbitrary argument trees.
fn function_below_binders(ctx: &Context, mut a: &Db, mut b: &Db, fuel: &mut usize) -> bool {
    for peeled in 0..SPINE {
        match (a, b) {
            (Db::Lam(_, ab), Db::Lam(_, bb)) | (Db::Pi(_, ab), Db::Pi(_, bb)) => {
                if *fuel == 0 {
                    return false;
                }
                *fuel -= 1;
                a = ab;
                b = bb;
            }
            _ => return peeled != 0 && (function_head(ctx, a) || function_head(ctx, b)),
        }
    }
    false
}
pub(super) fn align_definitions(ctx: &Context, a: &Db, b: &Db) -> bool {
    if !ACTIVE.with(|v| v.get()) || WHNF_EXHAUSTED.with(|v| v.get()) {
        return false;
    }
    let direct = function_head(ctx, a) || function_head(ctx, b);
    let mut reserved = ReservedFuel(WHNF_FUEL.with(|v| v.replace(0)));
    if !direct && !function_below_binders(ctx, a, b, &mut reserved.0) {
        return false;
    }
    (head_closure::equivalent(ctx, a, b, &mut reserved.0)
        || borrowed_congruence_with_limit(Some(ctx), a, b, &mut reserved.0, 512, 128))
        && !WHNF_EXHAUSTED.with(|v| v.get())
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/folded/tests.rs"]
mod tests;
