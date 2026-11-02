//! select_hoist — a decision fabric's selects hoisted out of its arithmetic: every select an add, a shift or a compare
//! reads is cofactored away, so the fabric becomes arithmetic over its inputs, compares of that arithmetic, and one
//! tree of selects over the compares' outcomes.
//!
//! [`decision_fabric_from_source`](super::fabric_rtl::decision_fabric_from_source) if-converts a decision into a
//! chain: a compare picks a value, the value feeds a sum, the sum feeds the next compare, so the decision's depth is
//! every carry chain along the chain in series. For any pure `f`, `f(p ? t : e) = p ? f(t) : f(e)`: `f` can compute on
//! both arms at once, before `p` is known. Applied until no select is read by anything but another select's arm (the
//! Shannon expansion of the fabric on its predicates), every carry chain reads only inputs and other carry chains, and
//! the decisions resolve in one tree of selects after them.
//!
//! Predicates are canonical, `x < y` (signed) or `x = y`: `>`, `≥`, `≤` and `≠` swap the operands or the arms. A
//! predicate decided on a path is decided wherever it recurs on that path, and so are its converses (`x < y` makes
//! `y < x` and `x = y` false). Nodes are hash-consed, so cofactors that meet again are one node. Once no select feeds
//! arithmetic, the tree is rebuilt with its predicates ordered by when they arrive (the depth of their compares, in the
//! units of [`op_delay`](super::fabric_pipe::op_delay)), the latest at the root: a late compare drives the select next
//! to the output instead of passing through the whole tree.
//!
//! The pass takes a datapath width, as the carry-save lowering does. Constants are `width`-bit words: a fold or a
//! constant compare is the one at that width, so the pass at a narrow width is the pass at 64 bits restricted to it,
//! and our SAT can check it exhaustively there. Shifts never fold: a shift count past the width means different things
//! at different widths.

use super::fabric_rtl::{AluOp, CmpKind, DecisionFabric, LoopCell, Src};
use std::collections::{BTreeMap, BTreeSet, HashMap};

type Id = usize;

/// The nodes a hoisted fabric may grow to before the pass refuses it: an expansion that would outgrow this many is
/// one whose predicates do not collapse, and the fabric is better left as it is.
pub const HOIST_NODE_LIMIT: usize = 1 << 16;

/// A canonical predicate over node ids: `x < y` on the signed words, or `x = y` with `x ≤ y` by id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Pred {
    Lt(Id, Id),
    Eq(Id, Id),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Node {
    In(usize),
    Const(i64),
    Arith(AluOp, Id, Id),
    Sel(Pred, Id, Id),
}

/// A predicate as built: canonical, or decided by its constant operands.
enum Built {
    Open(Pred),
    Decided(bool),
}

struct Graph {
    nodes: Vec<Node>,
    index: HashMap<Node, Id>,
    width: u32,
}

impl Graph {
    fn add(&mut self, n: Node) -> Result<Id, String> {
        if let Some(&id) = self.index.get(&n) {
            return Ok(id);
        }
        if self.nodes.len() >= HOIST_NODE_LIMIT {
            return Err(format!("hoisting outgrows {HOIST_NODE_LIMIT} nodes: its predicates do not collapse"));
        }
        self.nodes.push(n);
        self.index.insert(n, self.nodes.len() - 1);
        Ok(self.nodes.len() - 1)
    }

    /// `c` as a `width`-bit word, sign-extended: the one representative of its class at that width.
    fn word(&self, c: i64) -> i64 {
        let shift = 64 - self.width;
        (c << shift) >> shift
    }

    fn konst(&mut self, c: i64) -> Result<Id, String> {
        let c = self.word(c);
        self.add(Node::Const(c))
    }

    fn value(&self, id: Id) -> Option<i64> {
        match self.nodes[id] {
            Node::Const(c) => Some(c),
            _ => None,
        }
    }

