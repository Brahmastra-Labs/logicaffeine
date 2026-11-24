//! Reduced ordered binary decision diagrams.
//!
//! A Boolean function over the variables `0..vars`, ordered by index (variable 0 at the top), as a directed acyclic
//! graph of decision nodes `(var ? hi : lo)` sharing every subgraph. Nodes are hash-consed through a unique table
//! and no node has equal children, so the diagram of a function is canonical: two equal functions are the same
//! [`NodeId`], and equivalence is one comparison. This is the representation bddbddb gives a Datalog relation, a
//! set of tuples as the characteristic function of their bit encodings, where a join is a relational product
//! ([`Bdd::rel_prod`]) and a projection is an existential ([`Bdd::exists`]).
//!
//! Every connective goes through one memoized `if-then-else` ([`Bdd::ite`]); quantification, the relational product
//! and renaming each memoize within a call. Node 0 is false and node 1 is true.

use std::collections::{BTreeSet, HashMap};

/// A node of a [`Bdd`]: 0 is false, 1 is true, the rest decision nodes.
pub type NodeId = u32;

const FALSE: NodeId = 0;
const TRUE: NodeId = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Node {
    var: u32,
    lo: NodeId,
    hi: NodeId,
}

/// A manager of decision diagrams over `0..vars`: every function built through it shares its nodes.
pub struct Bdd {
    vars: u32,
    nodes: Vec<Node>,
    unique: HashMap<Node, NodeId>,
    ite_cache: HashMap<(NodeId, NodeId, NodeId), NodeId>,
}

impl Bdd {
    /// A manager over the variables `0..vars`.
    pub fn new(vars: u32) -> Self {
        let terminal = |v| Node { var: vars, lo: v, hi: v };
        Bdd { vars, nodes: vec![terminal(FALSE), terminal(TRUE)], unique: HashMap::new(), ite_cache: HashMap::new() }
    }

    /// How many variables the manager orders.
    pub fn vars(&self) -> u32 {
        self.vars
    }

    /// Every node the manager holds, terminals included: its memory.
    pub fn size(&self) -> usize {
        self.nodes.len()
    }

    /// The constant function.
    pub fn constant(&self, value: bool) -> NodeId {
        if value { TRUE } else { FALSE }
    }

    /// The function that is variable `v`.
    pub fn var(&mut self, v: u32) -> NodeId {
        assert!(v < self.vars, "variable {v} of a manager over {}", self.vars);
        self.mk(v, FALSE, TRUE)
    }

    fn mk(&mut self, var: u32, lo: NodeId, hi: NodeId) -> NodeId {
        if lo == hi {
            return lo;
        }
        let node = Node { var, lo, hi };
        if let Some(&id) = self.unique.get(&node) {
            return id;
        }
        let id = self.nodes.len() as NodeId;
        self.nodes.push(node);
        self.unique.insert(node, id);
        id
    }

    fn level(&self, f: NodeId) -> u32 {
        self.nodes[f as usize].var
    }

    /// The cofactors of `f` by variable `v`, which must not be below `f`'s top variable.
    fn cofactors(&self, f: NodeId, v: u32) -> (NodeId, NodeId) {
        let n = self.nodes[f as usize];
        if n.var == v { (n.lo, n.hi) } else { (f, f) }
    }

    /// `if c then t else e`.
    pub fn ite(&mut self, c: NodeId, t: NodeId, e: NodeId) -> NodeId {
        if c == TRUE {
            return t;
        }
        if c == FALSE {
            return e;
        }
        if t == e {
            return t;
        }
        if t == TRUE && e == FALSE {
            return c;
        }
        if let Some(&r) = self.ite_cache.get(&(c, t, e)) {
            return r;
        }
        let v = self.level(c).min(self.level(t)).min(self.level(e));
        let (c0, c1) = self.cofactors(c, v);
        let (t0, t1) = self.cofactors(t, v);
        let (e0, e1) = self.cofactors(e, v);
        let lo = self.ite(c0, t0, e0);
        let hi = self.ite(c1, t1, e1);
        let r = self.mk(v, lo, hi);
        self.ite_cache.insert((c, t, e), r);
        r
    }

    /// `¬f`.
    pub fn not(&mut self, f: NodeId) -> NodeId {
        self.ite(f, FALSE, TRUE)
    }

    /// `f ∧ g`.
    pub fn and(&mut self, f: NodeId, g: NodeId) -> NodeId {
        self.ite(f, g, FALSE)
    }

