//! One source construction allowance for a complete build/replay binding operation.
use super::super::reference::{
    Reference, ReferenceBinding, ReferenceDeclaration, ReferenceInductiveBlock, ReferenceProjection,
};
use super::super::reference_projection_shared::SharedProjectionReader;
use super::super::{declined, DiagnosticCode, Result};
use crate::certificate::EnvironmentLimits;
use crate::recursor::bounded::{RecursorBudget, RecursorGenerationError};
use crate::{Context, Term};

pub(super) struct SourceWork {
    pub(super) budget: RecursorBudget,
}
fn exhausted() -> super::super::Verdict {
    declined(
        DiagnosticCode::ResourceLimit,
        "package source construction allowance",
    )
}
fn generation(error: RecursorGenerationError) -> super::super::Verdict {
    match error {
        RecursorGenerationError::ResourceLimit => exhausted(),
        RecursorGenerationError::Kernel(error) => super::super::kernel(error),
    }
}
impl SourceWork {
    /// Policy over existing public limits; no file-format or public limit fields change.
    /// Work is abstract node/string operations; allocation is cumulative 64-byte credits.
    /// Checked arithmetic refuses overflow instead of wrapping or silently becoming unlimited.
    pub(super) fn new(limits: &EnvironmentLimits) -> Result<Self> {
        if limits.max_depth == 0 || limits.max_nodes == 0 || limits.max_tree_nodes == 0 {
            return Err(exhausted());
        }
        let nodes = u64::try_from(limits.max_nodes).map_err(|_| exhausted())?;
        let entries = u64::try_from(limits.max_entries).map_err(|_| exhausted())?;
        let work = limits
            .max_tree_nodes
            .checked_mul(256)
            .and_then(|v| nodes.checked_mul(64).and_then(|n| v.checked_add(n)))
            .and_then(|v| entries.checked_mul(32).and_then(|n| v.checked_add(n)))
            .ok_or_else(exhausted)?;
        let allocation = limits
            .max_tree_nodes
            .checked_mul(8)
            .and_then(|v| nodes.checked_mul(16).and_then(|n| v.checked_add(n)))
            .and_then(|v| entries.checked_mul(8).and_then(|n| v.checked_add(n)))
            .ok_or_else(exhausted)?;
        Ok(Self {
            budget: RecursorBudget {
                max_depth: limits.max_depth.min(crate::type_checker::MAX_TERM_DEPTH),
                max_tree_nodes: limits.max_tree_nodes,
                max_store_nodes: limits.max_nodes.min(u32::MAX as usize),
                work,
                allocation,
                materialized_nodes: limits.max_tree_nodes,
            },
        })
    }
    fn reserve_source(&mut self, nodes: u64) -> Result<()> {
        self.budget
            .spend(nodes.checked_mul(8).ok_or_else(exhausted)?)
            .map_err(generation)?;
        self.budget
            .reserve(nodes.checked_mul(128).ok_or_else(exhausted)?)
            .map_err(generation)?;
        self.budget.materialized_nodes = self
            .budget
            .materialized_nodes
            .checked_sub(nodes)
            .ok_or_else(exhausted)?;
        Ok(())
    }
    pub(super) fn declaration(
        &mut self,
        context: &Context,
        reference: &Reference,
        name: &str,
    ) -> Result<ReferenceDeclaration> {
        let metrics = reference.source_root_metrics(
            name,
            false,
            self.budget.max_depth,
            self.budget.materialized_nodes,
        )?;
        self.reserve_source(metrics.tree_nodes)?;
        let mut provider = FamilyProjectionReader {
            context,
            reference,
            work: self,
        };
        reference.declaration_kernel_with(name, &mut provider)
    }
    pub(super) fn value(
        &mut self,
        context: &Context,
        reference: &Reference,
        name: &str,
    ) -> Result<Term> {
        let metrics = reference.source_root_metrics(
            name,
            true,
            self.budget.max_depth,
            self.budget.materialized_nodes,
        )?;
        self.reserve_source(metrics.tree_nodes)?;
        let mut provider = FamilyProjectionReader {
            context,
            reference,
            work: self,
        };
        reference.value_kernel_with(name, &mut provider)
    }
    pub(super) fn block(
        &mut self,
        context: &Context,
        reference: &Reference,
        name: &str,
    ) -> Result<ReferenceInductiveBlock<Term>> {
        let name = self.cache_name(name)?;
        let metrics = reference.preflight_recursor_blocks_kernel(
            std::slice::from_ref(&name),
            self.budget.max_depth,
            self.budget.materialized_nodes,
        )?;
        self.reserve_source(metrics.tree_nodes)?;
        let mut provider = FamilyProjectionReader {
            context,
            reference,
            work: self,
        };
        reference.recursor_block_kernel_with(&name, &mut provider)
    }
    pub(super) fn family(
        &mut self,
        context: &Context,
        reference: &Reference,
        name: &str,
    ) -> Result<()> {
        let shape = reference.family_shape(name)?;
        bind_family_metadata(context, &shape, &mut self.budget)
    }
    pub(super) fn levels(&mut self, levels: &[String], terms: &[&Term]) -> Result<bool> {
        self.budget.inventory(levels.len()).map_err(generation)?;
        let mut allowed = std::collections::BTreeSet::new();
        for level in levels {
            self.budget
                .spend((level.len() as u64 + 1).saturating_mul(levels.len() as u64 + 1))
                .map_err(generation)?;
            allowed.insert(level.as_str());
        }
        if allowed.len() != levels.len() {
            return Ok(false);
        }
        for term in terms {
            if !self.borrowed_levels(term, &allowed)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    /// Universe scope is a read-only property. Walk borrowed syntax directly,
    /// retaining the shared work/allocation budget and per-root depth/size caps.
    /// Sequence continuations keep scratch space proportional to nesting depth.
    fn borrowed_levels(
        &mut self,
        root: &Term,
        allowed: &std::collections::BTreeSet<&str>,
    ) -> Result<bool> {
        use crate::Universe;
        enum Frame<'a> {
            Term(&'a Term, usize),
            Level(&'a Universe, usize),
            Terms(&'a [Term], usize),
            Levels(&'a [Universe]),
            Definitions(&'a [(String, Term)], usize),
        }
        let mut pending = Vec::new();
        macro_rules! push {
            ($frame:expr) => {{
                self.budget.inventory(1).map_err(generation)?;
                pending.push($frame);
            }};
        }
        push!(Frame::Term(root, 1));
        let mut nodes = 0u64;
        let mut valid = true;
        while let Some(frame) = pending.pop() {
            self.budget.spend(1).map_err(generation)?;
            if let Frame::Term(_, depth) | Frame::Level(_, depth) = &frame {
                nodes = nodes.checked_add(1).ok_or_else(exhausted)?;
                if *depth > self.budget.max_depth || nodes > self.budget.max_tree_nodes {
                    return Err(exhausted());
                }
            }
            match frame {
                Frame::Term(term, depth) => match term {
                    Term::Sort(level) => push!(Frame::Level(level, 1)),
                    Term::Const { levels, .. } => push!(Frame::Levels(levels)),
                    Term::Pi {
                        param_type,
                        body_type,
                        ..
                    } => {
                        push!(Frame::Term(body_type, depth + 1));
                        push!(Frame::Term(param_type, depth + 1));
                    }
                    Term::Lambda {
                        param_type, body, ..
                    } => {
                        push!(Frame::Term(body, depth + 1));
                        push!(Frame::Term(param_type, depth + 1));
                    }
                    Term::App(function, argument) => {
                        push!(Frame::Term(argument, depth + 1));
                        push!(Frame::Term(function, depth + 1));
                    }
                    Term::Let {
                        ty, value, body, ..
                    } => {
                        push!(Frame::Term(body, depth + 1));
                        push!(Frame::Term(value, depth + 1));
                        push!(Frame::Term(ty, depth + 1));
                    }
                    Term::Match {
                        discriminant,
                        motive,
                        cases,
                    } => {
                        push!(Frame::Terms(cases, depth + 1));
                        push!(Frame::Term(motive, depth + 1));
                        push!(Frame::Term(discriminant, depth + 1));
                    }
                    Term::Fix { body, .. } => push!(Frame::Term(body, depth + 1)),
                    Term::MutualFix { defs, .. } => push!(Frame::Definitions(defs, depth + 1)),
                    Term::Annotated { term, .. } => push!(Frame::Term(term, depth + 1)),
                    Term::Var(_) | Term::Global(_) | Term::Lit(_) | Term::Hole => {}
                },
                Frame::Level(level, depth) => match level {
                    Universe::Var(name) => {
                        self.budget
                            .spend((name.len() as u64 + 1).saturating_mul(allowed.len() as u64 + 1))
                            .map_err(generation)?;
                        valid &= allowed.contains(name.as_str());
                    }
                    Universe::Succ(child) => push!(Frame::Level(child, depth + 1)),
                    Universe::Max(left, right) | Universe::IMax(left, right) => {
                        push!(Frame::Level(right, depth + 1));
                        push!(Frame::Level(left, depth + 1));
                    }
                    Universe::SProp | Universe::Prop | Universe::Type(_) => {}
                },
                Frame::Terms(terms, depth) => {
                    if let Some((first, rest)) = terms.split_first() {
                        if !rest.is_empty() {
                            push!(Frame::Terms(rest, depth));
                        }
                        push!(Frame::Term(first, depth));
                    }
                }
                Frame::Levels(levels) => {
                    if let Some((first, rest)) = levels.split_first() {
                        if !rest.is_empty() {
                            push!(Frame::Levels(rest));
                        }
                        push!(Frame::Level(first, 1));
                    }
                }
                Frame::Definitions(defs, depth) => {
                    if let Some(((_, first), rest)) = defs.split_first() {
                        if !rest.is_empty() {
                            push!(Frame::Definitions(rest, depth));
                        }
                        push!(Frame::Term(first, depth));
                    }
                }
            }
        }
        Ok(valid)
    }

    pub(super) fn lookup_name(&mut self, name: &str, entries: usize) -> Result<()> {
        // A B-tree performs at most a small linear node scan at each logarithmic level.
        let levels = (usize::BITS - entries.saturating_add(1).leading_zeros()) as u64 + 1;
        self.budget
            .spend(
                (name.len() as u64 + 1)
                    .saturating_mul(levels)
                    .saturating_mul(16),
            )
            .map_err(generation)
    }
    pub(super) fn cache_name(&mut self, name: &str) -> Result<String> {
        self.budget.inventory(1).map_err(generation)?;
        self.budget
            .spend(name.len() as u64 + 1)
            .map_err(generation)?;
        self.budget
            .reserve(name.len() as u64 + 64)
            .map_err(generation)?;
        Ok(name.to_owned())
    }
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/package/source_work/controls.rs"]
mod controls;

/// Every projection first binds source-owned family metadata in exactly the checked context
/// used by the typed adapter. This guard holds no cache that could cross certificate prefixes.
struct FamilyProjectionReader<'a, 'b> {
    context: &'a Context,
    reference: &'a Reference,
    work: &'b mut SourceWork,
}
impl ReferenceProjection for FamilyProjectionReader<'_, '_> {
    fn generated_depth_limit(&self) -> Option<usize> {
        Some(
            self.work
                .budget
                .max_depth
                .min(crate::type_checker::MAX_TERM_DEPTH),
        )
    }
    fn project(
        &mut self,
        parent: &str,
        index: usize,
        discriminant: Term,
        scope: &[ReferenceBinding<'_>],
        remaining: &mut usize,
    ) -> Result<Term> {
        self.work.family(self.context, self.reference, parent)?;
        let mut provider = SharedProjectionReader {
            context: self.context,
            max_depth: self.work.budget.max_depth,
            budget: &mut self.work.budget,
        };
        provider.project(parent, index, discriminant, scope, remaining)
    }
}

#[path = "package_source_family.rs"]
mod family;
use family::bind_family_metadata;

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/package/source_work/family_controls.rs"]
mod family_controls;

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/package/source_work/borrowed_controls.rs"]
mod borrowed_controls;
