//! The `largo test` runner core.
//!
//! Studio-reusable: everything here is CLI-independent. The runner discovers
//! `## Test` blocks in a parsed program, executes each in a FRESH interpreter
//! world (fixture pollution is structurally impossible), applies the
//! constitution's semantics — `Expect` records-and-continues, `Expect this to
//! fail` fails on success, `Expect output:` compares byte-exact, deterministic
//! op budgets, loud skips — and renders a DETERMINISTIC report (name-ordered,
//! byte-identical across `-j N`; the golden in `test_runner_lock.rs`).

use serde::Serialize;

/// One test's verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Passed,
    /// Failed — with every recorded expectation failure (record-and-continue:
    /// one run reports everything, not just the first).
    Failed,
    /// Skipped via `Skip this test because "…"` — always loud, always counted.
    Skipped,
}

/// Cross-tier evidence for one test under `--tiers all` (footgun F18): what
/// the bytecode VM did with the test's computation, machine-readable in the
/// JSON report. Absent (`None` on [`TestOutcome::tier`]) when the guard did
/// not run — evidence is never invented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TierEvidence {
    /// The VM re-ran the computation and agreed on every axis — outcome
    /// class, FULL output byte-for-byte, and error text.
    Agreed,
    /// The VM cannot run this computation; `reason` says why. Reported as an
    /// explicit `tier: refused (…)` failure — never a pass, never a silent
    /// skip.
    Refused { reason: String },
    /// The VM ran and DISAGREED — always a compiler bug; `detail` carries the
    /// divergence bundle.
    Diverged {
        detail: String,
        /// The first top-level statement the tiers disagreed at, for a MOUNTED test compared
        /// through `page_host` (`0` is the program itself, `k` its `k`-th test statement).
        /// `None` for the plain interpreter/bytecode-VM computation comparison, which has no
        /// page statements to number.
        #[serde(skip_serializing_if = "Option::is_none")]
        statement: Option<usize>,
        /// The `.lg` source line that statement is written on. Always `None` today — no `Stmt`
        /// carries a span yet, and a line is never guessed; the field exists so a consumer can
        /// start reading it the day one does.
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<u32>,
    },
}

/// The finite domain a `For all` property quantified over, with the concrete
/// bound the runner actually enumerated/sampled — the disclosure that keeps a
/// green property from silently meaning "unbounded proof" (G13-full). Carried on
/// the [`TestOutcome`] so the inventory can classify a property honestly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertyDomain {
    /// `For all integers …:` — sampled over `[-1000, 1000]` (infinite domain, so
    /// never a proof, even under `--prove`).
    Integers,
    /// `For all naturals …:` — sampled over `[0, 1000]` (infinite domain).
    Naturals,
    /// `For all booleans …:` — the only FINITE domain, exhausted under `--prove`.
    Booleans,
}

impl PropertyDomain {
    /// The human-readable, disclosed bound (`[-1000, 1000]` etc.) — what the
    /// runner ACTUALLY covered, never a claim of unbounded truth.
    pub fn bound(self) -> &'static str {
        match self {
            PropertyDomain::Integers => "integers sampled over [-1000, 1000]",
            PropertyDomain::Naturals => "naturals sampled over [0, 1000]",
            PropertyDomain::Booleans => "booleans (finite: {false, true})",
        }
    }

    /// Whether this domain is FINITE — the only domain a property can be PROVED
    /// over by exhaustion (booleans). Infinite domains are only ever sampled.
    pub fn is_finite(self) -> bool {
        matches!(self, PropertyDomain::Booleans)
    }
}

/// The `For all` property a test exercised, if any — its domain (for bound
/// disclosure) and how many variables it quantified (for the exhaustion bound).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PropertyInfo {
    pub domain: PropertyDomain,
    pub vars: usize,
}

/// One executed (or skipped) test.
#[derive(Debug, Clone, Serialize)]
pub struct TestOutcome {
    /// The prose name — the test's identity.
    pub name: String,
    pub verdict: Verdict,
    /// Every failure recorded during the run (empty for pass/skip).
    pub failures: Vec<String>,
    /// The mandatory skip reason, when skipped.
    pub skip_reason: Option<String>,
    /// Ops executed (deterministic; identical across machines and `-j N`).
    pub ops: u64,
    /// True when the test ran clean but asserted NOTHING (no `Expect`/`For all`)
    /// — the vacuous-green the non-vacuity gate fails; surfaced here so the
    /// inventory can flag it explicitly (G13-full), not just fail it.
    pub vacuous: bool,
    /// Whether this test MOUNTED a page — the honest signal that it drives a document rather than
    /// checking a value, and what makes it a `Component` claim in the inventory. Read from the
    /// test's own body, so it says what the test DOES rather than what the run was configured to
    /// do: a project with both kinds must tell them apart within one document.
    pub mounted: bool,
    /// The `For all` property this test exercised, if any — `None` for a plain
    /// `Expect`-only example. Drives the inventory's property classification and
    /// its bound disclosure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property: Option<PropertyInfo>,
    /// Whether the run was under `--prove` (finite-domain exhaustion escalation).
    /// Carried so the inventory can distinguish an exhausted proof-by-cases from
    /// a sampled hold without re-reading the config.
    pub proved_by_exhaustion: bool,
    /// Cross-tier evidence under `--tiers all`; `None` when the guard did not
    /// run this test (flag off, or the test already failed/skipped).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<TierEvidence>,
}

/// The whole run, name-ordered (report order is part of determinism).
#[derive(Debug, Clone, Serialize)]
pub struct RunReport {
    pub seed: u64,
    pub outcomes: Vec<TestOutcome>,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    /// How many tests exercised a `For all` property (adversarial evidence).
    pub properties: usize,
    /// How many properties were PROVED by exhaustion (finite domains, `--prove`).
    pub proven: usize,
    /// The page tiers this run was CONFIGURED to check a mounted page against
    /// (`config.page_tiers`, `Ast` excluded — that one is the reference, never a "candidate"),
    /// labeled (`PageTierKind::label()`). `render_tier_summary` names them so a green `largo test
    /// --browser` cannot claim to have checked a tier it never launched, the way `vm=on` alone
    /// could not distinguish "the lowered VM only" from "the lowered VM AND a real Chrome."
    pub page_tiers: Vec<String>,
}

/// The earned rank of a run — a
/// ladder computed from EVIDENCE, never self-declared. The higher rungs
/// (COVERED/HARDENED/PROVEN/BATTLE-READY) require coverage, mutation, proofs,
/// and the full inspection, which light up as those waves land; the runner
/// computes the base rungs honestly today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rank {
    /// It compiles. Nothing is tested.
    Untried,
    /// A test failed — no rank is earned.
    Failing,
    /// All tests pass; deterministic; zero unbudgeted debts.
    Green,
    /// Green + at least one `For all` property held over its generated cases.
    Hardened,
    /// Hardened + at least one property PROVED by exhaustion (a certificate,
    /// not a sample).
    Proven,
}

impl Rank {
    pub fn label(self) -> &'static str {
        match self {
            Rank::Untried => "UNTRIED",
            Rank::Failing => "FAILING",
            Rank::Green => "GREEN",
            Rank::Hardened => "HARDENED",
            Rank::Proven => "PROVEN",
        }
    }
}

