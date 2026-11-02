//! Gain scaling + the range-safety certificate — the compiler side of the analog trust story.
//!
//! Values on nets are MATH-domain quantities; voltage is `2^e · math` under a single power-of-two
//! block scale `e` chosen so the largest net magnitude fits the usable rail half-span
//! (`headroom · (rail_hi − rail_lo) / 2`, differential-symmetric around mid-rail). Powers of two
//! keep every certificate multiplication exact in f64 and make the ADC descale a bit shift.
//! With a common scale, `Sum` weights ride unchanged, a `GainConst` is the same ratio in both
//! domains (gated against the target's realizable range), and the four-quadrant `Mul` divides by
//! a programmed `V_ref = 2^e` so its output returns to the block scale — the standard
//! translinear normalization.
//!
//! The produced [`RangeCertificate`] carries per-net math and volt bounds from directed-outward
//! interval propagation over the DECLARED input ranges. It is judged by the INDEPENDENT replay
//! checker in `logicaffeine_proof::interval_replay` (this producer converts to the checker's
//! neutral vocabulary via [`to_range_claim`]) — the checker trusts nothing computed here.

use super::cell::{ASrc, AnalogCell, AnalogNetlist};
use super::target::{AnaUnplaceable, AnalogTarget};
use super::{AnaLowerError, AssumedRange};

/// A directed-outward f64 interval — the same arithmetic the independent checker replays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MathBound {
    pub lo: f64,
    pub hi: f64,
}

impl MathBound {
    fn outward(self) -> MathBound {
        MathBound {
            lo: self.lo.next_down(),
            hi: self.hi.next_up(),
        }
    }
    fn add(self, o: MathBound) -> MathBound {
        MathBound {
            lo: self.lo + o.lo,
            hi: self.hi + o.hi,
        }
        .outward()
    }
    fn scale(self, w: f64) -> MathBound {
        let (a, b) = (w * self.lo, w * self.hi);
        MathBound {
            lo: a.min(b),
            hi: a.max(b),
        }
        .outward()
    }
    fn mul(self, o: MathBound) -> MathBound {
        let ps = [
            self.lo * o.lo,
            self.lo * o.hi,
            self.hi * o.lo,
            self.hi * o.hi,
        ];
        MathBound {
            lo: ps.iter().copied().fold(f64::INFINITY, f64::min),
            hi: ps.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        }
        .outward()
    }
    pub fn max_abs(self) -> f64 {
        self.lo.abs().max(self.hi.abs())
    }
}

/// The per-program gain schedule: one block scale for every net, the exact ADC descale, the
/// REALIZABLE multiplier reference, and the weight-quantized twin of the netlist (weights
/// snapped to the target's unit elements — what the silicon actually programs).
#[derive(Clone, Debug, PartialEq)]
pub struct GainSchedule {
    /// The block scale exponent: `volt = 2^scale_log2 · math` on every net.
    pub scale_log2: i32,
    /// The output descale exponent (`math = 2^out_descale_log2 · volt`); always `-scale_log2`.
    pub out_descale_log2: i32,
    /// The multiplier normalization reference in volts. When the graph carries a `Mul`, this is
    /// a member of the target's `mul_vref_options_v` equal to the block scale — chosen FROM the
    /// realizable set, never approximated into it.
    pub mul_vref_v: f64,
    /// The netlist with every programmed weight snapped to the target's unit-element grid —
    /// the configuration the fabric actually realizes (PHYSICAL weights, which under a
    /// per-net schedule include the exact power-of-two rescale between operand and cell
    /// scales). The snap error is a budget term (`ErrorSource::WeightQuant`), never silently
    /// absorbed.
    pub quantized: AnalogNetlist,
    /// Per-cell scale exponents (`volt_i = 2^{per_net_scale_log2[i]} · math_i`). Uniform
    /// schedules carry `scale_log2` everywhere; a per-net schedule (O4) lets internal nets
    /// diverge while inputs and the output stay on `scale_log2`, so the converter grid is
    /// untouched. The independent range checker judges per-cell scales natively.
    pub per_net_scale_log2: Vec<i32>,
}

