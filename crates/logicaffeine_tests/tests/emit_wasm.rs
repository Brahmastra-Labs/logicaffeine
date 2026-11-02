//! End-to-end TDD lock for `largo build --emit wasm` and `largo run --emit wasm`.
//!
//! The built-in WebAssembly backend compiles a LOGOS project DIRECTLY to a self-contained `.wasm`
//! (no rustc / cargo / wasm-bindgen) plus a `node` host shim. This suite drives the REAL `largo`
//! binary over throwaway projects and proves the emitted module — executed under node — prints
//! BYTE-IDENTICALLY to the tree-walking VM (`vm_outcome` is the oracle, so nothing is hard-coded).
//! It also pins the artifact layout (`target/<name>.wasm` + `.mjs`, a valid wasm header) and the
//! error UX (an unknown `--emit` target is rejected, not silently ignored). node-dependent tests
//! preflight `node` through the shared `require_tool` seam (G14): a missing `node` is a
//! machine-readable, COUNTED `LOGOS_TOOL_SKIP: node` marker — and a HARD FAILURE under
//! `LOGOS_REQUIRE_TOOLS` — never a silent SKIP-as-PASS. The toolchain-free build itself is
//! always exercised regardless.

#[path = "cli_common/mod.rs"]
mod common;
use common::require_tool;

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// The compiled `largo` binary under test, found where the run put it.
fn largo() -> Command {
    common::largo()
}

/// Scaffold a minimal LOGOS project (`Largo.toml` + `src/main.lg`) in a fresh temp dir.
fn scaffold(name: &str, source: &str) -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    std::fs::write(
        dir.path().join("Largo.toml"),
        format!("[package]\nname=\"{name}\"\nversion=\"0.1.0\"\nentry=\"src/main.lg\"\n"),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.lg"), source).unwrap();
    dir
}

