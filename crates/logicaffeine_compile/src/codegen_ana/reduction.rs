//! The streamed reduction — the flagship MAC lane.
//!
//! A float-accumulator loop over a sequence (`Repeat for each x in xs: Set acc to acc + …`)
//! becomes ONE analog tick datapath run once per element under the control plane's frame
//! schedule: the element is validated (the `Assume` contract), DAC-driven, the plane settles,
//! and the sample-and-hold commits the new accumulator — which LIVES IN ANALOG between frames,
//! never re-quantized. Only the final accumulator crosses the ADC. That asymmetry (per-element
//! DAC cost, one ADC, zero mid-stream conversions) is the analog win this lane exists to state.
//!
//! Semantics are sampled-data ZOH: each frame's update is exact arithmetic over the held
//! element, so the ideal tier reproduces the VM's loop trajectory bit-for-bit, and the physical
//! tier (converters + quantized weights) stays inside a budget ITERATED over the declared
//! maximum frame count — which is why a `Assume length of xs is between A and B.` contract is
//! REQUIRED: a budget over an unbounded stream would be a guess, and this lane does not guess.
//!
//! Extraction reuses the digital fabric's certified counted-loop front-end
//! (`loop_fabric_from_source`, read-only) on the contract-free twin, exactly like the DAG lane.

use std::collections::BTreeMap;

use crate::ast::stmt::{Pattern, Stmt, TypeExpr};
use crate::codegen_sva::fabric_rtl::{loop_fabric_from_source, LoopCell, Src};

use super::cell::{ASrc, AnalogCell, AnalogNetlist};
use super::control::{ControlProgram, CtrlStep};
use super::error_budget::{outward_sum, ErrorSource, ErrorTerm, OutputBudget};
use super::eval::eval_analog_dc;
use super::gain::{
    propagate_bounds, quantize_weights, select_scale, src_bound_of, GainSchedule, MathBound,
    NetBound, RangeCertificate,
};
use super::lower::{map_float_cell, source_without_range_contracts, MappedCell, NetRef};
use super::target::{AnaUnplaceable, AnalogCapability, AnalogCellKind, AnalogTarget};
use super::{assumed_len_ranges_in, assumed_ranges_in, AnaLowerError, AssumedRange};

/// A lowered streamed reduction. Tick-input convention: `In(0)` = the streamed element,
/// `In(1)` = the held accumulator; `out` = the next accumulator value.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalogReduction {
    pub tick: AnalogNetlist,
    /// The accumulator's initial value (a compile-time constant in this lane).
    pub init: f64,
    /// The declared length contract — the control plane schedules `max_len` frames.
    pub min_len: usize,
    pub max_len: usize,
    /// The declared element range (each word validated before the DAC fires).
    pub elem: AssumedRange,
    pub control: ControlProgram,
}

/// Which numeric family a streamed lane carries — decides constant decoding (float bit
/// patterns vs integer values), the cell mapper, and the grid-preservation obligation.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lane {
    Float,
    Int,
}

/// Lower `fn_name` (a single-`Seq of Float`-parameter accumulator loop) to a streamed analog
/// reduction realizable on `target`.
pub fn analog_reduction_from_source(
    src: &str,
    fn_name: &str,
    target: &AnalogTarget,
) -> Result<AnalogReduction, AnaLowerError> {
    reduction_impl(src, fn_name, target, Lane::Float)
}

/// Lower a `Seq of Int` accumulator loop to a GRID-PRESERVING streamed reduction — the
/// integer MAC. The exactness claim is made downstream by [`super::exactness_gate`] over the
/// iterated budget.
pub fn analog_int_reduction_from_source(
    src: &str,
    fn_name: &str,
    target: &AnalogTarget,
) -> Result<AnalogReduction, AnaLowerError> {
    reduction_impl(src, fn_name, target, Lane::Int)
}

