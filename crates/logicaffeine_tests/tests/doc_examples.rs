//! Doc-example coverage harness — every code example in `NEW_README.md` and
//! `new_docs/*.md` is extracted and verified here, so the documentation set
//! cannot silently rot.
//!
//! - Every ```logos block must compile (`compile_to_rust` → `Ok`). Complete
//!   programs (with a `##` block header) are compiled as-is; headerless
//!   fragments are wrapped in a minimal, content-keyed context so the snippet
//!   shown in the docs is exercised exactly.
//! - Every English → First-Order-Logic pair (the `"sentence" → ∀x…` lines) must
//!   reproduce its documented output via `compile`.
//! - No ```logos block may contain a `...` placeholder — doc examples are real,
//!   runnable code, not schematics.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'test(doc_)'`

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::PathBuf;

use logicaffeine_compile::compile::compile_to_rust;
use logicaffeine_language::compile;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// The documentation set under test: the staged root README plus every guide.
fn doc_files() -> Vec<PathBuf> {
    let root = repo_root();
    let mut files = vec![root.join("NEW_README.md")];
    let mut guides: Vec<PathBuf> = fs::read_dir(root.join("new_docs"))
        .expect("new_docs/ should exist")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
        .collect();
    guides.sort();
    files.extend(guides);
    files
}

struct Block {
    file: String,
    line: usize,
    lang: String,
    body: String,
}

/// Split a markdown file into fenced code blocks (tracking the info-string).
fn fenced_blocks(path: &PathBuf) -> Vec<Block> {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim_start();
        if let Some(info) = t.strip_prefix("```") {
            let lang = info.trim().to_string();
            let start = i + 1;
            let mut body = String::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                body.push_str(lines[i]);
                body.push('\n');
                i += 1;
            }
            out.push(Block { file: name.clone(), line: start, lang, body });
        }
        i += 1;
    }
    out
}

fn all_blocks() -> Vec<Block> {
    doc_files().iter().flat_map(fenced_blocks).collect()
}

fn has_block_header(body: &str) -> bool {
    body.lines().any(|l| l.trim_start().starts_with("## "))
}

fn indent4(body: &str) -> String {
    body.lines()
        .map(|l| if l.trim().is_empty() { String::new() } else { format!("    {l}") })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Shared-CRDT context for the CRDT-mutation snippet (defines every field kind it
/// touches and the objects it names).
const CRDT_PRELUDE: &str = "## Definition\n\
A Box is Shared and has:\n\
\x20   a points, which is ConvergentCount.\n\
\x20   a score, which is a Tally.\n\
\x20   a lines, which is a SharedSequence of Text.\n\
\x20   a title, which is a Divergent Text.\n\
\n\
## Main\n\
\x20   Let local be a new Box.\n\
\x20   Let game be a new Box.\n\
\x20   Let doc be a new Box.\n\
\x20   Let page be a new Box.\n\
\x20   Let remote be a new Box.\n";

/// Networking context for the distributed-verbs snippet.
const NET_PRELUDE: &str = "## Definition\n\
A Counter is Shared and has:\n\
\x20   a value, which is ConvergentCount.\n\
\n\
## Main\n\
\x20   Let counter be a new Counter.\n";

/// Make a doc `logos` block into a self-contained program. Complete programs are
/// returned unchanged; headerless fragments get the minimal context the snippet
/// assumes, so the exact lines from the docs are compiled.
fn make_program(body: &str) -> String {
    if has_block_header(body) {
        return body.to_string();
    }
    let crdt = ["Increase ", "Decrease ", "Append ", "Resolve ", "Merge "]
        .iter()
        .any(|k| body.contains(k));
    let net = ["Listen on", "Connect to", "Sync ", "Mount "]
        .iter()
        .any(|k| body.contains(k));
    let pipe = body.contains("Await the first of")
        || body.contains("Receive ")
        || (body.contains("Send ") && body.contains(" into "));

    if crdt {
        format!("{CRDT_PRELUDE}{}", indent4(body))
    } else if net {
        format!("{NET_PRELUDE}{}", indent4(body))
    } else if pipe {
        format!("## Main\n    Let jobs be a Pipe of Int.\n{}", indent4(body))
    } else {
        // Generic statement fragment. If it messages an agent without spawning
        // one, spawn it so the snippet is self-contained.
        let mut prelude = String::from("## Main\n");
        if body.contains("Send ") && body.contains(" to \"") && !body.contains("Spawn ") {
            prelude.push_str("    Spawn a Worker called \"w1\".\n");
        }
        format!("{prelude}{}", indent4(body))
    }
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extract `"english" → FOL` pairs from an untagged block (both the one-line form
/// and the two-line form used in the guides).
fn fol_pairs(body: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = body.lines().collect();
    let mut pairs = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        // One-line: "english"  →  FOL
        if line.starts_with('"') {
            if let Some(close) = line[1..].find('"') {
                let english = &line[1..1 + close];
                let rest = line[1 + close + 1..].trim_start();
                if let Some(fol) = rest.strip_prefix('→') {
                    pairs.push((english.to_string(), fol.trim().to_string()));
                    i += 1;
                    continue;
                }
                // Two-line: "english" then a following → FOL line.
                let mut j = i + 1;
                while j < lines.len() && lines[j].trim().is_empty() {
                    j += 1;
                }
                if j < lines.len() {
                    if let Some(fol) = lines[j].trim().strip_prefix('→') {
                        pairs.push((english.to_string(), fol.trim().to_string()));
                        i = j + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    pairs
}

#[test]
fn doc_no_placeholder_examples() {
    let bad: Vec<String> = all_blocks()
        .iter()
        .filter(|b| b.lang == "logos" && b.body.contains("..."))
        .map(|b| format!("{}:{}", b.file, b.line))
        .collect();
    assert!(
        bad.is_empty(),
        "logos doc examples must be real, not `...` schematics: {bad:?}"
    );
}

#[test]
fn doc_logos_examples_compile() {
    let mut failures = Vec::new();
    let mut tested = 0;
    for b in all_blocks().iter().filter(|b| b.lang == "logos") {
        let program = make_program(&b.body);
        match compile_to_rust(&program) {
            Ok(_) => tested += 1,
            Err(e) => failures.push(format!("{}:{} — {e:?}", b.file, b.line)),
        }
    }
    assert!(tested > 0, "expected to find logos doc examples");
    assert!(
        failures.is_empty(),
        "{} logos doc example(s) failed to compile:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn doc_fol_pairs_match() {
    let mut failures = Vec::new();
    let mut tested = 0;
    for b in all_blocks().iter().filter(|b| b.lang.is_empty()) {
        for (english, expected) in fol_pairs(&b.body) {
            match compile(&english) {
                Ok(got) if norm(&got) == norm(&expected) => tested += 1,
                Ok(got) => failures.push(format!(
                    "{}:{} {english:?}\n   want: {expected}\n   got:  {got}",
                    b.file, b.line
                )),
                Err(e) => failures.push(format!("{}:{} {english:?} — {e:?}", b.file, b.line)),
            }
        }
    }
    assert!(tested > 0, "expected to find English→FOL doc pairs");
    assert!(
        failures.is_empty(),
        "{} doc FOL pair(s) drifted from the compiler:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
