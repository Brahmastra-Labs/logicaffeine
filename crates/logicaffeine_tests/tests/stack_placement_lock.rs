//! ★ STACK PLACEMENT LOCK ★ — 3D placement over composed analog tiers with the vertical
//! interconnect as a COUNTED, refusable resource.
//!
//! What this file locks:
//!   1. LEGALITY + DETERMINISM — every cell on a real analog-tier site, no double
//!      occupancy, per-gap `vertical_used` never exceeds the gap's budget, and the same
//!      netlist places identically forever.
//!   2. THE DEGENERATE EMBED — on a single-tier stack, 3D placement IS the 2D placer's
//!      answer, site for site (the additive-extension covenant again, as an equation).
//!   3. SCARCITY REFUSES TYPED — a gap with too few vias yields
//!      `VerticalResourceExhausted`, never a silently-overcommitted layout; a stack with
//!      fewer sites than cells yields `StackTooSmall`.
//!   4. THE SEPARATION LAW — `leg_sigma` (2D) is EXACTLY the shared law at
//!      `Separation::SameTier`, and `CrossTier` is a flat categorical ceiling (separately
//!      manufactured dies do not share a Pelgrom gradient) that is WORSE than any plausible
//!      same-tier distance on the V1 arrays.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. Multi-tier support was S2's conscious growth;
//! the scarcity ledger and the categorical cross-tier penalty are the physics.

use logicaffeine_compile::codegen_ana::nonideal::{leg_sigma, separation_sigma, Separation};
use logicaffeine_compile::codegen_ana::stack::{
    CoolingBoundary, Material, StackTarget, Tier, TierFabric, VerticalClass,
};
use logicaffeine_compile::codegen_ana::stack_place::place_stack;
use logicaffeine_compile::codegen_ana::{
    analog_netlist_from_source, place_analog, sky130_nonideal, AnaLowerError, AnaUnplaceable,
    AnalogTarget,
};

/// Deep enough that a 2×2 tier cannot hold it alone — the chain must span tiers.
const CHAIN: &str = "## To chain (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return ((a * b) * 0.5 + (b * c) * 0.25) * 0.5 + ((a * c) * 0.125 + a * 0.0625) * 0.25.\n\n## Main\n    Show chain(0.5, 0.25, 0.75).\n";

const HOP_COST: f64 = 4.0;

fn tiny_tier() -> AnalogTarget {
    let mut t = AnalogTarget::ideal();
    t.rows = 3;
    t.cols = 3;
    t
}

fn two_tier(gap_tsvs: usize) -> StackTarget {
    let tier = |name: &'static str| Tier {
        name,
        fabric: TierFabric::Analog(tiny_tier()),
        material: Material::silicon(),
        thickness_um: 100.0,
    };
    StackTarget {
        name: "two-tier-test",
        tiers: vec![tier("analog-lo"), tier("analog-hi")],
        gaps: vec![vec![VerticalClass::tsv_v1(gap_tsvs)]],
        cooling: CoolingBoundary::forced_air_default(),
    }
}

#[test]
fn placement_is_legal_deterministic_and_ledgered() {
    let netlist = analog_netlist_from_source(CHAIN, "chain", &AnalogTarget::ideal()).unwrap();
    assert!(
        netlist.cells.len() > 9,
        "vacuity guard: the chain must overflow one 3×3 tier"
    );
    let stack = two_tier(64);
    let p = place_stack(&netlist, &stack, HOP_COST).expect("two tiers hold the chain");
    assert_eq!(p.site.len(), netlist.cells.len());

    let mut seen = std::collections::BTreeSet::new();
    for s in &p.site {
        assert!(s.tier < 2 && s.row < 3 && s.col < 3, "a real site: {s:?}");
        assert!(
            seen.insert((s.tier, s.row, s.col)),
            "double occupancy at {s:?}"
        );
    }
    assert_eq!(p.vertical_used.len(), 1, "one ledger entry per gap");
    assert!(
        p.vertical_used[0] <= 64,
        "the ledger never exceeds the budget"
    );
    assert!(
        p.vertical_used[0] > 0,
        "a spanning chain must actually cross the gap"
    );

    let again = place_stack(&netlist, &stack, HOP_COST).expect("deterministic");
    assert_eq!(again, p, "the same netlist places identically forever");
}

