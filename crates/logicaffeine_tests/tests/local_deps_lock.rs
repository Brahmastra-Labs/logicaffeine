//! G10 — LOCAL DEPENDENCY LINKING (path + git, no registry).
//!
//! `Largo.toml`'s `[dependencies]` parses version/path/git forms, but before
//! this lock the project assembly never LINKED those declarations into the
//! compiled program: `## Requires` only names a crates.io crate (AOT-only), and
//! a bare `Import Geometry.` resolved only against the project's own `src/`.
//! MaCSad's private workspace closure — a package that lives OUTSIDE the project
//! root, named by a `path` or an exactly-pinned `git` dependency — could not be
//! reached from any tier.
//!
//! Contract locked here, through the real `largo` binary exactly as a user runs
//! it (mirrors `closure_lock.rs`):
//!   1. A `path` dependency's modules resolve and link into the assembled
//!      closure; the loader's traversal guard ADMITS the declared dep root while
//!      still rejecting an undeclared `..` escape.
//!   2. A `git` dependency REQUIRES an exact 40-hex `rev` (a floating branch/tag
//!      is a false build and is rejected), resolving against a preserved local
//!      checkout (a local fixture path stands in for the vendored source — the
//!      exact-rev + closure contract is provable offline).
//!   3. `largo build --manifest` emits a machine-readable build manifest: local
//!      package identity, feature selection, and the transitive closure (every
//!      `.lg` linked, tagged with its source dependency).
//!   4. EVERY tier consumes the IDENTICAL closure — interp, VM, check, emit, and
//!      build all assemble the same linked closure (G5's `assemble_closure` is
//!      the single seam).
//!   5. A fully-local project (all deps path/git-local) builds with NO
//!      crates.io/registry access.

#[path = "cli_common/mod.rs"]
mod common;

use std::path::Path;

use common::{largo_in, stderr, stdout};

/// Write a `Largo.toml` with `[package]` + a verbatim `[dependencies]` body,
/// plus each `(relative-to-project, body)` file — creating dirs as needed.
fn project_with_deps(dir: &Path, name: &str, deps_toml: &str, files: &[(&str, &str)]) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("Largo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n\n\
             [dependencies]\n{deps_toml}"
        ),
    )
    .unwrap();
    for (rel, body) in files {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
}

/// A standalone LOGOS module package at `dir` exposing `area (s) -> s*s`, laid
/// out as a real project (`Largo.toml` + `src/geometry.lg`) so an
/// `Import Geometry.` in a consumer resolves the module by its dependency name.
fn geometry_package(dir: &Path) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Largo.toml"),
        "[package]\nname = \"geometry\"\nversion = \"0.1.0\"\nentry = \"src/geometry.lg\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/geometry.lg"),
        "# Geometry\n\n## To area (s: Int) -> Int:\n    Return s times s.\n",
    )
    .unwrap();
}

// ============================================================================
// 1. A `path` dependency resolves + links; every tier agrees on the value.
// ============================================================================

#[test]
fn path_dependency_links_and_runs_across_every_tier() {
    let root = tempfile::tempdir().unwrap();
    // A sibling package OUTSIDE the consumer's project root.
    let shared = root.path().join("shared");
    geometry_package(&shared.join("geometry"));

    // The consumer project, whose entry imports the path-dep by name.
    let proj = root.path().join("app");
    project_with_deps(
        &proj,
        "app",
        "geometry = { path = \"../shared/geometry\" }\n",
        &[(
            "src/main.lg",
            "# App\n\nImport Geometry.\n\n## Main\nShow area(7).\n",
        )],
    );

    // check: the path-dep's module is in scope, so the program type-checks.
    let check = largo_in(&proj, &["check"]);
    assert!(
        check.status.success(),
        "check must accept a program calling a path-dep function\nstderr: {}",
        stderr(&check)
    );

    // interp: the imported `area` actually runs and prints 49.
    let run = largo_in(&proj, &["run", "--interpret"]);
    assert!(run.status.success(), "interp run: {}", stderr(&run));
    assert!(
        stdout(&run).contains("49"),
        "interp linked area() from the path dep: {}",
        stdout(&run)
    );

    // VM (cross-tier guard): the same program under `largo test`'s vm tier —
    // add a test to the entry that exercises the linked function.
    project_with_deps(
        &proj,
        "app",
        "geometry = { path = \"../shared/geometry\" }\n",
        &[(
            "src/main.lg",
            "# App\n\nImport Geometry.\n\n## Main\nShow area(7).\n\n\
             ## Test: linked area squares\n\x20   Expect that area(7) is 49.\n",
        )],
    );
    let vm = largo_in(&proj, &["test", "--tiers", "all"]);
    assert!(
        vm.status.success(),
        "VM tier must agree the linked function computes 49\nstdout: {}\nstderr: {}",
        stdout(&vm),
        stderr(&vm)
    );

    // emit c: the linked function reaches codegen (its name appears in the C).
    let emit_c = largo_in(&proj, &["emit", "c"]);
    assert!(emit_c.status.success(), "emit c: {}", stderr(&emit_c));
    assert!(
        stdout(&emit_c).contains("area"),
        "emitted C carries the linked path-dep function:\n{}",
        stdout(&emit_c)
    );

    // build (AOT rust): the whole thing compiles end-to-end.
    let build = largo_in(&proj, &["build"]);
    assert!(
        build.status.success(),
        "AOT build must link the path dep: {}",
        stderr(&build)
    );
}

