//! `scripts/generate-docs.sh` assembles the handbook, and this is what holds the handbook to being one
//! whole, well-formed book.
//!
//! The generator (`scripts/release/docs/handbook.py`, behind the shell entry point) builds
//! `target/docs/LOGOS_DOCUMENTATION.md` from the documents a reader already has: the README's overview,
//! Getting started, every guide, every crate's README, the quick guide, the error catalog and the project
//! documents. The book is an ignored build output, so every check here runs the generator into a scratch
//! directory and reads what it wrote; nothing leans on a copy left over from an earlier run.
//!
//! What the book must be:
//! - **Whole.** Every guide under `docs/`, every document under `docs/project/`, and every workspace
//!   member's README is a chapter, found by its own title.
//! - **Well formed.** Every code fence it opens it closes, every `#anchor` it links resolves to a heading
//!   under GitHub's anchor rules, and no relative link survives outside code, so the one file reads the
//!   same wherever it is opened.
//! - **Current.** Its first lines name the workspace version.
//! - **Loud when an input goes missing.** A declared input that is not there stops the generator with
//!   its name, rather than leaving a quietly shorter book.
//!
//! The last test keeps the documentation's own citations honest: every backticked `*_lock` the README or a
//! guide names is a test that exists.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test generate_docs_paths_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .unwrap()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

const ENTRY: &str = "scripts/generate-docs.sh";
const ASSEMBLER: &str = "scripts/release/docs/handbook.py";

/// Run the generator with `args` against `root`, writing to a scratch file. Returns (success, book or
/// the generator's stderr).
fn generate_from(root: &Path, args: &[&str]) -> (bool, String) {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let out = dir.path().join("book.md");
    let run = Command::new("bash")
        .arg(repo_root().join(ENTRY))
        .args(args)
        .arg("--root")
        .arg(root)
        .arg("--out")
        .arg(&out)
        .output()
        .expect("bash runs the generator");
    if run.status.success() {
        (
            true,
            std::fs::read_to_string(&out).expect("the generator wrote the book"),
        )
    } else {
        (false, String::from_utf8_lossy(&run.stderr).into_owned())
    }
}

fn generate(args: &[&str]) -> String {
    let (ok, text) = generate_from(&repo_root(), args);
    assert!(ok, "{ENTRY} failed:\n{text}");
    text
}

fn is_fence(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

/// GitHub's anchor for a heading: the rendered text lower-cased, keeping letters, digits, `_`, `-` and
/// spaces, each space a hyphen.
fn slug(title: &str) -> String {
    let mut text = String::new();
    let mut chars = title.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '`' | '*' => {}
            '[' => {
                let label: String = chars.by_ref().take_while(|&c| c != ']').collect();
                text.push_str(&label);
                if chars.peek() == Some(&'(') {
                    for c in chars.by_ref() {
                        if c == ')' {
                            break;
                        }
                    }
                }
            }
            c => text.push(c),
        }
    }
    text.trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == ' ')
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// Every heading outside code, in order, with its anchor after GitHub's `-1`, `-2` deduplication.
fn headings(book: &str) -> Vec<(usize, String, String)> {
    let mut out = Vec::new();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut in_fence = false;
    for line in book.lines() {
        if is_fence(line) {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let hashes = line.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
            let title = line[hashes..]
                .trim()
                .trim_end_matches('#')
                .trim()
                .to_string();
            let base = slug(&title);
            let n = seen.entry(base.clone()).or_insert(0);
            let anchor = if *n == 0 {
                base.clone()
            } else {
                format!("{base}-{n}")
            };
            *n += 1;
            out.push((hashes, title, anchor));
        }
    }
    out
}

/// The book's prose: fenced blocks and inline code spans blanked.
fn prose(book: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in book.lines() {
        if is_fence(line) {
            in_fence = !in_fence;
            out.push('\n');
            continue;
        }
        if !in_fence {
            let mut ticks = false;
            for c in line.chars() {
                if c == '`' {
                    ticks = !ticks;
                } else if !ticks {
                    out.push(c);
                }
            }
        }
        out.push('\n');
    }
    out
}

