//! Result types for the translation validator.

use logicaffeine_compile::ParseError;

/// Why building a symbolic summary failed.
#[derive(Debug)]
pub enum TvError {
    /// The source did not parse.
    Parse(ParseError),
    /// The program uses a construct outside the supported Verifiable Core.
    Unsupported(String),
}

/// Outcome of cross-validating the LOGOS encoder against the tree-walking interpreter
/// on one program (the meta-soundness check, `work/PE_IMPROVE.md §4.2`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoundnessReport {
    /// The encoder's symbolic summary provably matches the interpreter's observable
    /// behavior (outputs and error/no-error).
    Agrees,
    /// The encoder and interpreter disagree — an encoder bug (or a genuine miscompile,
    /// if the encoder is trusted). The detail localizes the divergence.
    Disagrees {
        /// Human-readable description of the divergence.
        detail: String,
    },
    /// The program is outside the Verifiable Core, so the encoder did not model it.
    /// Soundly excluded from the check — never reported as agreement.
    Unsupported {
        /// Which construct fell outside the fragment.
        reason: String,
    },
    /// The source failed to parse (should not occur for well-formed corpus programs).
    ParseFailed {
        /// Parser diagnostic.
        detail: String,
    },
    /// A nondeterministic program: for every seed in the sweep, the seeded encoder provably
    /// matched the seeded interpreter (`Select` winners drawn from the same SplitMix64).
    /// This is the seeded-replay analog of [`Self::Agrees`].
    SeedReplayAgrees,
    /// A nondeterministic program where the seeded encoder and interpreter diverged at some
    /// seed — caught by the per-seed cross-check, so never a false agreement.
    SeedReplayDisagrees {
        /// Which seed diverged and how.
        detail: String,
    },
}
