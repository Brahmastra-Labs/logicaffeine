//! Programs in the documentation: every fenced block of the root README, every guide and every crate's and
//! app's README, and the one rule that makes a headerless fragment a complete program. `doc_examples` holds the
//! documentation to what these programs print; `statement_tier_totality_lock` runs them on every backend.
#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// The documentation set under test: the root README, every guide, and the README of every crate
/// and app (its crates.io page and rustdoc root).
pub fn doc_files() -> Vec<PathBuf> {
    let root = repo_root();
    let mut files = vec![root.join("README.md")];
    let mut guides: Vec<PathBuf> = fs::read_dir(root.join("docs"))
        .expect("docs/ should exist")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
        .collect();
    guides.sort();
    files.extend(guides);
    let mut readmes: Vec<PathBuf> = ["crates", "apps"]
        .iter()
        .flat_map(|parent| {
            fs::read_dir(root.join(parent)).unwrap_or_else(|e| panic!("{parent}/: {e}"))
        })
        .filter_map(|e| e.ok())
        .map(|e| e.path().join("README.md"))
        .chain(std::iter::once(root.join("scripts/wiki_trace/README.md")))
        .filter(|p| p.is_file())
        .collect();
    readmes.sort();
    files.extend(readmes);
    files
}

pub struct Block {
    pub file: String,
    pub line: usize,
    pub lang: String,
    pub body: String,
    /// Whether only blank lines separate this fence from the one before it — what makes an
    /// ```output block the output OF the block above it.
    pub follows_previous: bool,
}

/// Split a markdown file into fenced code blocks (tracking the info-string).
pub fn fenced_blocks(path: &PathBuf) -> Vec<Block> {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    let name = path
        .strip_prefix(repo_root())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.file_name().unwrap().to_string_lossy().into_owned());
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut only_blank_since_close = false;
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
            out.push(Block {
                file: name.clone(),
                line: start,
                lang,
                body,
                follows_previous: only_blank_since_close,
            });
            only_blank_since_close = true;
        } else if !t.is_empty() {
            only_blank_since_close = false;
        }
        i += 1;
    }
    out
}

pub fn all_blocks() -> Vec<Block> {
    doc_files().iter().flat_map(fenced_blocks).collect()
}

pub fn has_block_header(body: &str) -> bool {
    body.lines().any(|l| l.trim_start().starts_with("## "))
}

pub fn indent4(body: &str) -> String {
    body.lines()
        .map(|l| {
            if l.trim().is_empty() {
                String::new()
            } else {
                format!("    {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Shared-CRDT context for the CRDT-mutation snippet (defines every field kind it
/// touches and the objects it names).
pub const CRDT_PRELUDE: &str = "## Definition\n\
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
pub const NET_PRELUDE: &str = "## Definition\n\
A Counter is Shared and has:\n\
\x20   a value, which is ConvergentCount.\n\
\n\
## Main\n\
\x20   Let counter be a new Counter.\n";

/// Make a doc `logos` block into a self-contained program. Complete programs are
/// returned unchanged; headerless fragments get the minimal context the snippet
/// assumes, so the exact lines from the docs are compiled.
pub fn make_program(body: &str) -> String {
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
