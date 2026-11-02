use super::*;

#[test]
fn seed_rng_is_deterministic() {
    let mut a = SeededRng::new(SchedSeed(42));
    let mut b = SeededRng::new(SchedSeed(42));
    let seq_a: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
    let seq_b: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();
    assert_eq!(seq_a, seq_b, "same seed must yield the same stream");

    let mut c = SeededRng::new(SchedSeed(43));
    let seq_c: Vec<u64> = (0..16).map(|_| c.next_u64()).collect();
    assert_ne!(seq_a, seq_c, "different seeds must (almost surely) differ");
}

#[test]
fn seed_below_is_in_range() {
    let mut r = SeededRng::new(SchedSeed(7));
    for _ in 0..1000 {
        assert!(r.below(5) < 5);
    }
    assert_eq!(r.below(0), 0, "below(0) is 0");
    assert_eq!(r.below(1), 0, "below(1) is 0");
}

#[test]
fn replay_roundtrip_is_bit_identical() {
    let shapes = [
        (ChoiceKind::TaskPick, 3usize),
        (ChoiceKind::SelectWinner, 2),
        (ChoiceKind::TaskPick, 4),
        (ChoiceKind::TimerTieBreak, 2),
        (ChoiceKind::WorkerPlacement, 8),
    ];
    let mut rec = Chooser::record(SchedSeed(1234));
    let recorded: Vec<usize> = shapes.iter().map(|(k, n)| rec.decide(*k, *n)).collect();
    let trace = rec.into_trace();

    let mut rep = Chooser::replay(trace.clone());
    let replayed: Vec<usize> = shapes.iter().map(|(k, n)| rep.decide(*k, *n)).collect();
    assert_eq!(recorded, replayed, "replay reproduces the recorded choices");

    // Same seed + same shapes records identically (reproducibility).
    let mut rec2 = Chooser::record(SchedSeed(1234));
    let recorded2: Vec<usize> = shapes.iter().map(|(k, n)| rec2.decide(*k, *n)).collect();
    assert_eq!(recorded, recorded2, "same seed is reproducible");
    assert_eq!(trace.seed, SchedSeed(1234));
    assert_eq!(trace.choices.len(), shapes.len());
}

#[test]
#[should_panic(expected = "replay divergence")]
fn replay_divergence_panics_on_option_mismatch() {
    let mut rec = Chooser::record(SchedSeed(9));
    rec.decide(ChoiceKind::TaskPick, 3);
    let trace = rec.into_trace();

    let mut rep = Chooser::replay(trace);
    // Feed a different option count than was recorded -> divergence.
    rep.decide(ChoiceKind::TaskPick, 5);
}

#[test]
#[should_panic(expected = "replay divergence")]
fn replay_divergence_panics_on_kind_mismatch() {
    let mut rec = Chooser::record(SchedSeed(9));
    rec.decide(ChoiceKind::TaskPick, 3);
    let trace = rec.into_trace();

    let mut rep = Chooser::replay(trace);
    rep.decide(ChoiceKind::SelectWinner, 3);
}
