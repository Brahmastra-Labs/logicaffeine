//! P16 (CLI bundle) — `largo build --native-functions` (HOTSWAP §Axis-3 "the tools to
//! bundle them"). A program selects functions for the AOT-native tier with
//! `is exported for native`; the bundler scans them and pre-builds each into a cached
//! cdylib + manifest. The scan is fast (parse-only); the actual build is `#[ignore]`'d
//! (it shells out to `cargo`). Functions outside the scalar subset are skipped — they
//! keep running on VM+JIT, no gap at the seam.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{build_native_bundle, native_export_function_names};

const PROG: &str = "\
## To fast_add (a: Int, b: Int) -> Int is exported for native:
    Return a.

## To plain (a: Int) -> Int:
    Return a.

## To fast_id (n: Int) -> Int is exported for native:
    Return n.

## Main
Show fast_add(1, 2).
";

#[test]
fn scan_finds_only_native_exported_functions() {
    let names = native_export_function_names(PROG);
    assert_eq!(
        names,
        vec!["fast_add".to_string(), "fast_id".to_string()],
        "only `is exported for native` functions are selected, in source order"
    );
}

#[test]
fn scan_ignores_plain_and_c_exports() {
    let src = "\
## To exported_c (a: Int) -> Int is exported for c:
    Return a.

## To plain (a: Int) -> Int:
    Return a.

## Main
Show plain(1).
";
    assert!(
        native_export_function_names(src).is_empty(),
        "c-exports and unannotated functions are NOT bundled as native"
    );
}

#[test]
#[ignore = "builds cdylibs via cargo for every native-annotated function (slow)"]
fn bundle_builds_a_cdylib_per_native_function() {
    let dir = std::env::temp_dir().join(format!("logos_bundle_{}", std::process::id()));
    let manifest = build_native_bundle(PROG, &dir).expect("parses + types");

    let built: Vec<&str> = manifest.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        built,
        vec!["fast_add", "fast_id"],
        "every native-annotated scalar function is bundled"
    );
    for (name, so) in &manifest {
        assert!(so.exists(), "cdylib for {name} exists on disk at {so:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
