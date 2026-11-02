//! Determinism & order-independence of the multi-file assembler — the
//! reproducible-build foundation. Two tiers:
//!   A) BYTE-IDENTITY: assembling a fixed project always yields byte-identical
//!      `source` (across runs and across independent copies of the same files).
//!   B) REORDER-EQUIVALENCE: reordering sibling `Import` lines, or whole
//!      definition blocks within a module, yields an OBSERVATIONALLY-EQUIVALENT
//!      program — same interpreter output and the same set of definitions.
//!
//! These pin invariants a future refactor could silently break (e.g. a HashMap
//! leaking iteration order into `source` or into an error message).

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::Path;

use tempfile::tempdir;

use logicaffeine_compile::analysis::assemble_project;

/// Assemble a project and return its combined source.
fn assemble_source(entry: &Path) -> String {
    assemble_project(entry).expect("assemble").source
}

/// Assemble a project expected to FAIL; return the error string (`AssembledProgram`
/// isn't `Debug`, so `unwrap_err` won't do).
fn assemble_err(entry: &Path) -> String {
    match assemble_project(entry) {
        Ok(_) => panic!("expected an assembly error"),
        Err(e) => e,
    }
}

/// Interpret an assembled project as `largo run --interpret` does.
fn interpret_project(entry: &Path) -> String {
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

/// The sorted multiset of `## ` header lines in an assembled source — a
/// structural fingerprint of "which definitions are present," independent of the
/// order they were spliced.
fn def_headers(source: &str) -> Vec<String> {
    let mut hs: Vec<String> = source
        .lines()
        .map(str::trim_end)
        .filter(|l| l.starts_with("## ") && !l.starts_with("## Main"))
        .map(str::to_string)
        .collect();
    hs.sort();
    hs
}

/// Write a set of `(filename, body)` files into `dir`.
fn write_files(dir: &Path, files: &[(&str, &str)]) {
    for (name, body) in files {
        fs::write(dir.join(name), body).unwrap();
    }
}

// ============================================================================
// Tier A — byte-identity
// ============================================================================

/// Assembling the SAME entry twice is byte-identical (no run-to-run nondeterminism
/// leaks into `source`).
#[test]
fn assemble_source_is_byte_deterministic_across_runs() {
    let dir = tempdir().unwrap();
    write_files(
        dir.path(),
        &[
            (
                "d.lg",
                "# D\n\n## To d (n: Int) -> Int:\n    Return n plus 1.\n",
            ),
            (
                "c.lg",
                "# C\n\nImport D.\n\n## To c (n: Int) -> Int:\n    Return d(n) plus 10.\n",
            ),
            (
                "b.lg",
                "# B\n\nImport C.\n\n## To b (n: Int) -> Int:\n    Return c(n) plus 100.\n",
            ),
        ],
    );
    let main = dir.path().join("main.lg");
    fs::write(
        &main,
        "# App\n\nImport B and C.\n\n## Main\nShow b(0) plus c(0).\n",
    )
    .unwrap();

    let first = assemble_source(&main);
    for _ in 0..30 {
        assert_eq!(
            assemble_source(&main),
            first,
            "assembled source must be byte-stable across runs"
        );
    }
}

/// Two INDEPENDENT copies of the same project (different tempdirs) assemble to
/// byte-identical `source` — proves no absolute path or dir-specific value leaks
/// into the combined source.
#[test]
fn assemble_source_is_identical_across_independent_copies() {
    let files: &[(&str, &str)] = &[
        (
            "util.lg",
            "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n",
        ),
        (
            "geo.md",
            "# Geo\n\n## A Point has:\n    An x: Int.\n    A y: Int.\n",
        ),
    ];
    let main_src = "# App\n\nImport Util and Geo.\n\n## Main\nLet p be a new Point with x 1 and y 2.\nShow double(p's x).\n";

    let dir_a = tempdir().unwrap();
    write_files(dir_a.path(), files);
    fs::write(dir_a.path().join("main.lg"), main_src).unwrap();

    let dir_b = tempdir().unwrap();
    write_files(dir_b.path(), files);
    fs::write(dir_b.path().join("main.lg"), main_src).unwrap();

    assert_eq!(
        assemble_source(&dir_a.path().join("main.lg")),
        assemble_source(&dir_b.path().join("main.lg")),
        "identical files in different directories must assemble byte-identically"
    );
}

/// Owner-mangled private functions stay byte-deterministic across independent
/// copies — the mangle prefix is the (sanitized) file stem, never an absolute
/// path, so two copies of the same project assemble identically.
#[test]
fn mangled_private_functions_are_deterministic_across_copies() {
    let files: &[(&str, &str)] = &[(
        "util.lg",
        "# Util\n\n## To private helper (n: Int) -> Int:\n    Return n plus 1.\n\n## To compute (n: Int) -> Int:\n    Return helper(n).\n",
    )];
    let main_src = "# App\n\nImport Util.\n\n## Main\nShow compute(41).\n";

    let dir_a = tempdir().unwrap();
    write_files(dir_a.path(), files);
    fs::write(dir_a.path().join("main.lg"), main_src).unwrap();
    let dir_b = tempdir().unwrap();
    write_files(dir_b.path(), files);
    fs::write(dir_b.path().join("main.lg"), main_src).unwrap();

    let sa = assemble_source(&dir_a.path().join("main.lg"));
    assert!(
        sa.contains("util__helper"),
        "the private function is owner-mangled: {sa}"
    );
    assert_eq!(
        sa,
        assemble_source(&dir_b.path().join("main.lg")),
        "mangled source must be copy-stable"
    );
}

// ============================================================================
// Tier B — reorder-equivalence
// ============================================================================

/// Reordering sibling imports (`Import A and B` vs `Import B and A`) yields an
/// equivalent program: same interpreter output AND the same set of definitions.
#[test]
fn sibling_import_reorder_is_output_equivalent() {
    let files: &[(&str, &str)] = &[
        (
            "a.lg",
            "# A\n\n## To fa (n: Int) -> Int:\n    Return n plus 1.\n",
        ),
        (
            "b.lg",
            "# B\n\n## To fb (n: Int) -> Int:\n    Return n plus 2.\n",
        ),
    ];

    let dir1 = tempdir().unwrap();
    write_files(dir1.path(), files);
    fs::write(
        dir1.path().join("main.lg"),
        "# App\n\nImport A and B.\n\n## Main\nShow fa(10) plus fb(10).\n",
    )
    .unwrap();

    let dir2 = tempdir().unwrap();
    write_files(dir2.path(), files);
    fs::write(
        dir2.path().join("main.lg"),
        "# App\n\nImport B and A.\n\n## Main\nShow fa(10) plus fb(10).\n",
    )
    .unwrap();

    let m1 = dir1.path().join("main.lg");
    let m2 = dir2.path().join("main.lg");
    assert_eq!(interpret_project(&m1), "23");
    assert_eq!(
        interpret_project(&m1),
        interpret_project(&m2),
        "import order must not change output"
    );
    assert_eq!(
        def_headers(&assemble_source(&m1)),
        def_headers(&assemble_source(&m2)),
        "the same set of definitions must be present regardless of import order"
    );
}

/// Reordering whole definition blocks WITHIN a module is equivalent — the same
/// functions, same result.
#[test]
fn intra_module_definition_reorder_is_equivalent() {
    let dir1 = tempdir().unwrap();
    write_files(dir1.path(), &[(
        "lib.lg",
        "# Lib\n\n## To f (n: Int) -> Int:\n    Return n plus 1.\n\n## To g (n: Int) -> Int:\n    Return n plus 2.\n",
    )]);
    fs::write(
        dir1.path().join("main.lg"),
        "# App\n\nImport Lib.\n\n## Main\nShow f(10) plus g(10).\n",
    )
    .unwrap();

    let dir2 = tempdir().unwrap();
    write_files(dir2.path(), &[(
        "lib.lg",
        "# Lib\n\n## To g (n: Int) -> Int:\n    Return n plus 2.\n\n## To f (n: Int) -> Int:\n    Return n plus 1.\n",
    )]);
    fs::write(
        dir2.path().join("main.lg"),
        "# App\n\nImport Lib.\n\n## Main\nShow f(10) plus g(10).\n",
    )
    .unwrap();

    assert_eq!(interpret_project(&dir1.path().join("main.lg")), "23");
    assert_eq!(
        interpret_project(&dir1.path().join("main.lg")),
        interpret_project(&dir2.path().join("main.lg")),
        "definition order within a module must not change the result"
    );
}

/// A collision error message is deterministic across runs (sorted names + paths),
/// so a reproducible build never produces a spuriously-different diagnostic.
#[test]
fn collision_error_is_run_deterministic() {
    let dir = tempdir().unwrap();
    write_files(
        dir.path(),
        &[
            (
                "a.lg",
                "# A\n\n## To parse (n: Int) -> Int:\n    Return n plus 1.\n",
            ),
            (
                "b.lg",
                "# B\n\n## To parse (n: Int) -> Int:\n    Return n plus 2.\n",
            ),
        ],
    );
    let main = dir.path().join("main.lg");
    fs::write(
        &main,
        "# App\n\nImport A and B.\n\n## Main\nShow parse(10).\n",
    )
    .unwrap();

    let first = assemble_err(&main);
    assert!(
        first.contains("parse"),
        "collision names the ambiguous function: {first}"
    );
    for _ in 0..30 {
        assert_eq!(
            assemble_err(&main),
            first,
            "collision message must be run-deterministic"
        );
    }
}

/// Reordering the imports in the entry does not change the collision message
/// (names and files are sorted before rendering).
#[test]
fn collision_message_is_import_order_independent() {
    let dir = tempdir().unwrap();
    write_files(
        dir.path(),
        &[
            (
                "a.lg",
                "# A\n\n## To parse (n: Int) -> Int:\n    Return n plus 1.\n",
            ),
            (
                "b.lg",
                "# B\n\n## To parse (n: Int) -> Int:\n    Return n plus 2.\n",
            ),
        ],
    );
    let main = dir.path().join("main.lg");

    fs::write(
        &main,
        "# App\n\nImport A and B.\n\n## Main\nShow parse(10).\n",
    )
    .unwrap();
    let ab = assemble_err(&main);
    fs::write(
        &main,
        "# App\n\nImport B and A.\n\n## Main\nShow parse(10).\n",
    )
    .unwrap();
    let ba = assemble_err(&main);

    assert_eq!(ab, ba, "collision message must not depend on import order");
}
