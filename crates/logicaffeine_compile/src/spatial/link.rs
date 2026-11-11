//! The spatial link: each op of the one IR that the shared optimizer leaves the spatial chips
//! ([`super::shim::SpatialShim`]), as the section templates it composes from.
//!
//! The link reads the op and the graph's tensor facts: operand names, the fills the program declares, the shapes it
//! gives. The decisions only the spatial chips make about a whole program are taken once, before any op is linked, as
//! the [`Prelude`]: which tables are laid out as matrices, the interval each product's operands are proven in, and the
//! Q8_0 view each product over a stored weight reads its activation through. [`Prelude::agree_layouts`] bridges a
//! section's layout to the one the prelude chose, where they differ.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use super::ir::*;
use super::passes::from_qwen_ops as q;
use super::passes::from_tensor_op::{
    from_elementwise_with, from_fused_row, from_matmul, from_swiglu_shaped, ElementwiseNames, FusedRowFills,
    FusedRowNames, MatmulNames, SwiGluNames,
};
use super::passes::SpatialReject;
use super::pipeline::{check_datatypes, StageNames};
use super::target::SpatialTarget;
use super::value_range::Interval;
use crate::tensor_graph::front::TensorOpSpec;
use crate::tensor_graph::graph::{Elem, Op, Sym, TensorGraph};
use crate::tensor_recognize::{FusedRowSpec, Fill, RowReduceKind};

/// The decisions the spatial chips make about a whole program before any op of it is linked.
#[derive(Clone, Debug, Default)]
pub struct Prelude {
    /// The tables laid out dense — as a `rows × cols` matrix, a tile row holding `tile` rows — rather than as one flat
    /// row ([`dense_tables`]).
    pub dense: BTreeSet<String>,
    /// The interval each integer tensor is proven to hold, by name: a product's operand precision is the narrowest that
    /// holds its operands'.
    pub ranges: BTreeMap<String, Interval>,
    /// The view each product over a stored Q8_0 weight reads its activation through, by op.
    pub views: BTreeMap<usize, String>,
}

impl Prelude {
    /// The decisions for the ops `linked` of `g`, in the order they are linked: `agreed` the tables a deployment's other
    /// programs lay out dense, `q8` the weights it stores Q8_0, and `chip` the chip of a tensor-parallel deployment each
    /// result is computed on.
    pub fn of(
        g: &TensorGraph,
        linked: &[usize],
        target: &SpatialTarget,
        agreed: &BTreeSet<String>,
        ranges: BTreeMap<String, Interval>,
        q8: &BTreeSet<String>,
        chip: &dyn Fn(&str) -> Option<u16>,
    ) -> Prelude {
        // ★ ONE VIEW OF A VALUE, HOWEVER MANY PRODUCTS READ IT. A value is its name and the count of writes to that name
        // before it, so every product of one value names one view.
        let mut views = BTreeMap::new();
        let mut writes: BTreeMap<&str, usize> = BTreeMap::new();
        for &k in linked {
            let op = &g.ops[k];
            if matches!(op.prim(), Some(TensorOpSpec::Matmul(_))) && q8.contains(&op.ins[1]) {
                let a = op.ins[0].as_str();
                views.insert(k, super::passes::from_q8::q8_rows_name_on(a, writes.get(a).copied().unwrap_or(0), chip(&op.out)));
            }
            *writes.entry(op.out.as_str()).or_default() += 1;
        }
        Prelude { dense: dense_tables(g, linked, target, agreed), ranges, views }
    }

