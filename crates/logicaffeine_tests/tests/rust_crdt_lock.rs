//! ★ THE RUST-AOT CRDT `Merge`/`Converge` ≡VM LOCK ★ (RED-first, TDD).
//!
//! The Rust backend is the GOLDEN REFERENCE tier — a silent miscompile here is the worst outcome, so
//! this lock proves the emitted Rust `Merge <src> into <tgt>` / `Converge <src> into <tgt>` runs
//! BYTE-IDENTICALLY to the VM oracle by actually compiling the generated Rust with `cargo` and running
//! the binary.
//!
//! THE GAP this pins closed (each was RED before the fix):
//!   * A `Shared` struct with plain (`Int`/`Text`/…) fields: the old `impl Merge` touched ONLY CRDT-typed
//!     fields, so a plain-`Int` field was SILENTLY SKIPPED (an empty `merge` body) — the Rust output diverged
//!     from the VM (which folds every field: `Int` → `wrapping_add`).
//!   * `Converge` on a bare-`Int` field: it must take per-field MAX, but the old code emitted `.merge()`
//!     (which ADDs, or was a no-op on the skipped field) — never MAX, and there was no `join` method at all.
//!   * (`Merge`/`Converge` on a struct with NO CRDT fields still lowers: every field folds by kind, so the
//!     generated `merge`/`join` cover pure-scalar structs too.)
//!
//! THE ORACLE (the single source of truth every tier is held to) is
//! `logicaffeine_compile::semantics::arith::{crdt_merge_field, crdt_join_field}`:
//!   * merge  → CRDT field `self.f.merge(&other.f)`; `Int` field `self.f = self.f.wrapping_add(other.f)`;
//!              else (`Text`/`Bool`/`Float`/…) `self.f = other.f.clone()` (last-writer-wins).
//!   * join   → identical EXCEPT `Int` field `self.f = self.f.max(other.f)` (the idempotent lattice join).
//!
//! ⚠️ RED tests are the spec. If the Rust tier disagrees, fix the codegen — NEVER weaken this file. The
//! `cargo`-build half SKIPS cleanly when `cargo` / the workspace crates are unavailable; the pure VM/
//! tree-walker oracle half always runs.
#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome, vm_outcome};

// ============================================================
//  The corpus programs
// ============================================================

/// 1. A `Shared` `ConvergentCount` counter, bumped on two DISTINCT replicas, then `Merge`d — per-replica
///    MAX (a G-Counter is already idempotent). `a` gets +5, `b` gets +3, `Merge b into a` totals 5+3=8.
const SHARED_COUNTER_MERGE: &str =
    "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n\
    Let mutable a be a new Counter.\n\
    Increase a's points by 5.\n\
    Let mutable b be a new Counter.\n\
    Increase b's points by 3.\n\
    Merge b into a.\n\
    Show a's points.\n";

/// 2. The `Converge` twin of program 1 — a counter is idempotent under BOTH ops, so still 8.
const SHARED_COUNTER_CONVERGE: &str =
    "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n\
    Let mutable a be a new Counter.\n\
    Increase a's points by 5.\n\
    Let mutable b be a new Counter.\n\
    Increase b's points by 3.\n\
    Converge b into a.\n\
    Show a's points.\n";

/// 3. A `Shared` struct with plain-`Int` fields (no CRDT field): `Merge` ADDs each field → x=3+5=8, y=10+2=12.
///    (OLD code: the `impl Merge` skipped the plain fields → an EMPTY merge → the Rust output stayed (3,10) ≠ VM.)
const SHARED_PLAIN_INT_MERGE: &str =
    "## Definition\nA Pair is Shared and has:\n    x: Int.\n    y: Int.\n## Main\n\
    Let mutable a be a new Pair with x 3 and y 10.\n\
    Let mutable b be a new Pair with x 5 and y 2.\n\
    Merge b into a.\n\
    Show a's x.\n\
    Show a's y.\n";

/// 4. The `Converge` twin of program 3: per-field MAX → x=max(3,5)=5, y=max(10,2)=10.
const SHARED_PLAIN_INT_CONVERGE: &str =
    "## Definition\nA Pair is Shared and has:\n    x: Int.\n    y: Int.\n## Main\n\
    Let mutable a be a new Pair with x 3 and y 10.\n\
    Let mutable b be a new Pair with x 5 and y 2.\n\
    Converge b into a.\n\
    Show a's x.\n\
    Show a's y.\n";