/// Run `node <mjs> [args…]` and return its stdout (asserting a clean exit).
fn run_node(mjs: &Path, args: &[&str]) -> String {
    let out = Command::new("node")
        .arg(mjs)
        .args(args)
        .output()
        .expect("spawn node");
    assert!(
        out.status.success(),
        "node failed running {}:\n{}",
        mjs.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `largo build --emit wasm` in `dir`; assert success and return the emitted `(wasm, mjs)` paths.
fn build_emit_wasm(dir: &Path, name: &str) -> (PathBuf, PathBuf) {
    let out = largo()
        .current_dir(dir)
        .args(["build", "--emit", "wasm"])
        .output()
        .expect("spawn largo");
    assert!(
        out.status.success(),
        "`largo build --emit wasm` failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let target = dir.join("target");
    (
        target.join(format!("{name}.wasm")),
        target.join(format!("{name}.mjs")),
    )
}

/// The programs the toolchain-free backend must compile + run identically to the VM. Each is
/// self-contained (no overflow, no linker) so `--emit wasm` handles it without rustc.
const PROGRAMS: &[(&str, &str)] = &[
    ("scalar", "## Main\nShow 6 times 7.\n"),
    ("floaty", "## Main\nShow 10.0 divided by 4.0.\n"),
    ("texty", "## Main\nShow \"hello\" + \" world\".\n"),
    ("negation", "## Main\nShow 3 minus 10.\n"),
    ("multi", "## Main\nShow 1 plus 1.\nShow 2 times 3.\n"),
];

#[test]
fn build_emit_wasm_writes_a_valid_module_and_shim() {
    // The toolchain-free BUILD is exercised even without node: the bytes must be a valid wasm module
    // and the shim must be written beside them.
    for (name, src) in PROGRAMS {
        let dir = scaffold(name, src);
        let (wasm, mjs) = build_emit_wasm(dir.path(), name);
        assert!(wasm.exists(), "{name}.wasm was not written");
        assert!(mjs.exists(), "{name}.mjs host shim was not written");
        let bytes = std::fs::read(&wasm).unwrap();
        assert!(bytes.len() > 8, "{name}.wasm is suspiciously small");
        assert_eq!(
            &bytes[0..4],
            b"\0asm",
            "{name}.wasm lacks the wasm magic number"
        );
        assert_eq!(
            &bytes[4..8],
            &[1, 0, 0, 0],
            "{name}.wasm is not wasm version 1"
        );
    }
}

#[test]
fn build_emit_wasm_runs_byte_identically_to_the_vm() {
    if !require_tool("node") {
        return;
    }
    for (name, src) in PROGRAMS {
        let dir = scaffold(name, src);
        let (_wasm, mjs) = build_emit_wasm(dir.path(), name);
        let got = run_node(&mjs, &[]);
        let expected = logicaffeine_compile::compile::vm_outcome(src).output;
        assert_eq!(
            got.trim(),
            expected.trim(),
            "emitted wasm output disagrees with the VM for `{name}`:\n{src}"
        );
    }
}

#[test]
fn run_emit_wasm_compiles_and_runs_in_one_step() {
    if !require_tool("node") {
        return;
    }
    let src = "## Main\nShow 40 plus 2.\n";
    let dir = scaffold("runit", src);
    let out = largo()
        .current_dir(dir.path())
        .args(["run", "--emit", "wasm"])
        .output()
        .expect("spawn largo");
    assert!(
        out.status.success(),
        "`largo run --emit wasm` failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        logicaffeine_compile::compile::vm_outcome(src).output.trim(),
        "`largo run --emit wasm` output disagrees with the VM"
    );
}

/// A linked-build failure that is a wasm32 BASE/sysroot absence (not a wiring bug) — a capability
/// `LOGOS_REQUIRE_TOOLS` cannot force (it is a rustup target / lld / sysroot, not a PATH binary), so it
/// is recorded through `skip_missing_capability` (counted + forceable via `LOGOS_REQUIRE_CAPS`). `rustc`
/// and `cargo` themselves are PATH tools and are preflighted via `require_tool`, so their absence is
/// loud under `LOGOS_REQUIRE_TOOLS`; a real wiring/program bug has a different message and still panics.
fn is_wasm32_base_absence(stderr: &str) -> bool {
    ["wasm32", "lld", "base wasm32", "sysroot"]
        .iter()
        .any(|m| stderr.contains(m))
}

#[test]
fn build_emit_wasm_linked_computes_bigint_matching_the_vm() {
    if !require_tool("node") {
        return;
    }
    // The linked tier compiles the BigInt runtime with the Rust toolchain — preflight it so its
    // absence is a COUNTED marker (and loud under LOGOS_REQUIRE_TOOLS), never a false green.
    if !require_tool("cargo") || !require_tool("rustc") {
        return;
    }
    // Programs the SELF-CONTAINED backend would trap on (i64 overflow) but the linked BigInt runtime
    // computes exactly — the whole point of `--emit wasm-linked`.
    for (name, src) in [
        ("bigpow", "## Main\nShow 2 to the power of 200.\n"),
        ("bigmul", "## Main\nShow 99999999999 times 99999999999.\n"),
        (
            "bigchain",
            "## Main\nShow (2 to the power of 100) times (3 to the power of 50).\n",
        ),
        // MIXED heap + BigInt: a Text literal + concat (emitter heap, slab-allocated) around a BigInt.
        (
            "bigmixed",
            "## Main\nShow \"result = \" + (2 to the power of 128).\n",
        ),
    ] {
        let dir = scaffold(name, src);
        let out = largo()
            .current_dir(dir.path())
            .args(["build", "--emit", "wasm-linked"])
            .output()
            .expect("spawn largo");
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if is_wasm32_base_absence(&stderr) {
                if !common::skip_missing_capability(
                    "wasm32-base",
                    "wasm32 base/sysroot build unavailable",
                    "build_emit_wasm_linked_computes_bigint_matching_the_vm",
                ) {
                    return;
                }
            }
            panic!("`largo build --emit wasm-linked` failed for `{name}`:\nstdout: {}\nstderr: {stderr}", String::from_utf8_lossy(&out.stdout));
        }
        let mjs = dir.path().join("target").join(format!("{name}.mjs"));
        let got = run_node(&mjs, &[]);
        let expected = logicaffeine_compile::compile::vm_outcome(src).output;
        assert_eq!(
            got.trim(),
            expected.trim(),
            "`--emit wasm-linked` (real BigInt) output disagrees with the VM for `{name}`:\n{src}"
        );
    }
}

/// The custom sections of a wasm module, by name, with their sizes.
fn custom_sections(module: &[u8]) -> Vec<(String, usize)> {
    fn leb(bytes: &[u8], at: &mut usize) -> usize {
        let (mut value, mut shift) = (0usize, 0);
        loop {
            let byte = bytes[*at];
            *at += 1;
            value |= usize::from(byte & 0x7f) << shift;
            if byte < 0x80 {
                return value;
            }
            shift += 7;
        }
    }
    assert_eq!(&module[..4], b"\0asm", "a wasm module");
    let mut at = 8;
    let mut out = Vec::new();
    while at < module.len() {
        let id = module[at];
        at += 1;
        let size = leb(module, &mut at);
        let end = at + size;
        if id == 0 {
            let mut name_at = at;
            let len = leb(module, &mut name_at);
            out.push((String::from_utf8_lossy(&module[name_at..name_at + len]).into_owned(), size));
        }
        at = end;
    }
    out
}

/// A linked module carries none of its runtime's debug information: `--gc-sections` drops the code a
/// program never reaches, and the link strips the DWARF the runtime was built with, which describes
/// all of it — a program of kilobytes otherwise shipped some hundred megabytes of `.debug_*` sections
/// that every run reads. The program still computes what the VM computes.
#[test]
fn a_linked_module_carries_none_of_its_runtimes_debug_information() {
    if !require_tool("node") || !require_tool("cargo") || !require_tool("rustc") {
        return;
    }
    let src = "## Main\nShow 2 to the power of 200.\n";
    let dir = scaffold("stripped", src);
    let out = largo()
        .current_dir(dir.path())
        .args(["build", "--emit", "wasm-linked"])
        .output()
        .expect("spawn largo");
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success()
        && is_wasm32_base_absence(&stderr)
        && !common::skip_missing_capability(
            "wasm32-base",
            "wasm32 base/sysroot build unavailable",
            "a_linked_module_carries_none_of_its_runtimes_debug_information",
        )
    {
        return;
    }
    assert!(out.status.success(), "`largo build --emit wasm-linked` failed:\n{stderr}");
    let target = dir.path().join("target");
    let module = std::fs::read(target.join("stripped.wasm")).expect("the linked module");
    let debug: Vec<(String, usize)> = custom_sections(&module)
        .into_iter()
        .filter(|(name, _)| name.starts_with(".debug"))
        .collect();
    assert!(
        debug.is_empty(),
        "the linked module ({} bytes) carries its runtime's debug sections: {debug:?}",
        module.len()
    );
    assert_eq!(
        run_node(&target.join("stripped.mjs"), &[]).trim(),
        logicaffeine_compile::compile::vm_outcome(src).output.trim(),
        "a stripped linked module computes what the VM computes"
    );
}

#[test]
fn unknown_emit_target_is_rejected() {
    let dir = scaffold("bad", "## Main\nShow 1.\n");
    let out = largo()
        .current_dir(dir.path())
        .args(["build", "--emit", "elf"])
        .output()
        .expect("spawn largo");
    assert!(
        !out.status.success(),
        "an unknown --emit target must fail, not silently succeed"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("elf") || stderr.contains("emit"),
        "the error should name the bad --emit target:\n{stderr}"
    );
}
