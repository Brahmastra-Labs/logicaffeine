//! ★ LOCK — every page the suite and the examples know traces IDENTICALLY on the reference tier
//! and on the lowered VM, or its divergence is written down. ★
//!
//! A browser runs the page's lowered LOGOS, and so does every in-process tier. This lock runs every
//! page scenario on two engines — the reference, the lowered page hosted by the tree-walker, and
//! the lowered page on the bytecode VM driven by `page_host` — and compares the page after every
//! statement, axis by axis (`page_trace_lock` pins what that comparison means).
//!
//! ★ THE PAGES ARE DISCOVERED, NOT LISTED. A list is complete the day it is written and stale the
//! day after. The corpus is every page in the two locks that drive pages through tests
//! (`headless_runtime_lock.rs`, `component_test_verbs_lock.rs`, read as SOURCE — their string
//! literals, with `format!` templates resolved against the file's `const`s) and every page file
//! under `examples/*/pages/`, plus the pages in `tier_scenarios/` written for the behaviours the
//! tiers are known to treat differently. A `## Main` script is run as a test of its own; a page
//! with no scenario at all is traced as it renders.
//!
//! ★ WHAT DIFFERS IS LEDGERED, SHRINK-ONLY. `lgx_known_divergences.txt` names each divergence —
//! the page, the axis, the first statement it differs at, and one line of cause — and each
//! scenario the lowered tier refuses. An unlisted divergence fails; a listed one that no longer
//! happens fails too, so the ledger can only shrink toward empty, which is the gate for collapsing
//! the tiers into one.

#[path = "support/rust_literals.rs"]
mod rust_literals;

use logicaffeine_compile::page_host::{compare_tests, PageTierKind, TierRun};
use rust_literals::{literals, read_stable, resolve_format};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const BUDGET: u64 = 50_000_000;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// A page source and the name it goes by in the ledger.
struct Page {
    id: String,
    source: String,
}

/// The pages a lock file's literals spell. A `## Main` script becomes a `## Test` of its own.
fn pages_in_rust(file: &str) -> Vec<Page> {
    let text = read_stable(&workspace().join(file));
    let lits = literals(&text);
    let consts: BTreeMap<String, String> = lits
        .iter()
        .filter_map(|l| l.const_name.clone().map(|n| (n, l.value.clone())))
        .collect();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut out = Vec::new();
    for l in &lits {
        let value = if l.is_format {
            match resolve_format(&l.value, &consts) {
                Some(v) => v,
                None => continue,
            }
        } else {
            l.value.clone()
        };
        if !(value.contains("## Component") || value.contains("## Page")) {
            continue;
        }
        let has_test = value.contains("## Test");
        let main_mounts = value.contains("## Main") && value.contains("Mount ");
        if !has_test && !main_mounts {
            continue;
        }
        let source = value.replace("\n## Main\n", "\n## Test: main\n");
        let n = seen.entry(l.owner.clone()).or_insert(0);
        *n += 1;
        let short = Path::new(file)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let id = if *n == 1 {
            format!("{short}::{}", l.owner)
        } else {
            format!("{short}::{}#{n}", l.owner)
        };
        out.push(Page { id, source });
    }
    out
}

/// Every page file under `examples/*/pages/`, read through its own project closure
/// ([`logicaffeine_compile::page_host::assemble_stable`] — the same function `largo build
/// --static` compiles from), not a raw single-file read: a page that shares a layout or a type
/// across files is a normal project, not a corpus-discovery gap. One that carries no scenario of
/// its own is traced as it renders; a `## Main` script becomes a test of its own.
fn example_pages() -> Vec<Page> {
    let root = workspace();
    let mut out = Vec::new();
    let mut apps: Vec<PathBuf> = std::fs::read_dir(root.join("examples"))
        .unwrap()
        .flatten()
        .map(|e| e.path().join("pages"))
        .filter(|p| p.is_dir())
        .collect();
    apps.sort();
    for pages in apps {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&pages)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "lg"))
            .collect();
        files.sort();
        for f in files {
            let text = logicaffeine_compile::page_host::assemble_stable(&f)
                .unwrap_or_else(|e| panic!("could not stably assemble {}: {e}", f.display()));
            let mut source = text.replace("\n## Main\n", "\n## Test: main\n");
            if !source.contains("## Test") {
                source.push_str("\n## Test: the page renders\n    Expect that 1 is equal to 1.\n");
            }
            let id = f
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push(Page { id, source });
        }
    }
    out
}

