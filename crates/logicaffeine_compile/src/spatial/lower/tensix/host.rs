//! The host program: DRAM buffers from the tensors, circular buffers from the plan, semaphores from the IR,
//! one `CreateKernel` per launch, one `SetRuntimeArgs` per (site, launch) from the argument layout, then the
//! input fill, the golden, the run, the PCC and the ledger's `LOGOS_MM_CFG` / `LOGOS_MM_PASS` lines.

use super::args::{ArgSource, Axis, RuntimeArgLayout};
use super::cb::{data_format, CbPlan};
use super::roles::{rect_decompose, KernelClass};
use super::{template_name, template_of, SpatialLowerError};
use crate::codegen_sva::fabric_target::Precision;
use crate::spatial::ir::*;
use crate::spatial::target::{Fidelity, SpatialTarget};

pub(super) fn ident(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(true)
    {
        out.insert(0, 't');
    }
    out
}

fn ranges_cpp(rects: &[Rect]) -> String {
    let items: Vec<String> = rects
        .iter()
        .map(|r| format!("CoreRange({{{}, {}}}, {{{}, {}}})", r.x0, r.y0, r.x1, r.y1))
        .collect();
    format!(
        "CoreRangeSet(std::vector<CoreRange>{{{}}})",
        items.join(", ")
    )
}

fn fidelity_cpp(f: Fidelity) -> &'static str {
    match f {
        Fidelity::LoFi => "MathFidelity::LoFi",
        Fidelity::HiFi2 => "MathFidelity::HiFi2",
        Fidelity::HiFi3 => "MathFidelity::HiFi3",
        Fidelity::HiFi4 | Fidelity::Exact => "MathFidelity::HiFi4",
    }
}

fn fidelity_label(f: Fidelity) -> &'static str {
    match f {
        Fidelity::LoFi => "LoFi",
        Fidelity::HiFi2 => "HiFi2",
        Fidelity::HiFi3 => "HiFi3",
        Fidelity::HiFi4 | Fidelity::Exact => "HiFi4",
    }
}

/// The per-circular-buffer unpack mode of the compute kernels: a Float32 buffer that the schedule reloads into
/// the accumulator (`copy_tile` of a spilled partial) must be marked `UnpackToDestFp32` under fp32 accumulate —
/// otherwise the reload is routed through the 19-bit SrcA register and rounded to TF32 on every block boundary
/// (measured on ttsim: PCC 0.14 at 640³ blocked/exact). A buffer read by the FPU as an operand keeps `Default`.
fn unpack_to_dest_cpp(p: &SpatialProgram, cbs: &CbPlan, exact_f32: bool) -> String {
    let mut marked: Vec<u32> = Vec::new();
    if exact_f32 {
        if let Some(sched) = &p.schedule {
            for prog in &sched.programs {
                walk_ops(
                    &prog.ops.iter().map(|t| t.op.clone()).collect::<Vec<_>>(),
                    1,
                    &mut |op, _| {
                        if let ChoreoOp::Compute {
                            acc_reload: Some(b),
                            ..
                        } = op
                        {
                            let slot = cbs.slot(*b);
                            if slot.dtype == Precision::Fp32 && !marked.contains(&slot.index) {
                                marked.push(slot.index);
                            }
                        }
                    },
                );
            }
        }
    }
    if marked.is_empty() {
        return "{}".into();
    }
    marked.sort();
    let sets: Vec<String> = marked
        .iter()
        .map(|i| format!("utd[{i}] = UnpackToDestMode::UnpackToDestFp32;"))
        .collect();
    format!("[]{{ std::vector<UnpackToDestMode> utd(NUM_CIRCULAR_BUFFERS, UnpackToDestMode::Default); {} return utd; }}()", sets.join(" "))
}

/// The bounding extent of the placed sites: the smallest device grid that can host the program.
pub fn placed_extent(pl: &Placement) -> (usize, usize) {
    let sites = pl.sites();
    (
        sites.iter().map(|s| s.x as usize + 1).max().unwrap_or(0),
        sites.iter().map(|s| s.y as usize + 1).max().unwrap_or(0),
    )
}

/// The ledger's configuration line: the emergent plan, in the vendor's vocabulary, plus the deployment mesh.
pub fn cfg_line(p: &SpatialProgram) -> String {
    let s = p.summary();
    let b = s.block.unwrap_or(BlockShape {
        osh: 1,
        osw: 1,
        ibw: 1,
    });
    let pl = p.placement.as_ref().expect("a placed program");
    format!(
        "LOGOS_MM_CFG strategy={} grid={}x{} per_core={}x{} out_block={}x{} in0_block_w={} subblock={}x{} origin=\"compiler-emitted, all kernels generated\" mesh={}x{} sites={}",
        s.arrangement_label(),
        s.grid.1,
        s.grid.0,
        s.per_site_block.0,
        s.per_site_block.1,
        s.per_site_block.0,
        s.per_site_block.1,
        b.ibw,
        b.osh,
        b.osw,
        pl.mesh.x,
        pl.mesh.y,
        pl.sites().len()
    )
}

/// The receivers of the group `operand` at `site`: the group's rectangle minus its sender (a 1-D group).
fn receivers(
    p: &SpatialProgram,
    site: Site,
    operand: Operand,
) -> Result<(Rect, Site, usize), SpatialLowerError> {
    let pl = p.placement.as_ref().unwrap();
    let g = pl
        .groups
        .iter()
        .find(|g| g.operand == operand && g.rect.contains(site))
        .ok_or(SpatialLowerError::Unsupported(
            "a site outside every group of its operand",
        ))?;
    let r = g.rect;
    let rect = match g.direction {
        Direction::Row => {
            if r.y0 != r.y1
                || g.sender
                    != (Site {
                        x: r.x0,
                        y: r.y0,
                        chip: 0,
                    })
            {
                return Err(SpatialLowerError::GroupSpansSites { group: g.id });
            }
            Rect {
                x0: r.x0 + 1,
                y0: r.y0,
                x1: r.x1,
                y1: r.y1,
            }
        }
        Direction::Col => {
            if r.x0 != r.x1
                || g.sender
                    != (Site {
                        x: r.x0,
                        y: r.y0,
                        chip: 0,
                    })
            {
                return Err(SpatialLowerError::GroupSpansSites { group: g.id });
            }
            Rect {
                x0: r.x0,
                y0: r.y0 + 1,
                x1: r.x1,
                y1: r.y1,
            }
        }
    };
    if rect.x1 < rect.x0 || rect.y1 < rect.y0 {
        return Err(SpatialLowerError::GroupSpansSites { group: g.id });
    }
    Ok((rect, g.sender, r.len() - 1))
}

