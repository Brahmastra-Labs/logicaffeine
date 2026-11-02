//! The IR's meaning as an SMT-LIB obligation: two graphs compute the same values for every input, and for every meaning
//! of the ops neither pins down.
//!
//! [`eval`](super::eval) runs a graph on numbers; this module runs it on terms. Every tensor a step starts with is an
//! array of unknown values. A primitive that only moves values — a head slice, a row taken, rows gathered, a transpose,
//! a join, rows written into a table — is the index map it is. Every other primitive is a function nothing is known of
//! but that it is one: of its operands' values, the same for every op of the same shape. An operand is read whole or
//! as a part of every unit, a result lands in its own buffer or in rows or units of a table, and a fused op runs its
//! parts and keeps what it writes outside itself, exactly as the evaluator does. Every array is zero past its length,
//! so two arrays are equal exactly when their values are.
//!
//! Terms are shared: two graphs that compute a value by the same ops in the same way compute the same term, and that
//! equality needs no solver. Two terms that are one open function of operands that pair up are equal when the operands
//! are; what is left are the pairs that differ in more — a part read in place where a copy was read, a result stored
//! into a table's rows where a row write copied it — and each is a lemma asking that the two be equal arrays, every
//! subterm both share standing for any values. The indices a step reads rows by are assumed in range where the program
//! it was lifted from reads them, and so is every interval the program proves of an integer it starts with; the
//! rewritten graph's own reads must then be in range too.
//!
//! An [`Obligation`] is SMT-LIB 2 text, printed the same way every time from the same two graphs: the tensors compared,
//! and lemmas whose premises a solver finds satisfiable and whose goals make them unsatisfiable exactly when the rewrite
//! keeps the meaning. A proof checked once is pinned by the text's digest and re-checked without a solver until the
//! graphs change.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::{self, Write as _};

use super::front::TensorOpSpec;
use super::graph::{Access, Dst, Elem, Op, Role, RowSel, Sym, TensorGraph};
use crate::tensor_recognize::qwen::RowIndex;

/// An array's element sort.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Sort {
    Real,
    Int,
}

impl Sort {
    fn of(elem: Elem) -> Sort {
        match elem {
            Elem::Float => Sort::Real,
            Elem::Int => Sort::Int,
        }
    }

    fn smt(self) -> &'static str {
        match self {
            Sort::Real => "Real",
            Sort::Int => "Int",
        }
    }

    fn zero(self) -> &'static str {
        match self {
            Sort::Real => "0.0",
            Sort::Int => "0",
        }
    }

    fn tag(self) -> char {
        match self {
            Sort::Real => 'R',
            Sort::Int => 'I',
        }
    }
}

/// Where a store's first value lands: a fixed offset, or the start of the row a parameter's first value names (1-based)
/// in a table of rows `pitch` wide.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum At {
    Fixed(usize),
    Row { param: usize, pitch: usize },
}

/// Which value of its source each value of a picked array is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Map {
    /// Value `j` is `(j div width)·pitch + offset + j mod width`: a part of every unit. One unit is `pitch` 0.
    Units { pitch: usize, offset: usize, width: usize },
    /// Value `j` of a `rows × cols` matrix's transpose.
    Transpose { rows: usize, cols: usize },
    /// Value `j` of the row a parameter's first value names (1-based), rows `cols` wide.
    Row { param: usize, cols: usize },
    /// Value `j` of rows gathered by an index array: row `indices[j div cols]` (1-based), rows `cols` wide.
    Rows { indices: usize, cols: usize },
}

/// An op computed exactly, over the reals or the integers: a matrix product, an elementwise sum or product (its second
/// operand one row of `row` values combined with every row, where it is one), a scale by a factor's exact value.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Arith {
    Matmul { m: usize, k: usize, n: usize },
    Binary { mul: bool, row: Option<usize> },
    Scale { factor: u64 },
}

/// One term: an array of values, `len` of them meaningful and zero past them.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Node {
    /// A tensor's values when the step starts.
    Start { name: String, sort: Sort, len: usize },
    /// A table nothing has written yet.
    Zeros { sort: Sort, len: usize },
    /// An op's result left open: function `f` of its operands.
    Apply { f: usize, args: Vec<usize>, len: usize },
    /// Values of `src` an index map picks.
    Pick { src: usize, map: Map, len: usize },
    /// `table` with the first `n` values of `src` stored from `at` — one after another, or each unit's share in its part
    /// of every unit of the table (`(pitch, offset, width)`).
    Store { table: usize, src: usize, at: At, unit: Option<(usize, usize, usize)>, n: usize },
    /// An integer array's values as reals.
    Real { src: usize },
    /// An op's result computed exactly from its operands.
    Arith { op: Arith, args: Vec<usize>, sort: Sort, len: usize },
}

/// A function the meaning leaves open: its name and its signature.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Fun {
    name: String,
    args: Vec<Sort>,
    sort: Sort,
}

/// A fact a run needs: value `index` of array `node` lies in `lo..=hi`, and why.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Bound {
    node: usize,
    index: usize,
    lo: i64,
    hi: i64,
    why: String,
}

/// The terms of both graphs, shared.
#[derive(Default)]
struct Terms {
    nodes: Vec<Node>,
    sorts: Vec<Sort>,
    lens: Vec<usize>,
    ids: HashMap<Node, usize>,
    funs: Vec<Fun>,
    fun_ids: HashMap<Fun, usize>,
}

impl Terms {
    fn node(&mut self, node: Node) -> usize {
        if let Some(&id) = self.ids.get(&node) {
            return id;
        }
        let (sort, len) = match &node {
            Node::Start { sort, len, .. } | Node::Zeros { sort, len } => (*sort, *len),
            Node::Apply { f, len, .. } => (self.funs[*f].sort, *len),
            Node::Pick { src, len, .. } => (self.sorts[*src], *len),
            Node::Store { table, .. } => (self.sorts[*table], self.lens[*table]),
            Node::Real { src } => (Sort::Real, self.lens[*src]),
            Node::Arith { sort, len, .. } => (*sort, *len),
        };
        let id = self.nodes.len();
        self.nodes.push(node.clone());
        self.sorts.push(sort);
        self.lens.push(len);
        self.ids.insert(node, id);
        id
    }

