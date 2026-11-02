//! W3.1 — `test_runner_lock`: the `largo test` runner's semantics, locked.
//!
//! The `largo test` runner locks:
//!
//! 1. GOLDEN REPORT — the rendered report is deterministic, name-ordered, and
//!    byte-locked (report text is product; §1.10: two runs of the suite
//!    produce byte-identical reports).
//! 2. `op_budget_lock` — a runaway test fails AT THE IDENTICAL OP on every
//!    run (the timeout that cannot flake, counted in ops, never seconds);
//!    the `This test may run up to N steps.` escalation is honored.
//! 3. `debt_ledger_lock` — skips are loud: the reason is mandatory, named in
//!    the report, and counted (the unbudgeted-skip gate is the CLI's job;
//!    the runner counts truthfully).
//! 4. Record-and-continue — one run reports EVERY failed expectation.
//! 5. `Expect this to fail:` fails when the block SUCCEEDS; an expected
//!    message must match the actual failure.
//! 6. `Expect output:` compares the test's `Show` output byte-exactly.
//! 7. Fresh worlds — a `Let` in one test never leaks into another.
//! 8. Filtering — substring over prose names runs exactly the subset.

use logicaffeine_compile::testrun::{
    render_report, render_tier_summary, run_tests, TestConfig, Verdict,
};

fn config() -> TestConfig {
    TestConfig::default()
}

/// The golden corpus: one pass, one fail (two failed expectations — both must
/// report), one skip. Names deliberately out of order to prove name-ordering.
const GOLDEN_SUITE: &str = "\
## Test: Zebra arithmetic holds\n\
\x20   Let x be 2 plus 2.\n\
\x20   Expect that x is 4.\n\
\n\
## Test: Alpha expectations both report\n\
\x20   Let y be 1.\n\
\x20   Expect that y is 2.\n\
\x20   Expect that y is 3.\n\
\n\
## Test: Mid skip is loud\n\
\x20   Skip this test because \"the flag has not shipped\".\n\
\x20   Expect that 1 is 1.\n";

/// 1. The report is deterministic, name-ordered, and byte-locked.
#[test]
fn golden_report_is_name_ordered_and_locked() {
    let report = run_tests(GOLDEN_SUITE, &config()).expect("the golden suite parses");
    let rendered = render_report(&report);
    // Power-assert format (Wave 5): the rendered condition, then each named
    // subexpression annotated with its actual value.
    let expected = "\
FAIL  Alpha expectations both report\n\
\x20     Expected that y is 2.\n\
\x20         y → 1\n\
\x20     Expected that y is 3.\n\
\x20         y → 1\n\
SKIP  Mid skip is loud — the flag has not shipped\n\
PASS  Zebra arithmetic holds\n\
\n\
1 passed · 1 failed · 1 skipped · seed 0\n";
    assert_eq!(
        rendered, expected,
        "the report text is product — golden-locked"
    );
}

/// Determinism: two runs render byte-identically.
#[test]
fn report_is_byte_identical_across_runs() {
    let a = render_report(&run_tests(GOLDEN_SUITE, &config()).unwrap());
    let b = render_report(&run_tests(GOLDEN_SUITE, &config()).unwrap());
    assert_eq!(a, b, "same suite + same seed ⇒ byte-identical report");
}

/// 2. `op_budget_lock`: the runaway test fails at the IDENTICAL op count on
/// every run, and the message names the budget mechanism.
#[test]
fn op_budget_fires_at_the_identical_op() {
    let runaway = "\
## Test: The loop that never ends\n\
\x20   Let n be 0.\n\
\x20   While 1 is 1:\n\
\x20       Set n to n plus 1.\n";
    let mut cfg = config();
    cfg.default_budget = 10_000;
    let first = run_tests(runaway, &cfg).expect("parses");
    let second = run_tests(runaway, &cfg).expect("parses");
    assert_eq!(first.outcomes.len(), 1);
    assert_eq!(
        first.outcomes[0].verdict,
        Verdict::Failed,
        "budget exhaustion is a failure"
    );
    assert!(
        first.outcomes[0].failures[0].contains("budget"),
        "the failure names the op budget: {:?}",
        first.outcomes[0].failures
    );
    assert_eq!(
        first.outcomes[0].ops, second.outcomes[0].ops,
        "the budget fires at the identical op on every run — the timeout that cannot flake"
    );
}

/// The `This test may run up to N steps.` escalation raises the budget.
#[test]
fn budget_escalation_sentence_is_honored() {
    let escalated = "\
## Test: A long loop with a declared budget\n\
\x20   This test may run up to 2000000 steps.\n\
\x20   Let n be 0.\n\
\x20   While n is less than 20000:\n\
\x20       Set n to n plus 1.\n\
\x20   Expect that n is 20000.\n";
    let mut cfg = config();
    cfg.default_budget = 10_000; // default would fail; the sentence must win
    let report = run_tests(escalated, &cfg).expect("parses");
    assert_eq!(
        report.outcomes[0].verdict,
        Verdict::Passed,
        "the escalation sentence overrides the default budget: {:?}",
        report.outcomes[0].failures
    );
}

