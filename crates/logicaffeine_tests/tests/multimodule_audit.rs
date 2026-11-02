//! Adversarial pit-of-success locks for the multi-file / scoping / codegen work.
//!
//! Each test pins an INVARIANT that must hold for the system to be correct — the
//! kind of thing a subtle refactor could silently break. They exercise the real
//! `compile_project` → rustc → run path, and the interpreter, so a regression
//! surfaces loudly.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::fs;
use std::path::Path;

use tempfile::tempdir;

use logicaffeine_compile::analysis::assemble_project;
use logicaffeine_compile::compile::compile_project;

/// Run an assembled project through the interpreter (enforcing scoping first,
/// exactly as `largo run --interpret` does); returns the joined output.
fn interpret(entry: &Path) -> String {
    logicaffeine_compile::compile::enforce_project_scope(entry).expect("scope");
    let assembled = assemble_project(entry).expect("assemble");
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

/// Compile a project and run it under rustc; return (ok, trimmed stdout, stderr).
fn build_and_run(entry: &Path) -> (bool, String, String) {
    let output = compile_project(entry).expect("compile_project should succeed");
    let r = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    (r.success, r.stdout.trim().to_string(), r.stderr)
}

const GEOMETRY: &str = "# Geometry\n\n## A Point has:\n    An x: Int.\n    A y: Int.\n";

/// INVARIANT: a value constructed by its BARE name and a parameter written with
/// the QUALIFIED name are ONE Rust type — the single-emit `type` alias makes the
/// spellings interchangeable, so rustc accepts the call.
#[test]
fn audit_bare_and_qualified_type_are_one_rust_type() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("geometry.md"), GEOMETRY).unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        "# App\n\nImport Geometry.\n\n## To sumq (p: Geometry::Point) -> Int:\n    Return p's x plus p's y.\n\n## Main\nLet p be a new Point with x 3 and y 4.\nShow sumq(p).\n",
    )
    .unwrap();
    let (ok, out, err) = build_and_run(&main_path);
    assert!(
        ok,
        "a bare `Point` value must satisfy a `Geometry::Point` param.\nstderr: {err}"
    );
    assert_eq!(out, "7");
}

/// INVARIANT: content-identity ignores formatting — two modules defining the same
/// function with DIFFERENT indentation but identical content still auto-merge.
#[test]
fn audit_content_hash_ignores_indentation() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("a.lg"),
        "# A\n\n## To shared (n: Int) -> Int:\n    Return n plus 1.\n",
    )
    .unwrap();
    // Same definition, indented differently.
    fs::write(
        dir.path().join("b.lg"),
        "# B\n\n## To shared (n: Int) -> Int:\n        Return n plus 1.\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        "# App\n\nImport A and B.\n\n## Main\nShow shared(41).\n",
    )
    .unwrap();
    let (ok, out, err) = build_and_run(&main_path);
    assert!(
        ok,
        "identical-content defs (differing only in indentation) must merge.\nstderr: {err}"
    );
    assert_eq!(out, "42");
}

/// INVARIANT: a deep transitive import chain (four levels: A→B→C→D) assembles
/// once, scopes correctly (each file imports the next it uses), and runs. Names
/// deliberately avoid the articles `a`/`an`/`the`, which are grammar, not
/// identifiers.
#[test]
fn audit_deep_transitive_import_chain() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("d.lg"),
        "# D\n\n## To deltaOf (n: Int) -> Int:\n    Return n plus 1.\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("c.lg"),
        "# C\n\nImport D.\n\n## To gammaOf (n: Int) -> Int:\n    Return deltaOf(n) plus 10.\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("b.lg"),
        "# B\n\nImport C.\n\n## To betaOf (n: Int) -> Int:\n    Return gammaOf(n) plus 100.\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("a.lg"),
        "# A\n\nImport B.\n\n## To alphaOf (n: Int) -> Int:\n    Return betaOf(n) plus 1000.\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        "# App\n\nImport A.\n\n## Main\nShow alphaOf(0).\n",
    )
    .unwrap();

    assert_eq!(interpret(&main_path), "1111");
    let (ok, out, err) = build_and_run(&main_path);
    assert!(ok, "deep chain must compile.\nstderr: {err}");
    assert_eq!(out, "1111");
}

