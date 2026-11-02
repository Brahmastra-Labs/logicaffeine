//! Bounded counterpart of source canonical derivation; shares one meter across the block.
use super::super::reference::{ReferenceInductiveBlock, ReferenceRecursor};
use super::super::{declined, exact, kernel, lean_type, normal, DiagnosticCode, Result, Verdict};
use super::{checked_body, invalid, CanonicalRecursor};
use crate::recursor::bounded::{
    derive_recursor_universe_poly_bounded, instantiate_universes_bounded, Construction,
    RecursorBudget, RecursorGenerationError,
};
use crate::term_store::Node;
use crate::{infer_type, Context, Term, Universe};
use std::collections::{HashMap, HashSet};

fn generation(error: RecursorGenerationError) -> Verdict {
    match error {
        RecursorGenerationError::ResourceLimit => declined(
            DiagnosticCode::ResourceLimit,
            "canonical source construction budget",
        ),
        RecursorGenerationError::Kernel(error) => kernel(error),
    }
}
fn inventory(b: &mut RecursorBudget, n: usize) -> Result<()> {
    b.inventory(n).map_err(generation)
}
fn text(b: &mut RecursorBudget, s: &str) -> Result<String> {
    b.spend(s.len() as u64 + 1).map_err(generation)?;
    b.reserve(s.len() as u64 + 64).map_err(generation)?;
    Ok(s.to_owned())
}
fn strings(b: &mut RecursorBudget, ss: &[String]) -> Result<Vec<String>> {
    inventory(b, ss.len())?;
    ss.iter().map(|s| text(b, s)).collect()
}
fn copy_term(b: &mut RecursorBudget, t: &Term) -> Result<Term> {
    let mut d = Construction::new(b).map_err(generation)?;
    let t = d.intern(t).map_err(generation)?;
    d.extract(t).map_err(generation)
}
fn admit_term(b: &mut RecursorBudget, t: &Term) -> Result<()> {
    let mut d = Construction::new(b).map_err(generation)?;
    d.intern(t).map_err(generation)?;
    Ok(())
}
fn admit_context_copy(b: &mut RecursorBudget, ctx: &Context) -> Result<()> {
    b.reserve(std::mem::size_of::<Context>() as u64 + 256)
        .map_err(generation)?;
    inventory(b, ctx.local_binding_capacity())?;
    for name in ctx.local_binding_names() {
        inventory(b, 1)?;
        b.spend(name.len() as u64 + 1).map_err(generation)?;
        b.reserve(name.len() as u64 + 128).map_err(generation)?;
    }
    Ok(())
}
fn context_copy(b: &mut RecursorBudget, ctx: &Context) -> Result<Context> {
    admit_context_copy(b, ctx)?;
    Ok(ctx.clone())
}
fn extend_context(b: &mut RecursorBudget, ctx: &Context, name: &str, ty: Term) -> Result<Context> {
    admit_context_copy(b, ctx)?;
    b.spend(name.len() as u64 + 1).map_err(generation)?;
    b.reserve(name.len() as u64 + 256).map_err(generation)?;
    Ok(ctx.extend(name, ty))
}
// Same exposed-telescope rules as the existing helper, consuming admitted trees rather
// than recursively cloning and rebuilding the Pi telescope only to discard it again.
fn declared_sort(ctx: &Context, b: &mut RecursorBudget, arity: &Term) -> Result<Universe> {
    let mut ctx = context_copy(b, ctx)?;
    let mut term = copy_term(b, arity)?;
    loop {
        b.spend(1).map_err(generation)?;
        match term {
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                ctx = extend_context(b, &ctx, &param, *param_type)?;
                term = *body_type;
            }
            Term::Sort(level) => return Ok(level),
            hidden => {
                term = normal(&ctx, &hidden)?;
                admit_term(b, &term)?;
                if !matches!(term, Term::Pi { .. } | Term::Sort(_)) {
                    return Err(invalid("source family arity does not end in a sort"));
                }
            }
        }
    }
}
pub(crate) fn telescope_len(ctx: &Context, b: &mut RecursorBudget, term: &Term) -> Result<usize> {
    let mut ctx = context_copy(b, ctx)?;
    let mut term = copy_term(b, term)?;
    let mut count = 0usize;
    loop {
        b.spend(1).map_err(generation)?;
        if let Term::Pi {
            param,
            param_type,
            body_type,
        } = term
        {
            ctx = extend_context(b, &ctx, &param, *param_type)?;
            term = *body_type;
            count = count.checked_add(1).ok_or_else(|| {
                declined(DiagnosticCode::ResourceLimit, "source telescope overflow")
            })?;
            continue;
        }
        let mut head = &term;
        while let Term::App(f, _) = head {
            b.spend(1).map_err(generation)?;
            head = f;
        }
        match head {
            Term::Global(name) | Term::Const { name, .. } => {
                b.spend(name.len() as u64 + 1).map_err(generation)?;
                if ctx.is_inductive(name) {
                    return Ok(count);
                }
            }
            Term::Sort(_) => return Ok(count),
            _ => {}
        }
        term = normal(&ctx, &term)?;
        admit_term(b, &term)?;
        if !matches!(term, Term::Pi { .. }) {
            return Ok(count);
        }
    }
}
fn rec_name(b: &mut RecursorBudget, owner: &str, index: Option<usize>) -> Result<String> {
    let bytes = (owner.len() as u64).saturating_add(48);
    b.spend(bytes).map_err(generation)?;
    b.reserve(bytes + 64).map_err(generation)?;
    Ok(match index {
        Some(i) => format!("{owner}/s:\"rec_{i}\""),
        None => format!("{owner}/s:\"rec\""),
    })
}
fn recursor_position(
    b: &mut RecursorBudget,
    recs: &[ReferenceRecursor<Term>],
    name: &str,
) -> Result<Option<usize>> {
    for (i, rec) in recs.iter().enumerate() {
        b.spend(
            (rec.declaration.name.len() as u64)
                .saturating_add(name.len() as u64)
                .saturating_add(1),
        )
        .map_err(generation)?;
        if rec.declaration.name == name {
            return Ok(Some(i));
        }
    }
    Ok(None)
}
fn same_levels(b: &mut RecursorBudget, left: &[String], right: &[String]) -> Result<bool> {
    b.spend(1).map_err(generation)?;
    if left.len() != right.len() {
        return Ok(false);
    }
    for (a, c) in left.iter().zip(right) {
        b.spend(
            (a.len() as u64)
                .saturating_add(c.len() as u64)
                .saturating_add(1),
        )
        .map_err(generation)?;
        if a != c {
            return Ok(false);
        }
    }
    Ok(true)
}
fn source_levels(b: &mut RecursorBudget, levels: &[String], term: &Term) -> Result<()> {
    inventory(b, levels.len())?;
    let mut allowed = HashSet::new();
    for l in levels {
        b.spend(l.len() as u64 + 1).map_err(generation)?;
        allowed.insert(l.as_str());
    }
    if allowed.len() != levels.len() {
        return Err(invalid(
            "source declaration has duplicate universe parameters",
        ));
    }
    let mut d = Construction::new(b).map_err(generation)?;
    let root = d.intern(term).map_err(generation)?;
    for used in d.free_level_vars(root).map_err(generation)? {
        d.budget.spend(used.len() as u64 + 1).map_err(generation)?;
        if !allowed.contains(used.as_str()) {
            return Err(invalid(
                "source declaration or rule uses an undeclared universe parameter",
            ));
        }
    }
    Ok(())
}
fn bind_type(
    ctx: &Context,
    b: &mut RecursorBudget,
    name: &str,
    levels: &[String],
    ty: &Term,
) -> Result<()> {
    source_levels(b, levels, ty)?;
    lean_type(ctx, ty)?;
    let (actual_levels, actual) = if let Some((ls, ty)) = ctx.poly_declaration(name) {
        (ls, ty)
    } else {
        (
            &[][..],
            ctx.get_global(name).ok_or_else(|| {
                invalid("source block declaration is absent from the certificate prefix")
            })?,
        )
    };
    if !same_levels(b, actual_levels, levels)? {
        return Err(invalid(
            "source block universe parameters differ from the certificate",
        ));
    }
    exact(ctx, actual, ty)
}
fn bind_block(
    ctx: &Context,
    block: &ReferenceInductiveBlock<Term>,
    b: &mut RecursorBudget,
) -> Result<()> {
    let first = block
        .members
        .first()
        .ok_or_else(|| invalid("empty source block"))?;
    // Shallow inventories and all source text are charged before source inventory scans.
    inventory(b, block.members.len())?;
    inventory(b, block.constructors.len())?;
    inventory(b, block.recursors.len())?;
    let count = (block.members.len() as u64)
        .saturating_add(block.constructors.len() as u64)
        .saturating_add(block.recursors.len() as u64)
        .saturating_add(1);
    let mut bytes = 0u64;
    for m in &block.members {
        inventory(b, m.all.len())?;
        inventory(b, m.constructors.len())?;
        inventory(b, m.declaration.level_params.len())?;
        bytes = bytes.saturating_add(m.declaration.name.len() as u64);
        for name in m
            .all
            .iter()
            .chain(&m.constructors)
            .chain(&m.declaration.level_params)
        {
            bytes = bytes.saturating_add(name.len() as u64 + 1);
        }
    }
    for c in &block.constructors {
        inventory(b, c.declaration.level_params.len())?;
        bytes = bytes
            .saturating_add(c.parent.len() as u64)
            .saturating_add(c.declaration.name.len() as u64);
        for name in &c.declaration.level_params {
            bytes = bytes.saturating_add(name.len() as u64 + 1);
        }
    }
    for r in &block.recursors {
        inventory(b, r.rules.len())?;
        inventory(b, r.all.len())?;
        inventory(b, r.declaration.level_params.len())?;
        bytes = bytes.saturating_add(r.declaration.name.len() as u64);
        for name in r.all.iter().chain(&r.declaration.level_params) {
            bytes = bytes.saturating_add(name.len() as u64 + 1);
        }
        for rule in &r.rules {
            bytes = bytes.saturating_add(rule.constructor.len() as u64 + 1);
        }
    }
    b.spend(
        bytes
            .saturating_add(1)
            .saturating_mul(count)
            .saturating_mul(8),
    )
    .map_err(generation)?;
    let all: Vec<&str> = block
        .members
        .iter()
        .map(|m| m.declaration.name.as_str())
        .collect();
    let mut constructors = HashSet::new();
    for member in &block.members {
        let source = &member.declaration;
        if source.is_unsafe
            || !member
                .all
                .iter()
                .map(String::as_str)
                .eq(all.iter().copied())
            || member.num_params != first.num_params
            || member.num_nested != first.num_nested
            || source.level_params != first.declaration.level_params
        {
            return Err(invalid(
                "source block safety, membership or common telescope mismatch",
            ));
        }
        bind_type(ctx, b, &source.name, &source.level_params, &source.ty)?;
        if !ctx.is_inductive(&source.name)
            || ctx.inductive_num_params(&source.name) != member.num_params
            || ctx.inductive_num_indices(&source.name) != member.num_indices
        {
            return Err(invalid(
                "certificate datatype parameter/index metadata differs from the source",
            ));
        }
        let same = match ctx.mutual_block_of(&source.name) {
            Some(actual) => actual.iter().map(String::as_str).eq(all.iter().copied()),
            None => all.len() == 1 && all[0] == source.name,
        };
        if !same {
            return Err(invalid(
                "certificate mutual block differs from source order",
            ));
        }
        if member.is_recursive != ctx.block_is_recursive(&source.name) {
            return Err(invalid(
                "source recursion flag differs from the checked inductive block",
            ));
        }
        // As in derive, is_reflexive is descriptive, unused in generation, and not certified.
        inventory(b, ctx.constructor_inventory_upper_bound(&source.name))?;
        if !ctx
            .constructor_headers(&source.name)
            .map(|(name, _)| name)
            .eq(member.constructors.iter().map(String::as_str))
        {
            return Err(invalid(
                "certificate constructor inventory differs from the source",
            ));
        }
        for (ordinal, name) in member.constructors.iter().enumerate() {
            let constructor = block
                .constructors
                .iter()
                .find(|c| c.declaration.name == *name)
                .ok_or_else(|| invalid("source constructor declaration missing"))?;
            if !constructors.insert(name.as_str())
                || constructor.parent != source.name
                || constructor.index != ordinal
                || constructor.num_params != member.num_params
                || constructor.declaration.is_unsafe
                || constructor.declaration.level_params != source.level_params
            {
                return Err(invalid(
                    "source constructor ownership/order/telescope mismatch",
                ));
            }
            bind_type(
                ctx,
                b,
                name,
                &constructor.declaration.level_params,
                &constructor.declaration.ty,
            )?;
            if constructor.num_params.checked_add(constructor.num_fields)
                != Some(telescope_len(ctx, b, &constructor.declaration.ty)?)
            {
                return Err(invalid("source constructor field count mismatch"));
            }
        }
    }
    if constructors.len() != block.constructors.len() {
        return Err(invalid("source block contains a foreign constructor"));
    }
    for rec in &block.recursors {
        source_levels(b, &rec.declaration.level_params, &rec.declaration.ty)?;
        lean_type(ctx, &rec.declaration.ty)?;
        for rule in &rec.rules {
            source_levels(b, &rec.declaration.level_params, &rule.rhs)?;
        }
        if rec.declaration.is_unsafe || !rec.all.iter().map(String::as_str).eq(all.iter().copied())
        {
            return Err(invalid("source recursor safety or membership mismatch"));
        }
        if ctx.get_global(&rec.declaration.name).is_some()
            || ctx.poly_declaration(&rec.declaration.name).is_some()
        {
            return Err(invalid("canonical source block must be derived before its first carried recursor is admitted"));
        }
    }
    Ok(())
}
fn universe_map(
    b: &mut RecursorBudget,
    keys: &[String],
    values: impl IntoIterator<Item = Option<String>>,
) -> Result<HashMap<String, Universe>> {
    inventory(b, keys.len())?;
    let mut map = HashMap::new();
    for (key, value) in keys.iter().zip(values) {
        let key = text(b, key)?;
        b.spend(key.len() as u64 + 1).map_err(generation)?;
        map.insert(key, value.map_or(Universe::Prop, Universe::Var));
    }
    Ok(map)
}
fn family_map(
    b: &mut RecursorBudget,
    keys: &[String],
    levels: &[String],
) -> Result<HashMap<String, Universe>> {
    if keys.len() != levels.len() {
        return Err(invalid("source family universe inventory mismatch"));
    }
    let values = strings(b, levels)?.into_iter().map(Some);
    universe_map(b, keys, values)
}
fn motive_map(
    b: &mut RecursorBudget,
    derived: &[String],
    levels: &[String],
    count: usize,
    large: bool,
) -> Result<HashMap<String, Universe>> {
    inventory(b, derived.len())?;
    let mut values = Vec::new();
    let offset = usize::from(large);
    for i in 0..derived.len() {
        values.push(if i < count {
            if large {
                Some(text(b, &levels[0])?)
            } else {
                None
            }
        } else {
            Some(text(b, &levels[offset + i - count])?)
        });
    }
    universe_map(b, derived, values)
}
fn instantiate(b: &mut RecursorBudget, t: &Term, sub: &HashMap<String, Universe>) -> Result<Term> {
    instantiate_universes_bounded(t, sub, b).map_err(generation)
}
fn constructor_value(b: &mut RecursorBudget, name: &str, levels: &[Universe]) -> Result<Term> {
    let mut d = Construction::new(b).map_err(generation)?;
    let name = d.symbol(name).map_err(generation)?;
    let root = if levels.is_empty() {
        d.add(Node::Global(name)).map_err(generation)?
    } else {
        inventory(d.budget, levels.len())?;
        let mut ls = Vec::new();
        for level in levels {
            ls.push(d.level(level).map_err(generation)?);
        }
        d.add(Node::Const {
            name,
            levels: ls.into_boxed_slice(),
        })
        .map_err(generation)?
    };
    d.extract(root).map_err(generation)
}
fn check_and_register(
    ctx: &mut Context,
    b: &mut RecursorBudget,
    name: &str,
    levels: &[String],
    ty: Term,
    body: Term,
) -> Result<CanonicalRecursor> {
    b.spend((name.len() as u64 + 1).saturating_mul(4))
        .map_err(generation)?;
    b.reserve((name.len() as u64 + 64).saturating_mul(4))
        .map_err(generation)?;
    checked_body(ctx, name, &body, &ty)?;
    let record = CanonicalRecursor {
        name: text(b, name)?,
        levels: strings(b, levels)?,
        ty: copy_term(b, &ty)?,
        body: copy_term(b, &body)?,
    };
    if levels.is_empty() {
        ctx.declare(name, ty, body).map_err(kernel)?;
    } else {
        ctx.declare_universe_poly(name, strings(b, levels)?, ty, body)
            .map_err(kernel)?;
    }
    Ok(record)
}