    fn fun(&mut self, fun: Fun) -> usize {
        if let Some(&id) = self.fun_ids.get(&fun) {
            return id;
        }
        self.funs.push(fun.clone());
        self.fun_ids.insert(fun, self.funs.len() - 1);
        self.funs.len() - 1
    }

    /// `src`'s values from `offset`, `width` of every `pitch`, `len` in all; `src` itself where that is all of it.
    fn units(&mut self, src: usize, pitch: usize, offset: usize, width: usize, len: usize) -> usize {
        let contiguous = len <= width || pitch == width;
        if contiguous && offset == 0 && len == self.lens[src] {
            return src;
        }
        let pitch = if contiguous { 0 } else { pitch };
        let width = if contiguous { len } else { width };
        self.node(Node::Pick { src, map: Map::Units { pitch, offset, width }, len })
    }

    /// `src` as an array of `sort`: an integer array's values as reals where reals are read.
    fn as_sort(&mut self, src: usize, sort: Sort) -> usize {
        match (self.sorts[src], sort) {
            (Sort::Int, Sort::Real) => self.node(Node::Real { src }),
            _ => src,
        }
    }
}

/// Why two graphs give no obligation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SmtError {
    /// An op of one side reads a tensor that holds no value when it runs.
    Missing { side: &'static str, op: usize, tensor: String },
    /// An op of one side does what no run of the evaluator does.
    Malformed { side: &'static str, op: usize, why: String },
    /// A tensor the step carries or a statement observes, which the rewritten graph does not keep.
    Unkept { tensor: String },
}

impl fmt::Display for SmtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SmtError::Missing { side, op, tensor } => write!(f, "{side} op {op} reads `{tensor}`, which holds no value when it runs"),
            SmtError::Malformed { side, op, why } => write!(f, "{side} op {op}: {why}"),
            SmtError::Unkept { tensor } => write!(f, "`{tensor}` is carried or observed, and the rewritten graph does not keep it"),
        }
    }
}

/// One side's run: every tensor's term as it stands, and the facts its reads need.
struct Run<'g> {
    side: &'static str,
    /// Whether products, elementwise sums and products and scales are computed exactly rather than left open.
    exact: bool,
    g: &'g TensorGraph,
    env: BTreeMap<String, usize>,
    bounds: Vec<Bound>,
}

