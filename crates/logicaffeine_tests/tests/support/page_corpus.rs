//! The page corpus the tier locks trace — DISCOVERED, never listed: every page in the locks that
//! drive pages through tests (read as SOURCE — their string literals, with `format!` templates
//! resolved against the file's `const`s), every page file under `examples/*/pages/` read through its
//! own project closure, and every page in `tier_scenarios/` written for a behaviour the tiers are
//! known to treat differently. A `## Main` script is run as a test of its own; a page with no
//! scenario at all is traced as it renders.
//!
//! One reader for every lock that walks the corpus, so two locks can never disagree about which
//! pages exist or what a page's ledger id is. Needs `rust_literals` declared beside it.

use super::rust_literals::{literals, read_stable, resolve_format};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// A page source and the name it goes by in the ledger.
pub struct Page {
    pub id: String,
    pub source: String,
}

/// The lock files whose literals spell pages.
pub const LOCK_FILES: [&str; 2] = [
    "crates/logicaffeine_tests/tests/headless_runtime_lock.rs",
    "crates/logicaffeine_tests/tests/component_test_verbs_lock.rs",
];

/// The pages a lock file's literals spell. A `## Main` script becomes a `## Test` of its own.
pub fn pages_in_rust(file: &str) -> Vec<Page> {
    let text = read_stable(&workspace().join(file));
    let lits = literals(&text);
    let consts: BTreeMap<String, String> = lits
        .iter()
        .filter_map(|l| l.const_name.clone().map(|n| (n, l.value.clone())))
        .collect();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut out = Vec::new();
    for l in &lits {
        let value = if l.is_format {
            match resolve_format(&l.value, &consts) {
                Some(v) => v,
                None => continue,
            }
        } else {
            l.value.clone()
        };
        if !(value.contains("## Component") || value.contains("## Page")) {
            continue;
        }
        let has_test = value.contains("## Test");
        let main_mounts = value.contains("## Main") && value.contains("Mount ");
        if !has_test && !main_mounts {
            continue;
        }
        let source = value.replace("\n## Main\n", "\n## Test: main\n");
        let n = seen.entry(l.owner.clone()).or_insert(0);
        *n += 1;
        let short = Path::new(file)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let id = if *n == 1 {
            format!("{short}::{}", l.owner)
        } else {
            format!("{short}::{}#{n}", l.owner)
        };
        out.push(Page { id, source });
    }
    out
}

/// Every page file under `examples/*/pages/`, read through its own project closure
/// ([`logicaffeine_compile::page_host::assemble_stable`] — the same function `largo build
/// --static` compiles from), not a raw single-file read: a page that shares a layout or a type
/// across files is a normal project, not a corpus-discovery gap. One that carries no scenario of
/// its own is traced as it renders; a `## Main` script becomes a test of its own.
pub fn example_pages() -> Vec<Page> {
    let root = workspace();
    let mut out = Vec::new();
    let mut apps: Vec<PathBuf> = std::fs::read_dir(root.join("examples"))
        .unwrap()
        .flatten()
        .map(|e| e.path().join("pages"))
        .filter(|p| p.is_dir())
        .collect();
    apps.sort();
    for pages in apps {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&pages)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "lg"))
            .collect();
        files.sort();
        for f in files {
            let text = logicaffeine_compile::page_host::assemble_stable(&f)
                .unwrap_or_else(|e| panic!("could not stably assemble {}: {e}", f.display()));
            let mut source = text.replace("\n## Main\n", "\n## Test: main\n");
            if !source.contains("## Test") {
                source.push_str("\n## Test: the page renders\n    Expect that 1 is equal to 1.\n");
            }
            let id = f
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push(Page { id, source });
        }
    }
    out
}

/// The scenarios written for the tier locks: a page per behaviour the tiers are known to treat
/// differently, or that no other corpus page reaches (a trapping turn, a failure handler).
pub fn scenario_pages() -> Vec<Page> {
    let root = workspace();
    let dir = root.join("crates/logicaffeine_tests/tests/tier_scenarios");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "lg"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|f| Page {
            id: f
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/"),
            source: read_stable(&f),
        })
        .collect()
}

/// The whole corpus.
pub fn corpus() -> Vec<Page> {
    let mut pages: Vec<Page> = LOCK_FILES.iter().flat_map(|f| pages_in_rust(f)).collect();
    pages.extend(example_pages());
    pages.extend(scenario_pages());
    pages
}
