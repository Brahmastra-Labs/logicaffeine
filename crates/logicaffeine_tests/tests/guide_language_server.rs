//! The guide's code under the language server the Studio's editor runs.
//!
//! Every LOGOS example, exercise and prose snippet on `/guide` sits in the shared editor with
//! the language server on, so whatever the server says about the guide's own code is painted
//! under it: an error or warning squiggle beneath code the guide presents as working would tell
//! the reader the guide is wrong. These locks read the guide's code the way the page does —
//! `logicaffeine_lsp`'s analysis through `components::problems` — and hold it to that promise.

#![cfg(feature = "web-tests")]

use std::collections::BTreeSet;

use logicaffeine_lsp::document::DocumentState;
use logicaffeine_web::ui::components::problems::{problems, Problem, Severity};
use logicaffeine_web::ui::pages::guide::content::{ExampleMode, Runnability, SECTIONS};
use logicaffeine_web::ui::pages::guide::{fence, prose_blocks, Fence, ProseBlock};

/// Exact census of the LOGOS and Logic snippets the guide's prose quotes. Deliberately exact: a
/// snippet the splitter stopped finding must not pass for a snippet that went quiet.
const PROSE_SNIPPET_CENSUS: usize = 2;

fn found(source: &str) -> Vec<Problem> {
    problems(&DocumentState::new(source.to_string(), 0))
}

/// The errors and warnings in `source`, one line each, where a reader would see them.
fn loud(source: &str) -> Vec<String> {
    found(source)
        .into_iter()
        .filter(|p| p.severity >= Severity::Warning)
        .map(|p| {
            let line = source[..p.start].matches('\n').count() + 1;
            format!(
                "line {line} {:?} {} on {:?}: {}",
                p.severity,
                p.code.as_deref().unwrap_or("-"),
                &source[p.start..p.end],
                p.message.lines().next().unwrap_or("")
            )
        })
        .collect()
}

/// The fences a section's prose opens, by a plain reading of its lines: each opening line's tag.
fn fence_tags(content: &str) -> Vec<String> {
    let mut open = false;
    let mut tags = Vec::new();
    for line in content.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("```") {
            if !open {
                tags.push(rest.trim().to_string());
            }
            open = !open;
        }
    }
    tags
}

/// The LOGOS and Logic snippets the guide's prose quotes: `(section, code)`.
fn prose_snippets() -> Vec<(&'static str, String)> {
    SECTIONS
        .iter()
        .flat_map(|section| {
            prose_blocks(section.content).into_iter().filter_map(move |block| match block {
                ProseBlock::Code { tag, code } if matches!(fence(&tag), Some(Fence::Logos | Fence::Logic)) => {
                    Some((section.id, code))
                }
                _ => None,
            })
        })
        .collect()
}

/// A fence says what it holds: the page reads LOGOS and Logic in the shared editor and shows
/// shell and config text plainly, so an untagged or misspelt fence would quietly lose the
/// editor its code deserves.
#[test]
fn every_fence_names_a_language_the_guide_renders() {
    let mut wrong = Vec::new();
    let mut total = 0;
    for section in SECTIONS.iter() {
        for tag in fence_tags(section.content) {
            total += 1;
            if fence(&tag).is_none() {
                wrong.push(format!(
                    "§{} {}: a fence tagged {tag:?} — name its language (logos, logic, bash, powershell or toml)",
                    section.number, section.id
                ));
            }
        }
    }
    assert!(total > 0, "the guide's prose opens no fences at all — the scan is reading nothing");
    assert!(wrong.is_empty(), "{} of {total} fence(s) the page cannot place:\n{}", wrong.len(), wrong.join("\n"));
}

/// The page renders what the prose fences hold as code: every fence comes out of the splitter
/// as its own block, none left inside the markup.
#[test]
fn the_page_splits_out_every_fence() {
    for section in SECTIONS.iter() {
        let blocks = prose_blocks(section.content);
        let code = blocks.iter().filter(|b| matches!(b, ProseBlock::Code { .. })).count();
        let expected = fence_tags(section.content).len();
        assert_eq!(code, expected, "§{} {}: the page splits out {code} of its {expected} fence(s)", section.number, section.id);
        for block in &blocks {
            if let ProseBlock::Html(html) = block {
                assert!(!html.contains("<pre>"), "§{} {}: a fence is still rendered inside the prose markup", section.number, section.id);
            }
        }
    }
}

