//! The error budget — ε as a COMPILER OUTPUT, never a hand-tuned test tolerance.
//!
//! Every physical error source contributes a bounded math-domain term; the budget composes them
//! through the dataflow with directed-outward rounding (sound linear-sum, the standard
//! first-order perturbation bound plus the exact `Mul` cross term) and publishes the itemized
//! decomposition. `recheck()` re-derives the total from the terms and demands bit-equality —
//! dropping or editing a term is detected, mirroring `PlacementCertificate::recheck`.
//!
//! Phase scope: the IDEALIZED-physical sources — DAC/ADC quantization (half-LSB at the block
//! scale), the multiplier interaction cross-term, and an explicit f64-evaluator-rounding guard.
//! Mismatch / noise / drift terms arrive with the non-ideality model, each itemized and sourced.

use super::cell::{ASrc, AnalogCell, AnalogNetlist};
use super::gain::{GainSchedule, MathBound, RangeCertificate};
use super::nonideal::{leg_sigma, NonIdealityModel, KSIGMA};
use super::place::AnalogPlacement;
use super::target::AnalogTarget;

/// A named physical origin of error. The decomposition is user-facing spec: a refusal or a
/// budget report says exactly which arrow of the architecture costs what.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ErrorSource {
    /// Half-LSB input quantization of plane input `input` at `bits` resolution.
    DacQuant { input: usize, bits: u8 },
    /// Half-LSB output quantization at `bits` resolution.
    AdcQuant { bits: u8 },
    /// Programmed weights snapped to the unit-element grid — the deviation of the realized
    /// configuration from the ideal weights, propagated to the output.
    WeightQuant,
    /// The Σ εa·εb second-order interaction across all `Mul` tiles.
    MulInteraction,
    /// f64 evaluator rounding guard: the behavioral tiers compute in f64, not reals.
    EvalRounding,
    /// Per-leg programmed-gain error (programming accuracy + the Pelgrom distance gradient over
    /// the PLACED operand separation), at the k·σ envelope.
    TileGain,
    /// Per-tile output-referred offset at the k·σ envelope.
    TileOffset,
    /// Per-tile sampled-noise floor (kT/C class) at the k·σ envelope.
    Noise,
    /// Gain drift from the CERTIFIED thermal map's tile-to-operand temperature gradient
    /// (`gain_tempco · |ΔT_cell − ΔT_operand|`), propagated like any gain error.
    ThermalGain,
    /// Output-referred offset drift from a tile's absolute rise (`offset_tempco · ΔT`).
    ThermalOffset,
    /// A comparator whose certified decision box straddles its ±k·σ input-offset band: the
    /// switch may take EITHER branch, so the full branch-hull width is priced — honest
    /// uncertainty, never hidden.
    CompareBand,
    /// The offset residual surviving the chop average — intra-pair drift over one chop
    /// period (the frozen component cancels exactly by construction).
    ChopResidual,
}