/// The multicast rectangle of a site's receivers for `operand`, as `(start, end)` corners for a kernel on
/// `noc`: NOC 0 travels toward larger coordinates and takes the near corner first; NOC 1 travels the other
/// way and takes the far corner first (the vendor's `right → left+1`, `bottom → top+1`). The simulator refuses
/// a rectangle given against its NOC's direction.
pub fn mcast_corners(
    p: &SpatialProgram,
    site: Site,
    operand: Operand,
    noc: u8,
) -> Result<((u16, u16), (u16, u16)), SpatialLowerError> {
    let (r, _, _) = receivers(p, site, operand)?;
    Ok(if noc == 1 {
        ((r.x1, r.y1), (r.x0, r.y0))
    } else {
        ((r.x0, r.y0), (r.x1, r.y1))
    })
}

fn arg_value(
    p: &SpatialProgram,
    site: Site,
    source: &ArgSource,
    noc: u8,
) -> Result<String, SpatialLowerError> {
    let pl = p.placement.as_ref().unwrap();
    let items = pl.items_at(site);
    Ok(match source {
        ArgSource::TensorAddr(t) => {
            format!("(uint32_t)buf_{}->address()", ident(&p.tensor(*t).name))
        }
        ArgSource::CyclesAddr => "(uint32_t)buf_cycles->address()".into(),
        ArgSource::CyclesSlot(engine) => format!(
            "{}u",
            2 * pl
                .items_of_site
                .iter()
                .position(|(x, _)| *x == site)
                .unwrap_or(0)
                + *engine as usize
        ),
        ArgSource::ItemStart => format!("{}u", items.first().map(|i| i.idx()).unwrap_or(0)),
        ArgSource::ItemCount => format!("{}u", items.len()),
        ArgSource::ItemY0 => format!(
            "{}u",
            items
                .first()
                .map(|i| p.work.items[i.idx()].coord.0)
                .unwrap_or(0)
        ),
        ArgSource::ItemX0 => format!(
            "{}u",
            items
                .first()
                .map(|i| p.work.items[i.idx()].coord.1)
                .unwrap_or(0)
        ),
        ArgSource::ItemCountX => {
            let xs: std::collections::BTreeSet<usize> = items
                .iter()
                .map(|i| p.work.items[i.idx()].coord.1)
                .collect();
            format!("{}u", xs.len().max(1))
        }
        ArgSource::McastStart { operand, axis } => {
            let ((x, y), _) = mcast_corners(p, site, *operand, noc)?;
            match axis {
                Axis::X => format!("(uint32_t)P({x}, {y}).x"),
                Axis::Y => format!("(uint32_t)P({x}, {y}).y"),
            }
        }
        ArgSource::McastEnd { operand, axis } => {
            let (_, (x, y)) = mcast_corners(p, site, *operand, noc)?;
            match axis {
                Axis::X => format!("(uint32_t)P({x}, {y}).x"),
                Axis::Y => format!("(uint32_t)P({x}, {y}).y"),
            }
        }
        ArgSource::McastNumDests { operand } => format!("{}u", receivers(p, site, *operand)?.2),
        ArgSource::McastSender { operand, axis } => {
            let (_, s, _) = receivers(p, site, *operand)?;
            match axis {
                Axis::X => format!("(uint32_t)P({}, {}).x", s.x, s.y),
                Axis::Y => format!("(uint32_t)P({}, {}).y", s.x, s.y),
            }
        }
        ArgSource::SemHandle(s) => format!("(uint32_t)sem_{}", s.idx()),
        ArgSource::TensorDim { tensor, axis } => {
            let tiles = p.tensor(*tensor).tiles;
            format!(
                "{}u",
                match axis {
                    Axis::Y => tiles.rows_t,
                    Axis::X => tiles.cols_t,
                }
            )
        }
    })
}

/// The C++ element type a tensor's host-side vector holds — the one its DEVICE buffer is paged for.
///
/// ★ THE HOST MUST SPEAK THE TENSOR'S DTYPE. A GEMM's operands are always bf16, so the printer wrote
/// `std::vector<bfloat16>` unconditionally and nothing ever disagreed. A row op declares fp32, whose
/// tile is 4096 bytes rather than 2048 — and a bf16 source vector is then exactly half the buffer it
/// is uploaded into, which tt-metal catches at runtime as `Source vector is too small for mesh
/// buffer`. The type follows the declaration rather than the one case that used to be the only case.
fn host_elem_type(p: Precision) -> Result<&'static str, SpatialLowerError> {
    Ok(match p {
        Precision::Bf16 => "bfloat16",
        Precision::Fp32 => "float",
        other => return Err(SpatialLowerError::Precision { needed: other }),
    })
}

/// A C++ literal for `v`: its shortest round-trip decimal with `suffix`, or the `<cmath>` macro for a value no
/// decimal spells — `{:?}` would print `inf` and `NaN`, which C++ reads as undeclared names.
fn cpp_float(v: f64, suffix: &str) -> String {
    if v.is_nan() {
        "NAN".into()
    } else if v.is_infinite() {
        if v < 0.0 {
            "-INFINITY".into()
        } else {
            "INFINITY".into()
        }
    } else {
        format!("{v:?}{suffix}")
    }
}