impl Run<'_> {
    fn missing(&self, op: usize, tensor: &str) -> SmtError {
        SmtError::Missing { side: self.side, op, tensor: tensor.to_string() }
    }

    fn malformed(&self, op: usize, why: impl Into<String>) -> SmtError {
        SmtError::Malformed { side: self.side, op, why: why.into() }
    }

    fn len_of(&self, at: usize, t: &str) -> Result<usize, SmtError> {
        self.g.tensors.get(t).map(|t| t.len()).ok_or_else(|| self.missing(at, t))
    }

    fn sort_of(&self, at: usize, t: &str) -> Result<Sort, SmtError> {
        self.g.tensors.get(t).map(|t| Sort::of(t.elem)).ok_or_else(|| self.missing(at, t))
    }

    fn value(&self, at: usize, t: &str) -> Result<usize, SmtError> {
        self.env.get(t).copied().ok_or_else(|| self.missing(at, t))
    }

    /// The row a selector names, as where its row's values start in a table of rows `pitch` wide; the bounds it needs
    /// for a run's `n` values landing there to stay inside the table's `len`.
    fn row(&mut self, terms: &Terms, at: usize, sel: &RowSel, pitch: usize, last: usize, len: usize, what: &str) -> Result<At, SmtError> {
        let hi = if pitch == 0 || last >= len { 0 } else { ((len - 1 - last) / pitch) as i64 + 1 };
        if hi < 1 {
            return Err(self.malformed(at, format!("no row of {what} holds {} values", last + 1)));
        }
        match sel {
            RowSel::Fixed(r) if *r >= 1 && (*r as i64) <= hi => Ok(At::Fixed((r - 1) * pitch)),
            RowSel::Fixed(r) => Err(self.malformed(at, format!("row {r} of {what}"))),
            RowSel::Param(p) => {
                let param = self.value(at, p)?;
                if terms.lens[param] != 1 || terms.sorts[param] != Sort::Int {
                    return Err(self.malformed(at, format!("`{p}` names the row of {what} and is not one Int")));
                }
                self.bounds.push(Bound { node: param, index: 0, lo: 1, hi, why: format!("`{p}` names a row of {what}") });
                Ok(At::Row { param, pitch })
            }
        }
    }

    /// Operand `k` of `op`, read as its access says.
    fn read(&self, terms: &mut Terms, at: usize, op: &Op, k: usize) -> Result<usize, SmtError> {
        let whole = self.value(at, &op.ins[k])?;
        match op.access.get(k).copied().unwrap_or(Access::Whole) {
            Access::Whole => Ok(whole),
            Access::Strided(s) => {
                let len = terms.lens[whole];
                if s.pitch == 0 || s.offset + s.width > s.pitch || len % s.pitch != 0 {
                    return Err(self.malformed(at, format!("{len} values of `{}` are no whole units of {} to read {} from {} of", op.ins[k], s.pitch, s.width, s.offset)));
                }
                Ok(terms.units(whole, s.pitch, s.offset, s.width, len / s.pitch * s.width))
            }
        }
    }

    /// Store `result` where `dst` says.
    fn land(&mut self, terms: &mut Terms, at: usize, dst: &Dst, result: usize) -> Result<(), SmtError> {
        if let Some(name) = dst.own() {
            self.env.insert(name.to_string(), result);
            return Ok(());
        }
        let table = match self.env.get(&dst.buf) {
            Some(&t) => t,
            None => {
                let (sort, len) = (self.sort_of(at, &dst.buf)?, self.len_of(at, &dst.buf)?);
                terms.node(Node::Zeros { sort, len })
            }
        };
        if terms.sorts[table] != terms.sorts[result] {
            return Err(self.malformed(at, format!("a result of one element type landing in `{}` of another", dst.buf)));
        }
        let (n, len) = (terms.lens[result], terms.lens[table]);
        let last = match dst.unit {
            None => n.saturating_sub(1),
            Some(u) => {
                if u.width == 0 || n % u.width != 0 || u.offset + u.width > u.pitch {
                    return Err(self.malformed(at, format!("{n} values in shares of {} at {} of units of {}", u.width, u.offset, u.pitch)));
                }
                (n / u.width).saturating_sub(1) * u.pitch + u.offset + u.width - 1
            }
        };
        let place = match &dst.row {
            Some((sel, pitch)) => self.row(terms, at, sel, *pitch, last, len, &format!("`{}`", dst.buf))?,
            None if last < len || n == 0 => At::Fixed(0),
            None => return Err(self.malformed(at, format!("{n} values past the {len} of `{}`", dst.buf))),
        };
        let unit = dst.unit.map(|u| (u.pitch, u.offset, u.width));
        let stored = terms.node(Node::Store { table, src: result, at: place, unit, n });
        self.env.insert(dst.buf.clone(), stored);
        Ok(())
    }

    fn op(&mut self, terms: &mut Terms, at: usize, op: &Op) -> Result<(), SmtError> {
        match &op.sym {
            Sym::Fused(_) => {
                let before: BTreeSet<String> = self.env.keys().cloned().collect();
                self.parts(terms, at, op)?;
                let outlive: BTreeSet<&str> = op.writes().into_iter().collect();
                self.env.retain(|k, _| before.contains(k) || outlive.contains(k.as_str()));
                Ok(())
            }
            Sym::View { offset } => {
                let src = self.read(terms, at, op, 0)?;
                let len = self.len_of(at, &op.out)?;
                if offset + len > terms.lens[src] {
                    return Err(self.malformed(at, format!("a view of {len} values from {offset} into {} values", terms.lens[src])));
                }
                let v = terms.units(src, 0, *offset, len, len);
                self.land(terms, at, &op.dst, v)
            }
            Sym::Quantize(_) => {
                let x = self.read(terms, at, op, 0)?;
                self.land(terms, at, &op.dst, x)
            }
            Sym::Prim(spec) => {
                let ins: Vec<usize> = (0..op.ins.len()).map(|k| self.read(terms, at, op, k)).collect::<Result<_, _>>()?;
                let (result, state) = self.prim(terms, at, op, spec, &ins)?;
                for (k, v) in state {
                    self.env.insert(op.ins[k].clone(), v);
                }
                match result {
                    Some(r) => self.land(terms, at, &op.dst, r),
                    None => Ok(()),
                }
            }
        }
    }

    /// A fused op's parts in order, a member a rule built running its own parts in turn.
    fn parts(&mut self, terms: &mut Terms, at: usize, op: &Op) -> Result<(), SmtError> {
        let Sym::Fused(f) = &op.sym else { return self.op(terms, at, op) };
        for part in &f.parts {
            match part.sym {
                Sym::Fused(_) => self.parts(terms, at, part)?,
                _ => self.op(terms, at, part)?,
            }
        }
        Ok(())
    }

    /// The row a row index names: a literal, or the one Int operand `k` names.
    fn index(&mut self, terms: &Terms, at: usize, i: &RowIndex, op: &Op, k: usize, pitch: usize, last: usize, len: usize, what: &str) -> Result<At, SmtError> {
        match i {
            RowIndex::Literal(n) if *n >= 1 => self.row(terms, at, &RowSel::Fixed(*n as usize), pitch, last, len, what),
            RowIndex::Literal(n) => Err(self.malformed(at, format!("row {n} of {what}"))),
            RowIndex::Named(_) => {
                let name = op.ins.get(k).ok_or_else(|| self.malformed(at, "a named row with no operand naming it"))?.clone();
                self.row(terms, at, &RowSel::Param(name), pitch, last, len, what)
            }
        }
    }

    /// A primitive's result and the operands it writes in place: the index map of an op that moves values, a function
    /// left open of every other.
    #[allow(clippy::type_complexity)]
    fn prim(&mut self, terms: &mut Terms, at: usize, op: &Op, spec: &TensorOpSpec, ins: &[usize]) -> Result<(Option<usize>, Vec<(usize, usize)>), SmtError> {
        let name = |k: usize| format!("`{}`", op.ins[k]);
        match spec {
            TensorOpSpec::HeadSlice(s) => {
                let len = s.n / s.every.max(1) * s.width;
                Ok((Some(terms.units(ins[0], s.every, s.from, s.width, len)), Vec::new()))
            }
            TensorOpSpec::Transpose(t) => {
                let picked = terms.node(Node::Pick { src: ins[0], map: Map::Transpose { rows: t.rows, cols: t.cols }, len: t.rows * t.cols });
                Ok((Some(picked), Vec::new()))
            }
            TensorOpSpec::RowGather(r) => {
                let what = format!("{}'s {} rows", name(0), r.rows);
                let place = self.index(terms, at, &r.index, op, 1, r.cols, r.cols - 1, r.rows * r.cols, &what)?;
                let picked = match place {
                    At::Fixed(from) => terms.units(ins[0], 0, from, r.cols, r.cols),
                    At::Row { param, .. } => terms.node(Node::Pick { src: ins[0], map: Map::Row { param, cols: r.cols }, len: r.cols }),
                };
                Ok((Some(picked), Vec::new()))
            }
            TensorOpSpec::RowsGather(r) => {
                let indices = ins[1];
                if terms.sorts[indices] != Sort::Int || terms.lens[indices] < r.count {
                    return Err(self.malformed(at, format!("{} holds no {} Int row indices", name(1), r.count)));
                }
                for index in 0..r.count {
                    self.bounds.push(Bound { node: indices, index, lo: 1, hi: r.rows as i64, why: format!("{} names rows of {}", name(1), name(0)) });
                }
                let picked = terms.node(Node::Pick { src: ins[0], map: Map::Rows { indices, cols: r.cols }, len: r.count * r.cols });
                Ok((Some(picked), Vec::new()))
            }
            TensorOpSpec::Join(j) => {
                let (ha, hb) = (j.wa / j.heads.max(1), j.wb / j.heads.max(1));
                let units = j.rows * j.heads;
                let sort = if terms.sorts[ins[0]] == Sort::Int && terms.sorts[ins[1]] == Sort::Int { Sort::Int } else { Sort::Real };
                let (a, b) = (terms.as_sort(ins[0], sort), terms.as_sort(ins[1], sort));
                let zeros = terms.node(Node::Zeros { sort, len: units * (ha + hb) });
                let with_a = terms.node(Node::Store { table: zeros, src: a, at: At::Fixed(0), unit: Some((ha + hb, 0, ha)), n: units * ha });
                let joined = terms.node(Node::Store { table: with_a, src: b, at: At::Fixed(0), unit: Some((ha + hb, ha, hb)), n: units * hb });
                Ok((Some(joined), Vec::new()))
            }
            TensorOpSpec::SetRow(r) => {
                let n = r.cols * r.count;
                let what = format!("{}'s {} rows", name(0), r.rows);
                let place = self.index(terms, at, &r.index, op, 2, r.cols, n.saturating_sub(1), r.rows * r.cols, &what)?;
                let table = ins[0];
                if terms.sorts[table] == Sort::Int && terms.sorts[ins[1]] == Sort::Real {
                    return Err(self.malformed(at, format!("Reals written into {}, a table of Ints", name(0))));
                }
                let src = terms.as_sort(ins[1], terms.sorts[table]);
                let stored = terms.node(Node::Store { table, src, at: place, unit: None, n });
                Ok((None, vec![(0, stored)]))
            }
            TensorOpSpec::Matmul(_) | TensorOpSpec::Elementwise(_) | TensorOpSpec::Scale(_) if self.exact => {
                let op_ = match spec {
                    TensorOpSpec::Matmul(m) => Arith::Matmul { m: m.m, k: m.k, n: m.n },
                    TensorOpSpec::Elementwise(e) => Arith::Binary { mul: e.op == crate::tensor_recognize::ElementwiseOp::Mul, row: e.row },
                    TensorOpSpec::Scale(s) => Arith::Scale { factor: s.factor.get().to_bits() },
                    _ => unreachable!("the arms above"),
                };
                let (sort, len) = (self.sort_of(at, &op.out)?, self.len_of(at, &op.out)?);
                let args: Vec<usize> = ins.iter().map(|&i| terms.as_sort(i, sort)).collect();
                Ok((Some(terms.node(Node::Arith { op: op_, args, sort, len })), Vec::new()))
            }
            _ => {
                let args: Vec<Sort> = ins.iter().map(|&i| terms.sorts[i]).collect();
                let shape = shape_of(spec);
                let open = |terms: &mut Terms, out: usize, sort: Sort, len: usize| {
                    let f = terms.fun(Fun { name: format!("{}!{}!{}", spec.kind(), shape, out), args: args.clone(), sort });
                    terms.node(Node::Apply { f, args: ins.to_vec(), len })
                };
                let mut state = Vec::new();
                for (n, k) in spec.state_inputs().into_iter().enumerate() {
                    let (sort, len) = (terms.sorts[ins[k]], terms.lens[ins[k]]);
                    state.push((k, open(terms, n + 1, sort, len)));
                }
                let (sort, len) = (self.sort_of(at, &op.out)?, self.len_of(at, &op.out)?);
                Ok((Some(open(terms, 0, sort, len)), state))
            }
        }
    }
}

