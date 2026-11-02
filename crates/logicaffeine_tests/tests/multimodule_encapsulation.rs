//! Module ENCAPSULATION: a module may mark a function/type `private` so importers
//! cannot reference it (bare or qualified), while the defining module still uses
//! it internally and it still runs. Public-by-default — a module with no `private`
//! marker exports everything, exactly as before.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::fs;
use std::path::Path;

use tempfile::tempdir;

use logicaffeine_compile::analysis::assemble_project;
use logicaffeine_compile::compile::{compile_project, enforce_project_scope};

/// Interpret an assembled project as `largo run --interpret` does.
fn interpret_project(entry: &Path) -> String {
    enforce_project_scope(entry).expect("project scope check");
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

/// A library exposing PUBLIC `double`, which internally uses PRIVATE `twice`.
const UTIL_PRIVATE: &str = r#"# Util

## To private twice (n: Int) -> Int:
    Return n times 2.

## To double (n: Int) -> Int:
    Return twice(n).
"#;

fn write_project(dir: &Path, util: &str, main: &str) -> std::path::PathBuf {
    fs::write(dir.join("util.lg"), util).unwrap();
    let main_path = dir.join("main.lg");
    fs::write(&main_path, main).unwrap();
    main_path
}

/// The owner freely uses its own private helper — `double` calls `twice`, and the
/// program runs on both the interpreter and AOT.
#[test]
fn private_function_usable_by_owner() {
    let dir = tempdir().unwrap();
    let main = write_project(
        dir.path(),
        UTIL_PRIVATE,
        "# App\n\nImport Util.\n\n## Main\nShow double(21).\n",
    );

    assert_eq!(interpret_project(&main), "42");
    let output = compile_project(&main).expect("owner-uses-private must compile");
    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "must compile+run.\nstderr: {}",
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "42");
}

/// TRUE file-locality: two DIFFERENT modules may each declare their OWN private
/// `helper` (different bodies) without colliding — a private name is owner-local,
/// owner-mangled at assembly so the flat unit stays unambiguous.
#[test]
fn two_modules_can_each_have_a_private_helper() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("a.lg"),
        "# A\n\n## To private helper (n: Int) -> Int:\n    Return n plus 1.\n\n## To aVal (n: Int) -> Int:\n    Return helper(n).\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("b.lg"),
        "# B\n\n## To private helper (n: Int) -> Int:\n    Return n plus 2.\n\n## To bVal (n: Int) -> Int:\n    Return helper(n).\n",
    )
    .unwrap();
    let main = dir.path().join("main.lg");
    fs::write(
        &main,
        "# App\n\nImport A and B.\n\n## Main\nShow aVal(10) plus bVal(10).\n",
    )
    .unwrap();

    // aVal(10)=11 (A's helper +1), bVal(10)=12 (B's helper +2) → 23.
    assert_eq!(interpret_project(&main), "23");
    let output = compile_project(&main).expect("two private helpers must not collide");
    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "must compile+run.\nstderr: {}",
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "23");
}

/// A private function in a module whose FILE NAME is not a bare identifier
/// (hyphen, leading digit) must still mangle to a VALID identifier — the raw stem
/// `math-utils` would produce `math-utils__helper`, which the lexer rejects.
#[test]
fn private_helper_in_a_hyphenated_module() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("math-utils.lg"),
        "# MathUtils\n\n## To private helper (n: Int) -> Int:\n    Return n plus 1.\n\n## To compute (n: Int) -> Int:\n    Return helper(n).\n",
    )
    .unwrap();
    let main = dir.path().join("main.lg");
    fs::write(
        &main,
        "# App\n\nImport MathUtils from \"math-utils.lg\".\n\n## Main\nShow compute(41).\n",
    )
    .unwrap();

    assert_eq!(interpret_project(&main), "42");
    let output =
        compile_project(&main).expect("a hyphenated module with a private fn must compile");
    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "must compile+run.\nstderr: {}",
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "42");
}

/// A `private` name in one module must NOT poison the SAME name being PUBLIC in
/// another: importing the public one and using it is fine — the privacy gate only
/// fires for a name the file can't otherwise see.
#[test]
fn private_name_does_not_poison_a_public_name_elsewhere() {
    let dir = tempdir().unwrap();
    // A owns a PRIVATE `helper`; B exports a PUBLIC `helper`.
    fs::write(
        dir.path().join("a.lg"),
        "# A\n\n## To private helper (n: Int) -> Int:\n    Return n plus 1.\n\n## To aVal (n: Int) -> Int:\n    Return helper(n).\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("b.lg"),
        "# B\n\n## To helper (n: Int) -> Int:\n    Return n plus 100.\n",
    )
    .unwrap();
    let main = dir.path().join("main.lg");
    // main imports both and calls B's PUBLIC helper.
    fs::write(
        &main,
        "# App\n\nImport A and B.\n\n## Main\nShow helper(5) plus aVal(0).\n",
    )
    .unwrap();

    // B's helper(5)=105; A's aVal(0)=1 (via A's own private helper) → 106.
    assert_eq!(interpret_project(&main), "106");
    let output = compile_project(&main).expect("public name must not be poisoned by a private one");
    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "must compile+run.\nstderr: {}",
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "106");
}

/// The ENTRY may use its OWN private function — owner-mangling applies to the
/// entry's code too, and the owner is always allowed to reach its private names.
#[test]
fn entry_can_use_its_own_private_function() {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.lg");
    fs::write(
        &main,
        "# App\n\n## To private helper (n: Int) -> Int:\n    Return n plus 1.\n\n## Main\nShow helper(41).\n",
    )
    .unwrap();

    assert_eq!(interpret_project(&main), "42");
    let output = compile_project(&main).expect("entry using its own private fn must compile");
    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "must compile+run.\nstderr: {}",
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "42");
}

