//! Test-only composition of bounded elimination, ordinary generation, and native restoration.
use super::bounded_elimination::eliminate_nested_bounded;
use super::bounded_native_restoration::restore_native_term_bounded;
use super::{NativeRecursor, NestedBlock};
use crate::recursor::bounded::{
    derive_recursor_universe_poly_bounded, Construction, GenerationResult, RecursorBudget,
    RecursorGenerationError,
};
use crate::term_store::Node;
use crate::{Context, KernelError, Term, Universe};
use std::collections::HashMap;

fn limit() -> RecursorGenerationError {
    RecursorGenerationError::ResourceLimit
}
fn invalid(message: &str) -> RecursorGenerationError {
    KernelError::CertificationError(message.into()).into()
}
fn copy_name(budget: &mut RecursorBudget, name: &str) -> GenerationResult<String> {
    budget.spend(name.len() as u64 + 1)?;
    budget.reserve((name.len() as u64).saturating_add(64))?;
    Ok(name.to_owned())
}
fn equal_name(budget: &mut RecursorBudget, a: &str, b: &str) -> GenerationResult<bool> {
    budget.spend(
        (a.len() as u64)
            .saturating_add(b.len() as u64)
            .saturating_add(1),
    )?;
    Ok(a == b)
}

/// Preserve the ordinary wrapper's exact (not alpha-equivalent) source-signature requirement.
/// Shared-store identities compare admitted trees without recursively cloning constructor lists.
fn registered_exactly(
    ctx: &Context,
    block: &NestedBlock<'_>,
    budget: &mut RecursorBudget,
) -> GenerationResult<()> {
    let mut d = Construction::new(budget)?;
    d.budget.inventory(block.members.len())?;
    for decl in block.members {
        d.budget.spend(decl.name.len() as u64 + 1)?;
        let registered = ctx.get_global(&decl.name).ok_or_else(|| {
            invalid("native nested recursor requires the exact registered source declaration")
        })?;
        let expected = d.intern(&decl.sort)?;
        let actual = d.intern(registered)?;
        if expected != actual || ctx.inductive_num_params(&decl.name) != block.num_params {
            return Err(invalid(
                "native nested recursor requires the exact registered source declaration",
            ));
        }
        // Borrowed iterator is the same ordered inventory used by get_constructors.
        d.budget
            .inventory(ctx.constructor_inventory_upper_bound(&decl.name))?;
        let mut actual = ctx.constructor_headers(&decl.name);
        d.budget.inventory(decl.constructors.len())?;
        for (name, ty) in &decl.constructors {
            let Some((registered_name, registered_ty)) = actual.next() else {
                return Err(invalid(
                    "native nested recursor requires the exact registered constructor inventory",
                ));
            };
            if !equal_name(d.budget, name, registered_name)? {
                return Err(invalid(
                    "native nested recursor requires the exact registered constructor inventory",
                ));
            }
            let expected = d.intern(ty)?;
            let actual = d.intern(registered_ty)?;
            if expected != actual {
                return Err(invalid(
                    "native nested recursor requires the exact registered constructor inventory",
                ));
            }
        }
        if actual.next().is_some() {
            return Err(invalid(
                "native nested recursor requires the exact registered constructor inventory",
            ));
        }
    }
    Ok(())
}

fn clone_context(ctx: &Context, budget: &mut RecursorBudget) -> GenerationResult<Context> {
    // Context's local-name map is copied; binding payloads and the other large registries are Arc.
    // This covers the orchestration clone itself. Later checked registration owns its COW/checks.
    budget.reserve(std::mem::size_of::<Context>() as u64 + 256)?;
    budget.inventory(ctx.local_binding_capacity())?;
    for name in ctx.local_binding_names() {
        budget.inventory(1)?;
        budget.spend(name.len() as u64 + 1)?;
        budget.reserve(name.len() as u64 + 128)?;
    }
    Ok(ctx.clone())
}

