//! The release gate's summary of a pass that STOPPED before nextest printed its `Summary [` line.
//!
//! `scripts/lib/nextest-summary.sh` reads a pass's counts from that line. A pass that never reaches
//! it — the machine ran out of disk, the runner was killed — still printed one row per finished
//! test, and those rows are the only record of what it did. The fixture
//! (`tests/fixtures/gate_summary/nextest-main-pass-truncated.log`) is cut from
//! `logs/test-run-fast-20260926-023704.log`: the main pass started 29,131 tests, printed its last
//! result at 11,884 and stopped, having printed 326 `FAIL` rows and two `TIMEOUT` rows. The
//! wasm-jit, doctest and Lean-verdict passes that follow it are cut from the same run, rows and
//! Summary lines intact.
//!
//! A pass with no Summary line is INCOMPLETE, and the summary says so in the pass's row, in the
//! totals and on the machine-readable stderr line, with the figures its own rows establish: how
//! far it got, and every failure it named on the way.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// `(stdout, stderr)` of the summarizer over one gate_summary fixture.
fn summarize(fixture: &str) -> (String, String) {
    let script = repo_root().join("scripts/lib/nextest-summary.sh");
    let out = Command::new("bash")
        .arg(&script)
        .arg(
            repo_root()
                .join("crates/logicaffeine_tests/tests/fixtures/gate_summary")
                .join(fixture),
        )
        .output()
        .expect("run nextest-summary.sh");
    assert!(
        out.status.success(),
        "nextest-summary.sh failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (
        String::from_utf8(out.stdout).expect("utf8 summary"),
        String::from_utf8(out.stderr).expect("utf8 stderr"),
    )
}

fn row<'s>(summary: &'s str, label: &str) -> &'s str {
    summary
        .lines()
        .find(|l| l.starts_with(label))
        .unwrap_or_else(|| panic!("no `{label}` row in:\n{summary}"))
}

const TRUNCATED: &str = "nextest-main-pass-truncated.log";

#[test]
fn a_truncated_main_pass_counts_every_named_failure() {
    let (s, _) = summarize(TRUNCATED);
    let main = row(&s, " nextest  :");
    assert!(
        main.contains("INCOMPLETE"),
        "a pass with no Summary line is INCOMPLETE, and its row says so: {main}"
    );
    assert!(
        main.contains("11884 of 29131 tests"),
        "the row says how far the pass got — its last printed result against the count it started: {main}"
    );
    assert!(
        main.contains("326 failed") && main.contains("2 timed out"),
        "the row counts the failures its own rows name, never 0: {main}"
    );
    let named = s
        .lines()
        .filter(|l| l.contains("/29131)") || l.contains("(───────────)"))
        .count();
    assert_eq!(
        named, 328,
        "every one of the pass's 328 failing tests is listed:\n{s}"
    );
}

#[test]
fn the_totals_carry_the_incomplete_pass_and_say_it_is_incomplete() {
    let (s, _) = summarize(TRUNCATED);
    let totals = row(&s, " totals   :");
    assert!(
        totals.starts_with(" totals   : 12522 tests run, 342 failed, 2 timed out, 0 flaky"),
        "11884 + 485 + 146 + 7 tests and 326 + 9 + 7 failures — the main pass is not left out: {totals}"
    );
    assert!(
        totals.contains("INCOMPLETE") && totals.contains("PASS 1/2"),
        "totals built from a pass that never finished say which pass, so they are not read as a whole run: {totals}"
    );
}

#[test]
fn an_incomplete_pass_raises_no_reconciliation_against_a_count_it_never_printed() {
    let (s, _) = summarize(TRUNCATED);
    assert!(
        !s.contains("RECONCILIATION"),
        "the reconciliation compares named rows with a Summary's count; a pass with no Summary has no \
         count to disagree with, and '0 counted, 328 named' blames the extraction for the pass stopping:\n{s}"
    );
}

#[test]
fn the_passes_that_did_finish_keep_their_own_summaries() {
    let (s, _) = summarize(TRUNCATED);
    assert_eq!(
        row(&s, " wasm-jit :"),
        " wasm-jit : 485 tests run: 476 passed, 9 failed, 1516 skipped"
    );
    assert_eq!(
        row(&s, " doctests :"),
        " doctests : 146 passed, 7 failed, 0 ignored"
    );
    assert!(
        !row(&s, " nextest  :").contains("311 tests run"),
        "the Lean-verdict pass's Summary line is never borrowed by the main pass:\n{s}"
    );
}

#[test]
fn the_machine_readable_line_counts_the_incomplete_pass() {
    let (_, err) = summarize(TRUNCATED);
    assert!(
        err.contains("RESULT_FAILED=342 RESULT_TIMED_OUT=2 RESULT_INCOMPLETE=1"),
        "stderr carries the same totals and the number of passes with no Summary line: {err}"
    );
    let (_, green) = summarize("nextest-green.log");
    assert!(
        green.contains("RESULT_FAILED=0 RESULT_TIMED_OUT=0 RESULT_INCOMPLETE=0"),
        "a complete run reports no incomplete pass: {green}"
    );
}

