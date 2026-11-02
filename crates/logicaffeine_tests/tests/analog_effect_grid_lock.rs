//! ★ ANALOG EFFECT-GRID LOCK ★ — every element × every physics effect, provenance-classified
//! or honestly Unknown, with monotone ledgers.
//!
//! What this file locks:
//!   1. CENSUS — `ALL_EFFECTS` is duplicate-free and its declaration order IS the row index;
//!      a row is a fixed-size array, so totality is structural, not asserted.
//!   2. LEDGERS — per-element classified counts (GROW-ONLY: classification only advances) and
//!      the grid-wide `UNKNOWN_CEILING` (SHRINK-ONLY). Elements absent from the ledger are
//!      fully Unknown. Vacuity guard: every slot kind actually occurs (the machinery is
//!      load-bearing, not decorative).
//!   3. NO ORPHAN PHYSICS, NO PHANTOM REFERENCE — every `NonIdealityModel` field appears in
//!      exactly one slot's `params`, grid-wide; a full struct literal here makes ADDING a
//!      model field a compile error until this census is consciously updated.
//!   4. HONEST WORDS — every `Sourced` citation anchors (first word) in the dossier file
//!      `benchmarks/silicon/analog/sourced-constants.md`; Derived arithmetic / Modeled notes / Ideal
//!      justifications are substantive, not stubs.
//!   5. THE TAXONOMY IS TOTAL OVER THE BUDGET — every `ErrorSource` classifies via
//!      `effect_of` (physics terms cite non-empty effect lists, each classified on some
//!      mapped/interface element's row; analysis terms carry their reason).
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. A classified slot never returns to Unknown; the
//! ceiling never rises; a new model field or budget term lands here the moment it exists.

use std::collections::BTreeMap;

use logicaffeine_compile::codegen_ana::effects::{
    effect_of, effect_row, BudgetTermClass, EffectSlot, PhysicsEffect, ALL_EFFECTS, EFFECT_COUNT,
};
use logicaffeine_compile::codegen_ana::ontology::{element_name, role, ElementRole, ALL_ELEMENTS};
use logicaffeine_compile::codegen_ana::{ErrorSource, NonIdealityModel};

/// GROW-ONLY per-element classified-slot counts. An element absent here is fully Unknown.
/// Raising a count = new provenance work landed (celebrated); lowering one = a regression.
const CLASSIFIED_COUNTS: &[(&str, usize)] = &[
    ("Passive::Resistor", 2),
    ("Passive::Capacitor", 3),
    ("Semi::FloatingGate", 1),
    ("Source::Vccs", 1),
    ("Macro::WinnerTakeAll", 1),
    ("Macro::LinearCombTile", 6),
    ("Macro::GilbertMultiplier", 3),
    ("Macro::OtaIntegrator", 2),
    ("Macro::Comparator", 2),
    ("Macro::SampleHold", 3),
    ("Macro::Dac", 3),
    ("Macro::Adc", 3),
];

/// SHRINK-ONLY: 55 elements × 22 effects − 30 classified slots. Unknown is legal and ledgered;
/// this ceiling only ever moves DOWN, and exactly in step with the counts above.
const UNKNOWN_CEILING: usize = 1180;

const DOSSIER: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../benchmarks/silicon/analog/sourced-constants.md"
);

#[test]
fn the_effect_census_is_duplicate_free_and_declaration_order_is_the_row_index() {
    assert_eq!(ALL_EFFECTS.len(), EFFECT_COUNT);
    assert_eq!(
        EFFECT_COUNT, 22,
        "the effect census moved — a conscious edit updates this pin"
    );
    for (i, e) in ALL_EFFECTS.iter().enumerate() {
        assert_eq!(
            *e as usize, i,
            "{e:?} is out of declaration order in ALL_EFFECTS"
        );
    }
}

