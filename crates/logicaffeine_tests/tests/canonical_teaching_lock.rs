//! ★ LOCK — every surface that teaches LOGOS web development teaches ONE app, and proves it. ★
//!
//! The rule set lives in `common/canonical_teaching.rs`; this binary applies it to the surfaces that
//! are files: the documentation set (`README.md`, `docs/`, `LOGOS_QUICKGUIDE.md`), the `AGENTS.md`
//! every scaffolded project receives, `llms-full.txt`, and `examples/`. The web app's registries (the
//! guide, the Studio examples) are held to the same module by
//! `crates/logicaffeine_tests/tests/web_canonical_teaching_lock.rs`.
//!
//! A snippet in a `logos-dom` fence is parsed, mounted and rendered on the tree-walker and the VM
//! (which must agree), compiled for the browser, and its `## Test` blocks run. A fence that holds
//! LOGOS and carries no LOGOS tag is a failure, because nothing would ever run it.
//!
//! [`PENDING`] names violations on surfaces this lane does not own, each with the request made to
//! the lane that does. It only shrinks: an entry whose violation is gone fails until it is deleted.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test canonical_teaching_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/canonical_teaching.rs"]
mod canon;

use canon::{Pending, Rule, Snippet, Violation};
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// Sorted `*.<ext>` files directly under `dir` (repository-relative).
fn files_in(dir: &str, ext: &str) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(repo_root().join(dir))
        .unwrap_or_else(|e| panic!("cannot list {dir}: {e}"))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some(ext))
        .map(|p| format!("{dir}/{}", p.file_name().unwrap().to_string_lossy()))
        .collect();
    out.sort();
    out
}

/// The example projects under `examples/`, sorted.
fn example_projects() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(repo_root().join("examples"))
        .expect("examples/ exists")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("Largo.toml").is_file())
        .map(|e| format!("examples/{}", e.file_name().to_string_lossy()))
        .collect();
    out.sort();
    out
}

/// Every markdown teaching surface that is a file.
fn markdown_surfaces() -> Vec<String> {
    let mut out = vec![
        "README.md".to_string(),
        "LOGOS_QUICKGUIDE.md".to_string(),
        "apps/logicaffeine_cli/templates/AGENTS.md".to_string(),
    ];
    out.extend(files_in("docs", "md"));
    for project in example_projects() {
        if repo_root().join(&project).join("README.md").is_file() {
            out.push(format!("{project}/README.md"));
        }
    }
    out
}

/// The snippets a markdown surface holds: its fences and its inline code spans.
fn markdown_snippets(rel: &str) -> Vec<Snippet> {
    let text = read(rel);
    let mut out = canon::fences(rel, &text);
    out.extend(canon::inline_spans(rel, &text));
    out
}

/// `llms-full.txt` is the documentation concatenated, each part headed `SOURCE: <path> — <title>`.
/// Its parts are `(source path, text)`.
fn llms_full_parts() -> Vec<(String, String)> {
    let text = read("apps/logicaffeine_web/public/llms-full.txt");
    let rule = "================================================================";
    let mut parts = Vec::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find(&format!("{rule}\nSOURCE: ")) {
        let header = &rest[at + rule.len() + 1..];
        let source = header["SOURCE: ".len()..]
            .split(" — ")
            .next()
            .unwrap_or("")
            .to_string();
        let body_start = header
            .find(&format!("\n{rule}\n\n"))
            .map(|k| k + rule.len() + 3)
            .unwrap_or(header.len());
        let body = &header[body_start..];
        let end = body
            .find(&format!("\n\n{rule}\nSOURCE: "))
            .unwrap_or(body.len());
        parts.push((source, body[..end].to_string()));
        rest = &body[end..];
    }
    parts
}

/// Every page an example project ships, assembled with its imports, as `(surface, source)`.
fn example_pages() -> Vec<(String, Result<String, String>)> {
    let mut out = Vec::new();
    for project in example_projects() {
        let pages = format!("{project}/pages");
        if !repo_root().join(&pages).is_dir() {
            continue;
        }
        for page in files_in(&pages, "lg") {
            let assembled = logicaffeine_compile::analysis::assemble::assemble_project(
                &repo_root().join(&page),
            )
            .map(|a| a.source);
            out.push((page, assembled));
        }
    }
    out
}

