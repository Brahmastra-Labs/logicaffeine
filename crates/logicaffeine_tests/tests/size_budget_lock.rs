//! Everything we ship has a size budget, and the only numbers are in one file.
//!
//! `.config/size-budgets.toml` holds a budget for every published crate's `.crate` tarball
//! (`[crates]`, checked by `scripts/check-packaging.sh` step 3), for the `dist`-profile `largo` and
//! `logicaffeine-lsp` binaries (`[binaries]`, checked by `scripts/check-binary-sizes.sh`, which
//! release.yml runs on the x86_64 Linux builds), and for the WASM the frontend deploys (`[web]`, checked by
//! `scripts/web/wasm-size-gate.sh` on every deploy and by `check-binary-sizes.sh --web`). Each budget is a
//! measured size plus headroom: it comes down after a measured shrink, and raising one is a deliberate,
//! reviewed edit. This lock keeps the file complete (a crate without a budget is a crate nobody weighs),
//! well-formed for the flat reader the scripts share, and the only source of the numbers; the deploy gate
//! is also run on sparse fixtures to prove it weighs against the file to the byte.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn budgets() -> toml::Table {
    read(".config/size-budgets.toml")
        .parse()
        .expect(".config/size-budgets.toml parses as TOML")
}

/// Every workspace member whose manifest has no `publish` key — the set publish.yml uploads.
fn published() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for parent in ["crates", "apps", "scripts"] {
        let Ok(rd) = std::fs::read_dir(root().join(parent)) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(text) = std::fs::read_to_string(e.path().join("Cargo.toml")) else {
                continue;
            };
            let manifest: toml::Table = text.parse().unwrap();
            let Some(pkg) = manifest.get("package") else {
                continue;
            };
            if pkg.get("publish").is_none() {
                out.insert(pkg["name"].as_str().unwrap().to_string());
            }
        }
    }
    assert!(
        out.len() >= 10,
        "found only {} published crates — the discovery walk broke",
        out.len()
    );
    out
}

#[test]
fn every_published_crate_and_shipped_binary_has_a_budget() {
    let b = budgets();
    let crates: BTreeSet<String> = b["crates"]
        .as_table()
        .expect("[crates]")
        .keys()
        .cloned()
        .collect();
    let want = published();
    let missing: Vec<&String> = want.difference(&crates).collect();
    let stale: Vec<&String> = crates.difference(&want).collect();
    assert!(
        missing.is_empty(),
        "published crates with no [crates] budget: {missing:?}"
    );
    assert!(
        stale.is_empty(),
        "[crates] budgets for crates that are not published: {stale:?}"
    );
    let bins: BTreeSet<String> = b["binaries"]
        .as_table()
        .expect("[binaries]")
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        bins,
        ["largo", "logicaffeine-lsp"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        "[binaries] names the shipped binaries"
    );
    assert!(
        b["web"].as_table().expect("[web]").contains_key("wasm"),
        "[web] carries the WASM budget"
    );
    for (section, table) in &b {
        for (key, value) in table.as_table().unwrap() {
            let n = value
                .as_integer()
                .unwrap_or_else(|| panic!("[{section}] {key} must be an integer byte count"));
            assert!(n > 0, "[{section}] {key} must be positive");
        }
    }
}

/// The scripts read the budgets with a flat line reader (no TOML library on older CI runners), so
/// every entry is exactly `key = <integer>` on its own line.
#[test]
fn every_entry_is_a_flat_line_the_scripts_can_read() {
    for (n, line) in read(".config/size-budgets.toml").lines().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || (t.starts_with('[') && t.ends_with(']')) {
            continue;
        }
        let (k, v) = t
            .split_once('=')
            .unwrap_or_else(|| panic!("line {}: `{t}` is not `key = <integer>`", n + 1));
        assert!(
            !k.trim().is_empty() && v.trim().chars().all(|c| c.is_ascii_digit()),
            "line {}: `{t}` is not `key = <integer>` (no trailing comment)",
            n + 1
        );
    }
}

#[test]
fn the_checkers_read_the_budgets_and_nothing_else_carries_a_number() {
    let packaging = read("scripts/check-packaging.sh");
    let sizes = read("scripts/check-binary-sizes.sh");
    assert!(
        packaging.contains(".config/size-budgets.toml"),
        "check-packaging.sh must read .config/size-budgets.toml"
    );
    assert!(
        sizes.contains(".config/size-budgets.toml"),
        "check-binary-sizes.sh must read .config/size-budgets.toml"
    );
    // The one fixed number is crates.io's hard ceiling margin; no budget may be copied into a script.
    let big_numbers = |s: &str| {
        s.split(|c: char| !c.is_ascii_digit())
            .filter(|d| d.len() >= 5)
            .count()
    };
    assert_eq!(
        big_numbers(&sizes),
        0,
        "check-binary-sizes.sh carries a byte count of its own"
    );
    assert_eq!(
        big_numbers(&packaging),
        0,
        "check-packaging.sh carries a byte count of its own (the ceiling is `8 * 1024 * 1024`)"
    );
}

