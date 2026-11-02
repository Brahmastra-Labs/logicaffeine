//! ★ ANALOG TARGET ADL LOCK ★ — pins the `AnalogTarget` descriptor (the analog lane's ADL spine),
//! its presets, and the fail-closed typed-refusal surface of the mixed-signal tier.
//!
//! The analog lane is tolerance-≡ by physics, so it lives in its OWN ADL (`codegen_ana`), firewalled
//! from `codegen_sva`'s exact ≡VM regime. This lock pins:
//!   1. the `ideal()` and `sky130_fpaa()` preset descriptors field-by-field, incl. the provenance
//!      discipline (the ONE sourced physical figure — the SKY130 1.8 V rail — is cited; every other
//!      number is declared a modeled-design parameter, never a silicon claim);
//!   2. capability gating (`provides`, `require_gain_ratio`, `require_integrator_rate`) and the
//!      Display-stable [`AnaUnplaceable`] refusals;
//!   3. the compile-entry refusal behaviour: concurrency/networking → `Unsupported`, an integer-only
//!      program → `NoAnalogRegion`, float dataflow without declared ranges → `UnboundedInput` —
//!      a correct artifact or a named refusal, never a silently-wrong circuit.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. Changing a preset figure, a provenance string, or a
//! refusal category is a conscious edit here AND in `codegen_ana`.

use std::collections::BTreeSet;

use logicaffeine_base::si::{Hertz, Nanoseconds, Volts};
use logicaffeine_compile::codegen_ana::{
    analog_refusal, unsupported_reason, AnaLowerError, AnaUnplaceable, AnalogCapability,
    AnalogCellKind, AnalogTarget, ALL_ANALOG_CELL_KINDS,
};
use logicaffeine_compile::compile::compile_to_analog;
use logicaffeine_compile::optimize::oracle_analyze_with;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::{CompiledProgram, Compiler};

// ============================================================
//  Corpus
// ============================================================

/// Float dataflow, no declared input ranges — the analog lane must refuse `UnboundedInput`
/// (gain scaling without bounds would be a guess, and this lane does not guess).
const FLOAT_NO_RANGES: &str =
    "## Main\n    Let x be 0.5.\n    Let y be x * 2.0 + 0.25.\n    Show y.\n";

/// Integer-only program — nothing to map onto the analog plane: `NoAnalogRegion`.
const INT_ONLY: &str = "## Main\n    Show 7.\n";

/// Concurrency — outside the mixed-signal SoC model entirely: `Unsupported`.
const CONCURRENCY: &str = "## To produce (ch: Int):\n    Send 1 into ch.\n\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to produce with jobs.\n    Receive a from jobs.\n    Show a.\n";

/// Networking — host-only IO: `Unsupported`.
const NETWORKING: &str = "## Main\n    Connect to \"ws://127.0.0.1:9\".\n    Show \"connected\".\n";

fn compile_program(src: &str) -> CompiledProgram {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _p) = parsed.expect("parse");
        let oracle = oracle_analyze_with(stmts, interner);
        Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle)).expect("compile")
    })
}

// ============================================================
//  The cell-kind universe
// ============================================================

/// The 14 analog tile kinds, exactly once each: Kirchhoff summing, transconductance multiply,
/// programmed constant gain, OTA-C integration, continuous comparison, the comparator-driven
/// switch (O5), the translinear five (O6: exp/log/tanh/sqrt/ratio), sample-and-hold, and the
/// two plane-boundary converters.
#[test]
fn cell_kind_universe_is_pinned() {
    let all: BTreeSet<AnalogCellKind> = ALL_ANALOG_CELL_KINDS.iter().copied().collect();
    assert_eq!(
        all.len(),
        ALL_ANALOG_CELL_KINDS.len(),
        "duplicate entries in ALL_ANALOG_CELL_KINDS"
    );
    let expected: BTreeSet<AnalogCellKind> = [
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
    ]
    .into_iter()
    .collect();
    assert_eq!(all, expected);
}

// ============================================================
//  Preset descriptors — pinned field-by-field
// ============================================================

