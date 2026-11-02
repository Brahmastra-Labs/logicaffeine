//! Every browser scenario a doc under `docs/` cites as a lock exists in its harness.
//!
//! The plans (`docs/plans/studio-cohesion-plan.md`, `docs/plans/studio-examples-audit.md`, …)
//! record each change beside what holds it in place. A browser scenario lives in a JavaScript
//! harness that no compiler reads: rename it, or lose it in an edit, and the plan goes on citing a
//! lock that no longer locks anything. This reads each `scripts/web/test-<page>-e2e.sh (…)` citation
//! in every doc, the guides and the plans alike, and finds every scenario it names in
//! `scripts/web/test-<page>-e2e.mjs`. A name ending in `*` stands for every scenario with that prefix,
//! and must match at least one.
//!
//! `docs/plans/` is gitignored: the plans live on the machine they are written on. A clone has
//! none, so there it checks the guides alone.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Each browser-suite citation in `plan`: the harness it runs and the scenario names in the
/// parentheses that follow it, on the same line or wrapped to the next. Flags (`--firefox`) and
/// prose (`at 1920×1080`) are not names.
fn citations(plan: &str) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut rest = plan;
    while let Some(at) = rest.find("`scripts/web/test-") {
        let tail = &rest[at + 1..];
        let Some(close) = tail.find('`') else { break };
        let script = tail[..close].split_whitespace().next().unwrap_or_default();
        rest = &tail[close + 1..];
        let (Some(harness), Some(group)) = (script.strip_suffix(".sh"), rest.trim_start().strip_prefix('(')) else {
            continue;
        };
        let Some(end) = group.find(')') else { continue };
        let names = group[..end]
            .split('`')
            .skip(1)
            .step_by(2)
            .filter(|name| !name.starts_with('-'))
            .map(str::to_string)
            .collect();
        out.push((format!("{harness}.mjs"), names));
    }
    out
}

/// The scenario names a harness registers: `scenario('<name>', …)`.
fn scenarios(harness: &str) -> Vec<&str> {
    harness
        .split("scenario('")
        .skip(1)
        .filter_map(|tail| tail.split('\'').next())
        .collect()
}

fn found(name: &str, registered: &[&str]) -> bool {
    match name.strip_suffix('*') {
        Some(prefix) => registered.iter().any(|s| s.starts_with(prefix)),
        None => registered.contains(&name),
    }
}

/// Every Markdown file under `docs/`, the plans in `docs/plans/` included, by its path under
/// `docs/` and its text.
fn docs() -> Vec<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for path in std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .filter_map(|entry| entry.ok().map(|e| e.path()))
        {
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "md") {
                out.push(path);
            }
        }
    }
    let root = repo().join("docs");
    let mut paths = Vec::new();
    walk(&root, &mut paths);
    let mut out: Vec<(String, String)> = paths
        .into_iter()
        .map(|path| {
            let name = path.strip_prefix(&root).expect("a path under docs/").to_string_lossy().into_owned();
            (name, std::fs::read_to_string(&path).expect("read a doc"))
        })
        .collect();
    out.sort();
    out
}

/// A plan in `docs/plans/`, if this checkout has it. A clone never does, since the folder is
/// gitignored, and then there is nothing of the plan's to check.
fn plan(name: &str) -> Option<String> {
    let path = repo().join("docs/plans").join(name);
    match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => panic!("read {}: {e}", path.display()),
    }
}

