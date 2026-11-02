//! White-box unit tests live in `crates/logicaffeine_tests/unit/`, and a crate's `src/` holds only
//! the one-line stub that places them in its module tree.
//!
//! A published crate ships its `src/`. Test bodies there are dead weight in every tarball (and the
//! kernel's once embedded 10 MB of fixtures under `cfg(test)`), yet a unit test needs its module's
//! private items, so it must stay a child module of the code it tests. The resolution: the body
//! lives at `crates/logicaffeine_tests/unit/<crate dir>/<module path>.rs`, mirroring the module tree
//! (`src/reduction.rs`'s `mod tests` is `unit/logicaffeine_kernel/reduction/tests.rs`; a `src/main.rs`
//! root adds `main/`, a `src/bin/<name>.rs` root adds `bin/<name>/`), and the crate keeps only
//!
//! ```text
//! #[cfg(test)]
//! #[path = "<relative>/logicaffeine_tests/unit/<crate dir>/<module path>.rs"]
//! mod tests;
//! ```
//!
//! so the tests keep their names and their private access, still run under `-p <crate>`, and never
//! compile for a consumer (a `#[cfg(test)]` module file missing from a tarball is never read). This
//! lock holds four rules over every governed package (all but the test crate and the crates
//! `PUBLIC_MANIFEST.toml` holds back):
//!
//! 1. no `#[test]` function anywhere under `src/`;
//! 2. every test-only `mod` in `src/` is a stub whose `#[path]` resolves to exactly its mirrored
//!    `unit/` file;
//! 3. every `.rs` file under `unit/` is reached from some crate's module graph — an orphan is a test
//!    that silently stopped compiling and running;
//! 4. the remaining test-only attributes in `src/` (instrumentation that production code calls, such
//!    as the kernel's work counters, and helpers not yet moved) are frozen per file in
//!    `goldens/src_test_items.txt`: a count may only fall and a new file may not appear.
//!
//! Pure parsing over the shared module resolver (`common/module_graph.rs`), no cargo.

#[path = "common/module_graph.rs"]
mod module_graph;
#[path = "common/public_manifest.rs"]
mod public_manifest;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use module_graph::normalize;
use public_manifest::{held_back, packages, repo_root, Classifier};

const HOME: &str = "crates/logicaffeine_tests";
const GOLDEN: &str = "crates/logicaffeine_tests/tests/goldens/src_test_items.txt";

fn governed() -> Vec<(String, PathBuf)> {
    let held = held_back(&Classifier::load());
    let root = repo_root();
    let all: Vec<(String, PathBuf)> = packages()
        .into_keys()
        .filter(|dir| dir != HOME && !held.contains_key(dir))
        .map(|dir| (dir.clone(), root.join(&dir)))
        .filter(|(_, p)| p.join("src").is_dir())
        .collect();
    assert!(
        all.len() >= 10,
        "found only {} governed packages — the discovery walk broke",
        all.len()
    );
    all
}

fn unit_root() -> PathBuf {
    repo_root().join(HOME).join("unit")
}