/// G16 — `example_pages()` must read a page through its own project closure
/// ([`logicaffeine_compile::page_host::assemble_stable`]), not a raw single-file read: a page that
/// shares a layout or a type across files (`Import Layout.` + `{extends Site}`, the documented
/// `docs/frontend.md` Layouts pattern) is a normal, working project on every real product surface
/// (`largo check`/`test`/`build --static`) and must be a normal, working discovery here too.
#[test]
fn example_pages_resolves_a_cross_file_layout_import() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("layout.lg"),
        "## Component Site\n\n\
         View:\n    <header>nav</header>\n    <main>{block content}</main>\n",
    )
    .unwrap();
    let page = dir.path().join("about.lg");
    std::fs::write(
        &page,
        "Import Layout.\n\n\
         ## Page /about\n\n\
         View:\n    {extends Site}\n    {block content:}<h1 id=\"title\">About</h1>{end}\n\n\
         ## Test: it renders under the layout\n\n\
         Expect that the text of \"#title\" is equal to \"About\".\n",
    )
    .unwrap();

    let raw = read_stable(&page);
    let raw_err = compare_tests(&raw, &[PageTierKind::LoweredVm], 0, BUDGET, None)
        .expect_err("a raw single-file read never resolves the sibling `Import` — this is the bug");
    assert!(
        raw_err.contains("Site"),
        "the unfixed reading fails at `{{extends Site}}`, not at the import: {raw_err:?}"
    );

    let assembled = logicaffeine_compile::page_host::assemble_stable(&page)
        .expect("assemble_stable resolves the sibling `Import` before parsing");
    let results = compare_tests(&assembled, &[PageTierKind::LoweredVm], 0, BUDGET, None)
        .expect("the assembled source compiles and runs cleanly on the lowered VM tier");
    assert_eq!(
        results.len(),
        1,
        "exactly the one `## Test` this fixture declares"
    );
    assert!(
        matches!(&results[0].candidates[0].1, TierRun::Traced(_)),
        "the lowered VM must run this test, not refuse it: {:?}",
        results[0].candidates[0].1
    );
}

/// G12 — every tier `page_host::Scenario` drives must seed `routePath()` from a `## Page`'s own
/// declared URL the moment it auto-mounts, not the boot default `"/"`
/// (`Scenario::route()`/`World::enter`, `crates/logicaffeine_compile/src/page_host/scenario.rs`).
/// `gauntlet_findings_lock::a_pages_own_test_sees_its_own_declared_route` pins the SAME fixture's
/// tree-walker VERDICT through `testrun.rs`'s own separate execution path (its inline interpreter
/// run, not a `Scenario`); this pins the `Scenario`-driven reference trace AND the lowered VM
/// candidate directly, since the two paths do not share code and could regress independently.
#[test]
fn a_non_root_pages_scenario_sees_its_own_declared_route_on_every_tier() {
    let source = "\
## Page /about

View:
    <p id=\"path\">{routePath()}</p>

## Test: the page's own test sees its own URL

Expect that the text of \"#path\" is equal to \"/about\".
";
    let results = compare_tests(source, &[PageTierKind::LoweredVm], 0, BUDGET, None)
        .expect("the fixture compiles and runs cleanly");
    assert_eq!(
        results.len(),
        1,
        "exactly the one `## Test` this fixture declares"
    );
    let comparison = &results[0];
    let reference_doc = &comparison.reference.steps.last().unwrap().snapshot.document;
    assert!(
        reference_doc.contains("/about"),
        "the reference tier must render the page's own declared URL, not the boot default \"/\": \
         {reference_doc}"
    );
    match &comparison.candidates[0].1 {
        TierRun::Traced(t) => {
            let vm_doc = &t.steps.last().unwrap().snapshot.document;
            assert!(
                vm_doc.contains("/about"),
                "the lowered VM tier must seed the same route the reference tier does: {vm_doc}"
            );
        }
        other => panic!("the lowered VM must run this test, not refuse it: {other:?}"),
    }
}

#[test]
fn zzz_diag_trapping_turn_journal() {
    use logicaffeine_compile::page_host::Axis;
    let source = read_stable(
        &workspace().join("crates/logicaffeine_tests/tests/tier_scenarios/trapping_turn.lg"),
    );
    let results = compare_tests(&source, &[PageTierKind::LoweredVm], 0, BUDGET, None).unwrap();
    let c = &results[0];
    eprintln!("REFERENCE journal steps:");
    for s in &c.reference.steps {
        eprintln!(
            "  stmt {}: journal={:?}",
            s.statement,
            s.snapshot.axis(Axis::Journal)
        );
    }
    if let TierRun::Traced(t) = &c.candidates[0].1 {
        eprintln!("CANDIDATE journal steps:");
        for s in &t.steps {
            eprintln!(
                "  stmt {}: journal={:?}",
                s.statement,
                s.snapshot.axis(Axis::Journal)
            );
        }
    }
}