#[test]
fn every_prose_snippet_opens_without_an_error_or_warning() {
    let snippets = prose_snippets();
    let wrong: Vec<String> = snippets
        .iter()
        .flat_map(|(section, code)| loud(code).into_iter().map(move |p| format!("{section}: {p}")))
        .collect();
    assert_eq!(
        snippets.len(),
        PROSE_SNIPPET_CENSUS,
        "the guide's prose quotes {} LOGOS/Logic snippet(s), census {PROSE_SNIPPET_CENSUS} — update the census consciously",
        snippets.len()
    );
    assert!(wrong.is_empty(), "{} squiggle(s) under code the guide quotes as working:\n{}", wrong.len(), wrong.join("\n"));
}

/// An example that fails on purpose is the one place a squiggle belongs; every other example is
/// presented as working code and must open without an error or warning under it.
#[test]
fn every_example_presented_as_working_opens_without_an_error_or_warning() {
    let mut wrong = Vec::new();
    let mut checked = 0;
    for section in SECTIONS.iter() {
        for example in section.examples.iter() {
            if matches!(example.runnability, Runnability::ExpectedError { .. }) {
                continue;
            }
            checked += 1;
            for problem in loud(example.code) {
                wrong.push(format!("{} ({:?}): {problem}", example.id, example.mode));
            }
        }
    }
    assert!(checked > 100, "only {checked} examples were read");
    assert!(wrong.is_empty(), "{} squiggle(s) under examples presented as working:\n{}", wrong.len(), wrong.join("\n"));
}

/// Where `name` is read in `source`: every whole-word use outside comment lines.
fn reads_of(source: &str, name: &str) -> Vec<usize> {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = Vec::new();
    let mut line_start = 0;
    for line in source.split_inclusive('\n') {
        let code = line.trim_start();
        if !(code.starts_with("--") || (code.starts_with('#') && !code.starts_with("##"))) {
            for (i, _) in line.match_indices(name) {
                let before = line[..i].chars().next_back();
                let after = line[i + name.len()..].chars().next();
                if !before.is_some_and(word) && !after.is_some_and(word) {
                    out.push(line_start + i);
                }
            }
        }
        line_start += line.len();
    }
    out
}

/// "Never used" fades the code it names: it is a promise, so no example, exercise starter or
/// snippet may be told a variable it reads is unused.
#[test]
fn no_guide_code_is_told_a_variable_it_reads_is_never_used() {
    let mut sources: Vec<(String, String)> = Vec::new();
    for section in SECTIONS.iter() {
        for example in section.examples.iter() {
            sources.push((example.id.to_string(), example.code.to_string()));
        }
        for exercise in section.exercises.iter().filter(|e| e.mode != ExampleMode::Logic) {
            sources.push((exercise.id.to_string(), exercise.starter.to_string()));
        }
    }
    for (section, code) in prose_snippets() {
        sources.push((format!("{section} (prose)"), code));
    }
    let mut wrong = BTreeSet::new();
    for (id, source) in &sources {
        for p in found(source).into_iter().filter(|p| p.code.as_deref() == Some("unused-variable")) {
            let name = &source[p.start..p.end];
            let reads = reads_of(source, name).into_iter().filter(|&at| at != p.start).count();
            if reads > 0 {
                let line = source[..p.start].matches('\n').count() + 1;
                wrong.insert(format!("{id}:{line} '{name}' is read {reads} more time(s)"));
            }
        }
    }
    assert!(sources.len() > 200, "only {} pieces of code were read", sources.len());
    assert!(wrong.is_empty(), "{} false \"never used\" hint(s):\n{}", wrong.len(), wrong.into_iter().collect::<Vec<_>>().join("\n"));
}