impl GainSchedule {
    /// The REALIZED math dataflow: each physical weight mapped back to the math domain by the
    /// exact power-of-two rescale (`w_math_realized = w_phys · 2^{e_operand − e_cell}`).
    /// Under a uniform schedule this is the quantized netlist verbatim. Certificates and
    /// budgets judge THIS — the dataflow the silicon actually computes.
    pub fn realized_math_cells(&self) -> Vec<AnalogCell> {
        let e_io = self.scale_log2;
        let e = &self.per_net_scale_log2;
        let src_e = |s: &ASrc| -> i32 {
            match s {
                ASrc::Val(v) => e[*v],
                _ => e_io,
            }
        };
        self.quantized
            .cells
            .iter()
            .enumerate()
            .map(|(i, cell)| match cell {
                AnalogCell::Sum { inputs, weights } => AnalogCell::Sum {
                    inputs: inputs.clone(),
                    weights: inputs
                        .iter()
                        .zip(weights)
                        .map(|(x, w)| w * f64::powi(2.0, src_e(x) - e[i]))
                        .collect(),
                },
                AnalogCell::GainConst { a, k } => AnalogCell::GainConst {
                    a: *a,
                    k: k * f64::powi(2.0, src_e(a) - e[i]),
                },
                AnalogCell::Mul { a, b } => AnalogCell::Mul { a: *a, b: *b },
                AnalogCell::Select { c, t, f } => AnalogCell::Select {
                    c: *c,
                    t: *t,
                    f: *f,
                },
                AnalogCell::Exp { a } => AnalogCell::Exp { a: *a },
                AnalogCell::Log { a } => AnalogCell::Log { a: *a },
                AnalogCell::Tanh { a } => AnalogCell::Tanh { a: *a },
                AnalogCell::Sqrt { a } => AnalogCell::Sqrt { a: *a },
                AnalogCell::Div { a, b } => AnalogCell::Div { a: *a, b: *b },
            })
            .collect()
    }
}

/// One net's certified bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct NetBound {
    pub math: MathBound,
    pub volt_lo: f64,
    pub volt_hi: f64,
}

/// The range-safety certificate: per-net bounds + the physical envelope they were proven against.
/// Independently judged by `logicaffeine_proof::interval_replay::check_range_claim` via
/// [`to_range_claim`]; nothing here is trusted by the checker.
#[derive(Clone, Debug, PartialEq)]
pub struct RangeCertificate {
    /// Declared math-domain input bounds, in plane-input order.
    pub input_bounds: Vec<MathBound>,
    /// Per-cell bounds (SSA order).
    pub per_net: Vec<NetBound>,
    /// The usable half-span in volts the bounds were proven inside.
    pub rail_half_span_v: f64,
    /// The noise floor the starvation check used (`0.0` on an ideal target).
    pub noise_floor_v: f64,
    /// The demanded full-scale/noise ratio.
    pub min_full_scale_over_noise: f64,
}