fn reduction_impl(
    src: &str,
    fn_name: &str,
    target: &AnalogTarget,
    lane: Lane,
) -> Result<AnalogReduction, AnaLowerError> {
    let (elem, min_len, max_len) = check_reduction_contracts(src, fn_name, lane)?;

    let twin = source_without_range_contracts(src);
    let lf = loop_fabric_from_source(&twin, fn_name);

    // ── Shape gates: the single-accumulator streamed family. ──
    if lf.seq_input.is_none() {
        return Err(AnaLowerError::UnloweredOp(
            "not a sequence loop (no streamed input)".to_string(),
        ));
    }
    if lf.n_state != 1 {
        return Err(AnaLowerError::UnloweredOp(format!(
            "{} loop-carried accumulators (the streamed lane holds exactly one in the S&H)",
            lf.n_state
        )));
    }
    if !lf.epilogue.is_empty() || !lf.prologue_heap.is_empty() || !lf.invariants.is_empty() {
        return Err(AnaLowerError::UnloweredOp(
            "loop epilogue/prologue/invariants are not yet streamed".to_string(),
        ));
    }
    let init = match lf.init[0] {
        Src::Const(bits) => match lane {
            Lane::Float => f64::from_bits(bits as u64),
            Lane::Int => bits as f64,
        },
        _ => {
            return Err(AnaLowerError::UnloweredOp(
                "accumulator initial value must be a compile-time constant".to_string(),
            ))
        }
    };

    // ── Map the body: MemRead(iv) → the streamed element; state → the held accumulator. ──
    // Body-input convention (LoopFabric): In(0) = state, In(1) = the induction variable.
    for cell in &lf.body {
        let uses_iv_as_value = |s: &Src| matches!(s, Src::In(k) if *k >= 1);
        let illegal = match cell {
            LoopCell::MemRead { .. } => false,
            LoopCell::Arith { a, b, .. }
            | LoopCell::Func { a, b, .. }
            | LoopCell::VectorArith { a, b, .. } => uses_iv_as_value(a) || uses_iv_as_value(b),
            LoopCell::Select { ca, cb, t, e, .. } => {
                uses_iv_as_value(ca)
                    || uses_iv_as_value(cb)
                    || uses_iv_as_value(t)
                    || uses_iv_as_value(e)
            }
            LoopCell::HeapRead { addr } => uses_iv_as_value(addr),
            LoopCell::HeapWrite { addr, val } => uses_iv_as_value(addr) || uses_iv_as_value(val),
            LoopCell::HeapAlloc { size } => uses_iv_as_value(size),
        };
        if illegal {
            return Err(AnaLowerError::UnloweredOp(
                "the induction variable used as a value (only the element stream is analog)"
                    .to_string(),
            ));
        }
    }

    // Pre-mark the element wires: every `MemRead(iv)` IS tick input 0 (no tile, just the wire).
    let mut map: BTreeMap<usize, NetRef> = BTreeMap::new();
    for (i, cell) in lf.body.iter().enumerate() {
        if let LoopCell::MemRead { addr } = cell {
            if !matches!(addr, Src::In(1)) {
                return Err(AnaLowerError::UnloweredOp(
                    "sequence read off the stream head (only `xs[iv]` streams)".to_string(),
                ));
            }
            map.insert(i, NetRef::Cell(usize::MAX)); // sentinel: the element wire, resolved as In(0)
        }
    }
    let mut cells: Vec<AnalogCell> = Vec::new();
    let resolve_with = |map: &BTreeMap<usize, NetRef>, s: &Src| -> ASrc {
        match s {
            Src::In(0) => ASrc::In(1), // the held accumulator
            Src::In(_) => ASrc::In(1), // unreachable: iv-as-value gated above
            Src::Const(bits) => ASrc::Const(match lane {
                Lane::Float => f64::from_bits(*bits as u64),
                Lane::Int => *bits as f64,
            }),
            Src::Val(v) => match map.get(v) {
                Some(NetRef::Cell(c)) if *c == usize::MAX => ASrc::In(0), // the element wire
                Some(NetRef::Cell(c)) => ASrc::Val(*c),
                Some(NetRef::Const(f)) => ASrc::Const(*f),
                None => ASrc::Val(usize::MAX),
            },
        }
    };
    for (i, cell) in lf.body.iter().enumerate() {
        if matches!(cell, LoopCell::MemRead { .. }) {
            continue;
        }
        let resolve = |s: &Src| resolve_with(&map, s);
        let mapped = match lane {
            Lane::Float => map_float_cell(cell, &resolve)?,
            Lane::Int => super::int_lane::map_int_cell(cell, &resolve)?,
        };
        let mapped = match mapped {
            MappedCell::Tile(t) => NetRef::Cell(push(&mut cells, t)),
            MappedCell::Tiles(ts) => NetRef::Cell(super::lower::push_tiles(&mut cells, ts)),
            MappedCell::Fold(v) => NetRef::Const(v),
        };
        map.insert(i, mapped);
    }
    let out = resolve_with(&map, &lf.next[0]);
    if matches!(out, ASrc::Val(usize::MAX)) {
        return Err(AnaLowerError::UnloweredOp(
            "accumulator update references an unlowered value".to_string(),
        ));
    }
    let tick = AnalogNetlist {
        n_in: 2,
        cells,
        out,
    };
    if lane == Lane::Int {
        super::int_lane::assert_grid_preserving(&tick)?;
        if init.fract() != 0.0 {
            return Err(AnaLowerError::UnloweredOp(
                "non-integer accumulator initial value in the integer lane".to_string(),
            ));
        }
    }

    gate_reduction_on_target(&tick, target)?;

    let control = ControlProgram {
        frame: vec![
            CtrlStep::LoadDac { ch: 0 },
            CtrlStep::Settle,
            CtrlStep::SampleHold,
        ],
        frames: max_len,
        epilogue: vec![CtrlStep::ReadAdc { ch: 0 }],
        tick_ns: target.settle_ns.0,
    };
    Ok(AnalogReduction {
        tick,
        init,
        min_len,
        max_len,
        elem,
        control,
    })
}

