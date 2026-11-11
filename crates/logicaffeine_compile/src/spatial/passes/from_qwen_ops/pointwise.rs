//! The element-wise family: maps of one row, their blocked and matrix forms, and the activations.

use super::*;

// ── The element-wise family: one `Generic`, one body ──────────────────────────────────────────────

/// A unary map `y = f(x)` over a `rows × cols` tensor: a row in the row form below, a matrix tile by tile.
pub(super) fn unary_map(
    name: &str,
    kind: &str,
    datapath: Datapath,
    body: impl Fn(Box<TensorBody>) -> Box<TensorBody>,
    x: (&str, &Fill),
    out: &str,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, n) = shape;
    if rows > 1 {
        return matrix_unary_map(name, kind, datapath, body, x, out, shape, target);
    }
    let cols_t = tiles_of(n, target);
    if cols_t > row_block_tiles(target) {
        return blocked_unary_map(name, kind, datapath, body, x, out, n, target);
    }
    let mut b = FlatBuild::on(datapath, target)?;
    let xin = b.input(x.0, n, x.1, target);
    let out_buf = b.t.buffer("out", cols_t, b.precision);
    let w = wide(cols_t, target);
    let s = b.compute(
        kind,
        flat_space(n),
        vec![Slot {
            buf: xin,
            map: at(0),
        }],
        ident_map(w),
        *body(op(0)),
        out_buf,
    );
    let claim = claim_of(kind, &body(op(0)), b.precision);
    let whole = b.finish(
        out,
        n,
        vec![s],
        out_buf,
        format!("{name}_{n}"),
        claim,
        target,
    )?;
    whole_row_or(whole, || blocked_unary_map(name, kind, datapath, body, x, out, n, target), target)
}

/// `y = body(a, b)` over a `1 × n` row a site holds whole: one item over the row, both operands loaded whole and mapped
/// together — the form [`unary_map`] gives a row, for an element-wise op of two rows. The caller has chosen the
/// precision and the claim (the element-wise lowering, from its operands' fills).
#[allow(clippy::too_many_arguments)]
pub(crate) fn row_binary_map(
    name: String,
    kind: &str,
    body: TensorBody,
    a: (&str, &Fill),
    b: (&str, &Fill),
    out: &str,
    n: usize,
    precision: Precision,
    claim: NumericsClaim,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let cols_t = tiles_of(n, target);
    let mut fb = FlatBuild::at(precision, target)?;
    let ain = fb.input(a.0, n, a.1, target);
    let bin = fb.input(b.0, n, b.1, target);
    let out_buf = fb.t.buffer("out", cols_t, precision);
    let w = wide(cols_t, target);
    let s = fb.compute(
        kind,
        flat_space(n),
        vec![Slot { buf: ain, map: at(0) }, Slot { buf: bin, map: at(0) }],
        ident_map(w),
        body,
        out_buf,
    );
    fb.finish(out, n, vec![s], out_buf, name, claim, target)
}

/// ★ A POINTWISE MAP HAS NO CROSS-ELEMENT DEPENDENCE, so a row wider than a site can hold is simply
/// WORK, not a problem: one item per tile, a one-tile page, and the row streams. This is the same shape
/// `from_tensor_op::from_elementwise` has always had.
///
/// The whole-row form above stays for narrow rows on purpose — see `whole_row`: `place` clones the
/// program per site, so splitting a two-tile vector into two items buys nothing and doubles the stages.
/// The rule is therefore about the ROW, not about the operation: one item while the row fits the site's
/// budget, one item per tile once it does not.
#[allow(clippy::too_many_arguments)]
pub(super) fn blocked_unary_map(
    name: &str,
    kind: &str,
    datapath: Datapath,
    body: impl Fn(Box<TensorBody>) -> Box<TensorBody>,
    x: (&str, &Fill),
    out: &str,
    n: usize,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let precision = datapath.precision(target)?;
    let cols_t = tiles_of(n, target);
    let mut t = Template::new(target)?;
    let xt = t.tensor(
        x.0,
        TensorRole::Input,
        1,
        n,
        precision,
        crate::spatial::passes::from_tensor_op::init_of(x.1, n),
    );
    let yt = t.tensor(out, TensorRole::Output, 1, n, precision, TensorInit::Zero);
    let xin = t.buffer("in_x", 1, precision);
    let out_buf = t.buffer("out", 1, precision);
    let lx = t.load("load_x", xt, vec![(LV_ITEM_X, 1)], 1, xin);
    let w = wide(1, target);
    let s = t.stage(
        kind,
        StageKind::Compute(ComputeOp::Generic {
            iters: flat_space(w),
            ins: vec![ident_map(w)],
            out: ident_map(w),
            body: *body(op(0)),
        }),
        vec![xin],
        vec![out_buf],
    );
    let st = t.store(
        &format!("store_{out}"),
        yt,
        vec![(LV_ITEM_X, 1)],
        1,
        out_buf,
    );
    t.edge(lx, xin, s);
    t.edge(s, out_buf, st);
    t.finish(
        format!("{name}_{n}"),
        items_per_tile(cols_t),
        AccPolicy::for_result(precision),
        claim_of(kind, &body(op(0)), precision),
        target,
    )
}

