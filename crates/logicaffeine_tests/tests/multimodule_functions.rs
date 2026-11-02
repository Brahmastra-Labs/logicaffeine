//! Cross-module FUNCTION calls (the multi-file blocker).
//!
//! Sibling of `multimodule_types.rs`: that suite proved imported TYPES are shared
//! (`Geometry::Point` → `struct Geometry__Point`); this one proves imported
//! FUNCTIONS are callable — by their natural BARE name (`double(21)`) and by the
//! qualified escape hatch (`Util::double(21)`) — across files and directories, on
//! every execution tier, with hard-error diagnostics for ambiguous bare names.
//!
//! The whole `multimodule_types.rs` battery must stay green alongside these.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::fs;
use std::path::Path;

use tempfile::tempdir;

use logicaffeine_compile::analysis::assemble_project;
use logicaffeine_compile::compile::{compile_project, compile_to_wasm};

/// Interpret an assembled project exactly as `largo run --interpret` does:
/// assemble the whole project into one source, then tree-walk it. Returns the
/// joined output lines (panics on an engine error, surfacing it).
fn interpret_project(entry: &Path) -> String {
    // Per-file scoping is enforced on the interpreter tier too (parity with build).
    logicaffeine_compile::compile::enforce_project_scope(entry).expect("project scope check");
    let assembled = assemble_project(entry).expect("assemble project");
    let result = futures::executor::block_on(logicaffeine_compile::interpret_for_ui_with_args(
        &assembled.source,
        &["main".to_string()],
    ));
    assert!(
        result.error.is_none(),
        "interpreter error: {:?}",
        result.error
    );
    result.lines.join("\n")
}

/// A library module defining a single function `double` and NOTHING else — no
/// `## Main`, just a `## To` block. The importer must be able to call it bare.
const UTIL_LG: &str = r#"# Util

## To double (n: Int):
    Return n times 2.
"#;

/// Write the two-file repro (`util.lg` + `main.lg`) into `dir`; return the entry.
fn write_double_project(dir: &Path, main_src: &str) -> std::path::PathBuf {
    fs::write(dir.join("util.lg"), UTIL_LG).unwrap();
    let main_path = dir.join("main.lg");
    fs::write(&main_path, main_src).unwrap();
    main_path
}

const MAIN_BARE: &str = r#"# App

Import Util.

## Main
Show double(21).
"#;

const MAIN_QUALIFIED: &str = r#"# App

Import Util.

## Main
Show Util::double(21).
"#;

const GEOMETRY_MD: &str = r#"# Geometry

## A Point has:
    An x: Int.
    A y: Int.
"#;

/// Single-emit: an imported type used by its bare name produces exactly ONE Rust
/// struct (the canonical namespaced one), with a `type` alias for the bare
/// spelling — not a dead duplicate struct.
#[test]
fn multifile_imported_type_single_struct() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("geometry.md"), GEOMETRY_MD).unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        "# App\n\nImport Geometry.\n\n## Main\nLet p be a new Point with x 3 and y 4.\nShow p's x plus p's y.\n",
    )
    .unwrap();

    let output = compile_project(&main_path).expect("bare-type program should compile");
    let rust = &output.rust_code;
    assert!(
        rust.contains("struct Geometry__Point"),
        "canonical struct must be emitted.\n{rust}"
    );
    assert!(
        !rust.contains("struct Point"),
        "no dead duplicate bare struct — the bare spelling is a `type` alias.\n{rust}"
    );

    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "single-emit program must compile+run.\nstderr: {}",
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "7");
}

/// Milestone 4 — a BARE reference to an imported TYPE. The entry imports
/// Geometry and constructs `a new Point` by its natural bare name, reads its
/// fields, and prints `7`. The emitted Rust must reference the canonical
/// sanitized struct (`Geometry__Point`) and must NOT leak a raw `Geometry::`,
/// nor emit two distinct structs for the one type.
#[test]
fn multifile_bare_type_reference_interpreter() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("geometry.md"), GEOMETRY_MD).unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        r#"# App

