//! ★ ANALOG PROVENANCE LOCK ★ — every physical constant is SOURCED, DERIVED-with-arithmetic,
//! or loudly MODELED; changing a value or deleting a citation is a conscious two-file edit.
//!
//! The full dossier (URLs, exact quotes, arithmetic) lives at `benchmarks/silicon/analog/sourced-constants.md`;
//! this lock pins the constants the compiler actually uses AND the source markers in their
//! provenance strings, so a silent retune or an unmarked invention cannot land.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE.

use logicaffeine_compile::codegen_ana::{sky130_nonideal, AnalogTarget, KSIGMA};

#[test]
fn the_nonideality_constants_and_their_sources_are_pinned() {
    let m = sky130_nonideal();

    // SOURCED: Hasler Proc. IEEE 2020 — programmed analog parameters at better than 1%.
    assert_eq!(m.weight_gain_sigma, 0.01);
    assert!(
        m.provenance.contains("Hasler"),
        "weight sigma must cite its source: {}",
        m.provenance
    );

    // DERIVED: from the PDK's own mismatch model (vth0_slope), a stated 5T example — NOT silicon.
    assert_eq!(m.offset_sigma_v, logicaffeine_base::si::Volts(2.8e-3));
    assert!(
        m.provenance.contains("vth0_slope"),
        "offset must cite the PDK model: {}",
        m.provenance
    );
    assert!(
        m.provenance.contains("not silicon"),
        "offset must disclaim silicon: {}",
        m.provenance
    );

    // DERIVED: kT/C at 1 pF, 300 K.
    assert_eq!(m.noise_floor_v, logicaffeine_base::si::Volts(64.4e-6));
    assert!(
        m.provenance.contains("kT/C"),
        "noise floor must cite the formula: {}",
        m.provenance
    );

    // MODELED: the Pelgrom distance coefficient has no published sky130 figure — it must say so.
    assert_eq!(m.gm_distance_sigma_per_pitch, 0.002);
    assert!(
        m.provenance.contains("MODELED"),
        "the distance coefficient must be marked MODELED: {}",
        m.provenance
    );
    assert!(
        m.provenance
            .contains("benchmarks/silicon/analog/sourced-constants.md"),
        "the provenance must point at the dossier: {}",
        m.provenance
    );
}

#[test]
fn the_sigma_envelope_and_target_provenance_are_pinned() {
    assert_eq!(
        KSIGMA, 6.0,
        "the k·σ envelope is the determinism contract of every MC battery"
    );
    let t = AnalogTarget::sky130_fpaa();
    assert!(t.provenance.contains("SKY130"), "{}", t.provenance);
    assert!(
        t.provenance.contains("not silicon measurements"),
        "{}",
        t.provenance
    );
}
