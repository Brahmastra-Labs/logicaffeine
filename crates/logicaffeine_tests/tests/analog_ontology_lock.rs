//! ★ ANALOG ONTOLOGY LOCK ★ — the typed universe of analog circuit elements, census-complete
//! and classification-total, with monotone ledgers.
//!
//! What this file locks:
//!   1. CENSUS — `ALL_ELEMENTS` / `ALL_COMPUTE_OPS` are duplicate-free and name-distinct; a
//!      variant missing from a census const cannot hide (the classifying fns are exhaustive
//!      matches, so ADDING a variant breaks compilation until classified — the pit of success).
//!   2. LEDGERS — `KNOWN_COMPUTE_MAPPED` (grow-only: growth = a witnessed lowering, shrinkage
//!      = a regression), `KNOWN_UNREALIZED` (shrink-only: removal = a realization, celebrated),
//!      `KNOWN_PLANNED` ops. Every element appears in exactly the ledger its `role` claims.
//!   3. AGREEMENT WITH THE EMITTER — the realized op set is bijective with the ADL's tile
//!      kinds via `cell_kind_op`; every realized op's realizers are themselves ComputeMapped
//!      or Interface with that op in their role; the float corpus's emitted cells round-trip
//!      into mapped realizers (vacuousness-guarded).
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. Moving an element between ledgers is a conscious
//! edit here AND in `ontology::role`.

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_ana::ontology::{
    cell_kind_op, element_name, realizing_elements, role, ComputeOp, ElementRole, ALL_COMPUTE_OPS,
    ALL_ELEMENTS, REALIZED_OPS,
};
use logicaffeine_compile::codegen_ana::{
    analog_netlist_from_source, AnalogTarget, ALL_ANALOG_CELL_KINDS,
};

/// GROW-ONLY: the elements with a witnessed realization today (macros of the V1 chip + their
/// physical mechanisms).
const KNOWN_COMPUTE_MAPPED: &[&str] = &[
    "Passive::Capacitor",
    "Macro::AnalogSelect",
    "Macro::TranslinearLoop",
    "Macro::DiffPair",
    "Semi::Mosfet(SubthresholdExp)",
    "Semi::Bjt(ForwardActive)",
    "Semi::Diode(ForwardExp)",
    "Source::Vccs",
    "Macro::LinearCombTile",
    "Macro::GilbertMultiplier",
    "Macro::OtaIntegrator",
    "Macro::Comparator",
    "Macro::SampleHold",
];

/// SHRINK-ONLY: recognized, no realization yet. Removing an entry (an element found its
/// lowering) is the celebrated conscious edit.
const KNOWN_UNREALIZED: &[&str] = &[
    "Passive::Resistor",
    "Passive::Inductor",
    "Passive::MutualInductor",
    "Passive::TransmissionLine",
    "Passive::Varactor",
    "Passive::Memristor",
    "Source::Vcvs",
    "Source::Cccs",
    "Source::Ccvs",
    "Semi::Diode(Reverse)",
    "Semi::Diode(Breakdown)",
    "Semi::Bjt(Saturation)",
    "Semi::Bjt(Cutoff)",
    "Semi::Bjt(ReverseActive)",
    "Semi::Mosfet(SaturationSquareLaw)",
    "Semi::Mosfet(Triode)",
    "Semi::Mosfet(Cutoff)",
    "Semi::Jfet",
    "Semi::PhotoDiode",
    "Macro::WinnerTakeAll",
    "Macro::Chopper",
    "Macro::FilterBiquad",
    "Macro::Oscillator",
    "Macro::Vco",
    "Macro::Pll",
    "Macro::Lna",
    "Macro::Pga",
];

/// The named-future compute ops, each owned by a campaign phase (O5/O6/R3/O7).
const KNOWN_PLANNED_OPS: &[ComputeOp] = &[ComputeOp::ArgMax];

#[test]
fn the_census_is_duplicate_free_and_name_distinct() {
    let names: Vec<String> = ALL_ELEMENTS.iter().map(|e| element_name(*e)).collect();
    let set: BTreeSet<&String> = names.iter().collect();
    assert_eq!(set.len(), names.len(), "duplicate element in ALL_ELEMENTS");
    assert_eq!(
        ALL_ELEMENTS.len(),
        55,
        "the element census moved — a conscious edit updates this pin"
    );

    let ops: BTreeSet<ComputeOp> = ALL_COMPUTE_OPS.iter().copied().collect();
    assert_eq!(
        ops.len(),
        ALL_COMPUTE_OPS.len(),
        "duplicate op in ALL_COMPUTE_OPS"
    );
    assert_eq!(ALL_COMPUTE_OPS.len(), 15);
}