#[test]
fn ideal_preset_is_pinned() {
    let t = AnalogTarget::ideal();
    assert_eq!(t.name, "ideal");
    assert_eq!((t.rows, t.cols), (16, 16));
    assert_eq!(
        t.mul_tiles, 64,
        "multipliers are a counted scarce resource even on the ideal preset"
    );
    assert_eq!(t.rails_v, (Volts(0.0), Volts(1.8)));
    assert_eq!(t.headroom, 0.8);
    assert_eq!(
        t.cells,
        ALL_ANALOG_CELL_KINDS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
    );
    // The physical disciplines stay FINITE even on ideal: bounded fan-in, quantized weights,
    // discrete multiplier references — every certification exercises the real obligations.
    assert_eq!(t.sum_fanin, 8);
    assert_eq!(t.weight_bits, 12);
    assert_eq!(t.weight_range, (1.0 / 4096.0, 4.0));
    assert_eq!(
        t.mul_vref_options_v,
        [
            0.5,
            0.25,
            0.125,
            0.0625,
            0.03125,
            0.015625,
            0.0078125,
            0.00390625,
            0.001953125,
            0.0009765625
        ]
        .map(Volts)
        .to_vec()
    );
    assert_eq!(t.scale_log2_range, (-20, 0));
    assert_eq!((t.dac_bits, t.adc_bits), (12, 12));
    assert_eq!((t.n_dac, t.n_adc), (16, 16));
    assert_eq!(t.integrator_k_range, (Hertz(1.0), Hertz(1.0e9)));
    assert_eq!(t.settle_ns, Nanoseconds(100.0));
    assert_eq!(
        t.half_span_v(),
        0.8 * 1.8 / 2.0,
        "usable differential half-span: headroom · rail span / 2"
    );
    // The ideal preset makes NO physical claims and must say so.
    assert!(
        t.provenance.contains("ideal"),
        "ideal provenance must self-describe: {}",
        t.provenance
    );
    assert!(
        t.provenance.contains("no physical"),
        "ideal provenance must disclaim physical claims: {}",
        t.provenance
    );
}

#[test]
fn sky130_fpaa_preset_is_pinned_with_provenance() {
    let t = AnalogTarget::sky130_fpaa();
    assert_eq!(t.name, "sky130-fpaa");
    // The deliberately-boring V1 shape: 16×16 sites, scarce multipliers, fan-in-8 differential
    // linear tiles with 8-bit unit-element weights, modest converters.
    assert_eq!((t.rows, t.cols), (16, 16));
    assert_eq!(t.mul_tiles, 36);
    // The ONE sourced physical figure: SKY130's nominal digital-domain supply is 1.8 V.
    assert_eq!(t.rails_v, (Volts(0.0), Volts(1.8)));
    assert_eq!(t.headroom, 0.8);
    assert_eq!(
        t.cells,
        ALL_ANALOG_CELL_KINDS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
    );
    assert_eq!(t.sum_fanin, 8);
    assert_eq!(t.weight_bits, 8);
    assert_eq!(t.weight_range, (1.0 / 64.0, 127.0 / 64.0));
    assert_eq!(
        t.weight_lsb(),
        1.0 / 64.0,
        "127 magnitude steps over the max weight = one unit element"
    );
    assert_eq!(t.mul_vref_options_v, [0.5, 0.25, 0.125].map(Volts).to_vec());
    assert_eq!(t.scale_log2_range, (-8, 0));
    assert_eq!((t.dac_bits, t.adc_bits), (8, 8));
    assert_eq!((t.n_dac, t.n_adc), (8, 2));
    assert_eq!(t.integrator_k_range, (Hertz(1.0e3), Hertz(1.0e7)));
    assert_eq!(t.settle_ns, Nanoseconds(1000.0));
    // Provenance discipline: the rail is cited to the SKY130 PDK; everything else is declared a
    // modeled-design parameter (OUR fabric's design), never presented as silicon.
    assert!(
        t.provenance.contains("SKY130"),
        "must cite the PDK: {}",
        t.provenance
    );
    assert!(
        t.provenance.contains("1.8 V"),
        "must cite the sourced rail figure: {}",
        t.provenance
    );
    assert!(
        t.provenance.contains("design parameters"),
        "must declare design figures as design: {}",
        t.provenance
    );
    assert!(
        t.provenance.contains("not silicon"),
        "must disclaim silicon measurement: {}",
        t.provenance
    );
}

// ============================================================
//  Capability gating + typed unplaceability
// ============================================================

#[test]
fn capability_gating_provides_and_refuses() {
    let t = AnalogTarget::ideal();
    for kind in ALL_ANALOG_CELL_KINDS {
        assert!(
            t.provides(AnalogCapability::Cell(*kind)),
            "ideal must provide {kind:?}"
        );
    }
    assert!(t.provides(AnalogCapability::DacChannels(16)));
    assert!(!t.provides(AnalogCapability::DacChannels(17)));
    assert!(t.provides(AnalogCapability::AdcChannels(16)));
    assert!(!t.provides(AnalogCapability::AdcChannels(17)));

    let s = AnalogTarget::sky130_fpaa();
    assert!(s.provides(AnalogCapability::DacChannels(8)));
    assert!(!s.provides(AnalogCapability::DacChannels(9)));
    assert!(s.provides(AnalogCapability::AdcChannels(2)));
    assert!(!s.provides(AnalogCapability::AdcChannels(3)));
    assert!(s.provides(AnalogCapability::MulTiles(36)));
    assert!(
        !s.provides(AnalogCapability::MulTiles(37)),
        "multipliers are scarce and counted"
    );
}