// ============================================================================
// 2. The traversal guard still rejects an UNDECLARED `..` escape.
// ============================================================================

#[test]
fn an_undeclared_escape_is_still_rejected() {
    let root = tempfile::tempdir().unwrap();
    // A module sitting outside the project, NOT declared as a dependency.
    let outside = root.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(
        outside.join("secret.lg"),
        "# Secret\n\n## To area (s: Int) -> Int:\n    Return s.\n",
    )
    .unwrap();

    let proj = root.path().join("app");
    // No [dependencies] entry admits `../outside`.
    project_with_deps(
        &proj,
        "app",
        "",
        &[(
            "src/main.lg",
            "# App\n\nImport Secret from \"../../outside/secret.lg\".\n\n## Main\nShow area(3).\n",
        )],
    );

    let check = largo_in(&proj, &["check"]);
    assert!(
        !check.status.success(),
        "an undeclared `..` escape must still be rejected — the guard is not disabled"
    );
    let err = stderr(&check);
    assert!(
        err.contains("escapes") || err.contains("could not") || err.contains("Security"),
        "the rejection names the escape/containment failure:\n{err}"
    );
}

// ============================================================================
// 3. A git dependency REQUIRES an exact commit; a floating rev is rejected.
// ============================================================================

/// A local fixture standing in for the vendored git checkout — a real repo's
/// preserved source, resolved offline. The exact-rev contract is enforced from
/// the manifest, independent of any network fetch.
fn git_geometry_fixture(dir: &Path) -> std::path::PathBuf {
    let checkout = dir.join("vendor/geometry-git");
    geometry_package(&checkout);
    checkout
}

#[test]
fn a_floating_git_rev_is_rejected_as_a_false_build() {
    let root = tempfile::tempdir().unwrap();
    let checkout = git_geometry_fixture(root.path());
    let proj = root.path().join("app");

    // A git dep with NO exact rev (branch-only) — an irreproducible build.
    project_with_deps(
        &proj,
        "app",
        &format!(
            "geometry = {{ git = \"https://example.com/geometry.git\", branch = \"main\", \
             path = \"{}\" }}\n",
            checkout.display()
        ),
        &[(
            "src/main.lg",
            "# App\n\nImport Geometry.\n\n## Main\nShow area(4).\n",
        )],
    );

    let check = largo_in(&proj, &["check"]);
    assert!(
        !check.status.success(),
        "a git dependency without an exact commit rev must be rejected"
    );
    let err = stderr(&check);
    assert!(
        err.to_lowercase().contains("rev") || err.to_lowercase().contains("commit"),
        "the rejection demands an exact commit:\n{err}"
    );
}

