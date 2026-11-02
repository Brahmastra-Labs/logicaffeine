//! Canonical recursor construction from independently decoded source metadata.
//! This module is a local implementation draft; package admission has not been wired to it.

use super::reference::{ReferenceInductiveBlock, ReferenceRecursor};
use super::{
    apply, ctor_rule_applied, declared_arity, exact, independent_comparison, inductive_verdict,
    kernel, lean_type, normal, resolved_telescope_len, structure_recursor_body,
};
use super::{declined, rejected, DiagnosticCode, Result};
use crate::{infer_type, Context, Term, Universe};
use std::collections::{HashMap, HashSet};

#[derive(Clone)]
pub(super) struct CanonicalRecursor {
    pub name: String,
    pub levels: Vec<String>,
    pub ty: Term,
    pub body: Term,
}

fn invalid(message: &str) -> super::Verdict {
    rejected(DiagnosticCode::InvalidProof, message)
}

#[cfg(test)]
fn source_levels(levels: &[String], terms: &[&Term]) -> Result<()> {
    let allowed: HashSet<&str> = levels.iter().map(String::as_str).collect();
    if allowed.len() != levels.len() {
        return Err(invalid(
            "source declaration has duplicate universe parameters",
        ));
    }
    for term in terms {
        if crate::term::free_level_vars(term)
            .iter()
            .any(|level| !allowed.contains(level.as_str()))
        {
            return Err(invalid(
                "source declaration or rule uses an undeclared universe parameter",
            ));
        }
    }
    Ok(())
}

fn checked_body(ctx: &Context, name: &str, body: &Term, ty: &Term) -> Result<()> {
    let main = infer_type(ctx, body).map_err(kernel)?;
    exact(ctx, &main, ty)?;
    let independent = crate::recheck(ctx, body).map_err(|error| {
        super::independent_verdict(&error, format!("canonical source recursor {name}"))
    })?;
    independent_comparison(
        exact(ctx, &independent, ty),
        format!("canonical source recursor {name} type"),
    )
}

#[cfg(test)]
fn bind_type(ctx: &Context, name: &str, levels: &[String], ty: &Term) -> Result<()> {
    source_levels(levels, &[ty])?;
    // Source syntax uses Lean's exact typing discipline, even when its normal form agrees.
    lean_type(ctx, ty)?;
    let (actual_levels, actual) = if let Some((levels, ty)) = ctx.poly_declaration(name) {
        (levels, ty)
    } else {
        (
            &[][..],
            ctx.get_global(name).ok_or_else(|| {
                invalid("source block declaration is absent from the certificate prefix")
            })?,
        )
    };
    if actual_levels != levels {
        return Err(invalid(
            "source block universe parameters differ from the certificate",
        ));
    }
    exact(ctx, actual, ty)
}