    /// `f ∨ g`.
    pub fn or(&mut self, f: NodeId, g: NodeId) -> NodeId {
        self.ite(f, TRUE, g)
    }

    /// `f ⊕ g`.
    pub fn xor(&mut self, f: NodeId, g: NodeId) -> NodeId {
        let ng = self.not(g);
        self.ite(f, ng, g)
    }

    /// `f ∧ ¬g`.
    pub fn diff(&mut self, f: NodeId, g: NodeId) -> NodeId {
        let ng = self.not(g);
        self.and(f, ng)
    }

    /// `∃ vars. f`.
    pub fn exists(&mut self, f: NodeId, vars: &[u32]) -> NodeId {
        let set: BTreeSet<u32> = vars.iter().copied().collect();
        let mut memo = HashMap::new();
        self.exists_in(f, &set, &mut memo)
    }

    fn exists_in(&mut self, f: NodeId, set: &BTreeSet<u32>, memo: &mut HashMap<NodeId, NodeId>) -> NodeId {
        if f <= TRUE {
            return f;
        }
        let n = self.nodes[f as usize];
        if set.range(n.var..).next().is_none() {
            return f;
        }
        if let Some(&r) = memo.get(&f) {
            return r;
        }
        let lo = self.exists_in(n.lo, set, memo);
        let r = if set.contains(&n.var) {
            if lo == TRUE {
                TRUE
            } else {
                let hi = self.exists_in(n.hi, set, memo);
                self.or(lo, hi)
            }
        } else {
            let hi = self.exists_in(n.hi, set, memo);
            self.mk(n.var, lo, hi)
        };
        memo.insert(f, r);
        r
    }

    /// `∃ vars. f ∧ g`, without building the conjunction first: a join and its projection in one pass.
    pub fn rel_prod(&mut self, f: NodeId, g: NodeId, vars: &[u32]) -> NodeId {
        let set: BTreeSet<u32> = vars.iter().copied().collect();
        let mut memo = HashMap::new();
        self.rel_prod_in(f, g, &set, &mut memo)
    }

    fn rel_prod_in(
        &mut self,
        f: NodeId,
        g: NodeId,
        set: &BTreeSet<u32>,
        memo: &mut HashMap<(NodeId, NodeId), NodeId>,
    ) -> NodeId {
        if f == FALSE || g == FALSE {
            return FALSE;
        }
        if f == TRUE && g == TRUE {
            return TRUE;
        }
        if f == TRUE {
            let mut m = HashMap::new();
            return self.exists_in(g, set, &mut m);
        }
        if g == TRUE {
            let mut m = HashMap::new();
            return self.exists_in(f, set, &mut m);
        }
        let key = if f <= g { (f, g) } else { (g, f) };
        if let Some(&r) = memo.get(&key) {
            return r;
        }
        let v = self.level(f).min(self.level(g));
        let (f0, f1) = self.cofactors(f, v);
        let (g0, g1) = self.cofactors(g, v);
        let lo = self.rel_prod_in(f0, g0, set, memo);
        let r = if set.contains(&v) {
            if lo == TRUE {
                TRUE
            } else {
                let hi = self.rel_prod_in(f1, g1, set, memo);
                self.or(lo, hi)
            }
        } else {
            let hi = self.rel_prod_in(f1, g1, set, memo);
            self.mk(v, lo, hi)
        };
        memo.insert(key, r);
        r
    }

    /// `f` with each variable `from` replaced by `to`, all at once: a simultaneous substitution of variables, in any
    /// order relative to the manager's.
    pub fn rename(&mut self, f: NodeId, map: &[(u32, u32)]) -> NodeId {
        let to: HashMap<u32, u32> = map.iter().copied().collect();
        let mut memo = HashMap::new();
        self.rename_in(f, &to, &mut memo)
    }

    fn rename_in(&mut self, f: NodeId, to: &HashMap<u32, u32>, memo: &mut HashMap<NodeId, NodeId>) -> NodeId {
        if f <= TRUE {
            return f;
        }
        if let Some(&r) = memo.get(&f) {
            return r;
        }
        let n = self.nodes[f as usize];
        let lo = self.rename_in(n.lo, to, memo);
        let hi = self.rename_in(n.hi, to, memo);
        let target = to.get(&n.var).copied().unwrap_or(n.var);
        let x = self.var(target);
        let r = self.ite(x, hi, lo);
        memo.insert(f, r);
        r
    }