/// 3. `debt_ledger_lock`: the skip reason is named and counted.
#[test]
fn skips_are_loud_and_counted() {
    let report = run_tests(GOLDEN_SUITE, &config()).unwrap();
    assert_eq!(report.skipped, 1);
    let skip = report
        .outcomes
        .iter()
        .find(|o| o.verdict == Verdict::Skipped)
        .expect("the skipped test appears in the report — never silently dropped");
    assert_eq!(
        skip.skip_reason.as_deref(),
        Some("the flag has not shipped")
    );
}

/// 5. `Expect this to fail:` — success of the block is a failure; a matching
/// expected message passes.
#[test]
fn expect_fail_fails_on_success_and_matches_messages() {
    let suite = "\
## Test: Failure that arrives passes\n\
\x20   Let a be 10.\n\
\x20   Let b be 0.\n\
\x20   Expect this to fail with \"Division by zero\":\n\
\x20       Let x be a / b.\n\
\n\
## Test: Success where failure was expected\n\
\x20   Expect this to fail:\n\
\x20       Let y be 1 plus 1.\n";
    let report = run_tests(suite, &config()).expect("parses");
    let by_name = |n: &str| report.outcomes.iter().find(|o| o.name == n).unwrap();
    assert_eq!(
        by_name("Failure that arrives passes").verdict,
        Verdict::Passed,
        "{:?}",
        by_name("Failure that arrives passes").failures
    );
    let wrong = by_name("Success where failure was expected");
    assert_eq!(
        wrong.verdict,
        Verdict::Failed,
        "the block succeeded — that IS the failure"
    );
    assert!(
        wrong.failures[0].contains("succeeded"),
        "the failure says the block succeeded: {:?}",
        wrong.failures
    );
}

/// 6. `Expect output:` — byte-exact stdout comparison.
#[test]
fn expect_output_compares_byte_exactly() {
    let suite = "\
## Test: The answer prints\n\
\x20   Show 42.\n\
\x20   Expect output:\n\
\x20       42\n\
\n\
## Test: The wrong answer is caught\n\
\x20   Show 43.\n\
\x20   Expect output:\n\
\x20       42\n";
    let report = run_tests(suite, &config()).expect("parses");
    let by_name = |n: &str| report.outcomes.iter().find(|o| o.name == n).unwrap();
    assert_eq!(
        by_name("The answer prints").verdict,
        Verdict::Passed,
        "{:?}",
        by_name("The answer prints").failures
    );
    assert_eq!(
        by_name("The wrong answer is caught").verdict,
        Verdict::Failed
    );
}

/// 7. Fresh worlds: state never leaks between tests.
#[test]
fn each_test_runs_in_a_fresh_world() {
    let suite = "\
## Test: A defines x\n\
\x20   Let x be 7.\n\
\x20   Expect that x is 7.\n\
\n\
## Test: B must not see A's x\n\
\x20   Let x be 1.\n\
\x20   Expect that x is 1.\n";
    let report = run_tests(suite, &config()).expect("parses");
    assert_eq!(report.passed, 2, "both tests pass in isolated worlds");
}

/// 8. Filtering: substring over prose names.
#[test]
fn filter_selects_by_prose_substring() {
    let mut cfg = config();
    cfg.filter = Some("Zebra".to_string());
    let report = run_tests(GOLDEN_SUITE, &cfg).unwrap();
    assert_eq!(report.outcomes.len(), 1, "exactly the matching test runs");
    assert_eq!(report.outcomes[0].name, "Zebra arithmetic holds");
}

/// Table-driven tests: each data row is a NAMED sub-test — columns bind in
/// the body's scope, rows keep table order, and a failing row fails ALONE.
#[test]
fn case_table_rows_run_as_named_sub_tests() {
    let suite = "\
## Test: Addition cases\n\
\n\
For each of these cases:\n\
\n\
| a  | b | total |\n\
|----|---|-------|\n\
| 1  | 2 | 3     |\n\
| 99 | 1 | 100   |\n\
| 5  | 5 | 11    |\n\
\n\
Expect that a plus b is total.\n";
    let report = run_tests(suite, &config()).expect("parses");
    let names: Vec<&str> = report.outcomes.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Addition cases [a=1, b=2, total=3]",
            "Addition cases [a=99, b=1, total=100]",
            "Addition cases [a=5, b=5, total=11]",
        ],
        "each row is its own outcome, named by its bindings, in table order"
    );
    assert_eq!(report.passed, 2, "the true rows pass");
    assert_eq!(report.failed, 1, "the 5+5=11 row fails alone");
    let bad = report
        .outcomes
        .iter()
        .find(|o| o.verdict == Verdict::Failed)
        .unwrap();
    assert!(
        bad.name.contains("[a=5"),
        "the failing row names itself: {}",
        bad.name
    );
}

