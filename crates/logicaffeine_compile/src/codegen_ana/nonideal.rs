//! The non-ideality model — sourced physics, seeded instances, and the instance evaluator.
//!
//! Every constant here is SOURCED or DERIVED-with-shown-arithmetic (see
//! `benchmarks/silicon/analog/sourced-constants.md` for the full dossier with URLs and exact quotes) or
//! loudly MODELED (a design parameter of our fabric awaiting the characterization ladder:
//! schematic SPICE → PVT → Monte-Carlo mismatch → extraction → silicon). An invented physical
//! constant is none of these and ships never; `analog_provenance_lock` pins both the values
//! and their source strings.
//!
//! An [`InstanceSample`] is one manufactured die: per-leg gain errors (unit-element/programming
//! accuracy plus the Pelgrom DISTANCE term over the placed operand separation) and per-tile
//! output offsets and a frozen noise realization — drawn from fixed-seed SplitMix64 + Box–Muller,
//! so every Monte-Carlo battery is deterministic forever.

use logicaffeine_base::si::Volts;

use super::cell::{ASrc, AnalogCell};
use super::gain::GainSchedule;
use super::place::AnalogPlacement;
use super::target::AnalogTarget;

/// The characterized non-ideality figures a physical target carries.
#[derive(Clone, Debug, PartialEq)]
pub struct NonIdealityModel {
    /// Fractional 1σ error of a programmed weight (unit-element/programming accuracy class).
    pub weight_gain_sigma: f64,
    /// Additional fractional 1σ gain error per tile-pitch of operand separation — the Pelgrom
    /// distance-gradient lever the placer optimizes against.
    pub gm_distance_sigma_per_pitch: f64,
    /// Per-tile output-referred offset 1σ, volts.
    pub offset_sigma_v: Volts,
    /// Sampled-noise floor (kT/C class), volts rms, per tile.
    pub noise_floor_v: Volts,
    /// Additional fractional 1σ gain error between tiles on SEPARATELY MANUFACTURED tiers —
    /// a flat categorical ceiling, not a distance gradient: dies do not share a Pelgrom
    /// field, so cross-tier matching gets the class figure, full stop.
    pub cross_tier_gain_sigma: f64,
    /// Fractional gain drift per kelvin of TEMPERATURE DIFFERENCE between a tile and its
    /// operand tile — matching cares about gradients, so the coupled term prices
    /// `|ΔT_cell − ΔT_operand|`, not absolute temperature.
    pub gain_tempco_per_k: f64,
    /// Absolute output-referred offset drift, volts per kelvin of a tile's rise.
    pub offset_tempco_v_per_k: f64,
    /// The per-tile offset RESIDUAL after chopping, volts 1σ — intra-pair drift over one
    /// chop period (the frozen component cancels exactly; this is what survives).
    pub chop_residual_v: Volts,
    /// Sourcing statement — pinned verbatim by `analog_provenance_lock`.
    pub provenance: &'static str,
}