impl RunReport {
    /// True when the run is green: no failures AND no unbudgeted skips
    /// (the caller applies the skip budget — the runner only counts).
    pub fn all_passed(&self) -> bool {
        self.failed == 0
    }

    /// How many tests the compiled tier REFUSED under `--tiers all` — derived
    /// from the per-test evidence, so the count can never drift from it.
    pub fn tier_refused(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o.tier, Some(TierEvidence::Refused { .. })))
            .count()
    }

    /// The earned rank, from the run's evidence.
    pub fn rank(&self) -> Rank {
        if self.outcomes.is_empty() {
            Rank::Untried
        } else if self.failed > 0 {
            Rank::Failing
        } else if self.proven > 0 {
            Rank::Proven
        } else if self.properties > 0 {
            Rank::Hardened
        } else {
            Rank::Green
        }
    }
}

/// Render the fitness report (§1.12) — the last block of every run: the earned
/// rank and the evidence behind it, with the next rung named.
pub fn render_fitness(report: &RunReport) -> String {
    let rank = report.rank();
    let next = match rank {
        Rank::Untried => "write a `## Test` — nothing is checked yet.",
        Rank::Failing => "fix the failing test(s) to reach GREEN.",
        Rank::Green => "add a `For all` property to reach HARDENED.",
        Rank::Hardened => "run with `--prove` to certify a finite-domain property → PROVEN.",
        Rank::Proven => "COVERED/BATTLE-READY need coverage, mutation, and the full `--inspect`.",
    };
    format!(
        "════════════════════ FITNESS REPORT ════════════════════\n\
         \x20RANK: {}\n\
         \x20tests {} · properties {} (proven {}) · passed {} · failed {} · skipped {}\n\
         \x20Next rank: {}\n\
         ═════════════════════════════════════════════════════════\n",
        rank.label(),
        report.outcomes.len(),
        report.properties,
        report.proven,
        report.passed,
        report.failed,
        report.skipped,
        next,
    )
}

/// Runner configuration.
#[derive(Debug, Clone)]
pub struct TestConfig {
    /// Substring filter over prose names (None = all).
    pub filter: Option<String>,
    /// The suite seed (default 0 — reproducibility is the default).
    pub seed: u64,
    /// The default deterministic op budget per test; a test's
    /// `This test may run up to N steps.` sentence overrides it.
    pub default_budget: u64,
    /// Run capability-granted (real-resource) tests too. Off by default —
    /// live tests leave the no-flake guarantee and are counted separately.
    pub live: bool,
    /// `--prove`: escalate finite-domain properties to exhaustive proof.
    pub prove: bool,
    /// `--tiers all`: re-run each passing test's COMPUTATION on the bytecode VM
    /// and COMPARE — outcome class, FULL output byte-for-byte, and error text
    /// (a mismatch fails the test as a TIER DIVERGENCE); a computation the VM
    /// cannot run is an explicit `tier: refused (…)` failure with
    /// machine-readable [`TierEvidence`] — the user-facing F18
    /// "works-on-my-tier" guard (Wave 13), honest by construction.
    pub all_tiers: bool,
    /// Which page tiers a MOUNTED test also runs on and must agree with the reference (the page's
    /// lowered LOGOS on the tree-walker) — every candidate here beyond
    /// [`crate::page_host::PageTierKind::LoweredHere`] runs
    /// the test's whole trace through `page_host` and compares it against the reference axis by
    /// axis (`page_host::PageTrace::compare`), unconditionally, on every `largo test` run: "what
    /// ships is not what is tested" is caught for free, not only under `--tiers all`. Defaults to
    /// `[LoweredHere, LoweredVm]` — every mounted test also runs on the page's lowered LOGOS on the
    /// bytecode VM, the tier a browser actually runs. A `Browser` candidate here is exercised
    /// only when the caller hands [`run_tests_with_launcher`] a live launcher; without one it is
    /// reported as `tier: refused` rather than silently skipped.
    pub page_tiers: Vec<crate::page_host::PageTierKind>,
}

/// The candidate page tiers `config` checks a mounted page against, labeled — `LoweredHere`
/// excluded (it is the reference every candidate is compared to, never a candidate itself). Shared by both
/// [`RunReport`] construction sites so the label set can never drift between them.
fn page_tier_labels(config: &TestConfig) -> Vec<String> {
    config
        .page_tiers
        .iter()
        .filter(|k| **k != crate::page_host::PageTierKind::LoweredHere)
        .map(|k| k.label().to_lowercase().replace(' ', "-"))
        .collect()
}

impl Default for TestConfig {
    fn default() -> Self {
        TestConfig {
            filter: None,
            seed: 0,
            default_budget: 500_000_000,
            live: false,
            prove: false,
            all_tiers: false,
            page_tiers: vec![
                crate::page_host::PageTierKind::LoweredHere,
                crate::page_host::PageTierKind::LoweredVm,
            ],
        }
    }
}

/// Compare one test computation's run on the interpreter against its run on
/// the bytecode VM — the `--tiers all` comparison, PURE so it is unit-lockable
/// with synthetic divergent inputs. Both outputs are newline-joined lines (the
/// shared engine convention), compared byte-for-byte in FULL; error text is
/// compared exactly; the outcome class (clean vs errored) falls out of the
/// error comparison. Returns `None` on agreement, or the divergence bundle.
pub fn tier_divergence(
    interp_output: &str,
    interp_error: Option<&str>,
    vm_output: &str,
    vm_error: Option<&str>,
) -> Option<String> {
    let outputs_agree = interp_output == vm_output;
    let errors_agree = interp_error == vm_error;
    if outputs_agree && errors_agree {
        return None;
    }
    let mut axes = Vec::new();
    if interp_error.is_some() != vm_error.is_some() {
        axes.push("outcome class (one engine errored, the other ran clean)");
    } else if !errors_agree {
        axes.push("error text");
    }
    if !outputs_agree {
        axes.push("output");
    }
    Some(format!(
        "TIER DIVERGENCE (compiler bug — F18): this test passes on the interpreter, but \
         re-running its computation on the bytecode VM disagreed on: {}.\n\
         interpreter: output={:?} error={:?}\n\
         bytecode VM: output={:?} error={:?}",
        axes.join(", "),
        interp_output,
        interp_error,
        vm_output,
        vm_error,
    ))
}

/// Render a case-table cell for its sub-test's name (`[a=99, b=1]`): literal
/// cells render exactly; computed cells render as an ellipsis.
fn render_cell(
    cell: &logicaffeine_language::ast::stmt::Expr<'_>,
    interner: &crate::intern::Interner,
) -> String {
    use logicaffeine_language::ast::stmt::{Expr, Literal};
    match cell {
        Expr::Literal(Literal::Number(n)) => n.to_string(),
        Expr::Literal(Literal::Float(f)) => f.to_string(),
        Expr::Literal(Literal::Boolean(b)) => b.to_string(),
        Expr::Literal(Literal::Text(s)) => format!("{:?}", interner.resolve(*s)),
        _ => "…".to_string(),
    }
}

