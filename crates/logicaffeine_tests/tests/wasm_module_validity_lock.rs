//! ★ LOCK — "we emitted a module nobody checked" cannot happen again. ★
//!
//! Three real pages compiled to wasm and shipped an INVALID module in one session
//! (`signup_form`'s `checkMinLength`/`checkSame` chain, a `HttpError` field access, and this
//! campaign's own S8 landing) before anything ever asked wasmi/wasmparser whether the bytes it
//! produced actually validate. A backend that REFUSES a program it cannot lower is sound (a
//! `ParseError`, caught everywhere); a backend that claims success and hands back bytes the wasm
//! spec itself rejects is a lie nothing else here catches, since nothing downstream of
//! `compile_to_wasm` re-validates before shipping a page.
//!
//! This walks every `examples/*/pages/*.lg` file (every gauntlet app lands here once merged) and,
//! for each one that `compile_to_wasm` accepts at all, asserts the bytes it returns VALIDATE.
//! A page the backend soundly REFUSES (an unsupported construct, a sound `ParseError`) is not a
//! failure here — that is a correctness gate this file has no opinion about. Only "claimed success,
//! produced garbage" fails.

use logicaffeine_compile::compile::compile_to_wasm;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every `pages/*.lg` file under `examples/` — the "a page" convention every app in this campaign
/// shares (`counter`, `todo`, `task_board`, `marketing_site`, `fullstack_todo`, `tic_tac_toe`, …).
fn every_example_page() -> Vec<PathBuf> {
    let examples = repo_root().join("examples");
    let mut out = Vec::new();
    let Ok(apps) = std::fs::read_dir(&examples) else {
        return out;
    };
    for app in apps.flatten() {
        let pages = app.path().join("pages");
        let Ok(entries) = std::fs::read_dir(&pages) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|e| e.to_str()) == Some("lg") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn every_example_page_that_compiles_to_wasm_produces_a_valid_module() {
    let pages = every_example_page();
    assert!(
        !pages.is_empty(),
        "no `examples/*/pages/*.lg` files found — the walk itself is broken"
    );
    let engine = wasmi::Engine::default();
    let mut invalid = Vec::new();
    let mut refused = Vec::new();
    let mut checked = 0usize;
    for page in &pages {
        let src = std::fs::read_to_string(page)
            .unwrap_or_else(|e| panic!("read {}: {e}", page.display()));
        match compile_to_wasm(&src) {
            Ok(bytes) => {
                checked += 1;
                if let Err(e) = wasmi::Module::new(&engine, &bytes[..]) {
                    invalid.push(format!("{}: {e}", page.display()));
                }
            }
            Err(e) => refused.push(format!("{}: {e:?}", page.display())),
        }
    }
    assert!(
        checked > 0,
        "every example page refused to compile — nothing here was actually checked: {refused:?}"
    );
    assert!(
        invalid.is_empty(),
        "{} page(s) compiled to an INVALID wasm module — a claimed success, not a sound refusal:\n{}",
        invalid.len(),
        invalid.join("\n")
    );
}

/// Every `pages/*.lg` of each app directory under `dir`.
fn pages_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(apps) = std::fs::read_dir(dir) else {
        return out;
    };
    for app in apps.flatten() {
        let Ok(entries) = std::fs::read_dir(app.path().join("pages")) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|e| e.to_str()) == Some("lg") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// ★ EVERY PAGE THAT SHIPS REACHES A BROWSER. The walk above holds only the weaker half — what
/// compiles must validate — and lets a refused page pass. A refused page is a page no browser can
/// load, so here a refusal fails exactly as an invalid module does.
///
/// Each page is assembled with its imports the way `largo check` assembles it (a page's
/// `Import Layout.` pulls in the layout it extends), then compiled with `compile_to_wasm`, the
/// entry the dev server and the release bundler both use, and handed to a real engine. The apps are
/// every example under `examples/`, every gauntlet app parked under
/// `tests/fixtures/lgx_gauntlet_pending/`, and the page shapes under `tests/fixtures/wasm_pages/`
/// that once failed to reach a browser (`extends_with_state`: a page that extends a layout and sets
/// its own state); the tier scenarios under `tests/tier_scenarios/` are
/// component libraries a test mounts by name, so each is compiled with its `App` as a host entry.
#[test]
fn every_page_that_ships_compiles_to_a_module_an_engine_loads() {
    let mut pages = pages_under(&repo_root().join("examples"));
    pages.extend(pages_under(&repo_root().join(
        "crates/logicaffeine_tests/tests/fixtures/lgx_gauntlet_pending",
    )));
    pages.extend(pages_under(
        &repo_root().join("crates/logicaffeine_tests/tests/fixtures/wasm_pages"),
    ));
    assert!(
        pages.len() >= 25,
        "only {} pages found — the walk itself is broken",
        pages.len()
    );
    let engine = wasmi::Engine::default();
    let mut failed = Vec::new();
    let mut check = |what: String, compiled: Result<Vec<u8>, String>| match compiled {
        Ok(bytes) => {
            if let Err(e) = wasmi::Module::new(&engine, &bytes[..]) {
                failed.push(format!("{what}: INVALID module: {e}"));
            }
        }
        Err(e) => failed.push(format!("{what}: refused: {e}")),
    };
    for page in &pages {
        let rel = page
            .strip_prefix(repo_root())
            .unwrap_or(page)
            .display()
            .to_string();
        let compiled = logicaffeine_compile::analysis::assemble_project(page)
            .and_then(|assembled| compile_to_wasm(&assembled.source).map_err(|e| format!("{e:?}")));
        check(rel, compiled);
    }
    let scenarios = repo_root().join("crates/logicaffeine_tests/tests/tier_scenarios");
    let mut scenario_files: Vec<PathBuf> = std::fs::read_dir(&scenarios)
        .expect("tier_scenarios is readable")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("lg"))
        .collect();
    scenario_files.sort();
    assert!(
        !scenario_files.is_empty(),
        "no tier scenarios found — the walk itself is broken"
    );
    let entries = logicaffeine_compile::vm::reach::HostEntries::mounting(["App"]);
    for scenario in &scenario_files {
        let src = std::fs::read_to_string(scenario)
            .unwrap_or_else(|e| panic!("read {}: {e}", scenario.display()));
        let rel = scenario
            .strip_prefix(repo_root())
            .unwrap_or(scenario)
            .display()
            .to_string();
        check(
            rel,
            logicaffeine_compile::compile::compile_to_wasm_with(&src, &entries)
                .map_err(|e| format!("{e:?}")),
        );
    }
    assert!(
        failed.is_empty(),
        "{} page(s) cannot reach a browser:\n{}",
        failed.len(),
        failed.join("\n")
    );
}
