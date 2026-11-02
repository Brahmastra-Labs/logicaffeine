//! A function header names its function the same way to every pass.
//!
//! `## To chime:` — no parameters — defines `chime`, and `native`/`private` may come in either
//! order before the name. The loader, the module assembler, and the scope checker each read headers
//! with their own copy of the parse; some kept the colon (`chime:`), some took `private` for the
//! name. A private parameterless procedure was then neither owner-mangled nor known to be private:
//! two modules' private `chime` collided, and an importer could call one. Every pass now reads a
//! header through one function.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::fs;
use std::path::Path;

use tempfile::tempdir;

use logicaffeine_compile::analysis::assemble_project;
use logicaffeine_compile::compile::{compile_project, enforce_project_scope};

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

fn compiled_project(entry: &Path) -> String {
    let output = compile_project(entry).expect("the project must compile");
    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "must compile and run.\nstderr: {}\n\nRust:\n{}",
        result.stderr, output.rust_code
    );
    result.stdout.trim().to_string()
}

#[test]
fn two_modules_each_keep_a_private_procedure_with_no_parameters() {
    let dir = tempdir().unwrap();
    for (module, word) in [("a", "from a"), ("b", "from b")] {
        let upper = module.to_uppercase();
        fs::write(
            dir.path().join(format!("{module}.lg")),
            format!("# {upper}\n\n## To private chime:\n    Show \"{word}\".\n\n## To {module}Hello:\n    Call chime.\n"),
        )
        .unwrap();
    }
    let main = dir.path().join("main.lg");
    fs::write(
        &main,
        "# App\n\nImport A and B.\n\n## Main\nCall aHello.\nCall bHello.\n",
    )
    .unwrap();

    assert_eq!(interpret_project(&main), "from a\nfrom b");
    assert_eq!(compiled_project(&main), "from a\nfrom b");
}

#[test]
fn an_importer_cannot_call_a_private_procedure_with_no_parameters() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("util.lg"),
        "# Util\n\n## To private chime:\n    Show \"u\".\n\n## To hello:\n    Call chime.\n",
    )
    .unwrap();
    let main = dir.path().join("main.lg");
    fs::write(&main, "# App\n\nImport Util.\n\n## Main\nCall chime.\n").unwrap();

    let err = enforce_project_scope(&main)
        .expect_err("a private procedure must not be callable from an importer");
    assert!(
        err.contains("chime"),
        "the error must name the procedure, got: {err}"
    );
    assert!(
        err.to_lowercase().contains("private"),
        "the error must say it is private, got: {err}"
    );
}

#[test]
fn the_owner_calls_its_private_procedure_with_no_parameters() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("util.lg"),
        "# Util\n\n## To private chime:\n    Show \"u\".\n\n## To hello:\n    Call chime.\n",
    )
    .unwrap();
    let main = dir.path().join("main.lg");
    fs::write(&main, "# App\n\nImport Util.\n\n## Main\nCall hello.\n").unwrap();

    assert_eq!(interpret_project(&main), "u");
    assert_eq!(compiled_project(&main), "u");
}

/// A procedure's body is checked against its OWN module's imports, whatever its header's form: the
/// scope checker finds the module that defines `aHello` by the name its header gives.
#[test]
fn a_procedure_with_no_parameters_is_scope_checked_in_its_own_module() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("a.lg"),
        "# A\n\n## To aHello:\n    Call notify.\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("b.lg"),
        "# B\n\n## To notify:\n    Show \"b\".\n",
    )
    .unwrap();
    let main = dir.path().join("main.lg");
    fs::write(&main, "# App\n\nImport A and B.\n\n## Main\nCall aHello.\n").unwrap();

    let err =
        enforce_project_scope(&main).expect_err("`a.lg` calls `notify` without importing `B`");
    assert!(
        err.contains("notify"),
        "the error must name the call, got: {err}"
    );
    assert!(
        err.contains("a.lg"),
        "the error must name the module whose body makes the call, got: {err}"
    );
}

#[test]
fn a_procedure_with_no_parameters_reaches_what_its_module_imports() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("a.lg"),
        "# A\n\nImport B.\n\n## To aHello:\n    Call notify.\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("b.lg"),
        "# B\n\n## To notify:\n    Show \"b\".\n",
    )
    .unwrap();
    let main = dir.path().join("main.lg");
    fs::write(&main, "# App\n\nImport A.\n\n## Main\nCall aHello.\n").unwrap();

    assert_eq!(interpret_project(&main), "b");
    assert_eq!(compiled_project(&main), "b");
}

/// The same scope check for a `private` function with parameters — the header's modifier is never
/// taken for its name.
#[test]
fn a_private_function_is_scope_checked_in_its_own_module() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("a.lg"),
        "# A\n\n## To private helper (n: Int) -> Int:\n    Return bTwice(n).\n\n## To aVal (n: Int) -> Int:\n    Return helper(n).\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("b.lg"),
        "# B\n\n## To bTwice (n: Int) -> Int:\n    Return n times 2.\n",
    )
    .unwrap();
    let main = dir.path().join("main.lg");
    fs::write(
        &main,
        "# App\n\nImport A and B.\n\n## Main\nShow aVal(4).\n",
    )
    .unwrap();

    let err = enforce_project_scope(&main)
        .expect_err("`a.lg`'s private `helper` calls `bTwice` without importing `B`");
    assert!(
        err.contains("bTwice"),
        "the error must name the call, got: {err}"
    );
    assert!(
        err.contains("a.lg"),
        "the error must name the module whose body makes the call, got: {err}"
    );
}