fn fill_lines(
    p: &SpatialProgram,
    t: &TensorDecl,
    var: &str,
    padded_cols: usize,
) -> Result<Vec<String>, SpatialLowerError> {
    let (rows, cols) = (t.elems.rows, t.elems.cols);
    let ety = host_elem_type(t.dtype)?;
    let mut out = Vec::new();
    let assign = |expr: &str| {
        format!("for (uint32_t r = 0; r < {rows}u; ++r) for (uint32_t c = 0; c < {cols}u; ++c) {{ const uint32_t p = r * {cols}u + c; {var}[r * {padded_cols}u + c] = {ety}((float)({expr})); }}")
    };
    match &t.init {
        TensorInit::Zero => {}
        TensorInit::Pattern { modulus, shift } => {
            out.push(assign(&format!("(int64_t)(p % {modulus}u) + ({shift})")))
        }
        // One constant, emitted as one literal rather than a table as long as the tensor.
        TensorInit::ConstF(b) => out.push(assign(&cpp_float(f64::from_bits(*b), "f"))),
        TensorInit::Values(v) => {
            if v.len() > 65536 {
                return Err(SpatialLowerError::TooBig {
                    what: "literal tensor values",
                    need: v.len(),
                    have: 65536,
                });
            }
            let lits: Vec<String> = v.iter().map(|x| x.to_string()).collect();
            out.push(format!(
                "static const int64_t {var}_values[{}] = {{{}}};",
                v.len().max(1),
                lits.join(", ")
            ));
            out.push(assign(&format!("{var}_values[p]")));
        }
        TensorInit::FloatValues(v) => {
            if v.len() > 65536 {
                return Err(SpatialLowerError::TooBig {
                    what: "literal tensor values",
                    need: v.len(),
                    have: 65536,
                });
            }
            let lits: Vec<String> = v
                .iter()
                .map(|b| cpp_float(f64::from_bits(*b), ""))
                .collect();
            out.push(format!(
                "static const double {var}_values[{}] = {{{}}};",
                v.len().max(1),
                lits.join(", ")
            ));
            out.push(assign(&format!("{var}_values[p]")));
        }
        TensorInit::RandomBf16 { seed, shift_milli } => {
            let copy = if ety == "bfloat16" {
                "rnd[r * {cols}u + c]".to_string()
            } else {
                format!("{ety}(static_cast<float>(rnd[r * {{cols}}u + c]))")
            };
            let copy = copy.replace("{cols}", &cols.to_string());
            out.push(format!("{{ auto rnd = create_random_vector_of_bfloat16_native(2u * {rows}u * {cols}u, 1, {seed}, {}f); for (uint32_t r = 0; r < {rows}u; ++r) for (uint32_t c = 0; c < {cols}u; ++c) {var}[r * {padded_cols}u + c] = {copy}; }}", *shift_milli as f64 / 1000.0));
        }
        TensorInit::External => {
            return Err(SpatialLowerError::Unsupported(
                "a tensor whose data is external to the program",
            ))
        }
    }
    let _ = p;
    Ok(out)
}