/// A selective import cannot reach a PRIVATE name — selecting it does not bypass
/// the privacy gate.
#[test]
fn selective_import_cannot_reach_a_private_name() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("util.lg"),
        "# Util\n\n## To private twice (n: Int) -> Int:\n    Return n times 2.\n\n## To double (n: Int) -> Int:\n    Return twice(n).\n",
    )
    .unwrap();
    let main = dir.path().join("main.lg");
    fs::write(
        &main,
        "# App\n\nImport twice from Util.\n\n## Main\nShow twice(21).\n",
    )
    .unwrap();

    let err = enforce_project_scope(&main)
        .expect_err("a private name must not be selectively importable");
    assert!(
        err.to_lowercase().contains("private"),
        "selecting a private name must be rejected, got: {err}"
    );
}

/// A private function CALLED inside a string INTERPOLATION hole is owner-mangled
/// too — the call `{twice(n)}` must resolve to the module's real (mangled) private
/// function, not dangle.
#[test]
fn private_function_called_in_a_string_interpolation() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("util.lg"),
        "# Util\n\n## To private twice (n: Int) -> Int:\n    Return n times 2.\n\n## To label (n: Int) -> Text:\n    Return \"result: {twice(n)}\".\n",
    )
    .unwrap();
    let main = dir.path().join("main.lg");
    fs::write(&main, "# App\n\nImport Util.\n\n## Main\nShow label(21).\n").unwrap();

    // twice(21) = 42, inside the interpolation.
    assert_eq!(interpret_project(&main), "result: 42");
}

/// A private function whose NAME also appears inside a STRING LITERAL must not have
/// the string content mangled — only real identifier references are owner-mangled.
#[test]
fn private_name_inside_a_string_is_not_mangled() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("util.lg"),
        "# Util\n\n## To private tag (n: Int) -> Int:\n    Return n.\n\n## To label (n: Int) -> Text:\n    Return \"tag\".\n",
    )
    .unwrap();
    let main = dir.path().join("main.lg");
    fs::write(&main, "# App\n\nImport Util.\n\n## Main\nShow label(1).\n").unwrap();

    // The string is the literal word `tag`, NOT the mangled `util__tag`.
    assert_eq!(interpret_project(&main), "tag");
}

/// An importer that references a private name by its BARE spelling is rejected,
/// and the error names the symbol and says it is private.
#[test]
fn private_function_rejected_bare() {
    let dir = tempdir().unwrap();
    let main = write_project(
        dir.path(),
        UTIL_PRIVATE,
        "# App\n\nImport Util.\n\n## Main\nShow twice(21).\n",
    );

    let err = enforce_project_scope(&main).expect_err("a private reference must be rejected");
    assert!(
        err.contains("twice"),
        "error must name the private symbol, got: {err}"
    );
    assert!(
        err.to_lowercase().contains("private"),
        "error must say it is private, got: {err}"
    );
}

/// The qualified escape hatch does NOT bypass privacy — `Util::twice` is rejected
/// just like the bare form.
#[test]
fn private_function_rejected_qualified() {
    let dir = tempdir().unwrap();
    let main = write_project(
        dir.path(),
        UTIL_PRIVATE,
        "# App\n\nImport Util.\n\n## Main\nShow Util::twice(21).\n",
    );

    let err =
        enforce_project_scope(&main).expect_err("a qualified private reference must be rejected");
    assert!(
        err.to_lowercase().contains("private"),
        "qualified access must not bypass privacy, got: {err}"
    );
}

/// Public-by-default regression: a module with NO `private` marker exports every
/// definition, and a normal cross-file call still works.
#[test]
fn public_by_default_still_importable() {
    let dir = tempdir().unwrap();
    let util = "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n";
    let main = write_project(
        dir.path(),
        util,
        "# App\n\nImport Util.\n\n## Main\nShow double(21).\n",
    );

    enforce_project_scope(&main).expect("public function must be in scope");
    assert_eq!(interpret_project(&main), "42");
}

/// A PRIVATE type cannot be constructed by an importer (a type reference is gated
/// just like a private function call), but the module's public type still can.
#[test]
fn private_type_rejected_from_importer() {
    let dir = tempdir().unwrap();
    // `Secret` is private; `Point` is public.
    let geo = "# Geometry\n\n## A private Secret has:\n    A code: Int.\n\n## A Point has:\n    An x: Int.\n    A y: Int.\n";
    fs::write(dir.path().join("geometry.md"), geo).unwrap();
    let main_path = dir.path().join("main.lg");

    // Public `Point` works.
    fs::write(&main_path, "# App\n\nImport Geometry.\n\n## Main\nLet p be a new Point with x 3 and y 4.\nShow p's x plus p's y.\n").unwrap();
    assert_eq!(interpret_project(&main_path), "7");

    // Private `Secret` is rejected by scoping, with a clear message.
    fs::write(&main_path, "# App\n\nImport Geometry.\n\n## Main\nLet s be a new Secret with code 9.\nShow s's code.\n").unwrap();
    let err = enforce_project_scope(&main_path)
        .expect_err("a private type must not be constructible from an importer");
    assert!(
        err.contains("Secret"),
        "error must name the private type, got: {err}"
    );
    assert!(
        err.to_lowercase().contains("private"),
        "error must say it is private, got: {err}"
    );
}