/// The SKY130-class model.
///
/// SOURCE (each figure, per `benchmarks/silicon/analog/sourced-constants.md`):
/// - `weight_gain_sigma = 0.01` — Hasler, "Large-Scale Field-Programmable Analog Arrays",
///   Proc. IEEE 108(8) 2020: programmed analog parameters at "better than 1% accuracy"
///   over six decades; we take the 1% envelope as the 1σ class (conservative round-up).
/// - `offset_sigma_v = 2.8e-3` — DERIVED from the SkyWater PDK's own Monte-Carlo mismatch
///   model (`sky130_fd_pr__nfet_01v8__vth0_slope = 3.356e-03 V·µm`, pfet 5.856e-03) applied to
///   a stated 5T-OTA example (10µ/0.5µ pair, gm ratio 0.5): σ(V_os) ≈ 2.8 mV. A model-derived
///   example, NOT a published silicon figure (none exists for sky130 — the dossier says so).
/// - `noise_floor_v = 64.4e-6` — kT/C at C = 1 pF, T = 300 K: √(4.142e-21/1e-12) = 64.4 µV rms
///   (Johnson–Nyquist, textbook formula, arithmetic in the dossier).
/// - `gm_distance_sigma_per_pitch = 0.002` — MODELED design parameter (0.2%/pitch): the
///   Pelgrom distance term exists (Pelgrom et al., JSSC 1989) but no sky130 distance
///   coefficient is published; this figure parameterizes the placement machinery until the
///   characterization ladder replaces it, and is labeled MODELED, never silicon.
pub fn sky130_nonideal() -> NonIdealityModel {
    NonIdealityModel {
        weight_gain_sigma: 0.01,
        gm_distance_sigma_per_pitch: 0.002,
        offset_sigma_v: Volts(2.8e-3),
        noise_floor_v: Volts(64.4e-6),
        cross_tier_gain_sigma: 0.02,
        gain_tempco_per_k: 1.0e-3,
        offset_tempco_v_per_k: 1.0e-5,
        chop_residual_v: Volts(50.0e-6),
        provenance: "weight_gain_sigma: Hasler Proc. IEEE 2020, FG programming better than 1% \
                     (SOURCED, conservative). offset_sigma_v: DERIVED from SkyWater sky130_fd_pr \
                     vth0_slope mismatch model (3.356e-03 V.um nfet / 5.856e-03 pfet) on a stated \
                     5T example — model-derived, not silicon. noise_floor_v: kT/C at 1 pF, 300 K \
                     = 64.4 uV rms (Johnson-Nyquist). gm_distance_sigma_per_pitch: MODELED design \
                     parameter (Pelgrom 1989 distance term; no published sky130 coefficient), \
                     replaced by characterization, never presented as silicon. \
                     cross_tier_gain_sigma: MODELED design parameter (2%): separately \
                     manufactured tiers share no Pelgrom field — a flat categorical class \
                     figure, never a distance law. gain_tempco_per_k (1e-3/K) and \
                     offset_tempco_v_per_k (10 uV/K): MODELED drift-class design parameters \
                     (amplifier-offset-drift textbook class), priced against the CERTIFIED \
                     thermal map, never silicon. chop_residual_v (50 uV): MODELED intra-pair \
                     drift class surviving the chop average — the frozen offset cancels \
                     exactly by construction. Full dossier: \
                     benchmarks/silicon/analog/sourced-constants.md",
    }
}

/// The k·σ envelope every budget term and frozen draw uses. At k = 6 a fixed-seed battery is
/// deterministically green forever (p(>6σ) ≈ 2e-9 per draw, evaluated once at authoring time).
pub const KSIGMA: f64 = 6.0;

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn uniform(state: &mut u64) -> f64 {
    ((splitmix(state) >> 11) as f64 + 0.5) / (1u64 << 53) as f64
}

/// Box–Muller over SplitMix64, CLAMPED to the ±KSIGMA envelope the budget covers (a beyond-
/// envelope draw would be a budget bug, not a physics insight; clamping keeps fixed seeds
/// deterministic AND inside the modeled class).
fn gaussian(state: &mut u64) -> f64 {
    let (u1, u2) = (uniform(state), uniform(state));
    let g = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
    g.clamp(-KSIGMA, KSIGMA)
}

/// One manufactured die: per-cell per-leg fractional gain errors, per-cell output offsets
/// (volts), and a frozen per-cell noise realization (volts).
#[derive(Clone, Debug)]
pub struct InstanceSample {
    pub leg_gain_err: Vec<Vec<f64>>,
    pub offset_v: Vec<f64>,
    pub noise_v: Vec<f64>,
}

fn legs_of(cell: &AnalogCell) -> usize {
    match cell {
        AnalogCell::Sum { inputs, .. } => inputs.len(),
        AnalogCell::Mul { .. } | AnalogCell::GainConst { .. } => 1,
        // A switch has no programmed-gain leg; its comparator offset and noise still apply.
        AnalogCell::Select { .. } => 0,
        // Translinear tiles carry one bias/gain-error leg (multiplicative on the output law).
        AnalogCell::Exp { .. }
        | AnalogCell::Log { .. }
        | AnalogCell::Tanh { .. }
        | AnalogCell::Sqrt { .. }
        | AnalogCell::Div { .. } => 1,
    }
}

/// Physical separation of a tile from its operand — the Pelgrom law's input. Same-tier
/// separation is a distance (the gradient applies); separately manufactured tiers are a
/// CATEGORY, not a distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Separation {
    SameTier { pitches: f64 },
    CrossTier,
}

/// THE shared separation law every budget composes with: programming accuracy plus the
/// distance gradient in-tier; programming accuracy plus the flat categorical ceiling across
/// tiers. 2D [`leg_sigma`] and the 3D stacked budget are both thin wrappers over this.
pub fn separation_sigma(model: &NonIdealityModel, sep: Separation) -> f64 {
    match sep {
        Separation::SameTier { pitches } => {
            model.weight_gain_sigma + model.gm_distance_sigma_per_pitch * pitches
        }
        Separation::CrossTier => model.weight_gain_sigma + model.cross_tier_gain_sigma,
    }
}