/// A unary map over a MATRIX: the tensors keep the program's `rows × cols`, and every tile of the 2-D tile grid
/// is one work item mapped over its full `tile × tile` page — the tiling a matmul reads the same tensor in, so a
/// composition joins the two without a layout conversion.
#[allow(clippy::too_many_arguments)]
pub(super) fn matrix_unary_map(
    name: &str,
    kind: &str,
    datapath: Datapath,
    body: impl Fn(Box<TensorBody>) -> Box<TensorBody>,
    x: (&str, &Fill),
    out: &str,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols) = shape;
    let precision = datapath.precision(target)?;
    let mut t = Template::new(target)?;
    let xt = t.tensor(
        x.0,
        TensorRole::Input,
        rows,
        cols,
        precision,
        crate::spatial::passes::from_tensor_op::init_of(x.1, rows * cols),
    );
    let yt = t.tensor(
        out,
        TensorRole::Output,
        rows,
        cols,
        precision,
        TensorInit::Zero,
    );
    let (rows_t, cols_t) = (
        rows.div_ceil(target.tile.rows),
        cols.div_ceil(target.tile.cols),
    );
    let steps = crate::spatial::passes::from_tensor_op::tile_grid_steps(rows_t, cols_t);
    let xin = t.buffer("in_x", 1, precision);
    let out_buf = t.buffer("out", 1, precision);
    let lx = t.load("load_x", xt, steps.clone(), 1, xin);
    let w = target.tile.cols;
    let s = t.stage(
        kind,
        StageKind::Compute(ComputeOp::Generic {
            iters: crate::spatial::passes::from_tensor_op::space(target, w, None),
            ins: vec![ident_map(w)],
            out: ident_map(w),
            body: *body(op(0)),
        }),
        vec![xin],
        vec![out_buf],
    );
    let st = t.store(&format!("store_{out}"), yt, steps, 1, out_buf);
    t.edge(lx, xin, s);
    t.edge(s, out_buf, st);
    t.finish(
        format!("{name}_{rows}x{cols}"),
        crate::spatial::passes::from_tensor_op::items_per_tile_grid(rows_t, cols_t),
        AccPolicy::for_result(precision),
        claim_of(kind, &body(op(0)), precision),
        target,
    )
}

/// `y = silu(x)` — `x / (1 + e^-x)`, one fused SFPU kind.
pub fn from_silu(
    spec: &SiluSpec,
    names: &StageNames,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    unary_map(
        "silu",
        "silu",
        Datapath::Float,
        |x| silu_expr(x),
        names.operand(0),
        &names.out,
        pointwise_shape(shape, spec.n),
        target,
    )
}

/// `y = (x · M) >> S` — an int8 model's requant: the exact product, then an arithmetic shift.
pub fn from_int_requant(
    spec: &IntRequantSpec,
    names: &StageNames,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (m, s) = (spec.m as f64, spec.s);
    unary_map(
        "int_requant",
        "int_requant",
        Datapath::Int,
        |x| Box::new(TensorBody::Shr { arg: mul(x, konst(m)), by: s }),
        names.operand(0),
        &names.out,
        pointwise_shape(shape, spec.n),
        target,
    )
}