Import Geometry.

## Main
Let p be a new Point with x 3 and y 4.
Show p's x plus p's y.
"#,
    )
    .unwrap();

    assert_eq!(interpret_project(&main_path).trim(), "7");
}

#[test]
fn multifile_bare_type_reference_aot_runs() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("geometry.md"), GEOMETRY_MD).unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        r#"# App

Import Geometry.

## Main
Let p be a new Point with x 3 and y 4.
Show p's x plus p's y.
"#,
    )
    .unwrap();

    let output = compile_project(&main_path).expect("bare-type program should compile");
    let rust = &output.rust_code;
    assert!(
        !rust.contains("Geometry::Point"),
        "no raw namespaced form may leak into Rust.\n{rust}"
    );

    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "bare-type program must compile+run.\nstderr: {}\n\nRust:\n{}",
        result.stderr, result.rust_code
    );
    assert_eq!(result.stdout.trim(), "7");
}

/// A `::` inside a STRING literal is content, never a qualified-call rewrite —
/// even when the tail (`double`) matches a defined function. Locks the token
/// resolver against corrupting string data.
#[test]
fn multifile_double_colon_in_string_is_literal() {
    let dir = tempdir().unwrap();
    let main_path = write_double_project(
        dir.path(),
        "# App\n\nImport Util.\n\n## Main\nShow \"x::double\".\n",
    );
    assert_eq!(interpret_project(&main_path).trim(), "x::double");
}

/// A QUALIFIED escape-hatch call (`Util::double`) that lives INSIDE a string
/// interpolation hole resolves exactly like a bare `{double(21)}` would — the
/// hole is a real expression, so the `Alias::` prefix is stripped before the
/// interpolation is re-parsed. (A `::` in the surrounding literal text stays
/// literal — see `multifile_double_colon_in_string_is_literal`.)
#[test]
fn multifile_qualified_call_in_string_interpolation() {
    let dir = tempdir().unwrap();
    let main_path = write_double_project(
        dir.path(),
        "# App\n\nImport Util.\n\n## Main\nShow \"double is {Util::double(21)}\".\n",
    );
    assert_eq!(interpret_project(&main_path).trim(), "double is 42");
}

/// Adversarial: one interpolated string mixing a literal `::` (surrounding text,
/// left alone), a QUALIFIED hole (`{Util::double(10)}` → resolves), and a BARE
/// hole (`{double(1)}`). Only the two holes are rewritten; the `a::b` prose is
/// untouched — the hole scanner is parser-faithful and never leaks into literal
/// text.
#[test]
fn multifile_mixed_literal_and_qualified_interpolation() {
    let dir = tempdir().unwrap();
    let main_path = write_double_project(
        dir.path(),
        "# App\n\nImport Util.\n\n## Main\nShow \"a::b {Util::double(10)} and {double(1)}\".\n",
    );
    assert_eq!(interpret_project(&main_path).trim(), "a::b 20 and 2");
}

/// Milestone 10 — the WASM tier. The assembled multi-file program compiles to a
/// valid `.wasm` module through the direct wasm backend, proving imported
/// functions reach the wasm tier (which shares the `with_parsed_program` seam).
#[test]
fn multifile_wasm_tier_compiles() {
    let dir = tempdir().unwrap();
    let main_path = write_double_project(dir.path(), MAIN_BARE);
    let assembled = assemble_project(&main_path).expect("assemble project");
    let wasm =
        compile_to_wasm(&assembled.source).expect("multi-file program should compile to wasm");
    assert_eq!(
        &wasm[0..4],
        b"\0asm",
        "output must begin with the wasm magic"
    );
    assert!(wasm.len() > 8, "a real module is more than a header");
}

