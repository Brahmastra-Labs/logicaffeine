//! The physics-effect provenance grid.
//!
//! Every [`Element`] carries an [`EffectRow`]: a FIXED-SIZE array with one [`EffectSlot`] per
//! [`PhysicsEffect`], so a row physically cannot omit an effect — the pit of success is the
//! type. A slot is `Sourced` (citation anchored in `benchmarks/silicon/analog/sourced-constants.md`),
//! `Derived` (arithmetic shown), `Modeled` (a loud design parameter awaiting the
//! characterization ladder), `Ideal` (a physics justification for why the effect is absent),
//! or `Unknown` — and `Unknown` is legal, ledgered, and ceiling-bounded in
//! `analog_effect_grid_lock`, because an honest gap beats an invented figure forever.
//!
//! Two coherence theorems the lock enforces:
//! - **No orphan physics, no phantom reference**: every [`NonIdealityModel`](crate::codegen_ana::nonideal::NonIdealityModel) field appears in
//!   exactly one slot's `params` across the whole grid.
//! - **No budget term outside the taxonomy**: [`effect_of`] totally classifies every
//!   [`ErrorSource`] as either aggregated physics effects or explicit `Analysis` bookkeeping
//!   (the bound's own arithmetic — `MulInteraction`, `EvalRounding` — is not a die mechanism,
//!   and saying so is part of the honesty).
//!
//! [`NonIdealityModel`]: crate::codegen_ana::nonideal::NonIdealityModel

use super::error_budget::ErrorSource;
use super::ontology::Element;
use super::ontology::{FunctionalMacro, Passive, Semiconductor, SourceElement};

/// Every physical effect the grid classifies, one slot each. Declaration order IS the row
/// index (`ALL_EFFECTS[i] as usize == i`, pinned by the lock).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PhysicsEffect {
    // Noise:
    NoiseThermal,
    NoiseFlicker,
    NoiseShot,
    NoiseBurst,
    // Mismatch (Pelgrom):
    MismatchArea,
    MismatchDistance,
    // Drift and aging:
    DriftTemperature,
    AgingNbti,
    AgingHci,
    DriftPackageStress,
    // Parasitics:
    ParasiticResistance,
    ParasiticCapacitance,
    ParasiticInductance,
    // Coupling:
    CouplingCapacitive,
    CouplingSubstrate,
    CouplingSupply,
    CouplingElectromagnetic,
    CouplingThermal,
    // Converters and representation:
    ConverterInl,
    ConverterDnl,
    Quantization,
    // Dynamics:
    SettlingBandwidth,
}

pub const EFFECT_COUNT: usize = 22;

/// The effect census, in declaration (= index) order.
pub const ALL_EFFECTS: &[PhysicsEffect; EFFECT_COUNT] = &[
    PhysicsEffect::NoiseThermal,
    PhysicsEffect::NoiseFlicker,
    PhysicsEffect::NoiseShot,
    PhysicsEffect::NoiseBurst,
    PhysicsEffect::MismatchArea,
    PhysicsEffect::MismatchDistance,
    PhysicsEffect::DriftTemperature,
    PhysicsEffect::AgingNbti,
    PhysicsEffect::AgingHci,
    PhysicsEffect::DriftPackageStress,
    PhysicsEffect::ParasiticResistance,
    PhysicsEffect::ParasiticCapacitance,
    PhysicsEffect::ParasiticInductance,
    PhysicsEffect::CouplingCapacitive,
    PhysicsEffect::CouplingSubstrate,
    PhysicsEffect::CouplingSupply,
    PhysicsEffect::CouplingElectromagnetic,
    PhysicsEffect::CouplingThermal,
    PhysicsEffect::ConverterInl,
    PhysicsEffect::ConverterDnl,
    PhysicsEffect::Quantization,
    PhysicsEffect::SettlingBandwidth,
];

/// One provenance-classified cell of the grid. `params` names the [`NonIdealityModel`](crate::codegen_ana::nonideal::NonIdealityModel) fields
/// this slot is the census home of (each field lives in exactly one slot, grid-wide); a slot
/// mixing provenances takes the WEAKEST of its params' statuses.
///
/// [`NonIdealityModel`]: crate::codegen_ana::nonideal::NonIdealityModel
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectSlot {
    /// A published figure. The citation's first word anchors in the dossier file.
    Sourced {
        params: &'static [&'static str],
        citation: &'static str,
    },
    /// Computed from sourced inputs — the arithmetic is stated.
    Derived {
        params: &'static [&'static str],
        arithmetic: &'static str,
    },
    /// A design parameter of our fabric, awaiting the characterization ladder. Loud, never
    /// presented as silicon.
    Modeled {
        params: &'static [&'static str],
        note: &'static str,
    },
    /// The effect is genuinely absent for this element — with the physics reason.
    Ideal { justification: &'static str },
    /// Recognized, unclassified. Legal, ledgered, ceiling-bounded.
    Unknown,
}