/// What one test's authoritative tree-walker run hands back out of its session.
struct SessionRun {
    /// The run's own outcome — `None` when it needs an async runtime this target cannot drive.
    result: Option<Result<(), String>>,
    /// Every recorded expectation failure.
    failures: Vec<String>,
    assertions: u64,
    /// Properties this run proved by exhaustion.
    proven: u64,
    ops: u64,
}

/// Discover and execute every `## Test` in `source`. Each test runs in a
/// fresh interpreter world; the report is deterministic and name-ordered.
///
/// Returns `Err` only for program-level problems (parse errors); an individual
/// test failing is a `Failed` outcome, never an `Err`.
pub fn run_tests(source: &str, config: &TestConfig) -> Result<RunReport, String> {
    run_tests_with_launcher(source, config, None)
}

/// [`run_tests`], with a live [`crate::page_host::RemotePageLauncher`] so a MOUNTED test's
/// [`TestConfig::page_tiers`] can also exercise a `Browser` candidate — the seam `largo test
/// --browser` calls through once a real one exists (`apps/logicaffeine_cli/src/browser/**`).
/// `None` reports a `Browser` candidate as `tier: refused`, never a silent skip.
pub fn run_tests_with_launcher(
    source: &str,
    config: &TestConfig,
    launcher: Option<&dyn crate::page_host::RemotePageLauncher>,
) -> Result<RunReport, String> {
    use logicaffeine_language::ast::stmt::Stmt;

    // `## Main` is the PROGRAM's entry point, never test fixture: strip it
    // before parsing so no test's world ever executes it. A `Serve` entry must
    // not bind its port (or block forever) while pure tests run, and a Main
    // `Show` must not pollute a byte-exact `Expect output:`. Definitions
    // (`## To`, types, theorems) all survive — only the Main block goes.
    let source = crate::analysis::assemble::strip_main_block(source);
    if source.trim().is_empty() {
        // Nothing but a Main (or nothing at all): no tests can exist, and an
        // empty unit has no parse errors to report.
        return Ok(RunReport {
            seed: config.seed,
            outcomes: Vec::new(),
            passed: 0,
            failed: 0,
            skipped: 0,
            properties: 0,
            proven: 0,
            page_tiers: page_tier_labels(config),
        });
    }

    crate::ui_bridge::with_parsed_program(&source, |parsed, interner| {
        let (stmts, types, policies) = match parsed {
            Ok(t) => t,
            Err(advice) => return Err(advice),
        };

        // The shared world every test starts from: everything EXCEPT the
        // tests themselves (functions, types, theorems — the program's defs).
        let defs: Vec<Stmt> = stmts
            .iter()
            .filter(|s| !matches!(s, Stmt::TestDef { .. }))
            .cloned()
            .collect();

        // Discover, filter, and name-sort the tests (report order is part of
        // determinism — never completion order).
        let mut tests: Vec<&Stmt> = stmts
            .iter()
            .filter(|s| matches!(s, Stmt::TestDef { .. }))
            .collect();
        tests.sort_by_key(|s| match s {
            Stmt::TestDef { name, .. } => interner.resolve(*name).to_string(),
            _ => unreachable!("filtered to TestDef above"),
        });

        // Properties proved by exhaustion across the whole run (`--prove`).
        let proven_total = std::cell::Cell::new(0usize);

        // Execute one program (shared defs + a body) in a FRESH interpreter
        // AND a fresh sealed world (§1.7) — state can never leak between
        // tests or between table rows, and no unrecorded entropy exists.
        let execute = |program: &[Stmt],
                       body: &[Stmt],
                       name: String,
                       budget: u64,
                       caps: logicaffeine_system::testworld::Capabilities,
                       property: Option<PropertyInfo>,
                       mounted: bool|
         -> TestOutcome {
            // Identity-keyed world seed: adding or reordering tests never
            // shifts another test's stream (constitution D1.5).
            let mut h: u64 = 0xCBF2_9CE4_8422_2325 ^ config.seed;
            for b in name.bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01B3);
            }
            // A page test's world is more than the clock/entropy/env/fs `testworld` seals: the
            // document, its route, the journal, and the browser storage areas (local/session/shared)
            // are their own thread-locals (`semantics::dom`, `semantics::world`) that a fresh
            // `Interpreter` does not touch. Reset them too, exactly as `page_host::World::enter`
            // does, or a second test in the same file inherits the first one's page — and, for
            // `shared on`, the first one's TOPIC. A `## Page` boots at its own declared URL, not the
            // reset default `"/"` — `program` already carries the page's
            // `ComponentDef` and its auto-inserted `Mount`, so `routePath()` reads the real one from
            // this test's very first statement, on the tree-walker exactly as `page_host::Scenario`
            // already seeds it for the lowered VM and browser tiers.
            // The run is a tree-walker SESSION: its world, its program and everything read back
            // from it happen on one thread whose stack holds the full call depth
            // (`interpreter::on_session_stack`), and only plain data comes back.
            let run = crate::interpreter::on_session_stack(|| {
                crate::semantics::dom::reset();
                crate::page_host::boot_at_declared_route(program, interner);
                crate::semantics::drive::reset_clock();
                crate::semantics::world::reset();
                crate::concurrency::net_inbox::set_net_offline(true);
                logicaffeine_system::testworld::install(
                    logicaffeine_system::testworld::TestWorld::new(caps, h),
                );
                let mut interp = crate::interpreter::Interpreter::new(interner)
                    .with_type_registry(types)
                    .with_policies(policies.clone());
                // Hermetic tests get the sealed in-memory filesystem (§1.7): the
                // Vfs-backed `Read`/`Write` statements and `Given a file …` share
                // one source of truth. A filesystem-capability test gets real disk.
                if !caps.filesystem {
                    interp = interp
                        .with_vfs(std::sync::Arc::new(logicaffeine_system::fs::MemVfs::new()));
                }
                interp.enable_test_mode(budget);
                interp.set_prove_mode(config.prove);
                // Property generation is keyed by test identity (the same `h`), so a
                // counterexample is stable across runs and independent of test order.
                interp.set_gen_seed(h ^ 0x5DEE_CE66_1234_ABCD);
                // wasm32 has no thread to park on: a browser cannot `block_on` a future the way
                // the host can. Poll it once instead — a sealed world answers sleep, the clock and
                // files synchronously, so a computation that only needs THOSE resolves on that one
                // poll; one that genuinely needs the async runtime (real I/O, a live server) does
                // not, and is reported Skipped rather than hung or falsely passed.
                #[cfg(target_arch = "wasm32")]
                let result = if crate::interpreter::needs_async(program, interner) {
                    futures::FutureExt::now_or_never(interp.run(program))
                } else {
                    Some(interp.run_sync(program))
                };
                #[cfg(not(target_arch = "wasm32"))]
                let result = Some(if crate::interpreter::needs_async(program, interner) {
                    futures::executor::block_on(interp.run(program))
                } else {
                    interp.run_sync(program)
                });
                logicaffeine_system::testworld::uninstall();
                SessionRun {
                    result,
                    failures: interp.take_test_failures(),
                    assertions: interp.test_assertions(),
                    proven: interp.properties_proven(),
                    ops: interp.test_ops(),
                }
            });
            let Some(run_result) = run.result else {
                return TestOutcome {
                    name,
                    verdict: Verdict::Skipped,
                    failures: Vec::new(),
                    skip_reason: Some(
                        "needs the async runtime, which the in-browser runner does not drive"
                            .to_string(),
                    ),
                    ops: 0,
                    vacuous: false,
                    mounted,
                    property,
                    proved_by_exhaustion: false,
                    tier: None,
                };
            };
            proven_total.set(proven_total.get() + run.proven as usize);
            let assertions = run.assertions;
            let mut failures = run.failures;
            if let Err(e) = run_result {
                failures.push(e);
            }
            // G13 non-vacuity: a test that RAN CLEAN but asserted NOTHING (no
            // `Expect`/`For all`) is vacuous — a green that checked nothing. Fail
            // it loudly rather than count false confidence. (A run that already
            // errored keeps its real failure; a test isn't reached here when
            // skipped.)
            let vacuous = assertions == 0 && failures.is_empty();
            if vacuous {
                failures.push(
                    "vacuous test: it ran without a single assertion — add an `Expect …` \
                     or a `For all …` property (a test that checks nothing cannot pass)"
                        .to_string(),
                );
            }
            // Whether THIS test's finite-domain property was proved by
            // exhaustion (booleans under `--prove`): the interpreter's
            // per-run counter rose. Read before the tier replay, which does
            // not re-run properties.
            let proved_by_exhaustion = run.proven > 0;
            // `--tiers all` (Wave 13, footgun F18): a test that passes on the
            // interpreter must also RUN on the bytecode VM and AGREE — outcome
            // class, FULL output byte-for-byte, and error text. The assertions
            // stay the interpreter's authoritative verdict; both engines re-run
            // the COMPUTATION with the test-only statements no-op'd (the VM's
            // `compile_tiers` profile and the interpreter's `tiers_noop`
            // mirror) in fresh, identically-seeded sealed worlds — two engines,
            // one program. A computation the VM cannot run is an explicit typed
            // refusal — never a pass, never a silent skip.
            let mut tier: Option<TierEvidence> = None;
            if config.all_tiers && failures.is_empty() && !mounted {
                if crate::interpreter::needs_async(program, interner) {
                    tier = Some(TierEvidence::Refused {
                        reason: "the computation needs the async runtime (file I/O, sleep, \
                                 serve, or networking) — the bytecode VM is sync-only"
                            .to_string(),
                    });
                } else if crate::concurrency::uses_scheduler(program) {
                    tier = Some(TierEvidence::Refused {
                        reason: "the computation uses tasks/channels — the plain bytecode VM \
                                 has no scheduler driver"
                            .to_string(),
                    });
                } else {
                    // Interpreter replay of the bare computation, in a fresh
                    // world sealed with the SAME identity-keyed seed, as a session of its own.
                    let (replay_out, replay_err) = crate::interpreter::on_session_stack(|| {
                        crate::concurrency::net_inbox::set_net_offline(true);
                        logicaffeine_system::testworld::install(
                            logicaffeine_system::testworld::TestWorld::new(caps, h),
                        );
                        let mut replay = crate::interpreter::Interpreter::new(interner)
                            .with_type_registry(types)
                            .with_policies(policies.clone());
                        if !caps.filesystem {
                            replay = replay.with_vfs(std::sync::Arc::new(
                                logicaffeine_system::fs::MemVfs::new(),
                            ));
                        }
                        replay.enable_test_mode(budget);
                        replay.set_tiers_noop(true);
                        let replay_err = replay.run_sync(program).err();
                        logicaffeine_system::testworld::uninstall();
                        (replay.output.join("\n"), replay_err)
                    });

                    // The VM run of the SAME computation, in its own fresh
                    // world sealed with the same seed (entropy streams align).
                    crate::concurrency::net_inbox::set_net_offline(true);
                    logicaffeine_system::testworld::install(
                        logicaffeine_system::testworld::TestWorld::new(caps, h),
                    );
                    let vm_result = crate::vm::run_tiers_strict(
                        program,
                        interner,
                        Some(types),
                        Some(&policies),
                    );
                    logicaffeine_system::testworld::uninstall();

                    tier = Some(match vm_result {
                        Err(reason) => TierEvidence::Refused { reason },
                        Ok((vm_out, vm_err)) => match tier_divergence(
                            &replay_out,
                            replay_err.as_deref(),
                            &vm_out,
                            vm_err.as_deref(),
                        ) {
                            None => TierEvidence::Agreed,
                            Some(detail) => {
                                failures.push(detail.clone());
                                TierEvidence::Diverged {
                                    detail,
                                    statement: None,
                                    line: None,
                                }
                            }
                        },
                    });
                }
                if let Some(TierEvidence::Refused { reason }) = &tier {
                    failures.push(format!("tier: refused ({reason})"));
                }
            }
            // A MOUNTED test also runs, statement by statement, on every candidate of
            // `config.page_tiers` beyond the reference (`LoweredHere`) and must trace identically — the
            // `largo test` "what ships is not what is tested" guard, unconditional (not gated on
            // `--tiers all`: a page test's cross-tier agreement is not opt-in).
            if mounted && failures.is_empty() {
                let candidates: Vec<crate::page_host::PageTierKind> = config
                    .page_tiers
                    .iter()
                    .copied()
                    .filter(|k| *k != crate::page_host::PageTierKind::LoweredHere)
                    .collect();
                if !candidates.is_empty() {
                    let world = crate::page_host::World::for_test(&name, config.seed, caps, budget);
                    let scenario = crate::page_host::Scenario::new(
                        interner, types, &policies, &defs, body, world,
                    );
                    let reference = scenario.reference_trace();
                    for kind in candidates {
                        let evidence = match scenario.run(kind, launcher, &source) {
                            crate::page_host::TierRun::Refused { reason, .. } => {
                                TierEvidence::Refused { reason }
                            }
                            crate::page_host::TierRun::Traced(t) => {
                                // The journal axis is excluded. R3 has
                                // landed: an ordinary turn — an action, a real-listener `When …:`,
                                // a bound write — notes itself on the lowered tier exactly where
                                // the reference commits it, and agrees. What is left is narrower
                                // but still real: a TRAPPING turn never reaches its own write-back
                                // (rollback by construction skips everything after the trap, and
                                // `noteTurn` is placed there, matching the reference's own
                                // post-body `journal_push`), so the reference's `failed: Some(…)`
                                // entry has nothing to agree with on the lowered tier. Closing that
                                // needs whoever catches the trap (`page_host::lowered`'s guarded
                                // door, not this LOGOS-compiled body) to note the turn itself — a
                                // known, already-ledgered gap (`lgx_known_divergences.txt`'s
                                // remaining `journal @ …` rows), not a per-project regression to
                                // punish here.
                                let axes: Vec<_> = reference
                                    .compare(&t)
                                    .axes
                                    .into_iter()
                                    .filter(|a| a.axis != crate::page_host::Axis::Journal)
                                    .collect();
                                if axes.is_empty() {
                                    TierEvidence::Agreed
                                } else {
                                    let divergence = crate::page_host::Divergence { axes };
                                    TierEvidence::Diverged {
                                        detail: divergence.render(),
                                        statement: divergence.first_statement(),
                                        line: None,
                                    }
                                }
                            }
                        };
                        match &evidence {
                            TierEvidence::Refused { reason } => {
                                failures
                                    .push(format!("tier: refused on {} ({reason})", kind.label()));
                            }
                            TierEvidence::Diverged { detail, .. } => {
                                failures
                                    .push(format!("TIER DIVERGENCE on {}: {detail}", kind.label()));
                            }
                            TierEvidence::Agreed => {}
                        }
                        let agreed = matches!(evidence, TierEvidence::Agreed);
                        tier = Some(evidence);
                        if !agreed {
                            break;
                        }
                    }
                }
            }
            let verdict = if failures.is_empty() {
                Verdict::Passed
            } else {
                Verdict::Failed
            };
            TestOutcome {
                name,
                verdict,
                failures,
                skip_reason: None,
                ops: run.ops,
                vacuous,
                mounted,
                property,
                proved_by_exhaustion,
                tier,
            }
        };

        let mut outcomes: Vec<TestOutcome> = Vec::new();
        let mut properties = 0usize;
        for test in tests {
            let Stmt::TestDef {
                name,
                skip,
                budget,
                cases,
                body,
                capabilities,
            } = test
            else {
                unreachable!("filtered to TestDef above")
            };
            // The first `For all` property in the body classifies the test as a
            // property (for the inventory) and discloses its finite bound. A test
            // body carries at most one property in practice; the first is its
            // identity.
            // Whether this test MOUNTS a page — the honest signal that it drives a document
            // rather than checking a value: an explicit `Mount` in the body (how a bare
            // `## Component` test is driven), OR `defs` auto-mounting a `## Page` (how every
            // page's OWN test is driven — a page "mounts itself", so nothing in the body ever
            // spells `Mount`, but the cross-tier check below is exactly as owed to it).
            let mounted = body
                .iter()
                .any(|s| matches!(s, Stmt::Ui(crate::ast::stmt::UiStmt::Mount { .. })))
                || crate::page_host::defs_auto_mount_a_page(&defs);
            let property: Option<PropertyInfo> = body.iter().find_map(|s| match s {
                Stmt::ForAll { vars, domain, .. } => Some(PropertyInfo {
                    domain: match domain {
                        crate::ast::stmt::PropDomain::Integers => PropertyDomain::Integers,
                        crate::ast::stmt::PropDomain::Naturals => PropertyDomain::Naturals,
                        crate::ast::stmt::PropDomain::Booleans => PropertyDomain::Booleans,
                    },
                    vars: vars.len(),
                }),
                _ => None,
            });
            if property.is_some() {
                properties += 1;
            }
            let caps = logicaffeine_system::testworld::Capabilities {
                filesystem: capabilities.filesystem,
                network: capabilities.network,
                environment: capabilities.environment,
                clock: capabilities.clock,
            };
            let is_live = caps.filesystem || caps.network || caps.environment || caps.clock;
            let name_s = interner.resolve(*name).to_string();
            if let Some(f) = &config.filter {
                if !name_s.contains(f.as_str()) {
                    continue;
                }
            }
            if let Some(reason) = skip {
                outcomes.push(TestOutcome {
                    name: name_s,
                    verdict: Verdict::Skipped,
                    failures: Vec::new(),
                    skip_reason: Some(interner.resolve(*reason).to_string()),
                    ops: 0,
                    vacuous: false,
                    mounted,
                    property,
                    proved_by_exhaustion: false,
                    tier: None,
                });
                continue;
            }
            if is_live && !config.live {
                // Capability-granted tests leave the hermetic guarantee and
                // run only under `--live` — reported loudly, never silently.
                outcomes.push(TestOutcome {
                    name: name_s,
                    verdict: Verdict::Skipped,
                    failures: Vec::new(),
                    skip_reason: Some(
                        "uses a real resource — runs only under `largo test --live`".to_string(),
                    ),
                    ops: 0,
                    vacuous: false,
                    mounted,
                    property,
                    proved_by_exhaustion: false,
                    tier: None,
                });
                continue;
            }
            let budget = budget.unwrap_or(config.default_budget);

            // A `## Page` mounts itself, which the parser writes as a literal `Mount` among
            // `defs` — placed there for the PROGRAM, which runs before any test statement. A test
            // that arranges the scene (`Given that … answers`, `Given a file`, `Serve fetches with
            // routes`) needs that mount moved to just after the scene, exactly as `page_host`
            // drives it (`page_scene_lock`): otherwise a page that fetches `When the page loads`
            // already asked before its test could answer. (`Reload the page.` mounts the page
            // again by itself.) A no-op for anything that is not a page (nothing to extract, so
            // `arrange` returns `defs`/`body` unchanged).
            let arranged = crate::page_host::arrange(&defs, body);
            match cases {
                None => {
                    let mut program: Vec<Stmt> = arranged.defs.clone();
                    program.extend(arranged.body.iter().cloned());
                    outcomes.push(execute(
                        &program, body, name_s, budget, caps, property, mounted,
                    ));
                }
                Some(table) => {
                    // Each data row is a NAMED sub-test in table order: the
                    // columns bind as fresh `Let`s ahead of the body.
                    for row in &table.rows {
                        let mut program: Vec<Stmt> = arranged.defs.clone();
                        let mut bindings: Vec<String> = Vec::new();
                        for (column, cell) in table.columns.iter().zip(row.iter()) {
                            bindings.push(format!(
                                "{}={}",
                                interner.resolve(*column),
                                render_cell(cell, interner)
                            ));
                            program.push(Stmt::Let {
                                var: *column,
                                ty: None,
                                value: cell,
                                mutable: false,
                            });
                        }
                        program.extend(arranged.body.iter().cloned());
                        let sub_name = format!("{name_s} [{}]", bindings.join(", "));
                        outcomes.push(execute(
                            &program, body, sub_name, budget, caps, property, mounted,
                        ));
                    }
                }
            }
        }

        // G13: a `--filter` that matched ZERO tests is a mistake (a typo'd name
        // silently "passing" is exactly the false-green we forbid), never a green
        // empty run. Zero tests WITHOUT a filter is a legitimate no-test file and
        // stays an empty report (the CLI decides its own policy there).
        if let Some(f) = &config.filter {
            if outcomes.is_empty() {
                // Wording aligns with the CLI's own zero-match message so the
                // `closure_lock` gate (which asserts "matched none") passes
                // whichever layer surfaces it — one message, no drift.
                return Err(format!("filter {f:?} matched none of the discovered tests"));
            }
        }

        let passed = outcomes
            .iter()
            .filter(|o| o.verdict == Verdict::Passed)
            .count();
        let failed = outcomes
            .iter()
            .filter(|o| o.verdict == Verdict::Failed)
            .count();
        let skipped = outcomes
            .iter()
            .filter(|o| o.verdict == Verdict::Skipped)
            .count();
        Ok(RunReport {
            seed: config.seed,
            outcomes,
            passed,
            failed,
            skipped,
            properties,
            proven: proven_total.get(),
            page_tiers: page_tier_labels(config),
        })
    })
}