    /// `a op b`, folded when both are constants and the op's low bits depend on low bits alone, with the identities
    /// of an operand that is zero or all ones; the operands of a commutative op in id order.
    fn arith(&mut self, op: AluOp, a: Id, b: Id) -> Result<Id, String> {
        use AluOp::*;
        let low_bits = matches!(op, Add | Sub | Mul | And | Or | Xor);
        match (op, self.value(a), self.value(b)) {
            (_, Some(x), Some(y)) if low_bits => return self.konst(op.eval(x, y)),
            (And, Some(0), _) | (And, _, Some(0)) | (Mul, Some(0), _) | (Mul, _, Some(0)) => return self.konst(0),
            (And, Some(-1), _) | (Or | Xor | Add, Some(0), _) | (Mul, Some(1), _) => return Ok(b),
            (And, _, Some(-1)) | (Or | Xor | Add | Sub, _, Some(0)) | (Mul, _, Some(1)) => return Ok(a),
            (Shl | Shr | UShr, _, Some(0)) => return Ok(a),
            _ => {}
        }
        // x + (0 − y) = x − y and x − (0 − y) = x + y on wrapping words: a negation an arm brought is no chain of its
        // own.
        let negated = |id: Id| match self.nodes[id] {
            Node::Arith(Sub, z, y) if self.value(z) == Some(0) => Some(y),
            _ => None,
        };
        match (op, negated(a), negated(b)) {
            (Add, _, Some(y)) => return self.arith(Sub, a, y),
            (Add, Some(y), None) => return self.arith(Sub, b, y),
            (Sub, _, Some(y)) => return self.arith(Add, a, y),
            _ => {}
        }
        let (a, b) = if matches!(op, Add | Mul | And | Or | Xor) && b < a { (b, a) } else { (a, b) };
        self.add(Node::Arith(op, a, b))
    }

    /// The canonical form of `x cmp y` as `(predicate, holds-when-the-predicate-holds)`, or its value when its
    /// operands decide it.
    fn compare(&mut self, cmp: CmpKind, x: Id, y: Id) -> (Built, bool) {
        let (p, polarity) = match cmp {
            CmpKind::Lt => (Pred::Lt(x, y), true),
            CmpKind::Ge => (Pred::Lt(x, y), false),
            CmpKind::Gt => (Pred::Lt(y, x), true),
            CmpKind::Le => (Pred::Lt(y, x), false),
            CmpKind::Eq => (Pred::Eq(x.min(y), x.max(y)), true),
            CmpKind::Ne => (Pred::Eq(x.min(y), x.max(y)), false),
        };
        (self.decide_constant(p), polarity)
    }

    fn decide_constant(&self, p: Pred) -> Built {
        match p {
            Pred::Lt(x, y) if x == y => Built::Decided(false),
            Pred::Eq(x, y) if x == y => Built::Decided(true),
            Pred::Lt(x, y) => match (self.value(x), self.value(y)) {
                (Some(a), Some(b)) => Built::Decided(a < b),
                _ => Built::Open(p),
            },
            Pred::Eq(x, y) => match (self.value(x), self.value(y)) {
                (Some(a), Some(b)) => Built::Decided(a == b),
                _ => Built::Open(p),
            },
        }
    }

    fn sel(&mut self, p: Pred, t: Id, e: Id) -> Result<Id, String> {
        if t == e {
            return Ok(t);
        }
        self.add(Node::Sel(p, t, e))
    }

