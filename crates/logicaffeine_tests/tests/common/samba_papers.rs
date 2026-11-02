//! The Samba book's pinned sources (`benchmarks/samba_papers.tsv`): each row's columns, each source's searchable text,
//! and whether a number appears in it as written. `SAMBA_PAPERS` points a mutation run at a mutated table.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The file `var` names, or `default` under the repository.
pub fn read(var: &str, default: &str) -> String {
    let path = std::env::var(var).map(PathBuf::from).unwrap_or_else(|_| repo().join(default));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// One pin: a row of the table, by column name.
pub type Pin = BTreeMap<String, String>;

pub fn pins() -> Vec<Pin> {
    let table = read("SAMBA_PAPERS", "benchmarks/samba_papers.tsv");
    let mut lines = table.lines();
    let head: Vec<&str> = lines.next().expect("a header").split('\t').collect();
    assert_eq!(
        head,
        ["key", "title", "authors", "venue", "year", "locator", "captured", "capture", "sha256", "text"],
        "the table's columns"
    );
    lines
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let cells: Vec<&str> = l.split('\t').collect();
            assert_eq!(cells.len(), head.len(), "a row of {} cells: {l}", cells.len());
            head.iter().zip(cells).map(|(h, c)| (h.to_string(), c.to_string())).collect()
        })
        .collect()
}

/// The searchable text of each source, thousands separators removed.
pub fn source_texts(pins: &[Pin]) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for p in pins {
        let path = repo().join(&p["text"]);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("[{}] {}: {e}", p["key"], path.display()));
        let text = String::from_utf8_lossy(&bytes);
        let e = out.entry(p["key"].clone()).or_default();
        e.push_str(&ungroup(&text));
        e.push('\n');
    }
    out
}

/// `text` with every comma between a digit and three digits removed: `1,140` reads `1140`.
pub fn ungroup(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for i in 0..c.len() {
        let grouping = c[i] == ','
            && i > 0
            && c[i - 1].is_ascii_digit()
            && i + 3 < c.len()
            && c[i + 1..i + 4].iter().all(char::is_ascii_digit)
            && c.get(i + 4).is_none_or(|n| !n.is_ascii_digit());
        if !grouping {
            out.push(c[i]);
        }
    }
    out
}

/// Whether `number` appears in `text` as a whole number: no digit before it, and after it neither a digit nor a
/// decimal point followed by one.
pub fn appears(text: &str, number: &str) -> bool {
    let t = text.as_bytes();
    text.match_indices(number).any(|(i, _)| {
        let before = i.checked_sub(1).map(|j| t[j]);
        let after = t.get(i + number.len()).copied();
        let after2 = t.get(i + number.len() + 1).copied();
        !before.is_some_and(|b| b.is_ascii_digit() || b == b'.')
            && !after.is_some_and(|a| a.is_ascii_digit())
            && !(after == Some(b'.') && after2.is_some_and(|a| a.is_ascii_digit()))
    })
}
