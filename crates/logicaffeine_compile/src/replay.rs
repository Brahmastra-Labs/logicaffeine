//! The `.replay` failure artifact.
//!
//! A failing run emits a self-contained, human-diffable JSON bundle — seed + full
//! schedule trace + embedded source + input transcript + failure coordinate — that
//! reproduces the execution byte-for-byte on any machine. Consumed by
//! `largo replay --verify`, `largo debug`, the DAP adapter, and the Studio; a
//! bundle committed under `tests/replays/` is a permanent regression.
//!
//! The bundle carries its OWN wire representation of scheduling choices
//! ([`TraceChoice`]) rather than the runtime's `ChoicePoint`, so the artifact
//! format outlives internal enum shape.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Bundle format version. Bumping it requires a decision-record amendment in the
/// constitution and a new golden in `replay_lock.rs`.
pub const REPLAY_FORMAT: u32 = 1;

/// Why the artifact exists. `replay_lock.rs` holds a no-wildcard corpus over
/// every variant — adding a kind without a corpus entry fails the build there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// The program raised a runtime error (division by zero, bad index, …).
    RuntimeError,
    /// A hard `Require that` assertion failed.
    Assertion,
    /// The scheduler proved every remaining task is stuck.
    Deadlock,
    /// A `## Test` expectation failed (producible from Wave 3's runner).
    TestFailure,
}

impl FailureKind {
    /// Every variant, for exhaustive corpus drills.
    pub const ALL: [FailureKind; 4] = [
        FailureKind::RuntimeError,
        FailureKind::Assertion,
        FailureKind::Deadlock,
        FailureKind::TestFailure,
    ];

    /// Classify a runtime error message into a failure kind. `TestFailure` is
    /// assigned by the test runner (Wave 3), never inferred from message text.
    pub fn classify(message: &str) -> FailureKind {
        if message.contains("deadlock") {
            FailureKind::Deadlock
        } else if message.contains("Assertion failed") {
            FailureKind::Assertion
        } else {
            FailureKind::RuntimeError
        }
    }
}

/// The causal cursor (§0.4): `(epoch, task, seq)`. Wave 1 stamps `epoch` (the
/// count of recorded scheduling decisions at failure) and `task`; the per-task
/// event `seq` starts moving when the observation channel lands (Wave 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub epoch: u64,
    pub task: u64,
    pub seq: u64,
}

/// The embedded program: content hash + entry name + every source file, so the
/// bundle opens anywhere, forever.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSection {
    pub hash: String,
    pub entry: String,
    pub files: BTreeMap<String, String>,
}

/// One recorded scheduling decision (wire form of the runtime's `ChoicePoint`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceChoice {
    pub kind: String,
    pub options: u32,
    pub chosen: u32,
}

/// The schedule: the seed alone regenerates the trace while the source is
/// unchanged; the full trace makes drift detectable and carries explored
/// schedules that no seed generates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    pub seed: u64,
    pub trace: Vec<TraceChoice>,
}

/// One recorded stdin read, cursor-stamped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StdinRead {
    pub cursor: Cursor,
    pub data_b64: String,
}

/// Everything that crossed the world boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    pub args: Vec<String>,
    pub stdin: Vec<StdinRead>,
}

/// The failing sentence's source coordinates, when the failing tier carries spans.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureSpan {
    pub file: String,
    pub start: usize,
    pub end: usize,
}

/// Why this artifact exists: the reproduced-or-it-didn't-happen record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub kind: FailureKind,
    pub message: String,
    pub cursor: Cursor,
    pub span: Option<FailureSpan>,
    pub test: Option<String>,
}

/// The `.replay` bundle (constitution §0.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayBundle {
    pub format: u32,
    pub logos_version: String,
    pub source: SourceSection,
    pub schedule: Schedule,
    pub generation: Vec<u64>,
    pub transcript: Transcript,
    pub failure: Failure,
}

/// FNV-1a 64: a tiny, dependency-free, platform-identical content hash. The
/// bundle hash is a DRIFT DETECTOR (did the program change since recording?),
/// not a cryptographic commitment.
fn fnv1a64(bytes: &[u8], mut state: u64) -> u64 {
    const PRIME: u64 = 0x0000_0100_0000_01B3;
    for b in bytes {
        state ^= u64::from(*b);
        state = state.wrapping_mul(PRIME);
    }
    state
}

impl ReplayBundle {
    /// Pretty-printed JSON — human-diffable, git-committable.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a replay bundle always serializes")
    }

    /// Parse a bundle, refusing unknown format versions loudly (a future format
    /// is named in the error, never half-parsed).
    pub fn from_json(json: &str) -> Result<ReplayBundle, String> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("unreadable .replay bundle: {e}"))?;
        let format = value
            .get("format")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "unreadable .replay bundle: no format field".to_string())?;
        if format != u64::from(REPLAY_FORMAT) {
            return Err(format!(
                "unsupported .replay format {format} — this build reads format {REPLAY_FORMAT}"
            ));
        }
        serde_json::from_value(value).map_err(|e| format!("malformed .replay bundle: {e}"))
    }

    /// Content hash over the embedded files — deterministic and
    /// order-independent (the map is ordered; path and content both feed the
    /// state, length-delimited so boundaries can't alias).
    pub fn source_hash(files: &BTreeMap<String, String>) -> String {
        const OFFSET: u64 = 0xCBF2_9CE4_8422_2325;
        let mut state = OFFSET;
        for (path, content) in files {
            state = fnv1a64(&(path.len() as u64).to_le_bytes(), state);
            state = fnv1a64(path.as_bytes(), state);
            state = fnv1a64(&(content.len() as u64).to_le_bytes(), state);
            state = fnv1a64(content.as_bytes(), state);
        }
        format!("fnv1a64:{state:016x}")
    }
}

