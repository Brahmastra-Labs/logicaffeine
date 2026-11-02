//! The analog tile netlist — `codegen_ana`'s SSA IR.
//!
//! The analog analogue of the digital fabric's `LoopCell` graph, in the MATH domain: values are
//! `f64` quantities on nets (the gain-scaling pass later attaches per-net voltage scales; the
//! netlist itself stays scale-free). Cells are SSA-ordered: cell `i` defines `Val(i)`, and may
//! reference inputs, earlier vals, and constants.

use super::target::AnalogCellKind;

/// An operand of an analog tile: a plane input (a DAC channel), a prior tile's output net, or a
/// compile-time constant (a programmed reference).
///
/// Physically every operand is a DIFFERENTIAL net (`Vsignal = V⁺ − V⁻` around mid-rail): the
/// signed math value is the differential swing, which is why signed weights cost only a wire
/// swap and why the ±half-span voltage model is realizable on a single-supply process.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ASrc {
    In(usize),
    Val(usize),
    Const(f64),
}

impl ASrc {
    pub fn eval(&self, inputs: &[f64], vals: &[f64]) -> f64 {
        match self {
            ASrc::In(k) => inputs[*k],
            ASrc::Val(k) => vals[*k],
            ASrc::Const(c) => *c,
        }
    }
}

/// One analog tile. Ideal (math-domain) semantics are given per variant; physical parameters
/// (gm, C, rails) attach at the target/gain layers, not here.
#[derive(Clone, Debug, PartialEq)]
pub enum AnalogCell {
    /// Kirchhoff current-summing node with programmed input-conductance weights:
    /// `v = Σ wᵢ·xᵢ`. Signed weights are free (differential wiring).
    Sum {
        inputs: Vec<ASrc>,
        weights: Vec<f64>,
    },
    /// Four-quadrant transconductance multiplier for variable×variable: `v = a·b`.
    Mul { a: ASrc, b: ASrc },
    /// Programmed-conductance constant gain: `v = k·a`.
    GainConst { a: ASrc, k: f64 },
    /// Comparator-driven analog switch: `v = if c < 0 { t } else { f }` (the O5 tile). The
    /// condition is a DIFFERENCE net (the lowering emits `c = ca − cb` as a Sum); strict vs
    /// inclusive comparison is indistinguishable on the analog plane — the boundary lives
    /// inside the comparator's ambiguity band, which the budget prices (`CompareBand`).
    Select { c: ASrc, t: ASrc, f: ASrc },
    /// Translinear exponential (subthreshold-MOS / diff-pair current law): `v = eˣ`. NOT
    /// scale-invariant — translinear nets run at unit block scale (volt ≡ math), enforced by
    /// the schedule (the O6 pinning rule).
    Exp { a: ASrc },
    /// Translinear logarithm (forward-diode law): `v = ln x`, domain `x > 0` — certified
    /// fail-closed by the range rules.
    Log { a: ASrc },
    /// Differential-pair saturation: `v = tanh x`.
    Tanh { a: ASrc },
    /// Translinear square root (Gilbert 1975 loop): `v = √x`, domain `x > 0` for a bounded
    /// error slope (the budget's Lipschitz factor diverges at 0).
    Sqrt { a: ASrc },
    /// Translinear ratio (Gilbert 1975): `v = a / b`; the divisor's certified box must clear
    /// zero (`DivisorRangeSpansZero` otherwise — the starvation refusal).
    Div { a: ASrc, b: ASrc },
}

impl AnalogCell {
    /// The tile kind this cell demands of a target.
    pub fn kind(&self) -> AnalogCellKind {
        match self {
            AnalogCell::Sum { .. } => AnalogCellKind::Sum,
            AnalogCell::Mul { .. } => AnalogCellKind::Mul,
            AnalogCell::GainConst { .. } => AnalogCellKind::GainConst,
            AnalogCell::Select { .. } => AnalogCellKind::Select,
            AnalogCell::Exp { .. } => AnalogCellKind::Exp,
            AnalogCell::Log { .. } => AnalogCellKind::Log,
            AnalogCell::Tanh { .. } => AnalogCellKind::Tanh,
            AnalogCell::Sqrt { .. } => AnalogCellKind::Sqrt,
            AnalogCell::Div { .. } => AnalogCellKind::Div,
        }
    }

    /// Ideal math-domain evaluation.
    pub fn eval(&self, inputs: &[f64], vals: &[f64]) -> f64 {
        match self {
            AnalogCell::Sum {
                inputs: xs,
                weights,
            } => xs
                .iter()
                .zip(weights)
                .map(|(x, w)| w * x.eval(inputs, vals))
                .sum(),
            AnalogCell::Mul { a, b } => a.eval(inputs, vals) * b.eval(inputs, vals),
            AnalogCell::GainConst { a, k } => k * a.eval(inputs, vals),
            AnalogCell::Select { c, t, f } => {
                if c.eval(inputs, vals) < 0.0 {
                    t.eval(inputs, vals)
                } else {
                    f.eval(inputs, vals)
                }
            }
            AnalogCell::Exp { a } => a.eval(inputs, vals).exp(),
            AnalogCell::Log { a } => a.eval(inputs, vals).ln(),
            AnalogCell::Tanh { a } => a.eval(inputs, vals).tanh(),
            AnalogCell::Sqrt { a } => a.eval(inputs, vals).sqrt(),
            AnalogCell::Div { a, b } => a.eval(inputs, vals) / b.eval(inputs, vals),
        }
    }
}

/// A flat SSA analog netlist: the mapped region's dataflow, output tapped at `out`.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalogNetlist {
    /// Plane input arity (the region function's float parameters, in declaration order).
    pub n_in: usize,
    /// SSA tiles: cell `i` defines `Val(i)`.
    pub cells: Vec<AnalogCell>,
    /// The region's output net (the ADC tap).
    pub out: ASrc,
}