/// A primitive's shape: the first 8 bytes of the sha256 of its spec without the symbols it names, so two ops of one
/// shape are one function of their operands.
fn shape_of(spec: &TensorOpSpec) -> String {
    let text = format!("{spec:?}");
    let mut bare = String::new();
    let mut rest = text.as_str();
    while let Some(i) = rest.find("Symbol(") {
        bare.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(')').map_or(rest.len(), |e| e + 1);
        rest = &rest[end..];
    }
    bare.push_str(rest);
    hex(&crate::crypto_accel::sha256(bare.as_bytes())[..8])
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// One step of a proof a solver checks: SMT-LIB 2 premises — the terms it reads and the facts it assumes — and a goal
/// asserting that two terms differ, or that a read strays out of range. The premises are satisfiable, so nothing follows
/// from them vacuously; with the goal they are not, exactly when the step holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lemma {
    /// Declarations, definitions and the asserted facts.
    pub premises: String,
    /// The one assert of the goal.
    pub goal: String,
}

impl Lemma {
    /// The premises and the goal, as one SMT-LIB 2 script without commands.
    pub fn text(&self) -> String {
        format!("{}{}", self.premises, self.goal)
    }
}

/// What two graphs give a solver. A tensor both keep that the two graphs compute by one term needs no proof; one they
/// compute by different terms is equal when the open functions are applied to equal operands, down to the pairs of terms
/// that differ in more than their operands — an index map, a store, a value only one side computes. Each such pair is a
/// lemma, its terms defined down to the subterms both share, which stand for any values; so is every read of the
/// rewritten graph that its facts must keep in range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Obligation {
    /// What the proof checks beyond the terms both graphs share.
    pub lemmas: Vec<Lemma>,
    /// The tensors compared: every one both graphs keep that an op of either writes.
    pub compared: usize,
    /// Of those, the ones both graphs compute by the same term.
    pub identical: usize,
    /// The facts the lemmas assume, each saying what states it.
    pub assumed: Vec<String>,
    /// The tensors compared, by name, each marked computed alike or not.
    header: String,
}