/// A syntax error inside an IMPORTED module must report against THAT file, not
/// an offset into the merged source the user never wrote.
#[test]
fn multifile_syntax_error_names_the_imported_file() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("broken.lg"),
        "# Broken\n\n## To helper (n: Int:\n    Return n.\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        "# App\n\nImport Broken.\n\n## Main\nShow helper(1).\n",
    )
    .unwrap();

    let err = compile_project(&main_path).expect_err("a syntax error must fail compilation");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("broken.lg"),
        "the error must name the imported file that has the syntax error.\n{msg}"
    );
}

/// Per-file scoping — no transitive leakage. `main.lg` imports `Left`, and
/// `Left` imports `Base`, but `main.lg` never imports `Base`. Calling a `Base`
/// function directly from `main.lg` must be a hard error (you can only use what
/// you import), naming the symbol and the module that defines it.
#[test]
fn multifile_scope_rejects_transitive_leakage() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("base.lg"),
        "# Base\n\n## To baseSecret (n: Int):\n    Return n + 100.\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("left.lg"),
        "# Left\n\nImport Base.\n\n## To leftVal (n: Int):\n    Return baseSecret(n).\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    // main imports Left (which imports Base) but does NOT import Base itself.
    fs::write(
        &main_path,
        "# App\n\nImport Left.\n\n## Main\nShow baseSecret(5).\n",
    )
    .unwrap();

    let err = compile_project(&main_path)
        .expect_err("calling a non-imported module's function must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("baseSecret"),
        "the error must name the out-of-scope symbol.\n{msg}"
    );
    assert!(
        msg.contains("base.lg") || msg.contains("Base"),
        "the error must point at the module that defines it.\n{msg}"
    );
}

/// Interpreter-tier scoping parity: the check `largo run --interpret` runs
/// (`enforce_project_scope`) rejects the same transitive leak `largo build` does,
/// naming the out-of-scope symbol — so a program can never "run on interpret but
/// fail on build".
#[test]
fn multifile_scope_rejected_on_interpret_tier() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("base.lg"),
        "# Base\n\n## To baseSecret (n: Int):\n    Return n + 100.\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("left.lg"),
        "# Left\n\nImport Base.\n\n## To leftVal (n: Int):\n    Return baseSecret(n).\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        "# App\n\nImport Left.\n\n## Main\nShow baseSecret(5).\n",
    )
    .unwrap();

    let err = logicaffeine_compile::compile::enforce_project_scope(&main_path)
        .expect_err("interpreter tier must reject the leak too");
    assert!(
        err.contains("baseSecret"),
        "must name the out-of-scope symbol.\n{err}"
    );
    assert!(
        err.contains("base.lg") || err.contains("Base"),
        "must point at the defining module.\n{err}"
    );
}

/// The same symbol IS reachable through a proper import chain: `Left` legally
/// calls `Base`'s function because `Left` imports `Base`. Locks that scoping
/// permits what you import (no false positives on a valid program).
#[test]
fn multifile_scope_allows_imported_chain() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("base.lg"),
        "# Base\n\n## To baseVal (n: Int):\n    Return n + 100.\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("left.lg"),
        "# Left\n\nImport Base.\n\n## To leftVal (n: Int):\n    Return baseVal(n).\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        "# App\n\nImport Left.\n\n## Main\nShow leftVal(5).\n",
    )
    .unwrap();

    assert_eq!(interpret_project(&main_path).trim(), "105");
    let output = compile_project(&main_path).expect("a properly-imported chain must compile");
    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "must compile+run.\nstderr: {}",
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "105");
}

