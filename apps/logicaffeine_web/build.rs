//! Stages the `/data` runtime bundle for every wasm build of the web app.
//!
//! The wasm client fetches its heavyweight data — the lexicon every Learn exercise draws from,
//! benchmark results, legal pages — from `/data/*`, which `scripts/web/stage-web-data.sh` derives
//! from the repo's sources of truth into `public/data`. Staging used to be a manual step before
//! `dx serve`/`dx build`; a server started without it answered `/data/lexicon.json` with the
//! SPA's index.html, and every Learn module opened on "The word bank failed to load". Running
//! the script here makes a web build without its data impossible.
//!
//! Cargo re-runs this when a staged source changes or the staged output changes or goes missing.
//! A re-run with nothing new is a no-op on disk — the script writes a file only when its bytes
//! differ — so `dx serve`, which watches public/, never sees a phantom edit and never rebuilds
//! itself in a loop. Native builds (tests, the SSG prerender server) compile the same bytes in
//! with `include_str!`, so they skip staging. `web_data_bundle` in logicaffeine_tests locks it.
//!
//! A wasm build also exports its shadow-stack pointer, which the Studio's fault boundary resets
//! after a trap (`wasm_fault_boundary` in logicaffeine_tests).
//!
//! Every build, native or wasm, also bakes the root CHANGELOG's `[Unreleased]` section into the
//! roadmap's Next release block (`roadmap_page` in logicaffeine_tests).
//!
//! wasm32 has no threads: `logicaffeine_kernel::term_store::run_on_stack` (and the reader's
//! `term_parser::on_stack_for`) size a bigger native stack per call by spawning an OS thread with
//! it, which is `Unsupported` on this target — the browser client panics the instant a term nests
//! past `INLINE_DEPTH` instead. There is no per-call fix on wasm32: the client links ONE stack big
//! enough for the kernel's own worst case up front, and those two functions run inline on it (see
//! their `target_arch = "wasm32"` branches).
//!
//! The byte count below is `stack_bytes_for_depth(MAX_TERM_DEPTH)` — `BASE_STACK_BYTES` (2 MiB)
//! plus `BYTES_PER_RUNG` (48 KiB) for every one of the kernel's 1600 admitted rungs — computed
//! here as a literal rather than by depending on the kernel crate from a build script, which would
//! compile it twice (once for this script's host target, once for wasm32) on every build for one
//! constant. `wasm_link_stack_matches_kernel_worst_case` in logicaffeine_tests locks the two
//! numbers together: change `BASE_STACK_BYTES`, `BYTES_PER_RUNG`, or `MAX_TERM_DEPTH` and that
//! test goes red until this literal is updated to match.
//!
//! The benchmarks page's native build (tests, the SSG prerender) compiles every benchmark's eleven sources in. The list
//! of benchmarks is `benchmarks/lib/benchmarks.tsv`, so this script writes the table from it — `$OUT_DIR/bench_sources.rs`,
//! one entry per registry row with absolute `include_str!` paths — and a program added to the registry is on the page with
//! no second list to update.
//!
//! The same native build compiles in `benchmarks/results/latest-jit-code.json`, the machine code the JIT emitted on each
//! benchmark, when a run has written it: `$OUT_DIR/bench_jit_code.rs` is `Some(include_str!(…))` then and `None` before.

use std::path::{Path, PathBuf};
use std::process::Command;

// The roadmap's CHANGELOG reader, shared with the crate so the build and its tests run one parser.
#[path = "src/ui/pages/changelog.rs"]
#[allow(dead_code)]
mod changelog;

/// Every repo path scripts/web/stage-web-data.sh reads, relative to the repo root.
const STAGED_SOURCES: &[&str] = &[
    "scripts/web/stage-web-data.sh",
    "benchmarks/results/latest.json",
    "benchmarks/results/solvers.json",
    "benchmarks/results/latest-codec.json",
    "benchmarks/results/latest-interp.json",
    "benchmarks/results/eda/mc.json",
    "benchmarks/results/eda/bmc.json",
    "benchmarks/results/eda/rewrite.json",
    "benchmarks/results/eda/count.json",
    "benchmarks/results/arena/crypto.json",
    "benchmarks/programs",
    "benchmarks/profile/flamegraphs.pdf",
    "apps/logicaffeine_web/privacy.html",
    "apps/logicaffeine_web/terms.html",
    "crates/logicaffeine_lexicon/assets/lexicon.json",
];