/// 5a. `Converge` applied TWICE — MAX is idempotent, so still (5, 10).
const SHARED_PLAIN_INT_CONVERGE_TWICE: &str =
    "## Definition\nA Pair is Shared and has:\n    x: Int.\n    y: Int.\n## Main\n\
    Let mutable a be a new Pair with x 3 and y 10.\n\
    Let mutable b be a new Pair with x 5 and y 2.\n\
    Converge b into a.\n\
    Converge b into a.\n\
    Show a's x.\n\
    Show a's y.\n";

/// 5b. `Merge` applied TWICE — ADD is NOT idempotent: x=3+5+5=13, y=10+2+2=14.
const SHARED_PLAIN_INT_MERGE_TWICE: &str =
    "## Definition\nA Pair is Shared and has:\n    x: Int.\n    y: Int.\n## Main\n\
    Let mutable a be a new Pair with x 3 and y 10.\n\
    Let mutable b be a new Pair with x 5 and y 2.\n\
    Merge b into a.\n\
    Merge b into a.\n\
    Show a's x.\n\
    Show a's y.\n";

/// 6a. A `Shared` struct with BOTH a `ConvergentCount` field AND a plain `Int` field — the case the OLD
///     codegen SILENTLY SKIPPED. `Merge`: the counter takes per-replica MAX (5+3=8), the plain `Int`
///     field ADDs (7+2=9).
const MIXED_SHARED_MERGE: &str = "## Definition\nA Mixed is Shared and has:\n    points: ConvergentCount.\n    tag: Int.\n## Main\n\
    Let mutable a be a new Mixed with tag 7.\n\
    Increase a's points by 5.\n\
    Let mutable b be a new Mixed with tag 2.\n\
    Increase b's points by 3.\n\
    Merge b into a.\n\
    Show a's points.\n\
    Show a's tag.\n";

/// 6b. The `Converge` twin of 6a: the counter still totals 8 (idempotent), the plain `Int` field takes
///     MAX (max(7,2)=7).
const MIXED_SHARED_CONVERGE: &str = "## Definition\nA Mixed is Shared and has:\n    points: ConvergentCount.\n    tag: Int.\n## Main\n\
    Let mutable a be a new Mixed with tag 7.\n\
    Increase a's points by 5.\n\
    Let mutable b be a new Mixed with tag 2.\n\
    Increase b's points by 3.\n\
    Converge b into a.\n\
    Show a's points.\n\
    Show a's tag.\n";

/// Every corpus program, with the EXACT lines the VM oracle must produce.
fn corpus() -> Vec<(&'static str, &'static str, Vec<&'static str>)> {
    vec![
        ("shared_counter_merge", SHARED_COUNTER_MERGE, vec!["8"]),
        (
            "shared_counter_converge",
            SHARED_COUNTER_CONVERGE,
            vec!["8"],
        ),
        (
            "nonshared_int_merge",
            SHARED_PLAIN_INT_MERGE,
            vec!["8", "12"],
        ),
        (
            "nonshared_int_converge",
            SHARED_PLAIN_INT_CONVERGE,
            vec!["5", "10"],
        ),
        (
            "nonshared_int_converge_twice",
            SHARED_PLAIN_INT_CONVERGE_TWICE,
            vec!["5", "10"],
        ),
        (
            "nonshared_int_merge_twice",
            SHARED_PLAIN_INT_MERGE_TWICE,
            vec!["13", "14"],
        ),
        ("mixed_shared_merge", MIXED_SHARED_MERGE, vec!["8", "9"]),
        (
            "mixed_shared_converge",
            MIXED_SHARED_CONVERGE,
            vec!["8", "7"],
        ),
    ]
}

// ============================================================
//  1. The pure oracles agree with the pinned expectation (always runs)
// ============================================================

#[test]
fn the_oracles_agree_with_the_pinned_expectation() {
    for (label, src, want) in corpus() {
        let tw = tw_outcome(src);
        let vm = vm_outcome(src);
        assert!(
            tw.error.is_none(),
            "`{label}` tree-walker errored: {:?}",
            tw.error
        );
        assert!(vm.error.is_none(), "`{label}` VM errored: {:?}", vm.error);
        assert_eq!(
            tw.output.lines().collect::<Vec<_>>(),
            want,
            "`{label}`: tree-walker output must equal the pinned expectation"
        );
        assert_eq!(
            vm.output.lines().collect::<Vec<_>>(),
            want,
            "`{label}`: VM output must equal the pinned expectation"
        );
    }
}

// ============================================================
//  2. The RUST-AOT tier agrees with the VM — build + run the emitted Rust
// ============================================================

