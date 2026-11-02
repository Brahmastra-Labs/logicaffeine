//! W4.1 — `hermeticity_redteam`: escape attempts against the sealed world.
//!
//! Every test
//! runs in a sealed world — virtual clock, in-memory filesystem, seeded
//! randomness, offline network — so a test is a pure function of
//! `(program, seed)`. This battery TRIES TO ESCAPE and must fail:
//! real time, real files, and real entropy must be unreachable without a
//! capability sentence, and the simulated resources must behave exactly.

use logicaffeine_compile::testrun::{render_report, run_tests, TestConfig, Verdict};
use logicaffeine_system::testworld::{self, Capabilities, TestWorld, SEALED_EPOCH_MS};

fn config() -> TestConfig {
    TestConfig::default()
}

/// The sealed clock is FROZEN at the synthetic epoch; `Advance time by`
/// moves it exactly; a `sleep` costs no real time.
#[test]
fn sealed_clock_is_frozen_and_advances_virtually() {
    let suite = "\
## Test: Time is sealed\n\
\x20   Let t1 be now().\n\
\x20   Advance time by 3 seconds.\n\
\x20   Let t2 be now().\n\
\x20   Expect that t2 minus t1 is 3000.\n";
    let start = std::time::Instant::now();
    let report = run_tests(suite, &config()).expect("parses");
    assert_eq!(
        report.outcomes[0].verdict,
        Verdict::Passed,
        "the virtual clock advances exactly: {:?}",
        report.outcomes[0].failures
    );
    assert!(
        start.elapsed().as_millis() < 2500,
        "no real waiting happened (virtual time is instant)"
    );
}

/// Sealed randomness is a pure function of the seed: two whole runs render
/// byte-identically, and different suite seeds give different draws.
#[test]
fn sealed_randomness_is_deterministic_per_seed() {
    let suite = "\
## Test: Entropy is recorded\n\
\x20   Let a be randomInt(1, 1000000).\n\
\x20   Show a.\n\
\x20   Expect output:\n\
\x20       0\n";
    // The Expect output is WRONG on purpose: the failure message carries the
    // drawn value, so byte-identical reports prove byte-identical draws.
    let r1 = render_report(&run_tests(suite, &config()).unwrap());
    let r2 = render_report(&run_tests(suite, &config()).unwrap());
    assert_eq!(r1, r2, "same seed ⇒ identical draws, identical report");
    let mut other = config();
    other.seed = 42;
    let r3 = render_report(&run_tests(suite, &other).unwrap());
    assert_ne!(
        r1.replace("seed 0", "seed 42"),
        r3,
        "a different suite seed draws a different stream"
    );
}

/// `Given a file` seeds the sealed filesystem; a Vfs-backed `Read` reads the
/// EXACT content back — no real disk touched. The write-then-read round-trip
/// proves the seam end-to-end (not a vacuous `1 is 1`).
#[test]
fn given_file_reads_back_exact_content() {
    let suite = "\
## Test: The seeded fixture reads back exactly\n\
\x20   Given a file \"notes.txt\" containing \"hello sealed world\".\n\
\x20   Read contents from file \"notes.txt\".\n\
\x20   Expect that contents equals \"hello sealed world\".\n\
\n\
## Test: A written file round-trips in memory\n\
\x20   Write \"fresh bytes\" to file \"out.txt\".\n\
\x20   Read back from file \"out.txt\".\n\
\x20   Expect that back equals \"fresh bytes\".\n";
    let report = run_tests(suite, &config()).expect("parses");
    for o in &report.outcomes {
        assert_eq!(
            o.verdict,
            Verdict::Passed,
            "{}: the sealed filesystem round-trips exactly: {:?}",
            o.name,
            o.failures
        );
    }
}

/// The sealed filesystem does NOT touch real disk: a file written by one test
/// is invisible to the next (fresh world per test), and no file appears on the
/// host.
#[test]
fn sealed_writes_never_reach_real_disk() {
    let marker = "/tmp/logos-sealed-must-not-exist-9f2c.txt";
    let _ = std::fs::remove_file(marker);
    let suite = format!(
        "## Test: A write stays in memory\n\
\x20   Write \"leak?\" to file \"{marker}\".\n\
\x20   Expect that 1 is 1.\n"
    );
    let report = run_tests(&suite, &config()).expect("parses");
    assert_eq!(report.outcomes[0].verdict, Verdict::Passed);
    assert!(
        !std::path::Path::new(marker).exists(),
        "a sealed-world write must NEVER create a real file"
    );
}

/// Rust-level seam drill: a sealed read of an unseeded file fails with the
/// Socratic remedy naming BOTH escapes (`Given a file` and the capability).
#[test]
fn sealed_read_names_the_remedy() {
    testworld::install(TestWorld::new(Capabilities::default(), 7));
    let err = logicaffeine_system::file::read("missing.txt".to_string())
        .expect_err("an unseeded file is absent from the sealed filesystem");
    testworld::uninstall();
    assert!(
        err.contains("Given a file"),
        "names the seeding remedy: {err}"
    );
    assert!(
        err.contains("real filesystem"),
        "names the capability escape: {err}"
    );
}

/// Rust-level seam drill: the clock capability opts back into reality.
#[test]
fn clock_capability_returns_real_time() {
    testworld::install(TestWorld::new(Capabilities::default(), 7));
    assert_eq!(
        logicaffeine_system::time::now(),
        SEALED_EPOCH_MS,
        "sealed = the epoch"
    );
    testworld::uninstall();
    testworld::install(TestWorld::new(
        Capabilities {
            clock: true,
            ..Capabilities::default()
        },
        7,
    ));
    assert!(
        logicaffeine_system::time::now() > SEALED_EPOCH_MS,
        "the clock capability reads the real clock"
    );
    testworld::uninstall();
}

/// Capability-granted tests leave the hermetic guarantee: without `--live`
/// they are loudly gated, never silently run.
#[test]
fn live_tests_are_gated_behind_the_flag() {
    let suite = "\
## Test: Needs the real clock\n\
\x20   This test may use the real clock.\n\
\x20   Expect that 1 is 1.\n";
    let gated = run_tests(suite, &config()).expect("parses");
    assert_eq!(gated.outcomes[0].verdict, Verdict::Skipped);
    assert!(
        gated.outcomes[0]
            .skip_reason
            .as_deref()
            .unwrap_or("")
            .contains("--live"),
        "the gate names the flag: {:?}",
        gated.outcomes[0].skip_reason
    );
    let mut live = config();
    live.live = true;
    let run = run_tests(suite, &live).expect("parses");
    assert_eq!(
        run.outcomes[0].verdict,
        Verdict::Passed,
        "{:?}",
        run.outcomes[0].failures
    );
}