#[test]
fn every_row_matches_the_ledger_and_unknown_is_ceiling_bounded() {
    let ledger: BTreeMap<&str, usize> = CLASSIFIED_COUNTS.iter().copied().collect();
    assert_eq!(
        ledger.len(),
        CLASSIFIED_COUNTS.len(),
        "duplicate element in CLASSIFIED_COUNTS"
    );

    let (mut total_unknown, mut seen) = (0usize, BTreeMap::new());
    let (mut n_sourced, mut n_derived, mut n_modeled, mut n_ideal) = (0, 0, 0, 0);
    for e in ALL_ELEMENTS {
        let name = element_name(*e);
        let row = effect_row(*e);
        let classified = row.classified();
        total_unknown += EFFECT_COUNT - classified;
        assert_eq!(
            classified,
            ledger.get(name.as_str()).copied().unwrap_or(0),
            "`{name}`: classified-slot count disagrees with the GROW-ONLY ledger"
        );
        seen.insert(name.clone(), classified);
        for fx in ALL_EFFECTS {
            match row.slot(*fx) {
                EffectSlot::Sourced { .. } => n_sourced += 1,
                EffectSlot::Derived { .. } => n_derived += 1,
                EffectSlot::Modeled { .. } => n_modeled += 1,
                EffectSlot::Ideal { .. } => n_ideal += 1,
                EffectSlot::Unknown => {}
            }
        }
    }
    for (name, _) in CLASSIFIED_COUNTS {
        assert!(
            seen.contains_key(*name),
            "ledger names unknown element `{name}`"
        );
    }
    assert_eq!(
        total_unknown, UNKNOWN_CEILING,
        "the Unknown count moved — shrink the SHRINK-ONLY ceiling in step (never raise it)"
    );
    // Vacuity guard: every provenance kind is load-bearing somewhere.
    assert!(
        n_sourced >= 2,
        "no Sourced slots — the citation machinery is decorative"
    );
    assert!(n_derived >= 2, "no Derived slots");
    assert!(n_modeled >= 2, "no Modeled slots");
    assert!(n_ideal >= 2, "no Ideal slots");
}

#[test]
fn every_nonideality_field_lives_in_exactly_one_slot_grid_wide() {
    // A FULL struct literal: adding a NonIdealityModel field breaks this at compile time
    // until the census below is consciously extended.
    let _census_guard = NonIdealityModel {
        weight_gain_sigma: 0.0,
        gm_distance_sigma_per_pitch: 0.0,
        offset_sigma_v: logicaffeine_base::si::Volts(0.0),
        noise_floor_v: logicaffeine_base::si::Volts(0.0),
        cross_tier_gain_sigma: 0.0,
        gain_tempco_per_k: 0.0,
        offset_tempco_v_per_k: 0.0,
        chop_residual_v: logicaffeine_base::si::Volts(0.0),
        provenance: "",
    };
    let fields = [
        "weight_gain_sigma",
        "gm_distance_sigma_per_pitch",
        "offset_sigma_v",
        "noise_floor_v",
        "cross_tier_gain_sigma",
        "gain_tempco_per_k",
        "offset_tempco_v_per_k",
        "chop_residual_v",
    ];

    let mut homes: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for e in ALL_ELEMENTS {
        let row = effect_row(*e);
        for fx in ALL_EFFECTS {
            let params = match row.slot(*fx) {
                EffectSlot::Sourced { params, .. }
                | EffectSlot::Derived { params, .. }
                | EffectSlot::Modeled { params, .. } => params,
                EffectSlot::Ideal { .. } | EffectSlot::Unknown => &[][..],
            };
            for p in params {
                assert!(
                    fields.contains(p),
                    "slot {fx:?} of `{}` references phantom model field `{p}`",
                    element_name(*e)
                );
                homes
                    .entry(p)
                    .or_default()
                    .push(format!("{}::{fx:?}", element_name(*e)));
            }
        }
    }
    for f in fields {
        let hs = homes.get(f).map(Vec::as_slice).unwrap_or(&[]);
        assert_eq!(
            hs.len(),
            1,
            "model field `{f}` must have exactly one census home in the grid, found {hs:?}"
        );
    }
}

