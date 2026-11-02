//! ★ LOCK — the guide and the Studio teach the one canonical app, and prove it. ★
//!
//! The rule set is `crates/logicaffeine_tests/tests/common/canonical_teaching.rs`, shared with the
//! file-surface applier `crates/logicaffeine_tests/tests/canonical_teaching_lock.rs`. This binary
//! applies it to the registries the web app ships, read in memory rather than from source text: the
//! guide's sections (prose fences, examples, exercise answer keys) and the Studio's examples.
//!
//! A guide or Studio page is mounted and rendered on the tree-walker and the VM (which must agree),
//! compiled for the browser, and its `## Test` blocks run. A LOGOS program that declares a page must
//! be a web example (`ExampleMode::Web` in the guide, `Expected::WebPage` in the Studio) — a page
//! filed as an ordinary program is taught as something it is not.

#![cfg(feature = "web-tests")]

#[path = "common/canonical_teaching.rs"]
mod canon;

use canon::{Pending, Rule, Snippet, Violation};
use logicaffeine_compile::compile::compile_to_rust;
use logicaffeine_web::ui::examples::{Expected, ALL_CODE_EXAMPLES};
use logicaffeine_web::ui::pages::guide::content::{ExampleMode, SECTIONS};

/// Every teaching snippet the web app ships.
fn survey() -> Vec<Snippet> {
    let mut out = Vec::new();
    for section in SECTIONS.iter() {
        let surface = format!("guide §{} {}", section.number, section.id);
        out.extend(canon::fences(&surface, section.content));
        out.extend(canon::inline_spans(&surface, section.content));
        for ex in section.examples {
            let where_ = format!("guide example {}", ex.id);
            match ex.mode {
                ExampleMode::Web => out.push(Snippet::page_source(where_, ex.code)),
                ExampleMode::Imperative => out.push(program(where_, ex.code)),
                ExampleMode::Logic | ExampleMode::Theorem => {}
            }
        }
        for ex in section.exercises {
            let where_ = format!("guide exercise {} (answer key)", ex.id);
            match ex.mode {
                ExampleMode::Web => out.push(Snippet::page_source(where_, ex.reference)),
                ExampleMode::Imperative => out.push(program(where_, ex.reference)),
                ExampleMode::Logic | ExampleMode::Theorem => {}
            }
        }
    }
    for spec in ALL_CODE_EXAMPLES {
        let where_ = format!("studio {}", spec.vfs_path);
        match spec.expected {
            Expected::WebPage => out.push(Snippet::page_source(where_, spec.source)),
            _ => out.push(program(where_, spec.source)),
        }
    }
    out
}

/// A whole source that is an ordinary program: held to the dialect and page-filing rules, and run
/// by its own harness (`guide_examples`, `example_health`).
fn program(surface: String, body: &str) -> Snippet {
    Snippet {
        surface,
        line: 0,
        tag: canon::TAG_LOGOS.to_string(),
        attrs: Vec::new(),
        body: body.to_string(),
    }
}

/// A guide prose fence tagged `logos` is shown as a program, so it must compile like one — the
/// guide has no other harness for prose.
fn prose_programs_compile(snippets: &[Snippet]) -> Vec<Violation> {
    snippets
        .iter()
        .filter(|s| s.line > 0 && s.tag == canon::TAG_LOGOS && s.surface.starts_with("guide §"))
        .filter_map(|s| {
            let why = match compile_to_rust(&s.body) {
                Ok(rust) if rust.contains("compile_error!") => {
                    "the native tier refuses it".to_string()
                }
                Ok(_) => return None,
                Err(e) => format!("{e:?}"),
            };
            Some(Violation {
                rule: Rule::DoesNotCompile,
                location: s.location(),
                detail: why,
                first_line: String::new(),
            })
        })
        .collect()
}

/// Violations whose fix belongs to another lane, each with its request.
const PENDING: &[Pending] = &[];

#[test]
fn the_guide_and_the_studio_hold_to_the_canon() {
    let snippets = survey();
    let mut found: Vec<Violation> = snippets.iter().flat_map(canon::check).collect();
    found.extend(prose_programs_compile(&snippets));
    let report = canon::reconcile(&found, PENDING, &snippets);
    assert!(
        report.is_empty(),
        "{} canonical-teaching violation(s) in the web app:\n  {}",
        report.len(),
        report.join("\n  ")
    );
}

#[test]
fn the_survey_reaches_the_guide_and_the_studio() {
    let snippets = survey();
    let guide_pages = snippets
        .iter()
        .filter(|s| s.tag == canon::TAG_PAGE && s.surface.starts_with("guide "))
        .count();
    let studio_pages = snippets
        .iter()
        .filter(|s| s.tag == canon::TAG_PAGE && s.surface.starts_with("studio "))
        .count();
    assert!(
        guide_pages >= 4,
        "the guide's web examples and exercise keys are pages: found {guide_pages}"
    );
    assert!(
        studio_pages >= 3,
        "the Studio's web examples are pages: found {studio_pages}"
    );
    assert!(
        snippets
            .iter()
            .any(|s| s.surface.starts_with("guide §") && s.tag == canon::TAG_LOGOS),
        "the guide's prose fences are read"
    );
}

#[test]
fn a_page_filed_as_a_program_fails_by_name() {
    let src = "## Page /\nView:\n    <p>hi</p>\n";
    let found = canon::check(&program("studio mutation".into(), src));
    assert_eq!(
        found.iter().map(|v| v.rule).collect::<Vec<_>>(),
        vec![Rule::PageNotTaggedAsPage]
    );
}