/// Run the gain-scaling pass: propagate declared ranges, choose the block scale, gate realizable
/// ratios, and emit the schedule + certificate. `ranges` is one `AssumedRange` per plane input,
/// in input order (the caller resolves parameter names to input indices).
pub fn scale_pass(
    netlist: &AnalogNetlist,
    ranges: &[AssumedRange],
    target: &AnalogTarget,
) -> Result<(GainSchedule, RangeCertificate), AnaLowerError> {
    assert_eq!(
        ranges.len(),
        netlist.n_in,
        "one declared range per plane input"
    );
    let input_bounds: Vec<MathBound> = ranges
        .iter()
        .map(|r| MathBound { lo: r.lo, hi: r.hi })
        .collect();
    for b in &input_bounds {
        if !(b.lo.is_finite() && b.hi.is_finite() && b.lo <= b.hi) {
            return Err(AnaLowerError::UnboundedInput(format!(
                "declared range [{}, {}] is not a finite interval",
                b.lo, b.hi
            )));
        }
    }

    // Weight quantization FIRST: the certificate must bound the netlist silicon actually runs —
    // the unit-element-snapped twin — not the ideal weights. (The ideal ≡VM tier keeps the
    // original netlist; the snap deviation is the WeightQuant budget term.)
    let quantized = quantize_weights(netlist, target)?;

    // Forward outward-interval propagation in the math domain, over the QUANTIZED cells.
    let bounds = propagate_bounds(&quantized.cells, &input_bounds)?;

    // The block scale: the largest magnitude anywhere (inputs included — the DAC drives them at
    // the same scale) must fit the usable half-span, AND the scale must be one the fabric's
    // references support (`scale_log2_range`; a `Mul`-bearing graph must additionally land on a
    // realizable multiplier reference — rail safety does not imply Vref realizability).
    let half_span = target.half_span_v();
    let has_mul = netlist
        .cells
        .iter()
        .any(|c| matches!(c, AnalogCell::Mul { .. }));
    let has_translinear = netlist.cells.iter().any(|c| {
        matches!(
            c,
            AnalogCell::Exp { .. }
                | AnalogCell::Log { .. }
                | AnalogCell::Tanh { .. }
                | AnalogCell::Sqrt { .. }
                | AnalogCell::Div { .. }
        )
    });
    let all_bounds: Vec<MathBound> = input_bounds.iter().chain(bounds.iter()).copied().collect();
    let scale_log2 = match select_scale_pinned(&all_bounds, has_mul, target, has_translinear) {
        Ok(e) => e,
        // The block-scale limiter: one net's magnitude was dragging every net's scale. The
        // per-net pass (O4) decouples them; only if IT also refuses does the program refuse.
        Err(uniform_refusal) => {
            if has_translinear {
                return Err(uniform_refusal);
            }
            return per_net_scale_pass(&input_bounds, &quantized, &bounds, target)
                .map_err(|_| uniform_refusal);
        }
    };
    let s = f64::powi(2.0, scale_log2);

    // Certificate: volt bounds per net, then the starvation check against the noise floor. The
    // rail refusal below is unreachable under a free block scale (the retry above guarantees the
    // fit); it guards the future fixed-scale / real-noise-floor targets.
    let noise_floor_v = 0.0; // the ideal-physics floor; the non-ideality phase supplies a real one
    let min_ratio = 1.0;
    let mut per_net = Vec::with_capacity(bounds.len());
    for b in &bounds {
        let (volt_lo, volt_hi) = ((b.lo * s).next_down(), (b.hi * s).next_up());
        if volt_lo < -half_span || volt_hi > half_span {
            let needed_db = 20.0 * (b.max_abs() * s / half_span).log10();
            return Err(AnaLowerError::Unplaceable(
                AnaUnplaceable::DynamicRangeExceeded {
                    net: per_net.len(),
                    needed_db: needed_db.max(0.0),
                    available_db: 0.0,
                    target: target.name,
                },
            ));
        }
        per_net.push(NetBound {
            math: *b,
            volt_lo,
            volt_hi,
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
        rail_half_span_v: half_span,
        noise_floor_v,
        min_full_scale_over_noise: min_ratio,
    };
    Ok((schedule, cert))
}

/// One step of directed-outward interval propagation over analog cells — shared by the DAG
/// schedule (`scale_pass`) and the streamed-reduction iteration.
pub(crate) fn propagate_bounds(
    cells: &[AnalogCell],
    input_bounds: &[MathBound],
) -> Result<Vec<MathBound>, AnaLowerError> {
    let src_bound = |s: &ASrc, vals: &[MathBound]| -> MathBound {
        match s {
            ASrc::In(k) => input_bounds[*k],
            ASrc::Val(v) => vals[*v],
            ASrc::Const(c) => MathBound { lo: *c, hi: *c },
        }
    };
    let mut bounds: Vec<MathBound> = Vec::with_capacity(cells.len());
    for cell in cells {
        let b = match cell {
            AnalogCell::Sum { inputs, weights } => {
                let mut acc = MathBound { lo: 0.0, hi: 0.0 };
                for (x, w) in inputs.iter().zip(weights) {
                    acc = acc.add(src_bound(x, &bounds).scale(*w));
                }
                acc
            }
            AnalogCell::Mul { a, b } => src_bound(a, &bounds).mul(src_bound(b, &bounds)),
            AnalogCell::GainConst { a, k } => src_bound(a, &bounds).scale(*k),
            AnalogCell::Select { c, t, f } => {
                // Decided boxes take their branch; a straddling condition takes the hull —
                // the comparator's ambiguity is priced by the budget, not hidden here.
                let cb = src_bound(c, &bounds);
                let (tb, fb) = (src_bound(t, &bounds), src_bound(f, &bounds));
                if cb.hi < 0.0 {
                    tb
                } else if cb.lo >= 0.0 {
                    fb
                } else {
                    MathBound {
                        lo: tb.lo.min(fb.lo),
                        hi: tb.hi.max(fb.hi),
                    }
                    .outward()
                }
            }
            // The translinear family: monotone image boxes, outward; domains fail closed.
            AnalogCell::Exp { a } => {
                let ab = src_bound(a, &bounds);
                MathBound {
                    lo: ab.lo.exp(),
                    hi: ab.hi.exp(),
                }
                .outward()
            }
            AnalogCell::Log { a } => {
                let ab = src_bound(a, &bounds);
                if ab.lo <= 0.0 {
                    return Err(AnaLowerError::UnloweredOp(format!(
                        "log domain: operand box [{}, {}] touches the nonpositive axis",
                        ab.lo, ab.hi
                    )));
                }
                MathBound {
                    lo: ab.lo.ln(),
                    hi: ab.hi.ln(),
                }
                .outward()
            }
            AnalogCell::Tanh { a } => {
                let ab = src_bound(a, &bounds);
                MathBound {
                    lo: ab.lo.tanh(),
                    hi: ab.hi.tanh(),
                }
                .outward()
            }
            AnalogCell::Sqrt { a } => {
                let ab = src_bound(a, &bounds);
                if ab.lo <= 0.0 {
                    return Err(AnaLowerError::UnloweredOp(format!(
                        "sqrt domain: operand box [{}, {}] touches zero — the error slope \
                         diverges (the budget's Lipschitz factor needs lo > 0)",
                        ab.lo, ab.hi
                    )));
                }
                MathBound {
                    lo: ab.lo.sqrt(),
                    hi: ab.hi.sqrt(),
                }
                .outward()
            }
            AnalogCell::Div { a, b } => {
                let (ab, bb) = (src_bound(a, &bounds), src_bound(b, &bounds));
                if bb.lo <= 0.0 && bb.hi >= 0.0 {
                    return Err(AnaLowerError::Unplaceable(
                        AnaUnplaceable::DivisorRangeSpansZero {
                            net: bounds.len(),
                            lo: bb.lo,
                            hi: bb.hi,
                        },
                    ));
                }
                let ps = [ab.lo / bb.lo, ab.lo / bb.hi, ab.hi / bb.lo, ab.hi / bb.hi];
                MathBound {
                    lo: ps.iter().copied().fold(f64::INFINITY, f64::min),
                    hi: ps.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                }
                .outward()
            }
        };
        if !(b.lo.is_finite() && b.hi.is_finite()) {
            return Err(AnaLowerError::UnloweredOp(
                "range propagation overflowed — the declared ranges do not bound this dataflow"
                    .to_string(),
            ));
        }
        bounds.push(b);
    }
    Ok(bounds)
}

/// The bound of an operand given already-propagated cell bounds.
pub(crate) fn src_bound_of(
    s: &ASrc,
    input_bounds: &[MathBound],
    bounds: &[MathBound],
) -> MathBound {
    match s {
        ASrc::In(k) => input_bounds[*k],
        ASrc::Val(v) => bounds[*v],
        ASrc::Const(c) => MathBound { lo: *c, hi: *c },
    }
}

/// Choose the block scale for a set of bounds: the largest realizable power of two that fits
/// every OUTWARD-rounded endpoint inside the half-span — from the discrete Vref options when a
/// `Mul` is present (the scale IS the reference), else from the scale window. Shared by the DAG
/// and reduction passes.
pub(crate) fn select_scale_pinned(
    all_bounds: &[MathBound],
    has_mul: bool,
    target: &AnalogTarget,
    pin_unit_scale: bool,
) -> Result<i32, AnaLowerError> {
    if pin_unit_scale {
        let half_span = target.half_span_v();
        let (lo_e, hi_e) = target.scale_log2_range;
        let fits0 = all_bounds
            .iter()
            .all(|b| b.lo.next_down() >= -half_span && b.hi.next_up() <= half_span);
        if lo_e <= 0 && 0 <= hi_e && fits0 && !has_mul {
            return Ok(0);
        }
        if has_mul {
            // A Mul's reference must equal the block scale; translinear pins it to 1 V —
            // realizable only if the option set carries it.
            if target.mul_vref_options_v.iter().any(|v| v.0 == 1.0) && fits0 {
                return Ok(0);
            }
            return Err(AnaLowerError::Unplaceable(
                AnaUnplaceable::MulReferenceUnrealizable {
                    needed_v: 1.0,
                    options_v: target.mul_vref_options_v.iter().map(|v| v.0).collect(),
                    target: target.name,
                },
            ));
        }
        return Err(AnaLowerError::Unplaceable(
            AnaUnplaceable::DynamicRangeExceeded {
                net: 0,
                needed_db: 0.0,
                available_db: 0.0,
                target: target.name,
            },
        ));
    }
    select_scale(all_bounds, has_mul, target)
}

pub(crate) fn select_scale(
    all_bounds: &[MathBound],
    has_mul: bool,
    target: &AnalogTarget,
) -> Result<i32, AnaLowerError> {
    let half_span = target.half_span_v();
    let global_max = all_bounds
        .iter()
        .map(|b| b.max_abs())
        .fold(0.0_f64, f64::max);
    let fits = |e: i32| -> bool {
        let s = f64::powi(2.0, e);
        all_bounds
            .iter()
            .all(|b| (b.lo * s).next_down() >= -half_span && (b.hi * s).next_up() <= half_span)
    };
    let (lo_e, hi_e) = target.scale_log2_range;
    if has_mul {
        let mut opts: Vec<f64> = target.mul_vref_options_v.iter().map(|v| v.0).collect();
        opts.sort_by(|a, b| b.partial_cmp(a).unwrap());
        for v in &opts {
            let e = v.log2();
            if e.fract() != 0.0 {
                continue; // scales are powers of two; a non-p2 reference serves other schedules
            }
            let e = e as i32;
            if e >= lo_e && e <= hi_e && (global_max == 0.0 || fits(e)) {
                return Ok(e);
            }
        }
        let needed = if global_max == 0.0 {
            half_span
        } else {
            half_span / global_max
        };
        Err(AnaLowerError::Unplaceable(
            AnaUnplaceable::MulReferenceUnrealizable {
                needed_v: needed,
                options_v: target.mul_vref_options_v.iter().map(|v| v.0).collect(),
                target: target.name,
            },
        ))
    } else {
        let mut e = if global_max == 0.0 {
            0
        } else {
            ((half_span / global_max).log2().floor() as i32).clamp(lo_e, hi_e)
        }
        .min(hi_e);
        while global_max > 0.0 && !fits(e) && e > lo_e {
            e -= 1;
        }
        if global_max > 0.0 && !fits(e) {
            return Err(AnaLowerError::Unplaceable(
                AnaUnplaceable::DynamicRangeExceeded {
                    net: 0,
                    needed_db: 20.0
                        * (global_max * f64::powi(2.0, lo_e) / half_span)
                            .log10()
                            .max(0.0),
                    available_db: 0.0,
                    target: target.name,
                },
            ));
        }
        Ok(e)
    }
}

/// Snap every programmed weight to the target's sign+magnitude unit-element grid and gate each
/// nonzero snapped magnitude against the realizable range. Fan-in is checked here too (belt to
/// the crossbar treeify): a `Sum` wider than the physical summing node refuses typed.
pub(crate) fn quantize_weights(
    netlist: &AnalogNetlist,
    target: &AnalogTarget,
) -> Result<AnalogNetlist, AnaLowerError> {
    let lsb = target.weight_lsb();
    let snap = |w: f64| -> f64 { (w / lsb).round() * lsb };
    let gate = |wq: f64| -> Result<(), AnaLowerError> {
        if wq != 0.0 {
            target
                .require_gain_ratio(wq.abs())
                .map_err(AnaLowerError::Unplaceable)?;
        }
        Ok(())
    };
    let mut cells = Vec::with_capacity(netlist.cells.len());
    for cell in &netlist.cells {
        cells.push(match cell {
            AnalogCell::Sum { inputs, weights } => {
                if inputs.len() > target.sum_fanin {
                    return Err(AnaLowerError::Unplaceable(
                        AnaUnplaceable::SumFaninExceeded {
                            needed: inputs.len(),
                            max: target.sum_fanin,
                            target: target.name,
                        },
                    ));
                }
                let wq: Vec<f64> = weights.iter().map(|w| snap(*w)).collect();
                for w in &wq {
                    gate(*w)?;
                }
                AnalogCell::Sum {
                    inputs: inputs.clone(),
                    weights: wq,
                }
            }
            AnalogCell::GainConst { a, k } => {
                let kq = snap(*k);
                gate(kq)?;
                AnalogCell::GainConst { a: *a, k: kq }
            }
            AnalogCell::Mul { a, b } => AnalogCell::Mul { a: *a, b: *b },
            AnalogCell::Select { c, t, f } => AnalogCell::Select {
                c: *c,
                t: *t,
                f: *f,
            },
            AnalogCell::Exp { a } => AnalogCell::Exp { a: *a },
            AnalogCell::Log { a } => AnalogCell::Log { a: *a },
            AnalogCell::Tanh { a } => AnalogCell::Tanh { a: *a },
            AnalogCell::Sqrt { a } => AnalogCell::Sqrt { a: *a },
            AnalogCell::Div { a, b } => AnalogCell::Div { a: *a, b: *b },
        });
    }
    Ok(AnalogNetlist {
        n_in: netlist.n_in,
        cells,
        out: netlist.out,
    })
}

/// The per-net scale pass (O4): decouple net scales so one large intermediate no longer
/// drags every net (and the multiplier reference) down with it.
///
/// V1 constraints, each a soundness line rather than a limitation apology:
/// - inputs and the OUTPUT cell stay on one io scale (the converter grid is untouched);
/// - every net touching a `Mul` (operands and output) shares one reference scale drawn from
///   the target's realizable Vref set — the translinear normalization is exact only when all
///   four ports agree;
/// - a `Const`-fed cell stays on the io scale (constants are driven at the io reference);
/// - every physical weight (math weight × exact power-of-two rescale) must still land on the
///   realizable unit-element range after snapping — refused otherwise.
fn per_net_scale_pass(
    input_bounds: &[MathBound],
    quantized_math: &AnalogNetlist,
    bounds: &[MathBound],
    target: &AnalogTarget,
) -> Result<(GainSchedule, RangeCertificate), AnaLowerError> {
    let half_span = target.half_span_v();
    let (lo_e, hi_e) = target.scale_log2_range;
    let n = quantized_math.cells.len();
    let fits_one = |b: &MathBound, e: i32| -> bool {
        let s = f64::powi(2.0, e);
        (b.lo * s).next_down() >= -half_span && (b.hi * s).next_up() <= half_span
    };
    let best_for = |b: &MathBound| -> Option<i32> { (lo_e..=hi_e).rev().find(|e| fits_one(b, *e)) };

    // The io scale serves the inputs and the output cell.
    let out_cell = match quantized_math.out {
        ASrc::Val(v) => Some(v),
        _ => None,
    };
    let mut io_bounds: Vec<MathBound> = input_bounds.to_vec();
    if let Some(o) = out_cell {
        io_bounds.push(bounds[o]);
    }
    // Cells pinned to the io scale: the output and any Const-fed cell.
    let mut pinned_io = vec![false; n];
    if let Some(o) = out_cell {
        pinned_io[o] = true;
    }
    for (i, cell) in quantized_math.cells.iter().enumerate() {
        let has_const = match cell {
            AnalogCell::Sum { inputs, .. } => inputs.iter().any(|x| matches!(x, ASrc::Const(_))),
            AnalogCell::GainConst { a, .. } | AnalogCell::Mul { a, .. } => {
                matches!(a, ASrc::Const(_))
            }
            AnalogCell::Select { c, t, f } => [c, t, f].iter().any(|x| matches!(x, ASrc::Const(_))),
            AnalogCell::Exp { a }
            | AnalogCell::Log { a }
            | AnalogCell::Tanh { a }
            | AnalogCell::Sqrt { a } => matches!(a, ASrc::Const(_)),
            AnalogCell::Div { a, b } => [a, b].iter().any(|x| matches!(x, ASrc::Const(_))),
        };
        if has_const {
            pinned_io[i] = true;
        }
        // A switch passes its branch voltage through: the cell and its branch operands must
        // share a scale — V1 pins the whole select group to the io scale.
        if let AnalogCell::Select { t, f, .. } = cell {
            pinned_io[i] = true;
            for x in [t, f] {
                if let ASrc::Val(v) = x {
                    pinned_io[*v] = true;
                }
            }
        }
    }
    // Nets touching a Mul share the reference scale.
    let mut mul_class = vec![false; n];
    let mut mul_reads_input = false;
    for (i, cell) in quantized_math.cells.iter().enumerate() {
        if let AnalogCell::Mul { a, b } = cell {
            mul_class[i] = true;
            for x in [a, b] {
                match x {
                    ASrc::Val(v) => mul_class[*v] = true,
                    ASrc::In(_) => mul_reads_input = true,
                    ASrc::Const(_) => {}
                }
            }
        }
    }
    let has_mul = mul_class.iter().any(|m| *m);

    // Choose the reference scale from the realizable Vref set (largest first), covering every
    // mul-class net — and the io bounds too when a Mul reads an input (io joins the class).
    let e_m = if has_mul {
        let mut opts: Vec<f64> = target.mul_vref_options_v.iter().map(|v| v.0).collect();
        opts.sort_by(|a, b| b.partial_cmp(a).unwrap());
        let candidate = opts.iter().filter_map(|v| {
            let e = v.log2();
            (e.fract() == 0.0).then_some(e as i32)
        });
        let mut chosen = None;
        for e in candidate {
            if e < lo_e || e > hi_e {
                continue;
            }
            let class_ok = (0..n).all(|i| !mul_class[i] || fits_one(&bounds[i], e));
            let io_ok = !mul_reads_input || io_bounds.iter().all(|b| fits_one(b, e));
            if class_ok && io_ok {
                chosen = Some(e);
                break;
            }
        }
        Some(chosen.ok_or_else(|| {
            AnaLowerError::Unplaceable(AnaUnplaceable::MulReferenceUnrealizable {
                needed_v: half_span,
                options_v: target.mul_vref_options_v.iter().map(|v| v.0).collect(),
                target: target.name,
            })
        })?)
    } else {
        None
    };

    // The io scale: the largest in-range fit for inputs + output (pinned to e_m if io joined
    // the mul class).
    let e_io = if mul_reads_input {
        e_m.expect("mul_reads_input implies a Mul")
    } else {
        io_bounds
            .iter()
            .try_fold(hi_e, |acc, b| best_for(b).map(|e| acc.min(e)))
            .ok_or_else(|| dynamic_range_refusal(&io_bounds, lo_e, half_span, target))?
    };

    // Per-cell assignment.
    let mut per_net = vec![0i32; n];
    for i in 0..n {
        per_net[i] = if mul_class[i] {
            let e = e_m.expect("mul class implies e_m");
            if !fits_one(&bounds[i], e) {
                return Err(dynamic_range_refusal(&[bounds[i]], lo_e, half_span, target));
            }
            e
        } else if pinned_io[i] {
            if !fits_one(&bounds[i], e_io) {
                return Err(dynamic_range_refusal(&[bounds[i]], lo_e, half_span, target));
            }
            e_io
        } else {
            best_for(&bounds[i])
                .ok_or_else(|| dynamic_range_refusal(&[bounds[i]], lo_e, half_span, target))?
        };
    }

    // Physical weights: math weight × 2^{e_cell − e_operand}, snapped and gated.
    let lsb = target.weight_lsb();
    let snap = |w: f64| -> f64 { (w / lsb).round() * lsb };
    let src_e = |x: &ASrc| -> i32 {
        match x {
            ASrc::Val(v) => per_net[*v],
            _ => e_io,
        }
    };
    let mut phys_cells = Vec::with_capacity(n);
    for (i, cell) in quantized_math.cells.iter().enumerate() {
        phys_cells.push(match cell {
            AnalogCell::Sum { inputs, weights } => {
                let wq: Vec<f64> = inputs
                    .iter()
                    .zip(weights)
                    .map(|(x, w)| snap(w * f64::powi(2.0, per_net[i] - src_e(x))))
                    .collect();
                for w in &wq {
                    if *w != 0.0 {
                        target
                            .require_gain_ratio(w.abs())
                            .map_err(AnaLowerError::Unplaceable)?;
                    }
                }
                AnalogCell::Sum {
                    inputs: inputs.clone(),
                    weights: wq,
                }
            }
            AnalogCell::GainConst { a, k } => {
                let kq = snap(k * f64::powi(2.0, per_net[i] - src_e(a)));
                if kq != 0.0 {
                    target
                        .require_gain_ratio(kq.abs())
                        .map_err(AnaLowerError::Unplaceable)?;
                }
                AnalogCell::GainConst { a: *a, k: kq }
            }
            AnalogCell::Mul { a, b } => AnalogCell::Mul { a: *a, b: *b },
            AnalogCell::Select { c, t, f } => AnalogCell::Select {
                c: *c,
                t: *t,
                f: *f,
            },
            AnalogCell::Exp { a } => AnalogCell::Exp { a: *a },
            AnalogCell::Log { a } => AnalogCell::Log { a: *a },
            AnalogCell::Tanh { a } => AnalogCell::Tanh { a: *a },
            AnalogCell::Sqrt { a } => AnalogCell::Sqrt { a: *a },
            AnalogCell::Div { a, b } => AnalogCell::Div { a: *a, b: *b },
        });
    }
    let quantized = AnalogNetlist {
        n_in: quantized_math.n_in,
        cells: phys_cells,
        out: quantized_math.out,
    };

    // The certificate: per-net volt bounds at per-net scales, judged (per-cell) by the
    // untouched independent checker.
    let mut per_net_bounds = Vec::with_capacity(n);
    for (i, b) in bounds.iter().enumerate() {
        let s_i = f64::powi(2.0, per_net[i]);
        let (volt_lo, volt_hi) = ((b.lo * s_i).next_down(), (b.hi * s_i).next_up());
        if volt_lo < -half_span || volt_hi > half_span {
            return Err(dynamic_range_refusal(&[*b], lo_e, half_span, target));
        }
        per_net_bounds.push(NetBound {
            math: *b,
            volt_lo,
            volt_hi,
        });
    }

    let schedule = GainSchedule {
        scale_log2: e_io,
        out_descale_log2: -e_io,
        mul_vref_v: f64::powi(2.0, e_m.unwrap_or(e_io)),
        quantized,
        per_net_scale_log2: per_net,
    };
    let cert = RangeCertificate {
        input_bounds: input_bounds.to_vec(),
        per_net: per_net_bounds,
        rail_half_span_v: half_span,
        noise_floor_v: 0.0,
        min_full_scale_over_noise: 1.0,
    };
    Ok((schedule, cert))
}

fn dynamic_range_refusal(
    bounds: &[MathBound],
    lo_e: i32,
    half_span: f64,
    target: &AnalogTarget,
) -> AnaLowerError {
    let global_max = bounds.iter().map(|b| b.max_abs()).fold(0.0_f64, f64::max);
    AnaLowerError::Unplaceable(AnaUnplaceable::DynamicRangeExceeded {
        net: 0,
        needed_db: (20.0 * (global_max * f64::powi(2.0, lo_e) / half_span).log10()).max(0.0),
        available_db: 0.0,
        target: target.name,
    })
}

/// Convert a netlist + schedule + certificate into the INDEPENDENT checker's neutral vocabulary.
/// The producer's only role here is faithful translation; the judgment belongs to
/// `logicaffeine_proof::interval_replay::check_range_claim`.
pub fn to_range_claim(
    schedule: &GainSchedule,
    cert: &RangeCertificate,
) -> (
    Vec<logicaffeine_proof::interval_replay::RCell>,
    logicaffeine_proof::interval_replay::RangeClaim,
) {
    use logicaffeine_proof::interval_replay::{RCell, RSrc, RangeClaim};
    // The checker judges the dataflow silicon actually COMPUTES — the realized-math twin
    // (physical weights mapped back through the exact per-net rescales; identical to the
    // quantized weights under a uniform schedule).
    let realized = schedule.realized_math_cells();
    let conv = |s: &ASrc| match s {
        ASrc::In(k) => RSrc::In(*k),
        ASrc::Val(v) => RSrc::Val(*v),
        ASrc::Const(c) => RSrc::Const(*c),
    };
    let cells = realized
        .iter()
        .map(|c| match c {
            AnalogCell::Sum { inputs, weights } => RCell::Sum {
                inputs: inputs.iter().map(conv).collect(),
                weights: weights.clone(),
            },
            AnalogCell::Mul { a, b } => RCell::Mul {
                a: conv(a),
                b: conv(b),
            },
            AnalogCell::GainConst { a, k } => RCell::Gain { a: conv(a), k: *k },
            AnalogCell::Select { c, t, f } => RCell::Select {
                c: conv(c),
                t: conv(t),
                f: conv(f),
            },
            AnalogCell::Exp { a } => RCell::Exp { a: conv(a) },
            AnalogCell::Log { a } => RCell::Log { a: conv(a) },
            AnalogCell::Tanh { a } => RCell::Tanh { a: conv(a) },
            AnalogCell::Sqrt { a } => RCell::Sqrt { a: conv(a) },
            AnalogCell::Div { a, b } => RCell::Div {
                a: conv(a),
                b: conv(b),
            },
        })
        .collect();
    let claim = RangeClaim {
        input_lo: cert.input_bounds.iter().map(|b| b.lo).collect(),
        input_hi: cert.input_bounds.iter().map(|b| b.hi).collect(),
        scale_log2: schedule.per_net_scale_log2.clone(),
        node_lo_v: cert.per_net.iter().map(|n| n.volt_lo).collect(),
        node_hi_v: cert.per_net.iter().map(|n| n.volt_hi).collect(),
        rail_half_span_v: cert.rail_half_span_v,
        noise_floor_v: cert.noise_floor_v,
        min_full_scale_over_noise: cert.min_full_scale_over_noise,
    };
    (cells, claim)
}