    /// The node `id` on the paths where the predicates of `env` hold as it says: every select whose predicate `env`
    /// decides (or decides a converse of) is its arm, and everything above it is rebuilt.
    fn restrict(&mut self, id: Id, env: &BTreeMap<Pred, bool>, memo: &mut HashMap<Id, Id>) -> Result<Id, String> {
        if let Some(&r) = memo.get(&id) {
            return Ok(r);
        }
        let r = match self.nodes[id] {
            Node::In(_) | Node::Const(_) => id,
            Node::Arith(op, a, b) => {
                let (a, b) = (self.restrict(a, env, memo)?, self.restrict(b, env, memo)?);
                self.arith(op, a, b)?
            }
            Node::Sel(p, t, e) => {
                let built = match p {
                    Pred::Lt(x, y) => {
                        let (x, y) = (self.restrict(x, env, memo)?, self.restrict(y, env, memo)?);
                        self.decide_constant(Pred::Lt(x, y))
                    }
                    Pred::Eq(x, y) => {
                        let (x, y) = (self.restrict(x, env, memo)?, self.restrict(y, env, memo)?);
                        self.decide_constant(Pred::Eq(x.min(y), x.max(y)))
                    }
                };
                let known = match built {
                    Built::Decided(v) => Ok(v),
                    Built::Open(q) => decided(env, q).ok_or(q),
                };
                match known {
                    Ok(true) => self.restrict(t, env, memo)?,
                    Ok(false) => self.restrict(e, env, memo)?,
                    Err(q) => {
                        let (t, e) = (self.restrict(t, env, memo)?, self.restrict(e, env, memo)?);
                        self.sel(q, t, e)?
                    }
                }
            }
        };
        memo.insert(id, r);
        Ok(r)
    }

    /// The nodes `root` reads, itself included, in id order (which is dataflow order: a node is built after its
    /// operands).
    fn cone(&self, root: Id) -> BTreeSet<Id> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            match self.nodes[id] {
                Node::In(_) | Node::Const(_) => {}
                Node::Arith(_, a, b) => stack.extend([a, b]),
                Node::Sel(p, t, e) => {
                    let (x, y) = operands(p);
                    stack.extend([x, y, t, e]);
                }
            }
        }
        seen
    }

    /// The predicate of the earliest select in `root`'s cone that arithmetic or a compare reads, if any. Its own
    /// operands read no such select, since one would be earlier still.
    fn first_blocking(&self, root: Id) -> Option<Pred> {
        let cone = self.cone(root);
        let is_sel = |id: Id| matches!(self.nodes[id], Node::Sel(..));
        let mut blocking = BTreeSet::new();
        for &id in &cone {
            match self.nodes[id] {
                Node::Arith(_, a, b) => blocking.extend([a, b].into_iter().filter(|&s| is_sel(s))),
                Node::Sel(p, _, _) => {
                    let (x, y) = operands(p);
                    blocking.extend([x, y].into_iter().filter(|&s| is_sel(s)));
                }
                Node::In(_) | Node::Const(_) => {}
            }
        }
        blocking.into_iter().next().map(|s| match self.nodes[s] {
            Node::Sel(p, _, _) => p,
            _ => unreachable!("only selects block"),
        })
    }

    /// The Shannon expansion of `root` on every predicate whose select arithmetic or a compare reads, earliest first.
    fn expand(&mut self, root: Id, env: &mut BTreeMap<Pred, bool>) -> Result<Id, String> {
        let r = self.restrict(root, env, &mut HashMap::new())?;
        let Some(p) = self.first_blocking(r) else {
            return Ok(r);
        };
        env.insert(p, true);
        let t = self.expand(r, env);
        env.insert(p, false);
        let e = self.expand(r, env);
        env.remove(&p);
        self.sel(p, t?, e?)
    }

    /// When each value is ready, in the units of `op_delay`: an add or a subtract is a carry chain (9), a compare a
    /// subtraction and its overflow fix or an equality's OR tree, a select its mask and two bitwise ops.
    fn arrival(&self, id: Id, memo: &mut HashMap<Id, f64>) -> f64 {
        if let Some(&a) = memo.get(&id) {
            return a;
        }
        let a = match self.nodes[id] {
            Node::In(_) | Node::Const(_) => 0.0,
            Node::Arith(op, a, b) => {
                let wiring = matches!(op, AluOp::Shl | AluOp::Shr | AluOp::UShr) && self.value(b).is_some()
                    || matches!(op, AluOp::And | AluOp::Or | AluOp::Xor)
                        && (self.value(a).is_some() || self.value(b).is_some());
                let d = if wiring {
                    0.0
                } else {
                    match op {
                        AluOp::Add | AluOp::Sub => 9.0,
                        AluOp::Mul => 40.0,
                        AluOp::And | AluOp::Or => 0.6,
                        AluOp::Xor => 1.0,
                        AluOp::Shl | AluOp::Shr | AluOp::UShr => 4.0,
                    }
                };
                self.arrival(a, memo).max(self.arrival(b, memo)) + d
            }
            Node::Sel(p, t, e) => {
                let ready = self.pred_arrival(p, memo);
                ready.max(self.arrival(t, memo)).max(self.arrival(e, memo)) + 2.6
            }
        };
        memo.insert(id, a);
        a
    }

    fn pred_arrival(&self, p: Pred, memo: &mut HashMap<Id, f64>) -> f64 {
        let (x, y) = operands(p);
        let ready = self.arrival(x, memo).max(self.arrival(y, memo));
        ready
            + match p {
                Pred::Lt(..) => 9.0 + 2.6,
                Pred::Eq(..) => 1.0 + 6.0 * 0.6,
            }
    }

    /// `root`, a tree of selects over select-free predicates, rebuilt with its predicates tested in `order`: the first
    /// present at the root, each subtree restricted to its path.
    fn reorder(&mut self, root: Id, order: &[Pred], env: &mut BTreeMap<Pred, bool>) -> Result<Id, String> {
        let r = self.restrict(root, env, &mut HashMap::new())?;
        let present: BTreeSet<Pred> = self
            .cone(r)
            .into_iter()
            .filter_map(|id| match self.nodes[id] {
                Node::Sel(p, _, _) => Some(p),
                _ => None,
            })
            .collect();
        let Some(&p) = order.iter().find(|p| present.contains(p)) else {
            return Ok(r);
        };
        env.insert(p, true);
        let t = self.reorder(r, order, env);
        env.insert(p, false);
        let e = self.reorder(r, order, env);
        env.remove(&p);
        self.sel(p, t?, e?)
    }
}