#[test]
fn every_element_sits_in_exactly_the_ledger_its_role_claims() {
    let mapped: BTreeSet<&str> = KNOWN_COMPUTE_MAPPED.iter().copied().collect();
    let unrealized: BTreeSet<&str> = KNOWN_UNREALIZED.iter().copied().collect();
    assert_eq!(
        mapped.len(),
        KNOWN_COMPUTE_MAPPED.len(),
        "duplicate in KNOWN_COMPUTE_MAPPED"
    );
    assert_eq!(
        unrealized.len(),
        KNOWN_UNREALIZED.len(),
        "duplicate in KNOWN_UNREALIZED"
    );
    assert!(mapped.is_disjoint(&unrealized));

    let mut derived_mapped = BTreeSet::new();
    let mut derived_unrealized = BTreeSet::new();
    for e in ALL_ELEMENTS {
        let name = element_name(*e);
        match role(*e) {
            ElementRole::ComputeMapped(ops) => {
                assert!(!ops.is_empty(), "`{name}`: a mapped element names its ops");
                for op in ops {
                    assert!(ALL_COMPUTE_OPS.contains(op), "`{name}`: unknown op {op:?}");
                }
                derived_mapped.insert(name);
            }
            ElementRole::Unrealized => {
                derived_unrealized.insert(name);
            }
            ElementRole::Support(_) | ElementRole::Interface(_) => {}
        }
    }
    let derived_mapped_refs: BTreeSet<&str> = derived_mapped.iter().map(|s| s.as_str()).collect();
    let derived_unrealized_refs: BTreeSet<&str> =
        derived_unrealized.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        derived_mapped_refs, mapped,
        "KNOWN_COMPUTE_MAPPED must equal the role-derived set exactly (grow-only — a new \
         realization is a conscious, celebrated edit in both places)"
    );
    assert_eq!(
        derived_unrealized_refs, unrealized,
        "KNOWN_UNREALIZED must equal the role-derived set exactly (shrink-only)"
    );
}

#[test]
fn the_realized_ops_are_bijective_with_the_tile_adl_and_realizers_agree() {
    // Bijection with the ADL's tile kinds.
    let via_kinds: BTreeSet<ComputeOp> = ALL_ANALOG_CELL_KINDS
        .iter()
        .map(|k| cell_kind_op(*k))
        .collect();
    assert_eq!(
        via_kinds.len(),
        ALL_ANALOG_CELL_KINDS.len(),
        "cell_kind_op must be injective"
    );
    let realized: BTreeSet<ComputeOp> = REALIZED_OPS.iter().copied().collect();
    assert_eq!(
        via_kinds, realized,
        "REALIZED_OPS ≡ the ADL's tile kinds, exactly"
    );

    // Realized ⊎ planned = the whole vocabulary.
    let planned: BTreeSet<ComputeOp> = KNOWN_PLANNED_OPS.iter().copied().collect();
    assert!(realized.is_disjoint(&planned));
    assert_eq!(realized.len() + planned.len(), ALL_COMPUTE_OPS.len());

    // Every op has realizers; realized ops' realizers are mapped/interface WITH the op listed.
    for op in ALL_COMPUTE_OPS {
        let realizers = realizing_elements(*op);
        assert!(
            !realizers.is_empty(),
            "{op:?} must name its realizing elements"
        );
        if realized.contains(op) {
            for r in realizers {
                match role(*r) {
                    ElementRole::ComputeMapped(ops) => assert!(
                        ops.contains(op),
                        "{:?} realizes {op:?} but its role omits it",
                        element_name(*r)
                    ),
                    ElementRole::Interface(i) => assert_eq!(i, *op),
                    other => panic!(
                        "{:?} realizes REALIZED op {op:?} but is {other:?} — a realized op \
                         cannot rest on an unrealized element",
                        element_name(*r)
                    ),
                }
            }
        } else {
            // A planned op must have at least one realizer that is NOT yet mapped — the
            // missing piece its lowering phase delivers. Already-mapped constituents are fine
            // (Select's comparator half exists today; its switch half does not).
            assert!(
                realizers.iter().any(|r| !matches!(
                    role(*r),
                    ElementRole::ComputeMapped(_) | ElementRole::Interface(_)
                )),
                "{op:?} is planned but every realizer is already mapped — promote the op \
                 (a conscious REALIZED_OPS + ledger edit) or name the missing element"
            );
        }
    }
}

#[test]
fn the_emitters_cells_round_trip_into_mapped_realizers() {
    let target = AnalogTarget::ideal();
    let src = "## To dot3 (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return a * 0.25 + b * 0.5 + c * 0.125.\n\n## Main\n    Show dot3(0.5, 0.25, 0.75).\n";
    let netlist = analog_netlist_from_source(src, "dot3", &target).expect("corpus lowers");
    assert!(!netlist.cells.is_empty(), "vacuousness guard");
    let mapped: BTreeSet<&str> = KNOWN_COMPUTE_MAPPED.iter().copied().collect();
    for cell in &netlist.cells {
        let op = cell_kind_op(cell.kind());
        for r in realizing_elements(op) {
            if matches!(role(*r), ElementRole::ComputeMapped(_)) {
                let name = element_name(*r);
                assert!(
                    mapped.contains(name.as_str()),
                    "emitted {op:?}'s realizer `{name}` missing from KNOWN_COMPUTE_MAPPED"
                );
            }
        }
    }
}
