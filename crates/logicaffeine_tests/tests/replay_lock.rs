//! W1.1 — `replay_lock`: every failing run is a replayable artifact.
//!
//! Claims locked here:
//!
//! 1. EXHAUSTIVE failure-kind catalog — every [`FailureKind`] has a corpus
//!    program (or an explicit lands-in-wave deferral); no wildcard arm, so a
//!    new kind cannot ship unclassified.
//! 2. Capture: a failing corpus program yields a bundle whose failure message
//!    is byte-identical to the live run's error, whose source is embedded
//!    verbatim, and whose format/seed/cursor fields are correct.
//! 3. Replay: `verify(bundle)` re-executes from the embedded source and
//!    reports `Reproduced` — byte-identical failure text and cursor.
//! 4. A passing program captures nothing (recording is free until failure).
//! 5. The JSON round-trips losslessly, and format v1 is golden-locked.
//! 6. Drift is detected, never silently misreplayed: a bundle whose recorded
//!    source hash no longer matches its embedded files reports `Diverged`.

use logicaffeine_compile::replay::{
    capture_failure, verify, Cursor, Failure, FailureKind, ReplayBundle, Schedule, SourceSection,
    TraceChoice, Transcript, Verify, REPLAY_FORMAT,
};
use std::collections::BTreeMap;

/// Data-dependent division by zero: a genuine runtime error on the oracle path
/// (no constant folding reaches the tree-walker's front door).
const DIV_BY_ZERO: &str = "## Main\n\
    \x20   Let a be 10.\n\
    \x20   Let b be 0.\n\
    \x20   Let x be a / b.\n\
    \x20   Show x.\n";

/// A hard assertion that must fail.
const HARD_ASSERT: &str = "## Main\n\
    \x20   Require that 1 is greater than 2.\n";

/// A receive with no possible sender: every task blocks, the scheduler proves
/// the deadlock (same corpus shape as `vm_concurrency::vm_deadlock_detected`).
const DEADLOCK: &str = "## Main\n\
    \x20   Let ch be a Pipe of Int.\n\
    \x20   Receive x from ch.\n\
    \x20   Show x.\n";

const PASSING: &str = "## Main\n\
    \x20   Show 42.\n";

const ENTRY: &str = "src/main.lg";

