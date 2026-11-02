//! The typed ontology of analog circuits.
//!
//! The 10,000-year framing: encode what analog circuits ARE — every element class physics
//! builds — not merely what this compiler lowers today. Three layers, one direction of
//! reference:
//!
//! ```text
//! Element (what physics builds)  ←— realized_by —  ComputeOp (what the compiler lowers)
//!         |                                                |
//!    EffectRow (effects.rs)                     AnalogCellKind (today's realized tile IR,
//!    [Element × PhysicsEffect]                   unchanged — injects via cell_kind_op)
//! ```
//!
//! The pit of success is the Rust exhaustive match: adding an `Element` variant breaks every
//! classifying `fn` at compile time until the classification is consciously written, and the
//! ledgers in `analog_ontology_lock` make the DIRECTION of change monotone — an element can
//! only move Unrealized → ComputeMapped (a witnessed lowering, celebrated), never silently
//! back. Nothing in the existing lanes moves: [`AnalogCellKind`] is untouched and injects
//! into the vocabulary totally.

use super::target::AnalogCellKind;

/// Two-terminal and passive network elements.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Passive {
    Resistor,
    Capacitor,
    Inductor,
    MutualInductor,
    TransmissionLine,
    Varactor,
    Memristor,
    AnalogSwitch,
    Crystal,
}

/// Independent and controlled sources — the linear-network vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceElement {
    IndependentV,
    IndependentI,
    /// Voltage-controlled current source — the transconductor, the linear tile's soul.
    Vccs,
    Vcvs,
    Cccs,
    Ccvs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiodeRegion {
    ForwardExp,
    Reverse,
    Breakdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BjtRegion {
    ForwardActive,
    Saturation,
    Cutoff,
    ReverseActive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MosRegion {
    SubthresholdExp,
    SaturationSquareLaw,
    Triode,
    Cutoff,
}

/// Semiconductor primitives, by device × operating region (the region IS the mathematical
/// law the element contributes — subthreshold is exp, saturation is square-law).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Semiconductor {
    Diode(DiodeRegion),
    Bjt(BjtRegion),
    Mosfet(MosRegion),
    Jfet,
    PhotoDiode,
    /// The Hasler programming substrate (the sourced <1% figure's home).
    FloatingGate,
}

/// Functional macros — the characterized, replicated blocks a chip is actually built from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FunctionalMacro {
    // The compute-mapped present (the V1 chip):
    LinearCombTile,
    GilbertMultiplier,
    OtaIntegrator,
    Comparator,
    SampleHold,
    // The compute-mapped future (each arrives with its planned op):
    DiffPair,
    TranslinearLoop,
    WinnerTakeAll,
    Chopper,
    AnalogSelect,
    // Support / interface macros:
    Ota,
    OpAmp,
    CurrentMirror,
    BandgapReference,
    BiasGenerator,
    VoltageReference,
    ClockDistribution,
    Dac,
    Adc,
    // Recognized, unrealized:
    FilterBiquad,
    Oscillator,
    Vco,
    Pll,
    Lna,
    Pga,
    LdoRegulator,
}

/// One element of the analog universe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Element {
    Passive(Passive),
    Source(SourceElement),
    Semiconductor(Semiconductor),
    Macro(FunctionalMacro),
}

