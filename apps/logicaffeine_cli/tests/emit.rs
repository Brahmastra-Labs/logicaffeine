//! `largo emit <rust|c|wasm|wasm-linked>` — the unified code-emission verb.
//!
//! `build --emit`/`run --emit` remain as working aliases (pinned by
//! emit_wasm.rs); this file specs the documented home.

mod common;

use common::*;
use tempfile::tempdir;

/// `largo emit rust` prints the generated Rust program to stdout.
#[test]
fn emit_rust_prints_generated_program() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "emit_rust_proj");
    let out = largo_in(dir.path(), &["emit", "rust"]);
    assert_eq!(out.status.code(), Some(0), "emit rust: {}", stderr(&out));
    let rust = stdout(&out);
    assert!(rust.contains("fn main()"), "generated Rust must have fn main():\n{rust}");
    assert!(
        rust.contains("logicaffeine_data"),
        "generated Rust must reference the runtime:\n{rust}"
    );
}

/// `largo emit c` prints the C translation to stdout.
#[test]
fn emit_c_prints_c_translation() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "emit_c_proj");
    let out = largo_in(dir.path(), &["emit", "c"]);
    assert_eq!(out.status.code(), Some(0), "emit c: {}", stderr(&out));
    let c = stdout(&out);
    assert!(
        c.contains("#include") && c.contains("int main"),
        "must look like a C program:\n{c}"
    );
}

/// `-o` writes the emitted code to a file and keeps stdout quiet.
#[test]
fn emit_rust_with_output_file() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "emit_o_proj");
    let out_path = dir.path().join("generated.rs");
    let out = largo_in(dir.path(), &["emit", "rust", "-o", out_path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "emit rust -o: {}", stderr(&out));
    let written = std::fs::read_to_string(&out_path).expect("-o file must exist");
    assert!(written.contains("fn main()"));
    assert!(
        !stdout(&out).contains("fn main()"),
        "code must not also spill to stdout with -o"
    );
}

/// An explicit FILE argument works outside any project (single-file mode).
#[test]
fn emit_rust_explicit_file_outside_project() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("standalone.lg");
    std::fs::write(&file, "# Main\n\n## Main\n\nShow 6 * 7.\n").unwrap();
    let out = largo_in(dir.path(), &["emit", "rust", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "emit rust FILE: {}", stderr(&out));
    assert!(stdout(&out).contains("fn main()"));
}

/// `largo emit wasm` produces the same artifact layout as `build --emit wasm`.
#[test]
fn emit_wasm_writes_module_and_shim() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "emit_wasm_proj");
    let out = largo_in(dir.path(), &["emit", "wasm"]);
    assert_eq!(out.status.code(), Some(0), "emit wasm: {}", stderr(&out));
    let wasm = std::fs::read(dir.path().join("target/emit_wasm_proj.wasm"))
        .expect("target/<name>.wasm must exist");
    assert_eq!(&wasm[0..4], b"\0asm", "wasm magic");
    assert!(dir.path().join("target/emit_wasm_proj.mjs").exists(), "host shim must exist");
}

/// Unknown emit targets are rejected as usage errors by the value parser.
#[test]
fn unknown_emit_target_is_a_usage_error() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "emit_bad");
    let out = largo_in(dir.path(), &["emit", "elf"]);
    assert_eq!(out.status.code(), Some(2));
}

/// A source file whose stem contains JS-hostile characters (apostrophes)
/// must still produce a syntactically valid host shim — the name is a
/// string in generated JavaScript and must be escaped, not interpolated.
#[test]
fn emit_wasm_file_with_apostrophe_stem_produces_valid_shim() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("o'brien.lg");
    std::fs::write(&file, "## Main\n\nShow 6 * 7.\n").unwrap();

    let out = largo_in(dir.path(), &["emit", "wasm", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "emit: {}", stderr(&out));

    let mjs = dir.path().join("o'brien.mjs");
    assert!(mjs.exists(), "shim must be written");
    let check = std::process::Command::new("node")
        .args(["--check", mjs.to_str().unwrap()])
        .output()
        .expect("node available");
    assert!(
        check.status.success(),
        "the shim must be valid JavaScript:\n{}",
        String::from_utf8_lossy(&check.stderr)
    );
}

