//! The CPU tier's link: a graph the shared optimizer leaves under a CPU shim, as one step of kernels over the graph's
//! tensors, emitted as C++ for the host's compiler beside the CPU library.
//!
//! Every tensor is a buffer of its own: `f32` values, 64-bit integers, or a checkpoint tensor's rows in their block
//! format read in place. Each op is a kernel: the phases of units the library computes it in (`lk_phases`), each phase a
//! space of units the host's threads share and the statement computing a range of them. A fused op is one kernel: its
//! parts' phases merged — a part whose units fall on the same token rows as the phase before it, reading of what that
//! phase writes only its own rows, joins it (each token row's parts computed in turn by one thread); a part independent
//! of the phase before it runs beside it in one space; any other starts a phase of its own after a barrier — and every
//! part's units computed by the function the part runs alone, so a fused op is its parts to the bit.
//!
//! An operand read in part (a strided access) is read in place: by its rows where its part is a kernel's row, by each
//! value's place where the kernel reads values; otherwise its part is gathered by a phase first. A result lands where its
//! destination says — its tensor, a row of a table, each row's share of a table's units — written there by the kernel
//! where the kernel writes rows at a pitch or values at their places, and otherwise into a scratch a phase places. A
//! quantize writes its operand's codes, scales and code sums beside it, which every product by checkpoint rows that reads
//! it takes. Any op the library has no kernel for is refused by name.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use super::kernels::{ATTN_SPLIT, LIBRARY_X86_AVX2_VNNI};
use super::shim::{CpuActivations, CpuCache, CpuIsa, CpuShim};
use crate::model::quant::GgmlType;
use crate::tensor_graph::front::TensorOpSpec;
use crate::tensor_graph::fuse::optimize;
use crate::tensor_graph::graph::{Access, Dst, Elem, Op, RowSel, Storage, Sym, TensorGraph};
use crate::tensor_graph::link::resolve;
use crate::tensor_graph::symbols::ClassId;
use crate::tensor_recognize::qwen::{GateAct, MoeAct, MoeScore, RowIndex, Window};
use crate::tensor_recognize::{ElementwiseOp, RowReduceKind};

/// Why the CPU link builds no program for a graph.
#[derive(Debug)]
pub enum LinkError {
    /// The optimized graph holds ops the CPU shim does not allow, each named.
    Refused(Vec<String>),
    /// An op the CPU library has no kernel for, named by its place and what it is.
    NotYet { op: usize, what: String },
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::Refused(ops) => write!(f, "the CPU shim refuses: {}", ops.join("; ")),
            LinkError::NotYet { op, what } => write!(f, "op {op}: the CPU library has no kernel for {what}"),
        }
    }
}

/// How a buffer of the CPU program holds its tensor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BufKind {
    F32,
    /// Binary16 values: an attention's cache under the binary16 cache, each written rounded and read widened.
    F16,
    /// Int8 codes under one binary16 scale every `unit` values (`lk_i8rows`): an attention's cache under an int8 cache,
    /// each unit written whole as `Keep::keep_row` keeps it — turned first by the Walsh–Hadamard transform where
    /// `turned` — and read as its codes times its scale.
    Int8Rows { unit: usize, turned: bool },
    I64,
    /// A checkpoint tensor's rows in their block format: `rows` rows of `cols` values, `pitch` bytes apart.
    Rows { format: GgmlType, rows: usize, cols: usize, pitch: usize },
}

/// A buffer of the CPU program: a tensor of the graph, by name, its values' count and how they are held.
#[derive(Clone, Debug)]
pub struct CpuBuffer {
    pub name: String,
    pub len: usize,
    pub kind: BufKind,
    /// The C++ identifier the step names it by.
    pub ident: String,
}

/// A linked step: its instruction set, its buffers, the body of its step function — each op's kernel followed by
/// `LK_AFTER(op)`, a hook the driver defines (empty in a model's program) — and the buffers each op writes, by its place.
#[derive(Clone, Debug)]
pub struct CpuProgram {
    pub isa: CpuIsa,
    pub buffers: Vec<CpuBuffer>,
    pub step: String,
    pub writes: Vec<Vec<String>>,
}

/// The bytes a row of `cols` values takes in `format`, for the formats the library reads in place.
fn row_bytes(format: GgmlType, cols: usize) -> Option<usize> {
    match format {
        GgmlType::Q8_0 => Some(cols / 32 * 34),
        GgmlType::Q4K if cols % 256 == 0 => Some(cols / 256 * 144),
        GgmlType::Q6K if cols % 256 == 0 => Some(cols / 256 * 210),
        GgmlType::F32 => Some(cols * 4),
        _ => None,
    }
}

/// A C++ identifier for a tensor's name.
fn ident(name: &str) -> String {
    let body: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    format!("t_{body}")
}

/// An `f32` literal.
fn flit(v: f64) -> String {
    let f = v as f32;
    if f.is_finite() {
        format!("{f:e}f")
    } else if f.is_nan() {
        "NAN".into()
    } else if f > 0.0 {
        "INFINITY".into()
    } else {
        "-INFINITY".into()
    }
}

/// One phase of a kernel: a space of `units` the host's threads share in chunks of at least `grain`, and the statement
/// computing units `[lo, hi)` of it.
#[derive(Clone, Debug, Default)]
struct Phase {
    units: usize,
    grain: usize,
    /// The token rows its units fall on, `(rows, per)`: units `[r·per, (r + 1)·per)` compute token row `r`'s results,
    /// reading of what they read by rows only row `r`. `None` where a unit reads across rows.
    rows: Option<(usize, usize)>,
    body: String,
    /// Every tensor the phase reads, the ones of them it reads beyond its own token rows, and every one it writes.
    reads: BTreeSet<String>,
    across: BTreeSet<String>,
    writes: BTreeSet<String>,
    /// Whether its units spread a token row's work over the threads — an attention's key/value heads and splits, a
    /// per-head product's outputs — so it joins no phase row by row (a joined phase runs each row's units on one thread).
    spread: bool,
}

/// Whether two accesses touch the same values: one tensor, and either is the whole of it or their ranges of its values
/// (`name@lo:hi`, a landing at fixed rows) meet.
fn overlaps(a: &str, b: &str) -> bool {
    let split = |x: &str| -> (String, Option<(usize, usize)>) {
        match x.rsplit_once('@').and_then(|(n, r)| r.split_once(':').map(|(lo, hi)| (n, lo.parse().ok(), hi.parse().ok()))) {
            Some((n, Some(lo), Some(hi))) => (n.to_string(), Some((lo, hi))),
            _ => (x.to_string(), None),
        }
    };
    let ((an, ar), (bn, br)) = (split(a), split(b));
    an == bn && match (ar, br) {
        (Some((a0, a1)), Some((b0, b1))) => a0 < b1 && b0 < a1,
        _ => true,
    }
}

/// Whether any access of `a` touches values one of `b` touches.
fn meet(a: &BTreeSet<String>, b: &BTreeSet<String>) -> bool {
    a.iter().any(|x| b.iter().any(|y| overlaps(x, y)))
}

impl Phase {
    /// A phase of `units` units of which each token row of `rows` takes an equal share, where it divides.
    fn new(units: usize, grain: usize, rows: Option<usize>, body: String) -> Phase {
        let rows = rows.filter(|&r| r > 0 && units % r == 0 && units > 0).map(|r| (r, units / r));
        Phase { units, grain: grain.max(1), rows, body, ..Phase::default() }
    }

    fn reading(mut self, names: impl IntoIterator<Item = String>) -> Phase {
        self.reads.extend(names);
        self
    }

    fn across(mut self, names: impl IntoIterator<Item = String>) -> Phase {
        let names: Vec<String> = names.into_iter().collect();
        self.reads.extend(names.iter().cloned());
        self.across.extend(names);
        self
    }

    fn writing(mut self, names: impl IntoIterator<Item = String>) -> Phase {
        self.writes.extend(names);
        self
    }

    fn spreading(mut self) -> Phase {
        self.spread = true;
        self
    }

    /// Whether `later` reads or writes nothing this phase writes, and writes nothing it reads.
    fn independent(&self, later: &Phase) -> bool {
        !meet(&later.reads, &self.writes) && !meet(&later.writes, &self.reads) && !meet(&later.writes, &self.writes)
    }

    /// Whether `later` can join this phase row by row: both fall on the same token rows, neither reads beyond its rows
    /// what the other writes, and neither spreads its rows over the threads.
    fn joins(&self, later: &Phase) -> bool {
        !self.spread
            && !later.spread
            && matches!((self.rows, later.rows), (Some((a, _)), Some((b, _))) if a == b)
            && !meet(&later.across, &self.writes)
            && !meet(&self.across, &later.writes)
    }

    /// This phase and `later` as one phase over their token rows: each row's units of this phase, then of `later`.
    fn join(self, later: Phase) -> Phase {
        let (rows, a) = self.rows.expect("a joined phase falls on rows");
        let (_, b) = later.rows.expect("a joined phase falls on rows");
        let body = format!(
            "{{ const long tlo = lo, thi = hi;\n{}\n{}\n}}",
            Self::at_rows(&self.body, a),
            Self::at_rows(&later.body, b)
        );
        Phase {
            units: rows,
            grain: 1,
            rows: Some((rows, 1)),
            body,
            reads: &self.reads | &later.reads,
            across: &self.across | &later.across,
            writes: &self.writes | &later.writes,
            spread: self.spread || later.spread,
        }
    }

    /// `body` over token rows `[tlo, thi)`, its own units `per` a row.
    fn at_rows(body: &str, per: usize) -> String {
        if per == 1 {
            format!("{{ const long lo = tlo, hi = thi;\n{body}\n}}")
        } else {
            format!("{{ const long lo = tlo * {per}L, hi = thi * {per}L;\n{body}\n}}")
        }
    }

    /// This phase and the independent `later` as one space: this phase's units, then `later`'s.
    fn beside(self, later: Phase) -> Phase {
        let n = self.units;
        let body = format!(
            "if(lo < {n}L){{ const long l0 = lo, h0 = hi < {n}L ? hi : {n}L; {{ const long lo = l0, hi = h0;\n{}\n}} }}\nif(hi > {n}L){{ const long l1 = (lo > {n}L ? lo : {n}L) - {n}L, h1 = hi - {n}L; {{ const long lo = l1, hi = h1;\n{}\n}} }}",
            self.body, later.body
        );
        Phase {
            units: n + later.units,
            grain: self.grain.min(later.grain),
            rows: None,
            body,
            reads: &self.reads | &later.reads,
            across: &self.across | &later.across,
            writes: &self.writes | &later.writes,
            spread: self.spread || later.spread,
        }
    }
}

/// A kernel of the step: what it declares before it runs — its scratch, its codes, its jobs — and its phases in order.
#[derive(Clone, Debug, Default)]
struct Kernel {
    setup: String,
    phases: Vec<Phase>,
}

impl Kernel {
    fn of(setup: String, phases: Vec<Phase>) -> Kernel {
        Kernel { setup, phases }
    }