/// Bind the datatype metadata before deriving any eliminator from the certificate context.
/// A matching spelling alone never establishes a matching datatype.
#[cfg(test)]
fn bind_block(ctx: &Context, block: &ReferenceInductiveBlock<Term>) -> Result<()> {
    let first = block
        .members
        .first()
        .ok_or_else(|| invalid("empty source block"))?;
    let all: Vec<String> = block
        .members
        .iter()
        .map(|m| m.declaration.name.clone())
        .collect();
    let mut constructors = HashSet::new();
    for member in &block.members {
        let source = &member.declaration;
        if source.is_unsafe
            || member.all != all
            || member.num_params != first.num_params
            || member.num_nested != first.num_nested
            || source.level_params != first.declaration.level_params
        {
            return Err(invalid(
                "source block safety, membership or common telescope mismatch",
            ));
        }
        bind_type(ctx, &source.name, &source.level_params, &source.ty)?;
        if !ctx.is_inductive(&source.name)
            || ctx.inductive_num_params(&source.name) != member.num_params
            || ctx.inductive_num_indices(&source.name) != member.num_indices
        {
            return Err(invalid(
                "certificate datatype parameter/index metadata differs from the source",
            ));
        }
        let actual_all: Vec<_> = ctx
            .mutual_block_of(&source.name)
            .map(|names| names.to_vec())
            .unwrap_or_else(|| vec![source.name.clone()]);
        if actual_all != all {
            return Err(invalid(
                "certificate mutual block differs from source order",
            ));
        }
        // This source flag controls Lean's structure-like recursor reduction. Derive its
        // value from the checked whole block rather than trusting the serialized Boolean.
        if member.is_recursive != ctx.block_is_recursive(&source.name) {
            return Err(invalid(
                "source recursion flag differs from the checked inductive block",
            ));
        }
        // is_reflexive describes higher-order recursive fields. Generation derives those
        // fields from checked constructor types and verifies every source computation rule;
        // it does not use or independently certify that descriptive source Boolean.
        let actual = ctx.get_constructors(&source.name);
        if actual
            .iter()
            .map(|(name, _)| name.to_string())
            .collect::<Vec<_>>()
            != member.constructors
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
            if !constructors.insert(name.clone())
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
                name,
                &constructor.declaration.level_params,
                &constructor.declaration.ty,
            )?;
            if constructor.num_params.checked_add(constructor.num_fields)
                != Some(resolved_telescope_len(ctx, &constructor.declaration.ty)?)
            {
                return Err(invalid("source constructor field count mismatch"));
            }
        }
    }
    if constructors.len() != block.constructors.len() {
        return Err(invalid("source block contains a foreign constructor"));
    }
    for recursor in &block.recursors {
        source_levels(
            &recursor.declaration.level_params,
            &[&recursor.declaration.ty],
        )?;
        lean_type(ctx, &recursor.declaration.ty)?;
        for rule in &recursor.rules {
            source_levels(&recursor.declaration.level_params, &[&rule.rhs])?;
        }
        if recursor.declaration.is_unsafe || recursor.all != all {
            return Err(invalid("source recursor safety or membership mismatch"));
        }
        if ctx.get_global(&recursor.declaration.name).is_some()
            || ctx.poly_declaration(&recursor.declaration.name).is_some()
        {
            return Err(invalid("canonical source block must be derived before its first carried recursor is admitted"));
        }
    }
    Ok(())
}