fn push(cells: &mut Vec<AnalogCell>, c: AnalogCell) -> usize {
    cells.push(c);
    cells.len() - 1
}

/// Contract gates: one sequence parameter of the lane's element family with a declared LENGTH
/// range, and a declared ELEMENT range on the loop variable (integer bounds for the int lane).
fn check_reduction_contracts(
    src: &str,
    fn_name: &str,
    lane: Lane,
) -> Result<(AssumedRange, usize, usize), AnaLowerError> {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _pol) = match parsed {
            Ok(t) => t,
            Err(advice) => {
                return Err(AnaLowerError::UnloweredOp(format!(
                    "parse failed: {advice}"
                )))
            }
        };
        let Some((params, body)) = stmts.iter().find_map(|s| match s {
            Stmt::FunctionDef {
                name, params, body, ..
            } if interner.resolve(*name) == fn_name => Some((params, *body)),
            _ => None,
        }) else {
            return Err(AnaLowerError::UnloweredOp(format!(
                "function `{fn_name}` not found"
            )));
        };
        if params.len() != 1 {
            return Err(AnaLowerError::UnloweredOp(
                "the streamed lane takes exactly one sequence parameter".to_string(),
            ));
        }
        let (pname_sym, pty) = &params[0];
        let pname = interner.resolve(*pname_sym);
        let elem_matches = |s: &str| match lane {
            Lane::Float => matches!(s, "Float" | "Real"),
            Lane::Int => matches!(s, "Int" | "Nat"),
        };
        let is_lane_seq = matches!(
            pty,
            TypeExpr::Generic { base, params: ps }
                if matches!(interner.resolve(*base), "Seq" | "List")
                    && ps.len() == 1
                    && matches!(&ps[0], TypeExpr::Primitive(s) | TypeExpr::Named(s)
                        if elem_matches(interner.resolve(*s)))
        );
        if !is_lane_seq {
            return Err(AnaLowerError::NoAnalogRegion);
        }

        let len_ranges = assumed_len_ranges_in(body, interner);
        let Some(len) = len_ranges.iter().find(|r| r.var == pname) else {
            return Err(AnaLowerError::UnboundedInput(format!(
                "sequence `{pname}` has no declared length range (`Assume length of {pname} is between A and B.`) — \
                 a budget over an unbounded stream would be a guess"
            )));
        };
        if len.lo < 0 || len.hi < len.lo || len.hi == 0 {
            return Err(AnaLowerError::UnboundedInput(format!(
                "length range [{}, {}] is not a usable frame bound",
                len.lo, len.hi
            )));
        }

        // The loop variable's declared range = the element contract.
        let loop_var = body.iter().find_map(|s| match s {
            Stmt::Repeat {
                pattern: Pattern::Identifier(v),
                ..
            } => Some(*v),
            _ => None,
        });
        let Some(loop_var) = loop_var else {
            return Err(AnaLowerError::UnloweredOp(
                "no `Repeat for each` loop found".to_string(),
            ));
        };
        let vname = interner.resolve(loop_var);
        let ranges = assumed_ranges_in(body, interner);
        let Some(elem) = ranges.iter().find(|r| r.var == vname) else {
            return Err(AnaLowerError::UnboundedInput(format!(
                "element `{vname}` has no declared range (`Assume {vname} is between A and B.`)"
            )));
        };
        if lane == Lane::Int && (elem.lo.fract() != 0.0 || elem.hi.fract() != 0.0) {
            return Err(AnaLowerError::UnboundedInput(format!(
                "element `{vname}`'s range [{}, {}] is not an integer range",
                elem.lo, elem.hi
            )));
        }
        Ok((elem.clone(), len.lo as usize, len.hi as usize))
    })
}