/// The CHOPPED physical budget: the schedule runs every frame twice around a `ChopSwap` and
/// averages, so the frozen per-tile offset CANCELS EXACTLY — `TileOffset` is replaced by the
/// modeled `ChopResidual` drift class, and the noise term is √2-averaged (two independent
/// draws per result, bookkept outward). Everything else is the unchopped budget verbatim.
pub fn error_budget_chopped(
    netlist: &AnalogNetlist,
    cert: &RangeCertificate,
    schedule: &GainSchedule,
    target: &AnalogTarget,
    model: &NonIdealityModel,
    placement: &AnalogPlacement,
) -> OutputBudget {
    let unchopped = error_budget_nonideal(netlist, cert, schedule, target, model, placement);
    let bound_of = |src: &ASrc| -> MathBound {
        match src {
            ASrc::In(k) => cert.input_bounds[*k],
            ASrc::Val(v) => cert.per_net[*v].math,
            ASrc::Const(c) => MathBound { lo: *c, hi: *c },
        }
    };
    let realized = AnalogNetlist {
        n_in: schedule.quantized.n_in,
        cells: schedule.realized_math_cells(),
        out: schedule.quantized.out,
    };
    let s_of = |i: usize| f64::powi(2.0, schedule.per_net_scale_log2[i]);
    let n = realized.cells.len();
    let resid_d: Vec<f64> = (0..n)
        .map(|i| (KSIGMA * model.chop_residual_v.0 / s_of(i)).next_up())
        .collect();
    let tr = propagate_injected(&realized, &bound_of, &resid_d);

    let mut terms = Vec::with_capacity(unchopped.terms.len() + 1);
    for t in &unchopped.terms {
        match t.source {
            // The frozen offset cancels exactly; its residual class replaces it below.
            ErrorSource::TileOffset => {}
            // Two independent draws averaged: σ/√2, rounded outward.
            ErrorSource::Noise => terms.push(ErrorTerm {
                eps_math: (t.eps_math / std::f64::consts::SQRT_2).next_up(),
                source: ErrorSource::Noise,
            }),
            _ => terms.push(*t),
        }
    }
    if tr > 0.0 {
        terms.push(ErrorTerm {
            eps_math: tr,
            source: ErrorSource::ChopResidual,
        });
    }
    let eps_math = outward_sum(terms.iter().map(|t| t.eps_math));
    OutputBudget { eps_math, terms }
}

/// One itemized contribution, in the MATH domain at the region output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ErrorTerm {
    pub eps_math: f64,
    pub source: ErrorSource,
}

/// The per-output budget: `eps_math` is the outward-rounded sum of `terms`, pinned by `recheck`.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputBudget {
    pub eps_math: f64,
    pub terms: Vec<ErrorTerm>,
}

impl OutputBudget {
    /// Deterministic replay: the total must be EXACTLY the outward-rounded sum of the itemized
    /// terms, every term finite and non-negative. A dropped or edited term changes the sum and
    /// fails — the tamper probe below proves it.
    pub fn recheck(&self) -> bool {
        if self.terms.is_empty() || !self.eps_math.is_finite() {
            return false;
        }
        if self
            .terms
            .iter()
            .any(|t| !t.eps_math.is_finite() || t.eps_math < 0.0)
        {
            return false;
        }
        outward_sum(self.terms.iter().map(|t| t.eps_math)) == self.eps_math
    }

    /// Tamper probe: the budget with term `idx` removed (total left untouched) — `recheck` on
    /// the result must fail unless the removed term was exactly absorbed by rounding.
    #[must_use]
    pub fn with_term_removed(&self, idx: usize) -> OutputBudget {
        let mut terms = self.terms.clone();
        terms.remove(idx);
        OutputBudget {
            eps_math: self.eps_math,
            terms,
        }
    }
}

pub(crate) fn outward_sum(terms: impl Iterator<Item = f64>) -> f64 {
    let mut acc = 0.0_f64;
    for t in terms {
        // A zero term contributes nothing — bumping it would manufacture a phantom subnormal ε.
        if t > 0.0 {
            acc = (acc + t).next_up();
        }
    }
    acc
}

/// One ULP of `x`'s magnitude scale.
pub(crate) fn ulp_of(x: f64) -> f64 {
    let a = x.abs().max(f64::MIN_POSITIVE);
    a.next_up() - a
}

/// `acc + t`, outward — skipping the bump on a zero term (a phantom subnormal is not an error).
pub(crate) fn bump_add(acc: f64, t: f64) -> f64 {
    if t > 0.0 {
        (acc + t).next_up()
    } else {
        acc
    }
}

/// `a·b`, outward — zero products stay exactly zero.
pub(crate) fn bump_mul(a: f64, b: f64) -> f64 {
    let p = a * b;
    if p > 0.0 {
        p.next_up()
    } else {
        0.0
    }
}