    /// The kernel as the step runs it: its setup, then its phases as one run on the pool.
    fn emit(&self, out: &mut String) {
        out.push_str(&self.setup);
        if self.phases.is_empty() {
            return;
        }
        let spaces: Vec<String> = self.phases.iter().map(|p| format!("{{{}L, {}}}", p.units, p.grain)).collect();
        if let [p] = self.phases.as_slice() {
            let _ = writeln!(out, "  lk_phases(pool, {{{}}}, [&](int, long lo, long hi){{\n{}\n  }});", spaces.join(", "), p.body);
        } else {
            let cases: String = self.phases.iter().enumerate().map(|(i, p)| format!("    case {i}: {{\n{}\n    }} break;\n", p.body)).collect();
            let _ = writeln!(out, "  lk_phases(pool, {{{}}}, [&](int ph, long lo, long hi){{\n    switch(ph){{\n{cases}    }}\n  }});", spaces.join(", "));
        }
    }
}

/// A fused op's parts as one kernel: their setups in order, and each part's phases placed at the earliest phase after
/// the last one it depends on — joining the phase it depends on row by row where it can, else joining or running beside
/// the first phase after it, else after a barrier of its own.
fn fuse(parts: Vec<Kernel>) -> Kernel {
    let mut setup = String::new();
    let mut phases: Vec<Phase> = Vec::new();
    for k in parts {
        setup.push_str(&k.setup);
        for p in k.phases {
            let dep = phases.iter().rposition(|q| !q.independent(&p));
            if let Some(j) = dep.filter(|&j| phases[j].joins(&p)) {
                let q = phases.remove(j);
                phases.insert(j, q.join(p));
                continue;
            }
            let t = dep.map_or(0, |j| j + 1);
            if t < phases.len() {
                let q = phases.remove(t);
                phases.insert(t, if q.joins(&p) { q.join(p) } else { q.beside(p) });
            } else {
                phases.push(p);
            }
        }
    }
    Kernel { setup, phases }
}

/// How a kernel reads an operand: the whole tensor, or its part — `w` values of every `p` from `o` — in place.
#[derive(Clone, Debug)]
enum Src {
    Whole(String),
    Part { base: String, w: usize, p: usize, o: usize },
    /// A binary16 tensor (an attention's cache under the binary16 cache), its values read widened one by one.
    Half(String, Option<(usize, usize, usize)>),
    /// A cache in int8 rows, unturned: its values read as their codes times their unit's scale.
    Kept(String, Option<(usize, usize, usize)>),
}

impl Src {
    /// The operand's value `i` (an expression of `i`).
    fn at(&self, i: &str) -> String {
        match self {
            Src::Whole(b) => format!("{b}[{i}]"),
            Src::Part { base, w, p, o } => format!("{base}[(({i}) / {w}) * {p} + {o} + ({i}) % {w}]"),
            Src::Half(b, None) => format!("((float){b}[{i}])"),
            Src::Half(b, Some((w, p, o))) => format!("((float){b}[(({i}) / {w}) * {p} + {o} + ({i}) % {w}])"),
            Src::Kept(b, None) => format!("lk_i8get({b}, {i})"),
            Src::Kept(b, Some((w, p, o))) => format!("lk_i8get({b}, (({i}) / {w}) * {p} + {o} + ({i}) % {w})"),
        }
    }

    /// The operand read by rows of `width` values: its pointer and the values its rows lie apart, where its part is the
    /// row (or the whole tensor is read) of an `f32` tensor.
    fn rows(&self, width: usize) -> Option<(String, usize)> {
        match self {
            Src::Whole(b) => Some((b.clone(), width)),
            Src::Part { base, w, p, o } if *w == width => Some((format!("({base} + {o})"), *p)),
            Src::Part { .. } | Src::Half(..) | Src::Kept(..) => None,
        }
    }
}

/// Where a kernel writes its result: the landing table and the first value's place in it (a runtime expression), each
/// row's share of the table's units where the destination is by unit, and the result's row width.
#[derive(Clone, Debug)]
struct Land {
    table: String,
    base: String,
    unit: Option<(usize, usize, usize)>,
    /// The tensor the result lands in, by name.
    buf: String,
    /// The first value's place where it is known before the step runs: a table's fixed row (or its start), not its own
    /// tensor's whole and not a row a parameter names.
    at: Option<usize>,
    /// Whether the table is binary16: written value by value, each rounded.
    half: bool,
}

impl Land {
    /// Value `i` of the result's place (an expression of `i`).
    fn at(&self, i: &str) -> String {
        format!("{}[{}]", self.table, self.index(i))
    }

    /// Where value `i` of the result lands in its table (an expression of `i`).
    fn index(&self, i: &str) -> String {
        match self.unit {
            None => format!("{} + ({i})", self.base),
            Some((w, p, o)) => format!("{} + (({i}) / {w}) * {p} + {o} + ({i}) % {w}", self.base),
        }
    }

    /// What a result of `len` values writes: the values of the table it lands in, where its place is fixed, else the whole
    /// tensor.
    fn region(&self, len: usize) -> String {
        match self.at {
            Some(a) => {
                let extent = self.unit.map_or(len, |(w, p, _)| len.div_ceil(w.max(1)) * p);
                format!("{}@{a}:{}", self.buf, a + extent)
            }
            None => self.buf.clone(),
        }
    }

    /// The result written by rows of `width` values: its first row's pointer and the values its rows lie apart, where
    /// every row lands in one piece.
    fn rows(&self, width: usize) -> Option<(String, usize)> {
        if self.half {
            return None;
        }
        match self.unit {
            None => Some((format!("({} + {})", self.table, self.base), width)),
            Some((w, p, o)) if w == width => Some((format!("({} + {} + {o})", self.table, self.base), p)),
            Some(_) => None,
        }
    }
}

/// An operand as a kernel reads it by rows: its first row's pointer and the values its rows lie apart, the tensor the
/// kernel reads it from (the operand, or the scratch its part was gathered into), and whether it falls on the result's
/// token rows.
#[derive(Clone, Debug)]
struct In {
    ptr: String,
    pitch: usize,
    name: String,
    aligned: bool,
}

/// A result as a kernel writes it by rows: its first row's pointer and the values its rows lie apart, the tensor the
/// kernel writes (where it lands, or a scratch), and the phase that places the scratch where it lands.
#[derive(Clone, Debug)]
struct Out {
    ptr: String,
    pitch: usize,
    name: String,
    place: Option<Phase>,
}

impl Phase {
    /// The phase reading `i`: by its token rows where it falls on them, else across.
    fn input(self, i: &In) -> Phase {
        if i.aligned {
            self.reading([i.name.clone()])
        } else {
            self.across([i.name.clone()])
        }
    }

    fn output(self, o: &Out) -> Phase {
        self.writing([o.name.clone()])
    }
}

/// The (row, key/value head) units under which an attention whose rows span splits spreads its splits over the threads,
/// its parts then their merge: fewer than the cores of a large host.
const SPREAD_BELOW: usize = 64;

/// The CPU link's state over one graph.
struct Cx<'g> {
    g: &'g TensorGraph,
    kinds: BTreeMap<String, BufKind>,
    lens: BTreeMap<String, usize>,
    /// Every quantized activation: its codes' identifier stem.
    quantized: BTreeSet<String>,
    /// The results landing in a cache in int8 rows while their op is linked: each written whole into a scratch first,
    /// then kept a unit at a time where it lands.
    keeping: BTreeSet<String>,
    next: usize,
    precise: bool,
}

/// Link `g` under `shim`: the shared optimizer's graph for the CPU, every op of it a kernel of the CPU library.
pub fn link(g: &TensorGraph, shim: &CpuShim) -> Result<CpuProgram, LinkError> {
    if shim.isa != CpuIsa::X86Avx2Vnni {
        return Err(LinkError::NotYet { op: 0, what: format!("a library in {}", shim.isa.name()) });
    }
    let o = optimize(g, shim, &[]).graph;
    resolve(&o, shim).map_err(|u| LinkError::Refused(u.iter().map(|x| x.to_string()).collect()))?;
    // A per-head product's matrix by output is its heads' blocks each transposed: the checkpoint's rows are every head's
    // outputs, `a` values each.
    fn head_rows(ops: &[Op], out: &mut BTreeMap<String, (usize, usize)>) {
        for op in ops {
            match &op.sym {
                Sym::Fused(f) => head_rows(&f.parts, out),
                Sym::Prim(TensorOpSpec::HeadMatmul(h)) => {
                    out.insert(op.ins[1].clone(), (h.heads * h.b, h.a));
                }
                _ => {}
            }
        }
    }
    let mut by_head = BTreeMap::new();
    head_rows(&o.ops, &mut by_head);
    let halves: BTreeSet<String> = if shim.cache == CpuCache::F16 { o.caches() } else { BTreeSet::new() };
    let kept = if matches!(shim.cache, CpuCache::Int8Rows | CpuCache::Int8RowsRotated) { o.cache_units() } else { BTreeMap::new() };
    let turned = shim.cache == CpuCache::Int8RowsRotated;
    let mut buffers = Vec::new();
    let mut kinds: BTreeMap<String, BufKind> = BTreeMap::new();
    for t in o.tensors.values() {
        let kind = match (&t.storage, t.elem) {
            (Storage::Rows { format, transposed }, _) => {
                // The checkpoint's rows are the program's columns: a product's `n` outputs, each `k` values — head by
                // head for a per-head product's.
                let (rows, cols) = match by_head.get(&t.name) {
                    Some(&shape) if *transposed => shape,
                    _ if *transposed => (t.cols, t.rows),
                    _ => (t.rows, t.cols),
                };
                let pitch = row_bytes(*format, cols).ok_or_else(|| LinkError::NotYet { op: 0, what: format!("checkpoint rows in {format:?} (`{}`)", t.name) })?;
                BufKind::Rows { format: *format, rows, cols, pitch }
            }
            (_, Elem::Int) => BufKind::I64,
            (_, Elem::Float) if halves.contains(&t.name) => BufKind::F16,
            (_, Elem::Float) if kept.contains_key(&t.name) => {
                let not_yet = |what: String| LinkError::NotYet { op: 0, what };
                let units: Vec<usize> = kept[&t.name].iter().copied().collect();
                let [unit] = units[..] else {
                    return Err(not_yet(format!("a cache in int8 rows read in units of {:?} values (`{}`)", kept[&t.name], t.name)));
                };
                if unit == 0 || t.cols % unit != 0 {
                    return Err(not_yet(format!("a cache in int8 rows of {} values a row, no whole units of {unit} (`{}`)", t.cols, t.name)));
                }
                if turned && !unit.is_power_of_two() {
                    return Err(not_yet(format!("a turned cache row of {unit} values, not a power of two (`{}`)", t.name)));
                }
                BufKind::Int8Rows { unit, turned }
            }
            (_, Elem::Float) => BufKind::F32,
        };
        kinds.insert(t.name.clone(), kind.clone());
        buffers.push(CpuBuffer { name: t.name.clone(), len: t.len(), kind, ident: ident(&t.name) });
    }
    let lens = o.tensors.values().map(|t| (t.name.clone(), t.len())).collect();
    let mut cx = Cx { g: &o, kinds, lens, quantized: BTreeSet::new(), keeping: BTreeSet::new(), next: 0, precise: shim.activations == CpuActivations::Values };
    let mut step = String::new();
    let mut writes = Vec::with_capacity(o.ops.len());
    for (at, op) in o.ops.iter().enumerate() {
        let _ = writeln!(step, "  // op {at}: {}", op.kind());
        cx.kernel(at, op)?.emit(&mut step);
        let _ = writeln!(step, "  LK_AFTER({at});");
        writes.push(op.writes().into_iter().filter(|w| !matches!(cx.kinds.get(*w), Some(BufKind::Rows { .. }) | None)).map(String::from).collect());
    }
    Ok(CpuProgram { isa: shim.isa, buffers, step, writes })
}

