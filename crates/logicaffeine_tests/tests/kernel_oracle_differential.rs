//! LOGOS's own kernel and the `dag` core, on every Lean Kernel Arena case both read.
//!
//! The `dag` core checks a Lean export directly. The `Term` kernel reaches the same export through
//! the Lean importer (`interface::lean`), which also re-checks every declaration with the independent
//! `recheck`. The Arena's corpus is adversarial, and on the verdicts it decides the two may never
//! disagree: neither accepts an invalid proof, and a valid proof that both decide is accepted by
//! both. An `either` case may go either way in each. Where the `Term` path cannot decide a case
//! (it declines, or runs past its deadline), the case is pinned in [`TERM_GAPS`] with the reason,
//! and that list only shrinks: a pinned case the `Term` path now decides fails until its row goes.
//!
//! The large inputs (init, std, Mathlib, …) are not run here: the importer is not built for their
//! size, and `arena_corpus` holds the `dag` core to them.

use logicaffeine_kernel::dag::{check_export, Verdict as Dag};
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

/// The cases the `Term` path cannot decide today, with the reason it gives.
const TERM_GAPS: &[(&str, &str)] = &[
    ("good/init-prelude.ndjson", "has no pinned literal reading"),
    (
        "good/perf/magma-list-deep-n21.ndjson",
        "has no pinned literal reading",
    ),
    (
        "good/perf/magma-list-deep-n36.ndjson",
        "has no pinned literal reading",
    ),
    ("good/perf/app-lam.ndjson", "materialization budget"),
    ("good/other/nat-lit-sub.ndjson", DEADLINE_PASSED),
    ("good/perf/fueled-chain.ndjson", DEADLINE_PASSED),
];

const DEADLINE_PASSED: &str = "the deadline passed";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Expect {
    Accept,
    Reject,
    Either,
}

#[derive(Debug)]
enum Term {
    Accept,
    Reject(String),
    Undecided(String),
}

fn corpus(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/arena_corpus")
        .join(path)
}

fn dag(path: &str) -> Dag {
    let file = corpus(path);
    std::thread::Builder::new()
        .stack_size(8 << 30)
        .spawn(move || {
            check_export(BufReader::with_capacity(
                1 << 20,
                std::fs::File::open(file).unwrap(),
            ))
            .0
        })
        .unwrap()
        .join()
        .expect("the dag checker panicked")
}

/// The `Term` path's verdict, or `Undecided` once `deadline` passes. The checking thread is left to
/// finish on its own: each test runs in its own process, which ends with the test.
fn term(path: &str, deadline: Duration) -> Term {
    let text = std::fs::read_to_string(corpus(path)).unwrap();
    let (send, receive) = mpsc::channel();
    std::thread::Builder::new()
        .stack_size(8 << 30)
        .spawn(move || {
            let _ = send.send(logicaffeine_kernel::interface::lean::check_str(&text));
        })
        .unwrap();
    match receive.recv_timeout(deadline) {
        Ok(Ok(())) => Term::Accept,
        Ok(Err(verdict)) if verdict.code() == 1 => Term::Reject(format!("{:?}", verdict.reason())),
        Ok(Err(verdict)) => Term::Undecided(format!("{:?}", verdict.reason())),
        Err(_) => Term::Undecided(DEADLINE_PASSED.to_string()),
    }
}

fn differential(path: &str, expect: Expect, deadline: Duration) {
    let gap = TERM_GAPS
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(_, why)| *why);
    // A case pinned for a reason other than the deadline asserts that reason, not a time: under a
    // loaded suite the work before it can outlast the ordinary deadline, so it gets the slow one.
    let deadline = match gap {
        Some(why) if why != DEADLINE_PASSED => deadline.max(SLOW_DEADLINE),
        _ => deadline,
    };
    let term = term(path, deadline);
    let dag = dag(path);
    if let (Expect::Reject, Term::Accept) = (expect, &term) {
        panic!("{path}: the Term kernel ACCEPTED an invalid proof");
    }
    if let (Expect::Reject, Dag::Accept) = (expect, &dag) {
        panic!("{path}: the dag core ACCEPTED an invalid proof");
    }
    match (&term, gap) {
        (Term::Undecided(why), Some(pinned)) => {
            assert!(
                why.contains(pinned),
                "{path}: undecided for another reason than the pinned one ({pinned}): {why}"
            )
        }
        (Term::Undecided(why), None) => panic!("{path}: the Term path cannot decide it: {why}"),
        (decided, Some(pinned)) => {
            panic!("{path}: the Term path now decides it ({decided:?}); remove its TERM_GAPS row ({pinned})")
        }
        (Term::Accept | Term::Reject(_), None) => {}
    }
    if expect == Expect::Either {
        return;
    }
    match (&term, &dag) {
        (Term::Accept, Dag::Accept)
        | (Term::Reject(_), Dag::Reject(_))
        | (Term::Undecided(_), _) => {}
        (Term::Reject(why), Dag::Accept) => {
            panic!("{path}: the Term kernel rejects a proof the dag core accepts: {why}")
        }
        (Term::Accept, Dag::Reject(e)) => {
            panic!("{path}: the dag core rejects a proof the Term kernel accepts: {e:?}")
        }
        (_, Dag::Decline(e)) => panic!("{path}: the dag core declined: {e:?}"),
    }
}

/// An ordinary case decides in well under a second on both paths; the deadline only bounds a case
/// pinned as running past it.
const DEADLINE: Duration = Duration::from_secs(30);
/// The cases `cases.rs` marks slow run with the ignored set, where the `Term` path's slowest decided
/// case (`perf/magma-list-pair-n21`) takes about a hundred seconds.
const SLOW_DEADLINE: Duration = Duration::from_secs(300);

macro_rules! arena_case {
    (#[ignore = $why:literal] $name:ident, $path:expr, $expect:ident) => {
        #[test]
        #[ignore = $why]
        fn $name() {
            differential($path, Expect::$expect, SLOW_DEADLINE)
        }
    };
    ($name:ident, $path:expr, $expect:ident) => {
        #[test]
        fn $name() {
            differential($path, Expect::$expect, DEADLINE)
        }
    };
}

macro_rules! arena_heavy_case {
    ($($input:tt)*) => {};
}

include!("arena_corpus/cases.rs");

#[test]
fn every_pinned_gap_is_a_case_of_the_corpus() {
    let cases = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/arena_corpus/cases.rs"),
    )
    .unwrap();
    for (path, why) in TERM_GAPS {
        assert!(!why.is_empty(), "{path}: a pinned gap names its reason");
        assert!(
            cases.contains(&format!("\"{path}\"")),
            "{path} is pinned but is not a vendored corpus case"
        );
    }
}
