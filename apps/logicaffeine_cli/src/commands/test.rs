//! `largo test` — run the project's `## Test` blocks.
//!
//! Semantics live in `logicaffeine_compile::testrun` (Studio-reusable); this
//! command owns project loading, the skip-budget gate, and exit codes:
//! exit 0 = green (all passed, skips within budget); exit 1 = any failure or
//! an unbudgeted skip (skips are debts, not courtesies). The runner sees the
//! WHOLE assembled module closure — a `## Test` in an imported module runs
//! exactly like one in the entry — and zero executed tests (none discovered,
//! or a filter that matched nothing) is a failure, never a silent green.

use logicaffeine_compile::page_host::{PageTierKind, RemotePageLauncher};
use logicaffeine_compile::testrun::{
    render_report, render_report_json, render_test_inventory_json, run_tests_with_launcher,
    TestConfig,
};

use crate::commands::require_project_root;
use crate::diagnostics::{self, Placed};
use crate::project::shape::Project;
use crate::ui::CliError;

/// Count the `## Test` blocks declared in an assembled source — the discovery
/// ledger behind the zero-match diagnostics (headers, not executed sub-tests:
/// a case table still counts as its one declaring block).
fn discovered_test_blocks(source: &str) -> usize {
    source
        .lines()
        .filter(|line| {
            let t = line.trim_start();
            t == "## Test" || t.starts_with("## Test:") || t.starts_with("## Test ")
        })
        .count()
}

/// A page whose tests could not run because it does not compile.
struct Uncompiled {
    /// `pages/<stem>.lg`.
    file: String,
    /// How many `## Test` blocks it declares.
    tests: usize,
    /// Why, rendered at the page's own `file:line:col`.
    diagnostic: String,
}

/// Why a page's source would not run its tests, rendered at the mistake: the parse error the
/// test runner met, placed through the page's closure — or, when the failure has no source
/// position, the runner's own words.
fn why_it_did_not_compile(page: &crate::commands::web_bundle::PageClosure, advice: &str) -> String {
    match logicaffeine_compile::compile::compile_to_program(&page.closure.source) {
        Err(pe) => {
            let at = Placed::locate(
                pe.span,
                &page.closure.source,
                &page.closure.source_map,
                &page.path,
            );
            diagnostics::render_parse_error(&pe, &at)
        }
        Ok(_) => advice.to_string(),
    }
}