/// The operand cell a leg reads, if it is a placed cell (DAC inputs / references arrive on
/// dedicated trunks and carry no separation).
pub(crate) fn leg_operand(cells: &[AnalogCell], cell_idx: usize, leg: usize) -> Option<usize> {
    let operand = match &cells[cell_idx] {
        AnalogCell::Sum { inputs, .. } => inputs[leg],
        AnalogCell::GainConst { a, .. } => *a,
        AnalogCell::Mul { a, .. } => *a,
        AnalogCell::Select { .. } => return None,
        AnalogCell::Exp { a }
        | AnalogCell::Log { a }
        | AnalogCell::Tanh { a }
        | AnalogCell::Sqrt { a }
        | AnalogCell::Div { a, .. } => *a,
    };
    match operand {
        ASrc::Val(v) => Some(v),
        _ => None,
    }
}

/// The per-leg 1σ gain-error class over a 2D placement — the shared law at `SameTier`.
pub fn leg_sigma(
    cell_idx: usize,
    leg: usize,
    cells: &[AnalogCell],
    placement: &AnalogPlacement,
    model: &NonIdealityModel,
) -> f64 {
    let pitches = leg_operand(cells, cell_idx, leg)
        .map(|v| placement.pitch_distance(cell_idx, v))
        .unwrap_or(0.0);
    separation_sigma(model, Separation::SameTier { pitches })
}

/// Draw one die. Deterministic in `seed`.
pub fn sample_instance(
    cells: &[AnalogCell],
    placement: &AnalogPlacement,
    model: &NonIdealityModel,
    seed: u64,
) -> InstanceSample {
    let mut st = seed ^ 0xD1E5_ED0F_ABBA_C0DE;
    let mut leg_gain_err = Vec::with_capacity(cells.len());
    let mut offset_v = Vec::with_capacity(cells.len());
    let mut noise_v = Vec::with_capacity(cells.len());
    for (i, cell) in cells.iter().enumerate() {
        let legs = (0..legs_of(cell))
            .map(|l| gaussian(&mut st) * leg_sigma(i, l, cells, placement, model))
            .collect();
        leg_gain_err.push(legs);
        offset_v.push(gaussian(&mut st) * model.offset_sigma_v.0);
        noise_v.push(gaussian(&mut st) * model.noise_floor_v.0);
    }
    InstanceSample {
        leg_gain_err,
        offset_v,
        noise_v,
    }
}

/// Evaluate one die: the volt-domain converter path over the QUANTIZED netlist with the
/// instance's gain errors, offsets, and frozen noise applied per tile.
pub fn eval_analog_instance(
    schedule: &GainSchedule,
    target: &AnalogTarget,
    inst: &InstanceSample,
    inputs: &[f64],
) -> f64 {
    let netlist = &schedule.quantized;
    assert_eq!(inputs.len(), netlist.n_in);
    let grid = super::eval::ConverterGrid::of(schedule, target);
    let s = grid.s;
    let vin: Vec<f64> = inputs.iter().map(|x| grid.dac_q(*x) * s).collect();
    let mut vals: Vec<f64> = Vec::with_capacity(netlist.cells.len());
    let src = |sc: &ASrc, vals: &[f64]| -> f64 {
        match sc {
            ASrc::In(k) => vin[*k],
            ASrc::Val(v) => vals[*v],
            ASrc::Const(c) => c * s,
        }
    };
    for (i, cell) in netlist.cells.iter().enumerate() {
        let disturb = inst.offset_v[i] + inst.noise_v[i];
        let v = match cell {
            AnalogCell::Sum {
                inputs: xs,
                weights,
            } => {
                xs.iter()
                    .zip(weights)
                    .enumerate()
                    .map(|(l, (x, w))| w * (1.0 + inst.leg_gain_err[i][l]) * src(x, &vals))
                    .sum::<f64>()
                    + disturb
            }
            AnalogCell::Mul { a, b } => {
                (1.0 + inst.leg_gain_err[i][0]) * src(a, &vals) * src(b, &vals)
                    / schedule.mul_vref_v
                    + disturb
            }
            AnalogCell::GainConst { a, k } => {
                (1.0 + inst.leg_gain_err[i][0]) * k * src(a, &vals) + disturb
            }
            AnalogCell::Select { c, t, f } => {
                let decided = src(c, &vals) + inst.offset_v[i];
                let branch = if decided < 0.0 {
                    src(t, &vals)
                } else {
                    src(f, &vals)
                };
                branch + inst.noise_v[i]
            }
            AnalogCell::Exp { a } => {
                (1.0 + inst.leg_gain_err[i][0]) * src(a, &vals).exp() + disturb
            }
            AnalogCell::Log { a } => (1.0 + inst.leg_gain_err[i][0]) * src(a, &vals).ln() + disturb,
            AnalogCell::Tanh { a } => {
                (1.0 + inst.leg_gain_err[i][0]) * src(a, &vals).tanh() + disturb
            }
            AnalogCell::Sqrt { a } => {
                (1.0 + inst.leg_gain_err[i][0]) * src(a, &vals).sqrt() + disturb
            }
            AnalogCell::Div { a, b } => {
                (1.0 + inst.leg_gain_err[i][0]) * src(a, &vals) / src(b, &vals) + disturb
            }
        };
        vals.push(v);
    }
    let out_v = src(&netlist.out, &vals);
    grid.adc_q(out_v / s)
}

