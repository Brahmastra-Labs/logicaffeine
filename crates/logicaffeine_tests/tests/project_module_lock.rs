//! ★ LOCK — a bare `Import <Name>.` names a module of THIS PROJECT, from anywhere in it. ★
//!
//! A largo project keeps its modules in `src/` and its pages in `pages/`. The loader resolved a
//! bare import only against the IMPORTING FILE'S OWN DIRECTORY, so `pages/app.lg` saying
//! `Import Shared.` could never reach `src/shared.lg` — the one record a page and its server agree
//! about was unreachable from the page, which is the whole point of putting it in `src/`.
//!
//! ★ THE RULE, and it is what the sentence already says: a bare name is a PROJECT module. The
//! importing file's own directory is searched FIRST (a module beside you is the one you meant, and
//! every project that resolves today keeps resolving), then the project's `src/`, then the project
//! root. An explicit `from "path"` import is untouched — it says exactly where to look.
//!
//! ★ THE PROJECT IS THE DIRECTORY HOLDING `Largo.toml`, discovered by walking up from the entry.
//! Nothing has to be threaded through the callers, so `largo build`, `largo test`, the LSP and a
//! plain `assemble_project` all agree about what a bare name means — three answers to that
//! question is how a page compiles in one tool and not another.
//!
//! ★ AND THE CONTAINMENT GATE STILL HOLDS. Widening WHERE a module may be found must not widen
//! what a module may BE: a `..` escape out of the project is still refused, which is asserted here
//! beside the widening rather than left to the security test that predates it.

use std::fs;
use std::path::{Path, PathBuf};

/// A project on disk: `Largo.toml` at the root, plus whatever files the test names.
fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("Largo.toml"),
        "[package]\nname = \"t\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    for (rel, body) in files {
        let p = dir.path().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }
    dir
}

fn assemble(root: &Path, entry: &str) -> Result<String, String> {
    logicaffeine_compile::analysis::assemble_project(&root.join(entry)).map(|a| a.source)
}

fn declares_shape(src: &str) -> bool {
    src.lines()
        .any(|l| l.trim_start().starts_with("## A Shape has:"))
}

const SHAPE: &str =
    "# Shared\n\nThe shape both halves agree on.\n\n## A Shape has:\n    a name, which is Text.\n";

#[test]
fn a_page_reaches_a_module_in_src() {
    // ★ THE HEADLINE — the scaffold's own layout. Without it a page cannot share a type with the
    // server it talks to, which is the entire reason the type lives in `src/`.
    let d = project(&[
        ("src/shared.lg", SHAPE),
        (
            "pages/app.lg",
            "# App\n\nProse about the page.\n\nImport Shared.\n\n## Main\nShow 1.\n",
        ),
    ]);
    let src = assemble(d.path(), "pages/app.lg").expect("assembles");
    assert!(
        declares_shape(&src),
        "the page must see the module's declaration:\n{src}"
    );
}

#[test]
fn a_module_beside_the_importer_still_resolves() {
    // The case that always worked. Every project in the repo depends on it.
    let d = project(&[
        ("src/shared.lg", SHAPE),
        (
            "src/server.lg",
            "# Server\n\nProse.\n\nImport Shared.\n\n## Main\nShow 1.\n",
        ),
    ]);
    let src = assemble(d.path(), "src/server.lg").expect("assembles");
    assert!(
        declares_shape(&src),
        "a sibling module must still resolve:\n{src}"
    );
}

#[test]
fn the_nearest_module_wins_over_the_one_in_src() {
    // ★ THE AMBIGUITY, decided deliberately: a module BESIDE you is the one you meant. Preferring
    // `src/` would silently change which file an existing project's import resolves to — the kind
    // of change that shows up as behaviour nobody edited.
    let d = project(&[
        ("src/shared.lg", SHAPE),
        (
            "pages/shared.lg",
            "# Shared\n\nThe page's own.\n\n## A Local has:\n    a n, which is Int.\n",
        ),
        (
            "pages/app.lg",
            "# App\n\nProse.\n\nImport Shared.\n\n## Main\nShow 1.\n",
        ),
    ]);
    let src = assemble(d.path(), "pages/app.lg").expect("assembles");
    assert!(
        src.contains("## A Local has:"),
        "the neighbour must win:\n{src}"
    );
    assert!(
        !declares_shape(&src),
        "and the one in src/ must not also be spliced:\n{src}"
    );
}

#[test]
fn a_module_at_the_project_root_resolves_too() {
    // A small project keeps its modules beside `Largo.toml` rather than under `src/`.
    let d = project(&[
        ("shared.lg", SHAPE),
        (
            "pages/app.lg",
            "# App\n\nProse.\n\nImport Shared.\n\n## Main\nShow 1.\n",
        ),
    ]);
    let src = assemble(d.path(), "pages/app.lg").expect("assembles");
    assert!(declares_shape(&src), "a root module must resolve:\n{src}");
}