fn rel(p: &Path) -> String {
    let root = normalize(&repo_root());
    normalize(p)
        .strip_prefix(&root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Where a unit test module's body lives.
fn mirrored(crate_dir: &Path, root: &Path, module: &[String]) -> PathBuf {
    let mut p = unit_root().join(crate_dir.file_name().unwrap());
    let root_rel = root
        .strip_prefix(crate_dir)
        .unwrap_or(root)
        .to_string_lossy()
        .replace('\\', "/");
    if root_rel == "src/main.rs" {
        p.push("main");
    } else if let Some(bin) = root_rel.strip_prefix("src/bin/") {
        p.push("bin");
        p.push(bin.trim_end_matches("/main.rs").trim_end_matches(".rs"));
    }
    let (last, dirs) = module.split_last().expect("a module has a name");
    for d in dirs {
        p.push(d);
    }
    p.push(format!("{last}.rs"));
    normalize(&p)
}

fn rs_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn no_test_function_lives_in_src() {
    let mut found = Vec::new();
    for (dir, path) in governed() {
        for f in rs_files(&path.join("src")) {
            for t in module_graph::test_fns_in_file(&f) {
                found.push(format!("{}:{} `{}` ({dir})", rel(&f), t.line, t.what));
            }
        }
    }
    assert!(
        found.is_empty(),
        "{} test function(s) in a crate's src/:\n  {}\n\nMove the test module's body to \
         {HOME}/unit/<crate dir>/<module path>.rs and leave `#[cfg(test)] #[path = \"…\"] mod <name>;` \
         in its place (cargo run -p logicaffeine-tests --example test_home -- apply <crate dir> does it).",
        found.len(),
        found.join("\n  ")
    );
}

#[test]
fn every_test_module_is_a_stub_into_its_mirrored_unit_file() {
    let mut problems = Vec::new();
    let mut stubs = 0usize;
    for (dir, path) in governed() {
        let graph = module_graph::walk(&path);
        for tm in &graph.test_mods {
            if !tm.file.starts_with(path.join("src")) {
                continue;
            }
            let want = mirrored(&path, &tm.root, &tm.module);
            let at = format!("{}:{}", rel(&tm.file), tm.line);
            match (&tm.target, tm.inline) {
                (_, true) => problems.push(format!(
                    "{at}: `mod {}` has its body inline — move it to {} behind a #[path] stub",
                    tm.name,
                    rel(&want)
                )),
                (Some(target), false) if normalize(target) == want => stubs += 1,
                (Some(target), false) => problems.push(format!(
                    "{at}: `mod {}` resolves to {}, but its home is {} ({dir})",
                    tm.name,
                    rel(target),
                    rel(&want)
                )),
                (None, false) => problems.push(format!("{at}: `mod {}` resolves nowhere", tm.name)),
            }
        }
    }
    assert!(
        stubs >= 500,
        "found only {stubs} unit stubs — the walker broke, or the tests went missing"
    );
    assert!(
        problems.is_empty(),
        "{} test module(s) out of place:\n  {}",
        problems.len(),
        problems.join("\n  ")
    );
}

#[test]
fn every_unit_file_is_reached_by_a_crate() {
    let mut reached: BTreeSet<PathBuf> = BTreeSet::new();
    for (_, path) in governed() {
        for f in module_graph::walk(&path).files {
            reached.insert(normalize(&f.path));
        }
    }
    let orphans: Vec<String> = rs_files(&unit_root())
        .into_iter()
        .filter(|f| !reached.contains(&normalize(f)))
        .map(|f| rel(&f))
        .collect();
    assert!(
        orphans.is_empty(),
        "{} unit file(s) no crate compiles — tests that silently stopped running:\n  {}\n\n\
         Declare each from its module with a `#[cfg(test)] #[path = \"…\"] mod …;` stub, or, if the \
         module it tested is gone, port its tests to where the behaviour now lives.",
        orphans.len(),
        orphans.join("\n  ")
    );
}

/// Every file in a governed `src/` with test-only attributes outside its test modules, and how many.
fn test_item_counts() -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for (_, path) in governed() {
        for item in module_graph::walk(&path).test_items {
            if item.file.starts_with(path.join("src")) {
                *counts.entry(rel(&item.file)).or_default() += 1;
            }
        }
    }
    counts
}

fn golden() -> BTreeMap<String, usize> {
    let text = std::fs::read_to_string(repo_root().join(GOLDEN))
        .unwrap_or_else(|e| panic!("read {GOLDEN}: {e}"));
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let (path, n) = l
                .rsplit_once('\t')
                .unwrap_or_else(|| panic!("{GOLDEN}: `{l}` is not `<path>\\t<count>`"));
            (
                path.to_string(),
                n.parse()
                    .unwrap_or_else(|_| panic!("{GOLDEN}: `{l}` has no count")),
            )
        })
        .collect()
}

#[test]
fn test_only_items_left_in_src_only_ever_shrink() {
    let now = test_item_counts();
    let frozen = golden();
    let mut grew = Vec::new();
    for (file, n) in &now {
        match frozen.get(file) {
            None => grew.push(format!("{file}: {n} test-only item(s), not on the list")),
            Some(m) if n > m => grew.push(format!("{file}: {n} test-only item(s), frozen at {m}")),
            _ => {}
        }
    }
    let stale: Vec<String> = frozen
        .iter()
        .filter(|(f, m)| now.get(*f).copied().unwrap_or(0) < **m)
        .map(|(f, m)| format!("{f}: now {}, listed {m}", now.get(f).copied().unwrap_or(0)))
        .collect();
    assert!(
        grew.is_empty(),
        "test-only code grew in src/:\n  {}\n\nA helper that only tests use belongs in the unit file \
         beside them ({HOME}/unit/…); instrumentation production code must call stays, frozen here.",
        grew.join("\n  ")
    );
    assert!(
        stale.is_empty(),
        "these files shed test-only items — lower their counts in {GOLDEN} (the list only shrinks):\n  {}",
        stale.join("\n  ")
    );
}