fn gate_reduction_on_target(
    tick: &AnalogNetlist,
    target: &AnalogTarget,
) -> Result<(), AnaLowerError> {
    let mut need = vec![
        AnalogCapability::Cell(AnalogCellKind::SampleHold),
        AnalogCapability::DacChannels(1),
        AnalogCapability::AdcChannels(1),
    ];
    for cell in &tick.cells {
        need.push(AnalogCapability::Cell(cell.kind()));
    }
    for cap in need {
        if !target.provides(cap) {
            return Err(AnaLowerError::Unplaceable(
                AnaUnplaceable::MissingCapability {
                    needed: cap,
                    target: target.name,
                },
            ));
        }
    }
    let n_mul = tick
        .cells
        .iter()
        .filter(|c| matches!(c, AnalogCell::Mul { .. }))
        .count();
    if n_mul > 0 && !target.provides(AnalogCapability::MulTiles(n_mul)) {
        return Err(AnaLowerError::Unplaceable(
            AnaUnplaceable::MissingCapability {
                needed: AnalogCapability::MulTiles(n_mul),
                target: target.name,
            },
        ));
    }
    if tick.cells.len() > target.rows * target.cols {
        return Err(AnaLowerError::Unplaceable(AnaUnplaceable::TooSmall {
            needed_cells: tick.cells.len(),
            target_rows: target.rows,
            target_cols: target.cols,
            target: target.name,
        }));
    }
    Ok(())
}

/// Ideal sampled evaluation: the VM's loop trajectory, tick by tick (bit-exact — same f64 ops
/// in the same order).
pub fn eval_analog_sampled(r: &AnalogReduction, xs: &[f64]) -> f64 {
    let mut acc = r.init;
    for x in xs {
        acc = eval_analog_dc(&r.tick, &[*x, acc]);
    }
    acc
}

/// The physical sampled tier: each element DAC-quantized on entry, the accumulator CONTINUOUS
/// across frames (it lives in the S&H), one ADC quantization at the end — over the
/// weight-quantized tick.
pub fn eval_analog_sampled_quantized(
    r: &AnalogReduction,
    schedule: &GainSchedule,
    target: &AnalogTarget,
    xs: &[f64],
) -> f64 {
    let grid = super::eval::ConverterGrid::of(schedule, target);
    let mut acc = r.init;
    for x in xs {
        acc = eval_analog_dc(&schedule.quantized, &[grid.dac_q(*x), acc]);
    }
    grid.adc_q(acc)
}

/// Schedule + certify the reduction: quantize the tick's weights, ITERATE the interval
/// propagation over the declared maximum frame count (hulling per net), choose the block scale
/// over the whole trajectory, and emit the certificate the independent checker replays with the
/// same iteration.
pub fn reduction_scale_pass(
    r: &AnalogReduction,
    target: &AnalogTarget,
) -> Result<(GainSchedule, RangeCertificate), AnaLowerError> {
    let quantized = quantize_weights(&r.tick, target)?;
    let elem = MathBound {
        lo: r.elem.lo,
        hi: r.elem.hi,
    };
    let mut acc = MathBound {
        lo: r.init,
        hi: r.init,
    };
    let mut hulls: Vec<MathBound> = vec![
        MathBound {
            lo: f64::INFINITY,
            hi: f64::NEG_INFINITY
        };
        quantized.cells.len()
    ];
    let mut acc_hull = acc;
    for _ in 0..r.max_len {
        let inputs = [elem, acc];
        let bounds = propagate_bounds(&quantized.cells, &inputs)?;
        for (h, b) in hulls.iter_mut().zip(&bounds) {
            h.lo = h.lo.min(b.lo);
            h.hi = h.hi.max(b.hi);
        }
        acc = src_bound_of(&quantized.out, &inputs, &bounds);
        acc_hull.lo = acc_hull.lo.min(acc.lo);
        acc_hull.hi = acc_hull.hi.max(acc.hi);
        if !(acc.lo.is_finite() && acc.hi.is_finite()) {
            return Err(AnaLowerError::UnloweredOp(
                "accumulator range diverges over the declared frame count".to_string(),
            ));
        }
    }
    // An empty-body (identity) reduction leaves hulls untouched; normalize.
    for h in hulls.iter_mut() {
        if h.lo > h.hi {
            *h = MathBound { lo: 0.0, hi: 0.0 };
        }
    }

    let has_mul = quantized
        .cells
        .iter()
        .any(|c| matches!(c, AnalogCell::Mul { .. }));
    let input_bounds = vec![elem, acc_hull];
    let all: Vec<MathBound> = input_bounds.iter().chain(hulls.iter()).copied().collect();
    let scale_log2 = select_scale(&all, has_mul, target)?;
    let s = f64::powi(2.0, scale_log2);

    let mut per_net = Vec::with_capacity(hulls.len());
    for b in &hulls {
        per_net.push(NetBound {
            math: *b,
            volt_lo: (b.lo * s).next_down(),
            volt_hi: (b.hi * s).next_up(),
        });
    }
    let per_net_scale_log2 = vec![scale_log2; quantized.cells.len()];
    let schedule = GainSchedule {
        scale_log2,
        out_descale_log2: -scale_log2,
        mul_vref_v: s,
        quantized,
        per_net_scale_log2,
    };
    let cert = RangeCertificate {
        input_bounds,
        per_net,
        rail_half_span_v: target.half_span_v(),
        noise_floor_v: 0.0,
        min_full_scale_over_noise: 1.0,
    };
    Ok((schedule, cert))
}