impl Obligation {
    /// The tensors compared and every lemma, as one text: what a proof of it is a proof of.
    pub fn text(&self) -> String {
        let mut t = self.header.clone();
        for l in &self.lemmas {
            t.push_str(&l.text());
        }
        t
    }

    /// The text's sha256, as 64 lowercase hex digits.
    pub fn digest(&self) -> String {
        hex(&crate::crypto_accel::sha256(self.text().as_bytes()))
    }
}

fn run<'g>(terms: &mut Terms, side: &'static str, exact: bool, g: &'g TensorGraph) -> Result<Run<'g>, SmtError> {
    let mut r = Run { side, exact, g, env: BTreeMap::new(), bounds: Vec::new() };
    for t in g.tensors.values().filter(|t| t.role != Role::Activation || t.fill.is_some()) {
        let start = terms.node(Node::Start { name: t.name.clone(), sort: Sort::of(t.elem), len: t.len() });
        r.env.insert(t.name.clone(), start);
    }
    for (at, op) in g.ops.iter().enumerate() {
        r.op(terms, at, op)?;
    }
    Ok(r)
}

/// The obligation that `rewritten` means what `reference` means: for every input and every meaning of the ops the
/// meaning leaves open, wherever the reference's reads are in range, the rewritten graph's are too and every tensor both
/// keep holds the same values. A tensor of one name is the same tensor in both.
pub fn obligation(reference: &TensorGraph, rewritten: &TensorGraph) -> Result<Obligation, SmtError> {
    obligation_under(reference, rewritten, false)
}

/// The obligation that two programs compute every tensor both keep alike when their products, elementwise sums and
/// products and scales are computed exactly — an algebraic identity at the shapes the programs declare. Every other op
/// is left open as [`obligation`] leaves it.
pub fn identity(lhs: &TensorGraph, rhs: &TensorGraph) -> Result<Obligation, SmtError> {
    obligation_under(lhs, rhs, true)
}

fn obligation_under(reference: &TensorGraph, rewritten: &TensorGraph, exact: bool) -> Result<Obligation, SmtError> {
    let mut terms = Terms::default();
    let a = run(&mut terms, "reference", exact, reference)?;
    let b = run(&mut terms, "rewritten", exact, rewritten)?;
    for t in reference.tensors.values().filter(|t| t.role == Role::State || t.observed) {
        if a.env.contains_key(&t.name) && !b.env.contains_key(&t.name) {
            return Err(SmtError::Unkept { tensor: t.name.clone() });
        }
    }
    let starts: BTreeMap<&str, usize> = reference
        .tensors
        .values()
        .filter_map(|t| terms.ids.get(&Node::Start { name: t.name.clone(), sort: Sort::of(t.elem), len: t.len() }).map(|&id| (t.name.as_str(), id)))
        .collect();
    let goals: Vec<(&str, usize, usize)> = b
        .env
        .iter()
        .filter_map(|(name, &y)| a.env.get(name).map(|&x| (name.as_str(), x, y)))
        .filter(|(name, x, y)| starts.get(name) != Some(x) || starts.get(name) != Some(y))
        .collect();
    let key = |b: &Bound| (b.node, b.index, b.lo, b.hi);
    let assumed_bounds: BTreeSet<(usize, usize, i64, i64)> = a.bounds.iter().map(key).collect();
    let mut owed: Vec<&Bound> = b.bounds.iter().filter(|x| !assumed_bounds.contains(&key(x))).collect();
    owed.sort();
    owed.dedup_by_key(|b| key(b));
    let (alike, differ): (Vec<_>, Vec<_>) = goals.iter().partition(|(_, x, y)| x == y);

    let mut header = String::from("; tensor_graph::smt — the rewritten graph means what the reference means\n");
    for (name, x, y) in &goals {
        let _ = writeln!(header, "; {} `{name}`", if x == y { "one term on both sides:" } else { "compared:" });
    }

    // The facts the step may assume: the range of every integer it starts with that the program proves one of, and every
    // read the reference makes in range.
    let mut facts: Vec<(usize, usize, i64, i64, String)> = Vec::new();
    for t in reference.tensors.values() {
        let (Some((lo, hi)), Some(&node)) = (t.range, starts.get(t.name.as_str())) else { continue };
        if t.elem != Elem::Int || (lo == i64::MIN && hi == i64::MAX) {
            continue;
        }
        for index in 0..t.len() {
            facts.push((node, index, lo, hi, format!("`{}` holds [{lo}, {hi}], the interval the program proves", t.name)));
        }
    }
    let mut known = BTreeSet::new();
    for bound in &a.bounds {
        if known.insert(key(bound)) {
            facts.push((bound.node, bound.index, bound.lo, bound.hi, format!("{}, as the reference reads it", bound.why)));
        }
    }

    // The lemmas, lowest first: each pair of terms the goals still come down to through the open functions, once every
    // pair proven before it is one term. A lemma assumes only what the lemmas before it prove.
    let mut lemmas = Vec::new();
    let mut assumed: Vec<String> = Vec::new();
    let mut proven: Vec<(usize, usize)> = Vec::new();
    loop {
        let class = classes(&terms, &proven);
        let mut pairs: Vec<(usize, usize, &str)> = Vec::new();
        let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();
        let mut work: Vec<(usize, usize, &str)> = differ.iter().rev().map(|(name, x, y)| (*x, *y, *name)).collect();
        while let Some((x, y, from)) = work.pop() {
            if class[x] == class[y] || !seen.insert((x, y)) {
                continue;
            }
            match congruent(&terms, x, y) {
                Some(kids) => work.extend(kids.into_iter().rev().map(|(cx, cy)| (cx, cy, from))),
                None => pairs.push((x, y, from)),
            }
        }
        let Some(&(x, y, from)) = pairs.iter().min_by_key(|(x, y, _)| (*x.max(y), *x.min(y))) else { break };
        assert!(!proven.contains(&(x.min(y), x.max(y))), "a proven pair is one class: `{from}` chosen twice");
        let (premises, cited) = premises_of(&terms, &class, &[x, y], &[x, y], &facts, &format!("`{from}` is one term on both sides"));
        let (lx, ly) = (cited.local[&x], cited.local[&y]);
        // Terms computed by exact arithmetic are compared value by value at each index, each a polynomial in values the
        // step starts with; any other two at one index the solver chooses, which index maps over a symbolic row need.
        let goal = if cited.arithmetic {
            let len = terms.lens[x].max(terms.lens[y]);
            let each: Vec<String> = (0..len).map(|i| format!("(= (select |n{lx}| {i}) (select |n{ly}| {i}))")).collect();
            format!("(assert (not (and true {})))\n", each.join(" "))
        } else {
            format!("(assert (not (= |n{lx}| |n{ly}|)))\n")
        };
        lemmas.push(Lemma { premises, goal });
        assumed.extend(cited.assumed);
        proven.push((x.min(y), x.max(y)));
    }
    if !owed.is_empty() {
        let nodes: Vec<usize> = owed.iter().map(|b| b.node).collect();
        let class = classes(&terms, &proven);
        let (mut premises, cited) = premises_of(&terms, &class, &nodes, &[], &facts, "the rewritten graph reads in range");
        for bound in &owed {
            let _ = writeln!(premises, "; owed: {}, as the rewritten graph reads it", bound.why);
        }
        let reads: Vec<String> = owed.iter().map(|b| within(cited.local[&b.node], b.index, b.lo, b.hi)).collect();
        lemmas.push(Lemma { premises, goal: format!("(assert (not (and true {})))\n", reads.join(" ")) });
        assumed.extend(cited.assumed);
    }
    let mut once = BTreeSet::new();
    assumed.retain(|a| once.insert(a.clone()));
    Ok(Obligation { lemmas, compared: goals.len(), identical: alike.len(), assumed, header })
}

