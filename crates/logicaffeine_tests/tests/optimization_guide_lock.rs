//! ★ LOCK — the guide's optimization table is the registry. ★
//!
//! `docs/execution-and-performance.md` lists every optimization keyword under the group the registry files it under
//! (the headings `largo opts` prints), and nothing else. An optimization the registry gains, loses or regroups goes
//! RED here until the guide says the same, so `## No <keyword>` never names a keyword a reader cannot find.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test optimization_guide_lock`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::optimization::REGISTRY;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const GUIDE: &str = "docs/execution-and-performance.md";
const HEADER: &str = "| Group | Keywords |";

fn guide_text() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(GUIDE);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

/// The guide's keyword table: each group row's keywords.
fn guide_groups(text: &str) -> BTreeMap<String, BTreeSet<String>> {
    assert_eq!(
        text.matches(HEADER).count(),
        1,
        "{GUIDE} has exactly one `{HEADER}` table"
    );
    let table = &text[text.find(HEADER).unwrap()..];
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in table.lines().skip(2).take_while(|l| l.starts_with('|')) {
        let cells: Vec<&str> = row.trim().trim_matches('|').split('|').map(str::trim).collect();
        assert_eq!(cells.len(), 2, "a keyword row has a group and its keywords: {row}");
        let keywords: BTreeSet<String> = cells[1]
            .split(',')
            .map(|k| {
                let k = k.trim();
                assert!(
                    k.len() > 2 && k.starts_with('`') && k.ends_with('`'),
                    "each keyword is code-spanned: {k:?} in {row}"
                );
                k.trim_matches('`').to_string()
            })
            .collect();
        assert!(
            groups.insert(cells[0].to_string(), keywords).is_none(),
            "the group {} has one row",
            cells[0]
        );
    }
    groups
}

fn registry_groups() -> BTreeMap<String, BTreeSet<String>> {
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for meta in REGISTRY {
        groups
            .entry(meta.group.to_string())
            .or_default()
            .insert(meta.keyword.to_string());
    }
    groups
}

#[test]
fn the_guide_files_every_optimization_under_its_registry_group() {
    let guide = guide_groups(&guide_text());
    let registry = registry_groups();
    let mut wrong = Vec::new();
    for group in guide.keys().chain(registry.keys()).collect::<BTreeSet<_>>() {
        let (said, filed) = (guide.get(group), registry.get(group));
        if said != filed {
            let empty = BTreeSet::new();
            let (said, filed) = (said.unwrap_or(&empty), filed.unwrap_or(&empty));
            wrong.push(format!(
                "{group}: missing {:?}, not in the registry {:?}",
                filed.difference(said).collect::<Vec<_>>(),
                said.difference(filed).collect::<Vec<_>>()
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{GUIDE}'s keyword table differs from the registry:\n{}",
        wrong.join("\n")
    );
}

/// The parser above reads what a reader reads: a row it cannot place fails, rather than the comparison passing over
/// a table it never saw.
#[test]
fn the_table_reader_refuses_what_it_cannot_read() {
    let good = format!("intro\n\n{HEADER}\n|---|---|\n| Bits | `bitintrinsics` |\n| Math | `simdreduce`, `tile` |\n\nafter\n");
    let groups = guide_groups(&good);
    assert_eq!(groups.len(), 2);
    assert_eq!(groups["Math"].len(), 2);
    for bad in [
        format!("{HEADER}\n|---|---|\n| Bits | bitintrinsics |\n"),
        format!("{HEADER}\n|---|---|\n| Bits | `a` |\n| Bits | `b` |\n"),
        format!("{HEADER}\n|---|---|\n| Bits | `a` | extra |\n"),
        "no table at all\n".to_string(),
    ] {
        assert!(
            std::panic::catch_unwind(|| guide_groups(&bad)).is_err(),
            "the reader must refuse:\n{bad}"
        );
    }
}
