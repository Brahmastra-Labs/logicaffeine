//! The `AnalogTarget` ADL — the analog lane's described-fabric spine, shaped as a
//! **characterized analog PDK for the compiler**, not a feature checklist.
//!
//! Mirrors the *pattern* of [`crate::codegen_sva::fabric_target::FabricTarget`] (capability
//! gating + typed refusal + cited presets) without sharing its lattice: the digital fabric's
//! vocabulary is exact ≡VM; this one is tolerance-≡ by physics. Two regimes, two ADLs.
//!
//! ## The physical model the descriptor encodes
//! Two physical tile classes realize the whole compute vocabulary:
//! - the **linear tile** — programmable-transconductance elements into a current-summing node
//!   (`Iᵢ = gmᵢ·Vᵢ`, Kirchhoff sums the currents): realizes `Sum`, and `GainConst` as the
//!   one-input degenerate case (NOT a separate physical macro);
//! - the **multiplier tile** — a differential four-quadrant (Gilbert/translinear) cell
//!   normalizing by a programmed reference (`Vout = Va·Vb/Vref`): scarce, like FPGA DSP blocks.
//!
//! Every net is DIFFERENTIAL (`Vsignal = V⁺ − V⁻` around `Vcm ≈ mid-rail`): the math-domain
//! signed span `±half_span` is the differential swing, signed weights are a wire swap, and
//! common-mode noise cancels. Fan-in is a physical resource (summing-node loading, settling,
//! noise), so `sum_fanin` is a hard per-tile limit the compiler trees around. Weights are
//! digitally programmed from matched unit elements (`weight_bits` sign+magnitude), so a weight
//! is QUANTIZED and its snap error is an itemized error-budget term. The multiplier's `Vref`
//! comes from a DISCRETE realizable set — rail safety does NOT imply reference realizability,
//! and the gain schedule must land the block scale on a member or refuse.
//!
//! Provenance discipline: a preset may carry exactly two kinds of numbers — a **sourced
//! physical figure** (cited in the doc block AND in [`AnalogTarget::provenance`]) or a
//! **modeled-design parameter** of OUR fabric. An invented physical constant is neither, and
//! ships never. The characterization ladder that ultimately fills these fields is
//! schematic-SPICE → PVT → Monte-Carlo mismatch → layout extraction → silicon measurement.

use std::collections::BTreeSet;

use logicaffeine_base::si::{Hertz, Nanoseconds, Volts};

/// An analog tile kind the fabric can realize.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AnalogCellKind {
    /// Kirchhoff current-summing node with programmed input-conductance weights (linear tile).
    Sum,
    /// Four-quadrant transconductance (Gilbert-style) multiplier for variable×variable.
    Mul,
    /// Programmed-conductance constant gain — the ONE-INPUT linear tile (same physical macro
    /// as [`AnalogCellKind::Sum`]).
    GainConst,
    /// OTA-C integrator: `dv/dt = k·a`, `k = gm/C`.
    Integrate,
    /// Continuous comparator — the analog→digital decision boundary.
    Compare,
    /// Comparator-driven analog switch (`if c < 0 then t else f`) — the O5 data-dependent
    /// tile: the comparator half decides, the transmission-gate half passes the voltage.
    Select,
    /// Translinear exponential (subthreshold current law) — O6.
    Exp,
    /// Translinear logarithm (forward-diode law) — O6.
    Log,
    /// Differential-pair saturation — O6.
    Tanh,
    /// Translinear square root (Gilbert loop) — O6.
    Sqrt,
    /// Translinear ratio — O6 (the divisor's range must clear zero).
    Div,
    /// Sample-and-hold, strobed by the control plane (the ZOH boundary between ticks).
    SampleHold,
    /// Plane boundary in: digital word → differential voltage (DAC).
    DacIn,
    /// Plane boundary out: differential voltage → digital word (ADC).
    AdcOut,
}

/// Every analog tile kind — the cell universe a superset target provides.
pub const ALL_ANALOG_CELL_KINDS: &[AnalogCellKind] = &[
    AnalogCellKind::Sum,
    AnalogCellKind::Mul,
    AnalogCellKind::GainConst,
    AnalogCellKind::Integrate,
    AnalogCellKind::Compare,
    AnalogCellKind::Select,
    AnalogCellKind::Exp,
    AnalogCellKind::Log,
    AnalogCellKind::Tanh,
    AnalogCellKind::Sqrt,
    AnalogCellKind::Div,
    AnalogCellKind::SampleHold,
    AnalogCellKind::DacIn,
    AnalogCellKind::AdcOut,
];

