//! The dataflow facts every rule reads, computed once and kept current as ops are absorbed.
//!
//! The graph being rewritten is a list of slots in program order: each slot an op, or empty once its op is absorbed into
//! another. Beside it, which live slots read each tensor, write it and compute it whole — updated through every change a
//! slot takes, since the slots are reached only through these methods. A rule asks the facts instead of scanning the
//! graph: who computes `t`, who reads it, whether anything between two slots touches it, and whether moving a slot up
//! to another would cross a slot that sees the difference.

use std::cell::{RefCell, RefMut};
use std::collections::{BTreeSet, HashMap};

use super::graph::Op;

/// The tensors an op reads: its operands, and the parameters that name the rows its result, or any part's, lands in.
pub fn reads(op: &Op) -> Vec<&str> {
    let mut r: Vec<&str> = op.ins.iter().map(String::as_str).collect();
    r.extend(op.row_params());
    r.sort_unstable();
    r.dedup();
    r
}

/// The tensors an op writes: its destination's buffer, what it writes in place, and what it keeps.
pub fn writes(op: &Op) -> Vec<&str> {
    op.writes()
}

/// The tensor an op computes whole into a buffer of its own, if any.
pub fn result(op: &Op) -> Option<&str> {
    op.result()
}

/// The slots, and the index of what each reads, writes and computes.
#[derive(Clone)]
pub struct Slots {
    slots: Vec<Option<Op>>,
    index: RefCell<Index>,
}

#[derive(Clone, Default)]
struct Index {
    reads: HashMap<String, BTreeSet<usize>>,
    writes: HashMap<String, BTreeSet<usize>>,
    results: HashMap<String, BTreeSet<usize>>,
    of: Vec<[Vec<String>; 3]>,
    changed: BTreeSet<usize>,
    moved: bool,
}

impl Index {
    fn maps(&mut self) -> [&mut HashMap<String, BTreeSet<usize>>; 3] {
        [&mut self.reads, &mut self.writes, &mut self.results]
    }

    fn enter(&mut self, i: usize, slot: &Option<Op>) {
        let names = |v: Vec<&str>| v.into_iter().map(String::from).collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>();
        let of = match slot {
            Some(op) => [names(reads(op)), names(writes(op)), result(op).map(String::from).into_iter().collect()],
            None => Default::default(),
        };
        for (map, names) in self.maps().into_iter().zip(&of) {
            for t in names {
                map.entry(t.clone()).or_default().insert(i);
            }
        }
        self.of[i] = of;
    }

    fn leave(&mut self, i: usize) {
        let of = std::mem::take(&mut self.of[i]);
        for (map, names) in self.maps().into_iter().zip(&of) {
            for t in names {
                if let Some(at) = map.get_mut(t) {
                    at.remove(&i);
                    if at.is_empty() {
                        map.remove(t);
                    }
                }
            }
        }
    }
}

impl Slots {
    pub fn new(ops: Vec<Op>) -> Self {
        Slots { slots: ops.into_iter().map(Some).collect(), index: RefCell::new(Index { moved: true, ..Index::default() }) }
    }

    fn index(&self) -> RefMut<'_, Index> {
        let mut ix = self.index.borrow_mut();
        if ix.moved {
            *ix = Index { of: vec![Default::default(); self.slots.len()], ..Index::default() };
            for (i, slot) in self.slots.iter().enumerate() {
                ix.enter(i, slot);
            }
        } else {
            for i in std::mem::take(&mut ix.changed) {
                ix.leave(i);
                ix.enter(i, &self.slots[i]);
            }
        }
        ix
    }

    fn changed(&self, i: usize) {
        self.index.borrow_mut().changed.insert(i);
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn slot(&self, i: usize) -> Option<&Op> {
        self.slots[i].as_ref()
    }

    pub fn get(&self, i: usize) -> &Op {
        self.slots[i].as_ref().expect("a live slot")
    }

    pub fn get_mut(&mut self, i: usize) -> &mut Op {
        self.changed(i);
        self.slots[i].as_mut().expect("a live slot")
    }

    pub fn take(&mut self, i: usize) -> Option<Op> {
        self.changed(i);
        self.slots[i].take()
    }

    pub fn put(&mut self, i: usize, op: Op) {
        self.changed(i);
        self.slots[i] = Some(op);
    }

    /// A new slot at `i`, every slot from `i` on moved one along.
    pub fn insert(&mut self, i: usize, op: Op) {
        self.slots.insert(i, Some(op));
        self.index.get_mut().moved = true;
    }

    pub fn live(&self) -> impl Iterator<Item = (usize, &Op)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| s.as_ref().map(|op| (i, op)))
    }

    pub fn into_ops(self) -> Vec<Op> {
        self.slots.into_iter().flatten().collect()
    }

    /// The slots that read `t`, in order.
    pub fn readers(&self, t: &str) -> Vec<usize> {
        self.index().reads.get(t).map_or_else(Vec::new, |at| at.iter().copied().collect())
    }

    /// The slot that computes `t` whole into its own buffer.
    pub fn producer(&self, t: &str) -> Option<usize> {
        self.index().results.get(t).and_then(|at| at.first().copied())
    }

    /// The slot that writes `t` whole: as its result, or as a result it keeps beside it.
    pub fn maker(&self, t: &str) -> Option<usize> {
        self.producer(t).or_else(|| {
            let at = self.index().writes.get(t).cloned().unwrap_or_default();
            at.into_iter().find(|&k| self.get(k).keeps.iter().any(|kept| kept == t))
        })
    }

    /// Whether no live slot strictly between `from` and `to`, but those of `skip`, writes `t` (`reads` too when asked).
    pub fn untouched(&self, t: &str, from: usize, to: usize, reads: bool, skip: &[usize]) -> bool {
        let ix = self.index();
        let none_in = |map: &HashMap<String, BTreeSet<usize>>| {
            map.get(t).is_none_or(|at| at.range(from + 1..to.max(from + 1)).all(|b| skip.contains(b)))
        };
        none_in(&ix.writes) && (!reads || none_in(&ix.reads))
    }

    /// Whether slot `j`'s op, moved up to slot `i`, would cross a live slot between them but those of `group`: one that
    /// writes what it reads or writes, or reads what it writes.
    pub fn crosses(&self, i: usize, j: usize, group: &[usize]) -> bool {
        self.crosses_from(j, i, j, group)
    }

    /// Whether slot `i`'s op, moved down to slot `j`, would cross a live slot between them but those of `group`.
    pub fn sinks_past(&self, i: usize, j: usize, group: &[usize]) -> bool {
        self.crosses_from(i, i, j, group)
    }

    /// Whether slot `at`'s op meets a live slot strictly between `lo` and `hi`, but those of `group`, that writes what it
    /// reads or writes, or reads what it writes.
    fn crosses_from(&self, at: usize, lo: usize, hi: usize, group: &[usize]) -> bool {
        let op = self.get(at);
        let (r, w) = (reads(op), writes(op));
        let ix = self.index();
        let between = |map: &HashMap<String, BTreeSet<usize>>, t: &str| {
            map.get(t).is_some_and(|slots| slots.range(lo + 1..hi.max(lo + 1)).any(|b| !group.contains(b)))
        };
        r.iter().chain(&w).any(|t| between(&ix.writes, t)) || w.iter().any(|t| between(&ix.reads, t))
    }
}

