//! P16 — persistent AOT-native artifact cache (HOTSWAP §Axis-3). The cache key hashes
//! the optimized Rust source AND the rustc toolchain, so an identical function reuses
//! its `.so` across runs while a toolchain change forces a rebuild (never reusing a
//! stale, ABI-mismatched library).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{
    aot_cache_key, build_native_cdylib_cached, compile_function_to_native_rust,
};

const PROG: &str = "\
## To add (a: Int, b: Int) -> Int:
    Return a.

## Main
Show add(1, 2).
";

#[test]
fn aot_cache_key_is_deterministic_and_source_sensitive() {
    let k1 = aot_cache_key("fn add() {}");
    let k2 = aot_cache_key("fn add() {}");
    let k3 = aot_cache_key("fn sub() {}");
    assert_eq!(k1, k2, "same source + toolchain → same key");
    assert_ne!(k1, k3, "different source → different key");
    assert_eq!(k1.len(), 16, "stable 64-bit hex key");
}

#[test]
#[ignore = "builds a cdylib via cargo (slow) — cache-reuse proof"]
fn aot_cache_reuses_the_built_artifact() {
    let module = compile_function_to_native_rust(PROG, "add")
        .expect("source compiles")
        .expect("scalar target yields a module");
    let cache = std::env::temp_dir().join(format!("logos_aot_cache_{}", std::process::id()));

    let so1 = build_native_cdylib_cached(&module.rust, "add", &cache).expect("first build");
    assert!(so1.exists());
    let m1 = std::fs::metadata(&so1).unwrap().modified().unwrap();

    // Identical source ⇒ cache hit: same path, and NO rebuild (mtime unchanged).
    let so2 = build_native_cdylib_cached(&module.rust, "add", &cache).expect("cache hit");
    assert_eq!(so1, so2, "cache returns the same artifact path");
    let m2 = std::fs::metadata(&so2).unwrap().modified().unwrap();
    assert_eq!(m1, m2, "a cache hit must not rebuild the cdylib");

    let _ = std::fs::remove_dir_all(&cache);
}