/// Milestone 5 — an imported function BODY references a TYPE from its own
/// module. `originX(p)` (defined in Geometry, over Geometry's `Point`) is called
/// from the entry on a bare-constructed `Point`; the whole thing resolves in one
/// compilation unit and prints the x field.
#[test]
fn multifile_crossfile_type_in_imported_fn_body() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("geometry.md"),
        "# Geometry\n\n## A Point has:\n    An x: Int.\n    A y: Int.\n\n## To originX (p: Point):\n    Return p's x.\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        "# App\n\nImport Geometry.\n\n## Main\nLet p be a new Point with x 9 and y 4.\nShow originX(p).\n",
    )
    .unwrap();

    assert_eq!(interpret_project(&main_path).trim(), "9");

    let output = compile_project(&main_path).expect("cross-file type in fn body should compile");
    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "must compile+run.\nstderr: {}",
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "9");
}

/// Content-hash identity: two modules that define the SAME name with byte-identical
/// bodies are the same definition — importing both auto-merges them (no collision).
/// Only genuinely-different same-named defs are a hard error (locked separately).
#[test]
fn multifile_identical_defs_auto_merge() {
    let dir = tempdir().unwrap();
    let shared = "## To shared (n: Int):\n    Return n plus 1.\n";
    fs::write(dir.path().join("a.lg"), format!("# A\n\n{shared}")).unwrap();
    fs::write(dir.path().join("b.lg"), format!("# B\n\n{shared}")).unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        "# App\n\nImport A and B.\n\n## Main\nShow shared(41).\n",
    )
    .unwrap();

    let output =
        compile_project(&main_path).expect("identical same-named defs must merge, not collide");
    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "merged program must compile+run.\nstderr: {}",
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "42");
}

/// Milestone 6 — a bare-name COLLISION across two modules is a hard error that
/// names both files. Never silent shadowing, never a cryptic rustc
/// duplicate-definition. Both `a.lg` and `b.lg` define `parse`.
#[test]
fn multifile_collision_is_hard_error() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("a.lg"),
        "# A\n\n## To parse (n: Int):\n    Return n + 1.\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("b.lg"),
        "# B\n\n## To parse (n: Int):\n    Return n + 2.\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        "# App\n\nImport A.\nImport B.\n\n## Main\nShow parse(10).\n",
    )
    .unwrap();

    let err = compile_project(&main_path).expect_err("a bare-name collision must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("parse"),
        "the error must name the colliding symbol.\n{msg}"
    );
    assert!(
        msg.contains("a.lg") && msg.contains("b.lg"),
        "the error must name BOTH defining files.\n{msg}"
    );
}

/// Milestone 7 — a legal import CYCLE with cross-file mutual recursion. `Even`
/// imports `Odd` and vice versa; `pingpong`/`pong` call each other across the
/// files. Assembly must not infinite-recurse, and the mutual recursion must run.
#[test]
fn multifile_import_cycle_mutual_recursion() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("even.lg"),
        "# Even\n\nImport Odd.\n\n## To pingpong (n: Int):\n    If n is less than 1:\n        Return 0.\n    Return pong(n - 1).\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("odd.lg"),
        "# Odd\n\nImport Even.\n\n## To pong (n: Int):\n    If n is less than 1:\n        Return 1.\n    Return pingpong(n - 1).\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        "# App\n\nImport Even.\n\n## Main\nShow pingpong(4).\n",
    )
    .unwrap();

    assert_eq!(interpret_project(&main_path).trim(), "0");

    let output = compile_project(&main_path).expect("mutually-recursive cycle should compile");
    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "must compile+run.\nstderr: {}",
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "0");
}

/// Milestone 8 — a DIAMOND import. `Base` is reached via both `Left` and `Right`;
/// its `baseVal` must be contributed exactly once (else a duplicate `fn` would
/// break rustc). Transitively-imported functions compose across the diamond.
#[test]
fn multifile_diamond_single_contribution() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("base.lg"),
        "# Base\n\n## To baseVal (n: Int):\n    Return n + 40.\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("left.lg"),
        "# Left\n\nImport Base.\n\n## To leftVal (n: Int):\n    Return baseVal(n).\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("right.lg"),
        "# Right\n\nImport Base.\n\n## To rightVal (n: Int):\n    Return baseVal(n).\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        "# App\n\nImport Left.\nImport Right.\n\n## Main\nLet total be leftVal(1) plus rightVal(1).\nShow total.\n",
    )
    .unwrap();

    assert_eq!(interpret_project(&main_path).trim(), "82");

    let output = compile_project(&main_path).expect("diamond import should compile");
    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "must compile+run (no duplicate fn).\nstderr: {}",
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "82");
}