/// Every snippet on every file surface, with the findings no single snippet carries.
fn survey() -> (Vec<Snippet>, Vec<Violation>) {
    let mut snippets = Vec::new();
    let markdown = markdown_surfaces();
    for rel in &markdown {
        snippets.extend(markdown_snippets(rel));
    }
    let mut found = Vec::new();

    // `llms_lock` holds each part of llms-full.txt byte-equal to its source file, so a part whose
    // source is surveyed above is covered by that survey; a part from anywhere else is read here.
    for (source, text) in llms_full_parts() {
        if !markdown.contains(&source) {
            let surface = format!("apps/logicaffeine_web/public/llms-full.txt ({source})");
            snippets.extend(canon::fences(&surface, &text));
            snippets.extend(canon::inline_spans(&surface, &text));
        }
    }

    for (page, assembled) in example_pages() {
        match assembled {
            Ok(source) => snippets.push(Snippet::page_source(page, source)),
            Err(e) => found.push(Violation {
                rule: Rule::DoesNotParse,
                location: page,
                detail: format!("the page does not assemble with its imports: {e}"),
                first_line: String::new(),
            }),
        }
    }

    for s in &snippets {
        found.extend(canon::check(s));
    }
    (snippets, found)
}

/// Violations whose fix belongs to another lane — a surface it owns, or a gap in code it owns — each
/// with the request made to that lane.
const PENDING: &[Pending] = &[];

#[test]
fn every_teaching_surface_holds_to_the_canon() {
    let (snippets, found) = survey();
    let report = canon::reconcile(&found, PENDING, &snippets);
    assert!(
        report.is_empty(),
        "{} canonical-teaching violation(s):\n  {}",
        report.len(),
        report.join("\n  ")
    );
}

#[test]
fn the_survey_reaches_every_surface() {
    let (snippets, _) = survey();
    let pages = snippets.iter().filter(|s| s.tag == canon::TAG_PAGE).count();
    assert!(
        pages >= 20,
        "expected the documentation and examples to hold at least 20 pages, found {pages}"
    );
    for surface in [
        "docs/frontend.md",
        "docs/web.md",
        "LOGOS_QUICKGUIDE.md",
        "apps/logicaffeine_cli/templates/AGENTS.md",
    ] {
        assert!(
            snippets.iter().any(|s| s.surface == surface),
            "{surface} contributed no snippet — the survey no longer reads it"
        );
    }
    let parts = llms_full_parts();
    assert!(
        parts.len() >= 10 && parts.iter().any(|(source, _)| source == "docs/frontend.md"),
        "llms-full.txt split into {} part(s) without docs/frontend.md — its separator format changed \
         or the guide left it, and it is no longer checked",
        parts.len()
    );
}

#[test]
fn every_legacy_form_is_caught() {
    let lines = [
        "Render:",
        "Render into app:",
        "    Let count be a signal 0.",
        "Whenever count changes, run redraw.",
        "Put count in the heading.",
    ];
    for line in lines {
        assert!(
            !canon::legacy_forms(line).is_empty(),
            "the legacy form {line:?} is not detected"
        );
    }
    for canonical in [
        "## Component Counter",
        "## Component Counter:",
        "## Component Row (label: Text)",
        "View:",
        "When todos change:",
        "Let count be 0.",
    ] {
        assert!(
            canon::legacy_forms(canonical).is_empty(),
            "{canonical:?} is canonical and must not be flagged"
        );
    }
}

fn rules_broken(tag: &str, attrs: &[&str], body: &str) -> Vec<Rule> {
    let s = Snippet {
        surface: "mutation".into(),
        line: 1,
        tag: tag.into(),
        attrs: attrs.iter().map(|a| a.to_string()).collect(),
        body: body.into(),
    };
    canon::check(&s).into_iter().map(|v| v.rule).collect()
}

const COUNTER_PAGE: &str = "## Page /\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\nView:\n    <button id=\"b\" on-click={bump}>+</button>\n    <p id=\"n\">{n}</p>\n\n## Test: a click counts\n\nClick \"#b\".\nExpect that the text of \"#n\" is equal to \"1\".\n";

#[test]
fn a_canonical_page_breaks_no_rule() {
    assert_eq!(
        rules_broken("logos-dom", &[], COUNTER_PAGE),
        Vec::<Rule>::new()
    );
}

#[test]
fn an_untagged_page_fails_by_name() {
    assert_eq!(
        rules_broken("", &[], COUNTER_PAGE),
        vec![Rule::UntaggedLogos]
    );
    assert_eq!(
        rules_broken("text", &[], "## Test: counts\n\nClick \"#more\".\n"),
        vec![Rule::UntaggedLogos]
    );
}