/// A discrete capability a lowering demands of an analog target — the unit of typed refusal.
/// Continuous demands (gain ratios, integrator rates, multiplier references) refuse through
/// their own [`AnaUnplaceable`] variants, which carry the offending value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalogCapability {
    /// A tile kind the netlist needs.
    Cell(AnalogCellKind),
    /// At least this many DAC input channels.
    DacChannels(usize),
    /// At least this many ADC output channels.
    AdcChannels(usize),
    /// At least this many multiplier tiles (the scarce resource).
    MulTiles(usize),
}

/// Why a netlist cannot be realized on a given analog target — a TYPED refusal, never a
/// silently-wrong circuit.
#[derive(Clone, Debug, PartialEq)]
pub enum AnaUnplaceable {
    /// The target lacks a discrete capability the netlist needs.
    MissingCapability {
        needed: AnalogCapability,
        target: &'static str,
    },
    /// The netlist needs more tiles than the described array carries.
    TooSmall {
        needed_cells: usize,
        target_rows: usize,
        target_cols: usize,
        target: &'static str,
    },
    /// A required programmed weight/gain ratio falls outside the target's realizable range.
    GainRatioUnrealizable {
        needed: f64,
        lo: f64,
        hi: f64,
        target: &'static str,
    },
    /// A required integrator rate (`gm/C`, in 1/s) falls outside the target's realizable range.
    IntegratorRateUnrealizable {
        needed_per_s: f64,
        lo: f64,
        hi: f64,
        target: &'static str,
    },
    /// The block scale a `Mul`-bearing graph needs is not a realizable multiplier reference —
    /// rail safety does not imply `Vref` realizability.
    MulReferenceUnrealizable {
        needed_v: f64,
        options_v: Vec<f64>,
        target: &'static str,
    },
    /// A summing tile demands more fan-in than the physical summing node carries (fan-in is a
    /// real resource: node loading, settling, noise) — tree the sum instead.
    SumFaninExceeded {
        needed: usize,
        max: usize,
        target: &'static str,
    },
    /// A net's declared range needs more dynamic range (rail span over noise floor, in dB)
    /// than the target provides.
    DynamicRangeExceeded {
        net: usize,
        needed_db: f64,
        available_db: f64,
        target: &'static str,
    },
    /// A translinear ratio whose divisor's certified box crosses (or touches) zero — the
    /// loop starves; no output box exists.
    DivisorRangeSpansZero { net: usize, lo: f64, hi: f64 },
    /// A stack's analog tiers together carry fewer sites than the netlist needs.
    StackTooSmall {
        needed_cells: usize,
        sites: usize,
        stack: &'static str,
    },
    /// A gap's counted vertical interconnect budget cannot carry the crossings a legal
    /// placement needs — vias are SCARCE, and running out is a refusal, not an overcommit.
    VerticalResourceExhausted { gap: usize, budget: usize },
}

impl std::fmt::Display for AnaUnplaceable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnaUnplaceable::MissingCapability { needed, target } => {
                write!(f, "analog target `{target}` lacks capability {needed:?}")
            }
            AnaUnplaceable::TooSmall {
                needed_cells,
                target_rows,
                target_cols,
                target,
            } => {
                write!(f, "analog target `{target}` is too small: needs {needed_cells} cells, array is {target_rows}x{target_cols}")
            }
            AnaUnplaceable::DivisorRangeSpansZero { net, lo, hi } => {
                write!(f, "translinear divisor on net {net} spans zero: certified box [{lo}, {hi}] — the loop starves")
            }
            AnaUnplaceable::StackTooSmall {
                needed_cells,
                sites,
                stack,
            } => {
                write!(f, "stack `{stack}` is too small: needs {needed_cells} cells, analog tiers carry {sites} sites")
            }
            AnaUnplaceable::VerticalResourceExhausted { gap, budget } => {
                write!(f, "gap {gap}'s vertical interconnect budget ({budget} vias) cannot carry the required crossings")
            }
            AnaUnplaceable::GainRatioUnrealizable {
                needed,
                lo,
                hi,
                target,
            } => {
                write!(f, "analog target `{target}` cannot realize gain ratio {needed}: realizable range is [{lo}, {hi}]")
            }
            AnaUnplaceable::IntegratorRateUnrealizable {
                needed_per_s,
                lo,
                hi,
                target,
            } => {
                write!(f, "analog target `{target}` cannot realize integrator rate {needed_per_s} /s: realizable range is [{lo}, {hi}] /s")
            }
            AnaUnplaceable::MulReferenceUnrealizable {
                needed_v,
                options_v,
                target,
            } => {
                write!(f, "analog target `{target}` cannot realize multiplier reference {needed_v} V: options are {options_v:?} V")
            }
            AnaUnplaceable::SumFaninExceeded {
                needed,
                max,
                target,
            } => {
                write!(f, "analog target `{target}` summing fan-in exceeded: needs {needed}, tile carries {max}")
            }
            AnaUnplaceable::DynamicRangeExceeded {
                net,
                needed_db,
                available_db,
                target,
            } => {
                write!(f, "analog target `{target}` dynamic range exceeded on net {net}: needs {needed_db} dB, {available_db} dB available")
            }
        }
    }
}

