//! Admission for universe substitution before the existing syntax operation allocates.
use super::{RecursorBudget, RecursorGenerationError};
use crate::{Literal, Term, Universe};
use std::collections::HashMap;

type Result<T> = std::result::Result<T, RecursorGenerationError>;
fn limit() -> RecursorGenerationError {
    RecursorGenerationError::ResourceLimit
}
fn spend(budget: &mut RecursorBudget, work: u64) -> Result<()> {
    budget.work = budget.work.checked_sub(work).ok_or_else(limit)?;
    Ok(())
}
fn reserve(budget: &mut RecursorBudget, bytes: u64) -> Result<()> {
    let credits = bytes.checked_add(63).ok_or_else(limit)? / 64;
    budget.allocation = budget.allocation.checked_sub(credits).ok_or_else(limit)?;
    Ok(())
}

enum Item<'a> {
    Term(&'a Term, usize),
    // A replacement is copied verbatim: variables inside it are not substituted again.
    Level(&'a Universe, usize, bool),
}
fn push<'a>(
    pending: &mut Vec<Item<'a>>,
    item: Item<'a>,
    budget: &mut RecursorBudget,
) -> Result<()> {
    if pending.len() >= budget.max_store_nodes {
        return Err(limit());
    }
    spend(budget, 1)?;
    reserve(budget, 128)?;
    pending.push(item);
    Ok(())
}