/// The caller caches this whole result before observing any recursor in the block.
#[cfg(test)]
pub(super) fn derive(
    ctx: &Context,
    block: &ReferenceInductiveBlock<Term>,
) -> Result<Vec<CanonicalRecursor>> {
    bind_block(ctx, block)?;
    let first = &block.members[0];
    let mut scratch = ctx.clone();
    if first.num_nested != 0 {
        // bind_block has already checked the independent source signatures by exact
        // conversion, universe telescope, ownership and constructor order. The native
        // generator additionally requires syntactic identity with the registered prefix.
        // Adapt to that API using the signatures just bound above: reference binder names
        // need not equal the reader's binder names. Keep the generator's identity gate,
        // and still validate all independent source recursor types and rules below.
        let members: Vec<_> = block
            .members
            .iter()
            .map(|member| {
                let name = &member.declaration.name;
                let sort = ctx
                    .get_global(name)
                    .ok_or_else(|| {
                        invalid("bound nested source family is absent from the registered prefix")
                    })?
                    .clone();
                let constructors = ctx
                    .get_constructors(name)
                    .into_iter()
                    .map(|(constructor, ty)| (constructor.to_string(), ty.clone()))
                    .collect();
                Ok(crate::NestedDecl {
                    name: name.clone(),
                    sort,
                    constructors,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        nested(
            &mut scratch,
            &members,
            first.num_params,
            &first.declaration.level_params,
            &block.recursors,
            &first.all,
            first.num_nested,
        )
    } else {
        ordinary(&mut scratch, block)
    }
}

#[cfg(test)]
fn ordinary(
    ctx: &mut Context,
    block: &ReferenceInductiveBlock<Term>,
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
    let single = motives == 1;
    let mut common_sort: Option<Universe> = None;
    for member in &block.members {
        let mut arity = declared_arity(ctx, &member.declaration.ty)?;
        while let Term::Pi { body_type, .. } = arity {
            arity = *body_type;
        }
        let Term::Sort(level) = arity else {
            return Err(invalid("source family arity does not end in a sort"));
        };
        if common_sort
            .as_ref()
            .is_some_and(|common| !common.equiv(&level))
        {
            return Err(invalid("source mutual members inhabit different sorts"));
        }
        common_sort = Some(level);
    }
    let large = Universe::Type(0).is_subtype_of(common_sort.as_ref().expect("nonempty block"));
    let mut result = Vec::new();
    let mut minor_offset = nparams + motives;
    for member in &block.members {
        let name = format!("{}/s:\"rec\"", member.declaration.name);
        let source = block
            .recursors
            .iter()
            .find(|r| r.declaration.name == name)
            .ok_or_else(|| invalid("source block lacks canonical recursor name"))?;
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
            crate::derive_recursor_universe_poly(ctx, &member.declaration.name)
                .map_err(inductive_verdict)?;
        let substitution: HashMap<String, Universe> = if single {
            if levels.len() != derived_levels.len() {
                return Err(invalid("source recursor universe arity mismatch"));
            }
            derived_levels
                .iter()
                .cloned()
                .zip(levels.iter().cloned().map(Universe::Var))
                .collect()
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
            derived_levels
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    (
                        name.clone(),
                        if i < count {
                            if large {
                                Universe::Var(levels[0].clone())
                            } else {
                                Universe::Prop
                            }
                        } else {
                            Universe::Var(levels[offset + i - count].clone())
                        },
                    )
                })
                .collect()
        };
        let ty = crate::instantiate_universes(&ty, &substitution);
        let mut body = crate::instantiate_universes(&body, &substitution);
        // This metadata was granted by the kernel only after proving every projection's
        // constructor-field computation rule. Inline those checked bodies, never their names.
        if let Some(structure) = ctx.struct_info(&member.declaration.name) {
            body = structure_recursor_body(
                ctx,
                &ty,
                nparams,
                minor_offset,
                prefix + 1,
                &structure.projections,
            )?;
        }
        exact(ctx, &source.declaration.ty, &ty)?;
        checked_body(ctx, &name, &body, &ty)?;
        result.push(CanonicalRecursor {
            name: name.clone(),
            levels: levels.clone(),
            ty: ty.clone(),
            body: body.clone(),
        });
        if levels.is_empty() {
            ctx.declare(&name, ty, body).map_err(kernel)?;
        } else {
            ctx.declare_universe_poly(&name, levels.clone(), ty, body)
                .map_err(kernel)?;
        }
        minor_offset += member.constructors.len();
    }
    // Rule references see only freshly constructed siblings in the scratch environment.
    for (member, canonical) in block.members.iter().zip(&result) {
        let source = block
            .recursors
            .iter()
            .find(|r| r.declaration.name == canonical.name)
            .expect("canonical source recursor");
        if source.rules.len() != member.constructors.len() {
            return Err(invalid("source computation rule count mismatch"));
        }
        let offset = canonical
            .levels
            .len()
            .checked_sub(params.len())
            .ok_or_else(|| invalid("source rule universes"))?;
        let family_substitution: HashMap<_, _> = params
            .iter()
            .cloned()
            .zip(
                canonical.levels[offset..]
                    .iter()
                    .cloned()
                    .map(Universe::Var),
            )
            .collect();
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
            if !seen.insert(rule.constructor.clone()) || rule.num_fields != constructor.num_fields {
                return Err(invalid(
                    "source computation rule field count or uniqueness mismatch",
                ));
            }
            let value = if params.is_empty() {
                Term::Global(rule.constructor.clone())
            } else {
                Term::Const {
                    name: rule.constructor.clone(),
                    levels: params
                        .iter()
                        .map(|p| family_substitution[p].clone())
                        .collect(),
                }
            };
            let ctor_ty =
                crate::instantiate_universes(&constructor.declaration.ty, &family_substitution);
            let expected = ctor_rule_applied(
                ctx,
                &canonical.name,
                &canonical.levels,
                &canonical.ty,
                value,
                ctor_ty,
                nparams,
                prefix,
                nparams,
            )?;
            let actual_ty = lean_type(ctx, &rule.rhs)?;
            let expected_ty = infer_type(ctx, &expected).map_err(kernel)?;
            exact(ctx, &actual_ty, &expected_ty)?;
            exact(ctx, &rule.rhs, &expected)?;
        }
    }
    Ok(result)
}

#[cfg(test)]
fn nested(
    ctx: &mut Context,
    members: &[crate::NestedDecl],
    nparams: usize,
    family_levels: &[String],
    recs: &[ReferenceRecursor<Term>],
    all: &[String],
    nested_count: usize,
) -> Result<Vec<CanonicalRecursor>> {
    fn compare(ctx: &Context, actual: &Term, expected: &Term, stage: &str) -> Result<()> {
        exact(ctx, actual, expected).map_err(|error| error.located(stage))
    }
    let native = crate::inductive_compile::derive_nested_block_recursors(
        ctx,
        &crate::inductive_compile::NestedBlock {
            members,
            num_params: nparams,
            levels: family_levels,
        },
    )
    .map_err(inductive_verdict)?;
    let motives = native.len();
    if nested_count.checked_add(members.len()) != Some(motives) || recs.len() != motives {
        return Err(rejected(
            DiagnosticCode::InvalidProof,
            "native nested recursor inventory mismatch",
        ));
    }
    // Carriers and constructor groups are open exactly over the shared parameters; the
    // parameters are in scope for their sorts and telescopes.
    let parameters = native
        .first()
        .map(|rec| rec.parameters.clone())
        .unwrap_or_default();
    if parameters.len() != nparams {
        return Err(declined(
            DiagnosticCode::UnsupportedFeature,
            "native nested reconstruction lost the uniform parameter telescope",
        ));
    }
    let mut local = ctx.clone();
    for (parameter, domain) in &parameters {
        local = local.extend(parameter, domain.clone());
    }
    // The sort a carrier's family lives in, after its indices.
    let family_sort = |carrier: &Term| -> Result<Universe> {
        let mut ty = normal(&local, &infer_type(&local, carrier).map_err(kernel)?)?;
        while let Term::Pi { body_type, .. } = ty {
            ty = *body_type;
        }
        match ty {
            Term::Sort(level) => Ok(level),
            _ => Err(rejected(
                DiagnosticCode::InvalidProof,
                "a nested carrier is not a family of types",
            )),
        }
    };
    // Each group contains original constructors partially applied to the
    // fixed container arguments, with their original field telescope. Every carrier
    // lives in the block's own sort, as every member of Lean's auxiliary block does.
    let block_sort = family_sort(&native[0].carrier)?;
    let mut constructor_groups = Vec::new();
    let mut indices = Vec::new();
    let mut minors = 0usize;
    for rec in &native {
        if !family_sort(&rec.carrier)?.equiv(&block_sort) {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "native nested container outside the family's universe",
            ));
        }
        let mut head = &rec.carrier;
        let mut arguments = Vec::new();
        while let Term::App(f, a) = head {
            arguments.push((**a).clone());
            head = f;
        }
        arguments.reverse();
        let (parent, levels) = match head {
            Term::Global(name) => (name, Vec::new()),
            Term::Const { name, levels } => (name, levels.clone()),
            _ => {
                return Err(declined(
                    DiagnosticCode::UnsupportedFeature,
                    "native nested carrier has no inductive head",
                ))
            }
        };
        if ctx.inductive_num_params(parent) != arguments.len() {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "native nested container parameters are not fully fixed",
            ));
        }
        indices.push(ctx.inductive_arity(parent) - arguments.len());
        let mut constructors = Vec::new();
        for (name, _) in ctx.get_constructors(parent) {
            let mut value = match head {
                Term::Const { .. } => Term::Const {
                    name: name.into(),
                    levels: levels.clone(),
                },
                _ => Term::Global(name.into()),
            };
            let mut ty = infer_type(ctx, &value).map_err(kernel)?;
            for argument in &arguments {
                let Term::Pi {
                    param, body_type, ..
                } = ty
                else {
                    return Err(declined(
                        DiagnosticCode::UnsupportedFeature,
                        "native constructor parameter telescope unavailable",
                    ));
                };
                ty = crate::substitute(&body_type, &param, argument);
                value = apply(value, argument.clone());
            }
            let fields = resolved_telescope_len(&local, &ty)?;
            constructors.push((name.to_string(), value, ty, arguments.len(), fields));
        }
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
    let mut checked = Vec::new();
    let mut result = Vec::new();
    // All checked implementations enter scope before decoding any exported
    // computation rule, because the rules refer to sibling recursors.
    for (i, rec) in native.into_iter().enumerate() {
        // Each source member's own recursor, then the auxiliaries' as `rec_1, rec_2, …` of
        // the block's first member.
        let (owner, suffix) = if i < members.len() {
            (&members[i].name, "rec".to_string())
        } else {
            (&members[0].name, format!("rec_{}", i + 1 - members.len()))
        };
        let name = format!("{}/s:{}", owner, serde_json::to_string(&suffix).unwrap());
        let position = recs
            .iter()
            .position(|r| r.declaration.name == name)
            .ok_or_else(|| {
                rejected(
                    DiagnosticCode::InvalidProof,
                    "missing canonical native nested recursor",
                )
            })?;
        let exported = &recs[position];
        if exported.num_params != nparams
            || exported.num_indices != indices[i]
            || exported.num_motives != motives
            || exported.num_minors != minors
        {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "native nested recursor telescope counts mismatch",
            ));
        }
        if exported.all != all {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "native nested recursor block membership mismatch",
            ));
        }
        if exported.is_k {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "a nested recursor has an invalid K flag",
            ));
        }
        // Lean's auxiliary block is a mutual block: it eliminates into every sort through
        // one shared motive universe when its members are types, and only into `Prop`
        // when they may be propositions (no single-member exception applies).
        let large = Universe::Type(0).is_subtype_of(&block_sort);
        let levels = exported.declaration.level_params.clone();
        if levels.len() != usize::from(large) + family_levels.len() {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
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
        // The LOGOS generator supports independent motive universes; this checked
        // instantiation supplies Lean's single motive universe (or `Prop`) to all of them
        // and Lean's names to the family's own levels.
        let offset = usize::from(large);
        let substitution = rec
            .levels
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let lean = if i < derived_motives {
                    if large {
                        Universe::Var(levels[0].clone())
                    } else {
                        Universe::Prop
                    }
                } else {
                    Universe::Var(levels[offset + (i - derived_motives)].clone())
                };
                (p.clone(), lean)
            })
            .collect();
        let ty = crate::instantiate_universes(&rec.ty, &substitution);
        let body = crate::instantiate_universes(&rec.body, &substitution);
        let expected = exported.declaration.ty.clone();
        lean_type(ctx, &expected)?;
        compare(ctx, &expected, &ty, &format!("native recursor {name} type"))?;
        checked_body(ctx, &name, &body, &expected)?;
        result.push(CanonicalRecursor {
            name: name.clone(),
            levels: levels.clone(),
            ty: expected.clone(),
            body: body.clone(),
        });
        if levels.is_empty() {
            ctx.declare(&name, expected, body).map_err(kernel)?;
        } else {
            ctx.declare_universe_poly(&name, levels.clone(), expected, body)
                .map_err(kernel)?;
        }
        checked.push((name, levels, ty, position));
    }
    // The rule telescope binds the shared parameters first. Constructor groups are
    // instantiated at those binders, never at the canonical names they were built over.
    let rule_parameters: Vec<(String, Term)> = parameters
        .iter()
        .enumerate()
        .map(|(i, (parameter, _))| (parameter.clone(), Term::Var(format!("arena.rule.{i}"))))
        .collect();
    let offset = usize::from(Universe::Type(0).is_subtype_of(&block_sort));
    let at_rule = |term: &Term, levels: &[String]| {
        let universes = family_levels
            .iter()
            .cloned()
            .zip(levels[offset..].iter().cloned().map(Universe::Var))
            .collect();
        rule_parameters.iter().fold(
            crate::instantiate_universes(term, &universes),
            |term, (parameter, binder)| crate::substitute(&term, parameter, binder),
        )
    };
    for ((name, levels, ty, position), constructors) in checked.iter().zip(&constructor_groups) {
        let rules = &recs[*position].rules;
        if rules.len() != constructors.len() {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "native nested recursor computation rule inventory mismatch",
            ));
        }
        let mut seen = HashSet::new();
        for rule in rules {
            let constructor = rule.constructor.clone();
            let Some((_, value, ctor_ty, result_parameters, fields)) =
                constructors.iter().find(|(n, ..)| n == &constructor)
            else {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "native nested rule names a foreign constructor",
                ));
            };
            if !seen.insert(constructor) || rule.num_fields != *fields {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "native nested rule has duplicate constructor or wrong field count",
                ));
            }
            let expected = ctor_rule_applied(
                ctx,
                name,
                levels,
                ty,
                at_rule(value, levels),
                at_rule(ctor_ty, levels),
                0,
                prefix,
                *result_parameters,
            )?;
            let actual = rule.rhs.clone();
            let actual_ty = lean_type(ctx, &actual)?;
            let expected_ty = infer_type(ctx, &expected).map_err(kernel)?;
            compare(
                ctx,
                &actual_ty,
                &expected_ty,
                &format!("native recursor {name} rule type"),
            )?;
            compare(
                ctx,
                &actual,
                &expected,
                &format!("native recursor {name} rule value"),
            )?;
        }
    }
    Ok(result)
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/source_recursor/controls.rs"]
mod controls;

#[path = "source_recursor_bounded.rs"]
mod bounded;
pub(super) use bounded::{derive_bounded, telescope_len};