/// Source declaration binding and every rule check are retained. The caller shares `budget`
/// across the whole operation, including other blocks, and reserves its configured worker stack.
pub(crate) fn derive_bounded(
    ctx: &Context,
    block: &ReferenceInductiveBlock<Term>,
    budget: &mut RecursorBudget,
) -> Result<Vec<CanonicalRecursor>> {
    if budget.max_depth == 0 || budget.max_depth > crate::type_checker::MAX_TERM_DEPTH {
        return Err(declined(
            DiagnosticCode::ResourceLimit,
            "canonical source worker depth",
        ));
    }
    crate::term_store::grow_stack_for_depth(budget.max_depth, || derive_inner(ctx, block, budget))
}
fn derive_inner(
    ctx: &Context,
    block: &ReferenceInductiveBlock<Term>,
    budget: &mut RecursorBudget,
) -> Result<Vec<CanonicalRecursor>> {
    bind_block(ctx, block, budget)?;
    let first = &block.members[0];
    let mut scratch = context_copy(budget, ctx)?;
    if first.num_nested == 0 {
        return ordinary(&mut scratch, block, budget);
    }
    inventory(budget, block.members.len())?;
    let mut members = Vec::new();
    for member in &block.members {
        let name = &member.declaration.name;
        let sort = ctx.get_global(name).ok_or_else(|| {
            invalid("bound nested source family is absent from the registered prefix")
        })?;
        inventory(budget, ctx.constructor_inventory_upper_bound(name))?;
        let mut constructors = Vec::new();
        for (name, ty) in ctx.constructor_headers(name) {
            inventory(budget, 1)?;
            constructors.push((text(budget, name)?, copy_term(budget, ty)?));
        }
        members.push(crate::NestedDecl {
            name: text(budget, name)?,
            sort: copy_term(budget, sort)?,
            constructors,
        });
    }
    nested(
        &mut scratch,
        &members,
        first.num_params,
        &first.declaration.level_params,
        &block.recursors,
        &first.all,
        first.num_nested,
        budget,
    )
}
fn ordinary(
    ctx: &mut Context,
    block: &ReferenceInductiveBlock<Term>,
    b: &mut RecursorBudget,
) -> Result<Vec<CanonicalRecursor>> {
    if block.recursors.len() != block.members.len() {
        return Err(invalid("source ordinary recursor inventory mismatch"));
    }
    let first = &block.members[0];
    let params = &first.declaration.level_params;
    let nparams = first.num_params;
    let motives = block.members.len();
    let minors = block.constructors.len();
    let prefix = nparams
        .checked_add(motives)
        .and_then(|n| n.checked_add(minors))
        .ok_or_else(|| {
            declined(
                DiagnosticCode::ResourceLimit,
                "source recursor prefix overflow",
            )
        })?;
    let telescope = prefix.checked_add(1).ok_or_else(|| {
        declined(
            DiagnosticCode::ResourceLimit,
            "source recursor telescope overflow",
        )
    })?;
    let single = motives == 1;
    let mut common_sort: Option<Universe> = None;
    for member in &block.members {
        let level = declared_sort(ctx, b, &member.declaration.ty)?;
        if common_sort
            .as_ref()
            .is_some_and(|common| !common.equiv(&level))
        {
            return Err(invalid("source mutual members inhabit different sorts"));
        }
        common_sort = Some(level);
    }
    let large = Universe::Type(0).is_subtype_of(common_sort.as_ref().expect("nonempty block"));
    inventory(b, block.members.len())?;
    let mut result = Vec::new();
    let mut minor_offset = nparams + motives;
    for member in &block.members {
        let name = rec_name(b, &member.declaration.name, None)?;
        let position = recursor_position(b, &block.recursors, &name)?
            .ok_or_else(|| invalid("source block lacks canonical recursor name"))?;
        let source = &block.recursors[position];
        if source.num_params != nparams
            || source.num_indices != member.num_indices
            || source.num_motives != motives
            || source.num_minors != minors
            || source.is_k != (single && ctx.k_constructor(&member.declaration.name).is_some())
        {
            return Err(invalid(
                "source ordinary recursor telescope or K metadata mismatch",
            ));
        }
        let levels = &source.declaration.level_params;
        let (derived_levels, ty, body) =
            derive_recursor_universe_poly_bounded(ctx, &member.declaration.name, b)
                .map_err(generation)?;
        let substitution = if single {
            if levels.len() != derived_levels.len() {
                return Err(invalid("source recursor universe arity mismatch"));
            }
            family_map(b, &derived_levels, levels)?
        } else {
            let count = derived_levels
                .len()
                .checked_sub(params.len())
                .filter(|&count| derived_levels[count..] == *params)
                .ok_or_else(|| invalid("canonical mutual universe inventory mismatch"))?;
            let offset = usize::from(large);
            if levels.len() != params.len() + offset {
                return Err(invalid("source mutual motive universe mismatch"));
            }
            motive_map(b, &derived_levels, levels, count, large)?
        };
        let ty = instantiate(b, &ty, &substitution)?;
        let mut body = instantiate(b, &body, &substitution)?;
        if let Some(structure) = ctx.struct_info(&member.declaration.name) {
            body = super::super::source_recursor_assembly::structure_recursor_body_bounded(
                ctx,
                &ty,
                nparams,
                minor_offset,
                telescope,
                &structure.projections,
                b,
            )?;
        }
        exact(ctx, &source.declaration.ty, &ty)?;
        result.push(check_and_register(ctx, b, &name, levels, ty, body)?);
        minor_offset = minor_offset
            .checked_add(member.constructors.len())
            .ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "source minor offset overflow",
                )
            })?;
    }
    for (member, canonical) in block.members.iter().zip(&result) {
        let position = recursor_position(b, &block.recursors, &canonical.name)?
            .ok_or_else(|| invalid("source block lacks canonical recursor name"))?;
        let source = &block.recursors[position];
        if source.rules.len() != member.constructors.len() {
            return Err(invalid("source computation rule count mismatch"));
        }
        let offset = canonical
            .levels
            .len()
            .checked_sub(params.len())
            .ok_or_else(|| invalid("source rule universes"))?;
        let substitution = family_map(b, params, &canonical.levels[offset..])?;
        inventory(b, source.rules.len())?;
        let mut seen = HashSet::new();
        for rule in &source.rules {
            let constructor = block
                .constructors
                .iter()
                .find(|c| {
                    c.declaration.name == rule.constructor && c.parent == member.declaration.name
                })
                .ok_or_else(|| {
                    invalid("source computation rule belongs to a foreign constructor")
                })?;
            b.spend(rule.constructor.len() as u64 + 1)
                .map_err(generation)?;
            if !seen.insert(rule.constructor.as_str()) || rule.num_fields != constructor.num_fields
            {
                return Err(invalid(
                    "source computation rule field count or uniqueness mismatch",
                ));
            }
            inventory(b, params.len())?;
            let mut levels = Vec::new();
            for level in &canonical.levels[offset..] {
                levels.push(Universe::Var(text(b, level)?));
            }
            let value = constructor_value(b, &rule.constructor, &levels)?;
            let ctor_ty = instantiate(b, &constructor.declaration.ty, &substitution)?;
            let expected = super::super::source_recursor_assembly::ctor_rule_applied_bounded(
                ctx,
                &canonical.name,
                &canonical.levels,
                &canonical.ty,
                &value,
                &ctor_ty,
                nparams,
                prefix,
                nparams,
                b,
            )?;
            let actual_ty = lean_type(ctx, &rule.rhs)?;
            let expected_ty = infer_type(ctx, &expected).map_err(kernel)?;
            exact(ctx, &actual_ty, &expected_ty)?;
            exact(ctx, &rule.rhs, &expected)?;
        }
    }
    Ok(result)
}
fn stage(b: &mut RecursorBudget, name: &str, suffix: &str) -> Result<String> {
    let n = (name.len() as u64)
        .saturating_add(suffix.len() as u64)
        .saturating_add(32);
    b.spend(n).map_err(generation)?;
    b.reserve(n + 64).map_err(generation)?;
    Ok(format!("native recursor {name} {suffix}"))
}
fn family_sort(ctx: &Context, b: &mut RecursorBudget, carrier: &Term) -> Result<Universe> {
    admit_term(b, carrier)?;
    let mut ty = normal(ctx, &infer_type(ctx, carrier).map_err(kernel)?)?;
    admit_term(b, &ty)?;
    while let Term::Pi { body_type, .. } = ty {
        ty = *body_type;
    }
    match ty {
        Term::Sort(level) => Ok(level),
        _ => Err(invalid("a nested carrier is not a family of types")),
    }
}
struct Constructor {
    name: String,
    value: Term,
    ty: Term,
    result_parameters: usize,
    fields: usize,
}
fn constructor_group(
    ctx: &Context,
    local: &Context,
    b: &mut RecursorBudget,
    carrier: &Term,
) -> Result<(usize, Vec<Constructor>)> {
    let mut d = Construction::new(b).map_err(generation)?;
    let root = d.intern(carrier).map_err(generation)?;
    let (head, arguments) = d.spine(root).map_err(generation)?;
    let (name, polymorphic) = match d.store.node(head) {
        Node::Global(name) => (*name, false),
        Node::Const { name, .. } => (*name, true),
        _ => {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "native nested carrier has no inductive head",
            ))
        }
    };
    let parent = text(d.budget, d.store.name(name))?;
    if ctx.inductive_num_params(&parent) != arguments.len() {
        return Err(declined(
            DiagnosticCode::UnsupportedFeature,
            "native nested container parameters are not fully fixed",
        ));
    }
    let indices = ctx
        .inductive_arity(&parent)
        .checked_sub(arguments.len())
        .ok_or_else(|| invalid("native nested carrier has too many parameters"))?;
    inventory(d.budget, ctx.constructor_inventory_upper_bound(&parent))?;
    let mut constructors = Vec::new();
    for (name, _) in ctx.constructor_headers(&parent) {
        inventory(d.budget, 1)?;
        let symbol = d.symbol(name).map_err(generation)?;
        let mut value = if polymorphic {
            let Node::Const { levels, .. } = d.store.node(head) else {
                unreachable!("polymorphic head")
            };
            inventory(d.budget, levels.len())?;
            let levels = levels.to_vec().into_boxed_slice();
            d.add(Node::Const {
                name: symbol,
                levels,
            })
            .map_err(generation)?
        } else {
            d.add(Node::Global(symbol)).map_err(generation)?
        };
        let mut ty = d.infer(ctx, value).map_err(generation)?;
        for &argument in &arguments {
            let Node::Pi {
                param, body_type, ..
            } = d.store.node(ty)
            else {
                return Err(declined(
                    DiagnosticCode::UnsupportedFeature,
                    "native constructor parameter telescope unavailable",
                ));
            };
            let (param, body) = (*param, *body_type);
            ty = d
                .instantiate(body, &[(param, argument)])
                .map_err(generation)?;
            value = d.app(value, argument).map_err(generation)?;
        }
        let ty = d.extract(ty).map_err(generation)?;
        let fields = telescope_len(local, d.budget, &ty)?;
        constructors.push(Constructor {
            name: text(d.budget, name)?,
            value: d.extract(value).map_err(generation)?,
            ty,
            result_parameters: arguments.len(),
            fields,
        });
    }
    Ok((indices, constructors))
}
fn at_rule(
    b: &mut RecursorBudget,
    term: &Term,
    parameters: &[(String, Term)],
    family_levels: &[String],
    levels: &[String],
) -> Result<Term> {
    let universes = family_map(b, family_levels, levels)?;
    let term = instantiate(b, term, &universes)?;
    let mut d = Construction::new(b).map_err(generation)?;
    let mut root = d.intern(&term).map_err(generation)?;
    // Preserve the original helper's sequential substitutions and binder names.
    for (i, (parameter, _)) in parameters.iter().enumerate() {
        let name = d
            .generated_name("arena.rule.", i, None)
            .map_err(generation)?;
        let variable = d.var(&name).map_err(generation)?;
        let parameter = d.symbol(parameter).map_err(generation)?;
        root = d
            .instantiate(root, &[(parameter, variable)])
            .map_err(generation)?;
    }
    d.extract(root).map_err(generation)
}
fn nested(
    ctx: &mut Context,
    members: &[crate::NestedDecl],
    nparams: usize,
    family_levels: &[String],
    recs: &[ReferenceRecursor<Term>],
    all: &[String],
    nested_count: usize,
    b: &mut RecursorBudget,
) -> Result<Vec<CanonicalRecursor>> {
    let expected = nested_count.checked_add(members.len()).ok_or_else(|| {
        declined(
            DiagnosticCode::ResourceLimit,
            "native nested inventory overflow",
        )
    })?;
    if recs.len() != expected {
        return Err(invalid("native nested recursor inventory mismatch"));
    }
    inventory(b, expected)?;
    let native =
        crate::inductive_compile::native_recursors_bounded::derive_nested_block_recursors_bounded(
            ctx,
            &crate::inductive_compile::NestedBlock {
                members,
                num_params: nparams,
                levels: family_levels,
            },
            expected,
            b,
        )
        .map_err(generation)?;
    let motives = native.len();
    if expected != motives {
        return Err(invalid("native nested recursor inventory mismatch"));
    }
    let parameters = &native
        .first()
        .ok_or_else(|| invalid("native nested recursor inventory mismatch"))?
        .parameters;
    if parameters.len() != nparams {
        return Err(declined(
            DiagnosticCode::UnsupportedFeature,
            "native nested reconstruction lost the uniform parameter telescope",
        ));
    }
    let mut local = context_copy(b, ctx)?;
    for (parameter, domain) in parameters {
        let domain = copy_term(b, domain)?;
        local = extend_context(b, &local, parameter, domain)?;
    }
    let block_sort = family_sort(&local, b, &native[0].carrier)?;
    inventory(b, native.len())?;
    inventory(b, native.len())?;
    let mut constructor_groups = Vec::new();
    let mut indices = Vec::new();
    let mut minors = 0usize;
    for rec in &native {
        if !family_sort(&local, b, &rec.carrier)?.equiv(&block_sort) {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "native nested container outside the family's universe",
            ));
        }
        let (index, constructors) = constructor_group(ctx, &local, b, &rec.carrier)?;
        indices.push(index);
        minors = minors.checked_add(constructors.len()).ok_or_else(|| {
            declined(
                DiagnosticCode::ResourceLimit,
                "native minor inventory overflow",
            )
        })?;
        constructor_groups.push(constructors);
    }
    let prefix = nparams
        .checked_add(motives)
        .and_then(|n| n.checked_add(minors))
        .ok_or_else(|| {
            declined(
                DiagnosticCode::ResourceLimit,
                "native recursor prefix overflow",
            )
        })?;
    inventory(b, native.len())?;
    inventory(b, native.len())?;
    let mut checked = Vec::new();
    let mut result = Vec::new();
    for (i, rec) in native.iter().enumerate() {
        let name = if i < members.len() {
            rec_name(b, &members[i].name, None)?
        } else {
            rec_name(b, &members[0].name, Some(i + 1 - members.len()))?
        };
        let position = recursor_position(b, recs, &name)?
            .ok_or_else(|| invalid("missing canonical native nested recursor"))?;
        let exported = &recs[position];
        if exported.num_params != nparams
            || exported.num_indices != indices[i]
            || exported.num_motives != motives
            || exported.num_minors != minors
        {
            return Err(invalid("native nested recursor telescope counts mismatch"));
        }
        if exported.all != all {
            return Err(invalid("native nested recursor block membership mismatch"));
        }
        if exported.is_k {
            return Err(invalid("a nested recursor has an invalid K flag"));
        }
        let large = Universe::Type(0).is_subtype_of(&block_sort);
        let levels = &exported.declaration.level_params;
        if levels.len() != usize::from(large) + family_levels.len() {
            return Err(invalid(
                "native nested recursor universe telescope mismatch",
            ));
        }
        let derived_motives = rec
            .levels
            .len()
            .checked_sub(family_levels.len())
            .filter(|&m| rec.levels[m..] == *family_levels)
            .ok_or_else(|| {
                declined(
                    DiagnosticCode::UnsupportedFeature,
                    "native reconstruction has unexpected motive universe parameters",
                )
            })?;
        let substitution = motive_map(b, &rec.levels, levels, derived_motives, large)?;
        let ty = instantiate(b, &rec.ty, &substitution)?;
        let body = instantiate(b, &rec.body, &substitution)?;
        let expected = &exported.declaration.ty;
        lean_type(ctx, expected)?;
        let location = stage(b, &name, "type")?;
        exact(ctx, expected, &ty).map_err(|error| error.located(&location))?;
        let expected = copy_term(b, expected)?;
        result.push(check_and_register(ctx, b, &name, levels, expected, body)?);
        checked.push((name, position, ty));
    }
    let offset = usize::from(Universe::Type(0).is_subtype_of(&block_sort));
    for ((name, position, ty), constructors) in checked.iter().zip(&constructor_groups) {
        let rec = &recs[*position];
        let levels = &rec.declaration.level_params;
        let rules = &rec.rules;
        if rules.len() != constructors.len() {
            return Err(invalid(
                "native nested recursor computation rule inventory mismatch",
            ));
        }
        inventory(b, rules.len())?;
        let mut seen = HashSet::new();
        for rule in rules {
            let mut found = None;
            for candidate in constructors {
                b.spend(
                    (candidate.name.len() as u64)
                        .saturating_add(rule.constructor.len() as u64)
                        .saturating_add(1),
                )
                .map_err(generation)?;
                if candidate.name == rule.constructor {
                    found = Some(candidate);
                    break;
                }
            }
            let Some(constructor) = found else {
                return Err(invalid("native nested rule names a foreign constructor"));
            };
            b.spend(rule.constructor.len() as u64 + 1)
                .map_err(generation)?;
            if !seen.insert(rule.constructor.as_str()) || rule.num_fields != constructor.fields {
                return Err(invalid(
                    "native nested rule has duplicate constructor or wrong field count",
                ));
            }
            let value = at_rule(
                b,
                &constructor.value,
                parameters,
                family_levels,
                &levels[offset..],
            )?;
            let ctor_ty = at_rule(
                b,
                &constructor.ty,
                parameters,
                family_levels,
                &levels[offset..],
            )?;
            let expected = super::super::source_recursor_assembly::ctor_rule_applied_bounded(
                ctx,
                name,
                levels,
                ty,
                &value,
                &ctor_ty,
                0,
                prefix,
                constructor.result_parameters,
                b,
            )?;
            let actual_ty = lean_type(ctx, &rule.rhs)?;
            let expected_ty = infer_type(ctx, &expected).map_err(kernel)?;
            let location = stage(b, name, "rule type")?;
            exact(ctx, &actual_ty, &expected_ty).map_err(|error| error.located(&location))?;
            let location = stage(b, name, "rule value")?;
            exact(ctx, &rule.rhs, &expected).map_err(|error| error.located(&location))?;
        }
    }
    Ok(result)
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/source_recursor/bounded/controls.rs"]
mod controls;