/// `y = min(max(x, LO), HI)` — saturation into `[LO, HI]`.
pub fn from_int_clamp(
    spec: &IntClampSpec,
    names: &StageNames,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (lo, hi) = (spec.lo as f64, spec.hi as f64);
    unary_map(
        "int_clamp",
        "int_clamp",
        Datapath::Int,
        |x| Box::new(TensorBody::Min(Box::new(TensorBody::Max(x, konst(lo))), konst(hi))),
        names.operand(0),
        &names.out,
        pointwise_shape(shape, spec.n),
        target,
    )
}

/// `y = max(x, 0)`, in the tensor's element kind.
pub fn from_relu(
    spec: &ReluSpec,
    names: &StageNames,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    unary_map(
        "relu",
        "relu",
        if spec.int { Datapath::Int } else { Datapath::Float },
        |x| Box::new(TensorBody::Max(x, konst(0.0))),
        names.operand(0),
        &names.out,
        pointwise_shape(shape, spec.n),
        target,
    )
}

/// The shape a pointwise op maps over: the program's `rows × cols` when it accounts for the `n` elements the
/// recognizer counted, a `1 × n` row otherwise.
pub(super) fn pointwise_shape(shape: (usize, usize), n: usize) -> (usize, usize) {
    if shape.0 * shape.1 == n {
        shape
    } else {
        (1, n)
    }
}

/// `y = sigmoid(x)` — `1/(1 + e^-x)`, the reciprocal of the silu denominator. No new SFPU kind: `Exp`
/// and `Recip` already exist and the composition of the two IS the sigmoid.
pub fn from_sigmoid(
    spec: &SigmoidSpec,
    names: &StageNames,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    unary_map(
        "sigmoid",
        "sigmoid",
        Datapath::Float,
        |x| {
            unary(
                SfpuKind::Recip,
                add(konst(1.0), unary(SfpuKind::Exp, sub(konst(0.0), x))),
            )
        },
        names.operand(0),
        &names.out,
        pointwise_shape(shape, spec.n),
        target,
    )
}

/// `y = gelu(x)` — ggml's tanh approximation in its one-exponential form `x / (1 + e^-2z)` with
/// `z = √(2/π)·(x + 0.044715·x³)`, exactly as the parser desugars it (same arrangement, same tails).
pub fn from_gelu(
    spec: &GeluSpec,
    names: &StageNames,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    const SQRT_2_OVER_PI: f64 = 0.797_884_560_802_865_4;
    const K: f64 = 0.044_715;
    unary_map(
        "gelu",
        "gelu",
        Datapath::Float,
        |x| {
            let cube = mul(x.clone(), mul(x.clone(), x.clone()));
            let z = mul(konst(SQRT_2_OVER_PI), add(x.clone(), mul(konst(K), cube)));
            let two_z = mul(konst(2.0), z);
            mul(
                x,
                unary(
                    SfpuKind::Recip,
                    add(konst(1.0), unary(SfpuKind::Exp, sub(konst(0.0), two_z))),
                ),
            )
        },
        names.operand(0),
        &names.out,
        pointwise_shape(shape, spec.n),
        target,
    )
}

/// `y = x·C` — one multiply by the program's constant.
pub fn from_scale(
    spec: &ScaleSpec,
    names: &StageNames,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let c = spec.factor.get();
    unary_map(
        "scale",
        "scale",
        Datapath::Float,
        |x| mul(x, konst(c)),
        names.operand(0),
        &names.out,
        pointwise_shape(shape, spec.n),
        target,
    )
}

/// `y = C·tanh(x/C)` — each element soft-capped (`capped`'s form: one exponential and one reciprocal).
pub fn from_softcap(
    spec: &SoftcapSpec,
    names: &StageNames,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let c = spec.cap.get();
    unary_map(
        "softcap",
        "softcap",
        Datapath::Float,
        |x| capped(x, c),
        names.operand(0),
        &names.out,
        pointwise_shape(shape, spec.n),
        target,
    )
}