/// Evaluate one die CHOPPED: two frames — normal and polarity-swapped (each frozen offset
/// enters negated) — with FRESH noise per frame (drawn from `frame_seed`), averaged by the
/// digital side. The frozen offsets cancel EXACTLY; what survives is a deterministic
/// intra-pair residual draw (the model's `chop_residual_v` class) and √2-averaged noise.
pub fn eval_analog_instance_chopped(
    schedule: &GainSchedule,
    target: &AnalogTarget,
    inst: &InstanceSample,
    model: &NonIdealityModel,
    inputs: &[f64],
    frame_seed: u64,
) -> f64 {
    let netlist = &schedule.quantized;
    assert_eq!(inputs.len(), netlist.n_in);
    let grid = super::eval::ConverterGrid::of(schedule, target);
    let s = grid.s;
    let vin: Vec<f64> = inputs.iter().map(|x| grid.dac_q(*x) * s).collect();
    let mut st = frame_seed ^ 0xC0FF_EEC0_5EED_0A11;
    let run_frame = |polarity: f64, st: &mut u64| -> f64 {
        let mut vals: Vec<f64> = Vec::with_capacity(netlist.cells.len());
        let src = |sc: &ASrc, vals: &[f64]| -> f64 {
            match sc {
                ASrc::In(k) => vin[*k],
                ASrc::Val(v) => vals[*v],
                ASrc::Const(c) => c * s,
            }
        };
        for (i, cell) in netlist.cells.iter().enumerate() {
            // Frozen offset rides the polarity; residual drift and fresh noise do not chop.
            let residual = gaussian(st) * model.chop_residual_v.0;
            let noise = gaussian(st) * model.noise_floor_v.0;
            let disturb = polarity * inst.offset_v[i] + residual + noise;
            let v = match cell {
                AnalogCell::Sum {
                    inputs: xs,
                    weights,
                } => {
                    xs.iter()
                        .zip(weights)
                        .enumerate()
                        .map(|(l, (x, w))| w * (1.0 + inst.leg_gain_err[i][l]) * src(x, &vals))
                        .sum::<f64>()
                        + disturb
                }
                AnalogCell::Mul { a, b } => {
                    (1.0 + inst.leg_gain_err[i][0]) * src(a, &vals) * src(b, &vals)
                        / schedule.mul_vref_v
                        + disturb
                }
                AnalogCell::GainConst { a, k } => {
                    (1.0 + inst.leg_gain_err[i][0]) * k * src(a, &vals) + disturb
                }
                AnalogCell::Select { c, t, f } => {
                    let decided = src(c, &vals) + polarity * inst.offset_v[i];
                    let branch = if decided < 0.0 {
                        src(t, &vals)
                    } else {
                        src(f, &vals)
                    };
                    branch + noise
                }
                AnalogCell::Exp { a } => {
                    (1.0 + inst.leg_gain_err[i][0]) * src(a, &vals).exp() + disturb
                }
                AnalogCell::Log { a } => {
                    (1.0 + inst.leg_gain_err[i][0]) * src(a, &vals).ln() + disturb
                }
                AnalogCell::Tanh { a } => {
                    (1.0 + inst.leg_gain_err[i][0]) * src(a, &vals).tanh() + disturb
                }
                AnalogCell::Sqrt { a } => {
                    (1.0 + inst.leg_gain_err[i][0]) * src(a, &vals).sqrt() + disturb
                }
                AnalogCell::Div { a, b } => {
                    (1.0 + inst.leg_gain_err[i][0]) * src(a, &vals) / src(b, &vals) + disturb
                }
            };
            vals.push(v);
        }
        match netlist.out {
            ASrc::In(k) => vin[k],
            ASrc::Val(v) => vals[v],
            ASrc::Const(c) => c * s,
        }
    };
    let a = run_frame(1.0, &mut st);
    let b = run_frame(-1.0, &mut st);
    grid.adc_q((a + b) / 2.0 / s)
}
