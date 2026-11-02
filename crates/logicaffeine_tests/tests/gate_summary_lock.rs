//! The release gate's SUMMARY must tell the truth.
//!
//! `scripts/run-all-tests-fast.sh` runs three passes (nextest main, the isolated wasm-jit
//! nextest pass, and doctests) into ONE log. Its summary block used to take `tail -1` of
//! every nextest `Summary [` line — which is the tiny wasm-jit pass, not the 20,000-test
//! main pass — and listed only `FAIL` rows, so every `TIMEOUT` vanished from the block. A
//! 20,340-test run with 86 failures and 13 timeouts printed "58 tests run: 58 passed".
//!
//! The extraction now lives in `scripts/lib/nextest-summary.sh`, a pure function of a log
//! file, so it can be locked against a fixture cut from a real run
//! (`tests/fixtures/gate_summary/nextest-two-pass.log`): every pass is reported by name,
//! the totals sum every pass, and timed-out tests are listed beside the failures. The gate
//! script must call the shared summarizer — the old inline `tail -1` extraction is banned.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn summarize(fixture: &str) -> String {
    let script = repo_root().join("scripts/lib/nextest-summary.sh");
    assert!(script.exists(), "missing {}", script.display());
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
    String::from_utf8(out.stdout).expect("utf8 summary")
}

#[test]
fn the_summary_reports_the_main_pass_not_the_last_pass() {
    let s = summarize("nextest-two-pass.log");
    assert!(
        s.contains("nextest  : 20340 tests run: 20241 passed (86 slow, 1 flaky), 86 failed, 13 timed out, 3 skipped"),
        "the main pass's Summary line must be reported under `nextest`:\n{s}"
    );
    assert!(
        s.contains("wasm-jit : 58 tests run: 58 passed, 2304 skipped"),
        "the isolated wasm-jit pass must be reported under its own name:\n{s}"
    );
    assert!(
        !s.contains("nextest  : 58 tests run"),
        "the wasm-jit pass must never masquerade as the main pass:\n{s}"
    );
}

#[test]
fn the_summary_totals_every_pass() {
    let s = summarize("nextest-two-pass.log");
    assert!(
        s.contains("doctests : 4 passed, 1 failed, 0 ignored"),
        "doctest totals are summed across every `test result:` line after PASS 2/2:\n{s}"
    );
    assert!(
        s.contains("totals   : 20403 tests run, 87 failed, 13 timed out, 1 flaky"),
        "the totals line sums the nextest passes and the doctests:\n{s}"
    );
}

#[test]
fn timed_out_tests_are_listed_beside_the_failures() {
    let s = summarize("nextest-two-pass.log");
    for expected in [
        "FAIL [   0.026s] (  411/20340) logicaffeine-cli::check_deep_regression deep_finding_is_source_mapped_through_the_splice",
        "TIMEOUT [1800.024s] (20309/20340) logicaffeine-compile::riscv_aot_lock riscv_matches_the_vm_on_the_benchmark_corpus",
        "TIMEOUT [1800.022s] (20314/20340) logicaffeine-compile::scratch_rv qa_execution_differential",
    ] {
        assert!(s.contains(expected), "missing from the FAILED TESTS block: {expected}\n{s}");
    }
    assert!(
        s.contains("FAILED DOCTESTS:")
            && s.contains("test crates/logicaffeine_compile/src/repl.rs - (line 20) ... FAILED"),
        "failed doctests are listed:\n{s}"
    );
}

#[test]
fn a_green_log_summarizes_green() {
    let s = summarize("nextest-green.log");
    assert!(s.contains("nextest  : 3 tests run: 3 passed"), "{s}");
    assert!(
        s.contains("totals   : 5 tests run, 0 failed, 0 timed out, 0 flaky"),
        "{s}"
    );
    assert!(
        !s.contains("FAILED TESTS"),
        "a green log has no failure block:\n{s}"
    );
}

/// A pass whose BUILD failed prints no `Summary [` line at all. The row for that pass must say
/// so — it must never borrow the NEXT pass's summary. This shape really happened: a broken tree
/// took the main pass down, only the small wasm-jit pass ran, and an unbounded "first Summary
/// after the marker" scan printed that pass's `248 tests run: 248 passed` on BOTH rows, so the
/// gate read as a green 248-test run when nothing in the main suite had been built.
#[test]
fn a_pass_that_failed_to_build_never_borrows_the_next_passs_summary() {
    let out = summarize("nextest-pass1-build-failure.log");
    let nextest_row = out
        .lines()
        .find(|l| l.starts_with(" nextest  :"))
        .expect("a nextest row");
    assert!(
        nextest_row.contains("FAILED TO BUILD"),
        "the main pass's row must say its build failed, got: {nextest_row}"
    );
    assert!(
        !nextest_row.contains("248"),
        "the main pass's row must NOT show the wasm-jit pass's numbers: {nextest_row}"
    );
    assert!(
        out.contains(" wasm-jit : 248 tests run: 248 passed"),
        "the wasm-jit pass still reports its own real numbers:\n{out}"
    );
}