/// Whether `x` and `y` are one open function, or one coercion, applied to operands that pair up: the pairs whose equality
/// makes them equal. Any other two terms are compared whole.
fn congruent(terms: &Terms, x: usize, y: usize) -> Option<Vec<(usize, usize)>> {
    match (&terms.nodes[x], &terms.nodes[y]) {
        (Node::Apply { f: f1, args: a1, len: l1 }, Node::Apply { f: f2, args: a2, len: l2 }) if f1 == f2 && l1 == l2 && a1.len() == a2.len() => {
            Some(a1.iter().copied().zip(a2.iter().copied()).collect())
        }
        (Node::Real { src: s1 }, Node::Real { src: s2 }) => Some(vec![(*s1, *s2)]),
        _ => None,
    }
}

/// `node` with each term it reads renamed by `f`.
fn renamed(node: &Node, f: &dyn Fn(usize) -> usize) -> Node {
    let at = |a: &At| match a {
        At::Fixed(s) => At::Fixed(*s),
        At::Row { param, pitch } => At::Row { param: f(*param), pitch: *pitch },
    };
    match node {
        Node::Start { .. } | Node::Zeros { .. } => node.clone(),
        Node::Apply { f: fun, args, len } => Node::Apply { f: *fun, args: args.iter().map(|&a| f(a)).collect(), len: *len },
        Node::Pick { src, map, len } => {
            let map = match map {
                Map::Row { param, cols } => Map::Row { param: f(*param), cols: *cols },
                Map::Rows { indices, cols } => Map::Rows { indices: f(*indices), cols: *cols },
                Map::Units { .. } | Map::Transpose { .. } => map.clone(),
            };
            Node::Pick { src: f(*src), map, len: *len }
        }
        Node::Store { table, src, at: place, unit, n } => Node::Store { table: f(*table), src: f(*src), at: at(place), unit: *unit, n: *n },
        Node::Real { src } => Node::Real { src: f(*src) },
        Node::Arith { op, args, sort, len } => Node::Arith { op: op.clone(), args: args.iter().map(|&a| f(a)).collect(), sort: *sort, len: *len },
    }
}

/// Each term's class once every pair of `proven` is one term: the least term of the class, where two terms are one
/// class when a proven pair joins them, when one constructor reads terms of the same classes, or when a chain of those
/// leads from one to the other. Terms are taken in the order they were made, so a term's operands have their classes
/// before it does.
fn classes(terms: &Terms, proven: &[(usize, usize)]) -> Vec<usize> {
    fn root(parent: &[usize], mut x: usize) -> usize {
        while parent[x] != x {
            x = parent[x];
        }
        x
    }
    fn join(parent: &mut [usize], a: usize, b: usize) {
        let (ra, rb) = (root(parent, a), root(parent, b));
        parent[ra.max(rb)] = ra.min(rb);
    }
    let mut parent: Vec<usize> = (0..terms.nodes.len()).collect();
    let mut partners: HashMap<usize, Vec<usize>> = HashMap::new();
    for &(a, b) in proven {
        partners.entry(a.max(b)).or_default().push(a.min(b));
    }
    let mut seen: HashMap<Node, usize> = HashMap::new();
    for (n, node) in terms.nodes.iter().enumerate() {
        let signature = renamed(node, &|k| root(&parent, k));
        match seen.get(&signature) {
            Some(&m) => join(&mut parent, n, m),
            None => {
                seen.insert(signature, n);
            }
        }
        for &p in partners.get(&n).into_iter().flatten() {
            join(&mut parent, n, p);
        }
    }
    (0..terms.nodes.len()).map(|n| root(&parent, n)).collect()
}

