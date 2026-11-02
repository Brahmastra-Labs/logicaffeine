//! ═══════════════════════════════════════════════════════════════════════
//! PLASTICINE PLACEMENT LOCK — P3 of the Plasticine lane: a metapipeline is allocated to PCUs/PMUs, placed on
//! the checkerboard, and statically routed — with the numbers an RDU engineer reads.
//!
//! A. A stage of depth D chains ceil(D / 6) PCUs (1..6 → 1, 7 → 2, 13 → 3); the last PCU's used stages are
//!    exact; FU utilization = cells / (PCUs × 6).
//! B. A buffer edge is N-buffered (writer→reader rule) inside one PMU while it fits, else spans PMUs; a tile
//!    the mesh cannot hold is refused, typed (`TooSmall` names PCUs/PMUs needed vs available).
//! C. Every placed unit sits on a cell of its kind (PCUs on even cells, PMUs on odd), no two units share a
//!    cell, a 64-stage pipeline fills the ISCA'17 mesh and a 65th stage is refused typed; the SARA 20×20
//!    mesh takes what the 16×8 cannot.
//! D. Routing is dimension-order: hop counts equal Manhattan distance via the PMU; link occupancy is counted
//!    per mesh link and compared to the Spade budget (2 vector links) — `routed_statically` flips when a
//!    link is oversubscribed.
//! E. The placement is a function of (pipeline, target, budget): identical inputs ⇒ identical placement.
//!
//! Honest boundary: MODELED placement/routing over published parameters; not plastiroute, not Tungsten.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(plasticine_place_lock)'`
//! ═══════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::fabric_target::Unplaceable;
use logicaffeine_compile::codegen_sva::metapipeline::{Metapipeline, Stage};
use logicaffeine_compile::codegen_sva::plasticine::PlasticineTarget;
use logicaffeine_compile::codegen_sva::plasticine_place::{
    allocate, lanes_need_banks, place, place_metapipeline, LinkBudget, Unit,
};

fn stage(i: usize, depth: usize, cells: usize) -> Stage {
    Stage {
        name: format!("s{i}"),
        resource_ii: 1,
        recurrence_ii: 1,
        latency: depth,
        cells,
        memory_ii: 1,
    }
}

fn chain(depths: &[usize]) -> Metapipeline {
    Metapipeline {
        stages: depths
            .iter()
            .enumerate()
            .map(|(i, &d)| stage(i, d, d))
            .collect(),
        edges: (0..depths.len().saturating_sub(1))
            .map(|i| (i, i + 1))
            .collect(),
    }
}

#[test]
fn a_stage_deeper_than_a_pcu_chains_pcus_and_fu_utilization_is_cells_over_slots() {
    let t = PlasticineTarget::isca17();
    for (depth, pcus, last) in [
        (1usize, 1usize, 1usize),
        (6, 1, 6),
        (7, 2, 1),
        (12, 2, 6),
        (13, 3, 1),
    ] {
        let mp = chain(&[depth]);
        let a = allocate(&mp, &[], &t).unwrap();
        assert_eq!(a.stages[0].pcus, pcus, "depth {depth}");
        assert_eq!(a.stages[0].stages_in_last, last, "depth {depth}");
        let p = place(&a, &t, LinkBudget::spade_default()).unwrap();
        let fu = p.fu_utilization(t.pcu_stages);
        assert!(
            (fu - depth as f64 / (pcus * 6) as f64).abs() < 1e-12,
            "FU utilization for depth {depth}: {fu}"
        );
    }
    // A pipeline deeper than the PCU's stage count is refused before allocation (the Metapipeline gate).
    let seven = Metapipeline {
        stages: (0..7).map(|i| stage(i, 1, 1)).collect(),
        edges: vec![],
    };
    assert!(matches!(
        allocate(&seven, &[], &t),
        Err(Unplaceable::MissingCapability { .. })
    ));
}

