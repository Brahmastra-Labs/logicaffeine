//! ★ STACK TARGET LOCK ★ — the constructive 2.5D/3D stack ADL: composed tiers, scarce
//! counted vertical resources, provenance-statused physics.
//!
//! What this file locks:
//!   1. THE V1 STACK — `sky130_stack_v1` is pinned: the sourced-power digital tier sits
//!      BELOW the analog tier (deliberately in its heat path), the digital power figure and
//!      its provenance are carried VERBATIM from `benchmarks/silicon/sky130_power.json`,
//!      TSVs and microbumps are counted scarce resources with dossier-anchored statuses.
//!   2. PROVENANCE DISCIPLINE — every Sourced/Derived/Modeled string is substantive, the
//!      dossier file exists, and the sourced conductivities anchor in it.
//!   3. SHAPE COHERENCE — malformed stacks refuse with a NAME (gap-count mismatch, phantom
//!      zero-count vertical class, more analog tiers than this phase carries).
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. A figure never loses its provenance; a scarce
//! count never becomes uncounted; the heat-path layout is a conscious design statement.

use logicaffeine_compile::codegen_ana::stack::{
    CoolingClass, FigureStatus, Material, StackShapeError, StackTarget, TierFabric, VerticalClass,
    VerticalKind,
};
use logicaffeine_compile::codegen_ana::AnalogTarget;

const DOSSIER: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../benchmarks/silicon/analog/thermal-constants.md"
);
const POWER_JSON: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../benchmarks/silicon/sky130_power.json"
);

#[test]
fn the_v1_stack_is_pinned_with_the_digital_tier_in_the_heat_path() {
    let s = StackTarget::sky130_stack_v1();
    assert_eq!(s.name, "sky130-stack-v1");
    s.validate().expect("the flagship preset is coherent");
    assert_eq!(s.tiers.len(), 2);
    assert_eq!(s.gaps.len(), 1);

    // Bottom-up: digital BELOW analog — the deliberate thermal statement.
    let TierFabric::Digital(d) = &s.tiers[0].fabric else {
        panic!(
            "tier 0 must be the digital heat source, got {:?}",
            s.tiers[0].fabric
        );
    };
    assert_eq!(d.design, "soft_core_scalar");
    assert!(
        d.provenance.contains("OpenLane Sky130 signoff") && d.provenance.contains("not a"),
        "the provenance must carry the signoff statement AND its honesty rider: {}",
        d.provenance
    );
    // Carried VERBATIM from the power ledger — the figure and the sentence that sources it. A re-harden
    // re-measures the ledger (scripts/record-silicon-power.py); the stack must be re-synced with it.
    let raw = std::fs::read_to_string(POWER_JSON).expect("the sourced power artifact exists");
    let json: serde_json::Value = serde_json::from_str(&raw).expect("the power artifact is JSON");
    assert_eq!(
        Some(d.power_w),
        json["power_w"]["total"].as_f64(),
        "the digital tier's watts must be the power ledger's power_w.total"
    );
    assert_eq!(
        Some(d.provenance),
        json["_provenance"].as_str(),
        "the digital tier's provenance must be the power ledger's, verbatim"
    );

    let TierFabric::Analog(a) = &s.tiers[1].fabric else {
        panic!("tier 1 must be the analog plane");
    };
    assert_eq!(
        a.name, "sky130-fpaa",
        "the analog tier IS the characterized 2D ADL"
    );

    // The gap's vertical classes: counted, scarce, statused.
    let gap = &s.gaps[0];
    let tsv = gap
        .iter()
        .find(|v| v.kind == VerticalKind::Tsv)
        .expect("TSVs present");
    assert_eq!(tsv.count, 64, "TSVs are a counted scarce resource");
    assert!(
        tsv.g_w_per_k > 0.0,
        "a TSV conducts heat — the design lever exists"
    );
    assert!(
        matches!(tsv.status, FigureStatus::Derived(_)),
        "TSV g is derived from sourced k_Cu"
    );
    let bump = gap
        .iter()
        .find(|v| v.kind == VerticalKind::MicroBump)
        .expect("bumps present");
    assert_eq!(bump.count, 256);
    assert!(matches!(bump.status, FigureStatus::Modeled(_)));

    assert_eq!(s.cooling.top, CoolingClass::ForcedAir);
    assert_eq!(s.cooling.t_junction_max_c, 85.0);
}

#[test]
fn provenance_strings_are_substantive_and_anchor_in_the_dossier() {
    let dossier = std::fs::read_to_string(DOSSIER)
        .unwrap_or_else(|e| panic!("dossier {DOSSIER} unreadable: {e}"));
    assert!(dossier.contains("148"), "silicon k anchors in the dossier");
    assert!(dossier.contains("401"), "copper k anchors in the dossier");
    assert!(
        dossier.contains("Incropera"),
        "the film-coefficient source is named"
    );

    let check = |st: &FigureStatus, what: &str| match st {
        FigureStatus::Sourced(s) | FigureStatus::Derived(s) | FigureStatus::Modeled(s) => {
            assert!(s.len() >= 25, "{what}: provenance is a stub: {s}");
        }
    };
    check(&Material::silicon().status, "silicon");
    check(&Material::underfill_class().status, "underfill");
    assert_eq!(Material::silicon().k_w_per_m_k, 148.0);
    assert!(matches!(
        Material::silicon().status,
        FigureStatus::Sourced(_)
    ));
    assert!(matches!(
        Material::underfill_class().status,
        FigureStatus::Modeled(_)
    ));
    for v in [VerticalClass::tsv_v1(1), VerticalClass::microbump_v1(1)] {
        check(&v.status, "vertical class");
    }
    // The derived TSV figure shows its arithmetic.
    let FigureStatus::Derived(d) = VerticalClass::tsv_v1(1).status else {
        unreachable!()
    };
    assert!(
        d.contains("k_Cu") && d.contains("401"),
        "derived means the arithmetic is shown: {d}"
    );
}

#[test]
fn malformed_stacks_refuse_with_a_name() {
    let mut s = StackTarget::sky130_stack_v1();
    s.gaps.clear();
    assert_eq!(
        s.validate(),
        Err(StackShapeError::GapCountMismatch { tiers: 2, gaps: 0 })
    );

    let mut phantom = StackTarget::sky130_stack_v1();
    phantom.gaps[0][0].count = 0;
    assert_eq!(
        phantom.validate(),
        Err(StackShapeError::EmptyVerticalClass { gap: 0 })
    );

    // S2's conscious growth: multiple analog tiers VALIDATE — 3D placement spans them.
    let mut twin = StackTarget::sky130_stack_v1();
    let analog_tier = twin.tiers[1].clone();
    twin.tiers.push(analog_tier);
    twin.gaps.push(vec![VerticalClass::tsv_v1(1)]);
    twin.validate()
        .expect("a two-analog-tier stack is a real 3D design, not a violation");

    let empty = StackTarget {
        name: "empty",
        tiers: vec![],
        gaps: vec![],
        cooling: logicaffeine_compile::codegen_ana::stack::CoolingBoundary::forced_air_default(),
    };
    assert_eq!(empty.validate(), Err(StackShapeError::NoTiers));

    // The degenerate single-tier embed is coherent by construction.
    StackTarget::single(AnalogTarget::ideal())
        .validate()
        .expect("single() validates");
}