/// One pass of multi-lane ε propagation over the physical (quantized) cells — THE engine every
/// budget composes with. Lane convention, uniform across the DAG, streamed-reduction, and
/// integrator budgets: `[input lanes…, weight-quant, cross]`. Each cell composes its operand
/// lanes (`Sum`/`GainConst` exactly, `Mul` by `|b|·εa + |a|·εb` over certificate bounds),
/// injects its weight-snap delta into the wq lane, and adds its `Σ εa·εb` interaction (from
/// the running totals) into the cross lane — so the second-order term is amplified downstream
/// like any other error. Every accumulation rounds outward via the bump helpers.
///
/// The callers differ ONLY in the input-lane mapping (`input_eps`/`input_total`) and in how
/// many passes they run (one for a DAG; one per frame with carried lanes for the streamed and
/// integrated shapes). A new physical source (e.g. the mixed-signal SoC's surrogate rounding)
/// is added HERE once and every budget inherits it.
pub(crate) struct EpsPass {
    pub cell_eps: Vec<Vec<f64>>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn propagate_eps_pass(
    cells_orig: &[AnalogCell],
    cells_q: &[AnalogCell],
    bound_of: &dyn Fn(&ASrc) -> MathBound,
    lanes: usize,
    wq_lane: usize,
    cross_lane: usize,
    input_eps: &dyn Fn(&ASrc, usize) -> f64,
    input_total: &dyn Fn(&ASrc) -> f64,
) -> EpsPass {
    let mut cell_eps: Vec<Vec<f64>> = Vec::with_capacity(cells_q.len());
    let mut cell_total: Vec<f64> = Vec::with_capacity(cells_q.len());
    let src_eps = |src: &ASrc, cell_eps: &[Vec<f64>], k: usize| -> f64 {
        match src {
            ASrc::Val(v) => cell_eps[*v][k],
            ASrc::Const(_) => 0.0,
            input => input_eps(input, k),
        }
    };
    let src_total = |src: &ASrc, cell_total: &[f64]| -> f64 {
        match src {
            ASrc::Val(v) => cell_total[*v],
            ASrc::Const(_) => 0.0,
            input => input_total(input),
        }
    };
    for (orig, phys) in cells_orig.iter().zip(cells_q) {
        // The snap delta this tile injects (linear tiles only; Mul carries no weights).
        let delta = match (orig, phys) {
            (AnalogCell::Sum { inputs, weights: w }, AnalogCell::Sum { weights: wq, .. }) => {
                outward_sum(
                    inputs
                        .iter()
                        .zip(w.iter().zip(wq))
                        .map(|(x, (a, b))| bump_mul((a - b).abs(), bound_of(x).max_abs())),
                )
            }
            (AnalogCell::GainConst { a, k }, AnalogCell::GainConst { k: kq, .. }) => {
                bump_mul((k - kq).abs(), bound_of(a).max_abs())
            }
            _ => 0.0,
        };
        let mut eps_k = vec![0.0_f64; lanes];
        let total;
        match phys {
            AnalogCell::Sum { inputs, weights } => {
                for k in 0..lanes {
                    let mut acc = 0.0_f64;
                    for (x, w) in inputs.iter().zip(weights) {
                        acc = bump_add(acc, bump_mul(w.abs(), src_eps(x, &cell_eps, k)));
                    }
                    eps_k[k] = acc;
                }
                total = outward_sum(
                    inputs
                        .iter()
                        .zip(weights)
                        .map(|(x, w)| bump_mul(w.abs(), src_total(x, &cell_total))),
                );
            }
            AnalogCell::GainConst { a, k: gain } => {
                for k in 0..lanes {
                    eps_k[k] = bump_mul(gain.abs(), src_eps(a, &cell_eps, k));
                }
                total = bump_mul(gain.abs(), src_total(a, &cell_total));
            }
            AnalogCell::Mul { a, b } => {
                let (amax, bmax) = (bound_of(a).max_abs(), bound_of(b).max_abs());
                for k in 0..lanes {
                    let ea = src_eps(a, &cell_eps, k);
                    let eb = src_eps(b, &cell_eps, k);
                    eps_k[k] = bump_add(bump_mul(bmax, ea), bump_mul(amax, eb));
                }
                let (ta, tb) = (src_total(a, &cell_total), src_total(b, &cell_total));
                eps_k[cross_lane] = bump_add(eps_k[cross_lane], bump_mul(ta, tb));
                total = bump_add(
                    bump_add(bump_mul(bmax, ta), bump_mul(amax, tb)),
                    bump_mul(ta, tb),
                );
            }
            AnalogCell::Select { t, f, .. } => {
                // The output IS one branch: worst branch, lane by lane. The comparator's own
                // ambiguity band is a separate itemized term (`CompareBand`), not smeared here.
                for k in 0..lanes {
                    eps_k[k] = src_eps(t, &cell_eps, k).max(src_eps(f, &cell_eps, k));
                }
                total = src_total(t, &cell_total).max(src_total(f, &cell_total));
            }
            // Translinear unaries: first-order Lipschitz amplification over the CERTIFIED
            // operand box (outward). exp' ≤ e^hi; ln' ≤ 1/lo; tanh' ≤ 1; √' ≤ 1/(2√lo) —
            // the domain gates (lo > 0) were enforced when the box was certified.
            AnalogCell::Exp { a } => {
                let l = bound_of(a).hi.exp().next_up();
                for k in 0..lanes {
                    eps_k[k] = bump_mul(l, src_eps(a, &cell_eps, k));
                }
                total = bump_mul(l, src_total(a, &cell_total));
            }
            AnalogCell::Log { a } => {
                let l = (1.0 / bound_of(a).lo).next_up();
                for k in 0..lanes {
                    eps_k[k] = bump_mul(l, src_eps(a, &cell_eps, k));
                }
                total = bump_mul(l, src_total(a, &cell_total));
            }
            AnalogCell::Tanh { a } => {
                for k in 0..lanes {
                    eps_k[k] = src_eps(a, &cell_eps, k);
                }
                total = src_total(a, &cell_total);
            }
            AnalogCell::Sqrt { a } => {
                let l = (0.5 / bound_of(a).lo.sqrt()).next_up();
                for k in 0..lanes {
                    eps_k[k] = bump_mul(l, src_eps(a, &cell_eps, k));
                }
                total = bump_mul(l, src_total(a, &cell_total));
            }
            AnalogCell::Div { a, b } => {
                // |δ(a/b)| ≤ δa/min|b| + max|a|·δb/min|b|² first-order; the δa·δb/min|b|²
                // second-order proxy rides the cross lane like Mul's.
                let bb = bound_of(b);
                let min_b = bb.lo.abs().min(bb.hi.abs());
                let la = (1.0 / min_b).next_up();
                let lb = (bound_of(a).max_abs() / (min_b * min_b)).next_up();
                for k in 0..lanes {
                    let ea = src_eps(a, &cell_eps, k);
                    let eb = src_eps(b, &cell_eps, k);
                    eps_k[k] = bump_add(bump_mul(la, ea), bump_mul(lb, eb));
                }
                let (ta, tb) = (src_total(a, &cell_total), src_total(b, &cell_total));
                let cross = bump_mul(ta * tb, (1.0 / (min_b * min_b)).next_up());
                eps_k[cross_lane] = bump_add(eps_k[cross_lane], cross);
                total = bump_add(bump_add(bump_mul(la, ta), bump_mul(lb, tb)), cross);
            }
        }
        if delta > 0.0 {
            eps_k[wq_lane] = bump_add(eps_k[wq_lane], delta);
        }
        cell_eps.push(eps_k);
        cell_total.push(if delta > 0.0 {
            bump_add(total, delta)
        } else {
            total
        });
    }
    EpsPass { cell_eps }
}

/// The out-lane readout of a pass for an arbitrary tap.
pub(crate) fn out_lanes(
    pass: &EpsPass,
    out: &ASrc,
    lanes: usize,
    input_eps: &dyn Fn(&ASrc, usize) -> f64,
) -> Vec<f64> {
    (0..lanes)
        .map(|k| match out {
            ASrc::Val(v) => pass.cell_eps[*v][k],
            ASrc::Const(_) => 0.0,
            input => input_eps(input, k),
        })
        .collect()
}

/// The f64 evaluator-rounding guard: two behavioral f64 evaluations of `ops_evaluated` tile
/// equations at the largest certified magnitude are being compared — bound their drift.
pub(crate) fn eval_rounding_guard(cert: &RangeCertificate, ops_evaluated: usize) -> f64 {
    let max_mag = cert
        .per_net
        .iter()
        .map(|n| n.math.max_abs())
        .chain(cert.input_bounds.iter().map(|b| b.max_abs()))
        .fold(f64::MIN_POSITIVE, f64::max);
    (16.0 * (ops_evaluated as f64 + 2.0) * ulp_of(max_mag)).next_up()
}

/// Math-domain half-LSB of a converter at the block scale.
pub(crate) fn half_lsb(cert: &RangeCertificate, schedule: &GainSchedule, bits: u8) -> f64 {
    cert.rail_half_span_v / f64::powi(2.0, i32::from(bits)) / f64::powi(2.0, schedule.scale_log2)
}

/// Compose the budget for a scheduled DAG netlist: one propagation pass with a lane per DAC
/// input, the wq lane, and the cross lane; then the itemized terms.
pub fn error_budget(
    netlist: &AnalogNetlist,
    cert: &RangeCertificate,
    schedule: &GainSchedule,
    target: &AnalogTarget,
) -> OutputBudget {
    let n_in = netlist.n_in;
    let dac_eps = half_lsb(cert, schedule, target.dac_bits);
    let adc_eps = half_lsb(cert, schedule, target.adc_bits);
    let bound_of = |src: &ASrc| -> MathBound {
        match src {
            ASrc::In(k) => cert.input_bounds[*k],
            ASrc::Val(v) => cert.per_net[*v].math,
            ASrc::Const(c) => MathBound { lo: *c, hi: *c },
        }
    };
    let (lanes, wq_lane, cross_lane) = (n_in + 2, n_in, n_in + 1);
    let input_eps = |src: &ASrc, k: usize| -> f64 {
        match src {
            ASrc::In(i) if *i == k && k < n_in => dac_eps,
            _ => 0.0,
        }
    };
    let input_total = |_: &ASrc| dac_eps;
    let realized = schedule.realized_math_cells();
    let pass = propagate_eps_pass(
        &netlist.cells,
        &realized,
        &bound_of,
        lanes,
        wq_lane,
        cross_lane,
        &input_eps,
        &input_total,
    );
    let out = out_lanes(&pass, &netlist.out, lanes, &input_eps);

    let mut terms: Vec<ErrorTerm> = out[..n_in]
        .iter()
        .enumerate()
        .filter(|(_, e)| **e > 0.0)
        .map(|(k, e)| ErrorTerm {
            eps_math: *e,
            source: ErrorSource::DacQuant {
                input: k,
                bits: target.dac_bits,
            },
        })
        .collect();
    terms.push(ErrorTerm {
        eps_math: adc_eps,
        source: ErrorSource::AdcQuant {
            bits: target.adc_bits,
        },
    });
    if out[wq_lane] > 0.0 {
        terms.push(ErrorTerm {
            eps_math: out[wq_lane],
            source: ErrorSource::WeightQuant,
        });
    }
    if out[cross_lane] > 0.0 {
        terms.push(ErrorTerm {
            eps_math: out[cross_lane],
            source: ErrorSource::MulInteraction,
        });
    }
    terms.push(ErrorTerm {
        eps_math: eval_rounding_guard(cert, netlist.cells.len()),
        source: ErrorSource::EvalRounding,
    });

    let eps_math = outward_sum(terms.iter().map(|t| t.eps_math));
    OutputBudget { eps_math, terms }
}

/// Crate-internal alias for [`propagate_injected`] (the calibrated budget lives one module over).
pub(crate) fn propagate_injected_pub(
    netlist_q: &AnalogNetlist,
    bound_of: &dyn Fn(&ASrc) -> MathBound,
    deltas: &[f64],
) -> f64 {
    propagate_injected(netlist_q, bound_of, deltas)
}

/// Propagate a per-cell INJECTED error vector to the output: each cell adds its own `deltas[i]`
/// at its output net, and downstream cells amplify it exactly like any other error lane.
/// Shared by the physical (non-ideal) terms.
fn propagate_injected(
    netlist_q: &AnalogNetlist,
    bound_of: &dyn Fn(&ASrc) -> MathBound,
    deltas: &[f64],
) -> f64 {
    let mut cell_eps: Vec<f64> = Vec::with_capacity(netlist_q.cells.len());
    let src_eps = |s: &ASrc, cell_eps: &[f64]| -> f64 {
        match s {
            ASrc::Val(v) => cell_eps[*v],
            _ => 0.0,
        }
    };
    for (i, cell) in netlist_q.cells.iter().enumerate() {
        let carried = match cell {
            AnalogCell::Sum { inputs, weights } => outward_sum(
                inputs
                    .iter()
                    .zip(weights)
                    .map(|(x, w)| bump_mul(w.abs(), src_eps(x, &cell_eps))),
            ),
            AnalogCell::GainConst { a, k } => bump_mul(k.abs(), src_eps(a, &cell_eps)),
            AnalogCell::Select { t, f, .. } => src_eps(t, &cell_eps).max(src_eps(f, &cell_eps)),
            AnalogCell::Exp { a } => {
                bump_mul(bound_of(a).hi.exp().next_up(), src_eps(a, &cell_eps))
            }
            AnalogCell::Log { a } => {
                bump_mul((1.0 / bound_of(a).lo).next_up(), src_eps(a, &cell_eps))
            }
            AnalogCell::Tanh { a } => src_eps(a, &cell_eps),
            AnalogCell::Sqrt { a } => bump_mul(
                (0.5 / bound_of(a).lo.sqrt()).next_up(),
                src_eps(a, &cell_eps),
            ),
            AnalogCell::Div { a, b } => {
                let bb = bound_of(b);
                let min_b = bb.lo.abs().min(bb.hi.abs());
                bump_add(
                    bump_mul((1.0 / min_b).next_up(), src_eps(a, &cell_eps)),
                    bump_mul(
                        (bound_of(a).max_abs() / (min_b * min_b)).next_up(),
                        src_eps(b, &cell_eps),
                    ),
                )
            }
            AnalogCell::Mul { a, b } => {
                let (amax, bmax) = (bound_of(a).max_abs(), bound_of(b).max_abs());
                bump_add(
                    bump_mul(bmax, src_eps(a, &cell_eps)),
                    bump_mul(amax, src_eps(b, &cell_eps)),
                )
            }
        };
        cell_eps.push(if deltas[i] > 0.0 {
            bump_add(carried, deltas[i])
        } else {
            carried
        });
    }
    match netlist_q.out {
        ASrc::Val(v) => cell_eps[v],
        _ => 0.0,
    }
}

/// The PHYSICAL budget: the converter/quantization budget plus the non-ideality terms —
/// per-leg gain error over the PLACED operand separations, per-tile offset, and the noise
/// floor, each at the k·σ envelope and propagated through the dataflow. This is the ε the
/// Monte-Carlo battery and (later) SPICE must live inside.
pub fn error_budget_nonideal(
    netlist: &AnalogNetlist,
    cert: &RangeCertificate,
    schedule: &GainSchedule,
    target: &AnalogTarget,
    model: &NonIdealityModel,
    placement: &AnalogPlacement,
) -> OutputBudget {
    let q = schedule.realized_math_cells();
    let sigma_of = move |i: usize, l: usize| leg_sigma(i, l, &q, placement, model);
    nonideal_budget_with(netlist, cert, schedule, target, model, &sigma_of)
}

/// The shared non-ideality composer, parameterized over the per-leg sigma law — the 2D path
/// feeds it `leg_sigma` (SameTier distances); the stacked path feeds the full separation law
/// including the categorical cross-tier ceiling. ONE composition, every geometry.
pub(crate) fn nonideal_budget_with(
    netlist: &AnalogNetlist,
    cert: &RangeCertificate,
    schedule: &GainSchedule,
    target: &AnalogTarget,
    model: &NonIdealityModel,
    sigma_of: &dyn Fn(usize, usize) -> f64,
) -> OutputBudget {
    let base = error_budget(netlist, cert, schedule, target);
    let realized = AnalogNetlist {
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

    // Per-cell injected deltas, math domain.
    let mut gain_d = Vec::with_capacity(q.cells.len());
    let mut off_d = Vec::with_capacity(q.cells.len());
    let mut noise_d = Vec::with_capacity(q.cells.len());
    for (i, cell) in q.cells.iter().enumerate() {
        let g = match cell {
            AnalogCell::Sum { inputs, weights } => {
                outward_sum(inputs.iter().zip(weights).enumerate().map(|(l, (x, w))| {
                    bump_mul(KSIGMA * sigma_of(i, l), w.abs() * bound_of(x).max_abs())
                }))
            }
            AnalogCell::GainConst { a, k } => {
                bump_mul(KSIGMA * sigma_of(i, 0), k.abs() * bound_of(a).max_abs())
            }
            AnalogCell::Mul { a, b } => bump_mul(
                KSIGMA * sigma_of(i, 0),
                bound_of(a).max_abs() * bound_of(b).max_abs(),
            ),
            AnalogCell::Select { .. } => 0.0,
            // Translinear bias-gain class: multiplicative on the output law — the injected
            // delta is k·σ times the cell's own certified output magnitude.
            AnalogCell::Exp { .. }
            | AnalogCell::Log { .. }
            | AnalogCell::Tanh { .. }
            | AnalogCell::Sqrt { .. }
            | AnalogCell::Div { .. } => {
                bump_mul(KSIGMA * sigma_of(i, 0), cert.per_net[i].math.max_abs())
            }
        };
        gain_d.push(g);
        off_d.push((KSIGMA * model.offset_sigma_v.0 / s_of(i)).next_up());
        noise_d.push((KSIGMA * model.noise_floor_v.0 / s_of(i)).next_up());
    }

    // CompareBand: a Select whose decision box cannot clear the comparator's offset band may
    // take either branch — inject the branch-hull width at that cell, amplified downstream.
    let mut band_d = vec![0.0_f64; q.cells.len()];
    let mut any_band = false;
    for (i, cell) in q.cells.iter().enumerate() {
        if let AnalogCell::Select { c, t, f } = cell {
            let s_c = match c {
                ASrc::Val(v) => f64::powi(2.0, schedule.per_net_scale_log2[*v]),
                _ => f64::powi(2.0, schedule.scale_log2),
            };
            let band_math = KSIGMA * model.offset_sigma_v.0 / s_c;
            let cb = bound_of(c);
            if cb.lo < band_math && cb.hi > -band_math {
                let (tb, fb) = (bound_of(t), bound_of(f));
                let hull_w = (tb.hi.max(fb.hi) - tb.lo.min(fb.lo)).next_up();
                band_d[i] = hull_w;
                any_band = true;
            }
        }
    }
    let mut terms = base.terms;
    if any_band {
        let tb = propagate_injected(q, &bound_of, &band_d);
        if tb > 0.0 {
            terms.push(ErrorTerm {
                eps_math: tb,
                source: ErrorSource::CompareBand,
            });
        }
    }
    let tg = propagate_injected(q, &bound_of, &gain_d);
    if tg > 0.0 {
        terms.push(ErrorTerm {
            eps_math: tg,
            source: ErrorSource::TileGain,
        });
    }
    let to = propagate_injected(q, &bound_of, &off_d);
    if to > 0.0 {
        terms.push(ErrorTerm {
            eps_math: to,
            source: ErrorSource::TileOffset,
        });
    }
    let tn = propagate_injected(q, &bound_of, &noise_d);
    if tn > 0.0 {
        terms.push(ErrorTerm {
            eps_math: tn,
            source: ErrorSource::Noise,
        });
    }
    let eps_math = outward_sum(terms.iter().map(|t| t.eps_math));
    OutputBudget { eps_math, terms }
}
