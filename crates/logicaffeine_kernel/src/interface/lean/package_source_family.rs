//! Bind metadata that selects projection parameters and constructor field ordinals.
//! Header expression fidelity remains the package's independent exact-type check.
use super::super::super::{rejected, DiagnosticCode, Result};
use super::{generation, Context, RecursorBudget, ReferenceInductiveBlock};
fn invalid() -> super::super::super::Verdict {
    rejected(
        DiagnosticCode::InvalidProof,
        "source family metadata differs from the checked certificate context",
    )
}
fn same_names(b: &mut RecursorBudget, a: &[String], c: &[String]) -> Result<bool> {
    if a.len() != c.len() {
        return Ok(false);
    }
    for (a, c) in a.iter().zip(c) {
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
pub(super) fn bind_family_metadata(
    ctx: &Context,
    block: &ReferenceInductiveBlock<u64>,
    b: &mut RecursorBudget,
) -> Result<()> {
    let first = block.members.first().ok_or_else(invalid)?;
    b.inventory(block.members.len()).map_err(generation)?;
    b.inventory(block.constructors.len()).map_err(generation)?;
    let count = (block.members.len() as u64)
        .saturating_add(block.constructors.len() as u64)
        .saturating_add(1);
    let mut bytes = 0u64;
    for member in &block.members {
        b.inventory(member.all.len()).map_err(generation)?;
        b.inventory(member.constructors.len()).map_err(generation)?;
        b.inventory(member.declaration.level_params.len())
            .map_err(generation)?;
        bytes = bytes.saturating_add(member.declaration.name.len() as u64 + 1);
        for name in member
            .all
            .iter()
            .chain(&member.constructors)
            .chain(&member.declaration.level_params)
        {
            bytes = bytes.saturating_add(name.len() as u64 + 1);
        }
    }
    for ctor in &block.constructors {
        b.inventory(ctor.declaration.level_params.len())
            .map_err(generation)?;
        bytes = bytes
            .saturating_add(ctor.parent.len() as u64)
            .saturating_add(ctor.declaration.name.len() as u64 + 1);
        for name in &ctor.declaration.level_params {
            bytes = bytes.saturating_add(name.len() as u64 + 1);
        }
    }
    // Covers repeated raw metadata comparisons and constructor lookup scans below.
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
    let mut seen = std::collections::BTreeSet::new();
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
            || !ctx.is_inductive(&source.name)
            || ctx.inductive_num_params(&source.name) != member.num_params
            || ctx.inductive_num_indices(&source.name) != member.num_indices
            || member.is_recursive != ctx.block_is_recursive(&source.name)
        {
            return Err(invalid());
        }
        let levels = ctx
            .poly_declaration(&source.name)
            .map_or(&[][..], |(ls, _)| ls);
        if !same_names(b, levels, &source.level_params)? {
            return Err(invalid());
        }
        match ctx.mutual_block_of(&source.name) {
            Some(actual) => {
                b.inventory(actual.len()).map_err(generation)?;
                for name in actual {
                    b.spend(name.len() as u64 + 1).map_err(generation)?;
                }
                if !actual.iter().map(String::as_str).eq(all.iter().copied()) {
                    return Err(invalid());
                }
            }
            None => {
                if all.len() != 1 || all[0] != source.name {
                    return Err(invalid());
                }
            }
        }
        // is_reflexive and num_nested are exporter descriptions, not projection inputs.
        // num_nested's common-block consistency is checked above; neither invents kernel evidence.
        b.inventory(ctx.constructor_inventory_upper_bound(&source.name))
            .map_err(generation)?;
        let mut headers = ctx.constructor_headers(&source.name);
        for (ordinal, name) in member.constructors.iter().enumerate() {
            let (actual, ty) = headers.next().ok_or_else(invalid)?;
            b.spend(actual.len() as u64 + 1).map_err(generation)?;
            if actual != name || ctx.constructor_inductive(name) != Some(source.name.as_str()) {
                return Err(invalid());
            }
            let constructor = block
                .constructors
                .iter()
                .find(|c| c.declaration.name == *name)
                .ok_or_else(invalid)?;
            if !seen.insert(name.as_str())
                || constructor.parent != source.name
                || constructor.index != ordinal
                || constructor.num_params != member.num_params
                || constructor.declaration.is_unsafe
                || constructor.declaration.level_params != source.level_params
            {
                return Err(invalid());
            }
            let levels = ctx.poly_declaration(name).map_or(&[][..], |(ls, _)| ls);
            if !same_names(b, levels, &constructor.declaration.level_params)? {
                return Err(invalid());
            }
            // This is a checked header, copied/normalized with the existing shared bounded counter.
            let fields = super::super::super::source_recursor::telescope_len(ctx, b, ty)?;
            if constructor.num_params.checked_add(constructor.num_fields) != Some(fields) {
                return Err(invalid());
            }
        }
        if headers.next().is_some() {
            return Err(invalid());
        }
    }
    if seen.len() != block.constructors.len() {
        return Err(invalid());
    }
    Ok(())
}
