//! G6+G12 — STRICT engine identity + honest cross-tier evidence, locked.
//!
//! The audit found two dishonesty seams: (1) "run on the VM" could silently
//! substitute the tree-walker (`ui_bridge`'s vm-reject/async fallbacks), so two
//! differently-labeled runs could execute the SAME engine; (2) `largo test
//! --tiers all` only checked that the VM didn't RAISE — it never compared
//! output. These locks pin the strict contract:
//!
//! - `run_strict_tw` / `run_strict_vm` carry ENGINE IDENTITY in the outcome,
//!   and strict-vm REFUSES (typed: `engine_ran = None`, error says why) any
//!   program the bytecode VM cannot run — never a silent tree-walker answer.
//! - `--tiers all` compares outcome class, FULL output, and error text between
//!   the two engines on ONE no-op'd-assertions program; a VM refusal is an
//!   explicit per-test `tier: refused (…)` line — never a pass, never silent.

use logicaffeine_compile::compile::{run_strict_tw, run_strict_vm, EngineId};
use logicaffeine_compile::testrun::{
    render_report, render_report_json, run_tests, tier_divergence, TestConfig, TierEvidence,
    Verdict,
};
use logicaffeine_compile::ui_bridge::{
    interpret_for_ui_sync_with_args, set_engine, Engine, InterpreterResult,
};

fn run_ui_on(engine: Engine, src: &str) -> InterpreterResult {
    struct ResetEngine;
    impl Drop for ResetEngine {
        fn drop(&mut self) {
            set_engine(Engine::Auto);
        }
    }

    set_engine(engine);
    let _reset = ResetEngine;
    interpret_for_ui_sync_with_args(src, &[])
}

/// A program BOTH engines run: identities differ, outcomes are byte-identical.
#[test]
fn both_engines_run_with_distinct_identities_and_identical_outcomes() {
    let src = "## Main\n\
        \x20   Let mutable total be 0.\n\
        \x20   Repeat for i from 1 to 5:\n\
        \x20       Set total to total + i.\n\
        \x20   Show total.\n\
        \x20   Show \"done\".\n";
    let tw = run_strict_tw(src);
    let vm = run_strict_vm(src);
    assert_eq!(
        tw.engine_ran,
        Some(EngineId::TreeWalker),
        "tw label ⇒ the tree-walker ran"
    );
    assert_eq!(
        vm.engine_ran,
        Some(EngineId::Vm),
        "vm label ⇒ the bytecode VM ran"
    );
    assert_ne!(
        tw.engine_ran, vm.engine_ran,
        "the two labels must never resolve to the same engine"
    );
    assert_eq!(EngineId::TreeWalker.label(), "tree-walker");
    assert_eq!(EngineId::Vm.label(), "vm");
    assert_eq!(
        tw.output, vm.output,
        "byte-identical output is the differential contract"
    );
    assert_eq!(tw.error, None, "clean program");
    assert_eq!(vm.error, None, "clean program");
    assert_eq!(tw.output, "15\ndone");
}

/// A program only the tree-walker runs (async: `Sleep`): strict-vm is a TYPED
/// refusal — nothing ran, the error says why — never the tree-walker's answer.
#[test]
fn strict_vm_refuses_an_async_program_never_substituting_the_tree_walker() {
    let src = "## Main\n\
        \x20   Show \"before\".\n\
        \x20   Sleep 1.\n\
        \x20   Show \"after\".\n";
    let tw = run_strict_tw(src);
    assert_eq!(tw.engine_ran, Some(EngineId::TreeWalker));
    assert_eq!(tw.error, None, "the tree-walker runs it: {:?}", tw.error);
    assert_eq!(tw.output, "before\nafter");

    let vm = run_strict_vm(src);
    assert_eq!(
        vm.engine_ran, None,
        "STRICT: nothing ran — no silent tree-walker substitution"
    );
    assert!(
        vm.refused(),
        "engine_ran = None + error = the typed refusal"
    );
    assert_eq!(
        vm.output, "",
        "a refusal carries NO output — especially not the tree-walker's"
    );
    let why = vm.error.as_deref().expect("the refusal says WHY");
    assert!(
        why.contains("async") || why.contains("sync-only"),
        "the refusal names the capability boundary: {why}"
    );
}