/// `expected_recursors` is the independently decoded source inventory, source members included.
/// One budget spans every phase and every output sibling; no phase resets any allowance.
/// Existing ordinary APIs and canonical caller rule checks remain unchanged.
pub(crate) fn derive_nested_block_recursors_bounded(
    ctx: &Context,
    block: &NestedBlock<'_>,
    expected_recursors: usize,
    budget: &mut RecursorBudget,
) -> GenerationResult<Vec<NativeRecursor>> {
    if budget.max_depth == 0 || budget.max_depth > crate::type_checker::MAX_TERM_DEPTH {
        return Err(limit());
    }
    crate::term_store::grow_stack_for_depth(budget.max_depth, || {
        derive(ctx, block, expected_recursors, budget)
    })
}
fn derive(
    ctx: &Context,
    block: &NestedBlock<'_>,
    expected_recursors: usize,
    budget: &mut RecursorBudget,
) -> GenerationResult<Vec<NativeRecursor>> {
    registered_exactly(ctx, block, budget)?;
    let elimination = eliminate_nested_bounded(ctx, block, expected_recursors, budget)?;
    let elimination = super::native_specialization::prepare(
        ctx,
        elimination,
        block.levels,
        &mut SpecializationBudget(budget),
    )?;
    budget.inventory(elimination.parameters.len())?;
    let mut names = Vec::with_capacity(elimination.parameters.len());
    for (name, _) in &elimination.parameters {
        names.push(copy_name(budget, name)?);
    }
    // Move owned image trees into the map. No restoration image is recursively cloned here.
    budget.inventory(elimination.restoration.len())?;
    let mut restoration = HashMap::with_capacity(elimination.restoration.len());
    for (name, image) in elimination.restoration {
        budget.spend(name.len() as u64 + 1)?;
        restoration.insert(name, image);
    }
    // Re-admit emitted signatures at the checking boundary; do not use supplied shallow
    // recursor types as a proxy for the size of these specialized declarations.
    {
        let mut d = Construction::new(budget)?;
        for member in &elimination.block {
            d.budget.spend(member.name.len() as u64 + 1)?;
            d.intern(&member.sort)?;
            d.budget.inventory(member.constructors.len())?;
            for (name, ty) in &member.constructors {
                d.budget.spend(name.len() as u64 + 1)?;
                d.intern(ty)?;
            }
        }
    }
    let mut specialized = clone_context(ctx, budget)?;
    if block.levels.is_empty() {
        specialized.add_mutual_inductives(&elimination.block)?;
    } else {
        specialized.add_universe_poly_inductives(block.levels, &elimination.block)?;
    }
    budget.inventory(block.levels.len())?;
    let mut instance = Vec::with_capacity(block.levels.len());
    for level in block.levels {
        instance.push(Universe::Var(copy_name(budget, level)?));
    }
    budget.inventory(elimination.block.len())?;
    let mut result = Vec::with_capacity(elimination.block.len());
    for member in &elimination.block {
        let (derived, ty, body) =
            derive_recursor_universe_poly_bounded(&specialized, &member.name, budget)?;
        let motives = derived
            .len()
            .checked_sub(block.levels.len())
            .ok_or_else(|| {
                invalid(
                    "native nested recursor universe telescope does not end in the family's levels",
                )
            })?;
        for (actual, expected) in derived[motives..].iter().zip(block.levels) {
            if !equal_name(budget, actual, expected)? {
                return Err(invalid(
                    "native nested recursor universe telescope does not end in the family's levels",
                ));
            }
        }
        // The verified suffix already equals block.levels: retain/move it instead of cloning.
        let levels = derived;
        let ty = restore_native_term_bounded(&ty, &restoration, &names, &instance, budget)?;
        let body = restore_native_term_bounded(&body, &restoration, &names, &instance, budget)?;
        // Preserve the original checked restoration boundary. Canonical callers continue their
        // independent recursor/rule checking; this function neither replaces nor weakens it.
        let actual = crate::infer_type(ctx, &body)?;
        {
            let mut d = Construction::new(budget)?;
            d.intern(&actual)?;
            d.intern(&ty)?;
        }
        if !crate::defeq_for_test(ctx, &actual, &ty) {
            return Err(invalid(
                "restored native nested recursor type differs from its checked implementation",
            ));
        }
        let mut d = Construction::new(budget)?;
        d.budget.spend(member.name.len() as u64 + 1)?;
        let carrier = if let Some(image) = restoration.get(&member.name) {
            d.intern(image)?
        } else {
            let symbol = d.symbol(&member.name)?;
            let mut carrier = if block.levels.is_empty() {
                d.add(Node::Global(symbol))?
            } else {
                d.budget.inventory(instance.len())?;
                let mut levels = Vec::with_capacity(instance.len());
                for level in &instance {
                    levels.push(d.level(level)?);
                }
                d.add(Node::Const {
                    name: symbol,
                    levels: levels.into_boxed_slice(),
                })?
            };
            for name in &names {
                let param = d.var(name)?;
                carrier = d.app(carrier, param)?;
            }
            carrier
        };
        let carrier = d.extract(carrier)?;
        d.budget.inventory(elimination.parameters.len())?;
        let mut parameters = Vec::with_capacity(elimination.parameters.len());
        for (name, ty) in &elimination.parameters {
            let name = copy_name(d.budget, name)?;
            let ty = d.intern(ty)?;
            parameters.push((name, d.extract(ty)?));
        }
        result.push(NativeRecursor {
            carrier,
            parameters,
            levels,
            ty,
            body,
        });
    }
    Ok(result)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/inductive_compile/native_recursors_bounded/controls.rs"]
mod controls;

/// The ordinary and bounded wrappers share the exact same namespace/restoration transform.
struct SpecializationBudget<'a>(&'a mut RecursorBudget);
impl super::native_specialization::Meter for SpecializationBudget<'_> {
    type Error = RecursorGenerationError;
    fn work(&mut self, n: u64) -> GenerationResult<()> {
        self.0.spend(n)
    }
    fn reserve(&mut self, n: u64) -> GenerationResult<()> {
        self.0.reserve(n)
    }
    fn inventory(&mut self, n: usize) -> GenerationResult<()> {
        self.0.inventory(n)
    }
    fn admit(&mut self, term: &Term) -> GenerationResult<()> {
        let mut d = Construction::new(self.0)?;
        d.intern(term)?;
        Ok(())
    }
    fn image(
        &mut self,
        name: &str,
        parameters: &[(String, Term)],
        levels: &[String],
    ) -> GenerationResult<Term> {
        let mut d = Construction::new(self.0)?;
        let symbol = d.symbol(name)?;
        let mut out = if levels.is_empty() {
            d.add(Node::Global(symbol))?
        } else {
            d.budget.inventory(levels.len())?;
            let mut instance = Vec::new();
            for level in levels {
                let name = copy_name(d.budget, level)?;
                instance.push(d.level(&Universe::Var(name))?);
            }
            d.add(Node::Const {
                name: symbol,
                levels: instance.into_boxed_slice(),
            })?
        };
        for (name, _) in parameters {
            let variable = d.var(name)?;
            out = d.app(out, variable)?;
        }
        d.extract(out)
    }
    fn invalid(&self, message: &str) -> Self::Error {
        invalid(message)
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/inductive_compile/native_recursors_bounded/specialization_budget_controls.rs"]
mod specialization_budget_controls;
