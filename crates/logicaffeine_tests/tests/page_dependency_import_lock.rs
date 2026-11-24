//! ★ LOCK — a page links the project's declared dependencies the way a program does. ★
//!
//! `largo check`, `largo test` and `largo build` of a program resolve `[dependencies]` before they
//! assemble anything, so `Import Crumbs.` finds a path dependency's `src/crumbs.logos`. The pages of
//! a site were assembled with no dependency roots at all, so the same `Import Crumbs.` in a page
//! failed `largo build --static` with "could not find a module named `crumbs`", and naming the file
//! (`Import Crumbs from "../../shared/src/crumbs.logos".`) was refused as escaping the project root.
//! The component a site's pages share was the one thing they could not share.
//!
//! Every test drives the real binary on two projects laid out side by side, as the finding's
//! reproduction does, and the containment gate stays shut for a path nobody declared.

#[path = "cli_common/mod.rs"]
mod common;
use common::http::{body, Server};
use common::{largo_in, stderr, stdout};

use std::path::{Path, PathBuf};

/// `shared/` (the `crumbs` package) and `site/` (which depends on it by path), with `page` as
/// `site/pages/index.logos`. Answers the site's root.
fn two_projects(dir: &Path, page: &str) -> PathBuf {
    let files: &[(&str, &str)] = &[
        (
            "shared/Largo.toml",
            "[package]\nname = \"crumbs\"\nversion = \"0.1.0\"\n",
        ),
        (
            "shared/src/crumbs.logos",
            "## Component Crumbs\n\nView:\n    <nav class=\"crumbs\"><ol>{children}</ol></nav>\n",
        ),
        (
            "site/Largo.toml",
            "[package]\nname = \"site\"\nversion = \"0.1.0\"\n\n[dependencies]\ncrumbs = { path = \"../shared\" }\n",
        ),
        ("site/pages/index.logos", page),
    ];
    for (rel, text) in files {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir.join("site")
}

const BY_NAME: &str = "Import Crumbs.\n\n## Page /\n\nView:\n    <Crumbs><li>here</li></Crumbs>\n";

const BY_PATH: &str = "Import Crumbs from \"../../shared/src/crumbs.logos\".\n\n## Page /\n\nView:\n    <Crumbs><li>here</li></Crumbs>\n";

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{} must exist: {e}", path.display()))
}

/// ★ THE LOAD-BEARING TEST — `Import Crumbs.` in a page finds the dependency's component.
#[test]
fn a_page_imports_a_component_from_a_path_dependency_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let root = two_projects(dir.path(), BY_NAME);
    let out = largo_in(&root, &["build", "--static"]);
    assert!(out.status.success(), "`largo build --static` failed:\n{}", stderr(&out));
    let front = read(&root.join("dist/index.html"));
    assert!(
        front.contains("<nav class=\"crumbs\">") && front.contains("<li>here</li>"),
        "the exported page holds the dependency's component:\n{front}"
    );
}

/// An explicit path into a DECLARED dependency's root passes the containment gate.
#[test]
fn a_page_imports_a_component_by_a_path_into_a_declared_dependency() {
    let dir = tempfile::tempdir().unwrap();
    let root = two_projects(dir.path(), BY_PATH);
    let out = largo_in(&root, &["build", "--static"]);
    assert!(out.status.success(), "`largo build --static` failed:\n{}", stderr(&out));
    assert!(read(&root.join("dist/index.html")).contains("<nav class=\"crumbs\">"));
}

/// `largo check` already linked a page's dependencies; it and the build agree.
#[test]
fn largo_check_agrees_with_the_build() {
    let dir = tempfile::tempdir().unwrap();
    let root = two_projects(dir.path(), BY_NAME);
    let out = largo_in(&root, &["check"]);
    assert!(
        out.status.success(),
        "`largo check` refused the page:\n{}{}",
        stdout(&out),
        stderr(&out)
    );
}

/// The dev server links the same dependencies, so the page it serves is the page that ships.
#[test]
fn the_dev_server_serves_the_dependency_component() {
    let dir = tempfile::tempdir().unwrap();
    let root = two_projects(dir.path(), BY_NAME);
    let mut server = Server::start(&root);
    let front = body(&server.get("/"));
    assert!(
        front.contains("<nav class=\"crumbs\">"),
        "`largo serve` links the page's dependency:\n{front}\n{}",
        server.stderr()
    );
}

/// An UNDECLARED path out of the project is still refused: declaring a dependency opens its root
/// and nothing else.
#[test]
fn an_undeclared_path_out_of_the_project_is_still_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = two_projects(dir.path(), BY_PATH);
    std::fs::write(
        root.join("Largo.toml"),
        "[package]\nname = \"site\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let out = largo_in(&root, &["build", "--static"]);
    let err = stderr(&out);
    assert!(!out.status.success(), "an undeclared escape must not build:\n{err}");
    assert!(err.contains("escapes the project root"), "the gate says why: {err}");
}
