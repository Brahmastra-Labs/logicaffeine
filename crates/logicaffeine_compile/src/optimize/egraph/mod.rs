//! The ARCHITECT — equality saturation over a compiler e-graph
//! (EXODIA Phase 4, sprints 17–22).
//!
//! Classic egg-style design: a hash-consed term bank over the shared
//! [`UnionFind`] (the same engine the kernel's congruence closure uses),
//! a worklist `rebuild` restoring the congruence invariant after unions,
//! budgeted saturation with a deterministic rule order, and bottom-up
//! cost extraction.
//!
//! Class facts (scalar kind + integer interval) come from the Oracle and
//! gate the conditional rewrites: `x / 2^n → x >> n` fires only with a
//! non-negativity proof, the Group-2 boolean laws fire only on proven
//! Bools, and EVERYTHING fails closed when no fact is present.

pub mod convert;
pub mod enode;
pub mod extract;
pub mod rules;

pub use enode::CompilerENode;

use std::collections::HashMap;

use logicaffeine_base::union_find::UnionFind;

use crate::optimize::ScalarKind;

pub type NodeId = usize;

/// Saturation stops after this many fixpoint iterations…
pub const SATURATION_ITERS: usize = 8;
/// …or when the term bank would exceed this many nodes.
pub const MAX_NODES: usize = 10_000;

/// Per-class knowledge, seeded from literals and the Oracle.
///
/// Facts describe the VALUE of every member of the class (if evaluation
/// succeeds); merging classes therefore INTERSECTS intervals.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClassFact {
    pub scalar: Option<ScalarKind>,
    pub range: Option<(i64, i64)>,
    /// The value (if evaluation succeeds) is a collection — list, text,
    /// map, or set. Kind facts are mutation-immune (contents change,
    /// kinds never do), so they merge by OR.
    pub collection: bool,
    /// The value is specifically a LIST (implies `collection`).
    pub list: bool,
}

pub struct CompilerEGraph {
    uf: UnionFind,
    /// Node id → the node as added (children canonical at insert time).
    nodes: Vec<CompilerENode>,
    /// Canonical node → its id. Stale keys (whose children later merged)
    /// are either still-correct or unreachable by canonical lookups.
    memo: HashMap<CompilerENode, NodeId>,
    /// Class root → member node ids.
    class_nodes: HashMap<NodeId, Vec<NodeId>>,
    /// Child class root → parent node ids (for congruence repair).
    uses: HashMap<NodeId, Vec<NodeId>>,
    /// Class root → facts.
    facts: HashMap<NodeId, ClassFact>,
    /// Roots whose parents need congruence repair.
    dirty: Vec<NodeId>,
}