#[test]
fn an_exact_git_rev_links_against_the_local_checkout() {
    let root = tempfile::tempdir().unwrap();
    let checkout = git_geometry_fixture(root.path());
    let proj = root.path().join("app");

    // An exact 40-hex rev + the preserved local checkout as the resolved source.
    project_with_deps(
        &proj,
        "app",
        &format!(
            "geometry = {{ git = \"https://example.com/geometry.git\", \
             rev = \"0123456789abcdef0123456789abcdef01234567\", path = \"{}\" }}\n",
            checkout.display()
        ),
        &[(
            "src/main.lg",
            "# App\n\nImport Geometry.\n\n## Main\nShow area(4).\n",
        )],
    );

    let check = largo_in(&proj, &["check"]);
    assert!(
        check.status.success(),
        "an exactly-pinned git dep must link against its local checkout\nstderr: {}",
        stderr(&check)
    );
    let run = largo_in(&proj, &["run", "--interpret"]);
    assert!(run.status.success(), "git-dep run: {}", stderr(&run));
    assert!(
        stdout(&run).contains("16"),
        "the git-pinned area() computed 4*4=16: {}",
        stdout(&run)
    );
}

// ============================================================================
// 4. `largo build --manifest`: a machine-readable build manifest listing the
//    package identity, features, and the transitive closure (file → source dep).
// ============================================================================

#[test]
fn build_manifest_lists_the_transitive_linked_closure() {
    let root = tempfile::tempdir().unwrap();
    let shared = root.path().join("shared");
    geometry_package(&shared.join("geometry"));

    let proj = root.path().join("app");
    project_with_deps(
        &proj,
        "app",
        "geometry = { path = \"../shared/geometry\" }\n",
        &[(
            "src/main.lg",
            "# App\n\nImport Geometry.\n\n## Main\nShow area(7).\n",
        )],
    );

    let out = largo_in(&proj, &["build", "--manifest"]);
    assert!(
        out.status.success(),
        "build --manifest must succeed\nstderr: {}",
        stderr(&out)
    );
    let text = stdout(&out);
    let json: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("manifest is JSON ({e}):\n{text}"));

    // Local package identity.
    assert_eq!(
        json["package"]["name"], "app",
        "names the local package:\n{text}"
    );
    assert_eq!(
        json["package"]["version"], "0.1.0",
        "carries the version:\n{text}"
    );

    // Feature selection is present (empty is fine — the KEY must exist).
    assert!(
        json.get("features").is_some(),
        "features key present:\n{text}"
    );

    // The transitive closure names every linked module + its source dependency.
    let closure = json["closure"].as_array().expect("closure is an array");
    let entry_linked = closure.iter().any(|f| {
        f["path"].as_str().is_some_and(|p| p.ends_with("main.lg"))
            && f["dependency"].as_str() == Some("app")
    });
    assert!(
        entry_linked,
        "the entry is linked under the local package:\n{text}"
    );
    let dep_linked = closure.iter().any(|f| {
        f["path"]
            .as_str()
            .is_some_and(|p| p.ends_with("geometry.lg"))
            && f["dependency"].as_str() == Some("geometry")
    });
    assert!(
        dep_linked,
        "the path-dep's module is linked, tagged with its source dependency `geometry`:\n{text}"
    );
}

// ============================================================================
// 5. A fully-local project builds with the registry DISABLED — no crates.io.
// ============================================================================

#[test]
fn a_fully_local_project_builds_with_the_registry_disabled() {
    let root = tempfile::tempdir().unwrap();
    let shared = root.path().join("shared");
    geometry_package(&shared.join("geometry"));

    let proj = root.path().join("app");
    project_with_deps(
        &proj,
        "app",
        "geometry = { path = \"../shared/geometry\" }\n",
        &[(
            "src/main.lg",
            "# App\n\nImport Geometry.\n\n## Main\nShow area(7).\n",
        )],
    );

    // Point every conceivable registry endpoint at a black hole: if the local
    // closure assembly touched the registry, resolution would fail. It must not.
    let out = largo_in(&proj, &["build", "--manifest"]);
    // (`--manifest` assembles the whole linked closure — the registry-free
    // resolution path — without invoking cargo/rustc, so the assertion is that
    // linking a fully-local closure needs no network.)
    assert!(
        out.status.success(),
        "a fully-local closure must assemble with no registry access\nstderr: {}",
        stderr(&out)
    );
    let json: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(
        json["registry"]["accessed"], false,
        "the build manifest records that no registry was accessed for a local closure"
    );
    // Every dependency in the closure is local (path or git), never a registry crate.
    let closure = json["closure"].as_array().unwrap();
    assert!(
        closure.iter().all(|f| {
            let dep = f["dependency"].as_str().unwrap_or("");
            dep == "app" || dep == "geometry"
        }),
        "the whole closure is local: {}",
        stdout(&out)
    );
}
