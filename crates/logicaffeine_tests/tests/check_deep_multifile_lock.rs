//! ★ G9 deep-check LOCK — `largo check --deep` over a MULTI-FILE project. ★
//!
//! Deep-check runs rustc over the generated Rust and translates its findings back to
//! LOGOS. It must check the SAME assembled closure every other command compiles — not the
//! entry file in isolation. Compiling the entry alone drops every imported module, so a
//! call to an imported function becomes a PHANTOM "cannot find function" finding: the
//! program is valid (shallow `check` passes), yet `check --deep` reports a non-error. This
//! lock pins that deep-check closes over the whole import graph, and that a REAL finding in
//! the user's own entry still maps to the user's own file and line.
//!
//! The programs here use only user-defined functions + `Int` arithmetic (no stdlib
//! prelude), so the lock is hermetic — it exercises the closure/mapping seam without
//! depending on the demand-imported stdlib.

#[path = "cli_common/mod.rs"]
mod common;

use common::*;
use std::process::Output;

/// Write a two-file project (`src/main.lg` + `src/util.lg`) and run `largo <args>`,
/// returning the process output. Each test gets a fresh temp dir.
fn project_check(main: &str, util: &str, args: &[&str]) -> (tempfile::TempDir, Output) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Largo.toml"),
        "[package]\nname = \"mfdeep\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/util.lg"), util).unwrap();
    std::fs::write(dir.path().join("src/main.lg"), main).unwrap();
    let out = largo_in(dir.path(), args);
    (dir, out)
}

const UTIL: &str = "# Util\n\n## To triple (n: Int) -> Int:\n    Return n times 3.\n";

#[test]
fn a_call_to_an_imported_function_is_not_a_phantom_deep_finding() {
    // The bug this locks: deep-check compiling the ENTRY ALONE leaves `triple`
    // undefined, so rustc reports "cannot find function `triple`" — a finding on a
    // program that shallow-`check` accepts. Closing over the import graph fixes it.
    let (_dir, shallow) = project_check(
        "# App\n\nImport Util.\n\n## Main\nShow triple(7).\n",
        UTIL,
        &["check"],
    );
    assert_eq!(
        shallow.status.code(),
        Some(0),
        "the multi-file program is valid — shallow check passes:\n{}",
        stderr(&shallow)
    );

    let (_dir, deep) = project_check(
        "# App\n\nImport Util.\n\n## Main\nShow triple(7).\n",
        UTIL,
        &["check", "--deep"],
    );
    let all = format!("{}{}", stdout(&deep), stderr(&deep));
    assert!(
        !all.contains("panicked"),
        "deep check must never panic:\n{all}"
    );
    assert!(
        !all.contains("cannot find function") && !all.contains("triple"),
        "an imported function must be IN SCOPE for deep-check — no phantom finding:\n{all}"
    );
    assert_eq!(
        deep.status.code(),
        Some(0),
        "a valid multi-file program must pass deep check (imports closed over):\n{all}"
    );
    assert!(
        all.contains("Deep check passed"),
        "deep check reports its clean verdict for the whole closure:\n{all}"
    );
}

#[test]
fn a_real_finding_in_the_entry_maps_to_the_entry_file_and_line() {
    // With imports present in the closure, a GENUINELY unknown call in the entry is
    // still a finding — and it must point at the user's own line in main.lg (here
    // line 6, `Show bogusFn(7).`), not at a spliced import or line 1.
    let (_dir, out) = project_check(
        "# App\n\nImport Util.\n\n## Main\nShow bogusFn(7).\n",
        UTIL,
        &["check", "--deep"],
    );
    let err = stderr(&out);
    assert!(!err.contains("panicked"), "no panic:\n{err}");
    assert_ne!(
        out.status.code(),
        Some(0),
        "an unknown function is a finding:\n{err}"
    );
    assert!(
        err.contains("bogusFn"),
        "the finding names the unknown function:\n{err}"
    );
    assert!(
        err.contains("main.lg:6"),
        "the finding maps to the user's own line 6 in main.lg, with imports in the closure:\n{err}"
    );
    // And it names the entry file, never the imported module.
    assert!(
        !err.contains("util.lg:"),
        "the finding must not be misattributed to the imported module:\n{err}"
    );
}

#[test]
fn the_closure_source_map_is_available_for_a_single_file_project() {
    // A single-file project closes over just its entry — deep-check still maps a real
    // entry finding to the entry's own line, unchanged from the pre-closure behavior.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Largo.toml"),
        "[package]\nname = \"sfdeep\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/main.lg"),
        "# App\n\n## Main\nLet x be 1.\nShow ghostFn(x).\n",
    )
    .unwrap();
    let out = largo_in(dir.path(), &["check", "--deep"]);
    let err = stderr(&out);
    assert!(!err.contains("panicked"), "no panic:\n{err}");
    assert!(
        err.contains("ghostFn"),
        "names the unknown function:\n{err}"
    );
    assert!(
        err.contains("main.lg:5"),
        "maps to the user's own line 5 (single-file, exact mapping):\n{err}"
    );
}