/// Render the human report — deterministic, golden-locked (`test_runner_lock`).
pub fn render_report(report: &RunReport) -> String {
    let mut out = String::new();
    for o in &report.outcomes {
        match o.verdict {
            Verdict::Passed => {
                out.push_str(&format!("PASS  {}\n", o.name));
            }
            Verdict::Skipped => {
                let reason = o.skip_reason.as_deref().unwrap_or("");
                out.push_str(&format!("SKIP  {} — {}\n", o.name, reason));
            }
            Verdict::Failed => {
                out.push_str(&format!("FAIL  {}\n", o.name));
                for f in &o.failures {
                    // A failure may be a multi-line power-assert; indent every
                    // line by 6 so the block sits cleanly under its test.
                    for line in f.lines() {
                        out.push_str(&format!("      {line}\n"));
                    }
                }
            }
        }
    }
    let refused = report.tier_refused();
    if refused > 0 {
        out.push_str(&format!(
            "\n{} passed · {} failed · {} skipped · {} tier-refused · seed {}\n",
            report.passed, report.failed, report.skipped, refused, report.seed
        ));
    } else {
        out.push_str(&format!(
            "\n{} passed · {} failed · {} skipped · seed {}\n",
            report.passed, report.failed, report.skipped, report.seed
        ));
    }
    out
}