/// `y = sign(x)·√max(|x|, ε)` — each element's signed root, written `sign(x) · m · rsqrt(m)` with `m = max(max(x, −x), ε)`:
/// below the floor the root is `±√ε` by the sign of `x`, and `0` at zero, as the VM's formula is.
pub fn from_signed_root(
    spec: &SignedRootSpec,
    names: &StageNames,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let eps = spec.eps.get();
    unary_map(
        "signed_root",
        "signed_root",
        Datapath::Float,
        |x| {
            let magnitude = Box::new(TensorBody::Max(x.clone(), sub(konst(0.0), x.clone())));
            let floored = Box::new(TensorBody::Max(magnitude, konst(eps)));
            let root = mul(floored.clone(), unary(SfpuKind::Rsqrt { eps_nano: 0, over: 1 }, floored));
            mul(unary(SfpuKind::Sign, x), root)
        },
        names.operand(0),
        &names.out,
        pointwise_shape(shape, spec.n),
        target,
    )
}

/// `G[t][l] = Σ_c X[t][l·C + c]` — each lane of each row summed on its own: the folded axis is the ROW (`e` over a lane's
/// `C` entries) and output column `u` (row `u / L`, lane `u mod L`) reads `X[u·C + e]`, a band an output.
pub fn from_lane_sums(spec: &LaneSumsSpec, names: &StageNames, target: &SpatialTarget) -> Result<SpatialProgram, SpatialReject> {
    let (units, cols) = (spec.rows * spec.lanes, spec.cols);
    if units == 0 || cols == 0 {
        return Err(SpatialReject::NotLowered("lane sums of nothing"));
    }
    let mut f = FlatBuild::new(target)?;
    let x = f.input(names.operand(0).0, units * cols, names.operand(0).1, target);
    let out_buf = f.t.buffer("out", tiles_of(units, target), f.precision);
    let tile_elems = target.tile.rows * target.tile.cols;
    let ones = f.t.tensor(
        "lane_sums_scaler",
        TensorRole::Param,
        target.tile.rows,
        target.tile.cols,
        f.precision,
        TensorInit::FloatValues(vec![1.0f64.to_bits(); tile_elems]),
    );
    let ones_in = f.t.buffer("in_scaler", 1, f.precision);
    let ls = f.t.load("load_scaler", ones, vec![], 1, ones_in);
    f.loads.push((ls, ones_in));
    let iters = vec![
        IterSpec { var: LV_GEN_ROW, extent: cols, kind: IterKind::Reduce { pool: ReducePool::Sum } },
        IterSpec { var: LV_GEN_COL, extent: units, kind: IterKind::Parallel },
    ];
    let mut slots = vec![Slot { buf: ones_in, map: const_map() }];
    let mut bands = Vec::with_capacity(units);
    for u in 0..units {
        slots.push(Slot { buf: x, map: ElemAccess { base: (u * cols) as i64, terms: vec![(LV_GEN_ROW, 1)], indirect: None } });
        bands.push((u + 1, *op(slots.len() - 1)));
    }
    let s = f.compute("lane_sums", iters, slots, per_col_map(), banded(bands), out_buf);
    let claim = approx("lane sums", f.precision);
    f.finish(&names.out, units, vec![s], out_buf, format!("lane_sums_{}x{}x{cols}", spec.rows, spec.lanes), claim, target)
}