/// What the catalog demands for a failure kind: a corpus program that produces
/// it TODAY, or the explicit wave where producing it becomes possible.
enum KindCorpus {
    Program(&'static str),
    LandsInWave(u8),
}

/// EXHAUSTIVE — no wildcard arm. Adding a `FailureKind` variant fails this
/// build until the kind gets a corpus program or a named deferral.
fn corpus(kind: FailureKind) -> KindCorpus {
    match kind {
        FailureKind::RuntimeError => KindCorpus::Program(DIV_BY_ZERO),
        FailureKind::Assertion => KindCorpus::Program(HARD_ASSERT),
        FailureKind::Deadlock => KindCorpus::Program(DEADLOCK),
        // `## Test` expectations need the Wave 2 grammar + Wave 3 runner.
        FailureKind::TestFailure => KindCorpus::LandsInWave(3),
    }
}

/// The drill: every producible failure kind captures an artifact that
/// classifies correctly and reproduces byte-identically under `verify`.
#[test]
fn every_failure_kind_captures_and_reproduces() {
    for kind in FailureKind::ALL {
        match corpus(kind) {
            KindCorpus::Program(source) => {
                let bundle = capture_failure(ENTRY, source, 0, &[])
                    .unwrap_or_else(|| panic!("{kind:?}: the corpus program must fail"));
                assert_eq!(
                    bundle.failure.kind, kind,
                    "{kind:?}: the failure classifies as its own kind (message: {})",
                    bundle.failure.message
                );
                assert!(
                    !bundle.failure.message.is_empty(),
                    "{kind:?}: the failure message is never empty"
                );
                assert_eq!(
                    verify(&bundle),
                    Verify::Reproduced,
                    "{kind:?}: replay from the embedded source reproduces the failure"
                );
            }
            KindCorpus::LandsInWave(wave) => {
                assert!(wave > 1, "{kind:?}: a deferral must name a FUTURE wave");
            }
        }
    }
}

/// A passing program captures nothing — the recorder costs a seed and a choice
/// vec until the moment of failure, and writes only then (D0.4).
#[test]
fn passing_program_captures_nothing() {
    assert_eq!(
        capture_failure(ENTRY, PASSING, 0, &[]),
        None,
        "a green run emits no artifact"
    );
}

/// The bundle is self-contained and truthful: format version, verbatim
/// embedded source, the recorded seed, a hash that matches the files, and a
/// Wave-1 cursor (seq stays 0 until the observation channel lands in Wave 7).
#[test]
fn bundle_is_self_contained_and_versioned() {
    let bundle = capture_failure(ENTRY, DIV_BY_ZERO, 0, &[]).expect("division by zero fails");
    assert_eq!(bundle.format, REPLAY_FORMAT);
    assert_eq!(bundle.source.entry, ENTRY);
    assert_eq!(
        bundle.source.files.get(ENTRY).map(String::as_str),
        Some(DIV_BY_ZERO),
        "the source is embedded verbatim"
    );
    assert_eq!(
        bundle.source.hash,
        ReplayBundle::source_hash(&bundle.source.files),
        "the recorded hash matches the embedded files"
    );
    assert_eq!(bundle.schedule.seed, 0);
    assert_eq!(
        bundle.failure.cursor.seq, 0,
        "event seq starts moving in Wave 7"
    );
}

/// A deadlock is a SCHEDULER verdict: its bundle carries the recorded decision
/// trace, and the failure cursor's epoch equals the decision count at death.
#[test]
fn deadlock_bundle_records_the_schedule() {
    let bundle = capture_failure(ENTRY, DEADLOCK, 0, &[]).expect("the receive deadlocks");
    assert_eq!(bundle.failure.kind, FailureKind::Deadlock);
    assert_eq!(
        bundle.failure.cursor.epoch,
        bundle.schedule.trace.len() as u64,
        "the cursor's epoch is the count of recorded scheduling decisions"
    );
}

fn fixed_bundle() -> ReplayBundle {
    let mut files = BTreeMap::new();
    files.insert(ENTRY.to_string(), HARD_ASSERT.to_string());
    ReplayBundle {
        format: REPLAY_FORMAT,
        logos_version: "test".to_string(),
        source: SourceSection {
            hash: "test-hash".to_string(),
            entry: ENTRY.to_string(),
            files,
        },
        schedule: Schedule {
            seed: 7,
            trace: vec![TraceChoice {
                kind: "TaskPick".to_string(),
                options: 1,
                chosen: 0,
            }],
        },
        generation: vec![],
        transcript: Transcript {
            args: vec!["prog".to_string()],
            stdin: vec![],
        },
        failure: Failure {
            kind: FailureKind::Assertion,
            message: "Assertion failed".to_string(),
            cursor: Cursor {
                epoch: 1,
                task: 0,
                seq: 0,
            },
            span: None,
            test: None,
        },
    }
}

/// JSON round-trip is the identity.
#[test]
fn json_roundtrips_losslessly() {
    let bundle = fixed_bundle();
    let json = bundle.to_json();
    let back = ReplayBundle::from_json(&json).expect("a bundle we wrote parses back");
    assert_eq!(back, bundle);
}

/// Format v1 is GOLDEN. This exact serialization is the committed artifact
/// format — changing it requires a format bump plus a constitution amendment,
/// not an edit to this string.
#[test]
fn format_v1_golden() {
    let expected = r###"{
  "format": 1,
  "logos_version": "test",
  "source": {
    "hash": "test-hash",
    "entry": "src/main.lg",
    "files": {
      "src/main.lg": "## Main\n    Require that 1 is greater than 2.\n"
    }
  },
  "schedule": {
    "seed": 7,
    "trace": [
      {
        "kind": "TaskPick",
        "options": 1,
        "chosen": 0
      }
    ]
  },
  "generation": [],
  "transcript": {
    "args": [
      "prog"
    ],
    "stdin": []
  },
  "failure": {
    "kind": "assertion",
    "message": "Assertion failed",
    "cursor": {
      "epoch": 1,
      "task": 0,
      "seq": 0
    },
    "span": null,
    "test": null
  }
}"###;
    assert_eq!(
        fixed_bundle().to_json(),
        expected,
        "format v1 serialization is locked"
    );
}

/// An unknown format version is refused loudly, never half-parsed.
#[test]
fn future_format_versions_are_refused() {
    let json = fixed_bundle()
        .to_json()
        .replacen("\"format\": 1", "\"format\": 999", 1);
    let err = ReplayBundle::from_json(&json).expect_err("format 999 must be refused");
    assert!(
        err.contains("999"),
        "the refusal names the offending version: {err}"
    );
}

/// Drift is detected, never silently misreplayed: if the recorded hash no
/// longer matches the embedded files, verify reports divergence instead of
/// pretending the replay is faithful.
#[test]
fn source_drift_reports_divergence() {
    let mut bundle = capture_failure(ENTRY, DIV_BY_ZERO, 0, &[]).expect("division by zero fails");
    bundle.source.hash = "blake-of-something-else".to_string();
    match verify(&bundle) {
        Verify::Diverged(reason) => {
            assert!(
                reason.contains("hash") || reason.contains("source"),
                "the divergence names its cause: {reason}"
            );
        }
        other => panic!("a hash-drifted bundle must report Diverged, got {other:?}"),
    }
}

/// A fixed bug is a `NoFailure` verdict — the committed-regression workflow's
/// green state (the bundle stays in tests/replays/ as proof the bug is dead).
#[test]
fn fixed_bug_reports_no_failure() {
    let mut bundle = capture_failure(ENTRY, DIV_BY_ZERO, 0, &[]).expect("division by zero fails");
    // "Fix" the program: replay the bundle against a corrected embedded source.
    let fixed = DIV_BY_ZERO.replace("Let b be 0.", "Let b be 2.");
    bundle.source.files.insert(ENTRY.to_string(), fixed);
    bundle.source.hash = ReplayBundle::source_hash(&bundle.source.files);
    assert_eq!(
        verify(&bundle),
        Verify::NoFailure,
        "a repaired program reports the recorded failure no longer fires"
    );
}