/// Every class the terms `from` reaches belong to, its own included.
fn reach(terms: &Terms, class: &[usize], from: usize) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut stack = vec![from];
    while let Some(n) = stack.pop() {
        if seen.insert(n) {
            out.insert(class[n]);
            stack.extend(children(&terms.nodes[n]));
        }
    }
    out
}

/// A lemma's terms as it numbers them, the facts it assumes, and whether it defines a term by exact arithmetic.
struct Cited {
    local: BTreeMap<usize, usize>,
    assumed: Vec<String>,
    arithmetic: bool,
}

/// The premises of a lemma about `roots`: each root defined down to the terms of a class more than one of `split` reaches
/// — each such class one constant for any values, zero past its length, as the lemmas before it prove its terms one —
/// and every fact about a term it reads.
fn premises_of(terms: &Terms, class: &[usize], roots: &[usize], split: &[usize], facts: &[(usize, usize, i64, i64, String)], what: &str) -> (String, Cited) {
    let reaches: Vec<BTreeSet<usize>> = split.iter().map(|&r| reach(terms, class, r)).collect();
    let shared = |n: usize| reaches.iter().filter(|r| r.contains(&class[n])).count() > 1;
    let mut defined = BTreeSet::new();
    let mut opaque: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    let mut stack: Vec<usize> = roots.to_vec();
    let mut visited = BTreeSet::new();
    while let Some(n) = stack.pop() {
        if !visited.insert(n) {
            continue;
        }
        let leaf = matches!(terms.nodes[n], Node::Start { .. } | Node::Zeros { .. });
        if !roots.contains(&n) && !leaf && (shared(n) || split.is_empty()) {
            opaque.entry(class[n]).or_default().insert(n);
        } else {
            defined.insert(n);
            stack.extend(children(&terms.nodes[n]));
        }
    }
    // One name a defined term, one a class of opaque terms: the first of each in the order the terms were made.
    let mut names: BTreeMap<usize, usize> = BTreeMap::new();
    for &n in &defined {
        names.insert(n, n);
    }
    for members in opaque.values() {
        let first = *members.iter().next().expect("a class has a member");
        for &m in members {
            names.insert(m, first);
        }
    }
    let shown: BTreeSet<usize> = names.values().copied().collect();
    let order: BTreeMap<usize, usize> = shown.iter().enumerate().map(|(k, &n)| (n, k)).collect();
    let local: BTreeMap<usize, usize> = names.iter().map(|(&n, first)| (n, order[first])).collect();
    let opaque_names: BTreeSet<usize> = opaque.values().map(|m| *m.iter().next().expect("a member")).collect();
    let mut text = format!("; lemma: {what}\n(set-logic ALL)\n");
    let mut declared: BTreeMap<&str, Sort> = BTreeMap::new();
    let mut funs: BTreeSet<usize> = BTreeSet::new();
    for &n in &defined {
        match &terms.nodes[n] {
            Node::Start { name, sort, .. } => {
                declared.insert(name, *sort);
            }
            Node::Apply { f, .. } => {
                funs.insert(*f);
            }
            _ => {}
        }
    }
    for (name, sort) in &declared {
        let _ = writeln!(text, "(declare-const |in!{name}| (Array Int {}))", sort.smt());
    }
    for &n in &opaque_names {
        let _ = writeln!(text, "(declare-const |o!{}| (Array Int {}))", local[&n], terms.sorts[n].smt());
    }
    let mut fun_lines: Vec<String> = funs
        .iter()
        .map(|&f| {
            let fun = &terms.funs[f];
            let args: Vec<String> = fun.args.iter().map(|s| format!("(Array Int {})", s.smt())).collect();
            let sig: String = fun.args.iter().map(|s| s.tag()).collect();
            format!("(declare-fun |f!{}!{sig}| ({}) (Array Int {}))", fun.name, args.join(" "), fun.sort.smt())
        })
        .collect();
    fun_lines.sort();
    for line in fun_lines {
        text.push_str(&line);
        text.push('\n');
    }
    for &n in &shown {
        let body = if opaque_names.contains(&n) {
            let (sort, len) = (terms.sorts[n], terms.lens[n]);
            format!("(lambda ((j Int)) (ite (and (<= 0 j) (< j {len})) (select |o!{}| j) {}))", local[&n], sort.zero())
        } else {
            expr(terms, n, &local)
        };
        let _ = writeln!(text, "(define-fun |n{}| () (Array Int {}) {body})", local[&n], terms.sorts[n].smt());
    }
    let mut assumed = Vec::new();
    for (node, index, lo, hi, why) in facts.iter().filter(|f| local.contains_key(&f.0)) {
        let _ = writeln!(text, "; assumed: {why}\n(assert {})", within(local[node], *index, *lo, *hi));
        assumed.push(why.clone());
    }
    let arithmetic = defined.iter().any(|&n| matches!(terms.nodes[n], Node::Arith { .. }));
    (text, Cited { local, assumed, arithmetic })
}

fn children(node: &Node) -> Vec<usize> {
    match node {
        Node::Start { .. } | Node::Zeros { .. } => Vec::new(),
        Node::Apply { args, .. } => args.clone(),
        Node::Pick { src, map, .. } => {
            let mut c = vec![*src];
            match map {
                Map::Row { param, .. } => c.push(*param),
                Map::Rows { indices, .. } => c.push(*indices),
                Map::Units { .. } | Map::Transpose { .. } => {}
            }
            c
        }
        Node::Store { table, src, at, .. } => {
            let mut c = vec![*table, *src];
            if let At::Row { param, .. } = at {
                c.push(*param);
            }
            c
        }
        Node::Real { src } => vec![*src],
        Node::Arith { args, .. } => args.clone(),
    }
}