    /// ★ LAYOUT IS AGREED AT THE EDGES, ONCE, FOR EVERY OP. Each op lowers in its own layout — an attention or a rope
    /// over flat rows, a matmul over a matrix. Where a section declares one of the program's matrices in the layout the
    /// prelude did not choose ([`Prelude::dense`]), that section's tensor is renamed to its twin (`X__rows` flat,
    /// `X__dense` dense) and ONE relayout section bridges the two: before the section for a matrix it reads, after it
    /// for one it writes. A twin is made once and shared by every later reader. Carried state is updated in place and
    /// is never bridged.
    pub fn agree_layouts(
        &self,
        g: &TensorGraph,
        ps: Vec<SpatialProgram>,
        twins: &mut BTreeSet<String>,
        passes: &dyn Fn(SpatialProgram) -> Result<SpatialProgram, SpatialReject>,
        target: &SpatialTarget,
    ) -> Result<Agreed, SpatialReject> {
        use q::{relayout_dense_to_rows, relayout_rows_to_dense};
        // The op's own sections agree with one another: only a tensor the op uses in ONE role throughout — read, or
        // written — meets another op at an edge. A carry updated by one section and read by the next is neither.
        let mut roles: BTreeMap<&str, HashSet<TensorRole>> = BTreeMap::new();
        for p in &ps {
            for t in &p.tensors {
                roles.entry(t.name.as_str()).or_default().insert(t.role);
            }
        }
        let (mut before, mut after) = (Vec::new(), Vec::new());
        let mut renames: BTreeMap<String, String> = BTreeMap::new();
        for p in &ps {
            for t in &p.tensors {
                if roles.get(t.name.as_str()).is_some_and(|r| r.len() > 1) {
                    continue;
                }
                let Some((r, c)) = g.tensors.get(&t.name).map(|x| (x.rows, x.cols)).filter(|(r, _)| *r > 1) else {
                    continue;
                };
                let section_dense = match (t.elems.rows, t.elems.cols) {
                    d if d == (r, c) => true,
                    d if d == (1, r * c) => false,
                    _ => continue,
                };
                if section_dense == self.dense.contains(&t.name) || renames.contains_key(&t.name) {
                    continue;
                }
                let twin = format!("{}__{}", t.name, if section_dense { "dense" } else { "rows" });
                match t.role {
                    TensorRole::Input => {
                        if twins.insert(twin.clone()) {
                            let from = (t.name.as_str(), &fill_of(g, &t.name));
                            let bridge = if section_dense {
                                relayout_rows_to_dense(from, &twin, r, c, t.dtype, target)?
                            } else {
                                vec![relayout_dense_to_rows(from, &twin, r, c, t.dtype, target)?]
                            };
                            for b in bridge {
                                before.push(passes(b)?);
                            }
                        }
                    }
                    TensorRole::Output => {
                        twins.insert(twin.clone());
                        let from = (twin.as_str(), &Fill::Unknown);
                        let bridge = if section_dense {
                            vec![relayout_dense_to_rows(from, &t.name, r, c, t.dtype, target)?]
                        } else {
                            relayout_rows_to_dense(from, &t.name, r, c, t.dtype, target)?
                        };
                        for b in bridge {
                            after.push(passes(b)?);
                        }
                    }
                    _ => continue,
                }
                renames.insert(t.name.clone(), twin);
            }
        }
        let sections = ps
            .into_iter()
            .map(|mut p| {
                for t in &mut p.tensors {
                    if let Some(twin) = renames.get(&t.name) {
                        t.name = twin.clone();
                    }
                }
                p
            })
            .collect();
        Ok(Agreed { before, sections, after })
    }
}

/// An op's sections with their layouts agreed ([`Prelude::agree_layouts`]): the bridges that run before them, for the
/// matrices they read, the op's own sections (their tensors renamed to the twins), and the bridges after them, for the
/// matrices they write.
#[derive(Clone, Debug)]
pub struct Agreed {
    pub before: Vec<SpatialProgram>,
    pub sections: Vec<SpatialProgram>,
    pub after: Vec<SpatialProgram>,
}

impl Agreed {
    /// The sections in the order they run: the bridges before, the op's own, the bridges after.
    pub fn into_vec(self) -> Vec<SpatialProgram> {
        self.before.into_iter().chain(self.sections).chain(self.after).collect()
    }
}

