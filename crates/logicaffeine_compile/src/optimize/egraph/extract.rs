//! Bottom-up cost extraction (cycle-tolerant).
//!
//! LogosCost is a static per-op estimate tuned to the VM/JIT tier: shifts,
//! adds and compares are unit ops; multiply is a few; divide/modulo are an
//! order worse. Costs propagate to a per-class fixpoint — cyclic classes
//! (x ≡ x + 0) simply never beat their finite leaf, so extraction always
//! terminates with a finite tree.

use std::collections::{HashMap, HashSet};

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
        Add(..) | Sub(..) | Shl(..) | Shr(..) | BitXor(..) | BitAnd(..) | BitOr(..) | And(..)
        | Or(..) | Not(..) | Eq(..) | Ne(..) | Lt(..) | Le(..) | Gt(..) | Ge(..) => 1,
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

/// A saturated e-graph read for extraction: each node with its children at their class roots, the class each node
/// is in, and each class's nodes in ascending id order. Built once per saturation; an extraction then reads only the
/// classes its root reaches, so extracting every statement of a run costs what those statements reach, not the run
/// times the graph.
pub struct Extraction {
    nodes: Vec<CompilerENode>,
    children: Vec<Vec<NodeId>>,
    class_of: Vec<NodeId>,
    members: HashMap<NodeId, Vec<NodeId>>,
}

impl Extraction {
    pub fn of(eg: &mut CompilerEGraph) -> Self {
        let count = eg.node_count();
        let mut nodes = Vec::with_capacity(count);
        let mut children = Vec::with_capacity(count);
        let mut class_of = Vec::with_capacity(count);
        let mut members: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for id in 0..count {
            let node = eg.canonical_node(id);
            children.push(node.children());
            nodes.push(node);
            let class = eg.find(id);
            class_of.push(class);
            members.entry(class).or_default().push(id);
        }
        Self { nodes, children, class_of, members }
    }

    /// The cheapest finite tree for `root`'s class whose every node passes `admissible`, or None when no such tree
    /// exists (the caller keeps the original expression). Nodes the filter rejects never become representatives —
    /// extraction for one statement must not reuse a STALE variable version or another statement's opaque (that
    /// would duplicate its effect).
    pub fn best_tree(&self, root: NodeId, admissible: &dyn Fn(&CompilerENode) -> bool) -> Option<ExtractTree> {
        let reached = self.reached(root, admissible);
        let best = self.best_costs(&reached, admissible);
        self.build_tree(&best, self.class_of[root])
    }

    /// Every node of every class reachable from `root`'s through the nodes `admissible` keeps, in ascending id order:
    /// the nodes whose costs `root`'s extraction reads. A rejected node reads no child, and a node's cost reads only
    /// its children's classes, so no other node changes a cost these hold.
    fn reached(&self, root: NodeId, admissible: &dyn Fn(&CompilerENode) -> bool) -> Vec<NodeId> {
        let mut seen: HashSet<NodeId> = HashSet::new();
        let mut classes = vec![self.class_of[root]];
        let mut reached = Vec::new();
        while let Some(class) = classes.pop() {
            if !seen.insert(class) {
                continue;
            }
            for &id in &self.members[&class] {
                reached.push(id);
                if admissible(&self.nodes[id]) {
                    classes.extend(self.children[id].iter().filter(|c| !seen.contains(*c)));
                }
            }
        }
        reached.sort_unstable();
        reached
    }

    /// Per-class best (cost, representative node id) over `reached`, to fixpoint: the nodes in ascending id order,
    /// round after round, a class's representative replaced only by a strictly cheaper node.
    fn best_costs(&self, reached: &[NodeId], admissible: &dyn Fn(&CompilerENode) -> bool) -> HashMap<NodeId, (u64, NodeId)> {
        let mut best: HashMap<NodeId, (u64, NodeId)> = HashMap::new();
        loop {
            let mut changed = false;
            for &id in reached {
                let node = &self.nodes[id];
                if !admissible(node) {
                    continue;
                }
                let mut total = op_cost(node);
                let mut ready = true;
                for c in &self.children[id] {
                    match best.get(c) {
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
                let class = self.class_of[id];
                match best.get(&class) {
                    Some(&(old, _)) if old <= total => {}
                    _ => {
                        best.insert(class, (total, id));
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

    fn build_tree(&self, best: &HashMap<NodeId, (u64, NodeId)>, class: NodeId) -> Option<ExtractTree> {
        let (_, rep) = *best.get(&class)?;
        let children = self.children[rep]
            .iter()
            .map(|c| self.build_tree(best, *c))
            .collect::<Option<Vec<_>>>()?;
        Some(ExtractTree { node: self.nodes[rep], children })
    }
}

/// The cheapest finite tree representing `root`'s class.
pub fn best_tree(eg: &mut CompilerEGraph, root: NodeId) -> ExtractTree {
    Extraction::of(eg)
        .best_tree(root, &|_| true)
        .unwrap_or_else(|| panic!("class has no finite-cost representative"))
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/egraph/extract/tests.rs"]
mod tests;