/// The CPU golden for a non-GEMM program, composed stage by stage from the SAME chain the kernels run.
///
/// ★ ONE GOLDEN PER STAGE, NOT ONE PER WORKLOAD. A normalization is not one kernel — it is a square,
/// a reduction, a transform and a rescale, each with its own register window. Printing a single fused
/// reference for it would be a SECOND implementation of the pipeline, correct only as long as someone
/// kept the two in step. Composing the reference the way the choreography composes the work means a
/// stage that is added, reordered or removed changes the golden by construction; there is nothing left
/// to keep in step. Each stage's arithmetic mirrors `spatial/eval.rs`, which is the reference
/// semantics, so epsilons and f64 accumulation are reproduced rather than re-derived.
fn golden_lines(
    p: &SpatialProgram,
    ids: &[(String, usize, usize)],
    tile: usize,
) -> Result<Vec<String>, SpatialLowerError> {
    let sched = p.schedule.as_ref().ok_or(SpatialLowerError::Unsupported(
        "a golden for an unscheduled program",
    ))?;
    // Which tensor fills a buffer, and the compute stages in the order the choreography runs them.
    // `ChoreoOp::Compute` does not name its destination — the `PackAcc` that follows it does — so the
    // two are paired here rather than assumed to be the same buffer every time.
    let mut from_tensor: std::collections::BTreeMap<BufferId, TensorId> = Default::default();
    let mut steps: Vec<(ComputeOp, Vec<BufferId>, BufferId)> = Vec::new();
    let mut pending: Option<(ComputeOp, Vec<BufferId>)> = None;
    for prog in &sched.programs {
        let ops: Vec<ChoreoOp> = prog.ops.iter().map(|t| t.op.clone()).collect();
        walk_ops(&ops, 1, &mut |op, _| match op {
            ChoreoOp::ReadTiles { tensor, into, .. } => {
                from_tensor.entry(*into).or_insert(*tensor);
            }
            ChoreoOp::Compute { op, ins, .. } => pending = Some((op.clone(), ins.clone())),
            ChoreoOp::PackAcc { to, .. } => {
                if let Some((op, ins)) = pending.take() {
                    steps.push((op, ins, *to));
                }
            }
            _ => {}
        });
    }
    if steps.is_empty() {
        return Err(SpatialLowerError::Unsupported(
            "a golden for a program with no compute stage",
        ));
    }
    // Every site runs the same chain, so keep the first instance of each distinct step.
    let mut seen: std::collections::BTreeSet<(Vec<BufferId>, BufferId)> = Default::default();
    steps.retain(|(_, ins, dst)| {
        let key = (
            ins.iter()
                .map(|b| template_of(p.buffer(*b)))
                .collect::<Vec<_>>(),
            template_of(p.buffer(*dst)),
        );
        seen.insert(key)
    });
    // ★ A GOLDEN PLANE IS AS WIDE AS THE DATA, NOT AS WIDE AS A PAGE. A page is what one work item
    // carries; a buffer's plane is what every item carries together. They coincide when the item is a
    // whole tile-row (a normalization) and differ completely when the items sweep the columns (an
    // elementwise map over a flat tensor is 64 one-tile items, page width 32, plane width 2048).
    // Sizing the plane by the page silently truncated the operands and then read past the end of the
    // result — a golden that is wrong in a way no lowering test can see. The width is seeded from the
    // tensor each load carries and propagated through the chain: only a reduction narrows it.
    let mut plane: std::collections::BTreeMap<BufferId, usize> = Default::default();
    for (b, t) in &from_tensor {
        plane.insert(template_of(p.buffer(*b)), p.tensor(*t).tiles.cols_t * tile);
    }
    for (op, ins, dst) in &steps {
        let w = match op {
            ComputeOp::Generic { iters, .. }
                if iters
                    .get(1)
                    .map(|a| matches!(a.kind, IterKind::Reduce { .. }))
                    .unwrap_or(false) =>
            {
                tile
            }
            _ => ins
                .first()
                .map(|b| {
                    plane
                        .get(&template_of(p.buffer(*b)))
                        .copied()
                        .unwrap_or(tile)
                })
                .unwrap_or(tile),
        };
        plane.insert(template_of(p.buffer(*dst)), w);
    }
    let width = |b: BufferId| {
        plane
            .get(&template_of(p.buffer(b)))
            .copied()
            .unwrap_or_else(|| p.buffer(b).kind.page_tiles() * tile)
    };
    // Placement gives every site its own instance of a buffer; the golden is ONE host-side reference
    // for the whole matrix, so it is keyed on the template every instance came from.
    let gvar = |b: BufferId| format!("g_{}", ident(&template_name(p, p.buffer(b))));
    let host_of = |b: BufferId| -> Option<(String, usize, usize)> {
        let t = from_tensor.get(&b)?;
        let name = ident(&p.tensor(*t).name);
        ids.iter()
            .find(|(id, _, _)| *id == name)
            .map(|(id, rp, cp)| (format!("{id}_host"), *rp, *cp))
    };
    let mut o = Vec::new();
    let mut declared: std::collections::BTreeSet<String> = Default::default();
    let declare = |o: &mut Vec<String>,
                   declared: &mut std::collections::BTreeSet<String>,
                   b: BufferId| {
        let (v, w) = (gvar(b), width(b));
        if !declared.insert(v.clone()) {
            return;
        }
        o.push(format!("std::vector<double> {v}(Mp * {w}u, 0.0);"));
        if let Some((host, rp, cp)) = host_of(b) {
            // A per-column weight is ONE tile-row tall however many rows the output has, so the copy
            // is bounded by the source tensor's own height, not by the output's. Running to `Mp` here
            // reads past the end of a 1×cols operand the moment the matrix is taller than one tile.
            o.push(format!(
                "for (uint32_t r = 0; r < Mp && r < {rp}u; ++r) for (uint32_t c = 0; c < {w}u && c < {cp}u; ++c) {v}[r * {w}u + c] = static_cast<double>({host}[r * {cp}u + c]);"
            ));
        }
    };
    for (op, ins, dst) in &steps {
        for b in ins {
            declare(&mut o, &mut declared, *b);
        }
        declare(&mut o, &mut declared, *dst);
        let (d, wd) = (gvar(*dst), width(*dst));
        match op {
            ComputeOp::Generic {
                iters,
                ins: gmaps,
                body,
                ..
            } => {
                if iters.len() != 2 {
                    return Err(SpatialLowerError::Unsupported("a golden for a Generic with a contraction axis (LV_GEN_K, a third folded iteration variable)"));
                }
                let (row_ax, col_ax) = (iters[0], iters[1]);
                // The golden spells the four tile-aligned broadcast forms and the column fold — the
                // same set the compute kernel prints. A row-axis fold, or a map with an offset, a
                // stride or an indirect term, is refused here in the same breath the kernel refuses it.
                if matches!(row_ax.kind, IterKind::Reduce { .. }) {
                    return Err(SpatialLowerError::Unsupported(
                        "a golden for a Generic reduction over the row axis on Tensix",
                    ));
                }
                for (i, m) in gmaps.iter().enumerate() {
                    let width = ins
                        .get(i)
                        .map(|b| p.buffer(*b).kind.page_tiles() * tile)
                        .unwrap_or(tile);
                    if m.broadcast_class(row_ax.var, col_ax.var, width).is_none() {
                        return Err(SpatialLowerError::Unsupported("a golden for a Generic access map with an offset, a stride or an indirect term on Tensix"));
                    }
                }
                // Each operand's coordinate, with an absent axis reading as constant — the same rule
                // `spatial/eval.rs` applies, written out in C++ so the two cannot drift.
                let coord = |i: usize, ax: IterSpec, v: &str| -> String {
                    match gmaps.get(i).map(|m| m.varies_along(ax.var)).unwrap_or(true) {
                        true => v.to_string(),
                        false => "0".to_string(),
                    }
                };
                let expr = |b: &TensorBody| -> Result<String, SpatialLowerError> {
                    fn go(
                        b: &TensorBody,
                        ins: &[BufferId],
                        gv: &dyn Fn(BufferId) -> String,
                        wd: &dyn Fn(BufferId) -> usize,
                        rc: &dyn Fn(usize) -> (String, String),
                    ) -> Result<String, SpatialLowerError> {
                        Ok(match b {
                            TensorBody::Operand(i) => {
                                let buf = *ins.get(*i).ok_or(SpatialLowerError::Unsupported(
                                    "a Generic body naming an absent operand",
                                ))?;
                                let (r, c) = rc(*i);
                                format!("{}[({r}) * {}u + ({c})]", gv(buf), wd(buf))
                            }
                            TensorBody::Const(bits) => {
                                format!("({})", cpp_float(f64::from_bits(*bits), ""))
                            }
                            TensorBody::Min(..)
                            | TensorBody::Max(..)
                            | TensorBody::FloorDiv(..)
                            | TensorBody::Shr { .. } => {
                                return Err(SpatialLowerError::Unsupported(
                                    "an integer model's min, max, floor division or arithmetic shift on the Tensix host — it prints none of them",
                                ))
                            }
                            TensorBody::Add(x, y) => format!(
                                "({} + {})",
                                go(x, ins, gv, wd, rc)?,
                                go(y, ins, gv, wd, rc)?
                            ),
                            TensorBody::Sub(x, y) => format!(
                                "({} - {})",
                                go(x, ins, gv, wd, rc)?,
                                go(y, ins, gv, wd, rc)?
                            ),
                            TensorBody::Mul(x, y) => format!(
                                "({} * {})",
                                go(x, ins, gv, wd, rc)?,
                                go(y, ins, gv, wd, rc)?
                            ),
                            TensorBody::Unary { kind, arg } => {
                                let a = go(arg, ins, gv, wd, rc)?;
                                match kind {
                                    SfpuKind::Exp => format!("std::exp({a})"),
                                    SfpuKind::Recip => format!("(1.0 / ({a}))"),
                                    SfpuKind::Rsqrt { eps_nano, over } => format!(
                                        "(1.0 / std::sqrt(({a}) / {}.0 + {:e}))",
                                        (*over).max(1),
                                        *eps_nano as f64 * 1e-9
                                    ),
                                    SfpuKind::Silu => format!("(({a}) / (1.0 + std::exp(-({a}))))"),
                                    SfpuKind::Relu => format!("std::fmax(0.0, {a})"),
                                    SfpuKind::Ln => {
                                        return Err(SpatialLowerError::Unsupported(
                                            "a golden for a natural-log transform on Tensix",
                                        ))
                                    }
                                }
                            }
                            TensorBody::Select { .. } => {
                                return Err(SpatialLowerError::Unsupported(
                                    "a golden for a piecewise Select body on Tensix",
                                ))
                            }
                        })
                    }
                    let rc = |i: usize| (coord(i, row_ax, "r"), coord(i, col_ax, "c"));
                    go(b, ins, &|b| gvar(b), &|b| width(b), &rc)
                };
                let e = expr(body)?;
                if let IterKind::Reduce { pool } = col_ax.kind {
                    let (init, step) = match pool {
                        ReducePool::Sum => ("0.0", "acc += v;"),
                        ReducePool::Max => ("-INFINITY", "acc = v > acc ? v : acc;"),
                        ReducePool::ArgMax => {
                            return Err(SpatialLowerError::Unsupported(
                                "a golden for an index-valued ArgMax reduction on Tensix",
                            ))
                        }
                    };
                    o.push("for (uint32_t r = 0; r < Mp; ++r) {".into());
                    o.push(format!("    double acc = {init};"));
                    o.push(format!(
                        "    for (uint32_t c = 0; c < {}u; ++c) {{ const double v = {e}; {step} }}",
                        col_ax.extent
                    ));
                    o.push(format!(
                        "    for (uint32_t c = 0; c < {wd}u; ++c) {d}[r * {wd}u + c] = acc;"
                    ));
                    o.push("}".into());
                } else {
                    o.push(format!(
                        "for (uint32_t r = 0; r < Mp; ++r) for (uint32_t c = 0; c < {wd}u; ++c)"
                    ));
                    o.push(format!("    {d}[r * {wd}u + c] = {e};"));
                }
            }
            // Refused in the same breath the compute kernel refuses them, so a program can never
            // reach silicon with a kernel the golden cannot check.
            ComputeOp::MatmulTiles { .. } => {
                return Err(SpatialLowerError::Unsupported(
                    "a matmul whose shapes are not a GEMM",
                ))
            }
            ComputeOp::Requant { .. } => {
                return Err(SpatialLowerError::Unsupported(
                    "a golden for a requant epilogue on Tensix (P4b)",
                ))
            }
            ComputeOp::Bias { .. } => {
                return Err(SpatialLowerError::Unsupported(
                    "a golden for a bias epilogue on Tensix (P4)",
                ))
            }
            ComputeOp::Relu => {
                return Err(SpatialLowerError::Unsupported(
                    "a golden for relu on Tensix (P4)",
                ))
            }
            ComputeOp::Scalar(_) => {
                return Err(SpatialLowerError::Unsupported(
                    "a golden for a scalar fabric body on Tensix (P4)",
                ))
            }
        }
    }
    let (last, wl) = {
        let (_, _, d) = steps.last().unwrap();
        (gvar(*d), width(*d))
    };
    o.push(format!("for (uint32_t r = 0; r < M; ++r) for (uint32_t c = 0; c < N; ++c) golden[r * Np + c] = static_cast<float>({last}[r * {wl}u + c]);"));
    Ok(o)
}

