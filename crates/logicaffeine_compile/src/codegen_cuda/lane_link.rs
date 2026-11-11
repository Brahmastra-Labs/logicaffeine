//! The lane's link: each op of the one IR, as the shared optimizer leaves it under the lane's shim, to the launches the
//! lane's kernels run it with.
//!
//! A lifted op takes the launch of its kind, refused in the lane's own words where no kernel computes it. A fused op of
//! a class the lane links whole takes the one launch that class's kernel is — a norm with its residual sum or its
//! sandwiched pre-norm, a product with its bias, a rotation with its per-head norm, a convolution with its SiLU and L2
//! norm, the gated pair, the mixture with its shared expert or over a chunk's sorted rows, a launch over several parts —
//! and a class it runs as its parts takes each part's launches in turn. A placement is a field of the launch: a part of
//! every unit read in place, a result landing in rows of a table or units of a wider tensor, values read in the key rows.
//! A producer that quantizes its result in its epilogue writes the quantized form beside, or instead of, its `f32`
//! result. A wide chunk's latent attention runs expanded: its key half, its value half, then the attention.
//!
//! What a launch does is decided here; how it is scheduled — four rows a warp, the shared-row attention, the split
//! counts, a batch's sequences as one launch — is the lane's schedule, after the link.

use std::collections::BTreeMap;

use crate::tensor_graph::front::TensorOpSpec;
use crate::tensor_graph::fuse::{allreduce_partials, binary, class, gates_rows, gemv, leaves, parts_norm, Weights};
use crate::tensor_graph::graph::{Access, Dst as IrDst, Elem, Fused, Op, RowSel as IrRowSel, Sym, TensorGraph};
use crate::tensor_graph::shim::{Shim, Support};
use crate::tensor_graph::symbols::ClassId;
use crate::tensor_recognize::qwen::{Activation, GateAct, RowIndex, Window};
use crate::tensor_recognize::RowReduceKind;

use super::lane::kernels;
use super::lane::plan::{
    attended_span, Advance, ConvPart, Dst, GemmPart, Launch, NormedPart, Part, PreNorm, RopeSide, Router, RowPart, RowSel, ScaleBy, Epilogue,
    SharedExperts, StepIo, GEMV_ROWS, QBLOCK,
};
use super::lane::weights::{Placement, Rows};
use super::lane_shim::{chunks_apart, LaneShim, Latent};
use super::qwen_emit::Q8MatKind;

/// What the link reads beside an op: the optimized graph, where each constant lives, the lane's shim, and the names the
/// step carries across steps.
pub struct Link<'a> {
    pub g: &'a TensorGraph,
    pub constants: &'a BTreeMap<String, Placement>,
    pub shim: &'a LaneShim,
    pub io: &'a StepIo,
}

fn dst(d: &IrDst) -> Dst {
    Dst {
        buf: d.buf.clone(),
        row: d.row.as_ref().map(|(sel, pitch)| {
            let sel = match sel {
                IrRowSel::Param(p) => RowSel::Param(p.clone()),
                IrRowSel::Fixed(n) => RowSel::Fixed(*n),
            };
            (sel, *pitch)
        }),
        unit: d.unit.map(|u| Part { pitch: u.pitch, offset: u.offset }),
    }
}

fn part(a: Access) -> Option<Part> {
    match a {
        Access::Whole => None,
        Access::Strided(s) => Some(Part { pitch: s.pitch, offset: s.offset }),
    }
}

/// The parameter a named row index reads, or the fixed row.
fn row_sel(index: &RowIndex, name: Option<&String>) -> Result<RowSel, String> {
    match index {
        RowIndex::Named(_) => name.cloned().map(RowSel::Param).ok_or_else(|| "a row index with no operand".to_string()),
        RowIndex::Literal(n) => Ok(RowSel::Fixed(*n as usize)),
    }
}