/// A concurrency program (tasks need the scheduler driver): strict-vm refuses,
/// strict-tw runs it on the tree-walker's deterministic scheduler.
#[test]
fn strict_vm_refuses_a_scheduler_program() {
    let src = "## To worker:\n\
        \x20   Show \"worker\".\n\
        \n\
        ## Main\n\
        \x20   Launch a task to worker.\n\
        \x20   Show \"main\".\n";
    let tw = run_strict_tw(src);
    assert_eq!(tw.engine_ran, Some(EngineId::TreeWalker));
    assert_eq!(tw.error, None, "{:?}", tw.error);
    assert!(
        tw.output.contains("worker") && tw.output.contains("main"),
        "{:?}",
        tw.output
    );

    let vm = run_strict_vm(src);
    assert_eq!(
        vm.engine_ran, None,
        "no scheduler driver on the plain VM ⇒ typed refusal"
    );
    assert_eq!(vm.output, "");
    assert!(
        vm.error.as_deref().is_some_and(|e| e.contains("scheduler")),
        "the refusal names the scheduler boundary: {:?}",
        vm.error
    );
}

/// The public UI/CLI selector is strict too: an explicitly selected VM never
/// inherits the product dispatcher's scheduler fallback.
#[test]
fn ui_bridge_explicit_vm_refuses_scheduler_instead_of_falling_back() {
    let src = "## To worker:\n\
        \x20   Show \"worker\".\n\
        \n\
        ## Main\n\
        \x20   Launch a task to worker.\n\
        \x20   Show \"main\".\n";

    let vm = run_ui_on(Engine::Vm, src);
    assert!(
        vm.lines.is_empty(),
        "a refusal carries no tree-walker output"
    );
    assert!(
        vm.error.as_deref().is_some_and(|e| e.contains("scheduler")),
        "the explicit VM refusal names its capability boundary: {:?}",
        vm.error
    );

    let tree = run_ui_on(Engine::TreeWalker, src);
    assert_eq!(
        tree.error, None,
        "the explicitly selected tree-walker runs it"
    );
    assert!(
        tree.lines.iter().any(|line| line == "worker")
            && tree.lines.iter().any(|line| line == "main"),
        "the actual tree-walker produced both task outputs: {:?}",
        tree.lines
    );
}

/// Unparseable source runs on NEITHER engine — both report nothing-ran with the
/// parse story; neither invents an execution.
#[test]
fn a_parse_error_runs_nothing_on_either_engine() {
    let src = "## Main\n\x20   Blorp the frobnicate quux.\n";
    let tw = run_strict_tw(src);
    let vm = run_strict_vm(src);
    assert_eq!(tw.engine_ran, None);
    assert_eq!(vm.engine_ran, None);
    assert!(tw.error.is_some(), "the parse failure is the error");
    assert!(vm.error.is_some(), "the parse failure is the error");
    assert_eq!(tw.output, "");
    assert_eq!(vm.output, "");
}

/// Runtime errors keep engine identity: BOTH engines genuinely ran (partial
/// output preserved) and agree on the error text — the differential oracle.
#[test]
fn runtime_errors_carry_engine_identity_and_agree() {
    let src = "## Main\n\
        \x20   Show \"start\".\n\
        \x20   Let a be 10.\n\
        \x20   Let b be 0.\n\
        \x20   Show a / b.\n";
    let tw = run_strict_tw(src);
    let vm = run_strict_vm(src);
    assert_eq!(
        tw.engine_ran,
        Some(EngineId::TreeWalker),
        "it RAN and then errored"
    );
    assert_eq!(vm.engine_ran, Some(EngineId::Vm), "it RAN and then errored");
    assert!(tw.error.is_some() && vm.error.is_some());
    assert_eq!(tw.error, vm.error, "error text agrees across engines");
    assert_eq!(tw.output, vm.output, "partial output agrees across engines");
    assert_eq!(tw.output, "start");
}