pub(crate) fn cmd_test(
    filter: Option<&str>,
    seed: u64,
    format: &str,
    json: bool,
    live: bool,
    prove: bool,
    tiers: Option<&str>,
    browser: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // `--browser`: a MOUNTED test's page tiers gain a real headless Chrome, and Chrome not being
    // found is the run's own failure, never a silent narrowing back to the in-process tiers —
    // matching how the repository's own suite treats `LOGOS_REQUIRE_TOOLS=chrome`.
    if browser {
        crate::browser::locate_chrome().ok_or_else(|| {
            CliError::new(
                "`--browser` was given but no Chrome/Chromium was found (set `CHROME_BIN`, or \
                 install one of google-chrome/google-chrome-stable/chromium/chromium-browser/chrome)"
                    .to_string(),
            )
        })?;
    }
    let project = Project::load(&require_project_root()?)?;
    // The pages are driven as the project serves them — its stylesheets linked, its files beside.
    let launcher =
        browser.then(|| crate::browser::ChromeLauncher::for_project(&project.root.join("static")));
    let launcher: Option<&dyn RemotePageLauncher> =
        launcher.as_ref().map(|l| l as &dyn RemotePageLauncher);
    // Link the declared local (`path`/`git`) dependencies into the closure.
    let dep_roots = crate::project::deps::resolve_dep_roots(&project.root, &project.manifest)?;
    // The SAME assembled closure every other command compiles: imported
    // modules' tests and definitions reach the runner exactly as the entry's.
    // A site with no entry program has no entry tests; its pages carry them.
    let source = match &project.entry {
        Some(entry) => {
            crate::closure::assemble_closure_with_deps(entry, dep_roots.clone())
                .map_err(|e| CliError::new(format!("could not assemble project: {e}")))?
                .source
        }
        None => String::new(),
    };

    // `--tiers all` (or a list naming `vm`) turns on the cross-tier F18 guard. An unrecognised
    // value is refused rather than dropped: a flag whose job is to make a claim about coverage
    // must never accept a word it does nothing with.
    let all_tiers = match tiers {
        None => false,
        Some(t) => {
            const KNOWN: &[&str] = &["all", "vm", "interpreter"];
            let mut named_vm = t.trim() == "all";
            for part in t.split(',') {
                let part = part.trim();
                if !KNOWN.contains(&part) {
                    return Err(CliError::new(format!(
                        "`--tiers {part}` is not a tier this command can run. It takes `all` (the \
                         interpreter and the bytecode VM, compared byte for byte), or a \
                         comma-separated list of `interpreter` and `vm`.\n\nThe AOT, C, WASM and \
                         browser tiers are covered by the repository's own differentials rather \
                         than by this flag — `--tiers aot` would have reported a green run with no \
                         comparison at all, which is why it is refused rather than ignored."
                    ))
                    .exit_code(crate::ui::EXIT_USAGE)
                    .into());
                }
                named_vm |= part == "vm" || part == "all";
            }
            named_vm
        }
    };
    let mut page_tiers = TestConfig::default().page_tiers;
    if browser {
        page_tiers.push(PageTierKind::Browser);
    }
    let config = TestConfig {
        filter: filter.map(str::to_string),
        seed,
        live,
        prove,
        all_tiers,
        page_tiers,
        ..TestConfig::default()
    };
    let mut report = run_tests_with_launcher(&source, &config, launcher).map_err(CliError::new)?;
    let mut discovered = discovered_test_blocks(&source);

    // A page is its OWN program (two `## Page` blocks in one source is a refusal, so pages cannot
    // be concatenated); each runs its tests and its outcomes merge into the run under the page's
    // file name, so a result says which file it came from and the failure gate below covers
    // pages without knowing they exist.
    let mut uncompiled: Vec<Uncompiled> = Vec::new();
    for (stem, path) in crate::commands::web_bundle::page_files(&project.pages_dir())
        .map_err(|e| CliError::new(e.to_string()))?
    {
        let file = format!("pages/{stem}.lg");
        let closure = crate::closure::assemble_closure_with_deps(&path, dep_roots.clone())
            .map_err(|e| CliError::new(format!("{file}: {e}")))?;
        let page = crate::commands::web_bundle::PageClosure {
            stem,
            path,
            closure,
        };
        let tests = discovered_test_blocks(&page.closure.source);
        discovered += tests;
        match run_tests_with_launcher(&page.closure.source, &config, launcher) {
            Ok(page_report) => {
                report.passed += page_report.passed;
                report.failed += page_report.failed;
                report.skipped += page_report.skipped;
                report.properties += page_report.properties;
                report.proven += page_report.proven;
                report
                    .outcomes
                    .extend(page_report.outcomes.into_iter().map(|mut o| {
                        o.name = format!("{file} › {}", o.name);
                        o
                    }));
            }
            Err(advice) => {
                let diagnostic = why_it_did_not_compile(&page, &advice);
                if tests == 0 {
                    // No tests are lost; the page's failure is `largo check`'s to report, and is
                    // named here so it is never silent.
                    crate::ui::warning(format!("{file} does not compile:\n{diagnostic}"));
                } else {
                    uncompiled.push(Uncompiled {
                        file,
                        tests,
                        diagnostic,
                    });
                }
            }
        }
    }

    // `--json` emits the machine-readable CLAIM INVENTORY (a superset of the raw
    // report): every `## Test` classified by kind with an honest status, vacuous
    // flags, disclosed finite bounds, and any concrete counterexample. It is
    // printed BEFORE the failure gate, so a run with a falsified property or a
    // vacuous example still yields the full inventory on stdout (exit 1 for the
    // shell, the document for the machine).
    if json {
        println!("{}", render_test_inventory_json(&report));
    } else {
        match format {
            "json" => println!("{}", render_report_json(&report)),
            _ => {
                print!("{}", render_report(&report));
                print!("{}", logicaffeine_compile::testrun::render_fitness(&report));
            }
        }
    }

    // G14 — surface which EXECUTION TIERS this run actually exercised, machine-readably, so a
    // green `largo test` is legible about what it proved: the interpreter oracle always, the
    // bytecode-VM cross-tier validator only under `--tiers all`. Emitted on stderr so it never
    // pollutes the byte-exact stdout (`--format json` stays a single clean document).
    eprint!(
        "{}",
        logicaffeine_compile::testrun::render_tier_summary(&report)
    );

    // A page that declares tests and does not compile is tests that did not run: the run fails,
    // naming the file and showing why — never "no tests were discovered" about a project holding
    // several.
    if let Some(last) = uncompiled.pop() {
        for earlier in &uncompiled {
            crate::ui::error(did_not_compile(earlier));
        }
        return Err(CliError::with_hint(
            did_not_compile(&last),
            "fix the page (`largo check` reports every file), then run `largo test` again",
        )
        .into());
    }

    // Zero executed tests is never a green run: a project with no `## Test`
    // blocks anywhere in its closure, or a filter that matched nothing, must
    // fail loudly — the audit's false-green ("0 passed" with exit 0).
    if report.outcomes.is_empty() {
        return Err(match filter {
            Some(f) => CliError::with_hint(
                format!("the filter {f:?} matched none of the {discovered} discovered test(s)"),
                "run `largo test` with no filter to see every discovered test",
            ),
            None => CliError::with_hint(
                "no tests were discovered — no `## Test` block exists in the project's closure",
                "write one: a `## Test: <prose name>` block with `Expect …` lines",
            ),
        }
        .into());
    }

    // The debt ledger's first gate: skips fail the run unless consciously
    // budgeted in Largo.toml (`[test] skip_budget = N` — a budget that should
    // only ever shrink).
    // Live-gated tests are gated by `--live`, not the skip budget — only
    // author-written `Skip this test because …` skips count against it.
    let authored_skips = report
        .outcomes
        .iter()
        .filter(|o| {
            o.verdict == logicaffeine_compile::testrun::Verdict::Skipped
                && o.skip_reason
                    .as_deref()
                    .map_or(true, |r| !r.contains("--live"))
        })
        .count();
    let skip_budget = project.manifest.test.as_ref().map_or(0, |t| t.skip_budget);
    if authored_skips > skip_budget {
        return Err(CliError::with_hint(
            format!(
                "{} skipped test(s) exceed the skip budget of {} — a skip is a debt, not a courtesy",
                authored_skips, skip_budget
            ),
            "fix or delete the skipped tests, or consciously budget them: [test] skip_budget in Largo.toml",
        )
        .into());
    }
    if !report.all_passed() {
        return Err(CliError::new(format!("{} test(s) failed", report.failed)).into());
    }
    Ok(())
}

/// The line for a page whose tests did not run because it does not compile.
fn did_not_compile(page: &Uncompiled) -> String {
    format!(
        "{} did not compile, so its {} test(s) did not run:\n{}",
        page.file, page.tests, page.diagnostic
    )
}