/// Compile a LOGOS program to Rust, stage it as a `cargo` project path-depending on the workspace runtime
/// crates, `cargo build` it, run the binary, and return its trimmed stdout. `None` on any missing tool /
/// build / run failure so the whole lock SKIPS cleanly off the bench box. Mirrors the workspace's
/// `logicaffeine-tests::common::run_logos` build path exactly (path deps + shared, sharded target cache).
fn rust_aot_output(src: &str, label: &str) -> Option<String> {
    let rust_code = compile_to_rust(src).ok()?;
    let workspace_root = workspace_root();
    let data = workspace_root.join("crates/logicaffeine_data");
    let system = workspace_root.join("crates/logicaffeine_system");
    if !data.join("Cargo.toml").exists() || !system.join("Cargo.toml").exists() {
        return None;
    }

    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "logos_rust_crdt_{}_{}_{}",
        std::process::id(),
        label,
        n
    ));
    let src_dir = dir.join("src");
    std::fs::create_dir_all(&src_dir).ok()?;

    let pkg_name = format!("logos_rust_crdt_{}_{}", std::process::id(), n);
    let cargo_toml = format!(
        "[package]\nname = \"{pkg}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [dependencies]\n\
         logicaffeine-data = {{ path = \"{data}\" }}\n\
         logicaffeine-system = {{ path = \"{system}\", features = [\"full\"] }}\n\
         tokio = {{ version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }}\n\
         serde = {{ version = \"1\", features = [\"derive\"] }}\n",
        pkg = pkg_name,
        data = data.display(),
        system = system.display(),
    );
    std::fs::write(dir.join("Cargo.toml"), cargo_toml).ok()?;
    std::fs::write(src_dir.join("main.rs"), &rust_code).ok()?;

    // A shared target cache keeps the runtime crates compiled once across the corpus. Try `--offline`
    // first (the workspace is already fetched), fall back to online if the cache is cold.
    let target_dir = shared_target_dir();
    let build = |offline: bool| {
        let mut args = vec!["run", "--quiet"];
        if offline {
            args.push("--offline");
        }
        Command::new("cargo")
            .args(&args)
            .current_dir(&dir)
            .env("CARGO_TARGET_DIR", &target_dir)
            .output()
    };
    let out = build(true).ok()?;
    let out = if !out.status.success() && String::from_utf8_lossy(&out.stderr).contains("--offline")
    {
        build(false).ok()?
    } else {
        out
    };
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let _ = std::fs::remove_dir_all(&dir);
    if !out.status.success() {
        eprintln!("[rust-crdt] `{label}` cargo build/run FAILED — the emitted Rust must compile+run ≡VM:\n{stderr}\n\n=== emitted Rust ===\n{rust_code}");
        return Some(format!("__BUILD_FAILED__: {stderr}"));
    }
    Some(stdout)
}

/// The workspace root — `CARGO_MANIFEST_DIR` is `crates/logicaffeine_compile`, so go up twice.
fn workspace_root() -> PathBuf {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap());
    manifest
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or(manifest)
}

/// One target cache shared across the corpus so the runtime crates compile once (not once per program).
fn shared_target_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("logos_rust_crdt_target_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    d
}

fn have_cargo() -> bool {
    Command::new("cargo")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn rust_aot_tier_agrees_with_vm() {
    if !have_cargo() {
        eprintln!("SKIP rust-aot execution: cargo not installed");
        return;
    }
    for (label, src, want) in corpus() {
        // The VM oracle for this program (self-consistent with the pinned expectation, checked above).
        let vm = vm_outcome(src);
        assert!(vm.error.is_none(), "`{label}` VM errored: {:?}", vm.error);
        let vm_lines: Vec<String> = vm.output.lines().map(|s| s.to_string()).collect();
        assert_eq!(
            vm_lines, want,
            "`{label}`: VM must equal the pinned expectation"
        );

        let got = match rust_aot_output(src, label) {
            Some(g) => g,
            None => {
                eprintln!("SKIP `{label}`: workspace runtime crates unavailable");
                return;
            }
        };
        assert!(
            !got.starts_with("__BUILD_FAILED__"),
            "`{label}`: the emitted Rust failed to compile/run — fix the codegen's Merge/Converge lowering.\n{got}"
        );
        assert_eq!(
            got.lines().collect::<Vec<_>>(),
            want,
            "`{label}`: Rust-AOT output must equal the VM oracle"
        );
    }
    // Clean up the shared cache after the whole corpus.
    let _ = std::fs::remove_dir_all(shared_target_dir());
}
