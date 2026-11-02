//! The ZOH integrator lane — counted Euler loops as OTA-C integration.
//!
//! The recognized idiom is explicit Euler: `Set x to x + <h·f(state)>` inside a counted
//! `Repeat`, possibly over SEVERAL mutable states (a cross-coupled oscillator is two). Each
//! state lives in a sample-and-hold; each frame the OTA legs integrate the held inputs for the
//! settling window and the S&H commits — zero-order hold, so the per-frame update is EXACT
//! arithmetic and the certified semantics is the VM's Euler trajectory, not an ODE-solver
//! approximation. The physical gate is the OTA rate identity: a programmed per-frame gain `c`
//! over frame time `T` demands `gm/C = |c|/T`, which must sit in the target's realizable
//! `integrator_k_range` — refused typed otherwise.
//!
//! Structurally this is the reduction lane without a stream: state parameters enter through the
//! DAC once (frame 0), the trajectory never leaves the analog plane, and one ADC reads the
//! result after the declared frame count.

use std::collections::BTreeMap;

use crate::ast::stmt::{Stmt, TypeExpr};
use crate::codegen_sva::fabric_rtl::{loop_fabric_from_source, LoopCell, Src};

use super::cell::{ASrc, AnalogCell, AnalogNetlist};
use super::control::{ControlProgram, CtrlStep};
use super::error_budget::{outward_sum, ErrorSource, ErrorTerm, OutputBudget};
use super::gain::{
    propagate_bounds, quantize_weights, select_scale, src_bound_of, GainSchedule, MathBound,
    NetBound, RangeCertificate,
};
use super::lower::{map_float_cell, source_without_range_contracts, MappedCell, NetRef};
use super::target::{AnaUnplaceable, AnalogCapability, AnalogCellKind, AnalogTarget};
use super::{assumed_ranges_in, AnaLowerError, AssumedRange};

/// A state's initial value: a function parameter (DAC-driven at frame 0) or a constant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InitSrc {
    Param(usize),
    Const(f64),
}

/// A lowered ZOH integrator. Tick-input convention: `In(s)` = held state `s`; `outs[s]` = the
/// next value of state `s`; the declared result is `outs`-resolved `result_state`.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalogIntegrator {
    pub tick: AnalogNetlist,
    pub outs: Vec<ASrc>,
    pub init: Vec<InitSrc>,
    pub frames: usize,
    pub result_state: usize,
    /// Declared ranges for the DAC-driven initial states, in parameter order.
    pub param_ranges: Vec<AssumedRange>,
    pub control: ControlProgram,
}