/// A described analog fabric. One lowering retargets across shapes by varying this descriptor;
/// each demand is either realizable on it or refused with a typed [`AnaUnplaceable`].
#[derive(Clone, Debug, PartialEq)]
pub struct AnalogTarget {
    /// Human name (carried into refusal errors).
    pub name: &'static str,
    /// Described tile-array dimensions (physical tile SITES; the linear/multiplier mix below
    /// says what populates them).
    pub rows: usize,
    pub cols: usize,
    /// Multiplier tiles among the sites — the scarce resource (DSP-block discipline).
    pub mul_tiles: usize,
    /// Supply rails in volts, `(low, high)`. Signals are differential around mid-rail; the
    /// usable signed span is `±headroom·(high−low)/2`.
    pub rails_v: (Volts, Volts),
    /// Usable fraction of the differential half-span (clipping guard band).
    pub headroom: f64,
    /// The tile kinds this target provides.
    pub cells: BTreeSet<AnalogCellKind>,
    /// Hard per-tile summing fan-in (a physical resource: node loading, settling, noise). The
    /// compiler trees larger sums.
    pub sum_fanin: usize,
    /// Weight resolution: sign + magnitude from matched unit elements. A programmed weight is
    /// quantized to `weight_lsb` steps; the snap error is a budget term.
    pub weight_bits: u8,
    /// Realizable programmed weight magnitudes, `(min, max)` — the unit-element range.
    pub weight_range: (f64, f64),
    /// The DISCRETE realizable multiplier normalization references in volts. A `Mul`-bearing
    /// graph's block scale must land on a member (or refuse) — see the module doc.
    pub mul_vref_options_v: Vec<Volts>,
    /// The block-scale exponent range the fabric's references/biasing support
    /// (`volts-per-math-unit = 2^e`, `e ∈ [min, max]`).
    pub scale_log2_range: (i32, i32),
    /// DAC / ADC converter resolutions in bits.
    pub dac_bits: u8,
    pub adc_bits: u8,
    /// DAC input / ADC output channel counts.
    pub n_dac: usize,
    pub n_adc: usize,
    /// Realizable integrator rates `gm/C` in 1/s, `(min, max)`.
    pub integrator_k_range: (Hertz, Hertz),
    /// Per-tick DC settling budget in nanoseconds (linear tile; the multiplier and longer
    /// routing legs get their own figures with the characterization phases).
    pub settle_ns: Nanoseconds,
    /// Sourcing statement: which figures are cited physical facts and which are modeled-design
    /// parameters. Pinned verbatim by `analog_target_lock` — changing sourcing is a conscious edit.
    pub provenance: &'static str,
}

impl AnalogTarget {
    /// IDEAL — the abstract superset preset: every tile kind, generous converters and ranges,
    /// no non-ideality. Even the ideal preset keeps the PHYSICAL DISCIPLINES finite (fan-in,
    /// quantized weights, discrete references) so every certification exercises the real
    /// compiler obligations. Makes NO physical claims.
    pub fn ideal() -> AnalogTarget {
        AnalogTarget {
            name: "ideal",
            rows: 16,
            cols: 16,
            mul_tiles: 64,
            rails_v: (Volts(0.0), Volts(1.8)),
            headroom: 0.8,
            cells: ALL_ANALOG_CELL_KINDS.iter().copied().collect(),
            sum_fanin: 8,
            weight_bits: 12,
            weight_range: (1.0 / 4096.0, 4.0),
            // Powers of two down from the half-span decade — generous but DISCRETE.
            mul_vref_options_v: [
                0.5,
                0.25,
                0.125,
                0.0625,
                0.03125,
                0.015625,
                0.0078125,
                0.00390625,
                0.001953125,
                0.0009765625,
            ]
            .map(Volts)
            .to_vec(),
            scale_log2_range: (-20, 0),
            dac_bits: 12,
            adc_bits: 12,
            n_dac: 16,
            n_adc: 16,
            integrator_k_range: (Hertz(1.0), Hertz(1.0e9)),
            settle_ns: Nanoseconds(100.0),
            provenance: "abstract ideal preset — design parameters only, no physical claims",
        }
    }