/// The tables the ops `linked` of `g` lay out DENSE, with `agreed`, the tables a deployment's other programs lay out
/// dense: every K and V an attention reads in the flash form (and a block of rows' queries and result, which the flash
/// form reads and writes a tile of rows at a time), and every multi-row tensor an op reads or writes as a matrix (a
/// stacked batch through a matmul, a pointwise op over a matrix). The layout is decided here, once per table, so a row
/// write or a row read on a table and the ops that read it whole can never disagree on it.
///
/// ★ A ROW NORM OVER A MATRIX HAS NO LAYOUT OF ITS OWN. It reads and writes in one layout, so it takes its tensors' —
/// the matrix when any of them is dense, a stack a matmul reads — and ties the others to it; left to itself it is the
/// flat row every per-head norm is. The tie runs to a fixed point, since a norm's input may be another norm's output.
pub fn dense_tables(g: &TensorGraph, linked: &[usize], target: &SpatialTarget, agreed: &BTreeSet<String>) -> BTreeSet<String> {
    use q::{attention_reads_dense, full_attention_reads_dense};
    let ops = || linked.iter().map(|&k| &g.ops[k]).filter_map(|op| op.prim().map(|spec| (op, spec)));
    let matrix = |name: &str, n: usize| g.shape_of(name, n).0 > 1;
    let mut dense = BTreeSet::new();
    for (op, spec) in ops() {
        let whole = || op.ins.iter().chain(std::iter::once(&op.out)).collect::<Vec<_>>();
        let flash_tables = |rows: usize| -> Vec<&String> {
            let (first, n) = if rows > 1 { (0, 3) } else { (1, 2) };
            op.ins.iter().skip(first).take(n).chain((rows > 1).then_some(&op.out)).collect()
        };
        let touched: Vec<&String> = match spec {
            // A block of query rows in the flash form — causal or full — reads a TILE of query rows per item and writes
            // its result a tile row at a time: its queries and its result are matrices too.
            TensorOpSpec::Attention(a) if attention_reads_dense(a, target) => flash_tables(a.rows),
            TensorOpSpec::FullAttention(a) if full_attention_reads_dense(a, target) => flash_tables(a.t),
            TensorOpSpec::Matmul(m) => [(&op.ins[0], m.m > 1), (&op.ins[1], m.k > 1), (&op.out, m.m > 1)]
                .into_iter()
                .filter(|(_, rows)| *rows)
                .map(|(t, _)| t)
                .collect(),
            TensorOpSpec::Elementwise(e) if matrix(&op.out, e.n) => whole(),
            TensorOpSpec::SwiGlu(s) if matrix(&op.out, s.total) => whole(),
            TensorOpSpec::Silu(u) if matrix(&op.out, u.n) => whole(),
            TensorOpSpec::IntRequant(u) if matrix(&op.out, u.n) => whole(),
            TensorOpSpec::IntClamp(u) if matrix(&op.out, u.n) => whole(),
            TensorOpSpec::Relu(u) if matrix(&op.out, u.n) => whole(),
            TensorOpSpec::Sigmoid(u) if matrix(&op.out, u.n) => whole(),
            TensorOpSpec::Gelu(u) if matrix(&op.out, u.n) => whole(),
            TensorOpSpec::Scale(u) if matrix(&op.out, u.n) => whole(),
            TensorOpSpec::Softcap(u) if matrix(&op.out, u.n) => whole(),
            TensorOpSpec::Decay(d) if d.rows > 1 => vec![&op.ins[0], &op.out],
            TensorOpSpec::CausalConv(c) if c.rows > 1 => vec![&op.ins[0], &op.out],
            _ => Vec::new(),
        };
        dense.extend(touched.into_iter().cloned());
    }
    dense.extend(agreed.iter().cloned());
    loop {
        let before = dense.len();
        // ★ A BLOCK OF ROWS WRITTEN INTO A MATRIX IS ONE. `Set rows` into a dense table moves whole tile rows of its
        // block, so the block is laid out as the table is.
        for (op, spec) in ops() {
            if let TensorOpSpec::SetRow(r) = spec {
                if r.count > 1 && dense.contains(&op.ins[0]) {
                    dense.insert(op.ins[1].clone());
                }
            }
        }
        for (op, spec) in ops() {
            let TensorOpSpec::FusedRow(f) = spec else { continue };
            if f.rows < 2 || g.shape_of(&op.out, f.rows * f.cols) != (f.rows, f.cols) {
                continue;
            }
            let tied: Vec<&String> = std::iter::once(&op.ins[0])
                .chain(f.residual.and(op.ins.get(1 + usize::from(f.weight.is_some()))))
                .chain(std::iter::once(&op.out))
                .collect();
            if tied.iter().any(|t| dense.contains(*t)) {
                dense.extend(tied.into_iter().cloned());
            }
        }
        if dense.len() == before {
            return dense;
        }
    }
}

