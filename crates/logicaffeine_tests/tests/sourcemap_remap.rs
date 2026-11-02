//! Source map: a rustc error over the GENERATED Rust is remapped to the LOGOS
//! source line it came from (not `main.rs`).

#![cfg(not(target_arch = "wasm32"))]

use std::fs;

use tempfile::tempdir;

use logicaffeine_compile::analysis::assemble_project;
use logicaffeine_compile::compile::remap_project_errors;

/// A call to an undefined function codegens to a dangling Rust call that rustc
/// rejects (E0425). The remap must point back at the LOGOS line the user wrote.
#[test]
fn rustc_error_remaps_to_logos_source() {
    let cache = tempdir().unwrap();
    // `mystery` is defined nowhere → the emitted Rust calls an unknown function.
    let source = "## Main\n    Let x be mystery(1).\n    Show x.\n";

    let remapped =
        remap_project_errors(source, cache.path(), None).expect("cargo check should run");

    assert!(
        !remapped.is_empty(),
        "the generated-code error must remap to a LOGOS location"
    );
    assert!(
        remapped
            .iter()
            .any(|e| e.source_line.contains("mystery") || e.message.contains("mystery")),
        "remapped error should reference the LOGOS `mystery` line, got: {remapped:?}"
    );
}

/// In a MULTI-FILE project, a rustc error in an imported module is attributed to
/// THAT `.lg` file (chaining the codegen map through the assembler's file map),
/// not the merged source or `main.rs`.
#[test]
fn multifile_rustc_error_names_the_source_file() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("util.lg"),
        "# Util\n\n## To helper (n: Int):\n    Return mystery(n).\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        "# App\n\nImport Util.\n\n## Main\nShow helper(1).\n",
    )
    .unwrap();

    let assembled = assemble_project(&main_path).expect("assemble");
    let cache = tempdir().unwrap();
    let remapped =
        remap_project_errors(&assembled.source, cache.path(), Some(&assembled.source_map))
            .expect("cargo check should run");

    assert!(!remapped.is_empty(), "the error must remap");
    assert!(
        remapped
            .iter()
            .any(|e| e.file.as_deref() == Some("util.lg")),
        "the error should be attributed to `util.lg`, got: {remapped:?}"
    );
}
