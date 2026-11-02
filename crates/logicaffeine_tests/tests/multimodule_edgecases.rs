//! Edge-case + security locks for module resolution/assembly: path-escape
//! rejection, deep nesting, `.lg`/`.md` precedence, empty/prose-only modules, and
//! importing one file under two aliases.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::Path;

use tempfile::tempdir;

use logicaffeine_compile::analysis::assemble_project;
use logicaffeine_compile::compile::enforce_project_scope;

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

fn assemble_err(entry: &Path) -> String {
    match assemble_project(entry) {
        Ok(_) => panic!("expected an assembly error"),
        Err(e) => e,
    }
}

/// SECURITY: an import that resolves OUTSIDE the project root is rejected (before
/// the file is even read), fail-closed.
#[test]
fn import_escaping_project_root_is_rejected() {
    let dir = tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(&project).unwrap();
    // A real file OUTSIDE the project root.
    fs::write(
        dir.path().join("secret.lg"),
        "# Secret\n\n## To leak (n: Int) -> Int:\n    Return n.\n",
    )
    .unwrap();
    let main = project.join("main.lg");
    fs::write(
        &main,
        "# App\n\nImport Secret from \"../secret.lg\".\n\n## Main\nShow leak(1).\n",
    )
    .unwrap();

    let err = assemble_err(&main);
    assert!(
        err.to_lowercase().contains("escape"),
        "an import escaping the project root must be rejected, got: {err}"
    );
}

/// Deep multi-directory nesting (4+ levels) resolves.
#[test]
fn deep_directory_nesting_resolves() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("a/b/c/d")).unwrap();
    fs::write(
        dir.path().join("a/b/c/d/leaf.lg"),
        "# Leaf\n\n## To leaf (n: Int) -> Int:\n    Return n plus 1.\n",
    )
    .unwrap();
    let main = dir.path().join("main.lg");
    fs::write(
        &main,
        "# App\n\nImport Leaf from \"a/b/c/d/leaf.lg\".\n\n## Main\nShow leaf(41).\n",
    )
    .unwrap();
    assert_eq!(interpret_project(&main), "42");
}

/// When a stem has BOTH `.lg` and `.md`, `.lg` wins (documented precedence).
#[test]
fn lg_wins_over_md_same_stem() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("foo.lg"),
        "# Foo\n\n## To foo (n: Int) -> Int:\n    Return n plus 1.\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("foo.md"),
        "# Foo\n\n## To foo (n: Int) -> Int:\n    Return n plus 100.\n",
    )
    .unwrap();
    let main = dir.path().join("main.lg");
    fs::write(&main, "# App\n\nImport Foo.\n\n## Main\nShow foo(0).\n").unwrap();
    assert_eq!(interpret_project(&main), "1"); // `.lg` (plus 1), not `.md` (plus 100)
}

/// A prose-only imported module (no code) is a harmless no-op — the entry runs.
#[test]
fn prose_only_imported_module_is_noop() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("doc.lg"),
        "# Doc\n\nJust documentation prose, no code here.\n",
    )
    .unwrap();
    let main = dir.path().join("main.lg");
    fs::write(&main, "# App\n\nImport Doc.\n\n## Main\nShow 42.\n").unwrap();
    assert_eq!(interpret_project(&main), "42");
}

/// Importing the SAME file under two aliases dedups (one set of definitions) and
/// runs — no double-emit, no collision.
#[test]
fn two_aliases_same_file_dedups_and_runs() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("util.lg"),
        "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n",
    )
    .unwrap();
    let main = dir.path().join("main.lg");
    fs::write(
        &main,
        "# App\n\nImport Util.\nImport Helper from \"util.lg\".\n\n## Main\nShow double(21).\n",
    )
    .unwrap();
    assert_eq!(interpret_project(&main), "42");
}
