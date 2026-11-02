//! Every LOGOS program the tree holds, discovered at run time from the tracked sources, never from a
//! committed snapshot:
//!
//! - every `.lg` file;
//! - every `logos` fence in a Markdown file;
//! - every Rust string literal that spells a program (`## Main` or `## To`), read as rustc reads it;
//! - every source field of a jsonl corpus row.
//!
//! The roots are the product's tracked directories (`crates`, `apps`, `benchmarks`, `docs`, `examples`)
//! and the top-level guides; build output and vendored trees are skipped. Programs are deduplicated by
//! content and identified by a hash of it, so a program keeps its id wherever it moves. Needs
//! `rust_literals` declared beside it.
#![allow(dead_code)]

use super::rust_literals::{literals, try_read_stable};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One program: its content hash, the first file it was found in, and its source.
#[derive(Clone, Debug)]
pub struct Program {
    pub id: String,
    pub origin: String,
    pub source: String,
}

pub fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("the workspace root")
}

const ROOTS: &[&str] = &["crates", "apps", "benchmarks", "docs", "examples"];
const TOP_FILES: &[&str] = &["README.md", "LOGOS_QUICKGUIDE.md"];
const SKIP_DIRS: &[&str] = &["target", "node_modules", "dist", "pkg", ".git"];

/// A stable 64-bit content hash (FNV-1a), printed as 16 hex digits.
pub fn content_id(source: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in source.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if p.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with("target") {
                walk(&p, out);
            }
        } else {
            out.push(p);
        }
    }
}

/// The body of each ```` ```logos ```` fence (any `logos…` info string) in `text`.
fn logos_fences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        if line.trim_start().starts_with("```logos") {
            let mut body = String::new();
            for l in lines.by_ref() {
                if l.trim_start().starts_with("```") {
                    break;
                }
                body.push_str(l);
                body.push('\n');
            }
            out.push(body);
        }
    }
    out
}

fn spells_a_program(s: &str) -> bool {
    s.contains("## Main") || s.contains("## To")
}

/// Every program the tree holds, sorted by id.
pub fn corpus() -> Vec<Program> {
    let root = workspace();
    let mut files = Vec::new();
    for r in ROOTS {
        walk(&root.join(r), &mut files);
    }
    for f in TOP_FILES {
        files.push(root.join(f));
    }
    let mut found: BTreeMap<String, Program> = BTreeMap::new();
    let mut add = |source: String, origin: &Path| {
        if !spells_a_program(&source) {
            return;
        }
        let id = content_id(&source);
        found.entry(id.clone()).or_insert_with(|| Program {
            id,
            origin: origin.strip_prefix(&root).unwrap_or(origin).display().to_string(),
            source,
        });
    };
    for path in &files {
        let ext = path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        if !matches!(ext.as_str(), "lg" | "md" | "rs" | "jsonl") {
            continue;
        }
        let Some(text) = try_read_stable(path) else { continue };
        match ext.as_str() {
            "lg" => add(text, path),
            "md" => {
                for body in logos_fences(&text) {
                    add(body, path);
                }
            }
            "rs" => {
                for lit in literals(&text) {
                    add(lit.value, path);
                }
            }
            _ => {
                for line in text.lines() {
                    let Ok(serde_json::Value::Object(row)) = serde_json::from_str::<serde_json::Value>(line) else { continue };
                    for key in ["src", "source", "program", "code"] {
                        if let Some(serde_json::Value::String(s)) = row.get(key) {
                            add(s.clone(), path);
                        }
                    }
                }
            }
        }
    }
    found.into_values().collect()
}

/// `n` programs of `all`, chosen by `seed` (SplitMix64 over the sorted ids), in id order.
pub fn sample(all: &[Program], n: usize, seed: u64) -> Vec<Program> {
    let mut s = seed;
    let mut keyed: Vec<(u64, &Program)> = all
        .iter()
        .map(|p| {
            s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = s;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            (z ^ (z >> 31), p)
        })
        .collect();
    keyed.sort_by_key(|(k, _)| *k);
    let mut picked: Vec<Program> = keyed.into_iter().take(n).map(|(_, p)| p.clone()).collect();
    picked.sort_by(|a, b| a.id.cmp(&b.id));
    picked
}