impl Cx<'_> {
    fn len_of(&self, n: &str) -> usize {
        self.lens.get(n).copied().unwrap_or(0)
    }

    /// The token rows of `op`'s result: its tensor's rows, where the graph holds it.
    fn rows_of(&self, op: &Op) -> Option<usize> {
        self.g.tensors.get(&op.out).or_else(|| op.dst.own().and_then(|n| self.g.tensors.get(n))).map(|t| t.rows)
    }

    /// Whether operand `k` of `op` falls on the result's token rows `rows`: its tensor has as many.
    fn aligned(&self, op: &Op, k: usize, rows: Option<usize>) -> bool {
        rows.is_some() && self.g.tensors.get(&op.ins[k]).map(|t| t.rows) == rows
    }

    fn int(&self, name: &str) -> bool {
        self.g.tensors.get(name).is_some_and(|t| t.elem == Elem::Int)
    }

    fn fresh(&mut self) -> usize {
        self.next += 1;
        self.next - 1
    }

    /// How `op` reads operand `k`.
    fn src(&self, op: &Op, k: usize) -> Src {
        let base = ident(&op.ins[k]);
        let half = self.kinds.get(&op.ins[k]) == Some(&BufKind::F16);
        let kept = matches!(self.kinds.get(&op.ins[k]), Some(BufKind::Int8Rows { .. }));
        match op.access.get(k).copied().unwrap_or(Access::Whole) {
            Access::Whole if half => Src::Half(base, None),
            Access::Strided(s) if half => Src::Half(base, Some((s.width, s.pitch, s.offset))),
            Access::Whole if kept => Src::Kept(base, None),
            Access::Strided(s) if kept => Src::Kept(base, Some((s.width, s.pitch, s.offset))),
            Access::Whole => Src::Whole(base),
            Access::Strided(s) => Src::Part { base, w: s.width, p: s.pitch, o: s.offset },
        }
    }

    /// Operand `k` read by rows of `width` values: in place where its part is the row, else gathered by a phase into a
    /// scratch of `len` values first (pushed onto `phases`).
    #[allow(clippy::too_many_arguments)]
    fn input(&mut self, op: &Op, k: usize, width: usize, len: usize, rows: Option<usize>, setup: &mut String, phases: &mut Vec<Phase>) -> In {
        let s = self.src(op, k);
        let aligned = self.aligned(op, k, rows);
        if let Some((ptr, pitch)) = s.rows(width) {
            return In { ptr, pitch, name: op.ins[k].clone(), aligned };
        }
        let id = self.fresh();
        let ty = if self.int(&op.ins[k]) { "int64_t" } else { "float" };
        let _ = writeln!(setup, "  static std::vector<{ty}> g{id}({len});");
        let body = format!("      for(long i = lo; i < hi; ++i) g{id}[i] = {};", s.at("i"));
        let p = Phase::new(len, 4096, rows, body).writing([format!("g{id}")]);
        phases.push(if aligned { p.reading([op.ins[k].clone()]) } else { p.across([op.ins[k].clone()]) });
        In { ptr: format!("g{id}.data()"), pitch: width, name: format!("g{id}"), aligned }
    }

    /// Where `op`'s result lands.
    fn land(&self, op: &Op) -> Land {
        if self.keeping.contains(&op.out) {
            let scratch = format!("kv_{}", ident(&op.out));
            return Land { table: format!("{scratch}.data()"), base: "0L".into(), unit: None, buf: scratch, at: Some(0), half: false };
        }
        let dst: &Dst = &op.dst;
        let base = match &dst.row {
            Some((RowSel::Fixed(r), pitch)) => format!("({r}L - 1) * {pitch}L"),
            Some((RowSel::Param(p), pitch)) => format!("({}[0] - 1) * {pitch}L", ident(p)),
            None => "0L".into(),
        };
        let at = match &dst.row {
            _ if dst.own().is_some() => None,
            Some((RowSel::Fixed(r), pitch)) => Some(r.saturating_sub(1) * pitch),
            Some((RowSel::Param(_), _)) => None,
            None => Some(0),
        };
        let half = self.kinds.get(&dst.buf) == Some(&BufKind::F16);
        Land { table: ident(&dst.buf), base, unit: dst.unit.map(|u| (u.width, u.pitch, u.offset)), buf: dst.buf.clone(), at, half }
    }

    /// The tensors a landing reads: the row parameter its place reads.
    fn land_reads(&self, op: &Op) -> Vec<String> {
        match &op.dst.row {
            Some((RowSel::Param(p), _)) => vec![p.clone()],
            _ => Vec::new(),
        }
    }

    /// `op`'s result written by rows of `width` values (`len` in all): in place where every row lands in one piece, else
    /// into a scratch a phase then places.
    fn output(&mut self, op: &Op, width: usize, len: usize, rows: Option<usize>, setup: &mut String) -> Out {
        let land = self.land(op);
        if let Some((ptr, pitch)) = land.rows(width) {
            return Out { ptr, pitch, name: land.region(len), place: None };
        }
        let id = self.fresh();
        let ty = if self.int(&op.dst.buf) { "int64_t" } else { "float" };
        let _ = writeln!(setup, "  static std::vector<{ty}> r{id}({len});");
        let body = format!("      for(long i = lo; i < hi; ++i) {} = r{id}[i];", land.at("i"));
        let place = Phase::new(len, 4096, rows, body).reading([format!("r{id}")]).across(self.land_reads(op)).writing([land.region(len)]);
        Out { ptr: format!("r{id}.data()"), pitch: width, name: format!("r{id}"), place: Some(place) }
    }

    /// `op`'s kernel: a result landing in a cache in int8 rows written whole into a scratch, then each unit of it kept
    /// where it lands; a turned cache read by an attention alone.
    fn kernel(&mut self, at: usize, op: &Op) -> Result<Kernel, LinkError> {
        let not_yet = |what: String| Err(LinkError::NotYet { op: at, what });
        let (attention, set_row) = (matches!(op.sym, Sym::Prim(TensorOpSpec::Attention(_))), matches!(op.sym, Sym::Prim(TensorOpSpec::SetRow(_))));
        if matches!(op.sym, Sym::Fused(_)) {
            return self.kernel_of(at, op);
        }
        for (k, name) in op.ins.iter().enumerate() {
            if matches!(self.kinds.get(name), Some(BufKind::Int8Rows { turned: true, .. })) && !(attention && (k == 1 || k == 2)) && !(set_row && k == 0) {
                return not_yet(format!("a read of the turned cache `{name}` by a {} (an attention alone reads a turned cache)", op.kind()));
            }
        }
        let Some(BufKind::Int8Rows { unit, turned }) = self.kinds.get(&op.dst.buf).cloned().filter(|_| !set_row) else {
            return self.kernel_of(at, op);
        };
        let n = self.len_of(&op.out);
        let land = self.land(op);
        let pitch = op.dst.row.as_ref().map_or(0, |(_, p)| *p);
        let whole = n > 0 && n % unit == 0 && pitch % unit == 0 && land.unit.is_none_or(|(w, p, o)| w % unit == 0 && p % unit == 0 && o % unit == 0);
        if !whole {
            return not_yet(format!("a result of {n} values landing in `{}` in pieces, not whole units of {unit} of its int8 rows", op.dst.buf));
        }
        self.keeping.insert(op.out.clone());
        let built = self.kernel_of(at, op);
        self.keeping.remove(&op.out);
        let mut k = built?;
        let scratch = format!("kv_{}", ident(&op.out));
        k.setup = format!("  static std::vector<float> {scratch}({n});\n{}", k.setup);
        let body = format!(
            "      for(long u = lo; u < hi; ++u) lk_keep_row({scratch}.data() + u * {unit}L, {unit}, {turned}, {}, {});",
            land.table,
            land.index(&format!("u * {unit}L"))
        );
        k.phases.push(Phase::new(n / unit, 1, self.rows_of(op), body).reading([scratch]).across(self.land_reads(op)).writing([land.region(n)]));
        Ok(k)
    }

    fn kernel_of(&mut self, at: usize, op: &Op) -> Result<Kernel, LinkError> {
        let not_yet = |what: String| Err(LinkError::NotYet { op: at, what });
        match &op.sym {
            Sym::Fused(f) if f.class == ClassId::RowWriteQuant => {
                // A row set carrying its rows' codes into the table's: the set, then the quantize of the rows it set.
                let [set, quant] = f.parts.as_slice() else { return not_yet("a row write's quantize of other than its set".into()) };
                let (Sym::Prim(TensorOpSpec::SetRow(r)), Sym::Quantize(_)) = (&set.sym, &quant.sym) else {
                    return not_yet("a row write's quantize of other than its set".into());
                };
                let k1 = self.kernel(at, set)?;
                let first = format!("((long)({}) - 1) * {}L", self.row_index(&r.index, set, 2), r.cols / 32);
                let k2 = self.quantize(at, quant, Some((first, r.count)))?;
                Ok(fuse(vec![k1, k2]))
            }
            Sym::Fused(f) => {
                let parts = f.parts.iter().map(|p| self.kernel(at, p)).collect::<Result<Vec<_>, _>>()?;
                Ok(fuse(parts))
            }
            Sym::View { offset } => {
                let n = self.len_of(&op.out);
                let (s, land) = (self.src(op, 0), self.land(op));
                let body = format!("      for(long i = lo; i < hi; ++i) {} = {};", land.at("i"), s.at(&format!("{offset}L + i")));
                let p = Phase::new(n, 4096, None, body).across([op.ins[0].clone()]).across(self.land_reads(op)).writing([land.region(n)]);
                Ok(Kernel::of(String::new(), vec![p]))
            }
            Sym::Quantize(_) => self.quantize(at, op, None),
            Sym::Prim(spec) => self.prim(at, op, spec),
        }
    }

    /// The codes, scales and code sums of a quantize's operand, beside its values (which the result aliases): of every
    /// row, or of `rows` rows from block `first` (a runtime expression) on. A tensor's codes are declared once, however
    /// many quantizes write them.
    fn quantize(&mut self, at: usize, op: &Op, rows: Option<(String, usize)>) -> Result<Kernel, LinkError> {
        let x = &op.ins[0];
        let t = self.g.tensors.get(x).ok_or_else(|| LinkError::NotYet { op: at, what: format!("a quantize of `{x}`, no tensor") })?;
        if t.cols % 32 != 0 {
            return Err(LinkError::NotYet { op: at, what: format!("a quantize of `{x}`: {} values a row, no whole 32-value blocks", t.cols) });
        }
        let (n, k) = (t.rows, t.cols);
        let mut setup = String::new();
        let mut phases = Vec::new();
        let xin = self.input(op, 0, k, n * k, Some(n), &mut setup, &mut phases);
        let q = ident(&op.out);
        if self.quantized.insert(op.out.clone()) {
            let _ = writeln!(setup, "  static std::vector<int8_t> {q}_q({n} * {k}); static std::vector<float> {q}_d({n} * {k} / 32); static std::vector<int> {q}_s({n} * {k} / 32);");
        }
        let _ = writeln!(setup, "  {q} = (float*){};", xin.ptr);
        let (units, from) = match &rows {
            Some((first, count)) => (count * k / 32, first.clone()),
            None => (n * k / 32, "0L".to_string()),
        };
        let body = format!("      lk_quant_blocks({}, {}L, {q}_q.data(), {q}_d.data(), {q}_s.data(), {k}, {from} + lo, {from} + hi);", xin.ptr, xin.pitch);
        let rows_of = if rows.is_some() { None } else { Some(n) };
        let p = Phase::new(units, 16, rows_of, body);
        phases.push(if rows.is_some() { p.across([xin.name.clone()]) } else { p.input(&xin) }.writing([op.out.clone()]));
        Ok(Kernel::of(setup, phases))
    }

    /// The codes of the activation a product reads: the quantize's beside it, or the kernel's own first phase's.
    #[allow(clippy::too_many_arguments)]
    fn codes(&mut self, op: &Op, k: usize, rows: Option<usize>, mm: usize, setup: &mut String, phases: &mut Vec<Phase>) -> (String, String, String, String) {
        let x = &op.ins[0];
        if self.quantized.contains(x) {
            let q = ident(x);
            return (format!("{q}_q.data()"), format!("{q}_d.data()"), format!("{q}_s.data()"), x.clone());
        }
        let xin = self.input(op, 0, k, mm * k, rows, setup, phases);
        let id = self.fresh();
        let _ = writeln!(setup, "  static std::vector<int8_t> q{id}({mm} * {k}); static std::vector<float> d{id}({mm} * {k} / 32); static std::vector<int> s{id}({mm} * {k} / 32);");
        let body = format!("      lk_quant_blocks({}, {}L, q{id}.data(), d{id}.data(), s{id}.data(), {k}, lo, hi);", xin.ptr, xin.pitch);
        phases.push(Phase::new(mm * k / 32, 16, rows, body).input(&xin).writing([format!("q{id}")]));
        (format!("q{id}.data()"), format!("d{id}.data()"), format!("s{id}.data()"), format!("q{id}"))
    }

    #[allow(clippy::too_many_lines)]
    fn prim(&mut self, at: usize, op: &Op, spec: &TensorOpSpec) -> Result<Kernel, LinkError> {
        let not_yet = |what: String| Err(LinkError::NotYet { op: at, what });
        let rows = self.rows_of(op);
        let mut setup = String::new();
        let mut phases: Vec<Phase> = Vec::new();
        match spec {
            TensorOpSpec::Matmul(m) => {
                let (w, x) = (op.ins[1].clone(), op.ins[0].clone());
                let (mm, n, k) = (m.m, m.n, m.k);
                let wi = ident(&w);
                let body = match self.kinds.get(&w).cloned() {
                    Some(BufKind::Rows { format: f @ (GgmlType::Q8_0 | GgmlType::Q4K | GgmlType::Q6K), cols, pitch, .. }) if cols == k => {
                        let fid = match f {
                            GgmlType::Q8_0 => 8,
                            GgmlType::Q4K => 12,
                            _ => 14,
                        };
                        if self.precise {
                            let xin = self.input(op, 0, k, mm * k, rows, &mut setup, &mut phases);
                            let out = self.output(op, n, mm * n, rows, &mut setup);
                            let body = if fid == 8 {
                                format!("      lk_gemm_q8_0_f32_units(lk_q8f_job{{{wi}, {pitch}L, {}, {}L, {}, {mm}, {n}, {k}, {}L}}, lo, hi);", xin.ptr, xin.pitch, out.ptr, out.pitch)
                            } else {
                                format!("      lk_gemm_kq_units(lk_kq_job{{{fid}, {wi}, {pitch}L, nullptr, nullptr, nullptr, {}, {}L, {}, {mm}, {n}, {k}, {}L}}, lo, hi);", xin.ptr, xin.pitch, out.ptr, out.pitch)
                            };
                            (body, vec![xin.name.clone(), w.clone()], out)
                        } else {
                            let (q, d, s, codes) = self.codes(op, k, rows, mm, &mut setup, &mut phases);
                            let out = self.output(op, n, mm * n, rows, &mut setup);
                            let body = if fid == 8 {
                                format!("      lk_gemm_q8_0_units(lk_q8_job{{{wi}, {pitch}L, {q}, {d}, {}, {mm}, {n}, {k}, {}L}}, lo, hi);", out.ptr, out.pitch)
                            } else {
                                format!("      lk_gemm_kq_units(lk_kq_job{{{fid}, {wi}, {pitch}L, {q}, {d}, {s}, nullptr, 0L, {}, {mm}, {n}, {k}, {}L}}, lo, hi);", out.ptr, out.pitch)
                            };
                            (body, vec![codes, x.clone(), w.clone()], out)
                        }
                    }
                    Some(BufKind::Rows { format: GgmlType::F32, cols, .. }) if cols == k => {
                        let xin = self.input(op, 0, k, mm * k, rows, &mut setup, &mut phases);
                        let out = self.output(op, n, mm * n, rows, &mut setup);
                        let body = format!("      lk_gemm_f32_units(lk_f32_job{{(const float*){wi}, {k}L, {}, {}L, {}, {mm}, {n}, {k}, {}L}}, lo, hi);", xin.ptr, xin.pitch, out.ptr, out.pitch);
                        (body, vec![xin.name.clone(), w.clone()], out)
                    }
                    Some(BufKind::F32) if !self.int(&x) => {
                        // A product by values in the program's layout: `b` is `k × n`.
                        let xin = self.input(op, 0, k, mm * k, rows, &mut setup, &mut phases);
                        let bin = self.input(op, 1, k * n, k * n, None, &mut setup, &mut phases);
                        let out = self.output(op, n, mm * n, rows, &mut setup);
                        let body = format!("      lk_gemm_vals_units({}, {}L, {}, {}, {}L, {mm}, {n}, {k}, lo, hi);", xin.ptr, xin.pitch, bin.ptr, out.ptr, out.pitch);
                        (body, vec![xin.name.clone(), bin.name.clone()], out)
                    }
                    Some(BufKind::I64) if self.int(&x) => {
                        let (a, b) = (self.src(op, 0), self.src(op, 1));
                        let out = self.output(op, n, mm * n, rows, &mut setup);
                        let body = format!(
                            "      for(long o = lo; o < hi; ++o) for(long i = 0; i < {mm}; ++i){{ int64_t s = 0; for(long kk = 0; kk < {k}; ++kk) s += {} * {}; ((int64_t*){})[i * {}L + o] = s; }}",
                            a.at(&format!("i * {k} + kk")),
                            b.at(&format!("kk * {n} + o")),
                            out.ptr,
                            out.pitch
                        );
                        (body, vec![x.clone(), w.clone()], out)
                    }
                    other => return not_yet(format!("a product by `{w}` held as {other:?}")),
                };
                let (body, reads, out) = body;
                phases.push(Phase::new(n, 8, None, body).across(reads).across(self.land_reads(op)).output(&out));
                phases.extend(out.place);
            }
            TensorOpSpec::Elementwise(e) => {
                let (a, b, land) = (self.src(op, 0), self.src(op, 1), self.land(op));
                let bi = e.row.map_or("i".to_string(), |w| format!("i % {w}"));
                let sym = match e.op {
                    ElementwiseOp::Add => "+",
                    ElementwiseOp::Mul => "*",
                };
                let body = format!("      for(long i = lo; i < hi; ++i) {} = {} {sym} {};", land.at("i"), a.at("i"), b.at(&bi));
                let p = self.reads_by_rows(Phase::new(e.n, 4096, rows, body), op, &[0, 1], rows);
                phases.push(p.across(self.land_reads(op)).writing([land.region(e.n)]));
            }
            TensorOpSpec::Scale(s) => phases.push(self.values(op, s.n, rows, &|x| format!("{x} * {}", flit(s.factor.get())))),
            TensorOpSpec::Scatter(_) => return not_yet("a scatter of rows".into()),
            TensorOpSpec::Map(_) => return not_yet("a map the program wrote as a loop".into()),
            TensorOpSpec::Softcap(s) => {
                let c = flit(s.cap.get());
                phases.push(self.values(op, s.n, rows, &|x| format!("lk_cap({x}, {c})")));
            }
            TensorOpSpec::Silu(s) => phases.push(self.exp_values(op, s.n, rows, "lk_silu_vals", &|x| format!("{x} / (1.f + lk_exp(0.f - {x}))"))),
            TensorOpSpec::Sigmoid(s) => phases.push(self.exp_values(op, s.n, rows, "lk_sigmoid_vals", &|x| format!("1.f / (1.f + lk_exp(0.f - {x}))"))),
            TensorOpSpec::Gelu(s) => phases.push(self.values(op, s.n, rows, &|x| format!("lk_gelu1({x})"))),
            TensorOpSpec::Relu(r) => {
                let zero = if self.int(&op.ins[0]) { "(int64_t)0" } else { "0.f" };
                phases.push(self.values(op, r.n, rows, &|x| format!("std::max({x}, {zero})")));
            }
            TensorOpSpec::SignedRoot(r) => {
                let eps = flit(r.eps.get());
                phases.push(self.values(op, r.n, rows, &|x| format!("lk_signed_root({x}, {eps})")));
            }
            TensorOpSpec::IntRequant(r) => phases.push(self.values(op, r.n, rows, &|x| format!("(int64_t)(({x} * (int64_t){}LL) >> {})", r.m, r.s))),
            TensorOpSpec::IntClamp(c) => phases.push(self.values(op, c.n, rows, &|x| format!("std::min(std::max({x}, (int64_t){}LL), (int64_t){}LL)", c.lo, c.hi))),
            TensorOpSpec::SwiGlu(s) => {
                let (gt, up, land) = (self.src(op, 0), self.src(op, 1), self.land(op));
                let body = match (&gt, &up, land.unit) {
                    (Src::Whole(g), Src::Whole(u), None) if !land.half => format!("      lk_swiglu_vals({g}, {u}, {} + {}, lo, hi);", land.table, land.base),
                    _ => format!("      for(long i = lo; i < hi; ++i){{ const float g = {}; {} = g / (1.f + lk_exp(0.f - g)) * {}; }}", gt.at("i"), land.at("i"), up.at("i")),
                };
                let p = self.reads_by_rows(Phase::new(s.total, 4096, rows, body), op, &[0, 1], rows);
                phases.push(p.across(self.land_reads(op)).writing([land.region(s.total)]));
            }
            TensorOpSpec::Decay(d) => {
                let heads = d.n / d.rows.max(1);
                let (ar, dt, al, land) = (self.src(op, 0), self.src(op, 1), self.src(op, 2), self.land(op));
                let body = format!(
                    "      for(long i = lo; i < hi; ++i){{ const long h = i % {heads}; {} = std::pow(1.f + lk_exp({} + {}), {}); }}",
                    land.at("i"),
                    ar.at("i"),
                    dt.at("h"),
                    al.at("h")
                );
                let p = self.reads_by_rows(Phase::new(d.n, 1024, rows, body), op, &[0], rows);
                phases.push(p.across([op.ins[1].clone(), op.ins[2].clone()]).across(self.land_reads(op)).writing([land.region(d.n)]));
            }
            TensorOpSpec::FusedRow(r) => {
                let n = r.rows * r.cols;
                // The weightless sugar normalizes the whole operand as one row.
                let whole_row = r.weight.is_none() && r.reduce == RowReduceKind::RmsNorm;
                let (units, cols) = if whole_row { (1, n) } else { (r.rows, r.cols) };
                let by = if whole_row && n != r.cols { None } else { rows };
                let xin = self.input(op, 0, cols, n, by, &mut setup, &mut phases);
                let mut k = 1;
                let mut operand = |present: bool| -> Option<usize> {
                    present.then(|| {
                        k += 1;
                        k - 1
                    })
                };
                let (wk, rk, bk) = (operand(r.weight.is_some()), operand(r.residual.is_some()), operand(r.bias.is_some()));
                let wrows = r.weight_rows.max(1);
                let win = match wk {
                    Some(kk) => Some(self.input(op, kk, wrows * r.cols, wrows * r.cols, None, &mut setup, &mut phases)),
                    None => None,
                };
                let bin = match bk {
                    Some(kk) => Some(self.input(op, kk, r.cols, r.cols, None, &mut setup, &mut phases)),
                    None => None,
                };
                let rin = match rk {
                    Some(kk) => Some(self.input(op, kk, r.cols, n, by, &mut setup, &mut phases)),
                    None => None,
                };
                if rin.as_ref().is_some_and(|i| i.pitch != r.cols) {
                    return not_yet("a residual read at a pitch of its own".into());
                }
                let out = self.output(op, cols, n, by, &mut setup);
                let ptr = |i: &Option<In>| i.as_ref().map_or("nullptr".to_string(), |i| i.ptr.clone());
                let (w, b, res) = (ptr(&win), ptr(&bin), ptr(&rin));
                let eps = flit(r.eps.get());
                let body = match r.reduce {
                    RowReduceKind::RmsNorm => format!("      lk_rmsnorm_rows({}, {}L, {res}, {w}, {wrows}, {}, {}L, {cols}, {eps}, lo, hi);", xin.ptr, xin.pitch, out.ptr, out.pitch),
                    RowReduceKind::LayerNorm => {
                        if win.is_none() || bin.is_none() {
                            return not_yet("a layer norm without its weight and bias".into());
                        }
                        format!("      lk_layernorm_rows({}, {}L, {w}, {wrows}, {b}, {}, {}L, {cols}, {eps}, lo, hi);", xin.ptr, xin.pitch, out.ptr, out.pitch)
                    }
                    RowReduceKind::Softmax => format!("      lk_softmax_rows({}, {}L, {}, {}L, {cols}, lo, hi);", xin.ptr, xin.pitch, out.ptr, out.pitch),
                };
                let mut p = Phase::new(units, 1, by, body).input(&xin);
                for i in [&win, &bin, &rin].into_iter().flatten() {
                    p = p.input(i);
                }
                phases.push(p.across(self.land_reads(op)).output(&out));
                phases.extend(out.place);
            }
            TensorOpSpec::L2Norm(l) => {
                let n = l.rows * l.cols;
                let xin = self.input(op, 0, l.cols, n, rows, &mut setup, &mut phases);
                let out = self.output(op, l.cols, n, rows, &mut setup);
                let body = format!("      lk_l2norm_rows({}, {}L, {}, {}L, {}, {}, lo, hi);", xin.ptr, xin.pitch, out.ptr, out.pitch, l.cols, flit(l.eps.get()));
                phases.push(Phase::new(l.rows, 1, rows, body).input(&xin).across(self.land_reads(op)).output(&out));
                phases.extend(out.place);
            }
            TensorOpSpec::Rope(r) => {
                let n = r.h * r.hd;
                let xin = self.input(op, 0, r.hd, n, rows, &mut setup, &mut phases);
                let out = self.output(op, r.hd, n, rows, &mut setup);
                let (cs, sn) = (ident(&op.ins[1]), ident(&op.ins[2]));
                let first = match &r.from {
                    Some((p, _)) => format!("({} - 1)", self.row_index(p, op, 3)),
                    None => "0L".into(),
                };
                let by_row = r.rows > 1 || r.from.is_some();
                let body = format!(
                    "      lk_rope_units({}, {}L, {cs}, {sn}, {}, {}L, {}, {}, {}, {first}, {by_row}, lo, hi);",
                    xin.ptr,
                    xin.pitch,
                    out.ptr,
                    out.pitch,
                    r.hd,
                    r.half,
                    (r.h / r.rows.max(1)).max(1)
                );
                let mut tables = vec![op.ins[1].clone(), op.ins[2].clone()];
                tables.extend(op.ins.get(3).cloned());
                phases.push(Phase::new(r.h, 4, rows, body).input(&xin).across(tables).across(self.land_reads(op)).output(&out));
                phases.extend(out.place);
            }
            TensorOpSpec::HeadMatmul(h) => {
                let xin = self.input(op, 0, h.heads * h.a, h.m * h.heads * h.a, rows, &mut setup, &mut phases);
                let out = self.output(op, h.heads * h.b, h.m * h.heads * h.b, rows, &mut setup);
                let w = &op.ins[1];
                let transposed = matches!(self.g.tensors.get(w).map(|t| &t.storage), Some(Storage::Rows { transposed: true, .. }));
                let weights = match self.kinds.get(w) {
                    Some(BufKind::Rows { format, pitch, .. }) => {
                        let f = match format {
                            GgmlType::Q8_0 => 8,
                            GgmlType::Q4K => 12,
                            GgmlType::Q6K => 14,
                            _ => 0,
                        };
                        format!("{f}, {}, {pitch}L, nullptr, {}", ident(w), i32::from(transposed))
                    }
                    Some(BufKind::F32) => format!("0, nullptr, 0L, {}, 0", ident(w)),
                    other => return not_yet(format!("a per-head product by `{w}` held as {other:?}")),
                };
                let body = format!(
                    "      lk_head_matmul_units(lk_hm_job{{{}, {}L, {weights}, {}, {}L, {}, {}, {}, {}}}, lo, hi);",
                    xin.ptr,
                    xin.pitch,
                    out.ptr,
                    out.pitch,
                    h.m,
                    h.heads,
                    h.a,
                    h.b
                );
                phases.push(Phase::new(h.m * h.heads * h.b, 16, rows, body).input(&xin).across([w.clone()]).across(self.land_reads(op)).output(&out).spreading());
                phases.extend(out.place);
            }
            TensorOpSpec::Attention(a) => {
                let ar = a.rows.max(1);
                let mut qin = self.input(op, 0, a.h * a.hd, ar * a.h * a.hd, Some(ar), &mut setup, &mut phases);
                let cache = |cx: &Self, k: usize, width: usize| -> (String, usize, usize) {
                    match cx.src(op, k) {
                        Src::Whole(b) => (b, width, 0),
                        Src::Part { base, p, o, .. } => (format!("({base} + {o})"), p, o),
                        Src::Half(b, None) => (format!("{b}.p"), width, 0),
                        Src::Half(b, Some((_, p, o))) => (format!("({b}.p + {o})"), p, o),
                        Src::Kept(b, None) => (b, width, 0),
                        Src::Kept(b, Some((_, p, o))) => (format!("lk_i8from({b}, {o}L)"), p, o),
                    }
                };
                let (kind, vkind) = (self.kinds.get(&op.ins[1]).cloned(), self.kinds.get(&op.ins[2]).cloned());
                let held = |k: &Option<BufKind>| match k {
                    Some(BufKind::F16) => 1,
                    Some(BufKind::Int8Rows { .. }) => 2,
                    _ => 0,
                };
                if held(&kind) != held(&vkind) {
                    return not_yet(format!("an attention over keys held as {kind:?} and values as {vkind:?}"));
                }
                let job = ["lk_attn_job", "lk_attn_job_h16", "lk_attn_job_i8"][held(&kind)];
                let (kc, kp, ko) = cache(self, 1, a.kv * a.hd);
                let (vc, vp, vo) = cache(self, 2, a.kv * a.vd);
                // A unit of an int8 cache holds every value a head's row of it reads; a turned one is the head's row.
                let mut turned = false;
                for (kind, width, pitch, o) in [(&kind, a.hd, kp, ko), (&vkind, a.vd, vp, vo)] {
                    if let Some(BufKind::Int8Rows { unit, turned: t }) = kind {
                        let fits = pitch % unit == 0 && (0..a.kv).all(|g| (o + g * width) % unit + width <= *unit);
                        if !fits || (*t && *unit != width) {
                            return not_yet(format!("an attention reading heads of {width} values at {o} + {width}·g of rows {pitch} apart from int8 rows of {unit}"));
                        }
                        turned |= *t;
                    }
                }
                // A turned cache's keys are H·k: the query turned (H·q/n) takes the same scores, and the values the
                // weights sum are H·v, turned back once each head's output is summed.
                if turned {
                    let id = self.fresh();
                    let _ = writeln!(setup, "  static std::vector<float> tq{id}({});", ar * a.h * a.hd);
                    let body = format!("      lk_turn_units({}, {}L, tq{id}.data(), {}L, {}, {}, lo, hi);", qin.ptr, qin.pitch, a.h * a.hd, a.hd, a.h);
                    phases.push(Phase::new(ar * a.h, 4, Some(ar), body).input(&qin).writing([format!("tq{id}")]));
                    qin = In { ptr: format!("tq{id}.data()"), pitch: a.h * a.hd, name: format!("tq{id}"), aligned: true };
                }
                let mut k = 3;
                let mut across = vec![op.ins[1].clone(), op.ins[2].clone()];
                let p0 = match &a.pos {
                    Some(p) => {
                        let v = format!("(int)({} - 1)", self.row_index(p, op, k));
                        if matches!(p, RowIndex::Named(_)) {
                            across.push(op.ins[k].clone());
                            k += 1;
                        }
                        v
                    }
                    None => "-1".into(),
                };
                let sinks = if a.sinks.is_some() {
                    across.push(op.ins[k].clone());
                    ident(&op.ins[k])
                } else {
                    "nullptr".into()
                };
                let win = match a.window {
                    Some(Window::Sliding(w)) => w as i64,
                    Some(Window::Chunked(c)) => -(c as i64),
                    None => 0,
                };
                let scale = a.scale.map_or(1.0 / (a.hd as f64).sqrt(), |s| s.get());
                let cap = a.softcap.map_or(0.0, |c| c.get());
                let out = self.output(op, a.h * a.vd, ar * a.h * a.vd, Some(ar), &mut setup);
                let args = format!(
                    "{job}{{{}, {}L, {kc}, {vc}, {sinks}, {}, {}L, {ar}, {h}, {kv}, {hd}, {vd}, {p0}, {l}, {win}, {kp}L, {vp}L, {}, {}}}",
                    qin.ptr,
                    qin.pitch,
                    out.ptr,
                    out.pitch,
                    flit(scale),
                    flit(cap),
                    h = a.h,
                    kv = a.kv,
                    hd = a.hd,
                    vd = a.vd,
                    l = a.l
                );
                let span = match a.window {
                    Some(Window::Sliding(w) | Window::Chunked(w)) => w.min(a.l),
                    None => a.l,
                };
                let splits = span.div_ceil(ATTN_SPLIT);
                if splits > 1 && ar * a.kv < SPREAD_BELOW {
                    let id = self.fresh();
                    let _ = writeln!(setup, "  static std::vector<float> m{id}({});", ar * splits * a.h * (a.vd + 2));
                    let parts = format!("      lk_attention_part_units({args}, m{id}.data(), {splits}, lo, hi);");
                    let merge = format!("      lk_attention_merge_units({args}, m{id}.data(), {splits}, lo, hi);");
                    let reads: Vec<String> = across.iter().cloned().chain(self.land_reads(op)).collect();
                    phases.push(Phase::new(ar * a.kv * splits, 1, Some(ar), parts).input(&qin).across(reads.clone()).output(&out).writing([format!("m{id}")]).spreading());
                    phases.push(Phase::new(ar * a.kv, 1, Some(ar), merge).reading([format!("m{id}")]).across(reads).output(&out));
                } else {
                    let body = format!("      lk_attention_units({args}, lo, hi);");
                    phases.push(Phase::new(ar * a.kv, 1, Some(ar), body).input(&qin).across(across).across(self.land_reads(op)).output(&out).spreading());
                }
                if turned {
                    let body = format!("      lk_turn_units({o}, {p}L, {o}, {p}L, {}, {}, lo, hi);", a.vd, a.h, o = out.ptr, p = out.pitch);
                    phases.push(Phase::new(ar * a.h, 4, Some(ar), body).reading([out.name.clone()]).output(&out));
                }
                phases.extend(out.place);
            }
            TensorOpSpec::FullAttention(f) => {
                let n = f.t * f.h * f.hd;
                let ins: Vec<In> = (0..3).map(|k| self.input(op, k, n, n, None, &mut setup, &mut phases)).collect();
                let out = self.output(op, n, n, None, &mut setup);
                let body = format!("      lk_full_attention_units({}, {}, {}, {}, {}, {}, {}, lo, hi);", ins[0].ptr, ins[1].ptr, ins[2].ptr, out.ptr, f.t, f.h, f.hd);
                let mut p = Phase::new(f.t * f.h, 1, None, body);
                for i in &ins {
                    p = p.across([i.name.clone()]);
                }
                phases.push(p.across(self.land_reads(op)).output(&out));
                phases.extend(out.place);
            }
            TensorOpSpec::RowGather(r) => {
                let idx = self.row_index(&r.index, op, 1);
                let table = ident(&op.ins[0]);
                let mut reads = vec![op.ins[0].clone()];
                reads.extend(op.ins.get(1).cloned());
                match self.kinds.get(&op.ins[0]).cloned() {
                    Some(BufKind::Rows { format: f @ (GgmlType::Q8_0 | GgmlType::Q4K | GgmlType::Q6K), pitch, .. }) => {
                        let out = self.output(op, r.cols, r.cols, Some(1), &mut setup);
                        let call = match f {
                            GgmlType::Q8_0 => format!("lk_gather_q8_0({table}, {pitch}L, (long)({idx} - 1), {}, {}, 1.f);", out.ptr, r.cols),
                            _ => format!("lk_gather_kq({}, {table}, {pitch}L, (long)({idx} - 1), {}, {}, 1.f);", if f == GgmlType::Q4K { 12 } else { 14 }, out.ptr, r.cols),
                        };
                        phases.push(Phase::new(1, 1, Some(1), format!("      if(lo < hi) {call}")).across(reads).across(self.land_reads(op)).output(&out));
                        phases.extend(out.place);
                    }
                    Some(kind @ (BufKind::Rows { format: GgmlType::F32, .. } | BufKind::F32 | BufKind::I64)) => {
                        let land = self.land(op);
                        let cast = if matches!(kind, BufKind::Rows { .. }) { format!("((const float*){table})") } else { table };
                        let body = format!("      for(long c = lo; c < hi; ++c) {} = {cast}[(long)({idx} - 1) * {cols} + c];", land.at("c"), cols = r.cols);
                        phases.push(Phase::new(r.cols, 4096, Some(1), body).across(reads).across(self.land_reads(op)).writing([land.region(r.cols)]));
                    }
                    other => return not_yet(format!("a row of `{}` held as {other:?}", op.ins[0])),
                }
            }
            TensorOpSpec::RowsGather(r) => {
                let idx = ident(&op.ins[1]);
                let table = ident(&op.ins[0]);
                let out = self.output(op, r.cols, r.count * r.cols, Some(r.count), &mut setup);
                let (y, yp) = (&out.ptr, out.pitch);
                let body = match self.kinds.get(&op.ins[0]).cloned() {
                    Some(BufKind::Rows { format: GgmlType::Q8_0, pitch, .. }) => {
                        format!("      for(long s = lo; s < hi; ++s) lk_gather_q8_0({table}, {pitch}L, (long)({idx}[s] - 1), {y} + s * {yp}L, {}, 1.f);", r.cols)
                    }
                    Some(BufKind::Rows { format: f @ (GgmlType::Q4K | GgmlType::Q6K), pitch, .. }) => format!(
                        "      for(long s = lo; s < hi; ++s) lk_gather_kq({}, {table}, {pitch}L, (long)({idx}[s] - 1), {y} + s * {yp}L, {}, 1.f);",
                        if f == GgmlType::Q4K { 12 } else { 14 },
                        r.cols
                    ),
                    Some(BufKind::Rows { format: GgmlType::F32, .. }) => format!(
                        "      for(long s = lo; s < hi; ++s) std::memcpy({y} + s * {yp}L, (const float*){table} + (long)({idx}[s] - 1) * {cols}, sizeof(float) * {cols});",
                        cols = r.cols
                    ),
                    Some(BufKind::F32 | BufKind::I64) => format!(
                        "      for(long s = lo; s < hi; ++s) for(long c = 0; c < {cols}; ++c) ({y})[s * {yp}L + c] = {table}[(long)({idx}[s] - 1) * {cols} + c];",
                        cols = r.cols
                    ),
                    other => return not_yet(format!("rows of `{}` held as {other:?}", op.ins[0])),
                };
                phases.push(Phase::new(r.count, 1, Some(r.count), body).across(op.ins[..2].to_vec()).across(self.land_reads(op)).output(&out));
                phases.extend(out.place);
            }
            TensorOpSpec::ArgMax(a) => {
                // Each chunk's largest and its place, then the first largest of them: the first of equals overall.
                let id = self.fresh();
                let parts = a.n.div_ceil(4096);
                let x = self.src(op, 0);
                let land = self.land(op);
                let ty = if self.int(&op.ins[0]) { "int64_t" } else { "float" };
                let _ = writeln!(setup, "  static std::vector<{ty}> bv{id}({parts}); static std::vector<long> bi{id}({parts});");
                let body0 = format!(
                    "      for(long c = lo; c < hi; ++c){{ long b = c * 4096L; for(long i = b + 1; i < std::min((c + 1) * 4096L, {n}L); ++i) if({} > {}) b = i; bv{id}[c] = {}; bi{id}[c] = b; }}",
                    x.at("i"),
                    x.at("b"),
                    x.at("b"),
                    n = a.n
                );
                let body1 = format!("      if(lo < hi){{ long b = 0; for(long c = 1; c < {parts}; ++c) if(bv{id}[c] > bv{id}[b]) b = c; {} = (int64_t)bi{id}[b] + 1; }}", land.at("0"));
                phases.push(Phase::new(parts, 1, None, body0).across([op.ins[0].clone()]).writing([format!("bv{id}")]));
                phases.push(Phase::new(1, 1, None, body1).across([format!("bv{id}")]).across(self.land_reads(op)).writing([land.region(1)]));
            }
            TensorOpSpec::SetRow(r) if matches!(self.kinds.get(&op.ins[0]), Some(BufKind::Int8Rows { .. })) => {
                let Some(BufKind::Int8Rows { unit, turned }) = self.kinds.get(&op.ins[0]).cloned() else { unreachable!() };
                if r.cols % unit != 0 {
                    return not_yet(format!("a row of {} values set in `{}`, not whole units of {unit} of its int8 rows", r.cols, op.ins[0]));
                }
                let idx = self.row_index(&r.index, op, 2);
                let src = self.input(op, 1, r.cols, r.cols * r.count, Some(r.count), &mut setup, &mut phases);
                let per = r.cols / unit;
                let body = format!(
                    "      for(long u = lo; u < hi; ++u){{ const long row = u / {per}L, e = (u % {per}L) * {unit}L; lk_keep_row({} + row * {}L + e, {unit}, {turned}, {}, ((long)({idx}) - 1 + row) * {}L + e); }}",
                    src.ptr,
                    src.pitch,
                    ident(&op.ins[0]),
                    r.cols
                );
                phases.push(Phase::new(r.count * per, 1, Some(r.count), body).input(&src).across(op.ins.get(2).cloned()).writing([op.ins[0].clone()]));
            }
            TensorOpSpec::SetRow(r) => {
                let idx = self.row_index(&r.index, op, 2);
                let s = self.src(op, 1);
                let table = ident(&op.ins[0]);
                let body = format!("      for(long i = lo; i < hi; ++i) {table}[((long)({idx}) - 1) * {cols}L + i] = {};", s.at("i"), cols = r.cols);
                let mut p = Phase::new(r.cols * r.count, 4096, Some(r.count), body);
                p = if self.g.tensors.get(&op.ins[1]).map(|t| t.rows) == Some(r.count) { p.reading([op.ins[1].clone()]) } else { p.across([op.ins[1].clone()]) };
                phases.push(p.across(op.ins.get(2).cloned()).writing([op.ins[0].clone()]));
            }
            TensorOpSpec::HeadSlice(s) => {
                let (x, land) = (self.src(op, 0), self.land(op));
                let n = s.n / s.every * s.width;
                let body = format!("      for(long i = lo; i < hi; ++i) {} = {};", land.at("i"), x.at(&format!("(i / {w}) * {e} + {f} + i % {w}", w = s.width, e = s.every, f = s.from)));
                let p = self.reads_by_rows(Phase::new(n, 4096, rows, body), op, &[0], rows);
                phases.push(p.across(self.land_reads(op)).writing([land.region(n)]));
            }
            TensorOpSpec::Join(j) => {
                let (ha, hb) = (j.wa / j.heads, j.wb / j.heads);
                let (a, b, land) = (self.src(op, 0), self.src(op, 1), self.land(op));
                let unit = ha + hb;
                let body = format!(
                    "      for(long i = lo; i < hi; ++i){{ const long u = i / {unit}, e = i % {unit}; {} = e < {ha} ? {} : {}; }}",
                    land.at("i"),
                    a.at(&format!("u * {ha} + e")),
                    b.at(&format!("u * {hb} + e - {ha}"))
                );
                let p = self.reads_by_rows(Phase::new(j.rows * unit * j.heads, 4096, rows, body), op, &[0, 1], rows);
                phases.push(p.across(self.land_reads(op)).writing([land.region(j.rows * unit * j.heads)]));
            }
            TensorOpSpec::Transpose(t) => {
                let (x, land) = (self.src(op, 0), self.land(op));
                let body = format!("      for(long i = lo; i < hi; ++i) {} = {};", land.at("i"), x.at(&format!("(i % {r}) * {c} + i / {r}", r = t.rows, c = t.cols)));
                phases.push(Phase::new(t.rows * t.cols, 4096, None, body).across([op.ins[0].clone()]).across(self.land_reads(op)).writing([land.region(t.rows * t.cols)]));
            }
            TensorOpSpec::LaneMean(m) => {
                let (x, land) = (self.src(op, 0), self.land(op));
                let body = format!(
                    "      for(long i = lo; i < hi; ++i){{ const long t = i / {c}, c = i % {c}; float s = 0.f; for(long l = 0; l < {lanes}; ++l) s += {}; {} = s / {}; }}",
                    x.at(&format!("(t * {lanes} + l) * {c} + c", lanes = m.lanes, c = m.cols)),
                    land.at("i"),
                    flit(m.lanes as f64),
                    c = m.cols,
                    lanes = m.lanes
                );
                phases.push(Phase::new(m.rows * m.cols, 1024, None, body).across([op.ins[0].clone()]).across(self.land_reads(op)).writing([land.region(m.rows * m.cols)]));
            }
            TensorOpSpec::LaneSpread(p) => {
                let (x, land) = (self.src(op, 0), self.land(op));
                let scale = p.by.as_ref().map_or(String::new(), |_| format!(" * {}", self.src(op, 1).at(&format!("t * {} + l", p.lanes))));
                let body = format!(
                    "      for(long i = lo; i < hi; ++i){{ const long t = i / ({lanes} * {c}), l = (i / {c}) % {lanes}, c = i % {c}; {} = {}{scale}; }}",
                    land.at("i"),
                    x.at(&format!("t * {} + c", p.cols)),
                    lanes = p.lanes,
                    c = p.cols
                );
                phases.push(Phase::new(p.rows * p.lanes * p.cols, 1024, None, body).across(op.ins.clone()).across(self.land_reads(op)).writing([land.region(p.rows * p.lanes * p.cols)]));
            }
            TensorOpSpec::LaneSums(m) => {
                let (x, land) = (self.src(op, 0), self.land(op));
                let body = format!("      for(long i = lo; i < hi; ++i){{ float s = 0.f; for(long c = 0; c < {c}; ++c) s += {}; {} = s; }}", x.at(&format!("i * {} + c", m.cols)), land.at("i"), c = m.cols);
                phases.push(Phase::new(m.rows * m.lanes, 64, None, body).across([op.ins[0].clone()]).across(self.land_reads(op)).writing([land.region(m.rows * m.lanes)]));
            }
            TensorOpSpec::NgramIds(g) => {
                let id = |k: usize| ident(&op.ins[k]);
                let land = self.land(op);
                let nm1 = g.ngram - 1;
                let body = format!(
                    "      if(lo < hi){{ long at = 0; int64_t* r = {hist}; for(long ti = 0; ti < {tokens}; ++ti){{ const int64_t cur = {toks}[ti]; int64_t x = (cur - 1) * {mul}[0]; bool seen = false;\n\
                     \x20       for(long k = 0; k < {nm1}; ++k){{ int64_t back = r[{nm1} - 1 - k]; if(back == 0) back = {eos}LL; seen |= back == {eos}LL; if(seen) back = {eos}LL; x ^= (back - 1) * {mul}[k + 1];\n\
                     \x20         for(long h = 0; h < {per}; ++h){{ const long head = k * {per} + h; {} = ({modu}[head] ? x % {modu}[head] : 0) + {offs}[head] + 1; ++at; }} }}\n\
                     \x20       for(long k = 0; k + 1 < {nm1}; ++k) r[k] = r[k + 1]; r[{nm1} - 1] = cur; }} }}",
                    land.at("at"),
                    hist = id(1),
                    tokens = g.tokens,
                    toks = id(0),
                    mul = id(2),
                    eos = g.eos,
                    per = g.per_order,
                    modu = id(3),
                    offs = id(4)
                );
                phases.push(Phase::new(1, 1, None, body).across(op.ins[..5].to_vec()).across(self.land_reads(op)).writing([land.buf, op.ins[1].clone()]));
            }
            TensorOpSpec::CausalConv(c) => {
                let xin = self.input(op, 0, c.chan, c.rows * c.chan, None, &mut setup, &mut phases);
                let out = self.output(op, c.chan, c.rows * c.chan, None, &mut setup);
                if out.pitch != c.chan {
                    return not_yet(format!("a convolution written {} values a row apart", out.pitch));
                }
                let (Src::Whole(wc), Src::Whole(cs)) = (self.src(op, 1), self.src(op, 2)) else { return not_yet("a convolution of strided weights or state".into()) };
                let body = format!("      lk_causal_conv_units({}, {}L, {wc}, {cs}, {}, {}, {}, {}, lo, hi);", xin.ptr, xin.pitch, out.ptr, c.taps, c.chan, c.rows);
                phases.push(Phase::new(c.chan, 64, None, body).across([xin.name.clone(), op.ins[1].clone(), op.ins[2].clone()]).across(self.land_reads(op)).output(&out).writing([op.ins[2].clone()]));
                phases.extend(out.place);
            }
            TensorOpSpec::GatedDelta(gd) => {
                let r = gd.rows.max(1);
                let lens = [r * gd.hk * gd.d, r * gd.hk * gd.d, r * gd.hv * gd.d, r * gd.hv * gd.d, r * gd.hv, r * gd.hv];
                let mut ins: Vec<In> = Vec::new();
                for (k, len) in lens.into_iter().enumerate() {
                    ins.push(self.input(op, k, len, len, None, &mut setup, &mut phases));
                }
                let Src::Whole(st) = self.src(op, 6) else { return not_yet("a gated delta rule over a strided state".into()) };
                ins.push(self.input(op, 7, gd.d, gd.d, None, &mut setup, &mut phases));
                let n = r * gd.hv * gd.d;
                let out = self.output(op, n, n, None, &mut setup);
                let body = format!(
                    "      lk_gated_delta_units(lk_gdn_job{{{}, {}, {}, {}, {}, {}, {st}, {}, {}, {}, {}, {}, {r}, {}, {}}}, lo, hi);",
                    ins[0].ptr,
                    ins[1].ptr,
                    ins[2].ptr,
                    ins[3].ptr,
                    ins[4].ptr,
                    ins[5].ptr,
                    ins[6].ptr,
                    out.ptr,
                    gd.hv,
                    gd.hk,
                    gd.d,
                    flit(gd.eps.get()),
                    i32::from(gd.gate == GateAct::Sigmoid)
                );
                let mut p = Phase::new(gd.hv, 1, None, body).across([op.ins[6].clone()]);
                for i in &ins {
                    p = p.across([i.name.clone()]);
                }
                phases.push(p.across(self.land_reads(op)).output(&out).writing([op.ins[6].clone()]));
                phases.extend(out.place);
            }
            TensorOpSpec::Moe(m) => self.moe(at, op, m, &mut setup, &mut phases)?,
            TensorOpSpec::LutAttention(l) => {
                let (q, kc, vc, table) = (self.src(op, 0), self.src(op, 1), self.src(op, 2), self.src(op, 3));
                let len = match &l.pos {
                    Some(p) => format!("(long)({})", self.row_index(p, op, 4)),
                    None => format!("{}L", l.l),
                };
                let land = self.land(op);
                let group = l.h / l.kv.max(1);
                let body = format!(
                    "      std::vector<int64_t> sc({len}), wt({len});\n      for(long head = lo; head < hi; ++head){{ const long kvh = head / {group}; int64_t mx = INT64_MIN;\n\
                     \x20       for(long j = 0; j < {len}; ++j){{ int64_t acc = 0; for(long e = 0; e < {hd}; ++e) acc += {} * {}; sc[j] = acc; mx = std::max(mx, acc); }}\n\
                     \x20       int64_t den = 0; for(long j = 0; j < {len}; ++j){{ const int64_t s = ((mx - sc[j]) * (int64_t){m}LL) >> {s}; wt[j] = {}; den += wt[j]; }}\n\
                     \x20       for(long e = 0; e < {hd}; ++e){{ int64_t num = 0; for(long j = 0; j < {len}; ++j) num += wt[j] * {}; int64_t qt = num / den; if((num % den != 0) && ((num < 0) != (den < 0))) qt -= 1; {} = qt; }} }}",
                    q.at(&format!("head * {} + e", l.hd)),
                    kc.at(&format!("j * {} + kvh * {} + e", l.kv * l.hd, l.hd)),
                    table.at(&format!("std::max((int64_t)0, std::min(s, (int64_t){}))", l.table_len - 1)),
                    vc.at(&format!("j * {} + kvh * {} + e", l.kv * l.hd, l.hd)),
                    land.at(&format!("head * {} + e", l.hd)),
                    hd = l.hd,
                    m = l.m,
                    s = l.s
                );
                phases.push(Phase::new(l.h, 1, None, body).across(op.ins.clone()).across(self.land_reads(op)).writing([land.region(l.h * l.hd)]));
            }
        }
        Ok(Kernel::of(setup, phases))
    }

    /// A mixture of experts for one row, in the library's phases: the activation quantized where an expert's matrices are
    /// checkpoint rows read one word a block, the router's logits from its values, the selection, the kept experts'
    /// gate and up rows, their hidden rows quantized alike, and the down rows summed over the kept experts.
    fn moe(&mut self, at: usize, op: &Op, m: &crate::tensor_recognize::qwen::MoeSpec, setup: &mut String, phases: &mut Vec<Phase>) -> Result<(), LinkError> {
        let id = self.fresh();
        let (d, dff, ne, kk) = (m.d, m.dff, m.ne, m.k);
        let xin = self.input(op, 0, d, d, None, setup, phases);
        let mat = |cx: &Self, k: usize| -> Result<(String, bool), LinkError> {
            let name = &op.ins[k];
            match cx.kinds.get(name) {
                Some(BufKind::Rows { format, pitch, .. }) => {
                    let f = match format {
                        GgmlType::Q8_0 => 8,
                        GgmlType::Q4K => 12,
                        GgmlType::Q6K => 14,
                        GgmlType::F32 => 0,
                        other => return Err(LinkError::NotYet { op: at, what: format!("an expert matrix in {other:?}") }),
                    };
                    Ok((format!("lk_moe_mat{{{f}, {}, {pitch}L, nullptr}}", ident(name)), f != 0))
                }
                Some(BufKind::F32) => Ok((format!("lk_moe_mat{{0, nullptr, 0L, {}}}", ident(name)), false)),
                other => Err(LinkError::NotYet { op: at, what: format!("an expert matrix held as {other:?}") }),
            }
        };
        let (wr, wg, wu, wd) = (mat(self, 1)?, mat(self, 2)?, mat(self, 3)?, mat(self, 4)?);
        let one_word = !self.precise && (wg.1 || wu.1 || wd.1);
        if one_word && (d % 32 != 0 || dff % 32 != 0) {
            return Err(LinkError::NotYet { op: at, what: format!("a mixture of {d} by {dff}, no whole 32-value blocks to quantize") });
        }
        let mut at_k = 5;
        let mut opt = |present: bool| -> String {
            if !present {
                return "nullptr".into();
            }
            at_k += 1;
            ident(&op.ins[at_k - 1])
        };
        let rb = opt(m.router_bias.is_some());
        let (bg, bu, bd) = (opt(m.biases[0].is_some()), opt(m.biases[1].is_some()), opt(m.biases[2].is_some()));
        let bs = opt(m.selection_bias.is_some());
        let (clamped, limit, slope) = match m.act {
            MoeAct::Swiglu => (0, 0.0, 0.0),
            MoeAct::ClampedSwiglu { limit, slope } => (1, f64::from_bits(limit), f64::from_bits(slope)),
        };
        let (groups, kept) = m.groups.map_or((0, 0), |g| (g.count, g.kept));
        let out = self.output(op, d, d, Some(1), setup);
        let _ = writeln!(
            setup,
            "  static std::vector<int8_t> xq{id}({d}), hq{id}({kk} * {dff}); static std::vector<float> xd{id}({d} / 32 + 1), lg{id}({ne}), wt{id}({kk}), hid{id}({kk} * {dff}), hd{id}({kk} * {dff} / 32 + 1); static std::vector<int> xs{id}({d} / 32 + 1), ch{id}({kk}), hs{id}({kk} * {dff} / 32 + 1);\n\
             \x20 lk_moe_job moe{id}{{{}, {}, {}, {}, {}, {rb}, {bg}, {bu}, {bd}, {bs}, {}, {d}, {dff}, {ne}, {kk}, {}, {}, {}, {groups}, {kept}, {clamped}, {}, {}, {}, {}, xq{id}.data(), xd{id}.data(), xs{id}.data(), lg{id}.data(), ch{id}.data(), wt{id}.data(), nullptr, nullptr, hid{id}.data(), {}, hd{id}.data(), hs{id}.data()}};",
            xin.ptr,
            wr.0,
            wg.0,
            wu.0,
            wd.0,
            out.ptr,
            i32::from(m.renormalize),
            i32::from(m.score == MoeScore::Sigmoid),
            i32::from(m.weigh_inputs),
            flit(limit),
            flit(slope),
            flit(m.scale()),
            i32::from(one_word),
            if one_word { format!("hq{id}.data()") } else { "nullptr".into() }
        );
        let consts: Vec<String> = op.ins[1..].to_vec();
        let s = |n: &str| format!("{n}{id}");
        if one_word {
            phases.push(Phase::new(d / 32, 16, None, format!("      lk_quant_blocks({}, {d}L, xq{id}.data(), xd{id}.data(), xs{id}.data(), {d}, lo, hi);", xin.ptr)).across([xin.name.clone()]).writing([s("xq")]));
        }
        phases.push(Phase::new(ne, 1, None, format!("      lk_moe_router(moe{id}, lo, hi);")).across([xin.name.clone()]).across(consts.clone()).writing([s("lg")]));
        phases.push(Phase::new(1, 1, None, format!("      if(lo < hi) lk_moe_select(moe{id});")).across([s("lg")]).across(consts.clone()).writing([s("ch")]));
        phases.push(Phase::new(kk * dff, 8, None, format!("      lk_moe_gate_up(moe{id}, lo, hi);")).across([xin.name.clone(), s("xq"), s("ch")]).across(consts.clone()).writing([s("hid")]));
        if one_word {
            phases.push(Phase::new(kk * dff / 32, 16, None, format!("      lk_quant_blocks(hid{id}.data(), {dff}L, hq{id}.data(), hd{id}.data(), hs{id}.data(), {dff}, lo, hi);")).across([s("hid")]).writing([s("hq")]));
        }
        phases.push(Phase::new(d, 8, None, format!("      lk_moe_down(moe{id}, lo, hi);")).across([s("hid"), s("hq"), s("ch")]).across(consts).across(self.land_reads(op)).output(&out));
        phases.extend(out.place);
        Ok(())
    }

    /// A phase reading operands `ks` of `op`: by their token rows where they fall on the result's, else across.
    fn reads_by_rows(&self, p: Phase, op: &Op, ks: &[usize], rows: Option<usize>) -> Phase {
        let (by_rows, across): (Vec<usize>, Vec<usize>) = ks.iter().partition(|&&k| self.aligned(op, k, rows));
        p.reading(by_rows.into_iter().map(|k| op.ins[k].clone())).across(across.into_iter().map(|k| op.ins[k].clone()))
    }

    /// A one-operand op over `n` values, each result `f` of its operand's value, written where it lands.
    fn values(&self, op: &Op, n: usize, rows: Option<usize>, f: &dyn Fn(&str) -> String) -> Phase {
        let (x, land) = (self.src(op, 0), self.land(op));
        let body = format!("      for(long i = lo; i < hi; ++i) {} = {};", land.at("i"), f(&x.at("i")));
        let p = Phase::new(n, 4096, rows, body);
        self.reads_by_rows(p, op, &[0], rows).across(self.land_reads(op)).writing([land.region(n)])
    }

    /// An exp-made activation over `n` values: the library's vector kernel where operand and result are whole, else each
    /// value by `f` — the same function, lane for lane.
    fn exp_values(&self, op: &Op, n: usize, rows: Option<usize>, kernel: &str, f: &dyn Fn(&str) -> String) -> Phase {
        let (x, land) = (self.src(op, 0), self.land(op));
        match (&x, land.unit) {
            (Src::Whole(b), None) if !land.half => {
                let body = format!("      {kernel}({b}, {} + {}, lo, hi);", land.table, land.base);
                let p = Phase::new(n, 4096, rows, body);
                self.reads_by_rows(p, op, &[0], rows).across(self.land_reads(op)).writing([land.region(n)])
            }
            _ => self.values(op, n, rows, f),
        }
    }

    /// A row index's value as the program reads it: a literal, or the scalar operand `k`.
    fn row_index(&self, i: &RowIndex, op: &Op, k: usize) -> String {
        match i {
            RowIndex::Literal(n) => format!("{n}L"),
            RowIndex::Named(_) => format!("{}[0]", ident(&op.ins[k])),
        }
    }
}

