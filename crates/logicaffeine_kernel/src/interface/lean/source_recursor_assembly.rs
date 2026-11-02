//! Canonical source rule and structure assembly using the shared bounded constructor.
use super::{declined, kernel, rejected, DiagnosticCode, Result};
use crate::recursor::bounded::{
    instantiate_universes_bounded, Construction, RecursorBudget, RecursorGenerationError,
};
use crate::term_store::{Node, TermId};
use crate::{Context, Term, Universe};

fn generation(error: RecursorGenerationError) -> super::Verdict {
    match error {
        RecursorGenerationError::ResourceLimit => declined(
            DiagnosticCode::ResourceLimit,
            "canonical source construction budget",
        ),
        RecursorGenerationError::Kernel(error) => kernel(error),
    }
}
fn invalid(message: &str) -> super::Verdict {
    rejected(DiagnosticCode::InvalidProof, message)
}
fn open_pi(d: &mut Construction<'_>, ty: TermId, name: &str) -> Result<(TermId, TermId)> {
    let Node::Pi {
        param,
        param_type,
        body_type,
    } = d.store.node(ty)
    else {
        return Err(invalid("canonical source telescope is too short"));
    };
    let (param, domain, body) = (*param, *param_type, *body_type);
    let value = d.var(name).map_err(generation)?;
    let body = d.instantiate(body, &[(param, value)]).map_err(generation)?;
    Ok((domain, body))
}
pub(super) fn ctor_rule_applied_bounded(
    ctx: &Context,
    rec: &str,
    levels: &[String],
    rec_ty: &Term,
    ctor_value: &Term,
    cty: &Term,
    nparams: usize,
    prefix: usize,
    result_parameters: usize,
    budget: &mut RecursorBudget,
) -> Result<Term> {
    if nparams > prefix {
        return Err(invalid("source rule parameters exceed its prefix"));
    }
    let mut d = Construction::new(budget).map_err(generation)?;
    let mut ty = d.intern(rec_ty).map_err(generation)?;
    let mut ctor = d.intern(ctor_value).map_err(generation)?;
    let mut cty = d.intern(cty).map_err(generation)?;
    let mut binders = Vec::new();
    for i in 0..prefix {
        d.budget.inventory(1).map_err(generation)?;
        let name = d
            .generated_name("arena.rule.", i, None)
            .map_err(generation)?;
        let (domain, body) = open_pi(&mut d, ty, &name)?;
        binders.push((name, domain));
        ty = body;
    }
    for (name, _) in binders.iter().take(nparams) {
        let (_, body) = open_pi(&mut d, cty, name)?;
        cty = body;
        let value = d.var(name).map_err(generation)?;
        ctor = d.app(ctor, value).map_err(generation)?;
    }
    while matches!(d.store.node(cty), Node::Pi { .. }) {
        d.budget.inventory(1).map_err(generation)?;
        let name = d
            .generated_name("arena.rule.", binders.len(), None)
            .map_err(generation)?;
        let (domain, body) = open_pi(&mut d, cty, &name)?;
        let value = d.var(&name).map_err(generation)?;
        ctor = d.app(ctor, value).map_err(generation)?;
        binders.push((name, domain));
        cty = body;
    }
    let (_, arguments) = d.spine(cty).map_err(generation)?;
    let name = d.symbol(rec).map_err(generation)?;
    let mut rhs = if levels.is_empty() {
        d.add(Node::Global(name)).map_err(generation)?
    } else {
        d.budget.inventory(levels.len()).map_err(generation)?;
        let mut instantiated = Vec::new();
        for level in levels {
            d.budget
                .reserve(level.len() as u64 + 64)
                .map_err(generation)?;
            instantiated.push(d.level(&Universe::Var(level.clone())).map_err(generation)?);
        }
        d.add(Node::Const {
            name,
            levels: instantiated.into_boxed_slice(),
        })
        .map_err(generation)?
    };
    for (name, _) in binders.iter().take(prefix) {
        let value = d.var(name).map_err(generation)?;
        rhs = d.app(rhs, value).map_err(generation)?;
    }
    for index in arguments.into_iter().skip(result_parameters) {
        rhs = d.app(rhs, index).map_err(generation)?;
    }
    rhs = d.app(rhs, ctor).map_err(generation)?;
    for (name, domain) in binders.into_iter().rev() {
        rhs = d.lam(&name, domain, rhs).map_err(generation)?;
    }
    let result = d.extract(rhs).map_err(generation)?;
    super::lean_type(ctx, &result)?;
    Ok(result)
}