#[test]
fn provenance_words_are_substantive_and_sourced_citations_anchor_in_the_dossier() {
    let dossier = std::fs::read_to_string(DOSSIER)
        .unwrap_or_else(|e| panic!("dossier {DOSSIER} unreadable: {e}"));
    for e in ALL_ELEMENTS {
        let row = effect_row(*e);
        let name = element_name(*e);
        for fx in ALL_EFFECTS {
            match row.slot(*fx) {
                EffectSlot::Sourced { citation, .. } => {
                    assert!(citation.len() >= 30, "`{name}` {fx:?}: citation is a stub");
                    let anchor = citation.split_whitespace().next().unwrap();
                    assert!(
                        dossier.contains(anchor),
                        "`{name}` {fx:?}: citation anchor `{anchor}` not found in the dossier \
                         — a Sourced figure must live in benchmarks/silicon/analog/sourced-constants.md"
                    );
                }
                EffectSlot::Derived { arithmetic, .. } => {
                    assert!(
                        arithmetic.len() >= 30,
                        "`{name}` {fx:?}: arithmetic is a stub"
                    );
                }
                EffectSlot::Modeled { note, .. } => {
                    assert!(note.len() >= 15, "`{name}` {fx:?}: Modeled note is a stub");
                }
                EffectSlot::Ideal { justification } => {
                    assert!(
                        justification.len() >= 30,
                        "`{name}` {fx:?}: an Ideal claim needs a real physics justification"
                    );
                }
                EffectSlot::Unknown => {}
            }
        }
    }
}

#[test]
fn every_budget_term_classifies_into_the_taxonomy() {
    // Variant census: one representative per ErrorSource variant. The match below has NO
    // wildcard, so a new variant breaks this lock at compile time until classified here
    // AND in effect_of.
    let representatives = [
        ErrorSource::DacQuant { input: 0, bits: 8 },
        ErrorSource::AdcQuant { bits: 8 },
        ErrorSource::WeightQuant,
        ErrorSource::MulInteraction,
        ErrorSource::EvalRounding,
        ErrorSource::TileGain,
        ErrorSource::TileOffset,
        ErrorSource::Noise,
        ErrorSource::ThermalGain,
        ErrorSource::ThermalOffset,
        ErrorSource::CompareBand,
        ErrorSource::ChopResidual,
    ];
    for r in &representatives {
        match r {
            ErrorSource::DacQuant { .. }
            | ErrorSource::AdcQuant { .. }
            | ErrorSource::WeightQuant
            | ErrorSource::MulInteraction
            | ErrorSource::EvalRounding
            | ErrorSource::TileGain
            | ErrorSource::TileOffset
            | ErrorSource::Noise
            | ErrorSource::ThermalGain
            | ErrorSource::ThermalOffset
            | ErrorSource::CompareBand
            | ErrorSource::ChopResidual => {}
        }
    }

    // A physics effect cited by a budget term must be classified (non-Unknown) on at least
    // one mapped/interface element's row — the budget never cites physics the grid shrugs at.
    let classified_on_mapped = |fx: PhysicsEffect| {
        ALL_ELEMENTS.iter().any(|e| {
            matches!(
                role(*e),
                ElementRole::ComputeMapped(_) | ElementRole::Interface(_)
            ) && !matches!(effect_row(*e).slot(fx), EffectSlot::Unknown)
        })
    };
    let mut n_physics = 0;
    let mut n_analysis = 0;
    for r in &representatives {
        match effect_of(r) {
            BudgetTermClass::Physics(fxs) => {
                n_physics += 1;
                assert!(
                    !fxs.is_empty(),
                    "{r:?}: a physics term aggregates at least one effect"
                );
                for fx in fxs {
                    assert!(
                        classified_on_mapped(*fx),
                        "{r:?} cites {fx:?}, but no mapped/interface element classifies it"
                    );
                }
            }
            BudgetTermClass::Analysis(reason) => {
                n_analysis += 1;
                assert!(
                    reason.len() >= 30,
                    "{r:?}: analysis terms carry their reason"
                );
            }
        }
    }
    assert_eq!(
        n_physics, 10,
        "the physics/analysis split moved — conscious edit"
    );
    assert_eq!(n_analysis, 2);
}
