//! Deterministic one-variable DAG substitution matching type_checker::substitute.
//! This is private to test-only elimination: simultaneous substitution keeps its API.
use super::{resource, Construction, GenerationResult, Node, Symbol, TermId};

/// all_var_names membership, including binder names but excluding globals/universes.
fn mentions_name(d: &mut Construction<'_>, root: TermId, name: Symbol) -> GenerationResult<bool> {
    d.budget.inventory(1)?;
    let mut todo = vec![root];
    while let Some(id) = todo.pop() {
        d.budget.spend(1)?;
        let node = d.store.node(id);
        let found = match node {
            Node::Var(n) | Node::Fix { name: n, .. } | Node::Let { name: n, .. } => *n == name,
            Node::Pi { param, .. } | Node::Lambda { param, .. } => *param == name,
            Node::MutualFix { defs, .. } => {
                d.budget.spend(defs.len() as u64)?;
                defs.iter().any(|(n, _)| *n == name)
            }
            _ => false,
        };
        if found {
            return Ok(true);
        }
        d.budget.inventory(node.child_count())?;
        node.for_each_child(&mut |id| todo.push(id));
    }
    Ok(false)
}

/// Exact fresh_name/freshen choice. For mutual binders, names and every current
/// definition body are included; for ordinary binders only the scoped body is.
fn fresh(
    d: &mut Construction<'_>,
    base: Symbol,
    scoped: &[TermId],
    mutual_names: &[Symbol],
    variable: Symbol,
    value: TermId,
) -> GenerationResult<Symbol> {
    let mut counter = 0u32;
    loop {
        let bytes = d.store.name(base).len() as u64 + 16;
        d.budget.spend(bytes)?;
        d.budget.reserve(bytes + 64)?;
        let candidate = if counter == 0 {
            format!("{}'", d.store.name(base))
        } else {
            format!("{}'{counter}", d.store.name(base))
        };
        let symbol = d.symbol(&candidate)?;
        let mut taken = symbol == variable || d.store.occurs_free_sym(value, symbol);
        d.budget.spend(mutual_names.len() as u64 + 1)?;
        taken |= mutual_names.contains(&symbol);
        for &body in scoped {
            if taken {
                break;
            }
            taken = mentions_name(d, body, symbol)?;
        }
        if !taken {
            return Ok(symbol);
        }
        counter = counter.checked_add(1).ok_or_else(resource)?;
    }
}

/// `fresh` excludes its result from every variable/binder in each scoped child.
/// Therefore this instantiate cannot freshen any binder: it performs precisely
/// the reference rename_var operation and preserves deterministic spellings.
fn rename(
    d: &mut Construction<'_>,
    body: TermId,
    old: Symbol,
    new: Symbol,
) -> GenerationResult<TermId> {
    let value = d.add(Node::Var(new))?;
    d.instantiate(body, &[(old, value)])
}

/// Exact structural, not merely alpha, compatibility with the original *single*
/// substitution. Only the initial root occurrence check skips work. Once entered,
/// the algorithm traverses target-free children because reference substitution
/// may still freshen their binders when another child contains the target.
pub(super) fn substitute(
    d: &mut Construction<'_>,
    root: TermId,
    variable: Symbol,
    value: TermId,
) -> GenerationResult<TermId> {
    d.budget.spend(1)?;
    if !d.store.occurs_free_sym(root, variable) {
        return Ok(root);
    }
    enum Op {
        Visit(TermId),
        Keep(TermId),
        Build(Node, usize),
    }
    d.budget.inventory(1)?;
    let mut todo = vec![Op::Visit(root)];
    let mut out = Vec::new();
    while let Some(op) = todo.pop() {
        d.budget.spend(1)?;
        match op {
            Op::Keep(id) => {
                d.budget.inventory(1)?;
                out.push(id);
            }
            Op::Build(node, n) => {
                let base = out.len() - n;
                d.budget.inventory(n.saturating_add(1))?;
                let rebuilt = node.rebuild_with(&out[base..]);
                out.truncate(base);
                let rebuilt = d.add(rebuilt)?;
                out.push(rebuilt);
            }
            Op::Visit(id) => {
                if let Node::Var(name) = d.store.node(id) {
                    let result = if *name == variable { value } else { id };
                    d.budget.inventory(1)?;
                    out.push(result);
                    continue;
                }
                let n = d.store.node(id).child_count();
                if n == 0 {
                    d.budget.inventory(1)?;
                    out.push(id);
                    continue;
                }
                // Covers the copied node's payload, child ids, worklist entries and
                // possible mutual name/body scratch vectors before any are made.
                d.budget.inventory(n.saturating_mul(6).saturating_add(4))?;
                let mut node = d.store.node(id).clone();
                let mut children = Vec::new();
                node.for_each_child(&mut |child| children.push(child));
                let mut kept = vec![false; n];
                match &mut node {
                    Node::Pi { param, .. } | Node::Lambda { param, .. } => {
                        if *param == variable {
                            kept[1] = true;
                        } else if d.store.occurs_free_sym(value, *param) {
                            let new = fresh(d, *param, &[children[1]], &[], variable, value)?;
                            children[1] = rename(d, children[1], *param, new)?;
                            *param = new;
                        }
                    }
                    Node::Fix { name, .. } => {
                        if *name == variable {
                            kept[0] = true;
                        } else if d.store.occurs_free_sym(value, *name) {
                            let new = fresh(d, *name, &[children[0]], &[], variable, value)?;
                            children[0] = rename(d, children[0], *name, new)?;
                            *name = new;
                        }
                    }
                    Node::Let { name, .. } => {
                        if *name == variable {
                            kept[2] = true;
                        } else if d.store.occurs_free_sym(value, *name) {
                            let new = fresh(d, *name, &[children[2]], &[], variable, value)?;
                            children[2] = rename(d, children[2], *name, new)?;
                            *name = new;
                        }
                    }
                    Node::MutualFix { defs, .. } => {
                        if defs.iter().any(|(name, _)| *name == variable) {
                            kept.fill(true);
                        } else {
                            for i in 0..defs.len() {
                                let old = defs[i].0;
                                if d.store.occurs_free_sym(value, old) {
                                    d.budget.inventory(defs.len())?;
                                    let names: Vec<_> =
                                        defs.iter().map(|(name, _)| *name).collect();
                                    let new = fresh(d, old, &children, &names, variable, value)?;
                                    for child in &mut children {
                                        *child = rename(d, *child, old, new)?;
                                    }
                                    defs[i].0 = new;
                                }
                            }
                        }
                    }
                    _ => {}
                }
                todo.push(Op::Build(node, n));
                for i in (0..n).rev() {
                    todo.push(if kept[i] {
                        Op::Keep(children[i])
                    } else {
                        Op::Visit(children[i])
                    });
                }
            }
        }
    }
    Ok(out.pop().expect("one substitution root"))
}