/// Every element, exactly once, in canonical order — the census surface.
pub const ALL_ELEMENTS: &[Element] = &[
    Element::Passive(Passive::Resistor),
    Element::Passive(Passive::Capacitor),
    Element::Passive(Passive::Inductor),
    Element::Passive(Passive::MutualInductor),
    Element::Passive(Passive::TransmissionLine),
    Element::Passive(Passive::Varactor),
    Element::Passive(Passive::Memristor),
    Element::Passive(Passive::AnalogSwitch),
    Element::Passive(Passive::Crystal),
    Element::Source(SourceElement::IndependentV),
    Element::Source(SourceElement::IndependentI),
    Element::Source(SourceElement::Vccs),
    Element::Source(SourceElement::Vcvs),
    Element::Source(SourceElement::Cccs),
    Element::Source(SourceElement::Ccvs),
    Element::Semiconductor(Semiconductor::Diode(DiodeRegion::ForwardExp)),
    Element::Semiconductor(Semiconductor::Diode(DiodeRegion::Reverse)),
    Element::Semiconductor(Semiconductor::Diode(DiodeRegion::Breakdown)),
    Element::Semiconductor(Semiconductor::Bjt(BjtRegion::ForwardActive)),
    Element::Semiconductor(Semiconductor::Bjt(BjtRegion::Saturation)),
    Element::Semiconductor(Semiconductor::Bjt(BjtRegion::Cutoff)),
    Element::Semiconductor(Semiconductor::Bjt(BjtRegion::ReverseActive)),
    Element::Semiconductor(Semiconductor::Mosfet(MosRegion::SubthresholdExp)),
    Element::Semiconductor(Semiconductor::Mosfet(MosRegion::SaturationSquareLaw)),
    Element::Semiconductor(Semiconductor::Mosfet(MosRegion::Triode)),
    Element::Semiconductor(Semiconductor::Mosfet(MosRegion::Cutoff)),
    Element::Semiconductor(Semiconductor::Jfet),
    Element::Semiconductor(Semiconductor::PhotoDiode),
    Element::Semiconductor(Semiconductor::FloatingGate),
    Element::Macro(FunctionalMacro::LinearCombTile),
    Element::Macro(FunctionalMacro::GilbertMultiplier),
    Element::Macro(FunctionalMacro::OtaIntegrator),
    Element::Macro(FunctionalMacro::Comparator),
    Element::Macro(FunctionalMacro::SampleHold),
    Element::Macro(FunctionalMacro::DiffPair),
    Element::Macro(FunctionalMacro::TranslinearLoop),
    Element::Macro(FunctionalMacro::WinnerTakeAll),
    Element::Macro(FunctionalMacro::Chopper),
    Element::Macro(FunctionalMacro::AnalogSelect),
    Element::Macro(FunctionalMacro::Ota),
    Element::Macro(FunctionalMacro::OpAmp),
    Element::Macro(FunctionalMacro::CurrentMirror),
    Element::Macro(FunctionalMacro::BandgapReference),
    Element::Macro(FunctionalMacro::BiasGenerator),
    Element::Macro(FunctionalMacro::VoltageReference),
    Element::Macro(FunctionalMacro::ClockDistribution),
    Element::Macro(FunctionalMacro::Dac),
    Element::Macro(FunctionalMacro::Adc),
    Element::Macro(FunctionalMacro::FilterBiquad),
    Element::Macro(FunctionalMacro::Oscillator),
    Element::Macro(FunctionalMacro::Vco),
    Element::Macro(FunctionalMacro::Pll),
    Element::Macro(FunctionalMacro::Lna),
    Element::Macro(FunctionalMacro::Pga),
    Element::Macro(FunctionalMacro::LdoRegulator),
];

/// Layer 2: the compute vocabulary — the ops the ontology recognizes. The REALIZED subset is
/// bijective with today's tile IR; the rest are named-future (the `KNOWN_PLANNED` ledger).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ComputeOp {
    // Realized today (bijective with AnalogCellKind):
    Sum,
    Mul,
    GainConst,
    Integrate,
    Compare,
    SampleHold,
    DacIn,
    AdcOut,
    // Named-future (each arrives with its campaign phase):
    Select,
    Div,
    Sqrt,
    Exp,
    Log,
    Tanh,
    ArgMax,
}

pub const ALL_COMPUTE_OPS: &[ComputeOp] = &[
    ComputeOp::Sum,
    ComputeOp::Mul,
    ComputeOp::GainConst,
    ComputeOp::Integrate,
    ComputeOp::Compare,
    ComputeOp::SampleHold,
    ComputeOp::DacIn,
    ComputeOp::AdcOut,
    ComputeOp::Select,
    ComputeOp::Div,
    ComputeOp::Sqrt,
    ComputeOp::Exp,
    ComputeOp::Log,
    ComputeOp::Tanh,
    ComputeOp::ArgMax,
];

/// The ops whose tile kinds exist in the ADL today (`ALL_ANALOG_CELL_KINDS`) — the anchor of
/// the realized/planned split.
pub const REALIZED_OPS: &[ComputeOp] = &[
    ComputeOp::Sum,
    ComputeOp::Mul,
    ComputeOp::GainConst,
    ComputeOp::Integrate,
    ComputeOp::Compare,
    ComputeOp::SampleHold,
    ComputeOp::DacIn,
    ComputeOp::AdcOut,
    ComputeOp::Select,
    ComputeOp::Exp,
    ComputeOp::Log,
    ComputeOp::Tanh,
    ComputeOp::Sqrt,
    ComputeOp::Div,
];