#[test]
fn a_module_that_exists_nowhere_still_fails_loudly() {
    // ★ THE FAILURE THAT MUST STAY LOUD. An import nobody resolves used to be silently dropped,
    // and the first sign was a backend refusal in another file naming a type nobody had declared.
    let d = project(&[(
        "pages/app.lg",
        "# App\n\nProse.\n\nImport Missing.\n\n## Main\nShow 1.\n",
    )]);
    let err = assemble(d.path(), "pages/app.lg").expect_err("must not silently succeed");
    assert!(
        err.contains("Missing") || err.contains("missing"),
        "the error must name the module: {err}"
    );
}

#[test]
fn an_explicit_path_import_is_unchanged() {
    // `from "…"` says exactly where to look; the search order must not second-guess it.
    let d = project(&[
        ("src/shared.lg", SHAPE),
        (
            "pages/app.lg",
            "# App\n\nProse.\n\nImport Shared from \"../src/shared.lg\".\n\n## Main\nShow 1.\n",
        ),
    ]);
    let src = assemble(d.path(), "pages/app.lg").expect("assembles");
    assert!(
        declares_shape(&src),
        "an explicit path must resolve:\n{src}"
    );
}

#[test]
fn an_escape_out_of_the_project_is_still_refused() {
    // ★ WIDENING WHERE A MODULE MAY BE FOUND MUST NOT WIDEN WHAT IT MAY BE. The module exists and
    // is readable; it is simply outside the project, and that is the whole reason to refuse it.
    let outside = tempfile::tempdir().expect("tempdir");
    fs::write(outside.path().join("secret.lg"), SHAPE).unwrap();
    let d = project(&[(
        "pages/app.lg",
        &format!(
            "# App\n\nProse.\n\nImport Secret from \"{}\".\n\n## Main\nShow 1.\n",
            outside.path().join("secret.lg").display()
        ),
    )]);
    let r = assemble(d.path(), "pages/app.lg");
    assert!(
        r.is_err(),
        "an import outside the project must be refused, got:\n{r:?}"
    );
}

#[test]
fn a_directory_with_no_largo_toml_behaves_as_before() {
    // Not every `.lg` lives in a project (a scratch file, a doc example). With no `Largo.toml` to
    // find, the search is exactly the local one it always was — and a sibling still resolves.
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("shared.lg"), SHAPE).unwrap();
    fs::write(
        dir.path().join("app.lg"),
        "# App\n\nProse.\n\nImport Shared.\n\n## Main\nShow 1.\n",
    )
    .unwrap();
    let src = assemble(dir.path(), "app.lg").expect("assembles");
    assert!(
        declares_shape(&src),
        "a sibling must resolve with no project around it:\n{src}"
    );
}

#[test]
fn the_project_root_is_found_from_a_nested_page() {
    // Pages may sit in subdirectories; the walk up must not stop at the first parent.
    let d = project(&[
        ("src/shared.lg", SHAPE),
        (
            "pages/admin/settings.lg",
            "# Settings\n\nProse.\n\nImport Shared.\n\n## Main\nShow 1.\n",
        ),
    ]);
    let src = assemble(d.path(), "pages/admin/settings.lg").expect("assembles");
    assert!(
        declares_shape(&src),
        "a nested page must reach src/:\n{src}"
    );
}

#[test]
fn the_module_is_spliced_once_even_if_two_files_import_it() {
    // A diamond through the new search path must dedupe exactly as a local one does — a record
    // declared twice in one compilation unit is a redefinition, not a convenience.
    let d = project(&[
        ("src/shared.lg", SHAPE),
        (
            "src/mid.lg",
            "# Mid\n\nProse.\n\nImport Shared.\n\n## To mid () -> Int:\n    Return 1.\n",
        ),
        (
            "pages/app.lg",
            "# App\n\nProse.\n\nImport Shared and Mid.\n\n## Main\nShow mid().\n",
        ),
    ]);
    let src = assemble(d.path(), "pages/app.lg").expect("assembles");
    let n = src
        .lines()
        .filter(|l| l.trim_start().starts_with("## A Shape has:"))
        .count();
    assert_eq!(n, 1, "the module must be spliced exactly once:\n{src}");
}

/// Where the project root is, for the assertion below.
fn largo_root(from: &Path) -> Option<PathBuf> {
    let mut d = Some(from);
    while let Some(p) = d {
        if p.join("Largo.toml").is_file() {
            return Some(p.to_path_buf());
        }
        d = p.parent();
    }
    None
}

#[test]
fn the_fullstack_example_resolves_its_shared_module() {
    // ★ THE PROJECT THIS WAS FOUND IN, asserted against the real files rather than a fixture — a
    // rule that holds for a tempdir and not for the example in the repo has not landed.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let example = repo.join("examples/fullstack_todo");
    assert!(
        largo_root(&example.join("pages")).is_some(),
        "the example must be a project"
    );
    for entry in ["pages/app.lg", "src/server.lg"] {
        let src = logicaffeine_compile::analysis::assemble_project(&example.join(entry))
            .unwrap_or_else(|e| panic!("{entry} assembles: {e}"))
            .source;
        assert!(
            src.lines()
                .any(|l| l.trim_start().starts_with("## A Todo has:")),
            "{entry} must see the Todo declaration it imports"
        );
    }
}