/// Run a program on the capture engine — the seeded concurrent VM driver, the
/// one engine that serves sequential programs, concurrent programs, AND can
/// prove a deadlock. Capture and verify use the SAME engine, so failure text is
/// byte-identical by determinism.
fn run_traced(source: &str, seed: u64) -> (Option<String>, Vec<TraceChoice>) {
    let (result, trace) = crate::ui_bridge::run_vm_concurrent_traced(source, seed);
    let choices = trace
        .choices
        .iter()
        .map(|cp| TraceChoice {
            kind: format!("{:?}", cp.kind),
            options: cp.options as u32,
            chosen: cp.chosen as u32,
        })
        .collect();
    (result.error, choices)
}

/// Run `source` (entry file named `entry`) under `seed` with `args`; if the run
/// fails, capture the complete artifact. `None` when the program succeeds —
/// recording costs nothing until the moment of failure (D0.4).
pub fn capture_failure(
    entry: &str,
    source: &str,
    seed: u64,
    args: &[String],
) -> Option<ReplayBundle> {
    let (error, trace) = run_traced(source, seed);
    let message = error?;
    let kind = FailureKind::classify(&message);
    let epoch = trace.len() as u64;
    let mut files = BTreeMap::new();
    files.insert(entry.to_string(), source.to_string());
    let hash = ReplayBundle::source_hash(&files);
    Some(ReplayBundle {
        format: REPLAY_FORMAT,
        logos_version: env!("CARGO_PKG_VERSION").to_string(),
        source: SourceSection {
            hash,
            entry: entry.to_string(),
            files,
        },
        schedule: Schedule { seed, trace },
        generation: Vec::new(),
        transcript: Transcript {
            args: args.to_vec(),
            stdin: Vec::new(),
        },
        failure: Failure {
            kind,
            message,
            cursor: Cursor {
                epoch,
                task: 0,
                seq: 0,
            },
            span: None,
            test: None,
        },
    })
}

/// The verdict of re-executing a bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verify {
    /// The recorded failure reproduced byte-identically.
    Reproduced,
    /// The re-run no longer matches the recording (source drift, schedule
    /// divergence, different failure) — with the reason.
    Diverged(String),
    /// The re-run succeeded: the recorded bug no longer fires.
    NoFailure,
}

/// Re-execute the bundle from its embedded source under its recorded seed and
/// check the failure reproduces byte-identically. Drift is DETECTED and named,
/// never silently misreplayed.
pub fn verify(bundle: &ReplayBundle) -> Verify {
    let recomputed = ReplayBundle::source_hash(&bundle.source.files);
    if recomputed != bundle.source.hash {
        return Verify::Diverged(format!(
            "source hash mismatch: recorded {} but the embedded files hash to {recomputed} \
             — the bundle was edited since recording",
            bundle.source.hash
        ));
    }
    let Some(source) = bundle.source.files.get(&bundle.source.entry) else {
        return Verify::Diverged(format!(
            "the entry file {} is missing from the bundle",
            bundle.source.entry
        ));
    };
    let (error, live_trace) = run_traced(source, bundle.schedule.seed);
    let Some(message) = error else {
        return Verify::NoFailure;
    };
    if live_trace != bundle.schedule.trace {
        let at = live_trace
            .iter()
            .zip(bundle.schedule.trace.iter())
            .position(|(live, recorded)| live != recorded)
            .unwrap_or_else(|| live_trace.len().min(bundle.schedule.trace.len()));
        return Verify::Diverged(format!(
            "schedule diverged at decision {at}: recorded {} decisions, live run made {}",
            bundle.schedule.trace.len(),
            live_trace.len()
        ));
    }
    if message != bundle.failure.message {
        return Verify::Diverged(format!(
            "failure message changed: recorded {:?}, live {message:?}",
            bundle.failure.message
        ));
    }
    let live_kind = FailureKind::classify(&message);
    if live_kind != bundle.failure.kind {
        return Verify::Diverged(format!(
            "failure kind changed: recorded {:?}, live {live_kind:?}",
            bundle.failure.kind
        ));
    }
    let live_epoch = live_trace.len() as u64;
    if live_epoch != bundle.failure.cursor.epoch {
        return Verify::Diverged(format!(
            "failure cursor moved: recorded epoch {}, live epoch {live_epoch}",
            bundle.failure.cursor.epoch
        ));
    }
    Verify::Reproduced
}
