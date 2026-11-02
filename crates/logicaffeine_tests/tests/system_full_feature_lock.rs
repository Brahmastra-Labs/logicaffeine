//! `logicaffeine-system`'s `full` feature bundle must be FULL.
//!
//! Every generated project the toolchain stages — the e2e harness, `largo check --deep`'s
//! rustc pass, `compile_to_dir`, the CLI's project build — depends on
//! `logicaffeine-system` with `features = ["full", …]`. The stdlib prelude binds native
//! modules behind individual features (`http.lg` → `logicaffeine_system::http`, gated by
//! the `http` feature), so a capability feature missing from `full` surfaces as a rustc
//! "cannot find `http` in `logicaffeine_system`" in whichever staging path forgot to add
//! it by hand. The bundle is the single place to get this right: every native,
//! platform-independent capability feature belongs in `full`; only platform-specific
//! features (`io-uring` is Linux-only) stay opt-in.

use std::path::PathBuf;

fn system_manifest() -> toml::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_system/Cargo.toml");
    let text = std::fs::read_to_string(&path).expect("read logicaffeine_system/Cargo.toml");
    text.parse::<toml::Value>()
        .expect("parse logicaffeine_system/Cargo.toml")
}

fn feature_list(manifest: &toml::Value, name: &str) -> Vec<String> {
    manifest["features"][name]
        .as_array()
        .unwrap_or_else(|| panic!("feature `{name}` missing"))
        .iter()
        .map(|v| v.as_str().expect("feature entry is a string").to_string())
        .collect()
}

/// The transitive closure of a feature bundle over the crate's own feature names
/// (`dep:` entries and `crate/feature` forwards are leaves).
fn closure(manifest: &toml::Value, root: &str) -> std::collections::BTreeSet<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut stack = vec![root.to_string()];
    while let Some(f) = stack.pop() {
        if !seen.insert(f.clone()) {
            continue;
        }
        if manifest["features"].get(&f).is_some() {
            for child in feature_list(manifest, &f) {
                if !child.starts_with("dep:") && !child.contains('/') {
                    stack.push(child);
                }
            }
        }
    }
    seen
}

/// Every platform-independent native capability feature is reachable from `full`.
#[test]
fn full_includes_every_platform_independent_capability() {
    let m = system_manifest();
    let features = m["features"].as_table().expect("[features] table");
    let platform_specific = ["io-uring"];
    let bundles = ["full", "distributed", "default"];
    let full = closure(&m, "full");
    let missing: Vec<&String> = features
        .keys()
        .filter(|f| !platform_specific.contains(&f.as_str()) && !bundles.contains(&f.as_str()))
        .filter(|f| !full.contains(f.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "logicaffeine-system `full` is not full — capability feature(s) not reachable from it: {missing:?}. \
         Add them to `full` (or, if platform-specific, to the `platform_specific` list here with a reason)."
    );
}

/// The stdlib's `http.lg` binds `logicaffeine_system::http`; `full` must carry the `http` feature
/// so every `features = ["full"]` staging path compiles a program that touches the HTTP prelude.
#[test]
fn full_carries_the_http_feature_the_stdlib_prelude_binds() {
    let m = system_manifest();
    assert!(
        closure(&m, "full").contains("http"),
        "`full` must include `http`: the prelude's http.lg binds logicaffeine_system::http"
    );
}
