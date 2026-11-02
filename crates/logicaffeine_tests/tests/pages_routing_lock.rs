//! ★ LOCK — `## Page /path` is the URL. ★
//!
//! A page declares where it lives: `## Page /dashboard/settings`. That header was PARSED, carried
//! through the AST as `ComponentKind::Page { path, .. }` — and nothing ever read it. The URL came
//! entirely from the FILE NAME, so a page declaring `/dashboard/settings` served at `/app`, and the
//! sentence the author wrote did nothing at all.
//!
//! That is the failure this campaign keeps finding, and it is at its worst in a router: the author
//! is told their intent was understood, the page appears at an address they did not choose, and
//! every link they write to the address they DID choose 404s.
//!
//! ★ THE FILE NAME REMAINS THE DEFAULT, because a scaffold should not need a header to work —
//! `pages/app.lg` is the front page and `pages/about.lg` is `/about` by convention. The header
//! OVERRIDES it. Convention where you want none, a declaration where you do.

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr};

/// Scaffold a `--web` project, replace its page(s), and plan the site.
fn site_with(pages: &[(&str, &str)]) -> Result<Vec<String>, String> {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = largo_in(dir.path(), &["new", "site", "--web"]);
    assert!(out.status.success(), "largo new --web: {}", stderr(&out));
    let root = dir.path().join("site");
    std::fs::remove_file(root.join("pages/app.lg")).expect("replace the scaffolded page");
    for (name, src) in pages {
        std::fs::write(root.join("pages").join(name), src).expect("write page");
    }
    logicaffeine_cli::plan_site_assets(&root.join("pages"), &root.join("static"))
        .map(|assets| assets.into_iter().map(|(p, _, _)| p).collect())
}

const fn page(path: &str) -> &str {
    path
}

fn page_src(header: &str, body: &str) -> String {
    format!("{header}\nView:\n    <p>{body}</p>\n")
}

#[test]
fn a_declared_path_is_where_the_page_serves() {
    // The header was parsed and ignored; a page declaring `/about` served at `/app`.
    let paths = site_with(&[("app.lg", &page_src("## Page /about", "about"))]).expect("plans");
    assert!(
        paths.contains(&"/about".to_string()),
        "the declared path serves: {paths:?}"
    );
    assert!(
        paths.contains(&"/about.html".to_string()),
        "…and its `.html` twin, like every page: {paths:?}"
    );
}

#[test]
fn a_declared_nested_path_serves_nested() {
    // Nested paths are the reason a header exists at all: a file name cannot spell `/a/b` without
    // deciding that a directory means something, and a page's URL should not depend on where its
    // source happens to sit.
    let paths = site_with(&[("settings.lg", &page_src("## Page /dashboard/settings", "s"))])
        .expect("plans");
    assert!(
        paths.contains(&"/dashboard/settings".to_string()),
        "a nested declared path serves nested: {paths:?}"
    );
}

#[test]
fn a_page_declaring_the_root_owns_the_front_page() {
    // `/` is not special-cased by file name any more — a page can SAY it is the front page.
    let paths = site_with(&[("home.lg", &page_src("## Page /", "home"))]).expect("plans");
    assert!(
        paths.contains(&"/".to_string()),
        "the declared root serves: {paths:?}"
    );
    assert!(
        paths.contains(&"/index.html".to_string()),
        "…and `/index.html`, which is what a static host serves for a directory: {paths:?}"
    );
}

#[test]
fn the_file_name_is_still_the_default() {
    // ★ CONVENTION WHERE YOU WANT NONE. A scaffold must work without a header, and `app.lg` being
    // the front page is the convention every existing project relies on. The header OVERRIDES the
    // default; it does not replace the idea of one.
    let paths = site_with(&[
        ("app.lg", &page_src("## Component App", "front")),
        ("about.lg", &page_src("## Component About", "about")),
    ])
    .expect("plans");
    assert!(
        paths.contains(&"/".to_string()),
        "`app.lg` is still the front page: {paths:?}"
    );
    assert!(
        paths.contains(&"/about".to_string()),
        "`about.lg` is still `/about`: {paths:?}"
    );
}

#[test]
fn a_declared_path_beats_the_file_name_it_disagrees_with() {
    // The two rules meet here, and one has to win. The DECLARATION does — an author who wrote a
    // path meant it, and a file name they did not choose (or inherited from a rename) must not
    // quietly override it.
    let paths = site_with(&[("app.lg", &page_src("## Page /welcome", "w"))]).expect("plans");
    assert!(
        paths.contains(&"/welcome".to_string()),
        "the declaration wins: {paths:?}"
    );
    assert!(
        !paths.contains(&"/".to_string()),
        "…and the file-name default does not ALSO claim the root, which would make which page you \
         get depend on enumeration order: {paths:?}"
    );
}

#[test]
fn two_pages_claiming_one_path_is_a_loud_build_error() {
    // ★ THE EDGE THAT MATTERS MOST. Two pages at one URL means the one you get depends on the
    // order the directory happened to be read in — a decision nobody made, that changes between
    // machines. A build that picked silently would ship a site serving the wrong page.
    let err = site_with(&[
        ("one.lg", &page_src("## Page /same", "one")),
        ("two.lg", &page_src("## Page /same", "two")),
    ])
    .expect_err("two pages at one path is an error");
    assert!(
        err.contains("/same"),
        "the error names the path they fight over: {err}"
    );
}

#[test]
fn a_declared_path_must_start_with_a_slash() {
    // A path is absolute or it is not a path. `## Page about` would serve at a URL nobody can
    // predict, so it is refused where it is written rather than resolved to something plausible.
    let err = site_with(&[("app.lg", &page_src("## Page about", "a"))]);
    assert!(
        err.is_err(),
        "a path without a leading slash is refused: {err:?}"
    );
}

#[test]
fn the_pages_module_and_shim_hang_off_its_declared_path() {
    // The module's URL is derived from the page's, so a page at `/dashboard/settings` must not
    // put its module at the site root where another page's could collide with it.
    let paths =
        site_with(&[("s.lg", &page_src("## Page /dashboard/settings", "s"))]).expect("plans");
    let module = paths
        .iter()
        .find(|p| p.ends_with(".wasm"))
        .unwrap_or_else(|| panic!("a module: {paths:?}"));
    assert!(
        module.starts_with("/dashboard/"),
        "the module hangs off the page's own path: {module}"
    );
}