#[test]
fn the_gate_script_uses_the_shared_summarizer() {
    let gate = std::fs::read_to_string(repo_root().join("scripts/run-all-tests-fast.sh"))
        .expect("gate script");
    assert!(
        gate.contains("scripts/lib/nextest-summary.sh") || gate.contains("lib/nextest-summary.sh"),
        "run-all-tests-fast.sh must produce its summary through scripts/lib/nextest-summary.sh"
    );
    assert!(
        !gate.contains("Summary \\[' \"$LOG_FILE\" | tail -1"),
        "the inline `tail -1` Summary extraction is banned — it reported the wrong pass"
    );
}

/// A RETRIED test that exhausts its retries must be named. `.config/nextest.toml` gives the
/// `aot-cdylib` group `retries = 2`, so its final row reads `TRY 3 FAIL [...]` — and the
/// extraction pattern anchored `FAIL` to the start of the line, so the `TRY n ` prefix pushed
/// every one of them out of the block while they still counted in the totals. In the
/// 2026-09-11 baseline that hid two real failures in plain sight:
/// `aot_native_corpus::every_benchmark_runs_correctly_on_aot_native` (3/3 red — an AOT
/// narrowing bug that overflows an LCG seed) and
/// `hotswap_p17_wasm::scalar_function_builds_to_a_wasm_module`. Nobody can drive a suite to
/// green against a reporter that hides the whole AOT-native tier.
#[test]
fn a_retried_test_that_exhausts_its_retries_is_named() {
    let s = summarize("nextest-retried-failure.log");
    assert!(
        s.contains("TRY 3 FAIL [ 170.851s] (    3/6) logicaffeine-tests::aot_native_corpus every_benchmark_runs_correctly_on_aot_native"),
        "a test that failed every retry must appear in the FAILED TESTS block:\n{s}"
    );
}

/// …and a retried test that RECOVERED must not be. Its earlier attempts are still `TRY n FAIL`
/// rows in the log; listing those would report a flaky pass as a failure, which is the mirror
/// image of the bug above and just as unreadable.
#[test]
fn a_retried_test_that_recovered_is_not_named_as_a_failure() {
    let s = summarize("nextest-retried-failure.log");
    assert!(
        !s.contains("aot_native_tier_executes_and_is_dispatched"),
        "a test that passed on retry is FLAKY, not failed — it must not be listed:\n{s}"
    );
}

/// The block must account for every failure nextest counted. This is the guard that makes the
/// two tests above impossible to regress in a NEW row shape: the summarizer compares the
/// identities it names against the passes' own `N failed` / `N timed out` counts and says so,
/// loudly, when they disagree. A reporter that silently names fewer failures than it counted is
/// worse than no reporter.
#[test]
fn the_named_failures_reconcile_with_the_counted_totals() {
    let complete = summarize("nextest-retried-failure.log");
    assert!(
        !complete.contains("RECONCILIATION"),
        "3 named = 2 failed + 1 timed out, so there is nothing to warn about:\n{complete}"
    );

    // The two-pass fixture is a CUT of a real run: it carries 5 failure rows against a Summary
    // line counting 86 failed + 13 timed out. That gap must be reported, not glossed over.
    let truncated = summarize("nextest-two-pass.log");
    assert!(
        truncated.contains("RECONCILIATION"),
        "5 rows against 99 counted failures must raise the reconciliation warning:\n{truncated}"
    );
    assert!(
        truncated.contains("94"),
        "the warning must quantify the gap (99 counted - 5 named = 94 unnamed):\n{truncated}"
    );
}

/// A test that fails in BOTH nextest passes is one failing test, and each pass counts it. The
/// block names it once, so the reconciliation is per pass: every pass's own failures must be named
/// in its own section. Comparing the de-duplicated list against the SUM of the passes' counts
/// reported "1 unaccounted for" on a run that named every failure, a warning that fires on a
/// complete report and so trains readers to skip it.
#[test]
fn a_test_failing_in_both_passes_is_named_once_and_reconciles() {
    let s = summarize("nextest-failed-in-both-passes.log");
    assert_eq!(
        s.matches(
            "dom_tier_refusal_lock native_tiers_re_render_by_rebuilding_never_by_reusing_the_node"
        )
        .count(),
        1,
        "a test that failed in both passes is named exactly once:\n{s}"
    );
    assert!(
        s.contains("wasm_aot_lock the_generics_corpus_runs_on_wasm"),
        "the main pass's other failure is named:\n{s}"
    );
    assert!(
        !s.contains("RECONCILIATION"),
        "each pass's failures are all named in its own section, so there is nothing to warn about:\n{s}"
    );
}