/// A machine-readable census of which EXECUTION TIERS a `largo test` run actually
/// exercised — the runner's answer to G14's "a green run must be legible about what
/// it proved". The tree-walking interpreter is the always-on reference tier (every
/// `## Test` runs on it); the bytecode VM tier is a cross-tier VALIDATOR that only
/// runs under `--tiers all`, and when it does, each test's [`TierEvidence`] says
/// whether the VM agreed, refused, or diverged. Derived purely from the report, so
/// the census can never drift from the evidence it summarizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TierSummary {
    /// Tests that ran on the tree-walking interpreter (the reference oracle) — the
    /// whole non-skipped suite, always.
    pub interpreter: usize,
    /// Was the bytecode-VM cross-tier validator turned on for this run?
    pub vm_validator_ran: bool,
    /// VM agreed byte-for-byte (`--tiers all`).
    pub vm_agreed: usize,
    /// VM could not run the computation and REFUSED (`--tiers all`) — reported as a
    /// `tier: refused (…)` failure, never a silent skip.
    pub vm_refused: usize,
    /// VM ran and DISAGREED (`--tiers all`) — always a compiler bug.
    pub vm_diverged: usize,
}

impl RunReport {
    /// The execution-tier census for this run (see [`TierSummary`]).
    pub fn tier_summary(&self) -> TierSummary {
        let ran = |pred: fn(&TierEvidence) -> bool| {
            self.outcomes
                .iter()
                .filter(|o| o.tier.as_ref().is_some_and(pred))
                .count()
        };
        let vm_agreed = ran(|e| matches!(e, TierEvidence::Agreed));
        let vm_refused = ran(|e| matches!(e, TierEvidence::Refused { .. }));
        let vm_diverged = ran(|e| matches!(e, TierEvidence::Diverged { .. }));
        TierSummary {
            // Every executed (non-skipped) test ran on the interpreter oracle.
            interpreter: self
                .outcomes
                .iter()
                .filter(|o| o.verdict != Verdict::Skipped)
                .count(),
            vm_validator_ran: self.outcomes.iter().any(|o| o.tier.is_some()),
            vm_agreed,
            vm_refused,
            vm_diverged,
        }
    }
}