/// Every link target `](…)` in the prose.
fn link_targets(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("](") {
        rest = &rest[at + 2..];
        let end = rest.find([')', ' ', '\n']).unwrap_or(rest.len());
        out.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    out
}

/// The first `# ` title of a document outside its code fences.
fn title_of(rel: &str) -> String {
    let text = read(rel);
    headings(&text)
        .into_iter()
        .find(|(level, _, _)| *level == 1)
        .map(|(_, title, _)| title)
        .unwrap_or_else(|| panic!("{rel} has no `# ` title"))
}

/// Every document the book must hold: each guide, each project document, each member README.
fn required_documents() -> Vec<String> {
    let root = repo_root();
    let mut docs = Vec::new();
    for dir in ["docs", "docs/project"] {
        let mut here: Vec<String> = std::fs::read_dir(root.join(dir))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".md") && !(dir == "docs" && n == "README.md"))
            .map(|n| format!("{dir}/{n}"))
            .collect();
        here.sort();
        docs.extend(here);
    }
    let manifest: toml::Value = read("Cargo.toml").parse().unwrap();
    for member in manifest["workspace"]["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap())
    {
        let dirs: Vec<String> = match member.strip_suffix("/*") {
            Some(parent) => {
                let mut d: Vec<String> = std::fs::read_dir(root.join(parent))
                    .unwrap()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().join("Cargo.toml").is_file())
                    .map(|e| format!("{parent}/{}", e.file_name().to_string_lossy()))
                    .collect();
                d.sort();
                d
            }
            None => vec![member.to_string()],
        };
        docs.extend(
            dirs.into_iter()
                .map(|d| format!("{d}/README.md"))
                .filter(|r| root.join(r).is_file()),
        );
    }
    docs
}

#[test]
fn the_handbook_holds_every_guide_project_document_and_crate_readme() {
    let book = generate(&[]);
    let titles: BTreeSet<String> = headings(&book)
        .into_iter()
        .map(|(_, title, _)| title)
        .collect();
    let required = required_documents();
    assert!(
        required.len() >= 30,
        "expected the guides, project documents and crate READMEs, found {}",
        required.len()
    );
    let missing: Vec<String> = required
        .iter()
        .filter(|rel| !titles.contains(&title_of(rel)))
        .map(|rel| format!("{rel} (`{}`)", title_of(rel)))
        .collect();
    assert!(
        missing.is_empty(),
        "the handbook leaves out {} document(s):\n  {}",
        missing.len(),
        missing.join("\n  ")
    );
}

#[test]
fn every_fence_the_handbook_opens_it_closes() {
    let book = generate(&[]);
    let mut open: Option<usize> = None;
    for (i, line) in book.lines().enumerate() {
        if is_fence(line) {
            open = if open.is_some() { None } else { Some(i + 1) };
        }
    }
    assert!(
        open.is_none(),
        "the handbook opens a code fence at line {} and never closes it",
        open.unwrap()
    );
}

#[test]
fn every_anchor_the_handbook_links_is_a_heading_it_has() {
    let book = generate(&[]);
    let anchors: BTreeSet<String> = headings(&book).into_iter().map(|(_, _, a)| a).collect();
    let text = prose(&book);
    let links: Vec<String> = link_targets(&text)
        .into_iter()
        .filter_map(|t| t.strip_prefix('#').map(str::to_string))
        .collect();
    assert!(
        links.len() >= 100,
        "the contents and cross-references should be in-book links, found {}",
        links.len()
    );
    let dangling: BTreeSet<&String> = links.iter().filter(|a| !anchors.contains(*a)).collect();
    assert!(
        dangling.is_empty(),
        "the handbook links to anchors it does not have: {dangling:?}"
    );
}

#[test]
fn no_relative_link_survives_into_the_handbook() {
    let book = generate(&[]);
    let relative: BTreeSet<String> = link_targets(&prose(&book))
        .into_iter()
        .filter(|t| !t.starts_with('#') && !t.contains("://") && !t.starts_with("mailto:"))
        .collect();
    assert!(relative.is_empty(), "relative links only resolve beside the source file, not in the one-file book: {relative:?}");
}