/// The differential-oracle sanity: a diverse corpus produces byte-identical
/// outcomes on both strict engines, each under its own honest identity.
#[test]
fn differential_oracle_sanity_over_a_corpus() {
    let corpus = [
        "## Main\n\x20   Show 21 times 2.\n",
        "## Main\n\x20   Let mutable s be 0.\n\x20   Repeat for i from 1 to 10:\n\x20       Set s to s + i.\n\x20   Show s.\n",
        "## Main\n\x20   Let x be 7.\n\x20   If x is greater than 5:\n\x20       Show \"big\".\n\x20   Otherwise:\n\x20       Show \"small\".\n",
        "## Main\n\x20   Let xs be [3, 1, 2].\n\x20   Show the length of xs.\n",
        "## Main\n\x20   Let name be \"world\".\n\x20   Show name.\n",
    ];
    for src in corpus {
        let tw = run_strict_tw(src);
        let vm = run_strict_vm(src);
        assert_eq!(tw.engine_ran, Some(EngineId::TreeWalker), "{src}");
        assert_eq!(vm.engine_ran, Some(EngineId::Vm), "{src}");
        assert_eq!(
            (tw.output, tw.error),
            (vm.output, vm.error),
            "corpus program: {src}"
        );
    }
}

// ───────────────────────── `--tiers all` honesty ─────────────────────────

fn all_tiers() -> TestConfig {
    TestConfig {
        all_tiers: true,
        ..TestConfig::default()
    }
}

/// A tier-stable suite under `--tiers all`: every passing test carries the
/// machine-readable AGREED evidence — proof the comparison genuinely ran.
#[test]
fn tiers_all_records_agreement_evidence_per_test() {
    let suite = "## Test: arithmetic accumulates\n\
        \x20   Let mutable total be 0.\n\
        \x20   Repeat for i from 1 to 5:\n\
        \x20       Set total to total + i.\n\
        \x20   Show total.\n\
        \x20   Expect that total is 15.\n";
    let report = run_tests(suite, &all_tiers()).expect("parses");
    assert_eq!(report.passed, 1, "{:?}", report.outcomes);
    assert_eq!(
        report.outcomes[0].tier,
        Some(TierEvidence::Agreed),
        "a passing test under --tiers all carries the agreement evidence"
    );
    assert_eq!(report.tier_refused(), 0);
}

/// Without the flag no tier evidence is claimed — evidence is never invented.
#[test]
fn without_the_flag_no_tier_evidence_is_claimed() {
    let suite = "## Test: plain\n\x20   Expect that 1 is 1.\n";
    let report = run_tests(suite, &TestConfig::default()).expect("parses");
    assert_eq!(
        report.outcomes[0].tier, None,
        "no --tiers ⇒ no evidence field"
    );
}

/// A test whose computation the VM cannot run (async `Sleep`): the flag makes
/// it an explicit `tier: refused (…)` — NEVER a pass, never a silent skip —
/// with the machine-readable field in the JSON.
#[test]
fn tiers_all_refusal_is_loud_never_a_pass() {
    let suite = "## Test: a nap the VM cannot take\n\
        \x20   Show \"awake\".\n\
        \x20   Sleep 1.\n\
        \x20   Expect that 1 is 1.\n";
    // Baseline: the interpreter alone passes it.
    let plain = run_tests(suite, &TestConfig::default()).expect("parses");
    assert_eq!(plain.passed, 1, "{:?}", plain.outcomes);

    // Under --tiers all it is refused, loudly.
    let report = run_tests(suite, &all_tiers()).expect("parses");
    assert_eq!(report.passed, 0, "a tier refusal is NEVER a pass");
    assert_eq!(
        report.failed, 1,
        "and never a silent skip: {:?}",
        report.outcomes
    );
    assert_eq!(report.tier_refused(), 1, "counted in the report");
    let outcome = &report.outcomes[0];
    assert_eq!(outcome.verdict, Verdict::Failed);
    assert!(
        matches!(outcome.tier, Some(TierEvidence::Refused { .. })),
        "machine-readable refusal evidence: {:?}",
        outcome.tier
    );
    assert!(
        outcome
            .failures
            .iter()
            .any(|f| f.starts_with("tier: refused (")),
        "the explicit per-test line: {:?}",
        outcome.failures
    );
    // The human report names it; the JSON carries the machine field.
    assert!(
        render_report(&report).contains("tier: refused ("),
        "{}",
        render_report(&report)
    );
    let json = render_report_json(&report);
    assert!(
        json.contains("refused"),
        "machine-readable in the JSON: {json}"
    );
}