pub(super) fn structure_recursor_body_bounded(
    ctx: &Context,
    rec_ty: &Term,
    nparams: usize,
    minor: usize,
    telescope: usize,
    projections: &[String],
    budget: &mut RecursorBudget,
) -> Result<Term> {
    if nparams >= minor || minor.checked_add(1).is_none_or(|n| n >= telescope) {
        return Err(invalid(
            "structure recursor minor premise outside its telescope",
        ));
    }
    let mut d = Construction::new(budget).map_err(generation)?;
    let mut ty = d.intern(rec_ty).map_err(generation)?;
    let mut binders = Vec::new();
    for i in 0..telescope {
        d.budget.inventory(1).map_err(generation)?;
        let name = d
            .generated_name("arena.structure.rec.", i, None)
            .map_err(generation)?;
        let (domain, body) = open_pi(&mut d, ty, &name)?;
        binders.push((name, domain));
        ty = body;
    }
    let (head, _) = d.spine(binders[telescope - 1].1).map_err(generation)?;
    // Only the small family head is extracted; whole carrier spines remain in the DAG.
    let head = d.extract(head).map_err(generation)?;
    let levels = match head {
        Term::Global(_) => Vec::new(),
        Term::Const { levels, .. } => levels,
        _ => {
            return Err(invalid(
                "structure recursor major premise is not a structure",
            ))
        }
    };
    let major = d.var(&binders[telescope - 1].0).map_err(generation)?;
    let mut result = d.var(&binders[minor].0).map_err(generation)?;
    for projection in projections {
        d.budget
            .spend(projection.len() as u64 + 1)
            .map_err(generation)?;
        let mut projected = if levels.is_empty() {
            let body = ctx
                .get_definition_body(projection)
                .ok_or_else(|| invalid("structure projection has no checked body"))?;
            d.intern(body).map_err(generation)?
        } else {
            let (declared, _, body) = ctx
                .get_universe_poly(projection)
                .ok_or_else(|| invalid("structure projection has no checked polymorphic body"))?;
            if declared.len() != levels.len() {
                return Err(invalid("structure projection universe arity mismatch"));
            }
            d.budget.inventory(declared.len()).map_err(generation)?;
            let mut substitution = std::collections::HashMap::new();
            for (name, level) in declared.iter().zip(&levels) {
                // Admit universe copying before constructing the substitution map.
                let level = copy_projection_level(&mut d, level)?;
                d.budget
                    .spend((name.len() as u64).saturating_mul(2).saturating_add(1))
                    .map_err(generation)?;
                d.budget
                    .reserve(name.len() as u64 + 128)
                    .map_err(generation)?;
                substitution.insert(name.clone(), level);
            }
            let body =
                instantiate_universes_bounded(body, &substitution, d.budget).map_err(generation)?;
            d.intern(&body).map_err(generation)?
        };
        for (name, _) in binders.iter().take(nparams) {
            let argument = d.var(name).map_err(generation)?;
            projected = apply_projection(&mut d, projected, argument)?;
        }
        projected = apply_projection(&mut d, projected, major)?;
        result = d.app(result, projected).map_err(generation)?;
    }
    for (name, domain) in binders.into_iter().rev() {
        result = d.lam(&name, domain, result).map_err(generation)?;
    }
    d.extract(result).map_err(generation)
}
// Materializing this Sort charges the complete universe tree and string payload before
// the owned replacement is inserted. Merely interning the borrowed level does not pay for
// a second recursive Universe::clone performed by the caller.
fn copy_projection_level(d: &mut Construction<'_>, level: &Universe) -> Result<Universe> {
    let level = d.level(level).map_err(generation)?;
    let sort = d.add(Node::Sort(level)).map_err(generation)?;
    let Term::Sort(level) = d.extract(sort).map_err(generation)? else {
        unreachable!("sort extraction")
    };
    Ok(level)
}
fn apply_projection(
    d: &mut Construction<'_>,
    projection: TermId,
    argument: TermId,
) -> Result<TermId> {
    let Node::Lambda { param, body, .. } = d.store.node(projection) else {
        return Err(invalid("structure projection arity mismatch"));
    };
    let (param, body) = (*param, *body);
    d.instantiate(body, &[(param, argument)])
        .map_err(generation)
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/source_recursor_assembly/controls.rs"]
mod controls;

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/source_recursor_assembly/accounting_controls.rs"]
mod accounting_controls;