pub fn print_host(
    p: &SpatialProgram,
    target: &SpatialTarget,
    cbs: &CbPlan,
    classes: &[KernelClass],
    layout: &RuntimeArgLayout,
    exact_f32: bool,
    fidelity: Fidelity,
    cfg_line: &str,
) -> Result<String, SpatialLowerError> {
    let pl = p.placement.as_ref().unwrap();
    let n_sites = pl.items_of_site.len();
    let inputs: Vec<&TensorDecl> = p
        .tensors
        .iter()
        .filter(|t| t.role != TensorRole::Output)
        .collect();
    let out = p
        .output_tensor()
        .ok_or(SpatialLowerError::Unsupported("an output tensor"))?;
    if inputs.is_empty() {
        return Err(SpatialLowerError::Unsupported(
            "a host for a program with at least one input tensor",
        ));
    }
    let tile = target.tile.rows;
    // ★ THE GEMM TRIPLE KEEPS ITS EXACT VOCABULARY. Every row of the ttsim ledger is keyed on the
    // strings this host prints — `LOGOS_MM_CFG`, the M/N/K constants, the golden's shape — and
    // `spatial_cost_model_lock` re-derives the plan from them. So a GEMM prints byte-for-byte what
    // it printed before P4 widened this function, and everything else takes the general path.
    let a = inputs[0];
    let gemm = inputs.len() == 2
        && inputs[1].elems.rows == a.elems.cols
        && out.elems.rows == a.elems.rows
        && out.elems.cols == inputs[1].elems.cols;
    let (m, k, n) = if gemm {
        (a.elems.rows, a.elems.cols, inputs[1].elems.cols)
    } else {
        // Every non-GEMM op this printer lowers writes its output element-for-element, so the
        // output's own geometry is the loop bound and `K` is only the operand width.
        (out.elems.rows, a.elems.cols, out.elems.cols)
    };
    let (mp, kp, np) = if gemm {
        (
            a.tiles.rows_t * tile,
            a.tiles.cols_t * tile,
            inputs[1].tiles.cols_t * tile,
        )
    } else {
        (
            out.tiles.rows_t * tile,
            a.tiles.cols_t * tile,
            out.tiles.cols_t * tile,
        )
    };
    let out_f32 = out.dtype == Precision::Fp32;
    let floor = if p.claim == NumericsClaim::Exact {
        "0.9999"
    } else {
        "0.99"
    };
    let all_sites: std::collections::BTreeSet<Site> = p.sites();
    let all_rects = rect_decompose(&all_sites);
    let mut h = String::new();
    let w = |h: &mut String, s: &str| {
        h.push_str(s);
        h.push('\n');
    };
    w(&mut h, &format!("// host.cpp — printed from the spatial IR ({}). Every semaphore, circular buffer and kernel below is the", p.name));
    w(
        &mut h,
        "// image of one IR object; the plan it runs is the emergent one the passes chose.",
    );
    for inc in [
        "<tt-metalium/host_api.hpp>",
        "<tt-metalium/constants.hpp>",
        "<tt-metalium/bfloat16.hpp>",
        "<tt-metalium/tilize_utils.hpp>",
        "<tt-metalium/device.hpp>",
        "<tt-metalium/tt_metal.hpp>",
        "<tt-metalium/tensor_accessor_args.hpp>",
        "<tt-metalium/distributed.hpp>",
        "<fmt/core.h>",
        "<cmath>",
        "<cstdint>",
        "<cstdlib>",
        "<string>",
        "<vector>",
    ] {
        w(&mut h, &format!("#include {inc}"));
    }
    w(&mut h, "using namespace tt;");
    w(&mut h, "using namespace tt::tt_metal;");
    w(&mut h, "");
    w(
        &mut h,
        "static float logos_pcc(const std::vector<float>& x, const std::vector<float>& y) {",
    );
    w(&mut h, "    double mx = 0, my = 0;");
    w(
        &mut h,
        "    for (size_t i = 0; i < x.size(); ++i) { mx += x[i]; my += y[i]; }",
    );
    w(
        &mut h,
        "    mx /= (double)x.size(); my /= (double)y.size();",
    );
    w(&mut h, "    double sxy = 0, sxx = 0, syy = 0;");
    w(&mut h, "    for (size_t i = 0; i < x.size(); ++i) { const double dx = x[i] - mx, dy = y[i] - my; sxy += dx * dy; sxx += dx * dx; syy += dy * dy; }");
    w(
        &mut h,
        "    if (sxx == 0 || syy == 0) return (sxx == 0 && syy == 0 && x == y) ? 1.0f : 0.0f;",
    );
    w(
        &mut h,
        "    return static_cast<float>(sxy / std::sqrt(sxx * syy));",
    );
    w(&mut h, "}");
    w(&mut h, "");
    w(&mut h, "int main() {");
    // The contraction extent exists only for a GEMM. tt-metal builds with `-Werror -Wunused-variable`,
    // so declaring a `K` nothing reads is not a harmless extra line — it fails the build.
    if gemm {
        w(
            &mut h,
            &format!("    constexpr uint32_t M = {m}, N = {n}, K = {k};"),
        );
        w(
            &mut h,
            &format!("    constexpr uint32_t Mp = {mp}, Np = {np}, Kp = {kp};"),
        );
    } else {
        w(&mut h, &format!("    constexpr uint32_t M = {m}, N = {n};"));
        w(
            &mut h,
            &format!("    constexpr uint32_t Mp = {mp}, Np = {np};"),
        );
    }
    w(
        &mut h,
        "    auto mesh_device = distributed::MeshDevice::create_unit_mesh(0);",
    );
    let (px, py) = placed_extent(pl);
    w(
        &mut h,
        "    const CoreCoord device_grid = mesh_device->compute_with_storage_grid_size();",
    );
    w(
        &mut h,
        &format!("    constexpr uint32_t PLACED_X = {px}, PLACED_Y = {py};"),
    );
    w(
        &mut h,
        "    if (device_grid.x < PLACED_X || device_grid.y < PLACED_Y) {",
    );
    w(&mut h, "        fmt::print(\"LOGOS_GRID_MISMATCH device={}x{} placed={}x{}\\n\", device_grid.x, device_grid.y, PLACED_X, PLACED_Y);");
    w(&mut h, "        mesh_device->close();");
    w(&mut h, "        return 2;");
    w(&mut h, "    }");
    w(&mut h, "    fmt::print(\"LOGOS_GRID device={}x{} placed={}x{}\\n\", device_grid.x, device_grid.y, PLACED_X, PLACED_Y);");
    w(
        &mut h,
        "    distributed::MeshCommandQueue& cq = mesh_device->mesh_command_queue();",
    );
    w(
        &mut h,
        "    auto device_range = distributed::MeshCoordinateRange(mesh_device->shape());",
    );
    w(&mut h, "    Program program{};");
    w(
        &mut h,
        &format!("    fmt::print(\"{}\\n\");", cfg_line.replace('"', "\\\"")),
    );
    w(&mut h, "");
    // Tensors → DRAM buffers.
    for t in &p.tensors {
        let id = ident(&t.name);
        let tb = target.tile_bytes(t.dtype);
        w(&mut h, &format!("    distributed::DeviceLocalBufferConfig dram_{id}{{.page_size = {tb}, .buffer_type = BufferType::DRAM}};"));
        w(
            &mut h,
            &format!(
                "    distributed::ReplicatedBufferConfig cfg_{id}{{.size = {}}};",
                tb * t.tiles.count()
            ),
        );
        w(&mut h, &format!("    auto buf_{id} = distributed::MeshBuffer::create(cfg_{id}, dram_{id}, mesh_device.get());"));
    }
    // Device-cycle scratch: one word per site, written by each site's output writer and read back below.
    // Sized from the placement rather than the mesh so a partially-used grid does not read uninitialised words.
    // One 32-byte page per site rather than one word: DRAM transfers are 32-byte aligned, so a four-byte
    // page would be a misaligned write rather than a compact one. Only the first word of each page is used.
    w(&mut h, &format!("    distributed::DeviceLocalBufferConfig dram_cycles{{.page_size = 32, .buffer_type = BufferType::L1}};"));
    w(
        &mut h,
        &format!(
            "    distributed::ReplicatedBufferConfig cfg_cycles{{.size = {}}};",
            64 * n_sites.max(1)
        ),
    );
    w(&mut h, "    auto buf_cycles = distributed::MeshBuffer::create(cfg_cycles, dram_cycles, mesh_device.get());");
    w(&mut h, "");
    // Buffers → circular buffers (one per template, on the sites that instantiate it). A template that shares
    // another's memory is an index of that template's config, not a config of its own.
    for s in cbs.slots.iter().filter(|s| s.alias_of.is_none()) {
        let sites: std::collections::BTreeSet<Site> = s.sites.iter().copied().collect();
        let rects = if sites.is_empty() {
            all_rects.clone()
        } else {
            rect_decompose(&sites)
        };
        let indices: Vec<&super::cb::CbSlot> = std::iter::once(s)
            .chain(cbs.slots.iter().filter(|a| a.alias_of == Some(s.template)))
            .collect();
        let total = indices.iter().map(|x| x.total_bytes()).max().unwrap_or(0);
        let formats = indices
            .iter()
            .map(|x| Ok(format!("{{{}, {}}}", x.index, data_format(x.dtype)?)))
            .collect::<Result<Vec<_>, SpatialLowerError>>()?
            .join(", ");
        let pages: String = indices
            .iter()
            .map(|x| {
                format!(
                    " cb_{}.set_page_size({}, {});",
                    s.name, x.index, x.tile_bytes
                )
            })
            .collect();
        w(&mut h, &format!("    CircularBufferConfig cb_{0}({1}, {{{2}}});{3} CreateCircularBuffer(program, {4}, cb_{0});", s.name, total, formats, pages, ranges_cpp(&rects)));
    }
    w(&mut h, "");
    // Semaphores.
    for s in &p.semaphores {
        let sites: std::collections::BTreeSet<Site> = s.sites.iter().copied().collect();
        let rects = if sites.is_empty() {
            all_rects.clone()
        } else {
            rect_decompose(&sites)
        };
        w(
            &mut h,
            &format!(
                "    auto sem_{} = CreateSemaphore(program, {}, {});",
                s.id.idx(),
                ranges_cpp(&rects),
                s.init
            ),
        );
    }
    w(&mut h, "");
    // Kernels: one source per class, one `CreateKernel` per NOC launch of it.
    for (i, c) in classes.iter().enumerate() {
        match c.engine {
            Engine::Movement(e) => {
                // Compile-time args: the tensor accessors of every tensor the kernel touches, in tensor-id order.
                let mut tensors: Vec<TensorId> = Vec::new();
                let rep = &p.schedule.as_ref().unwrap().programs[c.representative];
                let ops: Vec<ChoreoOp> = rep.ops.iter().map(|t| t.op.clone()).collect();
                walk_ops(&ops, 1, &mut |op, _| {
                    if let ChoreoOp::ReadTiles { tensor, .. }
                    | ChoreoOp::WriteTiles { tensor, .. } = op
                    {
                        if !tensors.contains(tensor) {
                            tensors.push(*tensor);
                        }
                    }
                });
                tensors.sort();
                let mut line = format!("    std::vector<uint32_t> ca_{i};");
                for t in &tensors {
                    line.push_str(&format!(
                        " TensorAccessorArgs(*buf_{}).append_to(ca_{i});",
                        ident(&p.tensor(*t).name)
                    ));
                }
                // Appended AFTER every tensor, so the accessor offsets the kernels already compute are
                // untouched and only the new one takes the final slot.
                if super::args::stamps_cycles(&ops) {
                    line.push_str(&format!(
                        " TensorAccessorArgs(*buf_cycles).append_to(ca_{i});"
                    ));
                }
                w(&mut h, &line);
                for (j, l) in c.launches.iter().enumerate() {
                    let noc = l.noc.map(|n| n.0).unwrap_or(0);
                    w(&mut h, &format!(
                        "    auto k_{i}_{j} = CreateKernel(program, LOGOS_KERNEL_DIR \"{}\", {}, DataMovementConfig{{.processor = DataMovementProcessor::RISCV_{e}, .noc = NOC::RISCV_{noc}_default, .compile_args = ca_{i}}});",
                        c.file_name,
                        ranges_cpp(&l.rects)
                    ));
                }
            }
            Engine::Compute => {
                for (j, l) in c.launches.iter().enumerate() {
                    w(&mut h, &format!(
                        "    auto k_{i}_{j} = CreateKernel(program, LOGOS_KERNEL_DIR \"{}\", {}, ComputeConfig{{.math_fidelity = {}, .fp32_dest_acc_en = {}, .unpack_to_dest_mode = {}, .compile_args = {{}}}});",
                        c.file_name,
                        ranges_cpp(&l.rects),
                        fidelity_cpp(fidelity),
                        if exact_f32 { "true" } else { "false" },
                        unpack_to_dest_cpp(p, cbs, exact_f32)
                    ));
                }
            }
        }
    }
    w(&mut h, "");
    w(&mut h, "    auto P = [&](uint32_t x, uint32_t y) { return mesh_device->worker_core_from_logical_core(CoreCoord{x, y}); };");
    w(&mut h, "    (void)P;");
    // Runtime args: per (site, launch) from the layout.
    for (i, c) in classes.iter().enumerate() {
        let slots = &layout
            .per_kernel
            .iter()
            .find(|(n, _)| *n == c.file_name)
            .map(|(_, s)| s.clone())
            .unwrap_or_default();
        for (j, l) in c.launches.iter().enumerate() {
            let noc = l.noc.map(|n| n.0).unwrap_or(0);
            for site in &l.sites {
                let vals: Result<Vec<String>, _> = slots
                    .iter()
                    .map(|s| arg_value(p, *site, &s.source, noc))
                    .collect();
                w(&mut h, &format!("    SetRuntimeArgs(program, k_{i}_{j}, CoreCoord{{{}, {}}}, std::vector<uint32_t>{{{}}});", site.x, site.y, vals?.join(", ")));
            }
        }
    }
    let _ = pl;
    w(&mut h, "");
    // Inputs (padded to whole tiles), golden, tilize, upload.
    let idc = ident(&out.name);
    if gemm {
        let (ida, idb) = (ident(&a.name), ident(&inputs[1].name));
        w(
            &mut h,
            &format!("    std::vector<bfloat16> {ida}_host(Mp * Kp, bfloat16(0.0f));"),
        );
        w(
            &mut h,
            &format!("    std::vector<bfloat16> {idb}_host(Kp * Np, bfloat16(0.0f));"),
        );
        for l in fill_lines(p, a, &format!("{ida}_host"), kp)? {
            w(&mut h, &format!("    {l}"));
        }
        for l in fill_lines(p, inputs[1], &format!("{idb}_host"), np)? {
            w(&mut h, &format!("    {l}"));
        }
        w(&mut h, "    std::vector<float> golden(Mp * Np, 0.0f);");
        w(
            &mut h,
            "    for (uint32_t i = 0; i < M; ++i) for (uint32_t kk = 0; kk < K; ++kk) {",
        );
        w(
            &mut h,
            &format!("        const float av = static_cast<float>({ida}_host[i * Kp + kk]);"),
        );
        w(&mut h, &format!("        for (uint32_t j = 0; j < N; ++j) golden[i * Np + j] += av * static_cast<float>({idb}_host[kk * Np + j]);"));
        w(&mut h, "    }");
        w(
            &mut h,
            &format!("    auto {ida}_tiled = tilize_nfaces({ida}_host, Mp, Kp);"),
        );
        w(
            &mut h,
            &format!("    auto {idb}_tiled = tilize_nfaces({idb}_host, Kp, Np);"),
        );
        w(
            &mut h,
            &format!("    distributed::EnqueueWriteMeshBuffer(cq, buf_{ida}, {ida}_tiled, false);"),
        );
        w(
            &mut h,
            &format!("    distributed::EnqueueWriteMeshBuffer(cq, buf_{idb}, {idb}_tiled, false);"),
        );
    } else {
        // ★ N INPUTS, EACH IN ITS OWN GEOMETRY. A weighted row op carries three tensors and its
        // weight is a 1×cols row, so a single padded shape cannot describe them all: every tensor
        // is filled, tilized and uploaded at the width its OWN tile count declares.
        let mut ids: Vec<(String, usize, usize)> = Vec::new();
        for t in &inputs {
            let id = ident(&t.name);
            let ety = host_elem_type(t.dtype)?;
            let (rp, cp) = (t.tiles.rows_t * tile, t.tiles.cols_t * tile);
            w(
                &mut h,
                &format!("    std::vector<{ety}> {id}_host({rp}u * {cp}u, {ety}(0.0f));"),
            );
            for l in fill_lines(p, t, &format!("{id}_host"), cp)? {
                w(&mut h, &format!("    {l}"));
            }
            ids.push((id, rp, cp));
        }
        w(&mut h, "    std::vector<float> golden(Mp * Np, 0.0f);");
        for l in golden_lines(p, &ids, tile)? {
            w(&mut h, &format!("    {l}"));
        }
        for (id, rp, cp) in &ids {
            w(
                &mut h,
                &format!("    auto {id}_tiled = tilize_nfaces({id}_host, {rp}u, {cp}u);"),
            );
            w(
                &mut h,
                &format!(
                    "    distributed::EnqueueWriteMeshBuffer(cq, buf_{id}, {id}_tiled, false);"
                ),
            );
        }
    }
    w(&mut h, "    distributed::MeshWorkload workload; workload.add_program(device_range, std::move(program));");
    w(
        &mut h,
        "    distributed::EnqueueMeshWorkload(cq, workload, false);",
    );
    if out_f32 {
        w(
            &mut h,
            &format!("    std::vector<float> {idc}_tiled(Mp * Np);"),
        );
        w(
            &mut h,
            &format!("    distributed::EnqueueReadMeshBuffer(cq, {idc}_tiled, buf_{idc}, true);"),
        );
        w(
            &mut h,
            &format!("    auto {idc}_host = untilize_nfaces({idc}_tiled, Mp, Np);"),
        );
        w(&mut h, &format!("    std::vector<float> got(M * N); for (uint32_t i = 0; i < M; ++i) for (uint32_t j = 0; j < N; ++j) got[i * N + j] = {idc}_host[i * Np + j];"));
    } else {
        w(
            &mut h,
            &format!("    std::vector<bfloat16> {idc}_tiled(Mp * Np, bfloat16(0.0f));"),
        );
        w(
            &mut h,
            &format!("    distributed::EnqueueReadMeshBuffer(cq, {idc}_tiled, buf_{idc}, true);"),
        );
        w(
            &mut h,
            &format!("    auto {idc}_host = untilize_nfaces({idc}_tiled, Mp, Np);"),
        );
        w(&mut h, &format!("    std::vector<float> got(M * N); for (uint32_t i = 0; i < M; ++i) for (uint32_t j = 0; j < N; ++j) got[i * N + j] = static_cast<float>({idc}_host[i * Np + j]);"));
    }
    w(&mut h, "    std::vector<float> want(M * N); for (uint32_t i = 0; i < M; ++i) for (uint32_t j = 0; j < N; ++j) want[i * N + j] = golden[i * Np + j];");
    w(
        &mut h,
        "    long long checksum = 0; for (float v : got) checksum += (long long)std::llround(v);",
    );
    w(&mut h, "    long long want_checksum = 0; for (float v : want) want_checksum += (long long)std::llround(v);");
    w(&mut h, "    const float pcc = logos_pcc(want, got);");
    w(
        &mut h,
        "    fmt::print(\"LOGOS_CHECKSUM {} golden {}\\n\", checksum, want_checksum);",
    );
    w(
        &mut h,
        &format!(
            "    fmt::print(\"LOGOS spatial-IR matmul ({}) -- PCC = {{}}\\n\", pcc);",
            target.name
        ),
    );
    let exact_claim = p.claim == NumericsClaim::Exact;
    let ok = if exact_claim {
        format!("(pcc >= {floor}f && checksum == want_checksum)")
    } else {
        format!("(pcc >= {floor}f)")
    };
    w(&mut h, &format!("    const bool ok = {ok};"));
    w(&mut h, &format!("    if (ok) fmt::print(\"LOGOS_MM_PASS fidelity={} fp32={} l1acc={} pcc={{}} floor={floor}\\n\", pcc);", fidelity_label(fidelity), if exact_f32 { 1 } else { 0 }, p.work.plan.map(|pl| pl.pack_accumulate).unwrap_or(false) as u8));
    w(&mut h, "    else fmt::print(\"LOGOS_MM_FAIL pcc={} checksum={} golden={}\\n\", pcc, checksum, want_checksum);");
    w(
        &mut h,
        &format!("    std::vector<uint32_t> cyc({});", 16 * n_sites.max(1)),
    );
    w(
        &mut h,
        "    distributed::EnqueueReadMeshBuffer(cq, cyc, buf_cycles, true);",
    );
    // The MAX, not the mean: the slowest core is the program's length, and an average would hide exactly the
    // imbalance between engines that the whole plan search is trying to remove.
    w(&mut h, "    uint32_t device_cycles = 0; for (size_t i = 0; i < cyc.size(); i += 8) device_cycles = std::max(device_cycles, cyc[i]);");
    w(
        &mut h,
        "    fmt::print(\"LOGOS_MM_DEVICE_CYCLES={}\\n\", device_cycles);",
    );
    w(&mut h, "    mesh_device->close();");
    w(&mut h, "    return ok ? 0 : 1;");
    w(&mut h, "}");
    Ok(h)
}

pub fn print_cmake() -> String {
    [
        "cmake_minimum_required(VERSION 3.22...4.2)",
        "project(metal_example_logos_spatial)",
        "add_executable(metal_example_logos_spatial host.cpp)",
        "if(NOT TARGET TT::Metalium)",
        "    find_package(TT-Metalium REQUIRED)",
        "endif()",
        "target_link_libraries(metal_example_logos_spatial PUBLIC TT::Metalium)",
        "target_compile_definitions(metal_example_logos_spatial PRIVATE LOGOS_KERNEL_DIR=\"${CMAKE_CURRENT_SOURCE_DIR}/kernels/\")",
        "if(TARGET TT::CommonPCH)",
        "    tt_reuse_precompile_headers(metal_example_logos_spatial TT::CommonPCH)",
        "endif()",
        "",
    ]
    .join("\n")
}
