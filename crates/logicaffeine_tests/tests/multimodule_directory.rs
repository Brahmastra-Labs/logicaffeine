//! Directory / barrel modules: `Import Lexer.` resolves a `lexer/` directory via
//! a conventional entry file (`lexer/lexer.lg` preferred, then `lexer/mod.lg`), so
//! a whole subsystem is one import. A real file beside the directory always wins.

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

const TOKENIZE_PLUS_1: &str = "# Lexer\n\n## To tokenize (n: Int) -> Int:\n    Return n plus 1.\n";

/// `Import Lexer.` resolves `lexer/lexer.lg` (the self-descriptive convention).
#[test]
fn directory_module_via_dirname_entry() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("lexer")).unwrap();
    fs::write(dir.path().join("lexer/lexer.lg"), TOKENIZE_PLUS_1).unwrap();
    let main = dir.path().join("main.lg");
    fs::write(
        &main,
        "# App\n\nImport Lexer.\n\n## Main\nShow tokenize(41).\n",
    )
    .unwrap();
    assert_eq!(interpret_project(&main), "42");
}

/// `Import Lexer.` also resolves `lexer/mod.lg` (the barrel convention).
#[test]
fn directory_module_via_mod_convention() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("lexer")).unwrap();
    fs::write(dir.path().join("lexer/mod.lg"), TOKENIZE_PLUS_1).unwrap();
    let main = dir.path().join("main.lg");
    fs::write(
        &main,
        "# App\n\nImport Lexer.\n\n## Main\nShow tokenize(41).\n",
    )
    .unwrap();
    assert_eq!(interpret_project(&main), "42");
}

/// A real `lexer.lg` FILE beside a `lexer/` directory wins — file resolution is
/// tried before the directory probe, so adding a directory never silently
/// shadows an existing module file.
#[test]
fn file_beside_directory_wins() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("lexer.lg"),
        "# Lexer\n\n## To tokenize (n: Int) -> Int:\n    Return n plus 100.\n",
    )
    .unwrap();
    fs::create_dir_all(dir.path().join("lexer")).unwrap();
    fs::write(dir.path().join("lexer/lexer.lg"), TOKENIZE_PLUS_1).unwrap();
    let main = dir.path().join("main.lg");
    fs::write(
        &main,
        "# App\n\nImport Lexer.\n\n## Main\nShow tokenize(0).\n",
    )
    .unwrap();
    // The FILE (plus 100), not the directory entry (plus 1).
    assert_eq!(interpret_project(&main), "100");
}