/// How an op is linked: alone, as the one program a single-op source is, or as one section of a composition, its
/// float-ness read off the whole program's walk.
///
/// ★ THE LAYOUT IS THE DIFFERENCE, AND IT IS NOT NEGOTIABLE. A single-op program gives every tensor the shape its
/// source declared, which is right when the program is that one op. Inside a composition every activation is FLAT
/// (`1 × n`), because a tensor a matmul writes as `1 × 8` and a per-head rmsnorm reads as `2 × 4` has its element 4 in
/// two different places and no affine map reconciles them. So the ops whose natural layout is 2-D take their flat twin
/// in a section, and only there; everything whose natural layout already IS flat is linked by the one builder both
/// forms share.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Alone,
    /// `floats`: whether the op's datapath is float — its result's kind, or an arg-max's input's — which an
    /// activation's missing fill cannot say.
    Section { floats: bool },
}

/// Op `k` of `g` as the template sections it composes from — one per program the composer chains. Every op is one
/// section except the carried-state blocks, whose read half and write half are separate sections precisely so the order
/// between them is a checkable barrier rather than an assumption (`from_qwen_ops::from_gated_delta`).
///
/// ★ A LINKING COMPUTES ONLY IN DATATYPES ITS CHIP HAS: every section passes [`check_datatypes`], or the op is refused.
pub fn link(
    g: &TensorGraph,
    k: usize,
    prelude: &Prelude,
    form: Form,
    numerics: NumericsPolicy,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let programs = link_unchecked(g, k, prelude, form, numerics, target)?;
    for p in &programs {
        check_datatypes(p, target)?;
    }
    Ok(programs)
}

/// The fill the program declares `tensor` with: [`Fill::Unknown`] for an activation, which no declaration fills.
fn fill_of(g: &TensorGraph, tensor: &str) -> Fill {
    g.tensors.get(tensor).and_then(|t| t.fill.clone()).unwrap_or(Fill::Unknown)
}

/// Whether a declared fill holds floats. A fill the compiler cannot see (`Unknown`, an activation) is taken as float —
/// the datapath every other section's activations ride; inside a composition the walk's own float analysis decides
/// instead ([`Form::Section`]).
fn fill_is_float(fill: &Fill) -> bool {
    !matches!(fill, Fill::Const(_) | Fill::Pattern { .. } | Fill::Overrides { .. })
}

/// A row norm's operands by role. The spec's operand order is `[x, weight?, residual?, bias?]` — the present ones, in
/// that order — which is exactly how the op lists them.
fn row_norm_operands(op: &Op, g: &TensorGraph, f: &FusedRowSpec) -> (FusedRowNames, FusedRowFills) {
    let mut rest = op.ins.iter().skip(1);
    let mut slot = |have: bool| have.then(|| rest.next().cloned()).flatten();
    let (weight, residual, bias) = (slot(f.weight.is_some()), slot(f.residual.is_some()), slot(f.bias.is_some()));
    let fills = FusedRowFills {
        x: fill_of(g, &op.ins[0]),
        weight: weight.as_deref().map(|t| fill_of(g, t)),
        residual: residual.as_deref().map(|t| fill_of(g, t)),
        bias: bias.as_deref().map(|t| fill_of(g, t)),
    };
    (FusedRowNames { x: op.ins[0].clone(), y: op.out.clone(), weight, residual, bias }, fills)
}

