//! The binaries the suites spawn are the binaries that ship.
//!
//! Cargo exposes `CARGO_BIN_EXE_<name>` only to tests of the package that owns the binary, and two
//! packages emitting the same file name collide in the target directory, so the CLI suites cannot
//! spawn `largo` itself from this crate. Instead this crate builds `largo-under-test` (and
//! `logos-sat-under-test`), each of whose `main` is exactly one call to the product crate's library
//! entry, and the product's own `main` is that same call. A suite that passes against the shim has
//! therefore run the shipped program; this lock keeps it so: no logic may creep into either side.

use std::path::{Path, PathBuf};

use quote::ToTokens;

/// (shim, the product's main file, the one call both make).
const SHIMS: &[(&str, &str, &str)] = &[
    (
        "largo-under-test",
        "apps/logicaffeine_cli/src/main.rs",
        "logicaffeine_cli :: largo_main ()",
    ),
    (
        "logos-sat-under-test",
        "crates/logicaffeine_proof/src/bin/logos-sat.rs",
        "logicaffeine_proof :: satcli :: main ()",
    ),
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// The native `main`'s body, as a list of statement token strings.
fn main_body(file: &Path) -> Vec<String> {
    let src =
        std::fs::read_to_string(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
    let ast =
        syn::parse_file(&src).unwrap_or_else(|e| panic!("{} does not parse: {e}", file.display()));
    let mains: Vec<&syn::ItemFn> = ast
        .items
        .iter()
        .filter_map(|i| match i {
            syn::Item::Fn(f) if f.sig.ident == "main" => Some(f),
            _ => None,
        })
        .filter(|f| {
            !f.attrs.iter().any(|a| {
                a.to_token_stream()
                    .to_string()
                    .replace(' ', "")
                    .contains("cfg(target_arch=\"wasm32\")")
            })
        })
        .collect();
    assert_eq!(
        mains.len(),
        1,
        "{} must have exactly one native `main`",
        file.display()
    );
    mains[0]
        .block
        .stmts
        .iter()
        .map(|s| s.to_token_stream().to_string())
        .map(|s| s.trim_end_matches(';').trim().to_string())
        .collect()
}

/// Every `.rs` under `dir`, recursively.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
}

/// A suite finds the binary it spawns where the RUN put it: `support/under_test.rs` is the one file a
/// build-time binary path appears in, and there each run-time name is read before the build-time one.
/// A path baked in anywhere else is a suite that fails wherever the run is not the build (a nextest
/// archive, CI).
#[test]
fn every_suite_finds_its_binary_where_the_run_put_it() {
    let tests = root().join("crates/logicaffeine_tests/tests");
    let locator = tests.join("support/under_test.rs");
    let mut files = Vec::new();
    sources(&tests, &mut files);
    files.sort();
    let baked: Vec<String> = files
        .iter()
        .filter(|f| **f != locator)
        .filter(|f| {
            std::fs::read_to_string(f)
                .unwrap_or_default()
                .contains("env!(\"CARGO_BIN_EXE_")
        })
        .map(|f| f.strip_prefix(&root()).unwrap_or(f).display().to_string())
        .collect();
    assert!(
        baked.is_empty(),
        "a build-time binary path outside support/under_test.rs: {baked:?}"
    );
    let src = std::fs::read_to_string(&locator).expect("support/under_test.rs");
    for (shim, _, _) in SHIMS {
        let run = src.find(&format!("var_os(\"CARGO_BIN_EXE_{shim}\")"));
        let build = src.find(&format!("env!(\"CARGO_BIN_EXE_{shim}\")"));
        assert!(
            matches!((run, build), (Some(r), Some(b)) if r < b),
            "support/under_test.rs reads {shim}'s run-time path before its build-time one"
        );
    }
}

#[test]
fn every_shim_and_its_product_main_are_the_same_one_call() {
    let root = root();
    for (shim, product, call) in SHIMS {
        let shim_file = root.join(format!("crates/logicaffeine_tests/src/bin/{shim}.rs"));
        assert_eq!(
            main_body(&shim_file),
            vec![call.to_string()],
            "{shim}'s main must be exactly `{call}`"
        );
        assert_eq!(
            main_body(&root.join(product)),
            vec![call.to_string()],
            "{product}'s main must be exactly `{call}` — anything else the shipped binary does that the \
             shim does not is behaviour the suites never ran"
        );
    }
}

#[test]
fn every_test_crate_binary_is_a_declared_shim() {
    let root = root();
    let manifest: toml::Table =
        std::fs::read_to_string(root.join("crates/logicaffeine_tests/Cargo.toml"))
            .unwrap()
            .parse()
            .unwrap();
    let declared: Vec<(String, String)> = manifest
        .get("bin")
        .and_then(|b| b.as_array())
        .map(|bins| {
            bins.iter()
                .map(|b| {
                    (
                        b["name"].as_str().unwrap().to_string(),
                        b["path"].as_str().unwrap().to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let mut on_disk: Vec<String> =
        std::fs::read_dir(root.join("crates/logicaffeine_tests/src/bin"))
            .map(|rd| {
                rd.flatten()
                    .map(|e| {
                        e.file_name()
                            .to_string_lossy()
                            .trim_end_matches(".rs")
                            .to_string()
                    })
                    .collect()
            })
            .unwrap_or_default();
    on_disk.sort();
    let mut names: Vec<String> = declared.iter().map(|(n, _)| n.clone()).collect();
    names.sort();
    let mut shims: Vec<String> = SHIMS.iter().map(|(s, _, _)| s.to_string()).collect();
    shims.sort();
    assert_eq!(
        names, shims,
        "every [[bin]] of logicaffeine-tests is a shim this lock pins"
    );
    assert_eq!(on_disk, shims, "src/bin holds exactly the pinned shims");
    for (name, path) in declared {
        assert_eq!(
            path,
            format!("src/bin/{name}.rs"),
            "[[bin]] {name} points at its own file"
        );
    }
}
