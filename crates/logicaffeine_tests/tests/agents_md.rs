//! AGENTS.md — the self-teaching file every scaffolded project carries.
//!
//! Three locks: `largo new` and `largo init` both write it; every `largo …`
//! command line inside it re-parses against the real clap surface (the
//! teaching doc can never drift from the CLI); and the repo-root copy stays
//! byte-locked to the template plus its canonical preamble.

#[path = "cli_common/mod.rs"]
mod common;

use clap::CommandFactory;
use common::*;
use tempfile::tempdir;

const TEMPLATE: &str = include_str!("../../../apps/logicaffeine_cli/templates/AGENTS.md");

/// The repo-root rendering: a preamble locating the file, then the template.
const ROOT_PREAMBLE: &str = "\
<!-- Canonical copy of apps/logicaffeine_cli/templates/AGENTS.md (byte-locked
     by crates/logicaffeine_tests/tests/agents_md.rs). This file teaches agents to
     WRITE LOGOS; toolchain development in this repo is governed by CLAUDE.md. -->

";

#[test]
fn new_scaffolds_agents_md() {
    let dir = tempdir().unwrap();
    let out = largo_in(dir.path(), &["new", "demo"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));

    let agents = std::fs::read_to_string(dir.path().join("demo/AGENTS.md"))
        .expect("largo new must scaffold AGENTS.md");
    assert_eq!(agents, TEMPLATE, "the scaffold is the template, verbatim");
    assert!(agents.contains("largo check --json"));
}

#[test]
fn init_scaffolds_agents_md() {
    let dir = tempdir().unwrap();
    let out = largo_in(dir.path(), &["init", "--name", "demo"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        dir.path().join("AGENTS.md").exists(),
        "largo init must scaffold AGENTS.md"
    );
}

#[test]
fn agents_md_commands_are_real() {
    let mut checked = 0;
    for line in TEMPLATE.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("largo ") {
            continue;
        }
        let tokens: Vec<&str> = trimmed.split_whitespace().collect();
        logicaffeine_cli::cli::Cli::command()
            .try_get_matches_from(&tokens)
            .unwrap_or_else(|e| {
                panic!("AGENTS.md teaches a command the CLI rejects: `{trimmed}`\n{e}")
            });
        checked += 1;
    }
    assert!(
        checked >= 6,
        "the template must actually teach the loop (found {checked} largo lines)"
    );
}

#[test]
fn root_agents_md_matches_template() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../AGENTS.md");
    let content =
        std::fs::read_to_string(root).expect("the repo root carries the canonical AGENTS.md copy");
    assert_eq!(
        content,
        format!("{ROOT_PREAMBLE}{TEMPLATE}"),
        "root AGENTS.md = preamble + template, byte-locked"
    );
}

/// Every LOGOS snippet the template teaches, in order — its ```logos and ```logos-dom fences, the
/// tags the documentation harness runs (`canonical_teaching_lock`).
fn logos_snippets() -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = TEMPLATE.lines();
    while let Some(line) = lines.next() {
        if !matches!(line.trim(), "```logos" | "```logos-dom") {
            continue;
        }
        let body: Vec<&str> = lines.by_ref().take_while(|l| l.trim() != "```").collect();
        out.push(body.join("\n") + "\n");
    }
    out
}

/// A fence whose first line is a `##` header is LOGOS, and a fence of LOGOS is tagged so a harness
/// runs it — an untagged one is taught without ever being checked.
#[test]
fn every_logos_fence_in_the_template_is_tagged() {
    let mut lines = TEMPLATE.lines();
    while let Some(line) = lines.next() {
        let Some(tag) = line.trim().strip_prefix("```") else {
            continue;
        };
        let body: Vec<&str> = lines.by_ref().take_while(|l| l.trim() != "```").collect();
        let is_logos = body
            .iter()
            .find(|l| !l.trim().is_empty())
            .is_some_and(|l| l.trim_start().starts_with('#'));
        if is_logos {
            let page = body.iter().any(|l| l.starts_with("## Page "));
            let want = if page { "logos-dom" } else { "logos" };
            assert_eq!(
                tag,
                want,
                "a fence of LOGOS is tagged `{want}`:\n{}",
                body.join("\n")
            );
        }
    }
}

/// ★ EVERY SNIPPET THE TEMPLATE TEACHES IS REAL LOGOS, run the way a reader would run it. ★
///
/// AGENTS.md is the first LOGOS an agent reads in a new project and the first it copies. A snippet
/// that does not parse — or a page that parses and does not work — is taught as the canonical form
/// to every project `largo new` creates. So each one is placed where it belongs and put through
/// the command a reader would use: a page is served from `pages/` and its `## Test` must pass; a
/// program with tests must pass them; anything else must check clean.
#[test]
fn agents_md_snippets_are_real_logos() {
    let snippets = logos_snippets();
    assert!(
        snippets.len() >= 4,
        "the template teaches its shapes in code, not prose: {snippets:#?}"
    );
    for (i, snippet) in snippets.iter().enumerate() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let page = snippet.contains("## Page ");
        if page {
            std::fs::write(
                root.join("Largo.toml"),
                "[package]\nname = \"snippet\"\nversion = \"0.1.0\"\n",
            )
            .unwrap();
            std::fs::create_dir_all(root.join("pages")).unwrap();
            std::fs::write(root.join("pages/app.lg"), snippet).unwrap();
        } else {
            scaffold(root, "snippet");
            std::fs::write(root.join("src/main.lg"), snippet).unwrap();
        }
        let check = largo_in(root, &["check"]);
        assert!(
            check.status.success(),
            "snippet {i} does not check:\n{snippet}\n{}",
            stderr(&check)
        );
        if snippet.contains("## Test") {
            let test = largo_in(root, &["test"]);
            assert!(
                test.status.success(),
                "snippet {i}'s own test does not pass:\n{snippet}\n{}{}",
                stdout(&test),
                stderr(&test)
            );
        }
    }
}