/// The plans this checkout has, by file name.
fn plan_names() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(repo().join("docs/plans")) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .map(|path| path.file_name().expect("a file name").to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn every_browser_lock_a_doc_cites_exists() {
    let mut missing = Vec::new();
    for (doc, text) in docs() {
        for (harness, names) in citations(&text) {
            let source = std::fs::read_to_string(repo().join(&harness))
                .unwrap_or_else(|e| panic!("docs/{doc} cites {harness}, which cannot be read: {e}"));
            let registered = scenarios(&source);
            for name in names.iter().filter(|name| !found(name, &registered)) {
                missing.push(format!("docs/{doc} → {harness}: {name}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "{} browser lock(s) cited in docs/ are not in their harness:\n{}",
        missing.len(),
        missing.join("\n")
    );
}

#[test]
fn every_plan_is_read_with_the_guides() {
    let read: Vec<String> = docs().into_iter().map(|(doc, _)| doc).collect();
    for name in plan_names() {
        let doc = format!("plans/{name}");
        assert!(read.contains(&doc), "docs/{doc} is not among the docs whose citations are checked");
    }
}

#[test]
fn the_examples_audit_is_read() {
    let Some(audit) = plan("studio-examples-audit.md") else { return };
    assert!(
        !citations(&audit).is_empty(),
        "docs/plans/studio-examples-audit.md cites no browser lock the reader can see"
    );
}

#[test]
fn the_studio_plan_cites_its_browser_locks() {
    let Some(plan) = plan("studio-cohesion-plan.md") else { return };
    let cited: usize = citations(&plan).iter().map(|(_, names)| names.len()).sum();
    assert!(cited >= 40, "only {cited} browser locks were read from the plan — the citation reader has drifted from its format");
}

#[test]
fn a_citation_names_the_scenarios_in_its_parentheses() {
    assert_eq!(
        citations("x `scripts/web/test-studio-e2e.sh` (`a_bar_moves`, `the_drawer_*`) y"),
        vec![("scripts/web/test-studio-e2e.mjs".to_string(), vec!["a_bar_moves".to_string(), "the_drawer_*".to_string()])]
    );
}

#[test]
fn a_citation_with_flags_still_names_its_harness() {
    assert_eq!(
        citations("`scripts/web/test-studio-e2e.sh --firefox` (`the_scrollbars_paint_dark`)"),
        vec![("scripts/web/test-studio-e2e.mjs".to_string(), vec!["the_scrollbars_paint_dark".to_string()])]
    );
}

#[test]
fn flags_and_prose_in_the_parentheses_are_not_names() {
    assert_eq!(
        citations("`scripts/web/test-studio-e2e.sh` (`--firefox`) and `scripts/web/test-learn-e2e.sh` (`a_page_loads`, at 1920×1080 and phone)"),
        vec![
            ("scripts/web/test-studio-e2e.mjs".to_string(), vec![]),
            ("scripts/web/test-learn-e2e.mjs".to_string(), vec!["a_page_loads".to_string()]),
        ]
    );
}

#[test]
fn a_citation_wrapped_onto_the_next_line_still_names_its_scenarios() {
    assert_eq!(
        citations("Green: `scripts/web/test-studio-e2e.sh`\n      (`the_toggle_is_square`, desktop and phone)"),
        vec![("scripts/web/test-studio-e2e.mjs".to_string(), vec!["the_toggle_is_square".to_string()])]
    );
}

#[test]
fn a_citation_without_parentheses_names_nothing() {
    assert!(citations("the suite `scripts/web/test-studio-e2e.sh`; both engines").is_empty());
    assert!(citations("`scripts/web/stage-web-data.sh` (`restaging`)").is_empty(), "only browser suites are read");
}

#[test]
fn a_harness_registers_each_scenario_by_name() {
    let harness = "scenario('first', async (t) => {});\nscenario('second_one', 'V2', async (t) => {});\nconst scenarioCount = 2;";
    assert_eq!(scenarios(harness), vec!["first", "second_one"]);
}

#[test]
fn a_starred_name_needs_one_scenario_with_its_prefix() {
    let registered = ["the_drawer_grip_drags_freely", "a_bar_follows_the_pointer"];
    assert!(found("the_drawer_*", &registered));
    assert!(!found("the_output_*", &registered));
    assert!(found("a_bar_follows_the_pointer", &registered));
    assert!(!found("a_bar_follows", &registered));
}