/// Total injection of today's tile IR into the vocabulary — no wildcard, so a new cell kind
/// cannot escape the ontology.
pub fn cell_kind_op(k: AnalogCellKind) -> ComputeOp {
    match k {
        AnalogCellKind::Sum => ComputeOp::Sum,
        AnalogCellKind::Mul => ComputeOp::Mul,
        AnalogCellKind::GainConst => ComputeOp::GainConst,
        AnalogCellKind::Integrate => ComputeOp::Integrate,
        AnalogCellKind::Compare => ComputeOp::Compare,
        AnalogCellKind::Select => ComputeOp::Select,
        AnalogCellKind::Exp => ComputeOp::Exp,
        AnalogCellKind::Log => ComputeOp::Log,
        AnalogCellKind::Tanh => ComputeOp::Tanh,
        AnalogCellKind::Sqrt => ComputeOp::Sqrt,
        AnalogCellKind::Div => ComputeOp::Div,
        AnalogCellKind::SampleHold => ComputeOp::SampleHold,
        AnalogCellKind::DacIn => ComputeOp::DacIn,
        AnalogCellKind::AdcOut => ComputeOp::AdcOut,
    }
}

/// What a non-compute element does for the chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SupportFunction {
    Bias,
    Reference,
    Clock,
    Regulation,
    Programming,
    Routing,
}

/// Layer 3: an element's role — the total classification the ledgers pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElementRole {
    /// A lowering exists NOW for these ops (or the element is a realized constituent of one).
    ComputeMapped(&'static [ComputeOp]),
    /// Bias / reference / clock / regulation / programming / routing — never carries signal math.
    Support(SupportFunction),
    /// The plane boundary (converters).
    Interface(ComputeOp),
    /// In the ontology, no realization yet — ledgered shrink-only.
    Unrealized,
}

/// THE role classification — exhaustive over every leaf, no `_` arm anywhere.
pub fn role(e: Element) -> ElementRole {
    use ComputeOp as Op;
    use ElementRole::{ComputeMapped as Mapped, Interface, Support, Unrealized};
    use SupportFunction as Sf;
    match e {
        // ── Passives ──
        Element::Passive(p) => match p {
            // State storage IS realized: the S&H and the OTA-C integrator hold charge on caps.
            Passive::Capacitor => Mapped(&[Op::SampleHold, Op::Integrate]),
            Passive::AnalogSwitch => Support(Sf::Routing),
            Passive::Crystal => Support(Sf::Clock),
            Passive::Resistor
            | Passive::Inductor
            | Passive::MutualInductor
            | Passive::TransmissionLine
            | Passive::Varactor
            | Passive::Memristor => Unrealized,
        },
        // ── Sources ──
        Element::Source(s) => match s {
            // The transconductor is the linear tile's physical mechanism.
            SourceElement::Vccs => Mapped(&[Op::Sum, Op::GainConst]),
            SourceElement::IndependentV | SourceElement::IndependentI => Support(Sf::Reference),
            SourceElement::Vcvs | SourceElement::Cccs | SourceElement::Ccvs => Unrealized,
        },
        // ── Semiconductors: regions are future realizers (translinear/subthreshold lanes) —
        //    Unrealized until their ops lower; the ledger records each by name. ──
        Element::Semiconductor(s) => match s {
            Semiconductor::FloatingGate => Support(Sf::Programming),
            // The O6 promotions: the operating regions whose mathematical laws the
            // translinear tiles witness.
            Semiconductor::Mosfet(MosRegion::SubthresholdExp) => {
                Mapped(&[Op::Exp, Op::Tanh, Op::Div, Op::Sqrt])
            }
            Semiconductor::Bjt(BjtRegion::ForwardActive) => Mapped(&[Op::Div, Op::Sqrt]),
            Semiconductor::Diode(DiodeRegion::ForwardExp) => Mapped(&[Op::Log]),
            Semiconductor::Diode(_)
            | Semiconductor::Bjt(_)
            | Semiconductor::Mosfet(_)
            | Semiconductor::Jfet
            | Semiconductor::PhotoDiode => Unrealized,
        },
        // ── Macros ──
        Element::Macro(m) => match m {
            FunctionalMacro::LinearCombTile => Mapped(&[Op::Sum, Op::GainConst]),
            FunctionalMacro::GilbertMultiplier => Mapped(&[Op::Mul]),
            FunctionalMacro::OtaIntegrator => Mapped(&[Op::Integrate]),
            FunctionalMacro::Comparator => Mapped(&[Op::Compare, Op::Select]),
            FunctionalMacro::SampleHold => Mapped(&[Op::SampleHold]),
            FunctionalMacro::Dac => Interface(Op::DacIn),
            FunctionalMacro::Adc => Interface(Op::AdcOut),
            FunctionalMacro::CurrentMirror | FunctionalMacro::BiasGenerator => Support(Sf::Bias),
            FunctionalMacro::BandgapReference | FunctionalMacro::VoltageReference => {
                Support(Sf::Reference)
            }
            FunctionalMacro::ClockDistribution => Support(Sf::Clock),
            FunctionalMacro::Ota | FunctionalMacro::OpAmp | FunctionalMacro::LdoRegulator => {
                Support(Sf::Regulation)
            }
            FunctionalMacro::AnalogSelect => Mapped(&[Op::Select]),
            FunctionalMacro::TranslinearLoop => Mapped(&[Op::Div, Op::Sqrt, Op::Log]),
            FunctionalMacro::DiffPair => Mapped(&[Op::Exp, Op::Tanh]),
            FunctionalMacro::WinnerTakeAll
            | FunctionalMacro::Chopper
            | FunctionalMacro::FilterBiquad
            | FunctionalMacro::Oscillator
            | FunctionalMacro::Vco
            | FunctionalMacro::Pll
            | FunctionalMacro::Lna
            | FunctionalMacro::Pga => Unrealized,
        },
    }
}