/// The wasm host shim's argv must survive heap pressure: the program's bump
/// allocator grows through the address space, and argv memory must live
/// where the program can NEVER reach (a shim-grown page), not at a fixed
/// low offset.
#[test]
fn wasm_argv_survives_heap_pressure() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Largo.toml"),
        "[package]\nname = \"argv_pressure\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/main.lg"),
        "## Main\n\nLet av be args().\nLet s be \"\".\nLet i be 0.\nWhile i is less than 2000:\n    Set s to s + \"x\".\n    Set i to i + 1.\nShow item 2 of av.\n",
    )
    .unwrap();

    let out = largo_in(dir.path(), &["run", "--emit", "wasm", "HELLO_SENTINEL"]);
    assert_eq!(out.status.code(), Some(0), "run: {}", stderr(&out));
    assert!(
        stdout(&out).contains("HELLO_SENTINEL"),
        "argv must survive 20k heap allocations:\n{}",
        stdout(&out)
    );
}

/// A trapping program must still flush the output it produced before the
/// trap (and fail loudly), not swallow everything.
#[test]
fn wasm_trap_preserves_prior_output() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Largo.toml"),
        "[package]\nname = \"trap_out\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    )
    .unwrap();
    // Integer overflow WRAPS by spec on this tier; the deterministic trap is
    // division by zero (`i64.div_s`). The divisor is loop-computed so
    // constant folding cannot pre-resolve it.
    std::fs::write(
        dir.path().join("src/main.lg"),
        "## Main\n\nShow \"before\".\nLet i be 0.\nWhile i is less than 3:\n    Set i to i + 1.\nLet d be i - 3.\nShow 10 / d.\n",
    )
    .unwrap();

    let out = largo_in(dir.path(), &["run", "--emit", "wasm"]);
    assert_ne!(out.status.code(), Some(0), "the trap must fail the run");
    assert!(
        stdout(&out).contains("before"),
        "pre-trap output must not be swallowed:\n{}",
        stdout(&out)
    );
}

/// Every entry-consuming verb honors the `.md` fallback the build path has:
/// a project whose entry exists only as `src/main.md` must interpret, emit,
/// and check — not die with a bare file-not-found.
#[test]
fn md_entry_fallback_works_everywhere() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Largo.toml"),
        "[package]\nname = \"md_proj\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/main.md"), "# Main\n\n## Main\n\nShow 6 * 7.\n").unwrap();

    let interp = largo_in(dir.path(), &["run", "--interpret"]);
    assert_eq!(interp.status.code(), Some(0), "interpret .md: {}", stderr(&interp));
    assert!(stdout(&interp).contains("42"));

    let wasm = largo_in(dir.path(), &["emit", "wasm"]);
    assert_eq!(wasm.status.code(), Some(0), "emit wasm .md: {}", stderr(&wasm));

    let check = largo_in(dir.path(), &["check"]);
    assert_eq!(check.status.code(), Some(0), "check .md: {}", stderr(&check));
}

/// `largo run --emit <bogus>` must be rejected, never silently fall through
/// to the plain cargo build/run with the flag discarded.
#[test]
fn run_with_unknown_emit_target_is_rejected() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "run_emit_bad");
    let out = largo_in(dir.path(), &["run", "--emit", "bogus"]);
    assert_eq!(out.status.code(), Some(1), "must fail, not fall through");
    assert!(
        strip_ansi(&stderr(&out)).contains("unknown --emit target"),
        "must name the problem:\n{}",
        stderr(&out)
    );
}

/// `--emit` and `--interpret` are mutually exclusive on `run`.
#[test]
fn run_emit_conflicts_with_interpret() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "run_emit_conflict");
    let out = largo_in(dir.path(), &["run", "--emit", "wasm", "--interpret"]);
    assert_eq!(out.status.code(), Some(2), "clap must reject the combination");
}