    /// SKY130-FPAA — a modeled FPAA on the open SkyWater SKY130 process, in the deliberately
    /// boring V1 shape: a 16×16 site array with a scarce multiplier population, differential
    /// linear tiles with 8-bit unit-element weights and fan-in 8, modest converters, and ~8
    /// effective bits as the accuracy ambition.
    ///
    /// The rail is the ONE sourced physical figure: SKY130's nominal digital-domain supply is
    /// **1.8 V** (SkyWater SKY130 PDK, sky130_fd_pr device models; skywater-pdk.readthedocs.io).
    /// Everything else is OUR modeled fabric's design parameters — design choices, not silicon
    /// measurements. The non-ideality physics (mismatch sigmas, parasitics, noise floor) ships
    /// in its own phase with cited constants; until then this preset carries none rather than
    /// invented ones.
    pub fn sky130_fpaa() -> AnalogTarget {
        AnalogTarget {
            name: "sky130-fpaa",
            rows: 16,
            cols: 16,
            mul_tiles: 36,
            rails_v: (Volts(0.0), Volts(1.8)),
            headroom: 0.8,
            cells: ALL_ANALOG_CELL_KINDS.iter().copied().collect(),
            sum_fanin: 8,
            weight_bits: 8,
            weight_range: (1.0 / 64.0, 127.0 / 64.0),
            mul_vref_options_v: [0.5, 0.25, 0.125].map(Volts).to_vec(),
            scale_log2_range: (-8, 0),
            dac_bits: 8,
            adc_bits: 8,
            n_dac: 8,
            n_adc: 2,
            integrator_k_range: (Hertz(1.0e3), Hertz(1.0e7)),
            settle_ns: Nanoseconds(1000.0),
            provenance: "rails: SkyWater SKY130 PDK nominal digital supply 1.8 V (sky130_fd_pr; \
                         skywater-pdk.readthedocs.io). All other figures are modeled-FPAA design \
                         parameters, not silicon measurements.",
        }
    }

    /// The usable differential half-span in volts (`headroom · (rail_hi − rail_lo) / 2`).
    pub fn half_span_v(&self) -> f64 {
        self.headroom * (self.rails_v.1 .0 - self.rails_v.0 .0) / 2.0
    }

    /// The weight quantum: the smallest programmed-weight step the unit elements realize.
    /// Sign+magnitude with `weight_bits` total ⇒ `2^(weight_bits−1) − 1` magnitude steps over
    /// `weight_range.1`.
    pub fn weight_lsb(&self) -> f64 {
        let steps = (1u64 << (self.weight_bits - 1)) - 1;
        self.weight_range.1 / steps as f64
    }

    /// Whether this target provides a discrete capability.
    pub fn provides(&self, cap: AnalogCapability) -> bool {
        match cap {
            AnalogCapability::Cell(kind) => self.cells.contains(&kind),
            AnalogCapability::DacChannels(n) => self.n_dac >= n,
            AnalogCapability::AdcChannels(n) => self.n_adc >= n,
            AnalogCapability::MulTiles(n) => self.mul_tiles >= n,
        }
    }

    /// Gate a required programmed weight/gain magnitude against the realizable range.
    pub fn require_gain_ratio(&self, needed: f64) -> Result<(), AnaUnplaceable> {
        let (lo, hi) = self.weight_range;
        if needed >= lo && needed <= hi {
            Ok(())
        } else {
            Err(AnaUnplaceable::GainRatioUnrealizable {
                needed,
                lo,
                hi,
                target: self.name,
            })
        }
    }

    /// Gate a required integrator rate (`gm/C`, 1/s) against the realizable range.
    pub fn require_integrator_rate(&self, needed_per_s: f64) -> Result<(), AnaUnplaceable> {
        let (Hertz(lo), Hertz(hi)) = self.integrator_k_range;
        if needed_per_s >= lo && needed_per_s <= hi {
            Ok(())
        } else {
            Err(AnaUnplaceable::IntegratorRateUnrealizable {
                needed_per_s,
                lo,
                hi,
                target: self.name,
            })
        }
    }

    /// Gate a required multiplier reference voltage against the discrete realizable set
    /// (exact match — the schedule chooses FROM this set, it does not approximate into it).
    pub fn require_mul_reference(&self, needed_v: f64) -> Result<(), AnaUnplaceable> {
        if self.mul_vref_options_v.iter().any(|v| v.0 == needed_v) {
            Ok(())
        } else {
            Err(AnaUnplaceable::MulReferenceUnrealizable {
                needed_v,
                options_v: self.mul_vref_options_v.iter().map(|v| v.0).collect(),
                target: self.name,
            })
        }
    }
}