/// Which elements realize each compute op — exhaustive, non-empty. For REALIZED ops every
/// listed realizer is itself `ComputeMapped`/`Interface` (the lock proves it); for PLANNED
/// ops the realizers are the elements whose lowering phase will promote them.
pub fn realizing_elements(op: ComputeOp) -> &'static [Element] {
    match op {
        ComputeOp::Sum | ComputeOp::GainConst => &[
            Element::Macro(FunctionalMacro::LinearCombTile),
            Element::Source(SourceElement::Vccs),
        ],
        ComputeOp::Mul => &[Element::Macro(FunctionalMacro::GilbertMultiplier)],
        ComputeOp::Integrate => &[
            Element::Macro(FunctionalMacro::OtaIntegrator),
            Element::Passive(Passive::Capacitor),
        ],
        ComputeOp::Compare => &[Element::Macro(FunctionalMacro::Comparator)],
        ComputeOp::SampleHold => &[
            Element::Macro(FunctionalMacro::SampleHold),
            Element::Passive(Passive::Capacitor),
        ],
        ComputeOp::DacIn => &[Element::Macro(FunctionalMacro::Dac)],
        ComputeOp::AdcOut => &[Element::Macro(FunctionalMacro::Adc)],
        ComputeOp::Select => &[
            Element::Macro(FunctionalMacro::AnalogSelect),
            Element::Macro(FunctionalMacro::Comparator),
        ],
        ComputeOp::Div | ComputeOp::Sqrt => &[
            Element::Macro(FunctionalMacro::TranslinearLoop),
            Element::Semiconductor(Semiconductor::Bjt(BjtRegion::ForwardActive)),
            Element::Semiconductor(Semiconductor::Mosfet(MosRegion::SubthresholdExp)),
        ],
        ComputeOp::Exp | ComputeOp::Tanh => &[
            Element::Macro(FunctionalMacro::DiffPair),
            Element::Semiconductor(Semiconductor::Mosfet(MosRegion::SubthresholdExp)),
        ],
        ComputeOp::Log => &[
            Element::Macro(FunctionalMacro::TranslinearLoop),
            Element::Semiconductor(Semiconductor::Diode(DiodeRegion::ForwardExp)),
        ],
        ComputeOp::ArgMax => &[
            Element::Macro(FunctionalMacro::WinnerTakeAll),
            Element::Macro(FunctionalMacro::CurrentMirror),
        ],
    }
}

/// The census device: a stable name per element — exhaustive by construction (delegates to
/// the leaf enums' `Debug`, which the census lock pins for duplicates and coverage).
pub fn element_name(e: Element) -> String {
    match e {
        Element::Passive(p) => format!("Passive::{p:?}"),
        Element::Source(s) => format!("Source::{s:?}"),
        Element::Semiconductor(s) => format!("Semi::{s:?}"),
        Element::Macro(m) => format!("Macro::{m:?}"),
    }
}