/// INVARIANT: an OMITTED return type that infers to Bool codegens `-> bool`, not
/// a wrong `i64` fallback.
#[test]
fn audit_omitted_return_bool() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("util.lg"),
        "# Util\n\n## To isPositive (n: Int):\n    Return n is greater than 0.\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        "# App\n\nImport Util.\n\n## Main\nShow isPositive(5).\n",
    )
    .unwrap();
    let (ok, out, err) = build_and_run(&main_path);
    assert!(
        ok,
        "omitted Bool return must infer `-> bool`.\nstderr: {err}"
    );
    assert_eq!(out, "true");
}

/// INVARIANT: an OMITTED return type that infers to Text codegens `-> String`.
#[test]
fn audit_omitted_return_text() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("util.lg"),
        "# Util\n\n## To label (n: Int):\n    Return \"positive\".\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        "# App\n\nImport Util.\n\n## Main\nShow label(1).\n",
    )
    .unwrap();
    let (ok, out, err) = build_and_run(&main_path);
    assert!(
        ok,
        "omitted Text return must infer `-> String`.\nstderr: {err}"
    );
    assert_eq!(out, "positive");
}

/// INVARIANT: rename-on-import and the qualified escape hatch compose — `Import
/// Util as U.` then `U::double(...)` resolves to the merged bare function.
#[test]
fn audit_rename_import_plus_qualified_call() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("util.lg"),
        "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        "# App\n\nImport Util as U.\n\n## Main\nShow U::double(21).\n",
    )
    .unwrap();

    assert_eq!(interpret(&main_path), "42");
    let (ok, out, err) = build_and_run(&main_path);
    assert!(ok, "rename + qualified call must resolve.\nstderr: {err}");
    assert_eq!(out, "42");
}

/// INVARIANT (soundness — no false negative): a leak through a statement other
/// than a plain `Show`/`Let` is still caught. `main` reaches `leaf`'s function
/// through a `Sleep` operand WITHOUT importing `leaf` (only `mid` does); the
/// per-file scope check must flag it. Locks the completeness of the statement
/// walker — a statement dropped from `collect_calls_from_stmt` would hide this.
#[test]
fn audit_scope_catches_leak_through_a_non_show_statement() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("leaf.lg"),
        "# Leaf\n\n## To leafDelay (n: Int) -> Int:\n    Return n.\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("mid.lg"),
        "# Mid\n\nImport Leaf.\n\n## To midFn (n: Int) -> Int:\n    Return leafDelay(n).\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    // main imports Mid (so leaf is transitively in the unit) but calls leafDelay
    // through `Sleep`, which it never imported.
    fs::write(
        &main_path,
        "# App\n\nImport Mid.\n\n## Main\nSleep leafDelay(5).\n",
    )
    .unwrap();

    let err = logicaffeine_compile::compile::enforce_project_scope(&main_path)
        .expect_err("a leak through `Sleep` must be caught by scoping");
    assert!(
        err.contains("leafDelay"),
        "the scope error must name the leaked function, got: {err}"
    );
}

/// INVARIANT (no false positive): a realistic valid multi-file program — mutual
/// recursion across files, calls to stdlib/builtins, and its own helpers — is NOT
/// wrongly rejected by the scope check.
#[test]
fn audit_scope_no_false_positive_on_valid_program() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("math.lg"),
        "# Math\n\n## To square (n: Int) -> Int:\n    Return n times n.\n\n## To sumTo (n: Int) -> Int:\n    If n is less than 1:\n        Return 0.\n    Return n plus sumTo(n minus 1).\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        "# App\n\nImport Math.\n\n## To describe (n: Int) -> Int:\n    Return square(n) plus sumTo(n).\n\n## Main\nShow describe(3).\n",
    )
    .unwrap();

    // square(3)=9, sumTo(3)=6 → 15.
    assert_eq!(interpret(&main_path), "15");
    let (ok, out, err) = build_and_run(&main_path);
    assert!(
        ok,
        "a valid program must NOT be rejected by scoping.\nstderr: {err}"
    );
    assert_eq!(out, "15");
}
