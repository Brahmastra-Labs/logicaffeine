//! The RED/GREEN verdict must tell the truth.
//!
//! `scripts/test/tdd.sh` runs a selection of tests and demands a verdict: RED only when every
//! selected test ran and failed, GREEN only when every one ran and passed. The verdict is
//! `scripts/lib/tdd-verdict.py`, a pure function of JUnit reports, locked here against
//! fixtures cut from real `cargo nextest run --profile tdd` reports
//! (`tests/fixtures/tdd/nextest-*.xml`) plus the edge shapes a run can leave behind: an empty
//! selection, a skipped test, a report that is not XML at all.
//!
//! A verdict is only useful if it says WHY each test failed, so a RED can be checked for
//! failing for the right reason: the printed reason must be the panic's message (the
//! assertion, the `todo!()`), never the `panicked at <file>` header that precedes it.
//!
//! Both runners — the full baseline and tdd.sh — must go through the same brakes
//! (`scripts/lib/suite-guard.sh`), and the `tdd` nextest profile must never retry.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn fixture(name: &str) -> PathBuf {
    repo_root().join("crates/logicaffeine_tests/tests/fixtures/tdd").join(name)
}

struct Verdict {
    code: i32,
    out: String,
}

fn verdict(mode: &str, fixtures: &[&str]) -> Verdict {
    let script = repo_root().join("scripts/lib/tdd-verdict.py");
    assert!(script.exists(), "missing {}", script.display());
    let out = Command::new("python3")
        .arg(&script)
        .arg(mode)
        .args(fixtures.iter().map(|f| fixture(f)))
        .output()
        .expect("run tdd-verdict.py");
    Verdict {
        code: out.status.code().expect("exit code"),
        out: format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    }
}

fn line_for<'a>(out: &'a str, test: &str) -> &'a str {
    out.lines()
        .find(|l| l.contains(test) && (l.trim_start().starts_with("FAIL") || l.trim_start().starts_with("PASS") || l.trim_start().starts_with("SKIP")))
        .unwrap_or_else(|| panic!("no status line for {test} in:\n{out}"))
}

// --- verdicts ------------------------------------------------------------------------------

#[test]
fn every_test_failing_is_red() {
    let v = verdict("red", &["nextest-red.xml"]);
    assert_eq!(v.code, 0, "{}", v.out);
    assert!(v.out.contains("RED CONFIRMED — 4/4 failing"), "{}", v.out);
}

#[test]
fn every_test_failing_is_not_green() {
    let v = verdict("green", &["nextest-red.xml"]);
    assert_eq!(v.code, 1, "{}", v.out);
    assert!(v.out.contains("NOT GREEN — 4 of 4 failing"), "{}", v.out);
}

#[test]
fn every_test_passing_is_green() {
    let v = verdict("green", &["nextest-green.xml"]);
    assert_eq!(v.code, 0, "{}", v.out);
    assert!(v.out.contains("GREEN — 3/3 passing"), "{}", v.out);
}

#[test]
fn a_test_that_already_passes_is_never_red() {
    let v = verdict("red", &["nextest-green.xml"]);
    assert_eq!(v.code, 1, "{}", v.out);
    assert!(v.out.contains("NOT RED — 3 of 3 already pass"), "{}", v.out);
}

#[test]
fn a_mixed_run_is_neither_and_names_the_offenders() {
    let red = verdict("red", &["nextest-mixed.xml"]);
    assert_eq!(red.code, 1, "{}", red.out);
    assert!(red.out.contains("NOT RED — 2 of 4 already pass"), "{}", red.out);
    assert!(red.out.contains("    logicaffeine-web::content::tests::a_clean_tree_loads_with_no_issues"), "{}", red.out);

    let green = verdict("green", &["nextest-mixed.xml"]);
    assert_eq!(green.code, 1, "{}", green.out);
    assert!(green.out.contains("NOT GREEN — 2 of 4 failing"), "{}", green.out);
    assert!(green.out.contains("    logicaffeine-web::learn_state::tests::first_correct_answer_earns_base_xp"), "{}", green.out);
}

#[test]
fn a_skipped_test_is_neither_red_nor_green() {
    let red = verdict("red", &["junit-skipped.xml"]);
    assert_eq!(red.code, 1, "{}", red.out);
    assert!(red.out.contains("NOT RED — 1 of 2 skipped"), "{}", red.out);
    let green = verdict("green", &["junit-skipped.xml"]);
    assert_eq!(green.code, 1, "{}", green.out);
    assert!(green.out.contains("NOT GREEN — 1 of 2 skipped"), "{}", green.out);
}

#[test]
fn a_selection_of_nothing_proves_nothing() {
    for mode in ["red", "green"] {
        let v = verdict(mode, &["nextest-empty.xml"]);
        assert_eq!(v.code, 2, "{mode}: {}", v.out);
        assert!(v.out.contains("INVALID"), "{mode}: {}", v.out);
    }
}