/// `Replay these actions:` on the lowered VM tier (`tier_scenarios/replay.lg`'s own scenario) —
/// each named action calls its lowered entry (`__act_<C>_<action>`, via [`LoweredCalls`]) with no
/// arguments, exactly as the reference tier's `replay_actions` does
/// (`crate::page_host::lowered::LoweredPage::replay`).
///
/// [`LoweredCalls`]: logicaffeine_compile::page_host::LoweredCalls
#[test]
fn replay_runs_each_named_action_on_the_lowered_vm() {
    let source = "\
## Component App
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p><button id=\"b\" on-click={bump}>+</button>

## Test: replay runs the named actions
    Mount App.
    Replay these actions: bump, bump.
    Expect that the text of \"#n\" is equal to \"2\".
";
    let results = compare_tests(source, &[PageTierKind::LoweredVm], 0, BUDGET, None)
        .expect("the fixture compiles and runs cleanly");
    assert_eq!(
        results.len(),
        1,
        "exactly the one `## Test` this fixture declares"
    );
    match &results[0].candidates[0].1 {
        TierRun::Traced(t) => {
            let doc = &t.steps.last().unwrap().snapshot.document;
            assert!(
                doc.contains("<p id=\"n\">2</p>"),
                "both replayed `bump`s must run against the mounted component: {doc}"
            );
        }
        other => panic!(
            "`Replay these actions:` must run on the lowered VM tier, not refuse it: {other:?}"
        ),
    }
}

const LOCK_FILES: [&str; 2] = [
    "crates/logicaffeine_tests/tests/headless_runtime_lock.rs",
    "crates/logicaffeine_tests/tests/component_test_verbs_lock.rs",
];

/// The scenarios written for this lock: a page per behaviour the tiers are known to treat
/// differently, or that no other corpus page reaches (a trapping turn, a failure handler).
fn scenario_pages() -> Vec<Page> {
    let root = workspace();
    let dir = root.join("crates/logicaffeine_tests/tests/tier_scenarios");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "lg"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|f| Page {
            id: f
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/"),
            source: read_stable(&f),
        })
        .collect()
}

fn corpus() -> Vec<Page> {
    let mut pages: Vec<Page> = LOCK_FILES.iter().flat_map(|f| pages_in_rust(f)).collect();
    pages.extend(example_pages());
    pages.extend(scenario_pages());
    pages
}

// ---- The ledger. ----
//
// Parsing, the `(scenario, kind, statement) -> cause` map, and the staleness rule
// (`page_host::stale_rows`, tested in `page_host::ledger`'s own unit tests) are ALL shared with
// `browser_tier_lock`'s corpus lock, so the two readers cannot drift.

/// `(scenario, axis or "refused", statement) -> cause` — this lock's own plain rows only.
fn ledger() -> BTreeMap<(String, String, usize), String> {
    let rows =
        logicaffeine_compile::page_host::parse_ledger(include_str!("lgx_known_divergences.txt"));
    logicaffeine_compile::page_host::ledger_map(
        &rows,
        logicaffeine_compile::page_host::LedgerTier::LoweredVm,
    )
}

/// What every scenario of the corpus does on the lowered tier, keyed as the ledger keys it, with
/// the detail a failure message shows. `unparsed_pages` names each page (its own `Page::id`) whose
/// source did not even parse — none of ITS scenarios reached a single axis. `refused_at_zero` names
/// each scenario (`id::test`) refused at statement 0 — before any axis was ever compared, exactly
/// as unreachable as an unparsed page's own scenarios, just at the per-scenario rather than the
/// per-page grain. Both feed [`stale_rows`]: a scenario a run could not even CHECK must never be
/// mistaken for one that ran and quietly started agreeing.
fn observed(
    pages: &[Page],
) -> (
    BTreeMap<(String, String, usize), String>,
    Vec<String>,
    usize,
    Vec<String>,
    BTreeSet<String>,
) {
    let mut found = BTreeMap::new();
    let mut unparsed = Vec::new();
    let mut unparsed_pages = Vec::new();
    let mut refused_at_zero = BTreeSet::new();
    let mut scenarios = 0;
    for page in pages {
        let all = match compare_tests(&page.source, &[PageTierKind::LoweredVm], 0, BUDGET, None) {
            Ok(all) => all,
            Err(e) => {
                unparsed.push(format!("{}: {}", page.id, e.lines().next().unwrap_or("")));
                unparsed_pages.push(page.id.clone());
                continue;
            }
        };
        for c in all {
            scenarios += 1;
            let id = format!("{}::{}", page.id, c.test);
            match &c.candidates[0].1 {
                TierRun::Refused { statement, reason } => {
                    if *statement == 0 {
                        refused_at_zero.insert(id.clone());
                    }
                    found.insert((id, "refused".to_string(), *statement), reason.clone());
                }
                TierRun::Traced(t) => {
                    for a in c.reference.compare(t).axes {
                        let detail = logicaffeine_compile::page_host::describe_difference(
                            &a.reference,
                            &a.candidate,
                        );
                        found.insert((id.clone(), a.axis.name().to_string(), a.statement), detail);
                    }
                }
            }
        }
    }
    (found, unparsed, scenarios, unparsed_pages, refused_at_zero)
}