/// `M = the mean of X over L lanes` — row `t` of `M` at `[t·C, (t+1)·C)`, column `c` the sum of its lanes
/// `X[t·L·C + l·C + c]` times `1/L`: a band per row, each lane `l` read at `c + t·(L − 1)·C + l·C`, the lanes summed in
/// order as the VM sums them.
pub fn from_lane_mean(spec: &LaneMeanSpec, names: &StageNames, target: &SpatialTarget) -> Result<SpatialProgram, SpatialReject> {
    let (rows, lanes, cols) = (spec.rows, spec.lanes, spec.cols);
    let n = rows * cols;
    if n == 0 || lanes == 0 {
        return Err(SpatialReject::NotLowered("a lane mean of nothing"));
    }
    if row_needs_blocking(rows * lanes * cols, target) {
        return blocked_lane_mean(spec, names, target);
    }
    let mut b = FlatBuild::columns(n, target)?;
    let xin = b.input(names.operand(0).0, rows * lanes * cols, names.operand(0).1, target);
    let precision = b.precision;
    let out_buf = b.result_page("out", n, precision, target);
    let mut slots = Vec::with_capacity(rows * lanes);
    let mut bands = Vec::with_capacity(rows);
    for t in 0..rows {
        let mut sum: Option<Box<TensorBody>> = None;
        for l in 0..lanes {
            slots.push(Slot { buf: xin, map: at((t * (lanes - 1) * cols + l * cols) as i64) });
            let lane = op(slots.len() - 1);
            sum = Some(match sum {
                Some(s) => add(s, lane),
                None => lane,
            });
        }
        let sum = sum.expect("a lane");
        bands.push(((t + 1) * cols, *mul(sum, konst(1.0 / lanes as f64))));
    }
    let chain = b.pointwise("lane_mean", n, slots, banded(bands), out_buf, target)?;
    b.finish(&names.out, n, chain, out_buf, format!("lane_mean_{rows}x{lanes}x{cols}"), approx("lane mean", precision), target)
}

/// A lane mean whose lanes are wider than a site's page: one item a tile of the result — row `t`'s column tile `j`, item
/// `(t, j)` — loading that tile of each of its `L` lanes (lane `l`'s at `t·L·C + l·C + j` tiles in), their mean pointwise.
/// A lane must start on a tile boundary (`C` a whole number of tiles) for its tile to be one block of the input; a lane
/// that does not is refused by name.
pub(super) fn blocked_lane_mean(spec: &LaneMeanSpec, names: &StageNames, target: &SpatialTarget) -> Result<SpatialProgram, SpatialReject> {
    let (rows, lanes, cols) = (spec.rows, spec.lanes, spec.cols);
    if cols % target.tile.cols != 0 {
        return Err(SpatialReject::NotLowered("a lane mean wider than a site's page whose lanes do not start on tile boundaries"));
    }
    let lane_t = cols / target.tile.cols;
    let precision = elem_precision(target)?;
    let (x_name, x_fill) = names.operand(0);
    let (n_in, n) = (rows * lanes * cols, rows * cols);
    let mut t = Template::new(target)?;
    let xt = t.tensor(x_name, TensorRole::Input, 1, n_in, precision, crate::spatial::passes::from_tensor_op::init_of(x_fill, n_in));
    let yt = t.tensor(&names.out, TensorRole::Output, 1, n, precision, TensorInit::Zero);
    let (mut ins, mut loads) = (Vec::with_capacity(lanes), Vec::with_capacity(lanes));
    for l in 0..lanes {
        let buf = t.buffer(&format!("in_lane{l}"), 1, precision);
        loads.push(t.load_block_at(&format!("load_lane{l}"), xt, l * lane_t, vec![(LV_ITEM_Y, lanes * lane_t), (LV_ITEM_X, 1)], 1, 1, 1, buf));
        ins.push(buf);
    }
    let ob = t.buffer("out", 1, precision);
    let w = wide(1, target);
    let sum = (1..lanes).fold(op(0), |acc, l| add(acc, op(l)));
    let s = t.stage(
        "lane_mean",
        StageKind::Compute(ComputeOp::Generic { iters: flat_space(w), ins: vec![ident_map(w); lanes], out: ident_map(w), body: *mul(sum, konst(1.0 / lanes as f64)) }),
        ins.clone(),
        vec![ob],
    );
    let st = t.store_block("store_y", yt, 0, vec![(LV_ITEM_Y, lane_t), (LV_ITEM_X, 1)], 1, 1, 1, ob);
    for (l, b) in loads.iter().zip(&ins) {
        t.edge(*l, *b, s);
    }
    t.edge(s, ob, st);
    let items = (0..rows * lane_t)
        .map(|i| WorkItem { id: WorkItemId::new(i), coord: (i / lane_t, i % lane_t), out: TileRect { row0_t: 0, col0_t: i, h_t: 1, w_t: 1 } })
        .collect();
    t.finish(format!("lane_mean_{rows}x{lanes}x{cols}"), items, AccPolicy::for_result(precision), approx("lane mean", precision), target)
}

