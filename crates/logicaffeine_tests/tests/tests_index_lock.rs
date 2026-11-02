//! `tests/INDEX.md` maps the suites of this directory: a row per family of suites that share a name prefix.
//!
//! A suite's family is its name up to the first `_`, less a trailing number (`phase101a_…` is a `phase` suite,
//! `qwen38_…` a `qwen` one) when at least three letters remain (`e2e` stays `e2e`). Every family of eight or more
//! suites has a row; every row names a family that still has a suite; and each row's glob lists exactly its family,
//! so `ls` on it shows what the row describes.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test tests_index_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const INDEX: &str = "crates/logicaffeine_tests/tests/INDEX.md";
const FAMILY_SIZE: usize = 8;

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// The stem of every suite: each `.rs` file directly in `tests/`.
fn suites() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(tests_dir())
        .unwrap()
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_suffix(".rs")
                .map(str::to_string)
        })
        .collect();
    out.sort();
    out
}

/// A suite's family: its name up to the first `_`, less a trailing number and one optional letter when at least
/// three letters remain.
fn family(stem: &str) -> &str {
    let token = stem.split('_').next().unwrap_or(stem);
    let b = token.as_bytes();
    let mut end = b.len();
    if end >= 2 && b[end - 1].is_ascii_lowercase() && b[end - 2].is_ascii_digit() {
        end -= 1;
    }
    let before_digits = end;
    while end > 0 && b[end - 1].is_ascii_digit() {
        end -= 1;
    }
    let head = &token[..end];
    if end < before_digits && head.len() >= 3 && head.bytes().all(|c| c.is_ascii_lowercase()) {
        head
    } else {
        token
    }
}

/// `*` matches any run of characters.
fn glob(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == name,
        Some((head, rest)) => {
            name.starts_with(head)
                && (0..=name.len() - head.len()).any(|i| glob(rest, &name[head.len() + i..]))
        }
    }
}

/// The glob of each row of the index's family table, in order.
fn rows(index: &str) -> Vec<String> {
    index
        .lines()
        .filter_map(|l| l.strip_prefix("| `"))
        .filter_map(|l| l.split_once('`').map(|(g, _)| g.to_string()))
        .collect()
}

/// What is wrong with an index whose rows are `rows`, for a directory holding `stems`.
fn problems(rows: &[String], stems: &[String]) -> Vec<String> {
    let mut families: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for s in stems {
        families.entry(family(s)).or_default().push(s);
    }
    let mut out = Vec::new();
    let mut named = BTreeSet::new();
    for row in rows {
        let key = row.trim_end_matches('*').trim_end_matches('_');
        named.insert(key.to_string());
        let Some(members) = families.get(key) else {
            out.push(format!(
                "`{row}` names a family no suite belongs to — delete the row"
            ));
            continue;
        };
        let listed: Vec<&String> = stems.iter().filter(|s| glob(row, s)).collect();
        let strangers: Vec<&&String> = listed.iter().filter(|s| family(s) != key).collect();
        let missed: Vec<&&str> = members.iter().filter(|m| !glob(row, m)).collect();
        if !strangers.is_empty() || !missed.is_empty() {
            out.push(format!("`{row}` does not list exactly the `{key}` family: also lists {strangers:?}, misses {missed:?}"));
        }
    }
    for (key, members) in &families {
        if members.len() >= FAMILY_SIZE && !named.contains(*key) {
            out.push(format!(
                "the `{key}` family has {} suites and no row",
                members.len()
            ));
        }
    }
    out
}

#[test]
fn every_family_of_suites_has_a_true_row() {
    let index = std::fs::read_to_string(tests_dir().join("INDEX.md"))
        .unwrap_or_else(|e| panic!("read {INDEX}: {e}"));
    let rows = rows(&index);
    assert!(
        rows.len() >= 20,
        "{INDEX} has {} family rows — the table is missing or unreadable",
        rows.len()
    );
    let wrong = problems(&rows, &suites());
    assert!(
        wrong.is_empty(),
        "\n{INDEX} does not map the suites:\n  {}\n",
        wrong.join("\n  ")
    );
}

#[test]
fn a_family_is_its_prefix_less_a_trailing_number() {
    assert_eq!(family("phase101a_poly_inductive"), "phase");
    assert_eq!(family("phase1_garden_path"), "phase");
    assert_eq!(family("phase_zero_based"), "phase");
    assert_eq!(family("qwen38_bundle_contract_lock"), "qwen");
    assert_eq!(family("mlkem768_kpke"), "mlkem");
    assert_eq!(family("e2e_zones"), "e2e");
    assert_eq!(family("c_binding_scope_lock"), "c");
    assert_eq!(family("x86_backend"), "x86");
    assert_eq!(family("lean"), "lean");
}

#[test]
fn the_check_sees_each_way_an_index_goes_wrong() {
    let stems: Vec<String> = [
        "c_a", "c_b", "cuda_a", "cuda_b", "phase1_a", "phase_b", "gone_a",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let rows = |r: &[&str]| r.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(problems(&rows(&["c_*", "cuda_*", "phase*"]), &stems).is_empty());
    let too_wide = problems(&rows(&["c*", "cuda_*", "phase*"]), &stems);
    assert!(
        too_wide.iter().any(|p| p.contains("also lists")),
        "{too_wide:?}"
    );
    let too_narrow = problems(&rows(&["c_*", "cuda_*", "phase_*"]), &stems);
    assert!(
        too_narrow.iter().any(|p| p.contains("misses")),
        "{too_narrow:?}"
    );
    let stale = problems(&rows(&["c_*", "cuda_*", "phase*", "ghost_*"]), &stems);
    assert!(
        stale
            .iter()
            .any(|p| p.contains("`ghost_*` names a family no suite belongs to")),
        "{stale:?}"
    );
    let big: Vec<String> = (0..FAMILY_SIZE).map(|i| format!("wave_{i}")).collect();
    let unmapped = problems(&rows(&[]), &big);
    assert!(
        unmapped
            .iter()
            .any(|p| p.contains("the `wave` family has 8 suites and no row")),
        "{unmapped:?}"
    );
}