#[test]
fn buffers_are_n_buffered_inside_a_pmu_or_span_pmus_and_oversize_is_refused() {
    let t = PlasticineTarget::isca17();
    let words_per_pmu = 256 * 1024 / 4; // 65,536 words
    let mp = chain(&[1, 1]); // one edge (0,1): depth 2
    let a = allocate(&mp, &[words_per_pmu / 2], &t).unwrap();
    assert_eq!(a.buffers[0].depth, 2);
    assert_eq!(
        a.buffers[0].pmus, 1,
        "half a PMU double-buffered fits one PMU"
    );
    let a = allocate(&mp, &[words_per_pmu / 2 + 1], &t).unwrap();
    assert_eq!(
        a.buffers[0].pmus, 2,
        "one word more than a double-buffered PMU spans two PMUs"
    );
    let a = allocate(&mp, &[0], &t).unwrap();
    assert_eq!(
        a.buffers[0].pmus, 1,
        "a handshake-only edge still needs a PMU"
    );
    let three = Metapipeline {
        stages: (0..3).map(|i| stage(i, 1, 1)).collect(),
        edges: vec![(0, 2)],
    };
    let a = allocate(&three, &[words_per_pmu / 3], &t).unwrap();
    assert_eq!(a.buffers[0].depth, 3);
    assert_eq!(a.buffers[0].pmus, 1);
    // 65 PMUs' worth of buffer on a 64-PMU mesh is refused with the counts named.
    match allocate(&mp, &[words_per_pmu / 2 * 65], &t) {
        Err(Unplaceable::TooSmall {
            needed_cols,
            target_cols,
            target,
            ..
        }) => {
            assert_eq!(
                (needed_cols, target_cols, target),
                (65, 64, "plasticine-isca17")
            );
        }
        other => panic!("an oversize buffer must be refused typed, got {other:?}"),
    }
}

#[test]
fn units_land_on_cells_of_their_kind_and_the_mesh_fills_exactly() {
    let t = PlasticineTarget::isca17();
    // Six stages of depth 60 chain 10 PCUs each: 60 of the mesh's 64 PCUs.
    let fill = Metapipeline {
        stages: vec![
            stage(0, 60, 60),
            stage(1, 60, 60),
            stage(2, 60, 60),
            stage(3, 60, 60),
            stage(4, 60, 60),
            stage(5, 60, 60),
        ],
        edges: vec![],
    };
    let a = allocate(&fill, &[], &t).unwrap();
    assert_eq!(a.pcus(), 60);
    let p = place(&a, &t, LinkBudget::spade_default()).unwrap();
    assert_eq!(p.pcu_total, 64);
    assert_eq!(p.pmu_total, 64);
    assert!((p.pcu_utilization() - 60.0 / 64.0).abs() < 1e-12);
    let mut cells = std::collections::HashSet::new();
    for u in &p.placed {
        assert!(cells.insert((u.row, u.col)), "two units on one cell");
        match u.unit {
            Unit::Pcu { .. } => assert_eq!((u.row + u.col) % 2, 0, "PCUs on even cells"),
            Unit::Pmu { .. } => assert_eq!((u.row + u.col) % 2, 1, "PMUs on odd cells"),
        }
        assert!(u.row < t.pcu_rows && u.col < t.pcu_cols);
    }
    // 65 PCUs do not fit the 16x8 mesh (five depth-60 stages = 50 PCUs, plus a depth-90 stage = 15).
    let too_many = Metapipeline {
        stages: vec![
            stage(0, 60, 60),
            stage(1, 60, 60),
            stage(2, 60, 60),
            stage(3, 60, 60),
            stage(4, 60, 60),
            stage(5, 90, 90),
        ],
        edges: vec![],
    };
    match allocate(&too_many, &[], &t) {
        Err(Unplaceable::TooSmall {
            needed_rows,
            target_rows,
            ..
        }) => assert_eq!((needed_rows, target_rows), (65, 64)),
        other => panic!("65 PCUs must be refused typed, got {other:?}"),
    }
    let sixty_four = Metapipeline {
        stages: vec![
            stage(0, 60, 60),
            stage(1, 60, 60),
            stage(2, 60, 60),
            stage(3, 60, 60),
            stage(4, 60, 60),
            stage(5, 84, 84),
        ],
        edges: vec![],
    };
    assert_eq!(
        allocate(&sixty_four, &[], &t).unwrap().pcus(),
        64,
        "exactly the mesh fills"
    );
    let huge = Metapipeline {
        stages: (0..6).map(|i| stage(i, 66, 66)).collect(),
        edges: vec![],
    };
    assert!(
        matches!(allocate(&huge, &[], &t), Err(Unplaceable::TooSmall { .. })),
        "66 PCUs exceed the 16x8 mesh"
    );
    let big = PlasticineTarget::sara_20x20();
    assert_eq!(
        allocate(&huge, &[], &big).unwrap().pcus(),
        66,
        "the 20x20 mesh (200 PCUs) takes them"
    );
}