/// The reduction budget: the shared ε engine ITERATED over the declared maximum frame count —
/// the element's DAC half-LSB enters every frame, the weight-snap deviation injects every
/// frame, the accumulator's lanes carry, and one ADC half-LSB lands at the end.
pub fn reduction_error_budget(
    r: &AnalogReduction,
    cert: &RangeCertificate,
    schedule: &GainSchedule,
    target: &AnalogTarget,
) -> OutputBudget {
    use super::error_budget::{eval_rounding_guard, half_lsb, out_lanes, propagate_eps_pass};
    let dac_eps = half_lsb(cert, schedule, target.dac_bits);
    let adc_eps = half_lsb(cert, schedule, target.adc_bits);
    let bound_of = |src: &ASrc| -> MathBound {
        match src {
            ASrc::In(k) => cert.input_bounds[*k],
            ASrc::Val(v) => cert.per_net[*v].math,
            ASrc::Const(c) => MathBound { lo: *c, hi: *c },
        }
    };
    // Lanes: 0 = element DAC, 1 = weight quant, 2 = Mul interaction; the accumulator carries.
    let mut acc_eps = vec![0.0_f64; 3];
    for _ in 0..r.max_len {
        let carried = acc_eps.clone();
        let input_eps = |src: &ASrc, lane: usize| -> f64 {
            match src {
                ASrc::In(0) if lane == 0 => dac_eps,
                ASrc::In(0) => 0.0,
                _ => carried[lane],
            }
        };
        let input_total = |src: &ASrc| -> f64 {
            match src {
                ASrc::In(0) => dac_eps,
                _ => outward_sum(carried.iter().copied()),
            }
        };
        let pass = propagate_eps_pass(
            &r.tick.cells,
            &schedule.quantized.cells,
            &bound_of,
            3,
            1,
            2,
            &input_eps,
            &input_total,
        );
        acc_eps = out_lanes(&pass, &schedule.quantized.out, 3, &input_eps);
    }

    let mut terms = Vec::new();
    if acc_eps[0] > 0.0 {
        terms.push(ErrorTerm {
            eps_math: acc_eps[0],
            source: ErrorSource::DacQuant {
                input: 0,
                bits: target.dac_bits,
            },
        });
    }
    if acc_eps[1] > 0.0 {
        terms.push(ErrorTerm {
            eps_math: acc_eps[1],
            source: ErrorSource::WeightQuant,
        });
    }
    if acc_eps[2] > 0.0 {
        terms.push(ErrorTerm {
            eps_math: acc_eps[2],
            source: ErrorSource::MulInteraction,
        });
    }
    terms.push(ErrorTerm {
        eps_math: adc_eps,
        source: ErrorSource::AdcQuant {
            bits: target.adc_bits,
        },
    });
    terms.push(ErrorTerm {
        eps_math: eval_rounding_guard(cert, r.max_len * schedule.quantized.cells.len()),
        source: ErrorSource::EvalRounding,
    });
    let eps_math = outward_sum(terms.iter().map(|t| t.eps_math));
    OutputBudget { eps_math, terms }
}