fn link_unchecked(
    g: &TensorGraph,
    k: usize,
    prelude: &Prelude,
    form: Form,
    numerics: NumericsPolicy,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let op = &g.ops[k];
    let Sym::Prim(spec) = &op.sym else {
        return Err(SpatialReject::NotLowered("an op the front end did not lift — the spatial chips link every fused class by its parts"));
    };
    let fill = |i: usize| fill_of(g, &op.ins[i]);
    let names = StageNames { out: op.out.clone(), ins: op.ins.clone(), fills: (0..op.ins.len()).map(fill).collect() };
    let shape = |n: usize| g.shape_of(&op.out, n);
    let dense = |t: &str| prelude.dense.contains(t);
    // The datapath of an op whose element kind a section reads off the walk, and alone off its table's declared fill.
    let floats = || match form {
        Form::Section { floats } => floats,
        Form::Alone => fill_is_float(&names.fills[0]),
    };
    let one = |r: Result<SpatialProgram, SpatialReject>| r.map(|p| vec![p]);
    match spec {
        TensorOpSpec::Matmul(m) => {
            let names = MatmulNames { a: op.ins[0].clone(), b: op.ins[1].clone(), c: op.out.clone() };
            let fills = (fill(0), fill(1));
            if let Some(view) = prelude.views.get(&k).filter(|_| super::passes::from_q8::q8_gemv_applies(m, target)) {
                if let Some(sections) = super::passes::from_q8::q8_gemv(m, &names, (&fills.0, &fills.1), view, target)? {
                    return Ok(sections);
                }
            }
            let ranges = (prelude.ranges.get(&names.a).copied(), prelude.ranges.get(&names.b).copied());
            one(from_matmul(m, &names, (&fills.0, &fills.1), ranges, numerics, target))
        }
        // An activation whose producer is another section carries no declared fill, so the integer shortcut in the
        // elementwise builder cannot see that the data is float. The whole-program walk CAN — it propagates float-ness
        // from the declarations through the op graph — so a section says so.
        TensorOpSpec::Elementwise(e) => {
            let names = ElementwiseNames { a: op.ins[0].clone(), b: op.ins[1].clone(), c: op.out.clone() };
            let float_activations = form == (Form::Section { floats: true });
            one(from_elementwise_with(e, &names, (&fill(0), &fill(1)), float_activations, shape(e.n), target))
        }
        TensorOpSpec::SwiGlu(s) => {
            let names = SwiGluNames { gate: op.ins[0].clone(), up: op.ins[1].clone(), y: op.out.clone() };
            one(from_swiglu_shaped(s, &names, (&fill(0), &fill(1)), shape(s.total), target))
        }
        TensorOpSpec::FusedRow(f) => {
            let (row_names, fills) = row_norm_operands(op, g, f);
            let natural = (f.rows, f.cols);
            // A multi-row norm in a section is the flat row every per-head norm is, unless its result is a dense table.
            let layout = match form {
                Form::Alone => natural,
                Form::Section { .. } if f.rows > 1 && !dense(&op.out) => (1, f.rows * f.cols),
                Form::Section { .. } => shape(f.rows * f.cols),
            };
            // ★ A ROW WIDER THAN A SITE CAN HOLD STREAMS. The whole-row chain keeps `cols_t` tiles resident and is
            // refused outright past the accumulator budget, which at Qwen3.8's real `d = 5120` is every row in the
            // model. The blocked form says the same arithmetic in four sections that each fit, so the choice here is
            // about the WIDTH, never about the operation — and a stack of such rows, a batch's `B × d`, is the same four
            // sections over the matrix.
            //
            // ★ THE FLAT TWIN IS THE WHOLE-PROGRAM LAYOUT DISCIPLINE, NOT AN OPTIMIZATION. A 2x4 per-head norm that stays
            // two-dimensional produces `qr3` shaped (2, 4) while every other section still reads it flat as (1, 8), and
            // compose refuses the conflict. Layout is an AGREEMENT between sections, so the rule cannot depend on any one
            // section's width — it is what the tensors' other readers and writers need.
            // ★ AND A ROW ONE SITE CANNOT HOLD STREAMS TOO. Inside the page, a part of small tiles runs out of a site's
            // memory first (the RTX 3060's 16 × 16 tiles at 28 tiles of a row), so a whole-row form that fits no site
            // takes the streamed sections as a row past the page does.
            let plain = f.weight.is_none() && f.residual.is_none() && f.bias.is_none();
            let streamed: Option<fn(&FusedRowSpec, &StageNames, &SpatialTarget) -> Result<Vec<SpatialProgram>, SpatialReject>> = match f.reduce {
                RowReduceKind::RmsNorm if f.bias.is_none() => Some(q::blocked_rms_norm),
                RowReduceKind::LayerNorm if f.weight_rows == 1 => Some(q::blocked_layer_norm),
                RowReduceKind::Softmax if plain && f.rows == 1 && !q::row_needs_blocking(f.cols, target) => Some(q::streamed_softmax),
                _ => None,
            };
            let whole_or_streamed = |whole: Result<SpatialProgram, SpatialReject>| -> Result<Vec<SpatialProgram>, SpatialReject> {
                match (whole?, streamed) {
                    (w, Some(stream)) if matches!(form, Form::Section { .. }) && layout == natural && !q::fits_a_site(&w, target) => {
                        stream(f, &names, target)
                    }
                    (w, _) => Ok(vec![w]),
                }
            };
            match form {
                Form::Section { .. } if layout == natural && q::row_needs_blocking(f.cols, target) && streamed.is_some() => {
                    streamed.expect("a streamed form")(f, &names, target)
                }
                // A norm in lanes takes the flat form over a dense table too: its rows are a token's lanes, which no tile
                // row of the table holds, and the prelude bridges the table's layout to the flat row
                // ([`Prelude::agree_layouts`]).
                Form::Section { .. } if f.rows > 1 && (layout == (1, f.rows * f.cols) || f.weight_rows > 1) => {
                    one(q::from_row_norm_flat(f, &names, target))
                }
                Form::Section { .. } if layout != natural => Err(SpatialReject::NotLowered(
                    "a row norm whose rows cut across the rows of the matrix it normalizes — neither the matrix's tiling nor the flat row's",
                )),
                // A norm in lanes reads row `r`'s weight at row `r mod weight_rows`, which the flat form's bands and
                // items carry and the tile-row form's one broadcast weight row does not.
                Form::Alone if f.weight_rows > 1 => one(q::from_row_norm_flat(f, &names, target)),
                _ => whole_or_streamed(from_fused_row(f, &row_names, &fills, target)),
            }
        }
        TensorOpSpec::Rope(spec) => q::from_rope(spec, &names, target),
        TensorOpSpec::Silu(spec) => one(q::from_silu(spec, &names, shape(spec.n), target)),
        TensorOpSpec::IntRequant(spec) => one(q::from_int_requant(spec, &names, shape(spec.n), target)),
        TensorOpSpec::IntClamp(spec) => one(q::from_int_clamp(spec, &names, shape(spec.n), target)),
        TensorOpSpec::Relu(spec) => one(q::from_relu(spec, &names, shape(spec.n), target)),
        TensorOpSpec::LutAttention(spec) => q::from_lut_attention(spec, &names, target),
        TensorOpSpec::Sigmoid(spec) => one(q::from_sigmoid(spec, &names, shape(spec.n), target)),
        TensorOpSpec::Gelu(spec) => one(q::from_gelu(spec, &names, shape(spec.n), target)),
        TensorOpSpec::Scale(spec) => one(q::from_scale(spec, &names, shape(spec.n), target)),
        TensorOpSpec::Softcap(spec) => one(q::from_softcap(spec, &names, shape(spec.n), target)),
        TensorOpSpec::Decay(spec) => one(q::from_decay(spec, &names, target)),
        TensorOpSpec::L2Norm(spec) => one(q::from_l2norm(spec, &names, target)),
        TensorOpSpec::Transpose(spec) => {
            let floats = match form {
                Form::Section { floats } => floats,
                Form::Alone => true,
            };
            one(q::from_transpose(spec, &names, floats, target))
        }
        TensorOpSpec::HeadSlice(spec) => one(q::from_head_slice(spec, &names, target)),
        TensorOpSpec::Join(spec) => one(q::from_join(spec, &names, target)),
        TensorOpSpec::LaneMean(spec) => one(q::from_lane_mean(spec, &names, target)),
        TensorOpSpec::LaneSpread(spec) => one(q::from_lane_spread(spec, &names, target)),
        TensorOpSpec::LaneSums(spec) => one(q::from_lane_sums(spec, &names, target)),
        TensorOpSpec::SignedRoot(spec) => one(q::from_signed_root(spec, &names, shape(spec.n), target)),
        TensorOpSpec::NgramIds(spec) => q::from_ngram_ids(spec, &names, target),
        TensorOpSpec::HeadMatmul(spec) => one(q::from_head_matmul(spec, &names, target)),
        TensorOpSpec::CausalConv(spec) => q::from_causal_conv(spec, &names, target),
        // The tables of a block step's operands and output laid out as matrices; a one-row step reads none of them as a
        // table.
        TensorOpSpec::GatedDelta(spec) => {
            let tables: BTreeSet<String> = op.ins.iter().chain(std::iter::once(&op.out)).filter(|t| dense(t)).cloned().collect();
            q::from_gated_delta(spec, &names, &tables, target)
        }
        TensorOpSpec::Attention(spec) => q::from_attention(spec, &names, target),
        TensorOpSpec::FullAttention(spec) => q::from_full_attention(spec, &names, target),
        TensorOpSpec::Moe(spec) => q::from_moe(spec, &names, target),
        TensorOpSpec::RowGather(spec) => one(q::from_row_gather(spec, &names, floats(), dense(&op.ins[0]), target)),
        TensorOpSpec::RowsGather(spec) => one(q::from_rows_gather(spec, &names, floats(), dense(&op.ins[0]), target)),
        TensorOpSpec::ArgMax(spec) => q::from_argmax(spec, &names, floats(), target),
        TensorOpSpec::SetRow(spec) => {
            let held = (floats() && is_cache(g, &op.ins[0])).then(|| q::cache_precision(target)).transpose()?;
            one(q::from_set_row(spec, &names, floats(), dense(&op.ins[0]), held, target))
        }
        TensorOpSpec::Scatter(_) if dense(&op.ins[0]) => Err(SpatialReject::NotLowered(
            "a scatter into a table another op reads as a matrix — the scatter rewrites it as one flat row",
        )),
        TensorOpSpec::Scatter(_) if g.tensors.get(&op.ins[2]).is_some_and(|t| t.elem == Elem::Float) => {
            Err(SpatialReject::NotLowered("a scatter whose row indices are Reals"))
        }
        TensorOpSpec::Scatter(spec) => one(q::from_scatter(spec, &names, floats(), target)),
        TensorOpSpec::Map(spec) => {
            let reals: Vec<bool> = op.ins.iter().map(|t| g.tensors.get(t).is_none_or(|x| x.elem == Elem::Float)).collect();
            one(q::from_map(spec, &names, &reals, target))
        }
    }
}

/// Whether an attention of `g` reads `table` as its keys or its values: a cache, held at [`q::cache_precision`] by
/// every attention that reads it and so by every row write into it.
fn is_cache(g: &TensorGraph, table: &str) -> bool {
    g.ops.iter().any(|op| {
        matches!(op.prim(), Some(TensorOpSpec::Attention(_) | TensorOpSpec::FullAttention(_)))
            && op.ins.get(1..3).is_some_and(|kv| kv.iter().any(|t| t == table))
    })
}