/// Render the machine-readable tier census as ONE greppable line, mirroring the
/// `LOGOS_VALIDATORS_RAN:` convention the Rust suite uses for external validators.
/// A CI/release gate greps `LOGOS_TIERS_RAN:` to see, at a glance, which execution
/// tiers a green `largo test` actually exercised — the interpreter always, the VM
/// cross-tier validator only under `--tiers all`. When the VM validator was OFF, the
/// line says so explicitly (`vm=off`), so a fast default run never masquerades as
/// having proven cross-tier agreement it never ran.
///
/// A MOUNTED page test's own cross-tier check (G1: this is not gated on `--tiers
/// all`, and since `mounted` now recognizes a `## Page`'s own auto-mount and not
/// only a literal `Mount` statement, this line covers it too) additionally names
/// WHICH candidate page tiers actually ran, appending `page_tiers=<a,b,…>` — so a
/// green `largo test --browser` cannot read the same as a green plain `largo test`
/// when the difference is exactly whether a real Chrome was ever launched.
pub fn render_tier_summary(report: &RunReport) -> String {
    let s = report.tier_summary();
    let mut line = if s.vm_validator_ran {
        format!(
            "LOGOS_TIERS_RAN: interpreter={} vm=on vm_agreed={} vm_refused={} vm_diverged={}",
            s.interpreter, s.vm_agreed, s.vm_refused, s.vm_diverged
        )
    } else {
        format!("LOGOS_TIERS_RAN: interpreter={} vm=off", s.interpreter)
    };
    let page_tiers_ran = report
        .outcomes
        .iter()
        .any(|o| o.mounted && o.tier.is_some());
    if page_tiers_ran && !report.page_tiers.is_empty() {
        line.push_str(&format!(" page_tiers={}", report.page_tiers.join(",")));
    }
    line.push('\n');
    line
}

/// Render the machine report (`--format json`).
pub fn render_report_json(report: &RunReport) -> String {
    serde_json::to_string_pretty(report).expect("a run report always serializes")
}

// ============================================================
// G13-full — the machine-readable proof/test INVENTORY.
// ============================================================
//
// ONE structured document classifies EVERY claim a project makes by KIND, with an
// honest per-item status. It is a REPORTING layer over the existing outcomes — the
// runner's [`RunReport`] (examples + finite-enumeration properties) and the CLI's
// prove report (theorems + admitted axioms) — never a re-prover. The honesty
// invariants it enforces:
//
//   * A finite-enumeration property or theorem is marked `finite_enumeration` and
//     carries its disclosed `domain_bound`; it is NEVER `proof: true` (an
//     unrestricted mathematical proof). A green enumeration never silently reads
//     as an unbounded proof.
//   * An axiom / admitted assumption is `proof: false`, status `assumption` — a
//     premise, never a proven fact.
//   * A theorem/axiom NAME COLLISION is rejected upstream (see
//     [`Inventory::from_prove`]), so an axiom can never masquerade as a theorem.
//   * The counts are internally consistent:
//     examples + properties + theorems + axioms + admitted == the item total, and
//     the `proven` set ⊆ properties + theorems.