/// `P = O spread over L lanes [by S]` — unit `u` (row `u / L`, lane `u mod L`) at `[u·C, (u+1)·C)` holds row `u / L` of
/// `O`, read at `c − (u − u / L)·C`, times the unit's one weight `S[u]` with `by S`: a band per unit.
pub fn from_lane_spread(spec: &LaneSpreadSpec, names: &StageNames, target: &SpatialTarget) -> Result<SpatialProgram, SpatialReject> {
    let (rows, lanes, cols) = (spec.rows, spec.lanes, spec.cols);
    let units = rows * lanes;
    let n = units * cols;
    if n == 0 {
        return Err(SpatialReject::NotLowered("a lane spread of nothing"));
    }
    let mut b = FlatBuild::columns(n, target)?;
    let xin = b.input(names.operand(0).0, rows * cols, names.operand(0).1, target);
    let weights = spec.by.map(|_| b.input(names.operand(1).0, units, names.operand(1).1, target));
    let precision = b.precision;
    let out_buf = b.result_page("out", n, precision, target);
    let mut slots = Vec::with_capacity(2 * units);
    let mut bands = Vec::with_capacity(units);
    for u in 0..units {
        let t = u / lanes;
        slots.push(Slot { buf: xin, map: at((t * cols) as i64 - (u * cols) as i64) });
        let row = op(slots.len() - 1);
        let body = match weights {
            Some(s) => {
                slots.push(Slot { buf: s, map: fixed(u as i64) });
                mul(row, op(slots.len() - 1))
            }
            None => row,
        };
        bands.push(((u + 1) * cols, *body));
    }
    let chain = b.pointwise("lane_spread", n, slots, banded(bands), out_buf, target)?;
    let claim = if spec.by.is_some() { approx("lane spread", precision) } else { NumericsClaim::Exact };
    b.finish(&names.out, n, chain, out_buf, format!("lane_spread_{rows}x{lanes}x{cols}"), claim, target)
}

/// [`blocked_binary_map`] over a MATRIX: one item per tile of the 2-D grid, as [`matrix_unary_map`].
pub(super) fn matrix_binary_map(
    name: &str,
    kind: &str,
    body: impl Fn(Box<TensorBody>, Box<TensorBody>) -> Box<TensorBody>,
    a: (&str, &Fill),
    b: (&str, &Fill),
    out: &str,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols) = shape;
    let precision = elem_precision(target)?;
    let mut t = Template::new(target)?;
    let at_ = t.tensor(
        a.0,
        TensorRole::Input,
        rows,
        cols,
        precision,
        crate::spatial::passes::from_tensor_op::init_of(a.1, rows * cols),
    );
    let bt = t.tensor(
        b.0,
        TensorRole::Input,
        rows,
        cols,
        precision,
        crate::spatial::passes::from_tensor_op::init_of(b.1, rows * cols),
    );
    let yt = t.tensor(
        out,
        TensorRole::Output,
        rows,
        cols,
        precision,
        TensorInit::Zero,
    );
    let (rows_t, cols_t) = (
        rows.div_ceil(target.tile.rows),
        cols.div_ceil(target.tile.cols),
    );
    let steps = crate::spatial::passes::from_tensor_op::tile_grid_steps(rows_t, cols_t);
    let ain = t.buffer("in_a", 1, precision);
    let bin = t.buffer("in_b", 1, precision);
    let ob = t.buffer("out", 1, precision);
    let la = t.load("load_a", at_, steps.clone(), 1, ain);
    let lb = t.load("load_b", bt, steps.clone(), 1, bin);
    let w = target.tile.cols;
    let s = t.stage(
        kind,
        StageKind::Compute(ComputeOp::Generic {
            iters: crate::spatial::passes::from_tensor_op::space(target, w, None),
            ins: vec![ident_map(w), ident_map(w)],
            out: ident_map(w),
            body: *body(op(0), op(1)),
        }),
        vec![ain, bin],
        vec![ob],
    );
    let st = t.store(&format!("store_{out}"), yt, steps, 1, ob);
    t.edge(la, ain, s);
    t.edge(lb, bin, s);
    t.edge(s, ob, st);
    t.finish(
        format!("{name}_{rows}x{cols}"),
        crate::spatial::passes::from_tensor_op::items_per_tile_grid(rows_t, cols_t),
        AccPolicy::F32,
        approx(kind, precision),
        target,
    )
}