/// Lower a counted Euler loop over float state(s) to a ZOH integrator on `target`.
pub fn analog_integrator_from_source(
    src: &str,
    fn_name: &str,
    target: &AnalogTarget,
) -> Result<AnalogIntegrator, AnaLowerError> {
    let param_ranges = check_integrator_contracts(src, fn_name)?;

    let twin = source_without_range_contracts(src);
    let lf = loop_fabric_from_source(&twin, fn_name);

    if lf.seq_input.is_some() || lf.seq_heap_ptr.is_some() {
        return Err(AnaLowerError::UnloweredOp(
            "a streamed loop belongs to the reduction lane".to_string(),
        ));
    }
    if !lf.epilogue.is_empty() || !lf.prologue_heap.is_empty() || !lf.invariants.is_empty() {
        return Err(AnaLowerError::UnloweredOp(
            "loop epilogue/prologue/invariants are not yet integrated".to_string(),
        ));
    }
    let frames = match (lf.lo, lf.hi) {
        (Src::Const(lo), Src::Const(hi)) if hi >= lo => (hi - lo + 1) as usize,
        _ => {
            return Err(AnaLowerError::UnloweredOp(
                "the trip count must be a compile-time constant (the frame schedule is static)"
                    .to_string(),
            ))
        }
    };
    let n_state = lf.n_state;

    // Initial values: parameters (DAC at frame 0) or constants.
    let mut init = Vec::with_capacity(n_state);
    for s in &lf.init {
        init.push(match s {
            Src::In(k) => InitSrc::Param(*k),
            Src::Const(bits) => InitSrc::Const(f64::from_bits(*bits as u64)),
            Src::Val(_) => {
                return Err(AnaLowerError::UnloweredOp(
                    "a computed initial state is not yet integrated".to_string(),
                ))
            }
        });
    }
    for (s, i) in init.iter().enumerate() {
        if let InitSrc::Param(k) = i {
            if *k >= param_ranges.len() {
                return Err(AnaLowerError::UnboundedInput(format!(
                    "state {s} initializes from parameter {k}, which has no declared range"
                )));
            }
        }
    }

    // Map the body: held states are the tick inputs; the induction variable must not be a value.
    // Body-input convention: In(0..n_state) = states, In(n_state) = the induction variable.
    for cell in &lf.body {
        let illegal = |s: &Src| matches!(s, Src::In(k) if *k >= n_state);
        let bad = match cell {
            LoopCell::Arith { a, b, .. }
            | LoopCell::Func { a, b, .. }
            | LoopCell::VectorArith { a, b, .. } => illegal(a) || illegal(b),
            LoopCell::Select { ca, cb, t, e, .. } => {
                illegal(ca) || illegal(cb) || illegal(t) || illegal(e)
            }
            _ => true, // Mem/Heap in a counted integrator body has no analog home
        };
        if bad {
            return Err(AnaLowerError::UnloweredOp(
                "the induction variable / memory used inside an integrator body".to_string(),
            ));
        }
    }
    let mut map: BTreeMap<usize, NetRef> = BTreeMap::new();
    let mut cells: Vec<AnalogCell> = Vec::new();
    let resolve_with = |map: &BTreeMap<usize, NetRef>, s: &Src| -> ASrc {
        match s {
            Src::In(k) => ASrc::In(*k), // held state k
            Src::Const(bits) => ASrc::Const(f64::from_bits(*bits as u64)),
            Src::Val(v) => match map.get(v) {
                Some(NetRef::Cell(c)) => ASrc::Val(*c),
                Some(NetRef::Const(f)) => ASrc::Const(*f),
                None => ASrc::Val(usize::MAX),
            },
        }
    };
    for (i, cell) in lf.body.iter().enumerate() {
        let resolve = |s: &Src| resolve_with(&map, s);
        let mapped = match map_float_cell(cell, &resolve)? {
            MappedCell::Tile(t) => NetRef::Cell(push(&mut cells, t)),
            MappedCell::Tiles(ts) => NetRef::Cell(super::lower::push_tiles(&mut cells, ts)),
            MappedCell::Fold(v) => NetRef::Const(v),
        };
        map.insert(i, mapped);
    }
    let outs: Vec<ASrc> = lf.next.iter().map(|s| resolve_with(&map, s)).collect();
    if outs.iter().any(|o| matches!(o, ASrc::Val(usize::MAX))) {
        return Err(AnaLowerError::UnloweredOp(
            "a state update references an unlowered value".to_string(),
        ));
    }
    let tick = AnalogNetlist {
        n_in: n_state,
        cells,
        out: outs[lf.result_state],
    };

    // The OTA rate identity: every programmed per-frame gain must be realizable as gm/C over
    // the settling window.
    let t_frame_s = target.settle_ns.0 * 1.0e-9;
    for cell in &tick.cells {
        if let AnalogCell::GainConst { k, .. } = cell {
            if *k != 0.0 {
                target
                    .require_integrator_rate(k.abs() / t_frame_s)
                    .map_err(AnaLowerError::Unplaceable)?;
            }
        }
    }

    gate_integrator_on_target(&tick, n_state, target)?;

    let control = ControlProgram {
        // Frame 0 loads the initial states; every frame settles and commits.
        frame: vec![CtrlStep::Settle, CtrlStep::SampleHold],
        frames,
        epilogue: vec![CtrlStep::ReadAdc { ch: 0 }],
        tick_ns: target.settle_ns.0,
    };
    Ok(AnalogIntegrator {
        tick,
        outs,
        init,
        frames,
        result_state: lf.result_state,
        param_ranges,
        control,
    })
}

fn push(cells: &mut Vec<AnalogCell>, c: AnalogCell) -> usize {
    cells.push(c);
    cells.len() - 1
}