#[test]
fn the_single_tier_stack_is_the_2d_placer_site_for_site() {
    let target = AnalogTarget::ideal();
    let netlist = analog_netlist_from_source(CHAIN, "chain", &target).unwrap();
    let flat = place_analog(&netlist, &target).unwrap();
    let stacked =
        place_stack(&netlist, &StackTarget::single(target), HOP_COST).expect("degenerate");
    for (i, s) in stacked.site.iter().enumerate() {
        assert_eq!(s.tier, 0);
        assert_eq!((s.row, s.col), flat.site[i], "cell {i}: 3D degenerate ≡ 2D");
    }
    assert!(stacked.vertical_used.is_empty(), "no gaps, no ledger");
}

#[test]
fn scarcity_and_capacity_refuse_typed() {
    let netlist = analog_netlist_from_source(CHAIN, "chain", &AnalogTarget::ideal()).unwrap();
    // The chain spans tiers, but the gap offers no vias at all: a stack whose gap budget is
    // consumed must refuse by name. (Zero-count classes are a SHAPE error, so use count 1
    // against a chain needing several crossings.)
    match place_stack(&netlist, &two_tier(1), HOP_COST) {
        Err(AnaLowerError::Unplaceable(AnaUnplaceable::VerticalResourceExhausted {
            gap,
            budget,
        })) => {
            assert_eq!(gap, 0);
            assert_eq!(budget, 1);
        }
        other => panic!("1 via cannot carry a spanning chain — must refuse typed, got {other:?}"),
    }
    // Fewer sites than cells refuses too.
    let mut small = two_tier(64);
    small.tiers.pop();
    small.gaps.clear();
    match place_stack(&netlist, &small, HOP_COST) {
        Err(AnaLowerError::Unplaceable(AnaUnplaceable::StackTooSmall {
            needed_cells,
            sites,
            ..
        })) => {
            assert_eq!(needed_cells, netlist.cells.len());
            assert_eq!(sites, 9);
        }
        other => panic!("9 sites cannot hold the chain, got {other:?}"),
    }
}

#[test]
fn the_separation_law_subsumes_2d_and_makes_cross_tier_categorical() {
    let model = sky130_nonideal();
    let target = AnalogTarget::ideal();
    let netlist = analog_netlist_from_source(CHAIN, "chain", &target).unwrap();
    let flat = place_analog(&netlist, &target).unwrap();
    // 2D leg_sigma IS the shared law at SameTier — every existing budget is unchanged.
    for (i, cell) in netlist.cells.iter().enumerate() {
        let legs = match cell {
            logicaffeine_compile::codegen_ana::AnalogCell::Sum { inputs, .. } => inputs.len(),
            _ => 1,
        };
        for l in 0..legs {
            let via_law = separation_sigma(
                &model,
                Separation::SameTier {
                    pitches: {
                        use logicaffeine_compile::codegen_ana::{ASrc, AnalogCell};
                        let operand = match &netlist.cells[i] {
                            AnalogCell::Sum { inputs, .. } => inputs[l],
                            AnalogCell::GainConst { a, .. } | AnalogCell::Mul { a, .. } => *a,
                            AnalogCell::Select { .. }
                            | AnalogCell::Exp { .. }
                            | AnalogCell::Log { .. }
                            | AnalogCell::Tanh { .. }
                            | AnalogCell::Sqrt { .. }
                            | AnalogCell::Div { .. } => continue,
                        };
                        match operand {
                            ASrc::Val(v) => flat.pitch_distance(i, v),
                            _ => 0.0,
                        }
                    },
                },
            );
            assert_eq!(
                via_law,
                leg_sigma(i, l, &netlist.cells, &flat, &model),
                "cell {i} leg {l}: the 2D sigma must BE the shared law"
            );
        }
    }
    // Cross-tier is CATEGORICAL: a flat class figure, not a gradient — strictly worse than
    // any matched-cluster-scale same-tier separation (the placer keeps operand clusters
    // within a few pitches; 5 is already a loose cluster), and INDEPENDENT of distance.
    let cluster_scale = separation_sigma(&model, Separation::SameTier { pitches: 5.0 });
    let cross = separation_sigma(&model, Separation::CrossTier);
    assert!(
        cross > cluster_scale,
        "separately manufactured dies must beat any matched cluster: cross {cross} vs 5-pitch \
         same-tier {cluster_scale}"
    );
}
