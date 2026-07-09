//! Seed, deterministic RNG, and the choice trace — the determinism contract.
//!
//! Every nondeterministic scheduling decision flows through a single choke point,
//! [`Chooser::decide`]. In *record* mode it draws from a seeded RNG and logs a
//! [`ChoicePoint`]; in *replay* mode it returns the next recorded choice and
//! asserts the decision shape still matches (divergence detection). This is what
//! makes a concurrent program a deterministic function of `(program, seed)` and
//! exactly reproducible from `(program, trace)` — the property the interpreter,
//! VM, and translation validation all rely on.

/// The scheduling seed. A fixed seed makes execution fully deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SchedSeed(pub u64);

/// A small, deterministic, WASM-safe PRNG (SplitMix64).
///
/// SplitMix64 is chosen for being tiny, allocation-free, and identical on every
/// target (no platform `Rng`, no `Math.random`), so a seed reproduces bit-for-bit
/// across native and WASM.
#[derive(Debug, Clone)]
pub struct SeededRng {
    state: u64,
}

impl SeededRng {
    /// A fresh RNG from a seed.
    pub fn new(seed: SchedSeed) -> Self {
        SeededRng { state: seed.0 }
    }

    /// Next 64-bit value (SplitMix64).
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniformly-distributed index in `[0, n)`. Returns 0 when `n <= 1`.
    pub fn below(&mut self, n: usize) -> usize {
        if n <= 1 {
            return 0;
        }
        (self.next_u64() % n as u64) as usize
    }
}

/// The class of a scheduling decision — recorded so replay can detect divergence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChoiceKind {
    /// Which ready task to run next.
    TaskPick,
    /// Which ready branch of a `Select` wins.
    SelectWinner,
    /// Which blocked channel waiter to wake.
    ChanWaiterWake,
    /// Tie-break order among timers firing on the same logical tick.
    TimerTieBreak,
    /// Which worker a newly spawned task is placed on (M:N work-stealing).
    WorkerPlacement,
}

/// One recorded nondeterministic decision: its kind, how many options were
/// available, and which index was taken. This shape is intentionally
/// id-agnostic, so the trace format does not depend on `TaskId`/`ChanId`/etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChoicePoint {
    /// What kind of decision this was.
    pub kind: ChoiceKind,
    /// The number of options that were available.
    pub options: usize,
    /// The index chosen, in `[0, options)`.
    pub chosen: usize,
}

/// A full record of the scheduling decisions a run made. Replay it for an exact
/// re-execution, or compare two traces to detect a divergence.
#[derive(Debug, Clone, PartialEq)]
pub struct SchedTrace {
    /// The seed the run was recorded under.
    pub seed: SchedSeed,
    /// The decisions, in the order they were made.
    pub choices: Vec<ChoicePoint>,
}

/// The decision choke point: records (seeded) or replays scheduling choices.
///
/// THE single source of nondeterminism in the runtime — every scheduler choice
/// goes through [`Chooser::decide`]. Nothing else draws entropy.
#[derive(Debug, Clone)]
pub enum Chooser {
    /// Recording: draw from the seeded RNG and log each choice.
    Record {
        rng: SeededRng,
        seed: SchedSeed,
        choices: Vec<ChoicePoint>,
    },
    /// Replaying: re-issue the recorded choices, asserting the shapes still match.
    Replay { trace: SchedTrace, pos: usize },
}

impl Chooser {
    /// A fresh recording chooser seeded by `seed`.
    pub fn record(seed: SchedSeed) -> Self {
        Chooser::Record {
            rng: SeededRng::new(seed),
            seed,
            choices: Vec::new(),
        }
    }

    /// A replaying chooser that re-issues the decisions in `trace`.
    pub fn replay(trace: SchedTrace) -> Self {
        Chooser::Replay { trace, pos: 0 }
    }

    /// Resolve one decision among `options` choices, returning the chosen index.
    ///
    /// Record mode draws from the seeded RNG and logs the choice. Replay mode
    /// returns the recorded choice, panicking if the decision shape diverges from
    /// what was recorded (a different `kind` or `options` count, or running past
    /// the end of the trace).
    pub fn decide(&mut self, kind: ChoiceKind, options: usize) -> usize {
        match self {
            Chooser::Record { rng, choices, .. } => {
                let chosen = rng.below(options);
                choices.push(ChoicePoint { kind, options, chosen });
                chosen
            }
            Chooser::Replay { trace, pos } => {
                let cp = trace.choices.get(*pos).copied().unwrap_or_else(|| {
                    panic!(
                        "replay divergence: ran out of recorded choices at index {} \
                         (live decision was {:?} over {} options)",
                        pos, kind, options
                    )
                });
                assert_eq!(
                    cp.kind, kind,
                    "replay divergence at {}: recorded {:?}, live {:?}",
                    *pos, cp.kind, kind
                );
                assert_eq!(
                    cp.options, options,
                    "replay divergence at {}: recorded {} options, live {}",
                    *pos, cp.options, options
                );
                *pos += 1;
                cp.chosen
            }
        }
    }

    /// Finish and return the trace (returns the original trace for a replay chooser).
    pub fn into_trace(self) -> SchedTrace {
        match self {
            Chooser::Record { seed, choices, .. } => SchedTrace { seed, choices },
            Chooser::Replay { trace, .. } => trace,
        }
    }
}

#[cfg(test)]
mod tests {
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
}