/// The block names EVERY failure. It used to be cut at 200 lines, so a run with more failures
/// than that listed a subset while the reconciliation (computed before the cut) stayed silent.
#[test]
fn every_failure_is_named_however_many_there_are() {
    let s = summarize("nextest-over-200-failures.log");
    let named = s
        .lines()
        .filter(|l| l.contains("logicaffeine-tests::many_failures case_"))
        .count();
    assert_eq!(
        named, 205,
        "all 205 failures must be named, not the first 200:\n{s}"
    );
    assert!(
        s.contains("many_failures case_205"),
        "the last failure is named too"
    );
    assert!(
        !s.contains("RECONCILIATION"),
        "205 named = 205 counted:\n{s}"
    );
}

// ---- tracked writes: a test compares against a committed file, it never rewrites one ----

fn tracked_writes(args: &[&str]) -> std::process::Output {
    Command::new("bash")
        .arg(repo_root().join("scripts/lib/tracked-writes.sh"))
        .args(args)
        .output()
        .expect("run scripts/lib/tracked-writes.sh")
}

/// A scratch git repository holding two committed files, `ledger.json` and `golden.txt`.
fn scratch_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let git = |args: &[&str]| {
        let o = Command::new("git")
            .args([
                "-c",
                "user.name=gate",
                "-c",
                "user.email=gate@localhost",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir.path())
            .output()
            .expect("git");
        assert!(
            o.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    };
    std::fs::write(dir.path().join("ledger.json"), "{}\n").unwrap();
    std::fs::write(dir.path().join("golden.txt"), "a\n").unwrap();
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    dir
}

/// Snapshot `repo`, run `during`, snapshot again: the helper's exit code and the paths it names.
fn around(repo: &Path, during: impl FnOnce()) -> (i32, Vec<String>) {
    let (before, after) = (
        repo.join(".git").join("tracked-before"),
        repo.join(".git").join("tracked-after"),
    );
    let (before, after) = (
        before.to_str().unwrap().to_string(),
        after.to_str().unwrap().to_string(),
    );
    assert!(
        tracked_writes(&["snapshot", repo.to_str().unwrap(), &before])
            .status
            .success()
    );
    during();
    assert!(
        tracked_writes(&["snapshot", repo.to_str().unwrap(), &after])
            .status
            .success()
    );
    let o = tracked_writes(&["compare", &before, &after]);
    let named = String::from_utf8_lossy(&o.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    (o.status.code().unwrap_or(-1), named)
}

#[test]
fn a_run_that_writes_a_tracked_file_is_red_and_names_only_that_file() {
    let repo = scratch_repo();
    let r = repo.path();
    let (code, named) = around(r, || {
        std::fs::write(r.join("ledger.json"), "{\"rows\": [1]}\n").unwrap();
        std::fs::write(
            r.join("scratch.log"),
            "an untracked file is not a tracked write\n",
        )
        .unwrap();
    });
    assert_eq!((code, named), (1, vec!["ledger.json".to_string()]));
}

#[test]
fn rewriting_a_file_that_was_already_modified_is_still_a_tracked_write() {
    let repo = scratch_repo();
    let r = repo.path();
    std::fs::write(r.join("golden.txt"), "edited before the run\n").unwrap();
    let (code, named) = around(r, || {
        std::fs::write(r.join("golden.txt"), "rewritten by the run\n").unwrap()
    });
    assert_eq!((code, named), (1, vec!["golden.txt".to_string()]));
}

#[test]
fn a_run_that_leaves_every_tracked_file_as_it_found_it_is_green() {
    let repo = scratch_repo();
    let r = repo.path();
    std::fs::write(r.join("golden.txt"), "edited before the run\n").unwrap();
    let (code, named) = around(r, || {
        std::fs::write(r.join("scratch.log"), "untracked\n").unwrap()
    });
    assert_eq!((code, named), (0, Vec::<String>::new()));
}

#[test]
fn the_gate_script_turns_a_tracked_write_red() {
    let script =
        std::fs::read_to_string(repo_root().join("scripts/run-all-tests-fast.sh")).unwrap();
    let at = |needle: &str| {
        script
            .find(needle)
            .unwrap_or_else(|| panic!("run-all-tests-fast.sh lacks `{needle}`"))
    };
    let before = at("bash \"$TRACKED_WRITES\" snapshot \"$REPO_ROOT\" \"$TRACKED_BEFORE\"");
    let after = at("bash \"$TRACKED_WRITES\" snapshot \"$REPO_ROOT\" \"$TRACKED_AFTER\"");
    let compare = at("bash \"$TRACKED_WRITES\" compare \"$TRACKED_BEFORE\" \"$TRACKED_AFTER\"");
    assert!(
        before < at(" PASS 1/2 —"),
        "the first snapshot is taken before the first pass"
    );
    assert!(
        after > at(" PASS 3 —") && compare > after,
        "the second snapshot and the comparison follow the last pass"
    );
    assert!(
        script.contains("(( TRACKED_EXIT == 1 && EXIT_CODE == 0 )) && EXIT_CODE=1"),
        "a tracked write turns the run RED"
    );
    assert!(
        script.contains(" tracked  : ❌"),
        "the summary names the tracked files the run rewrote"
    );
}
