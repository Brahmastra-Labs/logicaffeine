//! Bottom-up cost extraction (cycle-tolerant).
//!
//! LogosCost is a static per-op estimate tuned to the VM/JIT tier: shifts,
//! adds and compares are unit ops; multiply is a few; divide/modulo are an
//! order worse. Costs propagate to a per-class fixpoint — cyclic classes
//! (x ≡ x + 0) simply never beat their finite leaf, so extraction always
//! terminates with a finite tree.

use std::collections::HashMap;

use super::{CompilerEGraph, CompilerENode, NodeId};

#[derive(Debug)]
pub struct ExtractTree {
    pub node: CompilerENode,
    pub children: Vec<ExtractTree>,
}

fn op_cost(node: &CompilerENode) -> u64 {
    use CompilerENode::*;
    match node {
        Int(_) | Bool(_) | Float(_) | Var(..) => 1,
        // Opaque subtrees re-emit the original expression; bias against
        // duplicating them when a modeled form exists.
        Opaque(_) => 4,
        Add(..) | Sub(..) | Shl(..) | Shr(..) | BitXor(..) | And(..) | Or(..) | Not(..)
        | Eq(..) | Ne(..) | Lt(..) | Le(..) | Gt(..) | Ge(..) => 1,
        Len(..) => 2,
        Index(..) => 3,
        Mul(..) => 4,
        Contains(..) => 6,
        Slice(..) => 8,
        // O(n) materialization — the fusion algebra exists to delete these.
        Copy(..) => 10,
        Concat(..) => 8,
        Div(..) | Mod(..) => 12,
    }
}

/// Per-class best (cost, representative node id), to fixpoint. Nodes the
/// `admissible` filter rejects never become representatives — extraction
/// for one statement must not reuse a STALE variable version or another
/// statement's opaque (that would duplicate its effect).
fn best_costs(
    eg: &mut CompilerEGraph,
    admissible: &dyn Fn(&CompilerENode) -> bool,
) -> HashMap<NodeId, (u64, NodeId)> {
    let mut best: HashMap<NodeId, (u64, NodeId)> = HashMap::new();
    loop {
        let mut changed = false;
        for id in 0..eg.node_count() {
            let node = eg.canonical_node(id);
            if !admissible(&node) {
                continue;
            }
            let mut total = op_cost(&node);
            let mut ready = true;
            for c in node.children() {
                match best.get(&c) {
                    Some(&(cc, _)) => total = total.saturating_add(cc),
                    None => {
                        ready = false;
                        break;
                    }
                }
            }
            if !ready {
                continue;
            }
            let root = eg.find(id);
            match best.get(&root) {
                Some(&(old, _)) if old <= total => {}
                _ => {
                    best.insert(root, (total, id));
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    best
}

fn build_tree(
    eg: &mut CompilerEGraph,
    best: &HashMap<NodeId, (u64, NodeId)>,
    class: NodeId,
) -> Option<ExtractTree> {
    let root = eg.find(class);
    let (_, rep) = *best.get(&root)?;
    let node = eg.canonical_node(rep);
    let children = node
        .children()
        .into_iter()
        .map(|c| build_tree(eg, best, c))
        .collect::<Option<Vec<_>>>()?;
    Some(ExtractTree { node, children })
}

/// The cheapest finite tree representing `root`'s class.
pub fn best_tree(eg: &mut CompilerEGraph, root: NodeId) -> ExtractTree {
    let best = best_costs(eg, &|_| true);
    build_tree(eg, &best, root)
        .unwrap_or_else(|| panic!("class has no finite-cost representative"))
}

/// The cheapest finite tree whose every node passes `admissible`, or None
/// when no such tree exists (the caller keeps the original expression).
pub fn best_tree_filtered(
    eg: &mut CompilerEGraph,
    root: NodeId,
    admissible: &dyn Fn(&CompilerENode) -> bool,
) -> Option<ExtractTree> {
    let best = best_costs(eg, admissible);
    build_tree(eg, &best, root)
}
