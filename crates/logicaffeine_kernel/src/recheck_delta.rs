//! Bounded independent delta exposure for the positive conversion probe.
use super::*;

#[cfg(test)]
pub(super) fn reduce(ctx: &Context, term: &Db, fuel: &mut usize) -> Option<Db> {
    reduce_with_limit(ctx, term, fuel, 64)
}
pub(super) fn reduce_with_limit(
    ctx: &Context,
    term: &Db,
    fuel: &mut usize,
    candidate: usize,
) -> Option<Db> {
    reduce_at(
        ctx,
        term,
        fuel,
        candidate,
        if candidate > 64 { 64 } else { *fuel },
    )
}
fn reduce_at(
    ctx: &Context,
    term: &Db,
    fuel: &mut usize,
    candidate: usize,
    depth: usize,
) -> Option<Db> {
    if depth == 0 {
        return None;
    }
    if *fuel == 0 {
        return None;
    }
    *fuel -= 1;
    if let Db::App(function, argument) = term {
        let function = reduce_at(ctx, function, fuel, candidate, depth - 1)?;
        // Preserve the original per-copy bound even in a wider delta probe.
        let allowance = if candidate > 64 {
            (*fuel).min(256)
        } else {
            *fuel
        };
        let mut remaining = allowance;
        let measured = congruence_nodes(argument, &mut remaining);
        *fuel -= allowance - remaining;
        let nodes = measured?;
        let copy = nodes.checked_add(1)?;
        if copy > *fuel {
            return None;
        }
        *fuel -= copy;
        return Some(Db::App(
            DbBox::new(function),
            DbBox::new((**argument).clone()),
        ));
    }
    let (name, levels) = match term {
        Db::Global(name) => (name.as_str(), None),
        Db::Const { name, levels } => (name.as_str(), Some(levels.as_slice())),
        _ => return None,
    };
    // These heads have independent reduction hooks before ordinary delta; leave them to that
    // dispatcher, including the legacy quotient spellings. `Quot_rec` is a checked definition
    // with no hook, and this probe still declines it: its derived body is left to ordinary δ.
    if name == "reduceBool"
        || primitive_rule(name).is_some()
        || matches!(name, "Quot_lift" | "Quot_rec" | "Quot_mk")
        || [
            crate::quotient::Kind::Type,
            crate::quotient::Kind::Constructor,
            crate::quotient::Kind::Lift,
            crate::quotient::Kind::Induction,
        ]
        .iter()
        .any(|kind| kind.name() == name)
    {
        return None;
    }
    let (ty, body) = if let Some(levels) = levels {
        match ctx.get_universe_poly(name) {
            Some((parameters, ty, body)) => {
                if parameters.len() != levels.len() {
                    return None;
                }
                (Some(ty), body)
            }
            None => (Some(ctx.get_global(name)?), ctx.get_definition_body(name)?),
        }
    } else {
        (None, ctx.get_definition_body(name)?)
    };
    // Bound each candidate's inspection so a large common head cannot consume
    // the entire shared allowance before a small alias on the other side.
    let limit = (*fuel).min(candidate);
    let mut admission = LocalAdmission::new(limit);
    let result = (|| {
        if let Some(ty) = ty {
            if !admission.free_names(ty).ok()?.is_empty() {
                return None;
            }
        }
        let before = admission.remaining;
        if !admission.free_names(body).ok()?.is_empty() {
            return None;
        }
        let body_work = before - admission.remaining;
        let before = admission.remaining;
        if let Some(levels) = levels {
            admission
                .scan(LocalAdmissionFrame::Levels(levels, 0))
                .ok()?;
        }
        let level_work = before - admission.remaining;
        Some((body_work, level_work))
    })();
    let inspected = limit - admission.remaining;
    *fuel -= inspected;
    let (body_work, level_work) = result?;
    if admission.max_depth > 64 {
        return None;
    }
    let reserve = if levels.is_some() {
        // The existing const door scans its header and may substitute a supplied
        // universe at every body node before lowering the instantiated body.
        body_work
            .checked_mul(level_work.max(1).checked_add(1)?)?
            .checked_mul(2)?
            .checked_add(inspected)?
    } else {
        body_work
    };
    if reserve > *fuel {
        return None;
    }
    *fuel -= reserve;
    if let Some(levels) = levels {
        const_body(ctx, name, levels)
    } else {
        definition_body(ctx, name)
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/delta/tests.rs"]
mod tests;
