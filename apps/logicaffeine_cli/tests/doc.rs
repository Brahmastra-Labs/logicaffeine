//! `largo doc [--out DIR]` — generate project documentation from `##` blocks.

mod common;

use common::*;
use tempfile::tempdir;

const DOCUMENTED: &str = "# Doc Demo\n\nA demo project for largo doc.\n\n\
## A Point has\n    an x (Int)\n    a y (Int)\n\n\
## To double (n: Int) -> Int:\n    Return n * 2.\n\n\
## Note\n\nDoubling distributes over addition.\n\n\
## Example\n\n    Show double(21).\n\n\
## Main\n\n    Show 99.\n";

fn scaffold_documented(dir: &std::path::Path) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Largo.toml"),
        "[package]\nname = \"doc_demo\"\nversion = \"0.1.0\"\ndescription = \"Documented demo\"\nentry = \"src/main.lg\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/main.lg"), DOCUMENTED).unwrap();
}

/// `largo doc` renders target/doc/<name>.md with the package title,
/// function signatures, type definitions, notes, and fenced examples —
/// and omits the `## Main` body.
#[test]
fn doc_generates_markdown_from_blocks() {
    let dir = tempdir().unwrap();
    scaffold_documented(dir.path());

    let out = largo_in(dir.path(), &["doc"]);
    assert_eq!(out.status.code(), Some(0), "doc: {}", stderr(&out));

    let md = std::fs::read_to_string(dir.path().join("target/doc/doc_demo.md"))
        .expect("target/doc/doc_demo.md must exist");
    assert!(md.contains("# doc_demo"), "package title heading:\n{md}");
    assert!(md.contains("Documented demo"), "manifest description:\n{md}");
    assert!(
        md.contains("To double (n: Int) -> Int"),
        "function signature must appear:\n{md}"
    );
    assert!(md.contains("A Point has"), "type definition must appear:\n{md}");
    assert!(
        md.contains("Doubling distributes over addition."),
        "note prose must appear:\n{md}"
    );
    assert!(md.contains("```"), "examples must be fenced:\n{md}");
    assert!(md.contains("Show double(21)."), "example body must appear:\n{md}");
    assert!(!md.contains("Show 99."), "## Main body must be omitted:\n{md}");
}

/// Blocks appear in source order.
#[test]
fn doc_preserves_block_order() {
    let dir = tempdir().unwrap();
    scaffold_documented(dir.path());
    assert_eq!(largo_in(dir.path(), &["doc"]).status.code(), Some(0));

    let md = std::fs::read_to_string(dir.path().join("target/doc/doc_demo.md")).unwrap();
    let type_pos = md.find("A Point has").expect("typedef present");
    let fn_pos = md.find("To double").expect("function present");
    let note_pos = md.find("Doubling distributes").expect("note present");
    assert!(type_pos < fn_pos && fn_pos < note_pos, "source order must hold:\n{md}");
}

/// `--out` redirects the output directory.
#[test]
fn doc_respects_out_dir() {
    let dir = tempdir().unwrap();
    scaffold_documented(dir.path());
    let out = largo_in(dir.path(), &["doc", "--out", "book"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(dir.path().join("book/doc_demo.md").exists());
}

/// A Main-only project still gets a titled document.
#[test]
fn doc_main_only_project_documents_title() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "plain_proj");
    let out = largo_in(dir.path(), &["doc"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let md = std::fs::read_to_string(dir.path().join("target/doc/plain_proj.md")).unwrap();
    assert!(md.contains("# plain_proj"));
}

/// `--out` pointing at an existing FILE gets a clear error, not a raw OS
/// code.
#[test]
fn doc_out_at_existing_file_is_a_clear_error() {
    let dir = tempdir().unwrap();
    scaffold_documented(dir.path());
    std::fs::write(dir.path().join("occupied"), b"x").unwrap();
    let out = largo_in(dir.path(), &["doc", "--out", "occupied"]);
    assert_eq!(out.status.code(), Some(1));
    let err = strip_ansi(&stderr(&out));
    assert!(
        err.contains("directory"),
        "must explain --out needs a directory:\n{err}"
    );
}

/// A project whose entry file is missing fails cleanly.
#[test]
fn doc_missing_entry_fails() {
    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join("Largo.toml"),
        "[package]\nname = \"no_entry\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let out = largo_in(dir.path(), &["doc"]);
    assert_eq!(out.status.code(), Some(1));
}