/// Uses the existing substitution semantics, including universe constructor simplification.
/// Output and all intermediate universe copies are admitted before invoking that operation.
/// The enclosing caller must reserve stack for both admitted term and universe depth.
pub(crate) fn instantiate_universes_bounded(
    term: &Term,
    substitution: &HashMap<String, Universe>,
    budget: &mut RecursorBudget,
) -> Result<Term> {
    let mut pending = Vec::new();
    push(&mut pending, Item::Term(term, 1), budget)?;
    let mut terms = 0u64;
    let mut levels = 0u64;
    let mut payload = 0u64;
    let mut level_payload = 0u64;
    let mut lookup_bytes = 0u64;
    while let Some(item) = pending.pop() {
        spend(budget, 1)?;
        match item {
            Item::Term(term, depth) => {
                if depth > budget.max_depth {
                    return Err(limit());
                }
                terms = terms.checked_add(1).ok_or_else(limit)?;
                if terms > budget.max_tree_nodes {
                    return Err(limit());
                }
                match term {
                    Term::Var(name) | Term::Global(name) => {
                        payload = payload.checked_add(name.len() as u64).ok_or_else(limit)?;
                    }
                    Term::Sort(level) => push(&mut pending, Item::Level(level, 1, true), budget)?,
                    Term::Const { name, levels } => {
                        payload = payload.checked_add(name.len() as u64).ok_or_else(limit)?;
                        for level in levels {
                            push(&mut pending, Item::Level(level, 1, true), budget)?;
                        }
                    }
                    Term::Pi {
                        param,
                        param_type,
                        body_type,
                    } => {
                        payload = payload.checked_add(param.len() as u64).ok_or_else(limit)?;
                        push(&mut pending, Item::Term(param_type, depth + 1), budget)?;
                        push(&mut pending, Item::Term(body_type, depth + 1), budget)?;
                    }
                    Term::Lambda {
                        param,
                        param_type,
                        body,
                    } => {
                        payload = payload.checked_add(param.len() as u64).ok_or_else(limit)?;
                        push(&mut pending, Item::Term(param_type, depth + 1), budget)?;
                        push(&mut pending, Item::Term(body, depth + 1), budget)?;
                    }
                    Term::App(function, argument) => {
                        push(&mut pending, Item::Term(function, depth + 1), budget)?;
                        push(&mut pending, Item::Term(argument, depth + 1), budget)?;
                    }
                    Term::Let {
                        name,
                        ty,
                        value,
                        body,
                    } => {
                        payload = payload.checked_add(name.len() as u64).ok_or_else(limit)?;
                        push(&mut pending, Item::Term(ty, depth + 1), budget)?;
                        push(&mut pending, Item::Term(value, depth + 1), budget)?;
                        push(&mut pending, Item::Term(body, depth + 1), budget)?;
                    }
                    Term::Match {
                        discriminant,
                        motive,
                        cases,
                    } => {
                        push(&mut pending, Item::Term(discriminant, depth + 1), budget)?;
                        push(&mut pending, Item::Term(motive, depth + 1), budget)?;
                        for case in cases {
                            push(&mut pending, Item::Term(case, depth + 1), budget)?;
                        }
                    }
                    Term::Fix { name, body } => {
                        payload = payload.checked_add(name.len() as u64).ok_or_else(limit)?;
                        push(&mut pending, Item::Term(body, depth + 1), budget)?;
                    }
                    Term::MutualFix { defs, .. } => {
                        for (name, body) in defs {
                            payload = payload.checked_add(name.len() as u64).ok_or_else(limit)?;
                            push(&mut pending, Item::Term(body, depth + 1), budget)?;
                        }
                    }
                    Term::Lit(literal) => {
                        let bytes = match literal {
                            Literal::Text(text) => text.len(),
                            Literal::BigInt(value) | Literal::Nat(value) => {
                                value.clone_payload_bytes()
                            }
                            _ => 0,
                        };
                        payload = payload.checked_add(bytes as u64).ok_or_else(limit)?;
                    }
                    Term::Hole => {}
                    Term::Annotated { term, .. } => {
                        push(&mut pending, Item::Term(term, depth + 1), budget)?;
                    }
                }
            }
            Item::Level(level, depth, substitute) => {
                if depth > budget.max_depth {
                    return Err(limit());
                }
                if let Universe::Var(name) = level {
                    spend(budget, name.len() as u64)?;
                    if substitute {
                        lookup_bytes = lookup_bytes
                            .checked_add(name.len() as u64)
                            .ok_or_else(limit)?;
                        if let Some(replacement) = substitution.get(name) {
                            push(&mut pending, Item::Level(replacement, depth, false), budget)?;
                            continue;
                        }
                    }
                    level_payload = level_payload
                        .checked_add(name.len() as u64)
                        .ok_or_else(limit)?;
                }
                levels = levels.checked_add(1).ok_or_else(limit)?;
                if terms
                    .checked_add(levels)
                    .is_none_or(|n| n > budget.max_tree_nodes)
                {
                    return Err(limit());
                }
                match level {
                    Universe::Succ(child) => push(
                        &mut pending,
                        Item::Level(child, depth + 1, substitute),
                        budget,
                    )?,
                    Universe::Max(left, right) | Universe::IMax(left, right) => {
                        push(
                            &mut pending,
                            Item::Level(left, depth + 1, substitute),
                            budget,
                        )?;
                        push(
                            &mut pending,
                            Item::Level(right, depth + 1, substitute),
                            budget,
                        )?;
                    }
                    _ => {}
                }
            }
        }
    }
    let expanded = terms.checked_add(levels).ok_or_else(limit)?;
    if expanded > budget.max_tree_nodes {
        return Err(limit());
    }
    // succ/max/imax can copy their complete arguments at every source level node.
    // The square bounds all such intermediate copies, even before simplification.
    let level_work = levels
        .checked_mul(levels.checked_add(level_payload).ok_or_else(limit)?)
        .ok_or_else(limit)?;
    let work = terms
        .checked_add(payload)
        .and_then(|n| n.checked_add(level_work))
        .and_then(|n| n.checked_add(lookup_bytes))
        .ok_or_else(limit)?;
    let bytes = terms
        .checked_mul(256)
        .and_then(|n| n.checked_add(payload.checked_mul(4)?))
        .and_then(|n| n.checked_add(level_work.checked_mul(128)?))
        .ok_or_else(limit)?;
    spend(budget, work)?;
    reserve(budget, bytes)?;
    budget.materialized_nodes = budget
        .materialized_nodes
        .checked_sub(expanded)
        .ok_or_else(limit)?;
    Ok(crate::instantiate_universes(term, substitution))
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recursor/bounded/universes/controls.rs"]
mod controls;