/// The deploy gate (`scripts/web/wasm-size-gate.sh`, run by deploy-frontend.yml after the SSG build) weighs a
/// build directory against `[web] wasm` (the eager bundle) and `[web] wasm_total` (every `.wasm` together),
/// to the byte: a bundle exactly at its budget passes, one byte over fails, and chunks that each fit but
/// together exceed the total fail. Run on sparse fixtures, so nothing real is built.
#[test]
fn the_deploy_gate_weighs_the_web_wasm_against_the_file() {
    let b = budgets();
    let web = b["web"].as_table().expect("[web]");
    let main = web
        .get("wasm")
        .and_then(|v| v.as_integer())
        .expect("[web] wasm") as u64;
    let total = web
        .get("wasm_total")
        .and_then(|v| v.as_integer())
        .expect("[web] wasm_total: the budget for every .wasm together") as u64;
    assert!(
        total >= main,
        "[web] wasm_total ({total}) is below the eager bundle's own budget ({main})"
    );
    assert!(
        read(".github/workflows/deploy-frontend.yml")
            .contains("run: ./scripts/web/wasm-size-gate.sh"),
        "deploy-frontend.yml must gate every deploy on scripts/web/wasm-size-gate.sh"
    );
    let gate = root().join("scripts/web/wasm-size-gate.sh");
    let text = read("scripts/web/wasm-size-gate.sh");
    let copied = text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|d| d.len() >= 5)
        .count();
    assert_eq!(
        copied, 0,
        "wasm-size-gate.sh carries a byte count of its own — its budgets live in [web]"
    );
    let run = |name: &str, files: &[(&str, u64)]| {
        let dir =
            std::env::temp_dir().join(format!("size_budget_lock-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        for (file, len) in files {
            std::fs::File::create(dir.join("assets").join(file))
                .unwrap()
                .set_len(*len)
                .unwrap();
        }
        let out = std::process::Command::new("bash")
            .arg(&gate)
            .arg(&dir)
            .current_dir(root())
            .output()
            .expect("bash runs the gate");
        let _ = std::fs::remove_dir_all(&dir);
        (
            out.status.code(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    };
    let (code, log) = run("at", &[("logicaffeine-web_bg-dxh0.wasm", main)]);
    assert_eq!(
        code,
        Some(0),
        "an eager bundle exactly at [web] wasm ({main} bytes) must pass the deploy gate:\n{log}"
    );
    let (code, log) = run("over", &[("logicaffeine-web_bg-dxh0.wasm", main + 1)]);
    assert_eq!(
        code,
        Some(1),
        "an eager bundle one byte over [web] wasm must fail the deploy gate:\n{log}"
    );
    assert!(
        log.contains("MAIN bundle over budget"),
        "the failure names the eager bundle:\n{log}"
    );
    let chunk = total - main + 1;
    let (code, log) = run(
        "total",
        &[
            ("logicaffeine-web_bg-dxh0.wasm", main),
            ("chunk_0.wasm", chunk),
        ],
    );
    assert_eq!(
        code,
        Some(1),
        "chunks that together exceed [web] wasm_total must fail the deploy gate:\n{log}"
    );
    assert!(
        log.contains("TOTAL wasm over budget"),
        "the failure names the total:\n{log}"
    );
    let (code, log) = run("missing", &[]);
    assert_eq!(
        code,
        Some(2),
        "a build directory with no eager bundle is an error, not a pass:\n{log}"
    );
    let (code, log) = run("chunks", &[("chunk_0.wasm", 1)]);
    assert_eq!(
        code,
        Some(2),
        "a lazy chunk is not the eager bundle — only `logicaffeine-web_bg-*.wasm` is:\n{log}"
    );
}

#[test]
fn ci_and_the_release_run_the_checks() {
    let test_yml = read(".github/workflows/test.yml");
    assert!(
        test_yml.contains("./scripts/check-packaging.sh"),
        "per-push CI must run the packaging check (tarball budgets)"
    );
    assert!(
        test_yml.lines().any(|l| l.trim() == "run: ./scripts/check-binary-sizes.sh"),
        "per-push CI must build the shipped binaries and hold each to its budget, so a commit that grows one past it \
         fails the day it lands, not at the release"
    );
    let release = read(".github/workflows/release.yml");
    assert!(
        release.contains(
            "check-binary-sizes.sh --bin-dir target/${{ matrix.target }}/dist --only largo"
        ),
        "release.yml must check largo's size against its budget"
    );
    assert!(
        release.contains("check-binary-sizes.sh --bin-dir target/${{ matrix.target }}/dist --only logicaffeine-lsp"),
        "release.yml must check logicaffeine-lsp's size against its budget"
    );
}

/// The web bundle builds for size. Cloudflare Pages refuses any file over 25 MiB, and at the release profile's
/// opt-level 3 the main wasm was 27.4 MiB (v0.11.0) — the deploy failed on the upload, after every gate before it
/// passed. At `s` it is 20.8 MiB. The `[web]` wasm budget stays under that limit, so the size gate refuses a bundle
/// the host would refuse.
#[test]
fn the_web_bundle_builds_for_size_and_its_budget_fits_the_host() {
    let manifest: toml::Table = read("Cargo.toml").parse().expect("Cargo.toml parses as TOML");
    let profile = manifest["profile"]["wasm-release"].as_table().expect("[profile.wasm-release]");
    assert_eq!(
        profile.get("opt-level").and_then(|v| v.as_str()),
        Some("s"),
        "[profile.wasm-release] opt-level must be \"s\": at 3 the main wasm passes Cloudflare Pages' 25 MiB per-file limit"
    );
    const PAGES_FILE_LIMIT: i64 = 25 * 1024 * 1024;
    let web = budgets()["web"].as_table().expect("[web] in .config/size-budgets.toml").clone();
    let wasm = web["wasm"].as_integer().expect("[web] wasm is a byte count");
    assert!(wasm < PAGES_FILE_LIMIT, "[web] wasm budget {wasm} B is not under Cloudflare Pages' {PAGES_FILE_LIMIT} B per-file limit");
}
