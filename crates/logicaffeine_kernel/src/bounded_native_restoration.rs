//! Test-only bounded sibling of restore_native_term. No nested-elimination caller is switched.
use crate::recursor::bounded::{
    Construction, GenerationResult, RecursorBudget, RecursorGenerationError,
};
use crate::term_store::{LevelId, LevelView, Node, TermId, TermStore};
use crate::{KernelError, Term, Universe};
use std::collections::HashMap;

fn limit() -> RecursorGenerationError {
    RecursorGenerationError::ResourceLimit
}
fn invalid(why: &str) -> RecursorGenerationError {
    RecursorGenerationError::Kernel(KernelError::CertificationError(why.into()))
}
fn inventory(d: &mut Construction<'_>, current: usize, extra: usize) -> GenerationResult<()> {
    if current
        .checked_add(extra)
        .is_none_or(|n| n > d.budget.max_store_nodes)
    {
        return Err(limit());
    }
    d.budget.inventory(extra)
}

/// Structural universe comparison, matching the ordinary restoration's `levels != instance`.
/// No normalization, recursive comparison, owned universe clone, or new store level is needed.
fn same_levels(
    store: &TermStore,
    budget: &mut RecursorBudget,
    actual: &[LevelId],
    expected: &[Universe],
) -> GenerationResult<bool> {
    if actual.len() != expected.len() {
        return Ok(false);
    }
    budget.inventory(actual.len())?;
    let mut pending: Vec<_> = actual
        .iter()
        .copied()
        .zip(expected)
        .map(|(id, u)| (id, u, 1usize))
        .collect();
    while let Some((id, expected, depth)) = pending.pop() {
        budget.spend(1)?;
        if depth > budget.max_depth {
            return Err(limit());
        }
        let push_count = match expected {
            Universe::Succ(_) => 1,
            Universe::Max(..) | Universe::IMax(..) => 2,
            _ => 0,
        };
        if pending
            .len()
            .checked_add(push_count)
            .is_none_or(|n| n > budget.max_store_nodes)
        {
            return Err(limit());
        }
        budget.inventory(push_count)?;
        match (store.level_view(id), expected) {
            (LevelView::Prop, Universe::Prop) | (LevelView::SProp, Universe::SProp) => {}
            (LevelView::Type(a), Universe::Type(b)) if a == *b => {}
            (LevelView::Var(a), Universe::Var(b)) => {
                budget.spend((a.len() as u64).saturating_add(b.len() as u64))?;
                if a != b {
                    return Ok(false);
                }
            }
            (LevelView::Succ(a), Universe::Succ(b)) => pending.push((a, b, depth + 1)),
            (LevelView::Max(a, b), Universe::Max(c, d))
            | (LevelView::IMax(a, b), Universe::IMax(c, d)) => {
                pending.push((a, c, depth + 1));
                pending.push((b, d, depth + 1));
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

/// Budget state is caller-owned and remains shared across sibling restorations. The existing
/// unbounded API is unchanged. A caller must retain an adequate stack for using/dropping outputs.
pub(crate) fn restore_native_term_bounded(
    term: &Term,
    restoration: &HashMap<String, Term>,
    params: &[String],
    instance: &[Universe],
    budget: &mut RecursorBudget,
) -> GenerationResult<Term> {
    if budget.max_depth == 0 || budget.max_depth > crate::type_checker::MAX_TERM_DEPTH {
        return Err(limit());
    }
    crate::term_store::grow_stack_for_depth(budget.max_depth, || {
        restore(term, restoration, params, instance, budget)
    })
}
fn restore(
    term: &Term,
    restoration: &HashMap<String, Term>,
    params: &[String],
    instance: &[Universe],
    budget: &mut RecursorBudget,
) -> GenerationResult<Term> {
    enum Task {
        Enter(TermId),
        Build(TermId, usize),
        Image(TermId, TermId, usize),
    }
    let mut d = Construction::new(budget)?;
    let root = d.intern(term)?;
    inventory(&mut d, 0, params.len())?;
    let mut names = Vec::with_capacity(params.len());
    for name in params {
        names.push(d.symbol(name)?);
    }
    let mut images = HashMap::new();
    let mut cache = HashMap::new();
    inventory(&mut d, 0, 1)?;
    let mut pending = vec![Task::Enter(root)];
    let mut out = Vec::new();
    while let Some(task) = pending.pop() {
        d.budget.spend(1)?;
        inventory(&mut d, out.len(), 1)?;
        match task {
            Task::Enter(id) => {
                if let Some(&result) = cache.get(&id) {
                    out.push(result);
                    continue;
                }
                let mut head = id;
                let mut args = Vec::new();
                while let Node::App(f, a) = d.store.node(head) {
                    let (f, a) = (*f, *a);
                    inventory(&mut d, args.len(), 1)?;
                    args.push(a);
                    head = f;
                }
                args.reverse();
                if let Node::Global(name) | Node::Const { name, .. } = d.store.node(head) {
                    d.budget
                        .spend((d.store.name(*name).len() as u64).saturating_mul(2))?;
                }
                let candidate = match d.store.node(head) {
                    Node::Global(name) if instance.is_empty() => Some(*name),
                    Node::Const { name, levels }
                        if restoration.contains_key(d.store.name(*name)) =>
                    {
                        if !same_levels(&d.store, d.budget, levels, instance)? {
                            return Err(invalid("native nested restoration met a universe instance other than the family's own"));
                        }
                        Some(*name)
                    }
                    _ => None,
                };
                let image = candidate.and_then(|name| {
                    restoration
                        .get(d.store.name(name))
                        .map(|image| (name, image))
                });
                if let Some((name, image)) = image {
                    if args.len() < params.len() {
                        return Err(invalid("native nested restoration met a partially applied shared parameter spine"));
                    }
                    let image = if let Some(&id) = images.get(&name) {
                        id
                    } else {
                        inventory(&mut d, images.len(), 1)?;
                        let image = d.intern(image)?;
                        images.insert(name, image);
                        image
                    };
                    inventory(
                        &mut d,
                        pending.len(),
                        args.len().checked_add(1).ok_or_else(limit)?,
                    )?;
                    pending.push(Task::Image(id, image, args.len()));
                    pending.extend(args.into_iter().rev().map(Task::Enter));
                } else {
                    let n = d.store.node(id).child_count();
                    inventory(&mut d, pending.len(), n.checked_add(1).ok_or_else(limit)?)?;
                    inventory(&mut d, 0, n)?;
                    let mut children = Vec::with_capacity(n);
                    d.store
                        .node(id)
                        .for_each_child(&mut |child| children.push(child));
                    pending.push(Task::Build(id, n));
                    pending.extend(children.into_iter().rev().map(Task::Enter));
                }
            }
            Task::Build(id, n) => {
                let base = out.len() - n;
                // rebuild_with copies a Const's level vector or a Match/MutualFix child vector.
                // Admit the owned payload BEFORE creating the Node passed to bounded add.
                let payload = match d.store.node(id) {
                    Node::Const { levels, .. } => levels.len(),
                    _ => n,
                };
                inventory(&mut d, 0, payload)?;
                let node = d.store.node(id).rebuild_with(&out[base..]);
                let result = d.add(node)?;
                out.truncate(base);
                inventory(&mut d, cache.len(), 1)?;
                cache.insert(id, result);
                out.push(result);
            }
            Task::Image(id, image, n) => {
                let base = out.len() - n;
                inventory(&mut d, 0, names.len())?;
                let pairs: Vec<_> = names
                    .iter()
                    .copied()
                    .zip(out[base..].iter().copied())
                    .collect();
                // Simultaneous capture-avoiding substitution keeps shared argument DAGs intact.
                // Images themselves are NOT recursively restored, matching the ordinary helper.
                let mut result = d.instantiate(image, &pairs)?;
                for index in base + names.len()..out.len() {
                    result = d.add(Node::App(result, out[index]))?;
                }
                out.truncate(base);
                inventory(&mut d, cache.len(), 1)?;
                cache.insert(id, result);
                out.push(result);
            }
        }
    }
    if out.len() != 1 {
        return Err(invalid("native restoration result inventory"));
    }
    d.extract(out[0])
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/inductive_compile/bounded_native_restoration/controls.rs"]
mod controls;