#[test]
fn a_page_in_a_program_fence_fails_by_name() {
    assert_eq!(
        rules_broken("logos", &[], COUNTER_PAGE),
        vec![Rule::PageNotTaggedAsPage]
    );
}

#[test]
fn a_mounted_component_fails_by_name_unless_the_lesson_is_multi_root() {
    let src = "## Component Counter\nState:\n    Let n be 0.\nView:\n    <p>{n}</p>\n\n## Main\nMount Counter.\n";
    assert_eq!(
        rules_broken("logos-dom", &[], src),
        vec![Rule::MountOutsideTest]
    );
    assert_eq!(
        rules_broken("logos-dom", &[canon::MULTI_ROOT], src),
        Vec::<Rule>::new()
    );
}

#[test]
fn a_mount_inside_a_test_is_canonical() {
    let src = "## Component Badge\nView:\n    <b id=\"b\">new</b>\n\n## Test: it renders\n\nMount Badge.\nExpect that the text of \"#b\" is equal to \"new\".\n";
    assert_eq!(rules_broken("logos-dom", &[], src), Vec::<Rule>::new());
}

#[test]
fn a_component_nothing_mounts_fails_by_name() {
    let src = "## Component Badge\nView:\n    <b>new</b>\n";
    assert_eq!(
        rules_broken("logos-dom", &[], src),
        vec![Rule::RendersNothing]
    );
}

#[test]
fn an_error_page_renders_through_the_router() {
    let src = "## Error page 404\nView:\n    <main><h1>Nothing here</h1></main>\n";
    assert_eq!(rules_broken("logos-dom", &[], src), Vec::<Rule>::new());
    let unreachable = "## Error page 418\nView:\n    <h1>teapot</h1>\n";
    assert_eq!(
        rules_broken("logos-dom", &[], unreachable),
        vec![Rule::FailsOnTheReferenceTier, Rule::RendersNothing]
    );
}

#[test]
fn a_failing_test_fails_by_name() {
    let src = COUNTER_PAGE.replace("is equal to \"1\"", "is equal to \"2\"");
    assert_eq!(rules_broken("logos-dom", &[], &src), vec![Rule::ATestFails]);
}

#[test]
fn a_snippet_that_does_not_parse_fails_by_name() {
    let src = "## Page /\nView:\n    <p>{if}</p>\n";
    assert_eq!(
        rules_broken("logos-dom", &[], src),
        vec![Rule::DoesNotParse]
    );
}

#[test]
fn a_logos_dom_fence_without_a_page_fails_by_name() {
    assert_eq!(
        rules_broken("logos-dom", &[], "## Main\nShow 1.\n"),
        vec![Rule::NotAPage]
    );
}

#[test]
fn a_legacy_view_fails_by_name() {
    let src = "## Main\nRender:\n    div:\n        \"hi\"\n";
    assert!(rules_broken("logos", &[], src).contains(&Rule::LegacyDialect));
}

#[test]
fn a_pending_entry_that_no_longer_matches_fails() {
    let pending = [Pending {
        surface: "nowhere.md",
        rule: Rule::LegacyDialect,
        needle: "Render:",
        request: "none",
    }];
    let report = canon::reconcile(&[], &pending, &[]);
    assert_eq!(
        report.len(),
        1,
        "a fixed pending entry must fail until it is deleted: {report:?}"
    );
}

#[test]
fn an_unlisted_violation_fails() {
    let s = Snippet {
        surface: "x.md".into(),
        line: 3,
        tag: "".into(),
        attrs: vec![],
        body: COUNTER_PAGE.into(),
    };
    let found = canon::check(&s);
    let report = canon::reconcile(&found, &[], &[s]);
    assert_eq!(report.len(), 1, "{report:?}");
    assert!(
        report[0].starts_with("[untagged-logos] x.md:3"),
        "{report:?}"
    );
}

#[test]
fn inline_spans_are_read_outside_fences_only() {
    let text = "Write `Render:` here.\n```\n`not a span`\n```\nand `View:` there.\n";
    let spans: Vec<String> = canon::inline_spans("t.md", text)
        .into_iter()
        .map(|s| s.body)
        .collect();
    assert_eq!(spans, vec!["Render:".to_string(), "View:".to_string()]);
}
