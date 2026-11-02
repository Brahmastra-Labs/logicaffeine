//! Tier-1 behavioral evaluation of an analog netlist.
//!
//! `eval_analog_dc` is the analog analogue of the digital fabric's `eval_grid`: the IDEAL
//! reference semantics. The plane settles to a DC operating point per control tick; with no
//! non-ideality and unit scale, that point is exactly the SSA dataflow evaluated in f64 — the
//! same operations in the same order as the VM, so the ideal tier is compared with
//! `logos_approx_eq` and is bit-exact in practice. Physics (scales, quantization, mismatch,
//! noise) enters through later layers, each with its own budgeted comparison — never here.

use super::cell::AnalogNetlist;
use super::gain::GainSchedule;
use super::target::AnalogTarget;

/// THE converter model — the single definition of the block scale and both converter grids
/// every physical tier shares (the behavioral evaluators, the instance evaluator, the streamed
/// and integrated steppers — and the mixed-signal SoC's fixed-point surrogate, which must
/// mirror these semantics bit-for-bit).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConverterGrid {
    /// The block scale, volts per math unit (`2^scale_log2`).
    pub s: f64,
    /// DAC step in volts over the usable differential span.
    pub dac_lsb: f64,
    /// ADC step in volts.
    pub adc_lsb: f64,
}

impl ConverterGrid {
    pub fn of(schedule: &GainSchedule, target: &AnalogTarget) -> ConverterGrid {
        let half_span = target.half_span_v();
        ConverterGrid {
            s: f64::powi(2.0, schedule.scale_log2),
            dac_lsb: 2.0 * half_span / f64::powi(2.0, i32::from(target.dac_bits)),
            adc_lsb: 2.0 * half_span / f64::powi(2.0, i32::from(target.adc_bits)),
        }
    }

    /// A math-domain input as the DAC actually drives it: rounded to the DAC grid at the block
    /// scale, returned in the MATH domain (round-half-away-from-zero, deterministic).
    pub fn dac_q(&self, x_math: f64) -> f64 {
        ((x_math * self.s) / self.dac_lsb).round() * self.dac_lsb / self.s
    }

    /// A math-domain result as the ADC reads it back: rounded to the ADC grid, math domain.
    pub fn adc_q(&self, y_math: f64) -> f64 {
        ((y_math * self.s) / self.adc_lsb).round() * self.adc_lsb / self.s
    }
}

/// Ideal DC operating point: evaluate the SSA tiles in order and read the output tap.
pub fn eval_analog_dc(netlist: &AnalogNetlist, inputs: &[f64]) -> f64 {
    assert_eq!(
        inputs.len(),
        netlist.n_in,
        "input arity must match the netlist"
    );
    let mut vals = Vec::with_capacity(netlist.cells.len());
    for cell in &netlist.cells {
        let v = cell.eval(inputs, &vals);
        vals.push(v);
    }
    netlist.out.eval(inputs, &vals)
}

/// DC operating point through the CONVERTER path: each input is driven by the DAC (rounded to
/// its LSB at the block scale), the plane settles in the VOLT domain (the `Mul` tile divides by
/// its programmed `V_ref`, returning products to the block scale), and the output is read
/// through the ADC (rounded to its LSB) and descaled exactly (power-of-two shift).
///
/// Rounding is round-half-away-from-zero (`f64::round`) — deterministic, and the half-LSB error
/// model in [`super::error_budget`](mod@super::error_budget) covers it. This is the first physically-shaped tier: its
/// deviation from [`eval_analog_dc`] must stay inside the compiler's own `OutputBudget`.
///
/// [`super::error_budget`]: mod@crate::codegen_ana::error_budget
pub fn eval_analog_quantized(
    schedule: &GainSchedule,
    target: &AnalogTarget,
    inputs: &[f64],
) -> f64 {
    // The physical tier runs what silicon runs — the weight-quantized twin — through the ONE
    // converter model: DAC-grid the inputs, settle (the math-domain walk; the volt-domain plane
    // with the Mul's V_ref normalization is exactly this times the block scale), ADC-grid out.
    let grid = ConverterGrid::of(schedule, target);
    let vin: Vec<f64> = inputs.iter().map(|x| grid.dac_q(*x)).collect();
    grid.adc_q(eval_analog_dc(&schedule.quantized, &vin))
}