fn operands(p: Pred) -> (Id, Id) {
    match p {
        Pred::Lt(x, y) | Pred::Eq(x, y) => (x, y),
    }
}

/// What the predicates `env` holds say of `p`: itself, or its converses on a total order (`y < x` or `x = y` true make
/// `x < y` false; `x < y` and `y < x` both false make `x = y` true).
fn decided(env: &BTreeMap<Pred, bool>, p: Pred) -> Option<bool> {
    if let Some(&v) = env.get(&p) {
        return Some(v);
    }
    let eq = |x: Id, y: Id| Pred::Eq(x.min(y), x.max(y));
    match p {
        Pred::Lt(x, y) => match (env.get(&Pred::Lt(y, x)), env.get(&eq(x, y))) {
            (Some(true), _) | (_, Some(true)) => Some(false),
            (Some(false), Some(false)) => Some(true),
            _ => None,
        },
        Pred::Eq(x, y) => match (env.get(&Pred::Lt(x, y)), env.get(&Pred::Lt(y, x))) {
            (Some(true), _) | (_, Some(true)) => Some(false),
            (Some(false), Some(false)) => Some(true),
            _ => None,
        },
    }
}

/// `df` with every select its arithmetic or compares read cofactored away and its tree of selects ordered latest
/// predicate first, at datapath width `width` (64 on silicon; narrower so our SAT can check the pass exhaustively).
/// Refused when `df` holds a cell with no combinational form (a functional unit, a memory or heap tile), or when the
/// expansion outgrows [`HOIST_NODE_LIMIT`].
pub fn hoist_selects(df: &DecisionFabric, width: u32) -> Result<DecisionFabric, String> {
    assert!((2..=64).contains(&width), "the datapath width is 2..=64 bits, not {width}");
    let mut g = Graph { nodes: Vec::new(), index: HashMap::new(), width };
    let mut vals: Vec<Id> = Vec::with_capacity(df.cells.len());
    let node = |g: &mut Graph, s: Src, vals: &[Id]| -> Result<Id, String> {
        match s {
            Src::In(k) => g.add(Node::In(k)),
            Src::Const(c) => g.konst(c),
            Src::Val(k) => Ok(vals[k]),
        }
    };
    for (i, cell) in df.cells.iter().enumerate() {
        let id = match *cell {
            LoopCell::Arith { op, a, b } => {
                let (a, b) = (node(&mut g, a, &vals)?, node(&mut g, b, &vals)?);
                g.arith(op, a, b)?
            }
            LoopCell::Select { cmp, ca, cb, t, e } => {
                let (x, y) = (node(&mut g, ca, &vals)?, node(&mut g, cb, &vals)?);
                let (t, e) = (node(&mut g, t, &vals)?, node(&mut g, e, &vals)?);
                match g.compare(cmp, x, y) {
                    (Built::Decided(v), polarity) => if v == polarity { t } else { e },
                    (Built::Open(p), true) => g.sel(p, t, e)?,
                    (Built::Open(p), false) => g.sel(p, e, t)?,
                }
            }
            ref other => return Err(format!("cell {i} ({other:?}) has no combinational form to hoist")),
        };
        vals.push(id);
    }
    let root = node(&mut g, df.result, &vals)?;
    let expanded = g.expand(root, &mut BTreeMap::new())?;

    let mut memo = HashMap::new();
    let mut order: Vec<(f64, Pred)> = g
        .cone(expanded)
        .into_iter()
        .filter_map(|id| match g.nodes[id] {
            Node::Sel(p, _, _) => Some(p),
            _ => None,
        })
        .collect::<BTreeSet<Pred>>()
        .into_iter()
        .map(|p| (g.pred_arrival(p, &mut memo), p))
        .collect();
    order.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let order: Vec<Pred> = order.into_iter().map(|(_, p)| p).collect();
    let tree = g.reorder(expanded, &order, &mut BTreeMap::new())?;

    let mut cells = Vec::new();
    let mut src_of: HashMap<Id, Src> = HashMap::new();
    for id in g.cone(tree) {
        let src = |n: Id| src_of[&n];
        let s = match g.nodes[id] {
            Node::In(k) => Src::In(k),
            Node::Const(c) => Src::Const(c),
            Node::Arith(op, a, b) => {
                cells.push(LoopCell::Arith { op, a: src(a), b: src(b) });
                Src::Val(cells.len() - 1)
            }
            Node::Sel(p, t, e) => {
                let (cmp, (x, y)) = match p {
                    Pred::Lt(..) => (CmpKind::Lt, operands(p)),
                    Pred::Eq(..) => (CmpKind::Eq, operands(p)),
                };
                cells.push(LoopCell::Select { cmp, ca: src(x), cb: src(y), t: src(t), e: src(e) });
                Src::Val(cells.len() - 1)
            }
        };
        src_of.insert(id, s);
    }
    Ok(DecisionFabric { n_in: df.n_in, cells, result: src_of[&tree], escaped_first_return: df.escaped_first_return })
}

/// Whether arithmetic or a compare of `df` reads a select's value: what [`hoist_selects`] leaves none of.
pub fn reads_a_select(df: &DecisionFabric) -> bool {
    let is_select = |s: Src| matches!(s, Src::Val(k) if matches!(df.cells[k], LoopCell::Select { .. }));
    df.cells.iter().any(|c| match *c {
        LoopCell::Arith { a, b, .. } => is_select(a) || is_select(b),
        LoopCell::Select { ca, cb, .. } => is_select(ca) || is_select(cb),
        _ => false,
    })
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/select_hoist/tests.rs"]
mod tests;