#[test]
fn gain_integrator_and_vref_gates_refuse_typed() {
    let s = AnalogTarget::sky130_fpaa();
    assert_eq!(s.require_gain_ratio(1.5), Ok(()));
    assert_eq!(s.require_gain_ratio(1.0 / 64.0), Ok(()));
    assert_eq!(
        s.require_gain_ratio(4.0),
        Err(AnaUnplaceable::GainRatioUnrealizable {
            needed: 4.0,
            lo: 1.0 / 64.0,
            hi: 127.0 / 64.0,
            target: "sky130-fpaa"
        })
    );
    assert_eq!(s.require_integrator_rate(1.0e5), Ok(()));
    assert_eq!(
        s.require_integrator_rate(1.0),
        Err(AnaUnplaceable::IntegratorRateUnrealizable {
            needed_per_s: 1.0,
            lo: 1.0e3,
            hi: 1.0e7,
            target: "sky130-fpaa"
        })
    );
    // Rail safety does NOT imply multiplier-reference realizability: Vref comes from the
    // discrete option set, exact match only.
    assert_eq!(s.require_mul_reference(0.25), Ok(()));
    assert_eq!(
        s.require_mul_reference(0.9),
        Err(AnaUnplaceable::MulReferenceUnrealizable {
            needed_v: 0.9,
            options_v: vec![0.5, 0.25, 0.125],
            target: "sky130-fpaa"
        })
    );
}

/// Every refusal variant renders a stable, named message — refusals are user-facing spec.
#[test]
fn unplaceable_display_is_stable() {
    let cases: &[(AnaUnplaceable, &str)] = &[
        (
            AnaUnplaceable::MissingCapability { needed: AnalogCapability::Cell(AnalogCellKind::Integrate), target: "t" },
            "analog target `t` lacks capability Cell(Integrate)",
        ),
        (
            AnaUnplaceable::TooSmall { needed_cells: 99, target_rows: 8, target_cols: 8, target: "t" },
            "analog target `t` is too small: needs 99 cells, array is 8x8",
        ),
        (
            AnaUnplaceable::GainRatioUnrealizable { needed: 100.0, lo: 0.0625, hi: 16.0, target: "t" },
            "analog target `t` cannot realize gain ratio 100: realizable range is [0.0625, 16]",
        ),
        (
            AnaUnplaceable::IntegratorRateUnrealizable { needed_per_s: 1.0, lo: 1000.0, hi: 10000000.0, target: "t" },
            "analog target `t` cannot realize integrator rate 1 /s: realizable range is [1000, 10000000] /s",
        ),
        (
            AnaUnplaceable::MulReferenceUnrealizable { needed_v: 0.9, options_v: vec![0.5, 0.25], target: "t" },
            "analog target `t` cannot realize multiplier reference 0.9 V: options are [0.5, 0.25] V",
        ),
        (
            AnaUnplaceable::DynamicRangeExceeded { net: 3, needed_db: 80.0, available_db: 48.0, target: "t" },
            "analog target `t` dynamic range exceeded on net 3: needs 80 dB, 48 dB available",
        ),
    ];
    for (e, want) in cases {
        assert_eq!(format!("{e}"), *want);
    }
}

// ============================================================
//  Fail-closed compile-entry refusals
// ============================================================

/// The typed refusal layer: concurrency/networking → `Unsupported`; integer-only → `NoAnalogRegion`;
/// float dataflow without declared ranges → `UnboundedInput`. Never a silent acceptance.
#[test]
fn analog_refusal_is_typed_and_fail_closed() {
    for src in [CONCURRENCY, NETWORKING] {
        let prog = compile_program(src);
        let reason = unsupported_reason(&prog).expect("concurrency/networking must be unsupported");
        assert_eq!(
            analog_refusal(&prog, &[]),
            Some(AnaLowerError::Unsupported(reason)),
            "src:\n{src}"
        );
    }

    let int_prog = compile_program(INT_ONLY);
    assert_eq!(unsupported_reason(&int_prog), None);
    assert_eq!(
        analog_refusal(&int_prog, &[]),
        Some(AnaLowerError::NoAnalogRegion)
    );

    let float_prog = compile_program(FLOAT_NO_RANGES);
    assert_eq!(unsupported_reason(&float_prog), None);
    match analog_refusal(&float_prog, &[]) {
        Some(AnaLowerError::UnboundedInput(msg)) => {
            assert!(
                msg.contains("Assume"),
                "the refusal must teach the range-declaration surface: {msg}"
            );
        }
        other => panic!("float program without ranges must refuse UnboundedInput, got {other:?}"),
    }
}

/// The `compile.rs` surface carries the typed refusal outward as a named `analog backend:` error.
#[test]
fn compile_to_analog_surface_refuses_with_named_errors() {
    for (src, needle) in [
        // `aot_unsupported::first` names the FIRST offload op — the Pipe's ChanNew precedes the spawn.
        (CONCURRENCY, "channel op"),
        (NETWORKING, "networking"),
        (INT_ONLY, "no analog region"),
        (FLOAT_NO_RANGES, "declared range"),
    ] {
        let err = compile_to_analog(src).expect_err("phase-0 analog backend must refuse");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("analog backend:"),
            "error must be named: {msg}"
        );
        assert!(
            msg.contains(needle),
            "error must carry the specific refusal `{needle}`: {msg}"
        );
    }
}