/// A total row: one slot per effect, by construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectRow(pub [EffectSlot; EFFECT_COUNT]);

impl EffectRow {
    pub const fn unknown() -> Self {
        EffectRow([EffectSlot::Unknown; EFFECT_COUNT])
    }

    /// Classify one slot. Double classification is a bug, not an override.
    pub fn with(mut self, e: PhysicsEffect, s: EffectSlot) -> Self {
        assert!(
            matches!(self.0[e as usize], EffectSlot::Unknown),
            "{e:?} classified twice in one row"
        );
        self.0[e as usize] = s;
        self
    }

    pub fn slot(&self, e: PhysicsEffect) -> EffectSlot {
        self.0[e as usize]
    }

    /// Non-`Unknown` slot count — the grow-only per-element ledger quantity.
    pub fn classified(&self) -> usize {
        self.0
            .iter()
            .filter(|s| !matches!(s, EffectSlot::Unknown))
            .count()
    }
}

/// THE grid — exhaustive over every element, total over every effect. Elements not yet
/// studied return the all-`Unknown` row (honest, ledgered); classified rows carry the
/// dossier's provenance. The [`NonIdealityModel`](crate::codegen_ana::nonideal::NonIdealityModel) fields live in the linear tile's row —
/// the model is tile-generic today and that row is its characterization home.
///
/// [`NonIdealityModel`]: crate::codegen_ana::nonideal::NonIdealityModel
pub fn effect_row(e: Element) -> EffectRow {
    use EffectSlot::{Derived, Ideal, Modeled, Sourced};
    use PhysicsEffect as Fx;
    match e {
        Element::Passive(Passive::Resistor) => EffectRow::unknown()
            .with(
                Fx::NoiseThermal,
                Sourced {
                    params: &[],
                    citation: "Johnson 1928 / Nyquist 1928: 4kTR thermal noise of any \
                               dissipative element — the law behind the dossier's kT/C entry",
                },
            )
            .with(
                Fx::NoiseShot,
                Ideal {
                    justification: "no potential barrier: carrier flow in a homogeneous \
                                    resistive film is not discrete-injection limited, so a \
                                    resistor exhibits Johnson noise only",
                },
            ),
        Element::Passive(Passive::Capacitor) => EffectRow::unknown()
            .with(
                Fx::NoiseThermal,
                Ideal {
                    justification: "a lossless reactance dissipates nothing and generates no \
                                    Johnson noise; the sampled kT/C variance is the switch \
                                    resistance's noise, booked on the sampling tile",
                },
            )
            .with(
                Fx::NoiseFlicker,
                Ideal {
                    justification: "1/f noise requires carrier trapping in an active \
                                    conduction path; an ideal dielectric conducts no signal \
                                    carriers",
                },
            )
            .with(
                Fx::NoiseShot,
                Ideal {
                    justification: "no barrier and no DC conduction current to be \
                                    shot-noise limited",
                },
            ),
        Element::Semiconductor(Semiconductor::FloatingGate) => EffectRow::unknown().with(
            Fx::MismatchArea,
            Sourced {
                params: &[],
                citation: "Hasler Proc. IEEE 108(8) 2020: programmed analog parameters at \
                           better than 1% accuracy over six decades — the programming-accuracy \
                           class of the fabric's weights",
            },
        ),
        Element::Macro(FunctionalMacro::WinnerTakeAll) => EffectRow::unknown().with(
            Fx::MismatchArea,
            Modeled {
                params: &[],
                note: "competitor input-referred offsets decide near-ties — the O7 margin \
                       certificate's resolution band (2*k*sigma) prices exactly this class",
            },
        ),
        Element::Source(SourceElement::Vccs) => EffectRow::unknown().with(
            Fx::MismatchArea,
            Modeled {
                params: &[],
                note: "gm mismatch of the transconductor is the physical origin of the linear \
                       tile's gain-error class — the figure is booked on that tile's row",
            },
        ),
        Element::Macro(FunctionalMacro::LinearCombTile) => EffectRow::unknown()
            .with(
                Fx::MismatchArea,
                Derived {
                    params: &["weight_gain_sigma", "offset_sigma_v", "chop_residual_v"],
                    arithmetic: "gain: Hasler 1% programming envelope taken as the 1-sigma \
                                 class (SOURCED, conservative round-up); offset: sigma(V_os) \
                                 ~ 2.8 mV from the SkyWater vth0_slope mismatch model on a \
                                 stated 5T example; chop residual: MODELED intra-pair drift \
                                 class — the slot takes the weakest param status",
                },
            )
            .with(
                Fx::MismatchDistance,
                Modeled {
                    params: &["gm_distance_sigma_per_pitch", "cross_tier_gain_sigma"],
                    note: "the Pelgrom 1989 distance term exists; no published sky130 \
                           coefficient — 0.2%/pitch parameterizes the placer until the \
                           characterization ladder replaces it. Across separately \
                           manufactured tiers the gradient gives way to a flat 2% categorical \
                           ceiling (dies share no Pelgrom field) — equally MODELED",
                },
            )
            .with(
                Fx::NoiseThermal,
                Derived {
                    params: &["noise_floor_v"],
                    arithmetic: "kT/C at C = 1 pF, T = 300 K: sqrt(4.142e-21/1e-12) = 64.4 uV \
                                 rms (Johnson-Nyquist; arithmetic in the dossier)",
                },
            )
            .with(
                Fx::DriftTemperature,
                Modeled {
                    params: &["gain_tempco_per_k", "offset_tempco_v_per_k"],
                    note: "drift-class design parameters priced against the CERTIFIED thermal \
                           map (S3): gain drifts with the tile-to-operand GRADIENT, offset \
                           with the absolute rise — awaiting characterization",
                },
            )
            .with(
                Fx::Quantization,
                Derived {
                    params: &[],
                    arithmetic: "programmed weights snap to the unit-element grid — the \
                                 WeightQuant budget term; half-grid arithmetic in gain.rs",
                },
            )
            .with(
                Fx::SettlingBandwidth,
                Modeled {
                    params: &[],
                    note: "behavioral tiers assume settled outputs; the ngspice tier measures \
                           sim_term against the pinned settling options",
                },
            ),
        Element::Macro(FunctionalMacro::GilbertMultiplier) => EffectRow::unknown()
            .with(
                Fx::MismatchArea,
                Modeled {
                    params: &[],
                    note: "shares the programmed-weight accuracy class characterized on the \
                           linear tile (the model is tile-generic today; a mul-specific \
                           figure awaits characterization)",
                },
            )
            .with(
                Fx::Quantization,
                Derived {
                    params: &[],
                    arithmetic: "the multiplier reference is drawn from the discrete mul_vref \
                                 option set — realizability gated at schedule time in gain.rs",
                },
            )
            .with(
                Fx::SettlingBandwidth,
                Modeled {
                    params: &[],
                    note: "as the linear tile",
                },
            ),
        Element::Macro(FunctionalMacro::OtaIntegrator) => EffectRow::unknown()
            .with(
                Fx::MismatchArea,
                Modeled {
                    params: &[],
                    note: "tile-generic gain/offset class today",
                },
            )
            .with(
                Fx::SettlingBandwidth,
                Modeled {
                    params: &[],
                    note: "integrator rate realizability is target-gated \
                           (require_integrator_rate)",
                },
            ),
        Element::Macro(FunctionalMacro::Comparator) => EffectRow::unknown()
            .with(
                Fx::MismatchArea,
                Modeled {
                    params: &[],
                    note: "input-referred offset of the pair — tile-generic class today; the \
                           O5 CompareBand term prices the ambiguity band",
                },
            )
            .with(
                Fx::SettlingBandwidth,
                Modeled {
                    params: &[],
                    note: "regeneration time — control-plane scheduled",
                },
            ),
        Element::Macro(FunctionalMacro::SampleHold) => EffectRow::unknown()
            .with(
                Fx::NoiseThermal,
                Modeled {
                    params: &[],
                    note: "kT/C sampled noise — the switch's Johnson noise frozen on the hold \
                           cap; tile-generic noise class today",
                },
            )
            .with(
                Fx::CouplingCapacitive,
                Modeled {
                    params: &[],
                    note: "switch charge injection and clock feedthrough onto the hold node — \
                           awaiting characterization",
                },
            )
            .with(
                Fx::SettlingBandwidth,
                Modeled {
                    params: &[],
                    note: "acquisition time — control-plane scheduled",
                },
            ),
        Element::Macro(FunctionalMacro::Dac) => EffectRow::unknown()
            .with(
                Fx::Quantization,
                Derived {
                    params: &[],
                    arithmetic: "half-LSB input quantization — the DacQuant budget term \
                                 (half_lsb in error_budget.rs)",
                },
            )
            .with(
                Fx::ConverterInl,
                Modeled {
                    params: &[],
                    note: "V1 spec class INL — characterization ladder",
                },
            )
            .with(
                Fx::ConverterDnl,
                Modeled {
                    params: &[],
                    note: "V1 spec class DNL — characterization ladder",
                },
            ),
        Element::Macro(FunctionalMacro::Adc) => EffectRow::unknown()
            .with(
                Fx::Quantization,
                Derived {
                    params: &[],
                    arithmetic: "half-LSB output quantization — the AdcQuant budget term \
                                 (half_lsb in error_budget.rs)",
                },
            )
            .with(
                Fx::ConverterInl,
                Modeled {
                    params: &[],
                    note: "V1 spec class INL — characterization ladder",
                },
            )
            .with(
                Fx::ConverterDnl,
                Modeled {
                    params: &[],
                    note: "V1 spec class DNL — characterization ladder",
                },
            ),
        // Everything else: recognized, honestly unclassified — the ledgered frontier.
        Element::Passive(
            Passive::Inductor
            | Passive::MutualInductor
            | Passive::TransmissionLine
            | Passive::Varactor
            | Passive::Memristor
            | Passive::AnalogSwitch
            | Passive::Crystal,
        )
        | Element::Source(
            SourceElement::IndependentV
            | SourceElement::IndependentI
            | SourceElement::Vcvs
            | SourceElement::Cccs
            | SourceElement::Ccvs,
        )
        | Element::Semiconductor(
            Semiconductor::Diode(_)
            | Semiconductor::Bjt(_)
            | Semiconductor::Mosfet(_)
            | Semiconductor::Jfet
            | Semiconductor::PhotoDiode,
        )
        | Element::Macro(
            FunctionalMacro::DiffPair
            | FunctionalMacro::TranslinearLoop
            | FunctionalMacro::Chopper
            | FunctionalMacro::AnalogSelect
            | FunctionalMacro::Ota
            | FunctionalMacro::OpAmp
            | FunctionalMacro::CurrentMirror
            | FunctionalMacro::BandgapReference
            | FunctionalMacro::BiasGenerator
            | FunctionalMacro::VoltageReference
            | FunctionalMacro::ClockDistribution
            | FunctionalMacro::FilterBiquad
            | FunctionalMacro::Oscillator
            | FunctionalMacro::Vco
            | FunctionalMacro::Pll
            | FunctionalMacro::Lna
            | FunctionalMacro::Pga
            | FunctionalMacro::LdoRegulator,
        ) => EffectRow::unknown(),
    }
}