#[test]
fn a_pass_that_never_built_is_still_a_build_failure_not_an_incomplete_run() {
    let (s, err) = summarize("nextest-pass1-build-failure.log");
    let main = row(&s, " nextest  :");
    assert!(
        main.contains("FAILED TO BUILD"),
        "no test row means the pass never started: {main}"
    );
    assert!(!main.contains("INCOMPLETE"), "{main}");
    assert!(
        err.contains("RESULT_INCOMPLETE=1"),
        "a pass with no Summary line is still counted as incomplete: {err}"
    );
}

/// `(stdout, stderr)` of the summarizer over a log written from `lines`, one per line.
fn summarize_text(name: &str, lines: &[&str]) -> (String, String) {
    let log = std::env::temp_dir().join(format!(
        "run_script_summary_lock-{name}-{}-{}.log",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(&log, format!("{}\n", lines.join("\n"))).expect("write the log");
    let out = Command::new("bash")
        .arg(repo_root().join("scripts/lib/nextest-summary.sh"))
        .arg(&log)
        .output()
        .expect("run nextest-summary.sh");
    let _ = std::fs::remove_file(&log);
    assert!(
        out.status.success(),
        "nextest-summary.sh failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (
        String::from_utf8(out.stdout).expect("utf8"),
        String::from_utf8(out.stderr).expect("utf8"),
    )
}

/// A raw nextest log (no ` PASS ` markers, as a hand-run `cargo nextest run` writes it) is one main
/// pass, and the same rule holds: stopped before its Summary line, it is INCOMPLETE and counted from
/// its rows. It holds no wasm-jit pass, which is absent rather than incomplete.
#[test]
fn a_raw_nextest_log_that_stopped_is_an_incomplete_main_pass() {
    let (s, err) = summarize_text(
        "raw-truncated",
        &[
            "    Starting 40 tests across 3 binaries",
            "        PASS [   0.010s] ( 1/40) logicaffeine-tests::alpha first",
            "        FAIL [   0.020s] ( 2/40) logicaffeine-tests::alpha second",
            "        SLOW [> 60.000s] (───────────) logicaffeine-tests::beta slow_one",
            "        PASS [   0.010s] ( 3/40) logicaffeine-tests::alpha third",
            "     TIMEOUT [ 1800.001s] ( 4/40) logicaffeine-tests::beta slow_one",
            "        FAIL [   0.030s] ( 5/40) logicaffeine-tests::gamma fifth",
            "error: failed to write: No space left on device (os error 28)",
        ],
    );
    assert_eq!(
        row(&s, " nextest  :"),
        " nextest  : <INCOMPLETE — no Summary line for PASS 1/2: the main pass stopped after 5 of 40 tests; 2 failed and 1 timed out among them>"
    );
    assert!(row(&s, " wasm-jit :").contains("not in this log"), "{s}");
    assert!(
        row(&s, " totals   :")
            .starts_with(" totals   : 5 tests run, 2 failed, 1 timed out, 0 flaky — INCOMPLETE"),
        "{s}"
    );
    for test in ["alpha second", "beta slow_one", "gamma fifth"] {
        assert!(s.contains(test), "{test} is named:\n{s}");
    }
    assert!(
        !s.contains("alpha first") && !s.contains("alpha third"),
        "a passing test is not named:\n{s}"
    );
    assert!(
        err.contains("RESULT_FAILED=2 RESULT_TIMED_OUT=1 RESULT_INCOMPLETE=1"),
        "{err}"
    );

    let (complete, err) = summarize_text(
        "raw-complete",
        &[
            "    Starting 2 tests across 1 binary",
            "        PASS [   0.010s] (1/2) logicaffeine-tests::alpha first",
            "        FAIL [   0.020s] (2/2) logicaffeine-tests::alpha second",
            "     Summary [   0.030s] 2 tests run: 1 passed, 1 failed, 0 skipped",
        ],
    );
    assert_eq!(
        row(&complete, " nextest  :"),
        " nextest  : 2 tests run: 1 passed, 1 failed, 0 skipped"
    );
    assert!(
        err.contains("RESULT_FAILED=1 RESULT_TIMED_OUT=0 RESULT_INCOMPLETE=0"),
        "{err}"
    );
}

/// A test that failed an attempt and then passed on a retry is FLAKY in an incomplete pass too: it
/// is counted among the tests run, not named as a failure.
#[test]
fn an_incomplete_pass_counts_a_recovered_retry_as_flaky() {
    let (s, _) = summarize_text(
        "raw-flaky",
        &[
            "    Starting 10 tests across 1 binary",
            "  TRY 1 FAIL [   0.010s] ( 1/10) logicaffeine-tests::alpha wobbly",
            "  TRY 2 PASS [   0.010s] ( 1/10) logicaffeine-tests::alpha wobbly",
            "        FAIL [   0.010s] ( 2/10) logicaffeine-tests::alpha broken",
        ],
    );
    assert!(
        row(&s, " totals   :")
            .starts_with(" totals   : 2 tests run, 1 failed, 0 timed out, 1 flaky"),
        "{s}"
    );
    assert!(
        !s.contains("alpha wobbly"),
        "a recovered test is not named as a failure:\n{s}"
    );
    assert!(s.contains("alpha broken"), "{s}");
}
