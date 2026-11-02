//! Factory calibration — manufacturing ADJUSTABLE circuits, not impossibly perfect ones.
//!
//! Production test measures each die through the loopback paths (DAC → tile → ADC), computes
//! per-leg gain trims and per-tile offset trims on the trim grid, and burns them beside the
//! configuration. The effective die is `compiler programming + per-die calibration`; what the
//! budget then carries for gain and offset is the RESIDUAL — half a trim step, a deterministic
//! bound, instead of the full k·σ manufacturing envelope. Noise cannot be trimmed and keeps its
//! envelope. This is the lever that turns per-die physics into a product-grade ε.

use super::cell::AnalogNetlist;
use super::cell::{ASrc, AnalogCell};
use super::error_budget::{bump_mul, outward_sum, ErrorSource, ErrorTerm, OutputBudget};
use super::gain::{GainSchedule, MathBound, RangeCertificate};
use super::nonideal::{InstanceSample, NonIdealityModel, KSIGMA};
use super::place::AnalogPlacement;
use super::target::AnalogTarget;

/// The trim DAC resolution — a design parameter of the tile (finer than the weight grid, since
/// trims correct errors, not values).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrimSpec {
    /// Fractional gain-trim step.
    pub gain_step: f64,
    /// Offset-trim step in volts.
    pub offset_step_v: f64,
}

/// The V1 tile's trim resolution: gain trim = a quarter unit element; offset trim = 0.5 mV
/// steps (a 6-bit trim over ±16 mV). Design parameters of OUR tile, not silicon claims.
pub fn v1_trim(target: &AnalogTarget) -> TrimSpec {
    TrimSpec {
        gain_step: target.weight_lsb() / 4.0,
        offset_step_v: 0.5e-3,
    }
}

/// Per-die trim values, each ON the trim grid.
#[derive(Clone, Debug, PartialEq)]
pub struct CalibrationTable {
    pub gain_trim: Vec<Vec<f64>>,
    pub offset_trim_v: Vec<f64>,
}

fn snap(x: f64, step: f64) -> f64 {
    (x / step).round() * step
}

/// Factory calibration of one measured die: the quantized negation of every measured error.
pub fn calibrate(inst: &InstanceSample, spec: &TrimSpec) -> CalibrationTable {
    CalibrationTable {
        gain_trim: inst
            .leg_gain_err
            .iter()
            .map(|legs| legs.iter().map(|e| snap(-e, spec.gain_step)).collect())
            .collect(),
        offset_trim_v: inst
            .offset_v
            .iter()
            .map(|o| snap(-o, spec.offset_step_v))
            .collect(),
    }
}

/// The calibrated die: residual gain/offset errors (≤ half a trim step by construction), the
/// noise realization untouched (noise is dynamic — no trim reaches it).
pub fn apply_calibration(inst: &InstanceSample, table: &CalibrationTable) -> InstanceSample {
    InstanceSample {
        leg_gain_err: inst
            .leg_gain_err
            .iter()
            .zip(&table.gain_trim)
            .map(|(legs, trims)| legs.iter().zip(trims).map(|(e, t)| e + t).collect())
            .collect(),
        offset_v: inst
            .offset_v
            .iter()
            .zip(&table.offset_trim_v)
            .map(|(o, t)| o + t)
            .collect(),
        noise_v: inst.noise_v.clone(),
    }
}

/// The CALIBRATED physical budget: gain and offset carry the residual half-step bound instead
/// of the k·σ manufacturing envelope; noise keeps its envelope; the converter/quantization
/// terms are unchanged.
pub fn error_budget_calibrated(
    netlist: &AnalogNetlist,
    cert: &RangeCertificate,
    schedule: &GainSchedule,
    target: &AnalogTarget,
    model: &NonIdealityModel,
    _placement: &AnalogPlacement,
    spec: &TrimSpec,
) -> OutputBudget {
    let base = super::error_budget::error_budget(netlist, cert, schedule, target);
    let realized = super::cell::AnalogNetlist {
        n_in: schedule.quantized.n_in,
        cells: schedule.realized_math_cells(),
        out: schedule.quantized.out,
    };
    let s_of = |i: usize| f64::powi(2.0, schedule.per_net_scale_log2[i]);
    let q = &realized;
    let bound_of = |src: &ASrc| -> MathBound {
        match src {
            ASrc::In(k) => cert.input_bounds[*k],
            ASrc::Val(v) => cert.per_net[*v].math,
            ASrc::Const(c) => MathBound { lo: *c, hi: *c },
        }
    };
    // Residual per-cell deltas: half a trim step, deterministic (calibration nulls the whole
    // measured error INCLUDING the distance term — that is the point of measuring the die).
    let mut gain_d = Vec::with_capacity(q.cells.len());
    let mut off_d = Vec::with_capacity(q.cells.len());
    let mut noise_d = Vec::with_capacity(q.cells.len());
    for (i, cell) in q.cells.iter().enumerate() {
        let g = match cell {
            AnalogCell::Sum { inputs, weights } => outward_sum(
                inputs
                    .iter()
                    .zip(weights)
                    .map(|(x, w)| bump_mul(spec.gain_step / 2.0, w.abs() * bound_of(x).max_abs())),
            ),
            AnalogCell::GainConst { a, k } => {
                bump_mul(spec.gain_step / 2.0, k.abs() * bound_of(a).max_abs())
            }
            AnalogCell::Mul { a, b } => bump_mul(
                spec.gain_step / 2.0,
                bound_of(a).max_abs() * bound_of(b).max_abs(),
            ),
            AnalogCell::Select { .. } => 0.0,
            AnalogCell::Exp { .. }
            | AnalogCell::Log { .. }
            | AnalogCell::Tanh { .. }
            | AnalogCell::Sqrt { .. }
            | AnalogCell::Div { .. } => {
                bump_mul(spec.gain_step / 2.0, cert.per_net[i].math.max_abs())
            }
        };
        gain_d.push(g);
        off_d.push((spec.offset_step_v / 2.0 / s_of(i)).next_up());
        noise_d.push((KSIGMA * model.noise_floor_v.0 / s_of(i)).next_up());
    }
    let mut terms = base.terms;
    let tg = super::error_budget::propagate_injected_pub(q, &bound_of, &gain_d);
    if tg > 0.0 {
        terms.push(ErrorTerm {
            eps_math: tg,
            source: ErrorSource::TileGain,
        });
    }
    let to = super::error_budget::propagate_injected_pub(q, &bound_of, &off_d);
    if to > 0.0 {
        terms.push(ErrorTerm {
            eps_math: to,
            source: ErrorSource::TileOffset,
        });
    }
    let tn = super::error_budget::propagate_injected_pub(q, &bound_of, &noise_d);
    if tn > 0.0 {
        terms.push(ErrorTerm {
            eps_math: tn,
            source: ErrorSource::Noise,
        });
    }
    let eps_math = outward_sum(terms.iter().map(|t| t.eps_math));
    OutputBudget { eps_math, terms }
}