fn within(node: usize, index: usize, lo: i64, hi: i64) -> String {
    format!("(and (<= {} (select |n{node}| {index})) (<= (select |n{node}| {index}) {}))", int(lo), int(hi))
}

fn int(v: i64) -> String {
    if v < 0 {
        format!("(- {})", v.unsigned_abs())
    } else {
        v.to_string()
    }
}

/// Term `n` as an SMT-LIB array expression, zero past its length.
fn expr(terms: &Terms, n: usize, local: &BTreeMap<usize, usize>) -> String {
    let (sort, len) = (terms.sorts[n], terms.lens[n]);
    let id = |x: &usize| local[x];
    let clamp = |body: String| format!("(lambda ((j Int)) (ite (and (<= 0 j) (< j {len})) {body} {}))", sort.zero());
    match &terms.nodes[n] {
        Node::Start { name, .. } => clamp(format!("(select |in!{name}| j)")),
        Node::Zeros { .. } => format!("((as const (Array Int {})) {})", sort.smt(), sort.zero()),
        Node::Apply { f, args, .. } => {
            let fun = &terms.funs[*f];
            let sig: String = fun.args.iter().map(|s| s.tag()).collect();
            let call = if args.is_empty() {
                format!("|f!{}!{sig}|", fun.name)
            } else {
                format!("(|f!{}!{sig}| {})", fun.name, args.iter().map(|a| format!("|n{}|", id(a))).collect::<Vec<_>>().join(" "))
            };
            clamp(format!("(select {call} j)"))
        }
        Node::Pick { src, map, .. } => {
            let index = match map {
                Map::Units { pitch: 0, offset, .. } => format!("(+ j {offset})"),
                Map::Units { pitch, offset, width } => format!("(+ (* (div j {width}) {pitch}) {offset} (mod j {width}))"),
                Map::Transpose { rows, cols } => format!("(+ (* (mod j {rows}) {cols}) (div j {rows}))"),
                Map::Row { param, cols } => format!("(+ (* (- (select |n{}| 0) 1) {cols}) j)", id(param)),
                Map::Rows { indices, cols } => format!("(+ (* (- (select |n{}| (div j {cols})) 1) {cols}) (mod j {cols}))", id(indices)),
            };
            clamp(format!("(select |n{}| {index})", id(src)))
        }
        Node::Store { table, src, at, unit, n: count } => {
            let start = match at {
                At::Fixed(s) => s.to_string(),
                At::Row { param, pitch } => format!("(* (- (select |n{}| 0) 1) {pitch})", id(param)),
            };
            let d = format!("(- j {start})");
            let (inside, from) = match unit {
                None => (format!("(and (<= 0 {d}) (< {d} {count}))"), d.clone()),
                Some((pitch, offset, width)) => (
                    format!("(and (<= 0 {d}) (< (div {d} {pitch}) {}) (<= {offset} (mod {d} {pitch})) (< (mod {d} {pitch}) {}))", count / width, offset + width),
                    format!("(+ (* (div {d} {pitch}) {width}) (- (mod {d} {pitch}) {offset}))"),
                ),
            };
            clamp(format!("(ite {inside} (select |n{}| {from}) (select |n{}| j))", id(src), id(table)))
        }
        Node::Real { src } => clamp(format!("(to_real (select |n{}| j))", id(src))),
        Node::Arith { op, args, .. } => {
            let at = |k: usize, index: String| format!("(select |n{}| {index})", id(&args[k]));
            clamp(match op {
                Arith::Matmul { k: 0, .. } => sort.zero().to_string(),
                Arith::Matmul { k, n: cols, .. } => {
                    let terms: Vec<String> = (0..*k)
                        .map(|kk| format!("(* {} {})", at(0, format!("(+ (* (div j {cols}) {k}) {kk})")), at(1, format!("(+ {} (mod j {cols}))", kk * cols))))
                        .collect();
                    if terms.len() == 1 {
                        terms[0].clone()
                    } else {
                        format!("(+ {})", terms.join(" "))
                    }
                }
                Arith::Binary { mul, row } => {
                    let b = row.map_or_else(|| at(1, "j".to_string()), |w| at(1, format!("(mod j {w})")));
                    format!("({} {} {b})", if *mul { "*" } else { "+" }, at(0, "j".to_string()))
                }
                Arith::Scale { factor } => format!("(* {} {})", exact_real(f64::from_bits(*factor)), at(0, "j".to_string())),
            })
        }
    }
}

/// `v`'s exact value as an SMT-LIB real: a binary64 is an integer times a power of two.
fn exact_real(v: f64) -> String {
    let bits = v.to_bits();
    let negative = bits >> 63 == 1;
    let exponent = ((bits >> 52) & 0x7ff) as i64;
    let fraction = bits & ((1u64 << 52) - 1);
    let (mut mantissa, mut power) = if exponent == 0 { (fraction, -1074) } else { (fraction | (1u64 << 52), exponent - 1075) };
    if mantissa == 0 {
        return "0.0".to_string();
    }
    while mantissa % 2 == 0 && power < 0 {
        mantissa /= 2;
        power += 1;
    }
    let magnitude = if power == 0 {
        format!("{mantissa}.0")
    } else if power > 0 {
        format!("(* {mantissa}.0 {}.0)", num_pow2(power as u32))
    } else {
        format!("(/ {mantissa}.0 {}.0)", num_pow2(-power as u32))
    };
    if negative {
        format!("(- {magnitude})")
    } else {
        magnitude
    }
}

/// `2^e` in decimal.
fn num_pow2(e: u32) -> String {
    let mut digits = vec![1u8];
    for _ in 0..e {
        let mut carry = 0u8;
        for d in digits.iter_mut() {
            let v = *d * 2 + carry;
            *d = v % 10;
            carry = v / 10;
        }
        if carry > 0 {
            digits.push(carry);
        }
    }
    digits.iter().rev().map(|d| (b'0' + d) as char).collect()
}