#[test]
fn every_discovered_page_traces_identically_or_is_ledgered() {
    let pages = corpus();
    let (found, unparsed, scenarios, unparsed_pages, refused_at_zero) = observed(&pages);
    let known = ledger();
    let unlisted: Vec<String> = found
        .iter()
        .filter(|(k, _)| !known.contains_key(*k))
        .map(|((id, kind, at), detail)| format!("{id} | {kind} @ {at} | <cause>\n{detail}"))
        .collect();
    let stale: Vec<String> = logicaffeine_compile::page_host::stale_rows(
        &known,
        &found,
        &unparsed_pages,
        &refused_at_zero,
    )
    .into_iter()
    .map(|(id, kind, at)| format!("{id} | {kind} @ {at}"))
    .collect();
    assert!(
        unlisted.is_empty() && stale.is_empty(),
        "{scenarios} scenarios over {} pages ({} did not parse: {unparsed:#?}).\n\n\
         UNLISTED divergences (fix the tier, or ledger each with its cause):\n{}\n\n\
         STALE ledger entries (they agree now — delete them; the ledger only shrinks):\n{}",
        pages.len(),
        unparsed.len(),
        unlisted.join("\n"),
        stale.join("\n")
    );
}

#[test]
fn the_corpus_is_discovered_from_every_source_it_names() {
    // A discovery that silently found nothing would make the ledger lock above vacuously green.
    for file in LOCK_FILES {
        let pages = pages_in_rust(file);
        assert!(
            pages.len() >= 6,
            "{file}: only {} page scenarios found — the reader is broken",
            pages.len()
        );
    }
    let examples = example_pages();
    let files: BTreeSet<&str> = examples.iter().map(|p| p.id.as_str()).collect();
    for app in [
        "tic_tac_toe",
        "task_board",
        "fullstack_todo",
        "marketing_site",
    ] {
        assert!(
            files
                .iter()
                .any(|f| f.contains(&format!("examples/{app}/pages/"))),
            "the examples' pages must be in the corpus: {files:?}"
        );
    }
    let (_, unparsed, scenarios, _, _) = observed(&corpus());
    assert!(
        unparsed.is_empty(),
        "every discovered page parses: {unparsed:#?}"
    );
    assert!(
        scenarios >= 30,
        "the corpus runs {scenarios} scenarios — fewer than it held when written"
    );
}

#[test]
fn the_literal_reader_reads_what_rustc_reads() {
    let src = r####"
        const A: &str = "x\n\
            y";
        fn one() { let s = format!("{A}## Test: t\n    Click \"#b\".\n{{z}}"); let c = '"'; }
        // "not a literal"
        fn two() { let r = r#"raw "quoted" {x}"#; let e = "\x20\u{41}"; }
    "####;
    let lits = literals(src);
    let values: Vec<&str> = lits.iter().map(|l| l.value.as_str()).collect();
    assert_eq!(
        values,
        [
            "x\ny",
            "{A}## Test: t\n    Click \"#b\".\n{{z}}",
            "raw \"quoted\" {x}",
            " A"
        ]
    );
    assert_eq!(lits[0].const_name.as_deref(), Some("A"));
    assert!(lits[1].is_format && lits[1].owner == "one");
    assert_eq!(lits[2].owner, "two");
    let consts: BTreeMap<String, String> = [("A".to_string(), "PAGE".to_string())].into();
    assert_eq!(
        resolve_format(&lits[1].value, &consts).as_deref(),
        Some("PAGE## Test: t\n    Click \"#b\".\n{z}")
    );
    assert_eq!(
        resolve_format("{}x", &consts),
        None,
        "a positional hole is not a constant"
    );
}