#[test]
fn the_handbook_names_the_workspace_version_and_nothing_machine_local() {
    let book = generate(&[]);
    let manifest: toml::Value = read("Cargo.toml").parse().unwrap();
    let version = manifest["workspace"]["package"]["version"]
        .as_str()
        .unwrap();
    let head: String = book.lines().take(5).collect::<Vec<_>>().join("\n");
    assert!(
        head.starts_with("# The Logicaffeine Handbook"),
        "the book opens with its title:\n{head}"
    );
    assert!(
        head.contains(&format!("LOGOS {version}")),
        "the book's first lines name LOGOS {version}:\n{head}"
    );
    let root = repo_root().to_string_lossy().into_owned();
    for bad in [root.as_str(), "LOGICAFFEINE 1.0", "/Users/", "/home/"] {
        assert!(!book.contains(bad), "the handbook carries `{bad}`");
    }
}

/// The paths the assembler's SOURCE list names.
fn source_list() -> Vec<String> {
    let src = read(ASSEMBLER);
    let start = src
        .find("SOURCE = [")
        .expect("the assembler declares SOURCE");
    let block = &src[start..start + src[start..].find(']').unwrap()];
    block
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

#[test]
fn the_source_appendix_holds_every_file_it_names_verbatim() {
    let book = generate(&["--with-source"]);
    let list = source_list();
    assert!(
        list.len() >= 8,
        "the appendix should carry the pipeline's core files, SOURCE names {}",
        list.len()
    );
    for rel in &list {
        assert!(
            book.contains(&format!("## `{rel}`")),
            "the appendix has no section for {rel}"
        );
        let body = read(rel);
        assert!(
            book.contains(body.trim_end_matches('\n')),
            "the appendix does not carry {rel} verbatim"
        );
    }
    let plain = generate(&[]);
    assert!(
        !plain.contains("# Appendix: the core source"),
        "the source appendix is opt-in"
    );
}

/// A scratch tree holding what the assembler reads: the root documents, the docs, and each member's
/// manifest and README.
fn copy_inputs(to: &Path) {
    let root = repo_root();
    let copy = |rel: &str| {
        let from = root.join(rel);
        let dest = to.join(rel);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(&from, &dest).unwrap_or_else(|e| panic!("copy {rel}: {e}"));
    };
    for rel in ["Cargo.toml", "README.md", "LOGOS_QUICKGUIDE.md"] {
        copy(rel);
    }
    for dir in ["docs", "docs/project"] {
        for entry in std::fs::read_dir(root.join(dir))
            .unwrap()
            .filter_map(|e| e.ok())
        {
            if entry.path().is_file() {
                copy(&format!("{dir}/{}", entry.file_name().to_string_lossy()));
            }
        }
    }
    for readme in required_documents()
        .into_iter()
        .filter(|r| r.ends_with("/README.md"))
    {
        copy(&readme);
        copy(&readme.replace("README.md", "Cargo.toml"));
    }
}

#[test]
fn a_missing_input_stops_the_generator_by_name() {
    // Inputs are read before any link is resolved, so the scratch copy needs only the inputs, and the
    // refusal is about the one taken away.
    let dir = tempfile::tempdir().unwrap();
    copy_inputs(dir.path());
    std::fs::remove_file(dir.path().join("docs/concurrency.md")).unwrap();
    let (ok, err) = generate_from(dir.path(), &[]);
    assert!(
        !ok,
        "the generator wrote a book without docs/concurrency.md"
    );
    assert!(
        err.contains("a declared input does not exist: docs/concurrency.md"),
        "the refusal names the missing input:\n{err}"
    );
}

#[test]
fn the_entry_point_fails_fast_and_quotes_every_heredoc() {
    let entry = read(ENTRY);
    assert!(
        entry.contains("set -euo pipefail"),
        "{ENTRY} must stop at the first failure"
    );
    for (i, line) in entry.lines().enumerate() {
        if let Some(at) = line.find("<<") {
            let rest = line[at + 2..].trim_start_matches('-').trim_start();
            assert!(
                rest.starts_with('\'') || rest.starts_with('"'),
                "{ENTRY}:{} opens an unquoted heredoc; its backticks would run as commands",
                i + 1
            );
        }
    }
    for dead in [
        "logos_core/",
        "\"src/lexer.rs\"",
        "\"src/parser/",
        "add_file",
    ] {
        assert!(
            !entry.contains(dead) && !read(ASSEMBLER).contains(dead),
            "the generator names `{dead}`, a retired shape"
        );
    }
}

#[test]
fn github_anchors_follow_githubs_rules() {
    assert_eq!(slug("Proof & verification"), "proof--verification");
    assert_eq!(slug("What's inside"), "whats-inside");
    assert_eq!(slug("The `largo` CLI"), "the-largo-cli");
    assert_eq!(slug("`logicaffeine_base`"), "logicaffeine_base");
    assert_eq!(
        slug("Imperative mode — the LOGOS language"),
        "imperative-mode--the-logos-language"
    );
    assert_eq!(slug("[Frontend](frontend.md) guide"), "frontend-guide");
    let book = "# A\n## A\n```\n# not a heading\n```\n## B\n";
    let anchors: Vec<String> = headings(book).into_iter().map(|(_, _, a)| a).collect();
    assert_eq!(anchors, vec!["a", "a-1", "b"]);
}

// ─────────────────────────────────────────────────────────────────────────────
// A document that names a test must name one that exists.
// ─────────────────────────────────────────────────────────────────────────────
//
// "The tests that pin this are X" is a promise to the reader: do not take my word
// for it, go and read this. A promise pointing at a file someone renamed away is
// worse than no promise, because it is confidently wrong and costs the reader the
// search before they discover it.
//
// The `_lock` suffix is this repo's own naming convention for a pinning test, so a
// backticked token ending in it is unambiguously a citation rather than prose. That
// makes the check precise: no guesswork about which identifiers are test names, and
// no false positives on the many other things documents quote in backticks.
//
// Scoped to every guide plus the root README, not one file, because the rot is not
// specific to any of them.

/// Every `*_lock` a document cites, paired with the document citing it.
fn cited_locks(root: &std::path::Path) -> Vec<(String, String)> {
    let mut docs = vec![root.join("README.md")];
    if let Ok(entries) = std::fs::read_dir(root.join("docs")) {
        let mut guides: Vec<_> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
            .collect();
        guides.sort();
        docs.extend(guides);
    }

    let mut out = Vec::new();
    for doc in docs {
        let Ok(text) = std::fs::read_to_string(&doc) else {
            continue;
        };
        let name = doc
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let mut rest = text.as_str();
        while let Some(open) = rest.find('`') {
            rest = &rest[open + 1..];
            let Some(close) = rest.find('`') else { break };
            let token = &rest[..close];
            rest = &rest[close + 1..];
            let looks_like_a_test = token.ends_with("_lock")
                && token.len() > 5
                && token
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if looks_like_a_test {
                out.push((name.clone(), token.to_string()));
            }
        }
    }
    out
}

fn test_file_exists(root: &std::path::Path, name: &str) -> bool {
    for top in ["crates", "apps"] {
        let Ok(entries) = std::fs::read_dir(root.join(top)) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            if entry
                .path()
                .join("tests")
                .join(format!("{name}.rs"))
                .is_file()
            {
                return true;
            }
        }
    }
    false
}

#[test]
fn every_lock_the_documentation_cites_exists() {
    let root = repo_root();
    let cited = cited_locks(&root);
    assert!(
        !cited.is_empty(),
        "the documentation should still point readers at the tests that pin it"
    );
    let missing: Vec<String> = cited
        .iter()
        .filter(|(_, name)| !test_file_exists(&root, name))
        .map(|(doc, name)| format!("{doc} cites {name}"))
        .collect();
    assert!(
        missing.is_empty(),
        "{} documentation citation(s) point at a test that does not exist:\n  {}\n\
         Rename the citation to match the tree, or delete the claim — a stale pointer \
         is worse than no pointer.",
        missing.len(),
        missing.join("\n  ")
    );
}