/// The comparison itself, unit-locked with synthetic divergent inputs: a
/// divergence in CLASS, in FULL output (byte-for-byte), or in error text is
/// caught — the audit's finding was that only error-presence was checked.
#[test]
fn tier_divergence_catches_every_axis() {
    // Perfect agreement ⇒ no divergence.
    assert_eq!(tier_divergence("a\nb", None, "a\nb", None), None);
    assert_eq!(
        tier_divergence("x", Some("Division by zero"), "x", Some("Division by zero")),
        None,
        "identical failures on both engines agree"
    );

    // FULL output, byte-for-byte: an interior difference is a divergence.
    let d = tier_divergence("line1\nline2", None, "line1\nline2 ", None)
        .expect("an interior byte difference diverges");
    assert!(d.contains("TIER DIVERGENCE"), "{d}");

    // Outcome class: interpreter clean, VM errored.
    let d =
        tier_divergence("out", None, "out", Some("vm exploded")).expect("class mismatch diverges");
    assert!(d.contains("TIER DIVERGENCE"), "{d}");

    // Error text: both errored, texts differ.
    let d = tier_divergence("", Some("Division by zero"), "", Some("divide error"))
        .expect("error text mismatch diverges");
    assert!(d.contains("TIER DIVERGENCE"), "{d}");

    // Output mismatch even when both are error-free and non-empty.
    let d = tier_divergence("42", None, "43", None).expect("output mismatch diverges");
    assert!(
        d.contains("42") && d.contains("43"),
        "the bundle shows both sides: {d}"
    );
}

/// End-to-end mechanism proof: multi-line output rides the comparison (a same-
/// output fixture agrees; the unit test above proves a divergent one cannot
/// slip through — no honest tier-divergent program exists while the engines
/// are differentially green, which is exactly the point).
#[test]
fn tiers_all_compares_full_multiline_output() {
    let suite = "## Test: three lines survive the crossing\n\
        \x20   Repeat for i from 1 to 3:\n\
        \x20       Show i.\n\
        \x20   Expect that 1 is 1.\n";
    let report = run_tests(suite, &all_tiers()).expect("parses");
    assert_eq!(report.passed, 1, "{:?}", report.outcomes);
    assert_eq!(report.outcomes[0].tier, Some(TierEvidence::Agreed));
}

/// Properties (`For all`) are the interpreter's own machinery: under
/// `--tiers all` the surrounding computation still crosses tiers (the property
/// statement no-ops on BOTH engines), so a property test earns evidence too.
#[test]
fn a_property_test_still_earns_tier_evidence() {
    let suite = "## Test: a property and a computation\n\
        \x20   Let base be 2 times 3.\n\
        \x20   Show base.\n\
        \x20   For all integers n:\n\
        \x20       Expect that n plus n is divisible by 2.\n";
    let report = run_tests(suite, &all_tiers()).expect("parses");
    assert_eq!(report.passed, 1, "{:?}", report.outcomes);
    assert_eq!(report.outcomes[0].tier, Some(TierEvidence::Agreed));
}