#[test]
fn a_build_that_never_ran_is_invalid_not_red() {
    let v = verdict("red", &["not-xml.xml"]);
    assert_eq!(v.code, 2, "{}", v.out);
    assert!(v.out.contains("INVALID"), "{}", v.out);
    let v = verdict("red", &["no-such-report.xml"]);
    assert_eq!(v.code, 2, "{}", v.out);
}

#[test]
fn reports_combine_across_files() {
    let v = verdict("red", &["nextest-red.xml", "nextest-green.xml"]);
    assert_eq!(v.code, 1, "{}", v.out);
    assert!(v.out.contains("NOT RED — 3 of 7 already pass"), "{}", v.out);
}

#[test]
fn an_unknown_mode_is_a_usage_error() {
    let v = verdict("amber", &["nextest-red.xml"]);
    assert_eq!(v.code, 2, "{}", v.out);
}

// --- reasons -------------------------------------------------------------------------------

#[test]
fn a_failed_assertion_is_reported_by_its_message() {
    let v = verdict("red", &["nextest-red.xml"]);
    let line = line_for(&v.out, "a_malformed_exercise_is_reported_by_path");
    assert!(line.contains("assertion `left == right` failed"), "reason must be the assertion, got:\n{line}");
    assert!(!line.contains("panicked at"), "reason must not be the panic header:\n{line}");
}

#[test]
fn a_todo_stub_is_reported_as_a_stub() {
    let v = verdict("red", &["nextest-red.xml"]);
    let line = line_for(&v.out, "first_correct_answer_earns_base_xp");
    assert!(line.contains("todo!() stub — not yet implemented"), "reason must say it is a stub, got:\n{line}");
}

#[test]
fn a_custom_panic_message_is_reported_in_full_first_line() {
    let v = verdict("red", &["nextest-red.xml"]);
    let line = line_for(&v.out, "every_card_counts_its_real_exercises");
    assert!(line.contains("introduction: card says"), "reason must be the test's own message, got:\n{line}");
}

#[test]
fn a_multi_line_message_says_how_much_more_there_is() {
    // A test that lists every violation reports only its first on the verdict line; the
    // count of what follows says a second run is not needed to find the rest.
    let v = verdict("red", &["nextest-red.xml"]);
    let listing = line_for(&v.out, "every_card_counts_its_real_exercises");
    assert!(listing.ends_with("(+17 more lines)"), "got:\n{listing}");
    let assertion = line_for(&v.out, "a_malformed_exercise_is_reported_by_path");
    assert!(assertion.ends_with("(+2 more lines)"), "left/right values follow an assertion; got:\n{assertion}");
    let stub = line_for(&v.out, "first_correct_answer_earns_base_xp");
    assert!(!stub.contains("more line"), "a one-line message has nothing more; got:\n{stub}");
}

#[test]
fn a_browser_failure_is_reported_by_its_message() {
    let v = verdict("red", &["junit-skipped.xml"]);
    let line = line_for(&v.out, "word_bank_is_served_as_json");
    assert!(line.ends_with("— not JSON"), "got:\n{line}");
}

// --- the runners ---------------------------------------------------------------------------

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn both_runners_share_the_suite_guard() {
    for runner in ["scripts/run-all-tests-fast.sh", "scripts/test/tdd.sh"] {
        let text = read(runner);
        assert!(text.contains("lib/suite-guard.sh"), "{runner} must source scripts/lib/suite-guard.sh");
        for check in ["suite_guard_brakes", "suite_guard_require_nextest", "suite_guard_refuse_overlap", "suite_guard_refuse_heavy"] {
            assert!(text.contains(check), "{runner} must call {check}");
        }
        assert!(!text.contains("pgrep -x cargo-nextest"), "{runner} re-inlines the overlap guard; it lives in scripts/lib/suite-guard.sh");
    }
}

#[test]
fn tdd_runs_use_a_profile_that_never_retries_and_writes_junit() {
    let config = read(".config/nextest.toml");
    let profile = config.split("[profile.tdd]").nth(1).expect("[profile.tdd] in .config/nextest.toml");
    let body = profile.split("\n[").next().unwrap();
    assert!(body.contains("retries = 0"), "the tdd profile must not retry — a retry turns a RED into a flaky GREEN");
    assert!(body.contains("fail-fast = false"), "the tdd profile must run every selected test");
    assert!(config.contains("[profile.tdd.junit]") && config.contains("path = \"junit.xml\""), "the tdd profile must write its JUnit report");
    let tdd = read("scripts/test/tdd.sh");
    assert!(tdd.contains("--profile tdd"), "tdd.sh must run nextest with --profile tdd");
    assert!(tdd.contains("--no-tests=fail"), "tdd.sh must refuse a filter that selects nothing");
    assert!(tdd.contains("rm -f \"$REPORT\""), "tdd.sh must delete a stale report before running, or an old verdict could be read");
}

#[test]
fn the_browser_suite_can_be_required_not_skipped() {
    let tdd = read("scripts/test/tdd.sh");
    assert!(tdd.contains("--require-browser"), "tdd.sh must require the browser suite to run");
    let wrapper = read("scripts/web/test-learn-e2e.sh");
    assert!(wrapper.contains("--require-browser"), "test-learn-e2e.sh must honour --require-browser");
}