/// A blocked pointwise map of TWO equal-length rows — the fused residual add.
pub(super) fn blocked_binary_map(
    name: &str,
    kind: &str,
    body: impl Fn(Box<TensorBody>, Box<TensorBody>) -> Box<TensorBody>,
    a: (&str, &Fill),
    b: (&str, &Fill),
    out: &str,
    n: usize,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let precision = elem_precision(target)?;
    let cols_t = tiles_of(n, target);
    let mut t = Template::new(target)?;
    let at_ = t.tensor(
        a.0,
        TensorRole::Input,
        1,
        n,
        precision,
        crate::spatial::passes::from_tensor_op::init_of(a.1, n),
    );
    let bt = t.tensor(
        b.0,
        TensorRole::Input,
        1,
        n,
        precision,
        crate::spatial::passes::from_tensor_op::init_of(b.1, n),
    );
    let yt = t.tensor(out, TensorRole::Output, 1, n, precision, TensorInit::Zero);
    let ain = t.buffer("in_a", 1, precision);
    let bin = t.buffer("in_b", 1, precision);
    let ob = t.buffer("out", 1, precision);
    let la = t.load("load_a", at_, vec![(LV_ITEM_X, 1)], 1, ain);
    let lb = t.load("load_b", bt, vec![(LV_ITEM_X, 1)], 1, bin);
    let w = wide(1, target);
    let s = t.stage(
        kind,
        StageKind::Compute(ComputeOp::Generic {
            iters: flat_space(w),
            ins: vec![ident_map(w), ident_map(w)],
            out: ident_map(w),
            body: *body(op(0), op(1)),
        }),
        vec![ain, bin],
        vec![ob],
    );
    let st = t.store(&format!("store_{out}"), yt, vec![(LV_ITEM_X, 1)], 1, ob);
    t.edge(la, ain, s);
    t.edge(lb, bin, s);
    t.edge(s, ob, st);
    t.finish(
        format!("{name}_{n}"),
        items_per_tile(cols_t),
        AccPolicy::F32,
        approx(kind, precision),
        target,
    )
}

/// `C = A ⊕ B` over a flat block of `n / w` rows of `w`, `B` one row: a band per row, each reading `B` from its row's
/// start — the one pointwise stage, blocked by the result's columns past an accumulator page.
#[allow(clippy::too_many_arguments)]
pub(crate) fn flat_row_broadcast(
    kind: crate::tensor_recognize::ElementwiseOp,
    names: &crate::spatial::passes::from_tensor_op::ElementwiseNames,
    fills: (&Fill, &Fill),
    n: usize,
    w: usize,
    precision: Precision,
    claim: NumericsClaim,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let mut b = FlatBuild::columns(n, target)?;
    b.precision = precision;
    let a_in = b.input(&names.a, n, fills.0, target);
    let row = b.input(&names.b, w, fills.1, target);
    let out = b.result_page("out", n, precision, target);
    let mut slots = vec![Slot { buf: a_in, map: at(0) }];
    let mut bands = Vec::new();
    for r in 0..n / w {
        slots.push(Slot { buf: row, map: at(-((r * w) as i64)) });
        let rhs = op(slots.len() - 1);
        let combined = match kind {
            crate::tensor_recognize::ElementwiseOp::Add => *add(op(0), rhs),
            crate::tensor_recognize::ElementwiseOp::Mul => *mul(op(0), rhs),
        };
        bands.push(((r + 1) * w, combined));
    }
    let chain = b.pointwise("elementwise_row", n, slots, banded(bands), out, target)?;
    b.finish_as(&names.c, n, chain, out, format!("elementwise_{}_row_{}x{w}", kind.c_op(), n / w), claim, precision, target)
}