/// What an op accumulates in integers: nothing, a sum whose operands' ranges no fact bounds, or every partial result
/// within an interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accumulates {
    Nothing,
    Unproven,
    Within(i128, i128),
}

impl Accumulates {
    /// The fewest bits of a two's-complement integer that holds every partial result, by its largest magnitude.
    pub fn bits(self) -> Option<u32> {
        match self {
            Accumulates::Within(lo, hi) => {
                let magnitude = lo.unsigned_abs().max(hi.unsigned_abs());
                Some(129 - magnitude.leading_zeros())
            }
            _ => None,
        }
    }
}

/// The interval every partial sum of `k` products of operands in `a` and `b` lies in: the running sums start at zero and
/// each adds one product, every product between the corners'.
pub fn product_sums(a: (i64, i64), b: (i64, i64), k: usize) -> (i128, i128) {
    let corners = [a.0 as i128 * b.0 as i128, a.0 as i128 * b.1 as i128, a.1 as i128 * b.0 as i128, a.1 as i128 * b.1 as i128];
    let (lo, hi) = (*corners.iter().min().expect("four corners"), *corners.iter().max().expect("four corners"));
    let k = k as i128;
    (lo.saturating_mul(k).min(0), hi.saturating_mul(k).max(0))
}

/// What `op` accumulates in integers, from the ranges `g` proves of its operands: a matrix product's running sums over
/// its inner dimension, an elementwise sum or product, a requant's product before its shift.
pub fn accumulator(op: &super::graph::Op, g: &super::graph::TensorGraph) -> Accumulates {
    use super::front::TensorOpSpec;
    use super::graph::Elem;
    let int = |k: usize| op.ins.get(k).and_then(|t| g.tensors.get(t)).is_some_and(|t| t.elem == Elem::Int);
    let range = |k: usize| op.ins.get(k).and_then(|t| g.tensors.get(t)).and_then(|t| t.range);
    let two = |f: &dyn Fn((i64, i64), (i64, i64)) -> (i128, i128)| match (range(0), range(1)) {
        (Some(a), Some(b)) => {
            let (lo, hi) = f(a, b);
            Accumulates::Within(lo, hi)
        }
        _ => Accumulates::Unproven,
    };
    match op.prim() {
        Some(TensorOpSpec::Matmul(m)) if int(0) && int(1) => two(&|a, b| product_sums(a, b, m.k)),
        Some(TensorOpSpec::Elementwise(e)) if int(0) && int(1) => match e.op {
            crate::tensor_recognize::ElementwiseOp::Add => two(&|a, b| (a.0 as i128 + b.0 as i128, a.1 as i128 + b.1 as i128)),
            crate::tensor_recognize::ElementwiseOp::Mul => two(&|a, b| product_sums(a, b, 1)),
        },
        Some(TensorOpSpec::IntRequant(q)) => match range(0) {
            Some(x) => {
                let (a, b) = (x.0 as i128 * q.m as i128, x.1 as i128 * q.m as i128);
                Accumulates::Within(a.min(b), a.max(b))
            }
            None => Accumulates::Unproven,
        },
        _ => Accumulates::Nothing,
    }
}