/// Bakes the root CHANGELOG's `[Unreleased]` section into the roadmap's Next release block, for
/// every target: the page shows unreleased work as soon as its entry is written, with no step to
/// forget, and carries only the headlines and two counts rather than the whole CHANGELOG.
fn bake_unreleased(root: &Path) {
    let path = root.join("CHANGELOG.md");
    println!("cargo:rerun-if-changed={}", path.display());
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let next = changelog::unreleased(&text);
    let source = format!(
        "pub static ADDED: &[&str] = &{:?};\npub const CHANGED: usize = {};\npub const FIXED: usize = {};\n",
        next.added, next.changed, next.fixed
    );
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("roadmap_next.rs");
    std::fs::write(&out, source).unwrap_or_else(|e| panic!("write {}: {e}", out.display()));
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let root = manifest.join("../..");
    bake_unreleased(&root);
    bench_sources();
    bench_jit_code();

    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("wasm32") {
        return;
    }

    println!("cargo:rustc-link-arg=-zstack-size=80740352");

    // The Studio's fault boundary (src/ui/fault.rs) puts the shadow-stack pointer back after a
    // trap, from JavaScript, so the module exports it.
    println!("cargo:rustc-link-arg=--export=__stack_pointer");

    for source in STAGED_SOURCES {
        println!("cargo:rerun-if-changed={}", root.join(source).display());
    }
    // The staged output itself: deleting public/data re-stages it on the next build.
    println!(
        "cargo:rerun-if-changed={}",
        manifest.join("public/data").display()
    );
    // A development run's results and profile, staged in place of the checked-in ones when set.
    println!("cargo:rerun-if-env-changed=BENCH_RESULTS_DIR");
    println!("cargo:rerun-if-env-changed=BENCH_PROFILE_DIR");

    let output = Command::new("bash")
        .arg(root.join("scripts/web/stage-web-data.sh"))
        .current_dir(&root)
        .output();
    match output {
        Ok(out) if out.status.success() => {}
        Ok(out) => panic!(
            "scripts/web/stage-web-data.sh failed ({}); the web build would ship without its /data bundle.\n{}{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(e) => panic!("could not run scripts/web/stage-web-data.sh ({e}); the web build would ship without its /data bundle"),
    }
}

/// `benchmarks/results/latest-jit-code.json`, the machine code the JIT emitted on each benchmark: `$OUT_DIR/bench_jit_code.rs`
/// is `Some(include_str!(…))` once a run has written it, else `None`.
fn bench_jit_code() {
    let manifest =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let code = manifest.join("../../benchmarks/results/latest-jit-code.json");
    println!("cargo:rerun-if-changed={}", code.display());
    let body = match code.canonicalize() {
        Ok(path) => format!("Some(include_str!({:?}))\n", path.display().to_string()),
        Err(_) => "None\n".to_string(),
    };
    let dest = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"))
        .join("bench_jit_code.rs");
    std::fs::write(&dest, body).unwrap_or_else(|e| panic!("{}: {e}", dest.display()));
}

fn bench_sources() {
    let manifest =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let benchmarks = manifest.join("../../benchmarks");
    let registry = benchmarks.join("lib/benchmarks.tsv");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", registry.display());
    let text = std::fs::read_to_string(&registry)
        .unwrap_or_else(|e| panic!("{}: {e}", registry.display()));
    let files = [
        ("c", "main.c"),
        ("cpp", "main.cpp"),
        ("rust", "main.rs"),
        ("zig", "main.zig"),
        ("go", "main.go"),
        ("java", "Main.java"),
        ("js", "main.js"),
        ("python", "main.py"),
        ("ruby", "main.rb"),
        ("nim", "main.nim"),
        ("logos", "main.lg"),
    ];
    let mut out = String::from("vec![\n");
    for line in text.lines().skip(1).filter(|l| !l.trim().is_empty()) {
        let id = line.split('\t').next().expect("an id column");
        out.push_str(&format!("    ({id:?}, BenchSources {{\n"));
        for (field, file) in files {
            let path = benchmarks.join("programs").join(id).join(file);
            let path = path
                .canonicalize()
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            out.push_str(&format!(
                "        {field}: include_str!({:?}),\n",
                path.display().to_string()
            ));
        }
        out.push_str("    }),\n");
    }
    out.push_str("]\n");
    let dest = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"))
        .join("bench_sources.rs");
    std::fs::write(&dest, out).unwrap_or_else(|e| panic!("{}: {e}", dest.display()));
}