#[test]
fn routing_is_dimension_order_with_hop_counts_and_link_occupancy_against_the_budget() {
    let t = PlasticineTarget::isca17();
    let mp = chain(&[1, 1]);
    let p = place_metapipeline(&mp, &[16], &t, LinkBudget::spade_default()).unwrap();
    assert_eq!(p.routes.len(), 1);
    let r = &p.routes[0];
    // PCU0 at (0,0), PMU nearest to both at (0,1), PCU1 at (0,2): 1 + 1 hops along a row.
    let pcu0 = p
        .placed
        .iter()
        .find(|u| matches!(u.unit, Unit::Pcu { stage: 0, .. }))
        .unwrap();
    let pcu1 = p
        .placed
        .iter()
        .find(|u| matches!(u.unit, Unit::Pcu { stage: 1, .. }))
        .unwrap();
    let pmu = p
        .placed
        .iter()
        .find(|u| matches!(u.unit, Unit::Pmu { .. }))
        .unwrap();
    assert_eq!((pcu0.row, pcu0.col), (0, 0));
    assert_eq!((pcu1.row, pcu1.col), (0, 2));
    assert_eq!((pmu.row, pmu.col), (0, 1));
    assert_eq!(r.hops, 2);
    assert_eq!(p.total_hops(), 2);
    assert_eq!(p.max_hops(), 2);
    assert_eq!(p.max_link_occupancy, 1);
    assert!(p.routed_statically);
    // Many edges through one corridor oversubscribe a link: a fan-in of five producers into one consumer
    // (six stages — the PCU's metapipeline depth; a seventh stage would be refused by the depth gate).
    let fan = Metapipeline {
        stages: (0..6).map(|i| stage(i, 1, 1)).collect(),
        edges: (0..5).map(|i| (i, 5)).collect(),
    };
    let p = place_metapipeline(&fan, &[16; 5], &t, LinkBudget::spade_default()).unwrap();
    assert!(
        p.max_link_occupancy > 2,
        "five routes converging on one PCU share links beyond the 2-vector-link budget: {}",
        p.max_link_occupancy
    );
    assert!(
        !p.routed_statically,
        "the report says the static routing did not fit"
    );
    let generous = place_metapipeline(
        &fan,
        &[16; 5],
        &t,
        LinkBudget {
            vector_links: 16,
            scalar_links: 4,
        },
    )
    .unwrap();
    assert!(generous.routed_statically, "a wider link budget routes it");
    let nine = Metapipeline {
        stages: (0..9).map(|i| stage(i, 1, 1)).collect(),
        edges: vec![],
    };
    assert!(
        matches!(
            place_metapipeline(&nine, &[], &t, LinkBudget::spade_default()),
            Err(Unplaceable::MissingCapability { .. })
        ),
        "nine stages exceed the PCU depth"
    );
    lanes_need_banks(&t).expect("16 lanes need 16 banks: the ISCA'17 PMU has them");
}

#[test]
fn the_placement_is_a_function_of_pipeline_target_and_budget() {
    let t = PlasticineTarget::isca17();
    let mp = chain(&[3, 7, 2]);
    let a = place_metapipeline(&mp, &[64, 128], &t, LinkBudget::spade_default()).unwrap();
    let b = place_metapipeline(&mp, &[64, 128], &t, LinkBudget::spade_default()).unwrap();
    assert_eq!(a, b, "deterministic");
    let c = place_metapipeline(
        &mp,
        &[64, 128],
        &PlasticineTarget::sara_20x20(),
        LinkBudget::spade_default(),
    )
    .unwrap();
    assert_ne!(
        a.pcu_total, c.pcu_total,
        "a different mesh is a different placement"
    );
    assert_eq!(a.pcu_used(), 1 + 2 + 1);
    assert_eq!(a.pmu_used(), 2);
    let j = serde_json::to_string(&a).unwrap();
    assert!(
        j.contains("\"routed_statically\"") && j.contains("\"max_link_occupancy\""),
        "the placement serializes for the report"
    );
}
