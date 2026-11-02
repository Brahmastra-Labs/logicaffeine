use super::*;
use crate::codegen_sva::rtl::parse_transition_system;
use logicaffeine_proof::bmc::{BmcOutcome, InductionOutcome};

#[test]
fn congested_approach_jams_and_bmc_finds_the_cycle() {
    let ts = parse_transition_system(&congested_approach(3, 7))
        .unwrap_or_else(|e| panic!("congested model did not parse: {}", e.message));
    match ts.bmc(24) {
        BmcOutcome::CounterexampleAt { .. } => {}
        other => panic!("an under-served approach must jam, got {other:?}"),
    }
}

#[test]
fn balanced_approach_provably_never_jams() {
    let ts = parse_transition_system(&balanced_approach(3, 7, 5))
        .unwrap_or_else(|e| panic!("balanced model did not parse: {}", e.message));
    assert_eq!(
        ts.prove_invariant(1),
        InductionOutcome::Proven,
        "a draining queue must be PROVEN jam-free"
    );
}

#[test]
fn a_well_sized_queue_does_not_overflow_within_the_horizon() {
    // The same congestion against a larger capacity survives longer before jamming — the
    // sizing lever. Within a short horizon it stays clean.
    let ts = parse_transition_system(&congested_approach(4, 15)).unwrap();
    assert_eq!(
        ts.bmc(8),
        BmcOutcome::NoneWithin(8),
        "a 4-bit queue should not jam within 8 cycles"
    );
}

#[test]
fn jam_is_pushed_later_by_more_capacity() {
    // Wider queue (bigger jam threshold) jams strictly later than a narrow one.
    let narrow = parse_transition_system(&congested_approach(3, 7)).unwrap();
    let wide = parse_transition_system(&congested_approach(4, 15)).unwrap();
    let cycle = |o: BmcOutcome| match o {
        BmcOutcome::CounterexampleAt { k, .. } => k,
        _ => u32::MAX,
    };
    let narrow_k = cycle(narrow.bmc(40));
    let wide_k = cycle(wide.bmc(40));
    assert!(
        narrow_k < wide_k,
        "more capacity must delay the jam: narrow@{narrow_k} vs wide@{wide_k}"
    );
}