impl CpuProgram {
    /// The program's C++ source with a driver that runs one step: every buffer read from `<dir>/<name>.bin` where the
    /// file is there (`f32`, binary16, little-endian `int64`, an int8 cache's codes then its units' binary16 scales, or
    /// the checkpoint rows' bytes) and zero otherwise, the step run on `threads` threads, and every buffer but the
    /// checkpoint's written to `<dir>/out/<name>.bin` as it was read — and, run with a third argument `trace`, every
    /// buffer each op writes as it stood after the op, to `<dir>/out/after-<op>-<name>.bin`.
    pub fn source_with_file_driver(&self) -> String {
        let mut s = String::from(LIBRARY_X86_AVX2_VNNI);
        s.push_str("\n#include <cstdio>\n#include <string>\n");
        for b in &self.buffers {
            let ty = match b.kind {
                BufKind::F32 => "float*",
                BufKind::F16 => "lk_h16buf",
                BufKind::Int8Rows { .. } => "lk_i8rows",
                BufKind::I64 => "int64_t*",
                BufKind::Rows { .. } => "const uint8_t*",
            };
            let _ = writeln!(s, "static {ty} {};", b.ident);
        }
        let by_name: BTreeMap<&str, &CpuBuffer> = self.buffers.iter().map(|b| (b.name.as_str(), b)).collect();
        s.push_str("static std::string lk_dir; static bool lk_trace = false;\nstatic void lk_write(const std::string& path, const void* p, size_t bytes){ if(FILE* f = std::fopen(path.c_str(), \"wb\")){ std::fwrite(p, 1, bytes, f); std::fclose(f); } }\n");
        s.push_str("static void lk_write_i8(const std::string& path, const lk_i8rows& c, size_t n){ if(FILE* f = std::fopen(path.c_str(), \"wb\")){ std::fwrite(c.q, 1, n, f); std::fwrite(c.s, 2, n / c.unit, f); std::fclose(f); } }\n");
        s.push_str("static void lk_after(int at){\n  if(!lk_trace) return;\n  switch(at){\n");
        for (at, names) in self.writes.iter().enumerate() {
            let _ = write!(s, "    case {at}:");
            for name in names {
                let Some(b) = by_name.get(name.as_str()) else { continue };
                if let BufKind::Int8Rows { .. } = b.kind {
                    let _ = write!(s, " lk_write_i8(lk_dir + \"/out/after-{at}-{name}.bin\", {}, {});", b.ident, b.len);
                    continue;
                }
                let (bytes, at_p) = match b.kind {
                    BufKind::I64 => (b.len * 8, b.ident.clone()),
                    BufKind::F16 => (b.len * 2, format!("{}.p", b.ident)),
                    _ => (b.len * 4, b.ident.clone()),
                };
                let _ = write!(s, " lk_write(lk_dir + \"/out/after-{at}-{name}.bin\", {at_p}, {bytes});");
            }
            s.push_str(" break;\n");
        }
        s.push_str("  }\n}\n#define LK_AFTER(at) lk_after(at)\n");
        s.push_str("static void lk_step(lk_pool& pool){\n");
        s.push_str(&self.step);
        s.push_str("}\n");
        s.push_str(
            "static void* lk_read(const std::string& path, size_t bytes){\n  void* p = std::calloc(bytes ? bytes : 1, 1);\n  if(FILE* f = std::fopen(path.c_str(), \"rb\")){ size_t got = std::fread(p, 1, bytes, f); (void)got; std::fclose(f); }\n  return p;\n}\n\
             int main(int argc, char** argv){\n  if(argc < 3) return 2;\n  const std::string dir = argv[1];\n  lk_dir = dir; lk_trace = argc > 3 && std::string(argv[3]) == \"trace\";\n  lk_pool pool(std::atoi(argv[2]));\n",
        );
        for b in &self.buffers {
            if let BufKind::Int8Rows { unit, .. } = b.kind {
                let _ = writeln!(
                    s,
                    "  {{ const uint8_t* f = (const uint8_t*)lk_read(dir + \"/{name}.bin\", {n} + {m} * 2); {id}.q = (int8_t*)std::malloc({n} ? {n} : 1); std::memcpy({id}.q, f, {n}); {id}.s = (uint16_t*)std::malloc({m} * 2 + 2); std::memcpy({id}.s, f + {n}, {m} * 2); {id}.unit = {unit}; }}",
                    name = b.name,
                    n = b.len,
                    m = b.len / unit,
                    id = b.ident
                );
                continue;
            }
            let bytes = match b.kind {
                BufKind::F32 => b.len * 4,
                BufKind::F16 => b.len * 2,
                BufKind::Int8Rows { .. } => unreachable!("an int8 cache is read above"),
                BufKind::I64 => b.len * 8,
                BufKind::Rows { rows, pitch, .. } => rows * pitch,
            };
            let (place, ty) = match b.kind {
                BufKind::F32 => (b.ident.clone(), "float*"),
                BufKind::F16 => (format!("{}.p", b.ident), "uint16_t*"),
                BufKind::Int8Rows { .. } => unreachable!("an int8 cache is read above"),
                BufKind::I64 => (b.ident.clone(), "int64_t*"),
                BufKind::Rows { .. } => (b.ident.clone(), "const uint8_t*"),
            };
            let _ = writeln!(s, "  {place} = ({ty})lk_read(dir + \"/{}.bin\", {bytes});", b.name);
        }
        s.push_str("  lk_step(pool);\n");
        for b in &self.buffers {
            let (bytes, from) = match b.kind {
                BufKind::F32 => (b.len * 4, b.ident.clone()),
                BufKind::F16 => (b.len * 2, format!("{}.p", b.ident)),
                BufKind::Int8Rows { .. } => {
                    let _ = writeln!(s, "  lk_write_i8(dir + \"/out/{}.bin\", {}, {});", b.name, b.ident, b.len);
                    continue;
                }
                BufKind::I64 => (b.len * 8, b.ident.clone()),
                BufKind::Rows { .. } => continue,
            };
            let _ = writeln!(s, "  lk_write(dir + \"/out/{}.bin\", {from}, {bytes});", b.name);
        }
        s.push_str("  return 0;\n}\n");
        s
    }
}
