//! ★ THERMAL PROVENANCE LOCK ★ — every figure the thermal model carries is sourced, derived,
//! or loudly modeled, and says which.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. Changing a carried figure or its sourcing is a
//! conscious edit here AND in the dossier.

use logicaffeine_compile::codegen_ana::stack::CoolingClass;
use logicaffeine_compile::codegen_ana::thermal::{
    film_coefficient_w_m2k, thermal_default_opts, THERMAL_MODEL_PROVENANCE,
};

const DOSSIER: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../benchmarks/silicon/analog/thermal-constants.md"
);

#[test]
fn the_model_provenance_names_every_figure_class_and_anchors_in_the_dossier() {
    for needle in [
        "Sourced",
        "Modeled",
        "DERIVED",
        "Incropera",
        "sky130_power.json",
        "k_Cu",
        "benchmarks/silicon/analog/thermal-constants.md",
        "never silicon",
    ] {
        assert!(
            THERMAL_MODEL_PROVENANCE.contains(needle),
            "the provenance statement must say `{needle}`: {THERMAL_MODEL_PROVENANCE}"
        );
    }
    let dossier = std::fs::read_to_string(DOSSIER).expect("the dossier exists");
    // The carried film-coefficient points and their sourced ranges.
    assert!(
        dossier.contains("2–25") || dossier.contains("2-25"),
        "natural-convection range"
    );
    assert!(
        dossier.contains("25–250") || dossier.contains("25-250"),
        "forced-air range"
    );
    assert!(dossier.contains("Table 1.1"), "the range's source is named");
    // The modeled model knobs anchor in the dossier.
    for needle in ["bias power", "thermal cell edge", "bond"] {
        assert!(
            dossier.contains(needle),
            "dossier must carry the `{needle}` model figure"
        );
    }
}

#[test]
fn the_carried_points_are_pinned_inside_their_sourced_ranges() {
    let natural = film_coefficient_w_m2k(CoolingClass::NaturalConvection);
    let forced = film_coefficient_w_m2k(CoolingClass::ForcedAir);
    let plate = film_coefficient_w_m2k(CoolingClass::ColdPlate);
    assert_eq!(
        (natural, forced, plate),
        (10.0, 100.0, 2000.0),
        "the carried points"
    );
    assert!(
        (2.0..=25.0).contains(&natural),
        "inside Incropera's free-gas range"
    );
    assert!(
        (25.0..=250.0).contains(&forced),
        "inside the forced-gas range"
    );
    assert!(
        (100.0..=20_000.0).contains(&plate),
        "inside the forced-liquid range"
    );

    let opts = thermal_default_opts();
    assert_eq!(opts.analog_grid, (4, 4));
    assert_eq!(opts.cell_edge_um, 500.0);
    assert_eq!(opts.bond_thickness_um, 10.0);
    assert_eq!(opts.tile_bias_w, 1.0e-4);
}