impl Link<'_> {
    fn numerics(&self) -> Q8MatKind {
        self.shim.knobs.activations
    }

    fn rows_of(&self, t: &str) -> Option<&Rows> {
        match self.constants.get(t) {
            Some(Placement::Rows(r)) => Some(r),
            _ => None,
        }
    }

    fn values(&self, t: &str) -> bool {
        matches!(self.constants.get(t), Some(Placement::Values(_)))
    }

    /// The launches `op` runs as, in order, each with the lifted ops it computes.
    pub fn op(&self, op: &Op) -> Result<Vec<(Launch, Vec<usize>)>, String> {
        let whole = |l: Launch| vec![(l, covered(op))];
        match &op.sym {
            Sym::View { offset } => Ok(whole(Launch::View { src: op.ins[0].clone(), offset: *offset, out: op.out.clone() })),
            Sym::Quantize(_) => {
                let n = self.g.tensors.get(&op.ins[0]).map(|t| t.len()).ok_or("a quantize of an unknown tensor")?;
                Ok(whole(Launch::Quant { x: op.ins[0].clone(), n }))
            }
            Sym::Prim(spec) => {
                let mut l = self.prim(op, spec)?;
                self.place(op, &mut l);
                Ok(whole(l))
            }
            Sym::Fused(f) => match self.shim.supports(f.class) {
                Support::Native => self.class_launches(op, f),
                Support::Decompose => f.parts.iter().map(|p| self.op(p)).collect::<Result<Vec<_>, _>>().map(|v| v.concat()),
                Support::Refuse(why) => Err(format!("{}: {why}", f.class)),
            },
        }
    }

    /// The launches a fused op runs as when the lane links its class whole, whatever the shim answers for the class: an
    /// error where the lane has no launch for the class, or where the op is one its kernels cannot run
    /// ([`LaneShim::fits`]).
    pub fn whole(&self, op: &Op) -> Result<Vec<(Launch, Vec<usize>)>, String> {
        let Sym::Fused(f) = &op.sym else { return self.op(op) };
        self.shim.fits(op, self.g)?;
        self.class_launches(op, f)
    }

    /// The launches of a fused op of a class the lane links whole, each with the lifted ops it computes: a product or a
    /// convolution in parts lists each part's in turn.
    fn class_launches(&self, op: &Op, f: &Fused) -> Result<Vec<(Launch, Vec<usize>)>, String> {
        match f.class {
            ClassId::Mla => self.latent(op, f),
            ClassId::AttnOutputGate if gates_rows(&f.parts) => self.gated_rows(op, f, false),
            ClassId::GluActMulQuant => match &f.parts[0].sym {
                Sym::Fused(gated) if gated.class == ClassId::AttnOutputGate && gates_rows(&gated.parts) => self.gated_rows(op, gated, true),
                _ => self.class_launch(op, f).map(|l| vec![(l, covered(op))]),
            },
            _ => self.class_launch(op, f).map(|l| {
                let ops = match l {
                    Launch::GemmParts { .. } | Launch::ConvParts { .. } => f.parts.iter().flat_map(covered).collect(),
                    _ => covered(op),
                };
                vec![(l, ops)]
            }),
        }
    }

    /// A batch's gated attentions (`attn_output_gate` over its sequences' rows): each sequence's attention storing its
    /// output gated by its row of the gate, then the set of that output into its row of the gated product — the row the
    /// batch's layout lays the output in, so the sets leave the step and the attentions run as one (`sequences`). Where
    /// the product is read quantized (`quant`), each attention quantizes its gated row in its store too, and its set
    /// carries the row's quantized form (PARITY-BATCH-ROW-QUANT). `op` is the op linked, the gated attentions `f` or a
    /// fold of them and their quantize.
    fn gated_rows(&self, op: &Op, f: &Fused, quant: bool) -> Result<Vec<(Launch, Vec<usize>)>, String> {
        let (product, rows) = f.parts.split_last().ok_or("a gated attention of no parts")?;
        let gated = binary(product).ok_or("an attention's gated product")?;
        let act = gated.gate.ok_or("a gate's activation")?;
        // The gated rows `f32` where something reads the product so; always, where it is not quantized.
        let plain = !quant || !self.only_projections(&op.out);
        let mut launches = Vec::new();
        for set in rows {
            let Some(TensorOpSpec::SetRow(sr)) = set.prim() else { continue };
            let attention = rows.iter().find(|p| p.out == set.ins[1]).ok_or("a row set from no attention")?;
            let index = row_sel(&sr.index, set.ins.get(2))?;
            let RowSel::Fixed(row) = index else { return Err("a batch's attention set at a row the step names".into()) };
            let mut l = self.one(attention)?;
            let Launch::Attention { gate, goff, quant: q, f32, .. } = &mut l else { return Err("a gated attention".into()) };
            (*gate, *goff, *q, *f32) = (Some((gated.b.clone(), act)), (row - 1) * sr.cols, quant, plain);
            launches.push((l, covered(attention)));
            let set_row = Launch::SetRow { table: op.out.clone(), index, src: set.ins[1].clone(), cols: sr.cols, count: sr.count, seq: None, f32: plain, quant };
            launches.push((set_row, covered(set)));
        }
        // The product, and the quantize where it rides along, are computed in the attentions' stores: their lifted ops
        // are counted with the last attention's.
        let last = launches.iter().rposition(|(l, _)| matches!(l, Launch::Attention { .. })).ok_or("a batch's gate over no attention")?;
        let counted: Vec<usize> = launches.iter().flat_map(|(_, ops)| ops.clone()).collect();
        launches[last].1.extend(covered(op).into_iter().filter(|k| !counted.contains(k)));
        launches[last].1.sort_unstable();
        Ok(launches)
    }

    /// The placements an op's reads and result take, as its launch's fields.
    fn place(&self, op: &Op, l: &mut Launch) {
        if let Some(p) = part(op.access[0]) {
            if let Launch::Norm { part, .. } | Launch::Rope { part, .. } | Launch::HeadGemv { part, .. } = l {
                *part = Some(p);
            }
        }
        if let (Launch::Attention { vstride, voff, .. }, Some(Access::Strided(s))) = (&mut *l, op.access.get(2)) {
            (*vstride, *voff) = (s.pitch, s.offset);
        }
        if op.dst.own().is_some() {
            return;
        }
        let to = dst(&op.dst);
        match l {
            Launch::Gemv { y, .. } | Launch::GemvF32 { y, .. } | Launch::Rope { y, .. } | Launch::Moe { y, .. } | Launch::Join { y, .. } | Launch::HeadGemv { y, .. } => *y = to,
            Launch::Norm { into, .. } => *into = Some(to),
            // Each part lands at its place in the row, where the row lands.
            Launch::NormParts { normed, cols, .. } => {
                for o in normed {
                    let at = o.y.unit.map_or(0, |u| u.offset);
                    let unit = Part { pitch: to.unit.map_or(*cols, |u| u.pitch), offset: to.unit.map_or(0, |u| u.offset) + at };
                    o.y = Dst { unit: Some(unit), ..to.clone() };
                }
            }
            _ => {}
        }
    }

    /// The widths of the parts row `t` of `rows` rows is made of, in order: the operands of the join of rows that makes
    /// it, each taken apart in turn where it is made of parts too, or the results its producers write into its units
    /// (`join_in_place`), each a join's taken apart alike. A norm of such a row sums it by parts, as a row normed by its
    /// parts where they live sums them (`allreduce_sumsq_norm`).
    fn row_parts(&self, t: &str, rows: usize) -> Option<Vec<usize>> {
        let join = |op: &Op| match op.prim() {
            Some(TensorOpSpec::Join(j)) if j.heads == 1 && j.rows == rows && op.access.iter().all(|a| *a == Access::Whole) => {
                let side = |x: &str, w: usize| self.row_parts(x, rows).unwrap_or_else(|| vec![w]);
                let mut parts = side(&op.ins[0], j.wa);
                parts.extend(side(&op.ins[1], j.wb));
                Some(parts)
            }
            _ => None,
        };
        if let Some(i) = self.g.producer(t) {
            return join(&self.g.ops[i]);
        }
        let pitch = self.g.tensors.get(t)?.len() / rows;
        let mut units: Vec<(usize, &Op)> = Vec::new();
        for o in self.g.ops.iter().filter(|o| o.dst.buf == t) {
            let u = o.dst.unit.filter(|u| u.pitch == pitch && o.dst.row.is_none())?;
            units.push((u.offset, o));
        }
        units.sort_by_key(|(at, _)| *at);
        let mut parts = Vec::new();
        for (at, o) in &units {
            if *at != parts.iter().sum::<usize>() {
                return None;
            }
            parts.extend(join(o).unwrap_or_else(|| vec![o.dst.unit.map_or(0, |u| u.width)]));
        }
        (units.len() > 1 && parts.iter().sum::<usize>() == pitch).then_some(parts)
    }

    /// A lifted op's launch, as the lane lowers its kind.
    fn prim(&self, op: &Op, spec: &TensorOpSpec) -> Result<Launch, String> {
        let (ins, out) = (&op.ins, op.out.clone());
        let numerics = self.numerics();
        let (knobs, batch) = (&self.shim.knobs, self.shim.batch);
        let refuse = |why: &str| Err(why.to_string());
        Ok(match spec {
            TensorOpSpec::Matmul(m) => {
                if m.m == 0 {
                    return refuse("a matmul of no rows");
                }
                let w = &ins[1];
                if let Some(r) = self.rows_of(w) {
                    if !r.transposed || r.rows != m.n || r.cols != m.k {
                        return refuse("the weight's checkpoint rows are not the projection's outputs");
                    }
                    if kernels::sixteen(r.ty) && numerics != Q8MatKind::Dequant && !kernels::sixteen_fits(m.k) {
                        return refuse("a 16-bit product on the tensor cores is whole 32s of its inputs");
                    }
                    let rows = kernels::gemm_blk_rows(kernels::format_arg(r.ty), numerics == Q8MatKind::TwoPassInt8);
                    let tiles = m.n.div_ceil(kernels::GEMM_TILE_N) * m.m.div_ceil(rows);
                    let tail = match kernels::gemm_blk_tail(tiles, knobs.chip.sms, m.k.div_ceil(128)) {
                        (whole, split) if split > 1 && m.m > GEMV_ROWS && numerics != Q8MatKind::Dequant && !batch && !kernels::sixteen(r.ty) => Some((whole, split)),
                        _ => None,
                    };
                    Launch::Gemv {
                        x: ins[0].clone(),
                        w: w.clone(),
                        format: r.ty,
                        bias: None,
                        epi: None,
                        y: Dst::to(&out),
                        k: m.k,
                        n: m.n,
                        m: m.m,
                        per_warp: if kernels::sixteen(r.ty) { kernels::sixteen_rows_a_warp(numerics, m.n) } else { 1 },
                        f32: true,
                        quant: false,
                        tail,
                        batch,
                    }
                } else if self.values(w) {
                    Launch::GemvF32 { x: ins[0].clone(), w: w.clone(), bias: None, epi: None, y: Dst::to(&out), k: m.k, n: m.n, m: m.m, split: 1, batch }
                } else {
                    return refuse("a matmul by a computed matrix has no kernel");
                }
            }
            TensorOpSpec::Elementwise(e) => Launch::Binary {
                op: e.op,
                a: ins[0].clone(),
                b: ins[1].clone(),
                gate: None,
                row: e.row,
                out,
                f32: true,
                quant: false,
                n: e.n,
            },
            TensorOpSpec::SwiGlu(s) => Launch::SwiGlu { g: ins[0].clone(), u: ins[1].clone(), out, f32: true, quant: false, n: s.total },
            TensorOpSpec::FusedRow(r) => {
                if r.reduce != RowReduceKind::RmsNorm {
                    return refuse("a softmax or layernorm row has no kernel in the lane yet");
                }
                if r.bias.is_some() {
                    return refuse("an RMSNorm with a bias");
                }
                if r.weight_rows > 1 {
                    return refuse("a norm in lanes (`in L lanes`), each row by its own row of the weight — the lane's norms read one weight row");
                }
                let mut it = ins.iter().skip(1);
                let w = r.weight.map(|_| it.next().expect("the weight operand").clone());
                let add = r.residual.map(|_| it.next().expect("the residual operand").clone());
                // A row joined from parts is summed by parts: each part's sum of squares as a norm of it alone sums it,
                // the sums added in order — what the row normed by its parts where they live computes.
                let parts = self.row_parts(&ins[0], r.rows).filter(|p| add.is_none() && op.access[0] == Access::Whole && p.len() <= kernels::NORM_PARTS_MAX && p.iter().sum::<usize>() == r.cols);
                if let Some(widths) = parts {
                    let at: Vec<usize> = widths.iter().scan(0, |o, w| Some(std::mem::replace(o, *o + w))).collect();
                    let unit = |offset: usize| Some(Part { pitch: r.cols, offset });
                    return Ok(Launch::NormParts {
                        parts: widths.iter().zip(&at).map(|(&cols, &o)| RowPart { x: ins[0].clone(), part: unit(o), cols }).collect(),
                        normed: at.iter().enumerate().map(|(k, &o)| NormedPart { part: k, y: Dst { unit: unit(o), ..Dst::to(&out) } }).collect(),
                        w,
                        rows: r.rows,
                        cols: r.cols,
                        eps: r.eps,
                    });
                }
                Launch::Norm {
                    x: ins[0].clone(),
                    part: None,
                    add,
                    pre: None,
                    sum: None,
                    w,
                    out,
                    into: None,
                    f32: true,
                    quant: false,
                    rows: r.rows,
                    cols: r.cols,
                    eps: r.eps,
                    scale: None,
                    fold: Vec::new(),
                }
            }
            TensorOpSpec::Rope(r) => {
                if r.hd > 1024 || 2 * r.half > r.hd || r.rows == 0 || r.h % r.rows != 0 {
                    return refuse("a head wider than 1024, a rotation wider than the head, or rows that do not divide the heads");
                }
                let pos = match &r.from {
                    Some((RowIndex::Named(_), _)) => Some(ins[3].clone()),
                    Some((RowIndex::Literal(_), _)) => return refuse("a rotation from a literal position"),
                    None => None,
                };
                Launch::Rope {
                    x: ins[0].clone(),
                    part: None,
                    norm: None,
                    cos: ins[1].clone(),
                    sin: ins[2].clone(),
                    pos,
                    y: Dst::to(&out),
                    heads: r.h,
                    per_row: r.h / r.rows,
                    hd: r.hd,
                    half: r.half,
                }
            }
            TensorOpSpec::Silu(s) => Launch::Unary { act: Activation::Silu, x: ins[0].clone(), y: out, n: s.n },
            TensorOpSpec::Sigmoid(s) => Launch::Unary { act: Activation::Sigmoid, x: ins[0].clone(), y: out, n: s.n },
            TensorOpSpec::Gelu(s) => Launch::Unary { act: Activation::Gelu, x: ins[0].clone(), y: out, n: s.n },
            TensorOpSpec::Scale(s) => Launch::Scale { x: ins[0].clone(), y: out, n: s.n, factor: s.factor.get() },
            TensorOpSpec::Decay(d) => {
                if d.rows == 0 || d.n % d.rows != 0 {
                    return refuse("a decay whose rows do not divide its heads");
                }
                Launch::Decay { ar: ins[0].clone(), dt: ins[1].clone(), al: ins[2].clone(), y: out, n: d.n, heads: d.n / d.rows }
            }
            TensorOpSpec::L2Norm(l) => Launch::L2Norm { x: ins[0].clone(), y: out, rows: l.rows, cols: l.cols, eps: l.eps },
            TensorOpSpec::CausalConv(c) => Launch::Conv {
                x: ins[0].clone(),
                w: ins[1].clone(),
                ring: ins[2].clone(),
                y: out,
                chan: c.chan,
                taps: c.taps,
                rows: c.rows,
                silu: false,
                l2: None,
            },
            TensorOpSpec::GatedDelta(d) => {
                if d.gate == GateAct::Sigmoid {
                    return refuse("a gated delta gated by a sigmoid (`with sigmoid gate Z`)");
                }
                if d.d > 1024 || d.d % 32 != 0 {
                    return refuse("a delta-rule head dim past 1024 or not a multiple of 32");
                }
                Launch::Delta {
                    q: ins[0].clone(),
                    k: ins[1].clone(),
                    v: ins[2].clone(),
                    z: ins[3].clone(),
                    b: ins[4].clone(),
                    a: ins[5].clone(),
                    st: ins[6].clone(),
                    gn: ins[7].clone(),
                    out,
                    f32: true,
                    quant: false,
                    hv: d.hv,
                    hk: d.hk,
                    d: d.d,
                    rows: d.rows,
                    eps: d.eps,
                    seqs: 1,
                    qk: None,
                }
            }
            TensorOpSpec::Softcap(s) => Launch::Softcap { x: ins[0].clone(), y: out, n: s.n, cap: s.cap.get() },
            TensorOpSpec::LaneSums(_) | TensorOpSpec::SignedRoot(_) => {
                return refuse("a lane sum or a signed root (`the lane sums of X over L lanes`, `the signed root of X`)")
            }
            TensorOpSpec::NgramIds(_) => return refuse("an n-gram hash (`the ngram ids of T after R by M modulo V from O ending at E`)"),
            TensorOpSpec::LaneMean(_) | TensorOpSpec::LaneSpread(_) => {
                return refuse("a hyper-connected residual's lanes (`the mean of X over L lanes`, `O spread over L lanes`)")
            }
            TensorOpSpec::Attention(a) => {
                if a.kv == 0 || a.h % a.kv != 0 {
                    return refuse("query heads that do not group evenly over the key/value heads");
                }
                if a.hd % 4 != 0 || a.vd % 2 != 0 {
                    return refuse("a key head not a whole number of four-value words, or a value head of an odd width");
                }
                let window = a.window.map(Window::span);
                let span = attended_span(a.l, window);
                let acc = kernels::accumulator(kernels::mode(numerics)).0;
                let Some(per_block) = kernels::attn_heads_per_block(a.h / a.kv, a.hd, a.vd, span, knobs.chip.sms, knobs.chip.smem_kib_per_block * 1024, acc)
                else {
                    return refuse("a head wider than one block's shared memory holds");
                };
                let splits = kernels::attn_splits(a.h / per_block, span, knobs.chip.sms);
                let mut rest = ins[3..].iter();
                let pos = match &a.pos {
                    Some(RowIndex::Named(_)) => rest.next().cloned(),
                    Some(RowIndex::Literal(_)) => return refuse("an attention bounded by a literal position"),
                    None => None,
                };
                Launch::Attention {
                    q: ins[0].clone(),
                    kc: ins[1].clone(),
                    vc: ins[2].clone(),
                    pos,
                    out,
                    heads: a.h,
                    group: a.h / a.kv,
                    hd: a.hd,
                    vd: a.vd,
                    len: a.l,
                    rows: a.rows,
                    window,
                    chunked: matches!(a.window, Some(Window::Chunked(_))),
                    sinks: a.sinks.map(|_| rest.next().cloned().expect("the sinks operand")),
                    vstride: a.kv * a.vd,
                    voff: 0,
                    scale: a.scale.map_or(1.0 / (a.hd as f64).sqrt(), |s| s.get()),
                    cap: a.softcap.map(|c| c.get()),
                    per_block,
                    splits,
                    parallel: chunks_apart(splits, a.rows, a.h / per_block, knobs.chip.sms),
                    split: None,
                    shared: None,
                    seq: None,
                    f32: true,
                    quant: false,
                    gate: None,
                    goff: 0,
                    stream: 0,
                }
            }
            TensorOpSpec::Moe(m) => {
                if !m.weighs_outputs_by_softmax() {
                    return refuse("an expert mixture whose router is not a softmax weighting its experts' outputs — scored by sigmoids, biased in its choice, limited to groups, scaled, or weighting its inputs");
                }
                let ok = |t: &str, rows: usize, cols: usize, transposed: bool| self.rows_of(t).is_some_and(|r| r.rows == rows && r.cols == cols && r.transposed == transposed);
                let router = if ok(&ins[1], m.ne, m.d, true) {
                    Router::Rows(self.rows_of(&ins[1]).expect("rows").ty)
                } else if self.values(&ins[1]) {
                    Router::F32
                } else {
                    return refuse("an expert mixture whose router is neither quantized checkpoint rows nor values");
                };
                if !(ok(&ins[2], m.ne * m.dff, m.d, false) && ok(&ins[3], m.ne * m.dff, m.d, false) && ok(&ins[4], m.ne * m.d, m.dff, false)) {
                    return refuse("an expert mixture whose experts are not quantized checkpoint rows");
                }
                if [1, 2, 3, 4].iter().any(|&k| self.rows_of(&ins[k]).is_some_and(|r| kernels::sixteen(r.ty))) {
                    return refuse("an expert mixture or its router in a 16-bit float format — the expert kernels decode quantized sub-blocks");
                }
                let mut rest = ins[5..].iter();
                let mut constant = |b: bool| -> Result<Option<String>, String> {
                    match b.then(|| rest.next().cloned()).flatten() {
                        Some(n) if self.values(&n) => Ok(Some(n)),
                        Some(_) => Err("an expert mixture whose biases are not constant values".to_string()),
                        None => Ok(None),
                    }
                };
                let rb = constant(m.router_bias.is_some())?;
                let biases = [constant(m.biases[0].is_some())?, constant(m.biases[1].is_some())?, constant(m.biases[2].is_some())?];
                if m.k == 0 || m.k > 32 || m.ne > 8192 {
                    return refuse("more than 32 experts kept, or more than 8192 experts");
                }
                Launch::Moe {
                    x: ins[0].clone(),
                    wr: ins[1].clone(),
                    wg: ins[2].clone(),
                    wu: ins[3].clone(),
                    wd: ins[4].clone(),
                    formats: [2, 3, 4].map(|k| self.rows_of(&ins[k]).expect("rows").ty),
                    y: Dst::to(&out),
                    d: m.d,
                    dff: m.dff,
                    experts: m.ne,
                    k: m.k,
                    router,
                    rb,
                    biases,
                    renormalize: m.renormalize,
                    act: m.act,
                    shared: None,
                    per_warp: 1,
                    rows: 1,
                    router_split: 1,
                    batch,
                }
            }
            TensorOpSpec::RowGather(r) => {
                let table = ins[0].clone();
                match (row_sel(&r.index, ins.get(1))?, self.constants.get(&table)) {
                    (RowSel::Param(p), Some(Placement::Rows(rr))) if !rr.transposed && rr.rows == r.rows && rr.cols == r.cols => {
                        Launch::Gather { table, index: p, out, cols: r.cols, rows: 1, format: Some(rr.ty), scale: None }
                    }
                    (RowSel::Param(p), Some(Placement::Values(_))) => Launch::Gather { table, index: p, out, cols: r.cols, rows: 1, format: None, scale: None },
                    // A row of a computed table the optimizer left a copy: the table is written in place before the
                    // row's last reader.
                    (RowSel::Fixed(row), None) if row >= 1 && row <= r.rows => {
                        Launch::Slice { x: table, y: out, groups: 1, every: r.rows * r.cols, from: (row - 1) * r.cols, width: r.cols }
                    }
                    _ => return refuse("a row of a constant table at a literal index, or of a table in no form a row reads"),
                }
            }
            TensorOpSpec::RowsGather(r) => {
                let table = ins[0].clone();
                let format = match self.constants.get(&table) {
                    Some(Placement::Rows(rr)) if !rr.transposed && rr.rows == r.rows && rr.cols == r.cols => Some(rr.ty),
                    Some(Placement::Values(_)) => None,
                    _ => return refuse("rows gathered from a table that is not a constant"),
                };
                Launch::Gather { table, index: ins[1].clone(), out, cols: r.cols, rows: r.count, format, scale: None }
            }
            TensorOpSpec::ArgMax(a) => Launch::ArgMax { x: ins[0].clone(), out, n: a.n, advance: None, rows: 1, cap: None, y: None },
            TensorOpSpec::SetRow(r) => {
                Launch::SetRow { table: ins[0].clone(), index: row_sel(&r.index, ins.get(2))?, src: ins[1].clone(), cols: r.cols, count: r.count, seq: None, f32: true, quant: false }
            }
            TensorOpSpec::Transpose(_) | TensorOpSpec::FullAttention(_) => return refuse("a transpose or an unmasked block attention has no kernel yet"),
            TensorOpSpec::Relu(r) => {
                if self.g.tensors.get(&ins[0]).is_some_and(|t| t.elem == Elem::Int) {
                    return refuse("a ReLU of integers");
                }
                Launch::Relu { x: ins[0].clone(), out, n: r.n, square: false, f32: true, quant: false }
            }
            TensorOpSpec::Scatter(_) => return refuse("a scatter of rows has no kernel yet"),
            TensorOpSpec::Map(_) => return refuse("a map the program wrote as a loop has no kernel yet"),
            TensorOpSpec::IntRequant(_) | TensorOpSpec::IntClamp(_) | TensorOpSpec::LutAttention(_) => {
                return refuse("a requant, a clamp or a LUT attention has no kernel yet")
            }
            TensorOpSpec::HeadSlice(h) => Launch::Slice { x: ins[0].clone(), y: out, groups: h.n / h.every, every: h.every, from: h.from, width: h.width },
            TensorOpSpec::Join(j) => Launch::Join { a: ins[0].clone(), b: ins[1].clone(), y: Dst::to(&out), rows: j.rows, heads: j.heads, ha: j.wa / j.heads, hb: j.wb / j.heads, yz: 0 },
            TensorOpSpec::HeadMatmul(m) => {
                let Some(r) = self.rows_of(&ins[1]) else { return refuse("a per-head matmul by a matrix that is not checkpoint rows") };
                if kernels::sixteen(r.ty) {
                    return refuse("a per-head matmul by a 16-bit float matrix — the per-head kernels decode quantized sub-blocks");
                }
                let rows_out = match (r.transposed, r.rows, r.cols) {
                    (true, rows, cols) if rows == m.heads * m.b && cols == m.a => true,
                    (false, rows, cols) if rows == m.heads * m.a && cols == m.b => false,
                    _ => return refuse("a per-head matmul whose checkpoint rows are not each head's outputs or inputs"),
                };
                if m.a % QBLOCK != 0 || m.b % QBLOCK != 0 {
                    return refuse("a per-head matmul whose heads are not whole 32-value blocks");
                }
                Launch::HeadGemv {
                    x: ins[0].clone(),
                    part: None,
                    w: ins[1].clone(),
                    format: r.ty,
                    y: Dst::to(&out),
                    m: m.m,
                    heads: m.heads,
                    a: m.a,
                    b: m.b,
                    rows_out,
                    shared: false,
                    upto: None,
                    batch,
                }
            }
        })
    }

    /// One launch of an op: its link, which is one launch.
    fn one(&self, op: &Op) -> Result<Launch, String> {
        let mut v = self.op(op)?;
        match v.len() {
            1 => Ok(v.remove(0).0),
            n => Err(format!("a member the lane runs as one launch links to {n}")),
        }
    }

    /// The launch a fused op of a class the lane links whole is.
    fn class_launch(&self, op: &Op, f: &Fused) -> Result<Launch, String> {
        let parts = &f.parts;
        let mut l = match f.class {
            ClassId::AddNorm => {
                let mut l = self.one(&parts[1])?;
                let Launch::Norm { x, add, sum, .. } = &mut l else { return Err("an add and norm over a norm".into()) };
                let (a, b) = (parts[0].ins[0].clone(), parts[0].ins[1].clone());
                (*x, *add, *sum) = (a, Some(b), op.keeps.first().cloned());
                l
            }
            // The norm folds every partial of the all-reduce its added operand is, each where it lives, in the order the
            // adds folded them.
            ClassId::AllreduceNorm => {
                let (mut partials, sum) = allreduce_partials(op).ok_or("an all-reduce and the norm its sum feeds")?;
                let mut l = self.one(parts.last().ok_or("an all-reduce's norm")?)?;
                let Launch::Norm { x, add, fold, .. } = &mut l else { return Err("an all-reduce's norm".into()) };
                // Added to the residual, the norm folds every partial onto it; normed alone (a post-norm), it reads its
                // last partial onto the others' fold — one addition, which commutes, so the program's sum bit for bit.
                if *x == sum {
                    *x = match add.take() {
                        Some(residual) => residual,
                        None => partials.pop().ok_or("an all-reduce of partials")?,
                    };
                } else if add.as_deref() == Some(sum.as_str()) {
                    *add = None;
                } else {
                    return Err("an all-reduce the norm adds".into());
                }
                *fold = partials;
                l
            }
            // Each part normed where it lives, by the whole row's sum of squares from each part's sum, and written where
            // the slice that takes it back lands.
            ClassId::AllreduceSumsqNorm => {
                let (parts, n, slices) = parts_norm(op).ok_or("a row normed by parts")?;
                return Ok(Launch::NormParts {
                    parts: parts.into_iter().map(|(x, cols)| RowPart { x, part: None, cols }).collect(),
                    normed: slices.into_iter().map(|(part, _, y)| NormedPart { part, y: dst(&y) }).collect(),
                    w: n.w,
                    rows: n.rows,
                    cols: n.cols,
                    eps: n.eps,
                });
            }
            // The norm scales its result as it stores it, the sum it writes on unscaled.
            ClassId::NormScale => {
                let Some(TensorOpSpec::Scale(s)) = parts[1].prim() else { return Err("a norm's scale".into()) };
                let mut l = self.one(&parts[0])?;
                let Launch::Norm { out, scale, .. } = &mut l else { return Err("a scale on a norm".into()) };
                (*out, *scale) = (op.out.clone(), Some(s.factor.get()));
                l
            }
            // The ReLU's result squared as it is stored: by its own launch, or last in the epilogue of the product it is
            // folded into.
            ClassId::ReluSqr => {
                let mut l = self.one(&parts[0])?;
                match &mut l {
                    Launch::Relu { out, square, .. } => (*out, *square) = (op.out.clone(), true),
                    Launch::Gemv { y, epi, .. } | Launch::GemvF32 { y, epi, .. } => {
                        epi.get_or_insert_with(Epilogue::default).relu_sq = true;
                        *y = Dst::to(&op.out);
                    }
                    _ => return Err("a squared ReLU".into()),
                }
                l
            }
            ClassId::NormThenAdd => {
                let (inner, outer) = (&parts[0], &parts[1]);
                let Launch::Norm { w, eps, .. } = self.one(inner)? else { return Err("a sandwich's inner norm".into()) };
                let mut l = self.one(outer)?;
                let Launch::Norm { x, add, pre, part, .. } = &mut l else { return Err("a sandwich's outer norm".into()) };
                (*x, *add, *pre) = (op.ins[0].clone(), Some(op.ins[1].clone()), Some(PreNorm { w, eps }));
                *part = self::part(op.access[0]);
                l
            }
            // Each part run over what the parts before it wrote: a mixture's sum with another array between it and the norm.
            ClassId::GemvNormEpilogue | ClassId::EmbedNorm | ClassId::MoeDeferredFinalize => {
                let mut launches = parts.iter().map(|p| self.one(p));
                let first = launches.next().ok_or("a fold of no parts")??;
                let mut l = launches.try_fold(first, |first, then| Ok::<_, String>(Launch::Fold { first: Box::new(first), then: Box::new(then?) }))?;
                // A projection takes its rows a warp in its fold as the GEMV takes them alone.
                if let (Launch::Fold { first, .. }, Some(p)) = (&mut l, gemv(&parts[0], self.g)) {
                    if let Launch::Gemv { per_warp, .. } = first.as_mut() {
                        *per_warp = self.shim.per_warp(&p);
                    }
                }
                l
            }
            // The set carries its rows' quantized form into the table's, as its source's producer stored it.
            ClassId::RowWriteQuant => {
                let mut l = self.one(&parts[0])?;
                let Launch::SetRow { f32, quant, .. } = &mut l else { return Err("a row set that carries its quantized form".into()) };
                (*f32, *quant) = (!self.only_projections(&op.out), true);
                l
            }
            // The attention stores its output gated, the gate's input read where its producer wrote it.
            ClassId::AttnOutputGate => {
                let mut l = self.one(&parts[0])?;
                let gated = binary(&parts[1]).ok_or("an attention's gated product")?;
                let Launch::Attention { out, gate, .. } = &mut l else { return Err("a gated attention".into()) };
                (*out, *gate) = (op.out.clone(), Some((gated.b, gated.gate.ok_or("a gate's activation")?)));
                l
            }
            // The product scales its result as it stores it: a bias before the scale is its own bias, a constant added
            // after it (`parts[0]` a scaled product then) the epilogue's last step.
            ClassId::GemmScale => {
                let mut l = self.one(&parts[0])?;
                let (Launch::Gemv { y, n, m, epi, .. } | Launch::GemvF32 { y, n, m, epi, .. }) = &mut l else { return Err("a scale on a product".into()) };
                let (n, m) = (*n, *m);
                let own = |t: &str| -> Result<bool, String> {
                    match self.g.tensors.get(t).map(|x| x.len()) {
                        Some(len) if self.values(t) && len == n => Ok(false),
                        Some(len) if self.values(t) && len == n * m => Ok(true),
                        _ => Err(format!("a product's epilogue reads `{t}`, a constant neither a row of its outputs nor their shape")),
                    }
                };
                let constant = || -> Result<String, String> {
                    let b = binary(&parts[1]).ok_or("a scale or bias on a product")?;
                    Ok(if b.a == parts[0].out { b.b } else { b.a })
                };
                let e = epi.get_or_insert_with(Epilogue::default);
                if e.scale.is_some() {
                    let c = constant()?;
                    e.after = Some((c.clone(), own(&c)?));
                } else {
                    e.scale = Some(match parts[1].prim() {
                        Some(TensorOpSpec::Scale(s)) => ScaleBy::Factor(s.factor.get()),
                        _ => {
                            let c = constant()?;
                            if own(&c)? { ScaleBy::Each(c) } else { ScaleBy::Row(c) }
                        }
                    });
                }
                *y = Dst::to(&op.out);
                l
            }
            ClassId::GemmBias => {
                let mut l = self.one(&parts[0])?;
                let (Launch::Gemv { bias, y, .. } | Launch::GemvF32 { bias, y, .. }) = &mut l else { return Err("a bias on a product".into()) };
                (*bias, *y) = (Some(op.ins[2].clone()), Dst::to(&op.out));
                l
            }
            ClassId::QkNormRope => {
                let Launch::Norm { w: Some(w), eps, .. } = self.one(&parts[0])? else { return Err("a rotation's per-head norm".into()) };
                let mut l = self.one(&parts[1])?;
                let Launch::Rope { x, norm, .. } = &mut l else { return Err("a normed rotation".into()) };
                (*x, *norm) = (op.ins[0].clone(), Some((w, eps)));
                l
            }
            ClassId::ConvBiasSilu | ClassId::ConvSiluL2norm => {
                let mut l = self.one(&parts[0])?;
                let Launch::Conv { y, silu, l2, .. } = &mut l else { return Err("a convolution's epilogue".into()) };
                (*y, *silu) = (op.out.clone(), true);
                if f.class == ClassId::ConvSiluL2norm {
                    let Some(TensorOpSpec::L2Norm(n)) = parts[1].prim() else { return Err("a convolution's L2 norm".into()) };
                    *l2 = Some((n.cols, n.eps));
                }
                l
            }
            // A step's delta rule norming its query and key heads itself: the rule reading them as the inputs' launch
            // wrote them, `qk` its norms' epsilon.
            ClassId::DeltaQkNorm => {
                let (delta, norms) = parts.split_last().ok_or("a delta rule norming its heads")?;
                let mut l = self.one(delta)?;
                let Launch::Delta { q, k, qk, .. } = &mut l else { return Err("a delta rule norming its heads".into()) };
                let unnormed = |t: &str| norms.iter().find(|n| n.out == t).map(|n| n.ins[0].clone());
                let (Some(uq), Some(uk)) = (unnormed(q), unnormed(k)) else { return Err("a delta rule's normed query and key".into()) };
                let Some(TensorOpSpec::L2Norm(n)) = norms[0].prim() else { return Err("a delta rule's norms of its heads".into()) };
                (*q, *k, *qk) = (uq, uk, Some(n.eps));
                l
            }
            ClassId::ConvParts => {
                let mut convs = Vec::new();
                for p in parts {
                    let Launch::Conv { x, w, ring, y, chan, taps, l2, .. } = self.one(p)? else { return Err("a part of a launch of convolutions".into()) };
                    convs.push((ConvPart { x, w, ring, y, chan, l2: l2.is_some() }, taps, l2));
                }
                let taps = convs[0].1;
                let norm = convs.iter().find_map(|c| c.2);
                let (block, eps) = match norm {
                    Some((d, e)) => (d, Some(e)),
                    None => (256, None),
                };
                Launch::ConvParts { parts: convs.into_iter().map(|c| c.0).collect(), taps, block, eps, seqs: 1 }
            }
            ClassId::GdnGating => {
                let sigmoid = parts.iter().find(|p| matches!(p.prim(), Some(TensorOpSpec::Sigmoid(_)))).ok_or("the write strength")?;
                let decay = parts.iter().find(|p| matches!(p.prim(), Some(TensorOpSpec::Decay(_)))).ok_or("the forget gate")?;
                let Some(TensorOpSpec::Decay(d)) = decay.prim() else { unreachable!() };
                Launch::Gates {
                    br: sigmoid.ins[0].clone(),
                    ar: decay.ins[0].clone(),
                    dt: decay.ins[1].clone(),
                    al: decay.ins[2].clone(),
                    beta: sigmoid.out.clone(),
                    decay: decay.out.clone(),
                    n: d.n,
                    heads: d.n / d.rows,
                }
            }
            ClassId::GdnGateProjections => {
                let gating = parts.iter().find(|p| class(p) == Some(ClassId::GdnGating)).ok_or("the gates")?;
                let Launch::Gates { br, ar, dt, al, beta, decay, heads, .. } = self.one(gating)? else { return Err("the gates".into()) };
                let product = |t: &str| parts.iter().find(|p| p.out == t).and_then(|p| gemv(p, self.g)).ok_or("a gate's product");
                let (pb, pa) = (product(&br)?, product(&ar)?);
                Launch::GateProj { x: pb.x, wb: pb.w, wa: pa.w, dt, al, beta, decay, k: pb.k, heads, parts: 1 }
            }
            ClassId::GluActMul => {
                let act = &parts[0];
                let gate = match act.prim() {
                    Some(TensorOpSpec::Silu(_)) => Activation::Silu,
                    Some(TensorOpSpec::Sigmoid(_)) => Activation::Sigmoid,
                    Some(TensorOpSpec::Gelu(_)) => Activation::Gelu,
                    _ => return Err("a gate's activation".into()),
                };
                let mut l = self.one(&parts[1])?;
                let Launch::Binary { a, b, gate: g, .. } = &mut l else { return Err("a gated product".into()) };
                let other = if *a == act.out { b.clone() } else { a.clone() };
                (*a, *b, *g) = (other, act.ins[0].clone(), Some(gate));
                l
            }
            ClassId::GemvGateUpGlu => {
                let glu = parts.last().ok_or("a gated pair's activation")?;
                let (g, u, act) = match glu.prim() {
                    Some(TensorOpSpec::SwiGlu(_)) => (glu.ins[0].clone(), glu.ins[1].clone(), None),
                    _ => match self.one(glu)? {
                        Launch::Binary { a, b, gate: Some(act), .. } => (b, a, Some(act)),
                        _ => return Err("a gated pair's activation".into()),
                    },
                };
                let product = |t: &str| -> Result<Launch, String> { self.one(parts.iter().find(|p| p.out == t).ok_or("a gated pair's product")?) };
                let (Launch::Gemv { x, w: wg, format: fg, bias: bg, k, n, m, tail, batch, .. }, Launch::Gemv { w: wu, format: fu, bias: bu, .. }) = (product(&g)?, product(&u)?) else {
                    return Err("a gated pair of products by checkpoint rows".into());
                };
                let gemv_rows = m <= GEMV_ROWS || self.numerics() == Q8MatKind::Dequant || batch;
                let paired = !gemv_rows && fg == fu && !kernels::sixteen(fg) && n % (kernels::GEMM_TILE_N / 2) == 0;
                let tail = tail.filter(|_| paired).map(|(whole, split)| (2 * whole, split));
                if (kernels::sixteen(fg) || kernels::sixteen(fu)) && fg != fu {
                    return Err("a gated pair of 16-bit products is one format".into());
                }
                let per_warp = if kernels::sixteen(fg) { kernels::sixteen_rows_a_warp(self.numerics(), n) } else { 1 };
                Launch::GemvGlu { x, wg, wu, formats: [fg, fu], biases: [bg, bu], act, out: op.out.clone(), f32: true, quant: false, k, n, m, per_warp, tail, batch }
            }
            ClassId::SharedExpertFusion => {
                let mixture = parts.iter().find(|p| matches!(p.prim(), Some(TensorOpSpec::Moe(_)))).ok_or("the mixture")?;
                let glu = parts.iter().find(|p| class(p) == Some(ClassId::GemvGateUpGlu)).ok_or("the shared expert's gated pair")?;
                let down = parts.iter().find(|p| gemv(p, self.g).is_some_and(|d| d.x == glu.out)).ok_or("the shared expert's down product")?;
                let Launch::GemvGlu { wg, wu, formats: [fg, fu], n: sdff, .. } = self.one(glu)? else { return Err("the shared expert's gated pair".into()) };
                let Launch::Gemv { w: wd, format: fd, .. } = self.one(down)? else { return Err("the shared expert's down product".into()) };
                let mut l = self.one(mixture)?;
                let Launch::Moe { y, shared, .. } = &mut l else { return Err("a mixture".into()) };
                (*y, *shared) = (Dst::to(&op.out), Some(SharedExperts { wg, wu, wd, formats: [fg, fu, fd], dff: sdff }));
                l
            }
            ClassId::MoeTokenSort => {
                let mixtures: Vec<&Op> = parts.iter().filter(|p| matches!(p.prim(), Some(TensorOpSpec::Moe(_)))).collect();
                let first = mixtures.first().ok_or("a chunk's mixtures")?;
                let src = parts.iter().find(|p| matches!(p.sym, Sym::View { .. })).map(|v| v.ins[0].clone()).ok_or("the chunk's rows")?;
                let mut l = self.prim(first, first.prim().expect("a mixture"))?;
                let Launch::Moe { x, y, rows, .. } = &mut l else { return Err("a mixture".into()) };
                (*x, *y, *rows) = (src, Dst::to(&op.out), mixtures.len());
                l
            }
            ClassId::EmbedScale => {
                let Some(TensorOpSpec::Scale(s)) = parts[1].prim() else { return Err("an embedding's scale".into()) };
                let mut l = self.one(&parts[0])?;
                let Launch::Gather { out, scale, .. } = &mut l else { return Err("a gather".into()) };
                (*out, *scale) = (op.out.clone(), Some(s.factor.get()));
                l
            }
            ClassId::RopeQkOneLaunch => {
                let mut sides = Vec::new();
                let mut shared = None;
                for p in parts {
                    let Launch::Rope { x, part, norm, y, heads, per_row, cos, sin, pos, hd, half } = self.one(p)? else { return Err("a side of a launch of rotations".into()) };
                    shared.get_or_insert((cos, sin, pos, hd, half));
                    sides.push(RopeSide { x, part, norm, y, heads, per_row, yz: 0 });
                }
                let (cos, sin, pos, hd, half) = shared.ok_or("a launch of rotations")?;
                Launch::RopeParts { sides, cos, sin, pos, hd, half, seqs: false }
            }
            ClassId::CapArgmax => {
                let Some(TensorOpSpec::Softcap(s)) = parts[0].prim() else { return Err("a soft cap before an arg-max".into()) };
                let mut l = self.one(&parts[1])?;
                let Launch::ArgMax { x, cap, y, .. } = &mut l else { return Err("an arg-max of a soft cap".into()) };
                // The capped row is written as the arg-max reads it where something past the fold reads it.
                let capped = parts[0].out.clone();
                (*x, *cap, *y) = (op.ins[0].clone(), Some(s.cap.get()), op.keeps.contains(&capped).then_some(capped));
                l
            }
            ClassId::FusedSampling => {
                let mut l = self.one(&parts[0])?;
                let Launch::ArgMax { out, advance, .. } = &mut l else { return Err("an arg-max".into()) };
                let seqs: Vec<(&str, &str)> = std::iter::once((self.io.next.as_str(), self.io.position.as_str()))
                    .chain(self.io.more.iter().map(|q| (q.next.as_str(), q.position.as_str())))
                    .collect();
                let item = seqs.iter().position(|(next, _)| next == out).ok_or("an arg-max that picks no sequence's token")?;
                *advance = Some(Advance { token: self.io.token.clone(), position: seqs[item].1.to_string(), item });
                l
            }
            ClassId::NormQuant
            | ClassId::AddNormQuant
            | ClassId::GluActMulQuant
            | ClassId::ElementwiseQuant
            | ClassId::GdnOutputQuant
            | ClassId::GemvQuant
            | ClassId::AttnOutputQuant => {
                let mut l = self.one(&parts[0])?;
                let only = self.only_projections(&op.out);
                match &mut l {
                    Launch::Norm { f32, quant, .. }
                    | Launch::Binary { f32, quant, .. }
                    | Launch::Relu { f32, quant, .. }
                    | Launch::SwiGlu { f32, quant, .. }
                    | Launch::Delta { f32, quant, .. }
                    | Launch::GemvGlu { f32, quant, .. }
                    | Launch::Gemv { f32, quant, .. }
                    | Launch::Attention { f32, quant, .. } => (*quant, *f32) = (true, !only),
                    Launch::Fold { then, .. } => match then.as_mut() {
                        Launch::Norm { f32, quant, .. } => (*quant, *f32) = (true, !only),
                        _ => return Err("a fold that quantizes its norm's result".into()),
                    },
                    _ => return Err("a producer that quantizes in its epilogue".into()),
                }
                l
            }
            // A gated delta layer's inputs: its products as one GEMV in parts, then its convolutions over the parts they
            // read, then its gates — from two parts, or projected from the activation in `f32` — each as it runs alone.
            ClassId::GdnInputEpilogue => {
                let inputs = self.shim.gdn_inputs(f, self.g)?;
                let products: Vec<_> = inputs.products.iter().map(|p| gemv(p, self.g).ok_or("a gated delta layer's product")).collect::<Result<_, _>>()?;
                let projections = self.projections(&products)?;
                let normed = inputs.convs.iter().find_map(|c| c.l2);
                let convs = Launch::ConvParts {
                    parts: inputs.convs.iter().map(|c| ConvPart { x: c.x.clone(), w: c.w.clone(), ring: c.ring.clone(), y: c.y.clone(), chan: c.chan, l2: c.l2.is_some() }).collect(),
                    taps: inputs.convs[0].taps,
                    block: normed.map_or(256, |(cols, _)| cols),
                    eps: normed.map(|(_, eps)| eps),
                    seqs: 1,
                };
                let gates = self.one(inputs.gating)?;
                Launch::Fold { first: Box::new(Launch::Fold { first: Box::new(projections), then: Box::new(convs) }), then: Box::new(gates) }
            }
            ClassId::MergedQkv | ClassId::QkvHeadEpilogue => {
                let members: Vec<&Op> = if f.class == ClassId::QkvHeadEpilogue { vec![op] } else { parts.iter().collect() };
                let (mut products, mut sides) = (Vec::new(), Vec::new());
                for m in members {
                    let product = match &m.sym {
                        Sym::Fused(e) if e.class == ClassId::QkvHeadEpilogue => {
                            sides.push(self.one(&e.parts[1])?);
                            &e.parts[0]
                        }
                        _ => m,
                    };
                    products.push(gemv(product, self.g).ok_or("products of one activation")?);
                }
                let projections = self.projections(&products)?;
                if sides.is_empty() {
                    projections
                } else {
                    let mut turned = Vec::new();
                    let mut shared = None;
                    for side in sides {
                        let Launch::Rope { x, part, norm, y, heads, per_row, cos, sin, pos, hd, half } = side else { return Err("a part's heads turned in its epilogue".into()) };
                        shared.get_or_insert((cos, sin, pos, hd, half));
                        turned.push(RopeSide { x, part, norm, y, heads, per_row, yz: 0 });
                    }
                    let (cos, sin, pos, hd, half) = shared.expect("a rotation");
                    Launch::Fold { first: Box::new(projections), then: Box::new(Launch::RopeParts { sides: turned, cos, sin, pos, hd, half, seqs: false }) }
                }
            }
            other => return Err(format!("the lane links `{other}` whole and has no launch for it")),
        };
        self.place(op, &mut l);
        Ok(l)
    }

    /// Products of one activation side by side as one launch: on the tensor cores past the GEMV's rows, the GEMV in
    /// parts at or below them, its rows a warp the parts' (`LaneShim::parts_per_warp`).
    fn projections(&self, products: &[crate::tensor_graph::fuse::GemvForm]) -> Result<Launch, String> {
        let first = products.first().ok_or("products of one activation")?;
        let Weights::Rows(format) = first.weights else { return Err("products by checkpoint rows".into()) };
        let gemm: Vec<GemmPart> = products.iter().map(|p| GemmPart { w: p.w.clone(), bias: p.bias.clone(), y: dst(&p.dst), n: p.n }).collect();
        Ok(if self.shim.on_tensor_cores(first) {
            let n: usize = gemm.iter().map(|p| p.n).sum();
            let rows = kernels::gemm_blk_rows(kernels::format_arg(format), self.numerics() == Q8MatKind::TwoPassInt8);
            let tiles = n.div_ceil(kernels::GEMM_TILE_N) * first.m.div_ceil(rows);
            let tail = match kernels::gemm_blk_tail(tiles, self.shim.knobs.chip.sms, first.k.div_ceil(128)) {
                (whole, split) if split > 1 => Some((whole, split)),
                _ => None,
            };
            Launch::GemmParts { x: first.x.clone(), format, k: first.k, m: first.m, parts: gemm, tail, per_warp: 1, batch: false }
        } else {
            Launch::GemmParts { x: first.x.clone(), format, k: first.k, m: first.m, parts: gemm, tail: None, per_warp: self.shim.parts_per_warp(products), batch: self.shim.batch }
        })
    }

    /// Whether every op that reads `x` — itself, or a view of its whole blocks — reads it as a product's quantized
    /// activation, and none reads it `f32` as well: the producer then writes only the quantized form.
    fn only_projections(&self, x: &str) -> bool {
        let mut names = vec![x.to_string()];
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut readers: Vec<(&Op, String)> = Vec::new();
        while let Some(t) = names.pop() {
            if !seen.insert(t.clone()) {
                continue;
            }
            for op in self.g.ops.iter().filter(|o| o.ins.contains(&t)) {
                match &op.sym {
                    Sym::View { offset } if offset % QBLOCK == 0 => names.push(op.out.clone()),
                    // A row set carrying its rows' quantized form hands its source's rows to the table's readers; the
                    // table it writes into it reads for no one.
                    Sym::Fused(f) if f.class == ClassId::RowWriteQuant => {
                        if op.ins.get(1) == Some(&t) {
                            names.push(op.out.clone());
                        }
                    }
                    _ => readers.push((op, t.clone())),
                }
            }
        }
        readers.iter().all(|(op, t)| {
            let projected = op.ins.iter().enumerate().any(|(k, n)| n == t && self.shim.activation(op, k, self.g).is_some());
            // A part that reads it `f32` beside: an expert mixture's `f32` router, or a product by values — a gated delta
            // layer's gates projected in `f32` beside its GEMV.
            let plain = leaves((*op).clone()).iter().any(|l| {
                (matches!(l.prim(), Some(TensorOpSpec::Moe(_))) && self.values(&l.ins[1])) || gemv(l, self.g).is_some_and(|p| p.x == *t && p.weights == Weights::Values)
            });
            projected && !plain && !matches!(op.sym, Sym::Quantize(_))
        })
    }

    /// A wide chunk's latent attention, expanded: its three launches from the parts that define it absorbed — the key
    /// half computing what the query's absorption did, the value half what the values' slice did, the attention the rest.
    fn latent(&self, op: &Op, f: &Fused) -> Result<Vec<(Launch, Vec<usize>)>, String> {
        let found = leaves(op.clone());
        let attn = found.iter().find(|l| matches!(l.prim(), Some(TensorOpSpec::Attention(_)))).ok_or("a latent attention")?;
        let Some(TensorOpSpec::Attention(a)) = attn.prim() else { unreachable!() };
        let join = found.iter().find(|l| matches!(l.prim(), Some(TensorOpSpec::Join(_))) && l.out == attn.ins[0]).ok_or("the attention's joined query")?;
        let heads_product = |x: &str| found.iter().find(|l| matches!(l.prim(), Some(TensorOpSpec::HeadMatmul(_))) && l.ins[0] == x);
        let absorb = found.iter().find(|l| matches!(l.prim(), Some(TensorOpSpec::HeadMatmul(_))) && l.out == join.ins[0]).ok_or("the query's absorption")?;
        let slice = found.iter().find(|l| l.out == absorb.ins[0]).ok_or("the query's unrotated part")?;
        let expand = heads_product(&attn.out).ok_or("the value half")?;
        let (Some(TensorOpSpec::HeadMatmul(ab)), Some(TensorOpSpec::HeadMatmul(ex))) = (absorb.prim(), expand.prim()) else { unreachable!() };
        let (Some(rk), Some(rv)) = (self.rows_of(&absorb.ins[1]), self.rows_of(&expand.ins[1])) else { return Err("the expansion's checkpoint rows".into()) };
        let pos = match &a.pos {
            Some(RowIndex::Named(_)) => Some(attn.ins[3].clone()),
            _ => None,
        };
        let latent = Latent {
            qr: slice.ins[0].clone(),
            qpp: join.ins[1].clone(),
            kc: attn.ins[1].clone(),
            pos,
            heads: a.h,
            kw: a.hd,
            lora: a.vd,
            nope: ab.a,
            len: a.l,
            rows: a.rows,
            scale: a.scale.map_or(1.0 / (a.hd as f64).sqrt(), |s| s.get()),
            cap: a.softcap.map(|c| c.get()),
            wkb: absorb.ins[1].clone(),
            fk: rk.ty,
            kro: rk.transposed,
            wvb: expand.ins[1].clone(),
            fv: rv.ty,
            vd: ex.b,
            ct: f.parts.iter().map(|p| p.out.clone()).find(|o| *o == op.out).unwrap_or_else(|| op.out.clone()),
        };
        let [kh, vh, attend] = self.shim.latent_launches(&latent).ok_or_else(|| "a latent head wider than one block's shared memory holds".to_string())?;
        let values = found.iter().find(|l| l.out == attn.ins[2]).ok_or("the values' slice")?;
        let (keys_ops, values_ops) = (covered(absorb), covered(values));
        let rest: Vec<usize> = covered(op).into_iter().filter(|k| !keys_ops.contains(k) && !values_ops.contains(k)).collect();
        Ok(vec![(kh, keys_ops), (vh, values_ops), (attend, rest)])
    }
}

/// The lifted ops an op computes, in order.
fn covered(op: &Op) -> Vec<usize> {
    let mut c = op.covers.clone();
    c.sort_unstable();
    c
}

/// The launches every op of `g` runs as, in order, each with the lifted ops it computes, or the first op the lane cannot
/// link and why.
pub fn link(g: &TensorGraph, constants: &BTreeMap<String, Placement>, shim: &LaneShim, io: &StepIo) -> Result<Vec<(Launch, Vec<usize>)>, (usize, String)> {
    let l = Link { g, constants, shim, io };
    let mut out = Vec::new();
    for (k, op) in g.ops.iter().enumerate() {
        out.extend(l.op(op).map_err(|why| (k, why))?);
    }
    Ok(out)
}