    /// `f` under the assignment `value`.
    pub fn eval(&self, f: NodeId, value: impl Fn(u32) -> bool) -> bool {
        let mut f = f;
        while f > TRUE {
            let n = self.nodes[f as usize];
            f = if value(n.var) { n.hi } else { n.lo };
        }
        f == TRUE
    }

    /// The assignments of all `vars` variables that satisfy `f`.
    pub fn sat_count(&self, f: NodeId) -> u128 {
        let mut memo = HashMap::new();
        self.count_below(f, &mut memo) << self.level(f).min(self.vars)
    }

    /// Satisfying assignments of the variables from `f`'s level down.
    fn count_below(&self, f: NodeId, memo: &mut HashMap<NodeId, u128>) -> u128 {
        if f == FALSE {
            return 0;
        }
        if f == TRUE {
            return 1;
        }
        if let Some(&c) = memo.get(&f) {
            return c;
        }
        let n = self.nodes[f as usize];
        let lo = self.count_below(n.lo, memo) << (self.level(n.lo) - n.var - 1);
        let hi = self.count_below(n.hi, memo) << (self.level(n.hi) - n.var - 1);
        let c = lo + hi;
        memo.insert(f, c);
        c
    }

    /// The assignments of `vars` alone that satisfy `f`, which must test no variable outside them: the size of the set
    /// `f` is the characteristic function of, when `vars` are its encoding's bits.
    pub fn sat_count_over(&self, f: NodeId, vars: &[u32]) -> u128 {
        let mut set = vars.to_vec();
        set.sort_unstable();
        set.dedup();
        // The variables of the set ordered before `level` (a terminal's level, `self.vars`, is after all of them).
        let rank = |level: u32| set.partition_point(|&v| v < level) as u32;
        let mut memo = HashMap::new();
        self.count_over(f, &set, &rank, &mut memo) << rank(self.level(f).min(self.vars))
    }

    fn count_over(&self, f: NodeId, set: &[u32], rank: &impl Fn(u32) -> u32, memo: &mut HashMap<NodeId, u128>) -> u128 {
        if f == FALSE {
            return 0;
        }
        if f == TRUE {
            return 1;
        }
        if let Some(&c) = memo.get(&f) {
            return c;
        }
        let n = self.nodes[f as usize];
        assert!(set.binary_search(&n.var).is_ok(), "the function tests variable {}, outside the counted ones", n.var);
        let here = rank(n.var);
        let lo = self.count_over(n.lo, set, rank, memo) << (rank(self.level(n.lo)) - here - 1);
        let hi = self.count_over(n.hi, set, rank, memo) << (rank(self.level(n.hi)) - here - 1);
        let c = lo + hi;
        memo.insert(f, c);
        c
    }

    /// The decision nodes `f` reaches (terminals excluded): the size of its diagram.
    pub fn node_count(&self, f: NodeId) -> usize {
        let mut seen = BTreeSet::new();
        let mut stack = vec![f];
        while let Some(g) = stack.pop() {
            if g > TRUE && seen.insert(g) {
                let n = self.nodes[g as usize];
                stack.push(n.lo);
                stack.push(n.hi);
            }
        }
        seen.len()
    }

    /// Every assignment of `vars` that satisfies `f`, each in the order `vars` lists them, every variable `f` does not
    /// test taken both ways. `f` must depend on no variable outside `vars`.
    pub fn assignments(&self, f: NodeId, vars: &[u32]) -> Vec<Vec<bool>> {
        let mut order: Vec<(u32, usize)> = vars.iter().enumerate().map(|(i, &v)| (v, i)).collect();
        order.sort_unstable();
        let mut out = Vec::new();
        let mut current = vec![false; vars.len()];
        self.enumerate(f, &order, 0, &mut current, &mut out);
        out
    }

    fn enumerate(&self, f: NodeId, order: &[(u32, usize)], i: usize, current: &mut Vec<bool>, out: &mut Vec<Vec<bool>>) {
        if f == FALSE {
            return;
        }
        if i == order.len() {
            assert_eq!(f, TRUE, "the function depends on a variable outside the enumerated ones");
            out.push(current.clone());
            return;
        }
        let (v, slot) = order[i];
        let (lo, hi) = self.cofactors(f, v);
        current[slot] = false;
        self.enumerate(lo, order, i + 1, current, out);
        current[slot] = true;
        self.enumerate(hi, order, i + 1, current, out);
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/bdd/tests.rs"]
mod tests;
