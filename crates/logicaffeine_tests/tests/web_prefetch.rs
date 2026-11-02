//! Background wasm-chunk prefetch ratchet.
//!
//! The site code-splits every `#[component(lazy)]` route body into its own
//! content-hashed `module_*.wasm` / `chunk_*.wasm` blob, so the FIRST visit to
//! each page would otherwise block on downloading that page's chunk. The fix has
//! three load-bearing pieces that must stay wired together, or the whole feature
//! silently stops working:
//!
//!   1. `scripts/web/gen-prefetch-manifest.sh` writes `public/chunks.json` — the list
//!      of lazy-chunk URLs, EXCLUDING the eager main bundle.
//!   2. The shell `index.html` reads that list and warms every chunk in the
//!      background once the first page is idle.
//!   3. `public/_headers` caches the chunks immutably and revalidates the
//!      (unhashed) manifest; the deploy workflow runs the generator.
//!
//! Each check is its own `#[test]` so a break names the piece that regressed.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(web_prefetch)'`

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

const MANIFEST_SCRIPT: &str = "scripts/web/gen-prefetch-manifest.sh";

/// Run the generator against a throwaway `public/assets/` fixture and return the
/// parsed `chunks.json`.
fn run_generator(assets: &[(&str, usize)]) -> serde_json::Value {
    let dir = tempfile::tempdir().expect("tempdir");
    let assets_dir = dir.path().join("public").join("assets");
    std::fs::create_dir_all(&assets_dir).expect("mkdir assets");
    for (name, bytes) in assets {
        std::fs::write(assets_dir.join(name), vec![0u8; *bytes]).expect("write fixture");
    }

    let script = repo_root().join(MANIFEST_SCRIPT);
    let out = Command::new("bash")
        .arg(&script)
        .arg(dir.path()) // absolute web-dir → the script's `cd $ROOT` can't confuse it
        .output()
        .unwrap_or_else(|e| panic!("spawn {script:?}: {e}"));
    assert!(
        out.status.success(),
        "gen-prefetch-manifest.sh failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let json = std::fs::read_to_string(dir.path().join("public").join("chunks.json"))
        .expect("chunks.json written");
    serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("chunks.json is not valid JSON: {e}\n{json}"))
}

fn urls(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .expect("chunks.json is a JSON array")
        .iter()
        .map(|c| {
            c["url"]
                .as_str()
                .expect("each entry has a string url")
                .to_string()
        })
        .collect()
}

#[test]
fn manifest_lists_lazy_chunks_and_excludes_the_main_bundle() {
    let v = run_generator(&[
        ("module_aaa.wasm", 3000),
        ("chunk_bbb.wasm", 1000),
        // the eager bundle + its JS — every visitor already has these; prefetching
        // them would be pure waste, so they must NOT appear.
        ("logicaffeine-web_bg-dxhZZZ.wasm", 9000),
        ("logicaffeine-web-dxhYYY.js", 500),
    ]);
    let urls = urls(&v);

    assert!(
        urls.contains(&"/assets/module_aaa.wasm".to_string()),
        "manifest must include module_* chunks; got {urls:?}"
    );
    assert!(
        urls.contains(&"/assets/chunk_bbb.wasm".to_string()),
        "manifest must include chunk_* chunks; got {urls:?}"
    );
    assert!(
        !urls.iter().any(|u| u.contains("logicaffeine-web_bg-")),
        "manifest must EXCLUDE the eager main bundle (already loaded on first paint); got {urls:?}"
    );
    assert!(
        !urls.iter().any(|u| u.ends_with(".js")),
        "manifest is wasm-only (the split glue lives in the main JS); got {urls:?}"
    );

    // Each entry carries a byte size so the runtime can budget / order.
    for entry in v.as_array().unwrap() {
        assert!(
            entry["bytes"].as_u64().is_some(),
            "each manifest entry needs a numeric `bytes`; got {entry}"
        );
    }
}

#[test]
fn manifest_is_sorted_smallest_first() {
    // Smallest chunks first → more pages become "ready" sooner.
    let v = run_generator(&[
        ("module_big.wasm", 5000),
        ("chunk_small.wasm", 100),
        ("module_mid.wasm", 2000),
    ]);
    let sizes: Vec<u64> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["bytes"].as_u64().unwrap())
        .collect();
    let mut sorted = sizes.clone();
    sorted.sort_unstable();
    assert_eq!(
        sizes, sorted,
        "manifest entries must be sorted smallest-first"
    );
}

#[test]
fn a_split_free_build_yields_an_empty_manifest() {
    // Plain `dx serve` produces no split chunks; the manifest must be `[]` so the
    // runtime prefetch cleanly no-ops rather than erroring.
    let v = run_generator(&[("logicaffeine-web_bg-dxhZZZ.wasm", 9000)]);
    assert_eq!(
        v.as_array().map(|a| a.len()),
        Some(0),
        "a build with no module_*/chunk_* wasm must yield an empty manifest; got {v}"
    );
}

#[test]
fn shell_carries_the_prefetch_bootstrap() {
    let html = read("apps/logicaffeine_web/index.html");
    assert!(
        html.contains("__wasmPrefetchStarted"),
        "index.html lost the background-prefetch bootstrap — subsequent-page navigation \
         goes back to blocking on each chunk download"
    );
    assert!(
        html.contains("chunks.json"),
        "the prefetch bootstrap must read the build-generated chunks.json manifest"
    );
    // The politeness gates the plan promised — a regression here would spend a
    // metered visitor's data or a weak phone's CPU eagerly.
    assert!(
        html.contains("saveData"),
        "prefetch must honor data-saver mode"
    );
    assert!(
        html.contains("hardwareConcurrency"),
        "prefetch must downgrade to network-only on low-core devices"
    );
}

#[test]
fn headers_cache_chunks_immutably_and_revalidate_the_manifest() {
    let headers = read("apps/logicaffeine_web/public/_headers");
    for family in ["/assets/module_*.wasm", "/assets/chunk_*.wasm"] {
        assert!(
            headers.contains(family),
            "_headers must cache the lazy chunk family `{family}` (content-hashed → immutable)"
        );
    }
    assert!(
        headers.contains("immutable"),
        "_headers must mark the content-hashed chunks immutable"
    );
    assert!(
        headers.contains("/chunks.json"),
        "_headers must carry a rule for the unhashed /chunks.json manifest so it revalidates"
    );
}

#[test]
fn deploy_workflow_generates_the_manifest() {
    let wf = read(".github/workflows/deploy-frontend.yml");
    assert!(
        wf.contains("gen-prefetch-manifest.sh"),
        "deploy-frontend.yml must run gen-prefetch-manifest.sh, or the deployed site \
         ships without chunks.json and the prefetch silently no-ops"
    );
}

#[test]
fn the_manifest_script_exists_and_is_executable() {
    let path = repo_root().join(MANIFEST_SCRIPT);
    assert!(path.exists(), "{MANIFEST_SCRIPT} is missing");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("stat script")
            .permissions()
            .mode();
        assert!(
            mode & 0o111 != 0,
            "{MANIFEST_SCRIPT} must be executable (the deploy step runs it directly)"
        );
    }
}