/// A test body's runtime error (outside Expect) fails the test with the error.
#[test]
fn runtime_errors_fail_the_test() {
    let suite = "\
## Test: Division by zero is a failure not a crash\n\
\x20   Let a be 10.\n\
\x20   Let b be 0.\n\
\x20   Let x be a / b.\n\
\x20   Expect that x is 1.\n";
    let report = run_tests(suite, &config()).expect("parses");
    assert_eq!(report.outcomes[0].verdict, Verdict::Failed);
    assert!(
        report.outcomes[0].failures[0].contains("Division by zero"),
        "the runtime error is the failure: {:?}",
        report.outcomes[0].failures
    );
}

/// G5 — `## Main` is a PROGRAM entry, never test fixture: its statements must
/// not execute inside any test's world. If Main leaked in, its `Show` would
/// pollute the byte-exact `Expect output:` (and a `Serve` entry would block a
/// test run on its port forever).
#[test]
fn main_statements_never_execute_during_tests() {
    let suite = "\
## Main\n\
\x20   Show \"main ran\".\n\
\n\
## Test: a test's world excludes the entry Main\n\
\x20   Show \"only me\".\n\
\x20   Expect output:\n\
\x20       only me\n";
    let report = run_tests(suite, &config()).expect("parses");
    assert_eq!(
        report.outcomes[0].verdict,
        Verdict::Passed,
        "Main's Show leaked into the test's output: {:?}",
        report.outcomes[0].failures
    );
}

/// Definitions stay in every test's world when Main is excluded — a `## To`
/// beside a `## Main` is still callable from a test.
#[test]
fn definitions_survive_main_exclusion() {
    let suite = "\
## To triple (n: Int) -> Int:\n\
\x20   Return n times 3.\n\
\n\
## Main\n\
\x20   Show triple(2).\n\
\n\
## Test: definitions reach the test world\n\
\x20   Expect that triple(5) is 15.\n";
    let report = run_tests(suite, &config()).expect("parses");
    assert_eq!(
        report.outcomes[0].verdict,
        Verdict::Passed,
        "{:?}",
        report.outcomes[0].failures
    );
}

/// G14 — the RUNNER reports which execution TIERS it actually exercised, machine-
/// readably, so a green `largo test` is legible. By DEFAULT (no `--tiers all`) only
/// the interpreter oracle runs, and the census must say `vm=off` — a fast default
/// must never masquerade as having proven cross-tier VM agreement it never ran. The
/// interpreter count is exactly the non-skipped tests.
#[test]
fn tier_summary_reports_interpreter_only_by_default() {
    let report = run_tests(GOLDEN_SUITE, &config()).expect("the golden suite parses");
    let s = report.tier_summary();
    // GOLDEN_SUITE: 1 pass + 1 fail + 1 skip → 2 executed on the interpreter.
    assert_eq!(
        s.interpreter, 2,
        "the interpreter ran every non-skipped test"
    );
    assert!(
        !s.vm_validator_ran,
        "the VM validator is OFF without --tiers all"
    );
    assert_eq!(
        render_tier_summary(&report),
        "LOGOS_TIERS_RAN: interpreter=2 vm=off\n",
        "the default census is the greppable interpreter-only line (vm=off)"
    );
}

/// Under `--tiers all` the census flips to `vm=on` and reports the VM validator's
/// verdict per test — agreed / refused / diverged — so "green" names exactly what
/// the cross-tier validator proved. A trivially-VM-runnable suite must AGREE.
#[test]
fn tier_summary_reports_vm_validator_under_tiers_all() {
    let suite = "\
## Test: Arithmetic runs on both tiers\n\
\x20   Let x be 2 plus 2.\n\
\x20   Expect that x is 4.\n";
    let mut cfg = config();
    cfg.all_tiers = true;
    let report = run_tests(suite, &cfg).expect("parses");
    let s = report.tier_summary();
    assert_eq!(s.interpreter, 1, "the interpreter ran the one test");
    assert!(
        s.vm_validator_ran,
        "the VM cross-tier validator ran under --tiers all"
    );
    assert_eq!(s.vm_agreed, 1, "the VM agreed byte-for-byte");
    assert_eq!(s.vm_refused, 0);
    assert_eq!(s.vm_diverged, 0);
    assert_eq!(
        render_tier_summary(&report),
        "LOGOS_TIERS_RAN: interpreter=1 vm=on vm_agreed=1 vm_refused=0 vm_diverged=0\n",
        "the --tiers all census names the VM validator's verdict, machine-readably"
    );
}

/// The tier census is DERIVED from the per-test evidence, so it can never drift
/// from the `tier_refused()` count the report already exposes.
#[test]
fn tier_summary_never_drifts_from_the_evidence() {
    let mut cfg = config();
    cfg.all_tiers = true;
    let report = run_tests(GOLDEN_SUITE, &cfg).expect("parses");
    let s = report.tier_summary();
    assert_eq!(
        s.vm_refused,
        report.tier_refused(),
        "the census's refused count must equal the report's own tier_refused()"
    );
}