/// The KIND of a claim — the top-level partition the inventory reports over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimKind {
    /// A `## Test`/`## Example` block asserted with `Expect`/`ExpectOutput`/
    /// `ExpectFail` — finite testing of concrete cases (never a proof).
    Example,
    /// A `For all <domain> …:` property test — finite enumeration/sampling over
    /// a disclosed domain (a proof ONLY when the domain is finite AND exhausted).
    Property,
    /// A `## Theorem`/`Theorem …` claim discharged by the proof engine.
    Theorem,
    /// A `## Axiom`/`Axiom …` — an ASSUMPTION admitted into the premise base,
    /// never proved.
    Axiom,
    /// Anything the proof engine ADMITS rather than proves (an admitted lemma).
    AdmittedAssumption,
    /// ★ A COMPONENT TEST — one that MOUNTS a page, drives it, and asserts about the rendered
    /// document.
    ///
    /// Not an `Example`, and the difference is what a claim inventory is for: an example checks a
    /// VALUE, this checks a PAGE. They fail in different ways and cover different risk, and
    /// reporting both as `example` told a reader that a run of twenty examples had checked twenty
    /// values when some of them had checked a document. It is what lets somebody ask "does this
    /// project test its UI at all?" and get an answer.
    ///
    /// Finite testing like an example, and never a proof: mounting a page and clicking it checks
    /// the cases you drove and nothing else.
    Component,
}

/// The honest per-item status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimStatus {
    /// An example whose assertions all held.
    Passed,
    /// An example/property whose check failed (a property failure carries a
    /// concrete counterexample; see [`InventoryItem::counterexample`]).
    Failed,
    /// A `## Test` that RAN CLEAN but asserted NOTHING — a green that checked
    /// nothing (the non-vacuity gate fails it; the inventory surfaces it too).
    Vacuous,
    /// A property that held over every case of a FINITE domain exhausted under
    /// `--prove` — a certificate over that finite domain, NOT an unrestricted
    /// proof (the item stays `finite_enumeration: true`, `proof: false`).
    ProvedByExhaustion,
    /// A property that held over its SAMPLED cases (an infinite domain, or the
    /// finite domain without `--prove`) — evidence, not a proof.
    Held,
    /// A property with a concrete counterexample — disproved.
    Falsified,
    /// A theorem the proof engine kernel-certified — an actual proof.
    Proved,
    /// A theorem the proof engine could not discharge.
    Unproved,
    /// An axiom / admitted assumption — a premise, reported as an assumption,
    /// explicitly not a proof.
    Assumption,
}

/// ONE classified claim in the inventory.
#[derive(Debug, Clone, Serialize)]
pub struct InventoryItem {
    pub kind: ClaimKind,
    /// The claim's identity (the test's prose name, or the theorem/axiom name).
    pub name: String,
    pub status: ClaimStatus,
    /// True when the claim was established by FINITE testing (an example, or a
    /// finite-enumeration property/bounded theorem) rather than an unbounded
    /// mathematical argument. The disclosure that keeps green from meaning
    /// "unbounded proof".
    pub finite_enumeration: bool,
    /// The disclosed finite bound (`integers sampled over [-1000, 1000]`, the
    /// exhausted boolean space, …) — present whenever a bound was actually
    /// covered, so a bounded claim can never masquerade as unbounded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_bound: Option<String>,
    /// A concrete counterexample, when the claim was falsified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterexample: Option<String>,
    /// True only for the vacuous example — a green that asserted nothing.
    pub vacuous: bool,
    /// Whether this item is an actual PROOF: true ONLY for a kernel-certified
    /// theorem. An example, a finite-enumeration property (even one exhausted
    /// over booleans), an axiom, and an admitted assumption are all `false` —
    /// finite testing and admitted premises are never unrestricted proofs.
    pub proof: bool,
    /// The independently re-checkable proof certificate — the kernel proof term,
    /// the goal it proves, and the theorem's disclosed premises. Present ONLY on a
    /// `proof: true` theorem: a third party can re-verify it against the kernel
    /// WITHOUT trusting the prover. Never present on an axiom/assumption (an
    /// assumption is not a proof), a finite-enumeration item, or an unproved
    /// theorem.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate: Option<crate::kernel::certificate::TheoremCertificate>,
    /// Whether the carried certificate RE-CHECKS via the independent kernel checker
    /// (type-checking the term to the claimed goal, not re-running the prover).
    /// `Some(true)` on a certificate that re-verifies; `None` when there is no
    /// certificate to check. A `proof: true` theorem whose certificate does NOT
    /// re-check is a soundness alarm the consumer must treat as a failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate_rechecks: Option<bool>,
}

/// The rolled-up census the inventory carries alongside its items — the
/// internally-consistent counts (examples + properties + theorems + axioms +
/// admitted == claims; proven ⊆ properties + theorems).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct InventorySummary {
    pub claims: usize,
    pub examples: usize,
    pub properties: usize,
    pub theorems: usize,
    pub axioms: usize,
    pub admitted_assumptions: usize,
    /// Items that are ACTUAL proofs (kernel-certified theorems) — never counts an
    /// axiom, an example, or a finite-enumeration property.
    pub proven: usize,
    /// Falsified examples/properties + unproved theorems + vacuous examples.
    pub failing: usize,
}

/// The whole inventory: every classified claim plus its rolled-up summary.
#[derive(Debug, Clone, Serialize)]
pub struct Inventory {
    pub items: Vec<InventoryItem>,
    pub summary: InventorySummary,
}

impl Inventory {
    /// Build the summary from the items, so the counts can never drift from the
    /// classification they summarize.
    fn summarize(items: &[InventoryItem]) -> InventorySummary {
        let count = |k: ClaimKind| items.iter().filter(|i| i.kind == k).count();
        InventorySummary {
            claims: items.len(),
            examples: count(ClaimKind::Example),
            properties: count(ClaimKind::Property),
            theorems: count(ClaimKind::Theorem),
            axioms: count(ClaimKind::Axiom),
            admitted_assumptions: count(ClaimKind::AdmittedAssumption),
            proven: items.iter().filter(|i| i.proof).count(),
            failing: items
                .iter()
                .filter(|i| {
                    matches!(
                        i.status,
                        ClaimStatus::Failed | ClaimStatus::Unproved | ClaimStatus::Vacuous
                    )
                })
                .count(),
        }
    }

    fn from_items(items: Vec<InventoryItem>) -> Inventory {
        let summary = Self::summarize(&items);
        Inventory { items, summary }
    }

    /// Classify every claim in a `largo test` [`RunReport`] — examples and
    /// finite-enumeration properties. Skipped tests carry no verified claim, so
    /// they are not inventoried as passing or failing evidence.
    pub fn from_run_report(report: &RunReport) -> Inventory {
        let mut items: Vec<InventoryItem> = Vec::new();
        for o in &report.outcomes {
            if o.verdict == Verdict::Skipped {
                continue;
            }
            match o.property {
                Some(info) => items.push(classify_property(o, info)),
                None => items.push(classify_example(o)),
            }
        }
        Self::from_items(items)
    }
}

/// Extract the concrete counterexample from a property's recorded failure — the
/// runner records it as `Property failed. Counterexample: <binding>`.
fn extract_counterexample(failures: &[String]) -> Option<String> {
    const MARK: &str = "Counterexample: ";
    failures
        .iter()
        .find_map(|f| f.split_once(MARK).map(|(_, cx)| cx.trim().to_string()))
}