impl CompilerEGraph {
    pub fn new() -> Self {
        CompilerEGraph {
            uf: UnionFind::new(),
            nodes: Vec::new(),
            memo: HashMap::new(),
            class_nodes: HashMap::new(),
            uses: HashMap::new(),
            facts: HashMap::new(),
            dirty: Vec::new(),
        }
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn find(&mut self, id: NodeId) -> NodeId {
        self.uf.find(id)
    }

    /// The stored node with children mapped to their CURRENT class roots.
    pub fn canonical_node(&mut self, id: NodeId) -> CompilerENode {
        let node = self.nodes[id];
        node.map_children(|c| self.uf.find(c))
    }

    /// Member node ids of `id`'s class, in insertion order (deterministic).
    pub fn class_members(&mut self, id: NodeId) -> Vec<NodeId> {
        let root = self.find(id);
        self.class_nodes.get(&root).cloned().unwrap_or_default()
    }

    /// Hash-consing insert: an existing congruent node is returned instead
    /// of allocating. Literal leaves seed their own class facts.
    pub fn add(&mut self, node: CompilerENode) -> NodeId {
        let canonical = node.map_children(|c| self.uf.find(c));
        if let Some(&existing) = self.memo.get(&canonical) {
            return self.uf.find(existing);
        }
        let id = self.uf.make_set();
        debug_assert_eq!(id, self.nodes.len());
        self.nodes.push(canonical);
        self.memo.insert(canonical, id);
        self.class_nodes.insert(id, vec![id]);
        for child in canonical.children() {
            self.uses.entry(child).or_default().push(id);
        }
        match canonical {
            CompilerENode::Int(k) => {
                self.facts.insert(
                    id,
                    ClassFact {
                        scalar: Some(ScalarKind::Int),
                        range: Some((k, k)),
                        ..ClassFact::default()
                    },
                );
            }
            CompilerENode::Bool(_) => {
                self.facts.insert(
                    id,
                    ClassFact { scalar: Some(ScalarKind::Bool), ..ClassFact::default() },
                );
            }
            CompilerENode::Float(_) => {
                self.facts.insert(
                    id,
                    ClassFact { scalar: Some(ScalarKind::Float), ..ClassFact::default() },
                );
            }
            // A slice that evaluates successfully ALWAYS yields a list.
            CompilerENode::Slice(..) => {
                self.facts.insert(
                    id,
                    ClassFact { collection: true, list: true, ..ClassFact::default() },
                );
            }
            // A copy that evaluates preserves its operand's kind; the
            // converter upgrades this when the operand is a proven list.
            _ => {}
        }
        id
    }

    /// Merge two classes. Facts intersect (both described the same value).
    /// Returns true if a merge actually happened.
    pub fn union(&mut self, a: NodeId, b: NodeId) -> bool {
        let ra = self.uf.find(a);
        let rb = self.uf.find(b);
        if ra == rb {
            return false;
        }
        let fa = self.facts.remove(&ra);
        let fb = self.facts.remove(&rb);
        self.uf.union(ra, rb);
        let root = self.uf.find(ra);
        let loser = if root == ra { rb } else { ra };

        let mut members = self.class_nodes.remove(&loser).unwrap_or_default();
        self.class_nodes.entry(root).or_default().append(&mut members);
        let mut moved_uses = self.uses.remove(&loser).unwrap_or_default();
        self.uses.entry(root).or_default().append(&mut moved_uses);

        let merged = match (fa, fb) {
            (Some(x), Some(y)) => ClassFact {
                scalar: x.scalar.or(y.scalar),
                range: match (x.range, y.range) {
                    (Some((al, ah)), Some((bl, bh))) => {
                        let lo = al.max(bl);
                        let hi = ah.min(bh);
                        debug_assert!(lo <= hi, "contradictory ranges merged — unsound rule?");
                        if lo <= hi { Some((lo, hi)) } else { Some((al, ah)) }
                    }
                    (r, None) | (None, r) => r,
                },
                // Kind facts are sound statements about the shared value:
                // either side's proof carries over.
                collection: x.collection || y.collection,
                list: x.list || y.list,
            },
            (Some(x), None) | (None, Some(x)) => x,
            (None, None) => ClassFact::default(),
        };
        self.facts.insert(root, merged);
        self.dirty.push(root);
        true
    }

    /// Restore the congruence invariant: parents of merged classes are
    /// re-canonicalized; newly congruent pairs are unioned (worklist).
    pub fn rebuild(&mut self) {
        while let Some(r) = self.dirty.pop() {
            let root = self.uf.find(r);
            let parents = self.uses.remove(&root).unwrap_or_default();
            let mut kept: Vec<NodeId> = Vec::with_capacity(parents.len());
            for p in parents {
                let canon = self.nodes[p].map_children(|c| self.uf.find(c));
                if let Some(&q) = self.memo.get(&canon) {
                    if self.uf.find(q) != self.uf.find(p) {
                        self.union(p, q);
                    }
                }
                let rep = self.uf.find(p);
                self.memo.insert(canon, rep);
                kept.push(p);
            }
            let now = self.uf.find(root);
            self.uses.entry(now).or_default().extend(kept);
        }
    }

    // ----- facts ---------------------------------------------------------

    pub fn set_scalar(&mut self, id: NodeId, kind: ScalarKind) {
        let root = self.find(id);
        self.facts.entry(root).or_default().scalar = Some(kind);
    }

    pub fn set_int_range(&mut self, id: NodeId, lo: i64, hi: i64) {
        let root = self.find(id);
        let fact = self.facts.entry(root).or_default();
        fact.scalar = Some(ScalarKind::Int);
        fact.range = Some((lo, hi));
    }

    pub fn scalar_of(&mut self, id: NodeId) -> Option<ScalarKind> {
        let root = self.find(id);
        self.facts.get(&root).and_then(|f| f.scalar)
    }

    pub fn int_range(&mut self, id: NodeId) -> Option<(i64, i64)> {
        let root = self.find(id);
        self.facts.get(&root).and_then(|f| f.range)
    }

    /// A PROVEN single value (point interval) — sound grounds for folding.
    pub fn int_value(&mut self, id: NodeId) -> Option<i64> {
        match self.int_range(id) {
            Some((lo, hi)) if lo == hi => Some(lo),
            _ => None,
        }
    }

    pub fn proven_nonneg(&mut self, id: NodeId) -> bool {
        matches!(self.int_range(id), Some((lo, _)) if lo >= 0)
    }

    /// Mark the class a proven collection (list/text/map/set).
    pub fn set_collection(&mut self, id: NodeId) {
        let root = self.find(id);
        self.facts.entry(root).or_default().collection = true;
    }

    /// Mark the class a proven LIST (implies collection).
    pub fn set_list(&mut self, id: NodeId) {
        let root = self.find(id);
        let fact = self.facts.entry(root).or_default();
        fact.collection = true;
        fact.list = true;
    }

    pub fn proven_collection(&mut self, id: NodeId) -> bool {
        let root = self.find(id);
        self.facts.get(&root).is_some_and(|f| f.collection)
    }

    pub fn proven_list(&mut self, id: NodeId) -> bool {
        let root = self.find(id);
        self.facts.get(&root).is_some_and(|f| f.list)
    }

    /// Does the class contain the literal `Bool(b)`?
    pub fn class_has_bool(&mut self, id: NodeId, b: bool) -> bool {
        self.class_members(id)
            .into_iter()
            .any(|m| self.nodes[m] == CompilerENode::Bool(b))
    }

    /// Whole-tree totality: the class has at least one member whose op is
    /// total and whose children are recursively total. A rewrite may only
    /// DELETE (stop evaluating) a provably total subterm — anything else
    /// could erase a runtime error or an effect. Bottom-up fixpoint,
    /// cycle-tolerant (cycles stay "not yet proven" until a leaf grounds
    /// them).
    ///
    /// Totality is FACT-CONDITIONAL where the op's only failure mode is a
    /// kind error: `Len`/`Copy`/`Contains` never raise over a PROVEN
    /// collection, and raise a type error over anything unproven — so
    /// without the fact they stay non-total, exactly like fold.rs's
    /// fail-closed `expr_is_total`.
    pub fn provably_total(&mut self, id: NodeId) -> bool {
        let mut total: HashMap<NodeId, bool> = HashMap::new();
        loop {
            let mut changed = false;
            for n in 0..self.nodes.len() {
                let node = self.nodes[n];
                let root = self.uf.find(n);
                if *total.get(&root).unwrap_or(&false) {
                    continue;
                }
                let op_total = match node {
                    CompilerENode::Len(c)
                    | CompilerENode::Copy(c)
                    | CompilerENode::Contains(c, _) => self.proven_collection(c),
                    _ => node.op_is_total(),
                };
                if op_total
                    && node
                        .children()
                        .iter()
                        .all(|&c| *total.get(&self.uf.find(c)).unwrap_or(&false))
                {
                    total.insert(root, true);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let root = self.find(id);
        *total.get(&root).unwrap_or(&false)
    }

    // ----- saturation ----------------------------------------------------

    /// Budgeted equality saturation: deterministic node order × rule order,
    /// unions applied immediately, congruence repaired per iteration. Stops
    /// at fixpoint, iteration cap, or the node-bank cap.
    pub fn saturate(&mut self, rules: &[rules::Rewrite]) {
        self.rebuild();
        'outer: for _ in 0..SATURATION_ITERS {
            let snapshot = self.nodes.len();
            let mut changed = false;
            for id in 0..snapshot {
                for rule in rules {
                    if self.nodes.len() + 4 > MAX_NODES {
                        self.rebuild();
                        break 'outer;
                    }
                    if let Some(replacement) = (rule.apply)(self, id) {
                        changed |= self.union(id, replacement);
                    }
                }
            }
            self.rebuild();
            if !changed && self.nodes.len() == snapshot {
                break;
            }
        }
    }
}

impl Default for CompilerEGraph {
    fn default() -> Self {
        Self::new()
    }
}