/// Milestone 11 — multi-directory. An explicit `from` path pulls a module out of
/// a subdirectory; the imported function is callable across the directory
/// boundary on both tiers.
#[test]
fn multifile_import_from_subdirectory() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("lib")).unwrap();
    fs::write(
        dir.path().join("lib/mathutil.lg"),
        "# MathUtil\n\n## To triple (n: Int):\n    Return n times 3.\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        "# App\n\nImport MathUtil from \"lib/mathutil.lg\".\n\n## Main\nShow triple(7).\n",
    )
    .unwrap();

    assert_eq!(interpret_project(&main_path).trim(), "21");

    let output = compile_project(&main_path).expect("subdirectory import should compile");
    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "subdirectory-import program must compile+run.\nstderr: {}",
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "21");
}

/// Milestone 1 — the issue's exact repro through the interpreter tier. A bare
/// call to a function defined in an imported module prints `42`.
#[test]
fn multifile_bare_function_double_21_interpreter() {
    let dir = tempdir().unwrap();
    let main_path = write_double_project(dir.path(), MAIN_BARE);
    assert_eq!(interpret_project(&main_path).trim(), "42");
}

/// Milestone 2 — the issue's exact repro through the real AOT path. A function
/// defined in an imported module is called BARE from the entry's Main; the
/// emitted Rust must define `fn double`, must not leak a raw `::`, and must
/// compile+run under rustc to print `42`.
#[test]
fn multifile_bare_function_double_21_aot_runs() {
    let dir = tempdir().unwrap();
    let main_path = write_double_project(dir.path(), MAIN_BARE);

    let output = compile_project(&main_path)
        .expect("multi-file program with an imported function should compile");
    let rust = &output.rust_code;
    assert!(
        rust.contains("fn double"),
        "the imported function must reach codegen as `fn double`.\n{rust}"
    );

    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "imported-function program must compile+run under rustc.\nstderr: {}\n\nRust:\n{}",
        result.stderr, result.rust_code
    );
    assert_eq!(
        result.stdout.trim(),
        "42",
        "double(21) across two files must print 42.\nRust:\n{}",
        result.rust_code
    );
}

/// Milestone 3 — the QUALIFIED escape hatch. `Util::double(21)` resolves to the
/// same bare `double` on both the interpreter and AOT; the emitted Rust names the
/// bare `fn double` and never leaks a raw or sanitized `Util::`/`Util__` function
/// path (the qualifier is a compile-time spelling, not a Rust symbol).
#[test]
fn multifile_qualified_function_call_interpreter() {
    let dir = tempdir().unwrap();
    let main_path = write_double_project(dir.path(), MAIN_QUALIFIED);
    assert_eq!(interpret_project(&main_path).trim(), "42");
}

#[test]
fn multifile_qualified_function_call_aot_runs() {
    let dir = tempdir().unwrap();
    let main_path = write_double_project(dir.path(), MAIN_QUALIFIED);

    let output = compile_project(&main_path).expect("qualified cross-module call should compile");
    let rust = &output.rust_code;
    assert!(
        !rust.contains("Util::double") && !rust.contains("Util__double"),
        "the qualifier must resolve away — no `Util::`/`Util__` function symbol.\n{rust}"
    );

    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "qualified-call program must compile+run.\nstderr: {}\n\nRust:\n{}",
        result.stderr, result.rust_code
    );
    assert_eq!(result.stdout.trim(), "42");
}
