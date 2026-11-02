//! Independent equality certification using de Bruijn conversion and compact numeral observation.
use super::*;
#[cfg(test)]
thread_local! {static CERTIFICATES:std::cell::Cell<usize>=const {std::cell::Cell::new(0)};}

pub(super) fn reduce(genv: &Context, lctx: &dyn LocalTypes, t: &Db) -> Option<Db> {
    if genv.checking_order_recurrence
        || !genv.simp_rules().is_empty()
        || WHNF_EXHAUSTED.with(|x| x.get())
    {
        return None;
    }
    let Db::App(f, right) = t else {
        return None;
    };
    let Db::App(head, left) = f.as_ref() else {
        return None;
    };
    let Db::Global(name) = head.as_ref() else {
        return None;
    };
    if genv.poly_declaration(name).is_some()
        || genv.is_constructor(name)
        || genv.is_inductive(name)
        || genv.has_pending_dependency(&Term::Global(name.clone()))
    {
        return None;
    }
    let signature = genv.get_definition_type(name)?;
    if genv.get_global(name) != Some(signature) {
        return None;
    }
    let mut result = signature;
    for _ in 0..2 {
        let Term::Pi {
            param_type,
            body_type,
            ..
        } = result
        else {
            return None;
        };
        if !matches!(&**param_type,Term::Global(n) if n=="Nat") {
            return None;
        }
        result = body_type;
    }
    let Term::Global(carrier) = result else {
        return None;
    };
    if genv.constructor_inventory_upper_bound("Nat") > 4096
        || !crate::int_inductive::canonical_nat_environment(genv)
        || !genv.is_inductive(carrier)
        || genv.get_global(carrier) != Some(&Term::Sort(Universe::Type(0)))
        || genv.get_definition_body(carrier).is_some()
        || genv.poly_declaration(carrier).is_some()
        || genv.inductive_num_params(carrier) != 0
        || genv.constructor_inventory_upper_bound(carrier) != 2
    {
        return None;
    }
    let constructors = genv.get_constructors(carrier);
    if constructors.len() != 2
        || constructors.iter().any(|(n, ty)| {
            *ty != result
                || genv.get_global(n) != Some(result)
                || genv.constructor_inductive(n) != Some(carrier.as_str())
                || genv.poly_declaration(n).is_some()
                || genv.get_definition_body(n).is_some()
        })
    {
        return None;
    }
    let parent = WHNF_DEPTH.with(|x| x.get());
    if parent == 0 || parent >= MAX_RECHECK_WHNF_DEPTH {
        return None;
    }
    let _guard = WhnfDepthGuard(parent);
    WHNF_DEPTH.with(|x| x.set(parent + 1));
    // A hit reuses only this kernel's four completed equations. Charge the
    // lookup to the active reduction allowance; exhausted calls cannot use it.
    if !nat_order_tick() {
        return None;
    }
    let outcomes = genv.cached_independent_equality_recurrence(name, || {
        #[cfg(test)]
        CERTIFICATES.with(|n| n.set(n.get() + 1));
        let body = genv.get_definition_body(name)?;
        let limit = WHNF_FUEL.with(|x| x.get().min(4096));
        let mut admission = LocalAdmission::new(limit);
        let free = admission.free_names(body);
        if !nat_order_spend(limit - admission.remaining + 1)
            || admission.max_depth > MAX_RECHECK_WHNF_DEPTH
            || !free.ok()?.is_empty()
        {
            return None;
        }
        to_db(body, &mut Vec::new()).ok()?;
        let ctx = genv.order_recurrence_context();
        let locals = vec![Db::Global("Nat".into()), Db::Global("Nat".into())];
        let g = |n: &str| Db::Global(n.into());
        let app = |f: Db, a: Db| Db::App(DbBox::new(f), DbBox::new(a));
        let call = |a: Db, b: Db| app(app(g(name), a), b);
        let succ = |a: Db| app(g("Succ"), a);
        let yes = constructors
            .iter()
            .position(|(n, _)| def_eq(&ctx, &locals, &call(g("Zero"), g("Zero")), &g(n)))?;
        let no = 1 - yes;
        if !def_eq(
            &ctx,
            &locals,
            &call(g("Zero"), succ(Db::Var(0))),
            &g(constructors[no].0),
        ) || !def_eq(
            &ctx,
            &locals,
            &call(succ(Db::Var(1)), g("Zero")),
            &g(constructors[no].0),
        ) || !def_eq(
            &ctx,
            &locals,
            &call(succ(Db::Var(1)), succ(Db::Var(0))),
            &call(Db::Var(1), Db::Var(0)),
        ) || WHNF_EXHAUSTED.with(|x| x.get())
        {
            return None;
        }
        Some([
            constructors[no].0.to_owned(),
            constructors[yes].0.to_owned(),
        ])
    })?;
    // Independent from the main kernel's two-order conjunction: observe both
    // finite numerals with this kernel's existing bounded arithmetic observer.
    // Unknown values remain inconclusive and use ordinary reduction.
    let a = arithmetic::numeral(genv, lctx, left)?;
    let b = arithmetic::numeral(genv, lctx, right)?;
    if WHNF_EXHAUSTED.with(|x| x.get()) {
        return None;
    }
    Some(Db::Global(outcomes[usize::from(a == b)].clone()))
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/equality/controls.rs"]
mod controls;