/// What a budget term IS: physics of the die, or arithmetic of the bound itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetTermClass {
    /// The physical effects this term aggregates — non-empty, each classified somewhere in
    /// the grid (the lock proves it).
    Physics(&'static [PhysicsEffect]),
    /// Bookkeeping of the bound, not a die mechanism — with the reason.
    Analysis(&'static str),
}

/// Total classification of every [`ErrorSource`] into the taxonomy — a budget term cannot
/// exist outside it (no wildcard; a new variant breaks this match until classified).
pub fn effect_of(src: &ErrorSource) -> BudgetTermClass {
    use PhysicsEffect as Fx;
    match src {
        ErrorSource::DacQuant { .. } | ErrorSource::AdcQuant { .. } | ErrorSource::WeightQuant => {
            BudgetTermClass::Physics(&[Fx::Quantization])
        }
        ErrorSource::TileGain => {
            BudgetTermClass::Physics(&[Fx::MismatchArea, Fx::MismatchDistance])
        }
        ErrorSource::TileOffset => BudgetTermClass::Physics(&[Fx::MismatchArea]),
        ErrorSource::Noise => BudgetTermClass::Physics(&[Fx::NoiseThermal]),
        ErrorSource::ThermalGain | ErrorSource::ThermalOffset => {
            BudgetTermClass::Physics(&[Fx::DriftTemperature])
        }
        ErrorSource::CompareBand => BudgetTermClass::Physics(&[Fx::MismatchArea]),
        ErrorSource::ChopResidual => BudgetTermClass::Physics(&[Fx::MismatchArea]),
        ErrorSource::MulInteraction => BudgetTermClass::Analysis(
            "the second-order eps_a*eps_b cross term of the perturbation bound — arithmetic \
             of the analysis, not a physical mechanism of the die",
        ),
        ErrorSource::EvalRounding => BudgetTermClass::Analysis(
            "f64 rounding of the behavioral evaluator — the model tier's arithmetic, not the \
             die's physics",
        ),
    }
}