/// Classify a plain `Expect`-only test as an EXAMPLE — or as a COMPONENT claim when it mounted a
/// page, since driving a document is a different kind of evidence from checking a value.
fn classify_example(o: &TestOutcome) -> InventoryItem {
    let status = if o.vacuous {
        ClaimStatus::Vacuous
    } else if o.verdict == Verdict::Passed {
        ClaimStatus::Passed
    } else {
        ClaimStatus::Failed
    };
    InventoryItem {
        kind: if o.mounted {
            ClaimKind::Component
        } else {
            ClaimKind::Example
        },
        name: o.name.clone(),
        status,
        // Examples check concrete cases — finite testing, always.
        finite_enumeration: true,
        domain_bound: None,
        counterexample: None,
        vacuous: o.vacuous,
        // An example is never a proof.
        proof: false,
        certificate: None,
        certificate_rechecks: None,
    }
}

/// Classify a `For all` test as a PROPERTY, disclosing its finite bound and —
/// honestly — never marking it an unrestricted proof.
fn classify_property(o: &TestOutcome, info: PropertyInfo) -> InventoryItem {
    let counterexample = extract_counterexample(&o.failures);
    let status = if counterexample.is_some() {
        // A concrete disproving witness — the honest falsification.
        ClaimStatus::Falsified
    } else if o.verdict == Verdict::Failed {
        // The property's body failed WITHOUT yielding a counterexample (a
        // runtime error or a budget hit) — a failure, but not a disproof.
        ClaimStatus::Failed
    } else if o.proved_by_exhaustion {
        ClaimStatus::ProvedByExhaustion
    } else {
        ClaimStatus::Held
    };
    InventoryItem {
        kind: ClaimKind::Property,
        name: o.name.clone(),
        status,
        // A property is checked by finite enumeration/sampling — ALWAYS, even a
        // boolean exhaustion. This is the line that stops a green property from
        // reading as an unbounded mathematical proof.
        finite_enumeration: true,
        domain_bound: Some(info.domain.bound().to_string()),
        counterexample,
        vacuous: false,
        // NEVER a proof: exhaustion over a finite domain is a certificate over
        // THAT domain, not an unrestricted proof; sampling is only evidence.
        proof: false,
        certificate: None,
        certificate_rechecks: None,
    }
}

/// A theorem/axiom claim handed to the inventory by the prove path. Kept tiny so
/// the CLI's `Outcome`/prove report can map onto it without a dependency cycle.
#[derive(Debug, Clone)]
pub struct ProvenClaim {
    pub name: String,
    /// True when the proof engine kernel-certified it.
    pub verified: bool,
    /// The disclosed finite bound when the theorem was checked over a bounded
    /// (finite) domain rather than proved unrestricted — `None` for an
    /// unbounded kernel proof.
    pub finite_bound: Option<String>,
    /// The independently re-checkable proof certificate — the kernel proof term,
    /// the goal it proves, and the theorem's disclosed premises — present ONLY for
    /// a genuine unbounded kernel proof. `None` for an unproved theorem, a
    /// finite-enumeration (bounded) theorem, or any surface that does not yet
    /// expose a kernel term. An axiom/assumption NEVER carries one.
    pub certificate: Option<crate::kernel::certificate::TheoremCertificate>,
}

impl Inventory {
    /// Build the inventory for a `largo prove` pass from its classified claims:
    /// theorems, axioms (assumptions), and admitted assumptions. REJECTS a
    /// name that appears both as a theorem and as an axiom/admitted assumption —
    /// an axiom must never be able to masquerade as a proved theorem.
    ///
    /// `axioms` and `admitted` are `(name)` lists; `theorems` are
    /// [`ProvenClaim`]s. Returns `Err(collision_message)` naming every colliding
    /// name, so the run can fail loud.
    pub fn from_prove(
        theorems: &[ProvenClaim],
        axioms: &[String],
        admitted: &[String],
    ) -> Result<Inventory, String> {
        use std::collections::BTreeSet;
        let theorem_names: BTreeSet<&str> = theorems.iter().map(|t| t.name.as_str()).collect();
        let assumption_names: BTreeSet<&str> = axioms
            .iter()
            .chain(admitted.iter())
            .map(String::as_str)
            .collect();
        let collisions: Vec<&str> = theorem_names
            .intersection(&assumption_names)
            .copied()
            .collect();
        if !collisions.is_empty() {
            return Err(format!(
                "name collision: {} appears as BOTH a theorem and an axiom/admitted assumption — \
                 an axiom is an ASSUMPTION and must never be able to masquerade as a proved \
                 theorem. Rename one so every claim has exactly one kind.",
                collisions
                    .iter()
                    .map(|n| format!("`{n}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        let mut items: Vec<InventoryItem> = Vec::new();
        for t in theorems {
            let bounded = t.finite_bound.is_some();
            // A PROOF only when kernel-certified AND unbounded. A bounded check is a
            // certificate over its finite domain, not an unrestricted proof.
            let proof = t.verified && !bounded;
            // The certificate rides ONLY on a genuine unbounded kernel proof — never on
            // a bounded (finite-enumeration) theorem, which has no unrestricted proof
            // term. When present, re-check it INDEPENDENTLY of the prover here, so the
            // inventory carries a verdict a consumer can trust without re-running search.
            let certificate = if proof { t.certificate.clone() } else { None };
            let certificate_rechecks = certificate
                .as_ref()
                .map(|c| crate::kernel::certificate::recheck_theorem(c).rechecks);
            items.push(InventoryItem {
                kind: ClaimKind::Theorem,
                name: t.name.clone(),
                status: if t.verified {
                    ClaimStatus::Proved
                } else {
                    ClaimStatus::Unproved
                },
                // A bounded (finite-domain) theorem is finite enumeration; an
                // unbounded kernel proof is not.
                finite_enumeration: bounded,
                domain_bound: t.finite_bound.clone(),
                counterexample: None,
                vacuous: false,
                proof,
                certificate,
                certificate_rechecks,
            });
        }
        for a in axioms {
            items.push(InventoryItem {
                kind: ClaimKind::Axiom,
                name: a.clone(),
                status: ClaimStatus::Assumption,
                finite_enumeration: false,
                domain_bound: None,
                counterexample: None,
                vacuous: false,
                proof: false,
                // An axiom is an ASSUMPTION, not a proof — it never gets a certificate.
                certificate: None,
                certificate_rechecks: None,
            });
        }
        for a in admitted {
            items.push(InventoryItem {
                kind: ClaimKind::AdmittedAssumption,
                name: a.clone(),
                status: ClaimStatus::Assumption,
                finite_enumeration: false,
                domain_bound: None,
                counterexample: None,
                vacuous: false,
                proof: false,
                // An admitted assumption is not a proof — it never gets a certificate.
                certificate: None,
                certificate_rechecks: None,
            });
        }
        Ok(Self::from_items(items))
    }
}

/// Render the `largo test --json` inventory document: the classified claims plus
/// their summary, the seed, and the raw outcomes (so a consumer keeps both the
/// classification and the underlying detail from one call).
pub fn render_test_inventory_json(report: &RunReport) -> String {
    let inventory = Inventory::from_run_report(report);
    let doc = serde_json::json!({
        "seed": report.seed,
        "inventory": inventory.items,
        "summary": inventory.summary,
        "outcomes": report.outcomes,
    });
    serde_json::to_string_pretty(&doc).expect("the inventory always serializes")
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_compile/testrun/inventory_tests.rs"]
mod inventory_tests;