/// Contract gates: every float parameter carries a declared range (they are the DAC-driven
/// initial states).
fn check_integrator_contracts(
    src: &str,
    fn_name: &str,
) -> Result<Vec<AssumedRange>, AnaLowerError> {
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
        let is_float = |t: &TypeExpr<'_>| {
            matches!(t, TypeExpr::Primitive(s) | TypeExpr::Named(s)
                if matches!(interner.resolve(*s), "Float" | "Real"))
        };
        if !params.iter().all(|(_, t)| is_float(t)) || params.is_empty() {
            return Err(AnaLowerError::NoAnalogRegion);
        }
        let ranges = assumed_ranges_in(body, interner);
        let mut out = Vec::with_capacity(params.len());
        for (p, _) in params.iter() {
            let pname = interner.resolve(*p);
            let Some(r) = ranges.iter().find(|r| r.var == pname) else {
                return Err(AnaLowerError::UnboundedInput(format!(
                    "parameter `{pname}` has no declared range (`Assume {pname} is between A and B.`)"
                )));
            };
            out.push(r.clone());
        }
        Ok(out)
    })
}

fn gate_integrator_on_target(
    tick: &AnalogNetlist,
    n_state: usize,
    target: &AnalogTarget,
) -> Result<(), AnaLowerError> {
    let mut need = vec![
        AnalogCapability::Cell(AnalogCellKind::Integrate),
        AnalogCapability::Cell(AnalogCellKind::SampleHold),
        AnalogCapability::DacChannels(n_state.max(1)),
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

/// One frame's next-state vector over given cells/outs (shared by the evaluators).
fn step(cells_src: &AnalogNetlist, outs: &[ASrc], state: &[f64]) -> Vec<f64> {
    let mut vals = Vec::with_capacity(cells_src.cells.len());
    for cell in &cells_src.cells {
        let v = cell.eval(state, &vals);
        vals.push(v);
    }
    outs.iter().map(|o| o.eval(state, &vals)).collect()
}

fn init_state(r: &AnalogIntegrator, params: &[f64]) -> Vec<f64> {
    r.init
        .iter()
        .map(|i| match i {
            InitSrc::Param(k) => params[*k],
            InitSrc::Const(c) => *c,
        })
        .collect()
}

/// Ideal trajectory: the VM's Euler loop, frame by frame (bit-exact — same f64 ops in order).
pub fn eval_analog_integrated(r: &AnalogIntegrator, params: &[f64]) -> f64 {
    let mut state = init_state(r, params);
    for _ in 0..r.frames {
        state = step(&r.tick, &r.outs, &state);
    }
    state[r.result_state]
}

/// The physical trajectory: initial states DAC-quantized once, the whole trajectory continuous
/// in the S&H (weight-quantized tick), one ADC at the end.
pub fn eval_analog_integrated_quantized(
    r: &AnalogIntegrator,
    schedule: &GainSchedule,
    target: &AnalogTarget,
    params: &[f64],
) -> f64 {
    let grid = super::eval::ConverterGrid::of(schedule, target);
    let mut state: Vec<f64> = init_state(r, params)
        .into_iter()
        .zip(&r.init)
        .map(|(v, i)| match i {
            InitSrc::Param(_) => grid.dac_q(v),
            InitSrc::Const(_) => v, // a programmed reference, not a DAC word
        })
        .collect();
    for _ in 0..r.frames {
        state = step(&schedule.quantized, &r.outs, &state);
    }
    grid.adc_q(state[r.result_state])
}

/// Schedule + certify: iterate the interval trajectory over the declared frames, hull per net
/// and per state, choose the block scale over everything, and emit the certificate.
pub fn integrator_scale_pass(
    r: &AnalogIntegrator,
    target: &AnalogTarget,
) -> Result<(GainSchedule, RangeCertificate), AnaLowerError> {
    let quantized = quantize_weights(&r.tick, target)?;
    let mut state: Vec<MathBound> = r
        .init
        .iter()
        .map(|i| match i {
            InitSrc::Param(k) => MathBound {
                lo: r.param_ranges[*k].lo,
                hi: r.param_ranges[*k].hi,
            },
            InitSrc::Const(c) => MathBound { lo: *c, hi: *c },
        })
        .collect();
    let mut state_hulls = state.clone();
    let mut hulls: Vec<MathBound> = vec![
        MathBound {
            lo: f64::INFINITY,
            hi: f64::NEG_INFINITY
        };
        quantized.cells.len()
    ];
    for _ in 0..r.frames {
        let bounds = propagate_bounds(&quantized.cells, &state)?;
        for (h, b) in hulls.iter_mut().zip(&bounds) {
            h.lo = h.lo.min(b.lo);
            h.hi = h.hi.max(b.hi);
        }
        let next: Vec<MathBound> = r
            .outs
            .iter()
            .map(|o| src_bound_of(o, &state, &bounds))
            .collect();
        for (h, b) in state_hulls.iter_mut().zip(&next) {
            h.lo = h.lo.min(b.lo);
            h.hi = h.hi.max(b.hi);
        }
        if next.iter().any(|b| !(b.lo.is_finite() && b.hi.is_finite())) {
            return Err(AnaLowerError::UnloweredOp(
                "state range diverges over the declared frame count".to_string(),
            ));
        }
        state = next;
    }
    for h in hulls.iter_mut() {
        if h.lo > h.hi {
            *h = MathBound { lo: 0.0, hi: 0.0 };
        }
    }

    let has_mul = quantized
        .cells
        .iter()
        .any(|c| matches!(c, AnalogCell::Mul { .. }));
    let input_bounds = state_hulls;
    let all: Vec<MathBound> = input_bounds.iter().chain(hulls.iter()).copied().collect();
    let scale_log2 = select_scale(&all, has_mul, target)?;
    let s = f64::powi(2.0, scale_log2);
    let per_net = hulls
        .iter()
        .map(|b| NetBound {
            math: *b,
            volt_lo: (b.lo * s).next_down(),
            volt_hi: (b.hi * s).next_up(),
        })
        .collect();
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

/// The iterated integrator budget: the shared ε engine run once per frame over the held
/// state vector — initial DAC errors on the parameter states carry through every frame, the
/// weight-snap deviation injects per frame, one ADC half-LSB lands at the end.
pub fn integrator_error_budget(
    r: &AnalogIntegrator,
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
    // Lanes: 0 = DAC (parameter states), 1 = weight quant, 2 = Mul interaction — per state.
    let mut state_eps: Vec<Vec<f64>> = r
        .init
        .iter()
        .map(|i| match i {
            InitSrc::Param(_) => vec![dac_eps, 0.0, 0.0],
            InitSrc::Const(_) => vec![0.0; 3],
        })
        .collect();
    for _ in 0..r.frames {
        let carried = state_eps.clone();
        let input_eps = |src: &ASrc, lane: usize| -> f64 {
            match src {
                ASrc::In(k) => carried[*k][lane],
                _ => 0.0,
            }
        };
        let input_total = |src: &ASrc| -> f64 {
            match src {
                ASrc::In(k) => outward_sum(carried[*k].iter().copied()),
                _ => 0.0,
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
        state_eps = r
            .outs
            .iter()
            .map(|o| out_lanes(&pass, o, 3, &input_eps))
            .collect();
    }

    let out_eps = &state_eps[r.result_state];
    let mut terms = Vec::new();
    if out_eps[0] > 0.0 {
        terms.push(ErrorTerm {
            eps_math: out_eps[0],
            source: ErrorSource::DacQuant {
                input: 0,
                bits: target.dac_bits,
            },
        });
    }
    if out_eps[1] > 0.0 {
        terms.push(ErrorTerm {
            eps_math: out_eps[1],
            source: ErrorSource::WeightQuant,
        });
    }
    if out_eps[2] > 0.0 {
        terms.push(ErrorTerm {
            eps_math: out_eps[2],
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
        eps_math: eval_rounding_guard(cert, r.frames * schedule.quantized.cells.len()),
        source: ErrorSource::EvalRounding,
    });
    let eps_math = outward_sum(terms.iter().map(|t| t.eps_math));
    OutputBudget { eps_math, terms }
}
