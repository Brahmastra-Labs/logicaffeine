//! ★ LOCK — a project written in `.logos` files is a whole project. ★
//!
//! `.logos` is the language's own name for its files and the one the Studio writes, but largo
//! only ever looked for `.lg`. A `.logos` page was not a page: `largo build --static` exported a
//! site with it missing, and a site of nothing but `.logos` pages exported NO pages, which a
//! publish that syncs with `--delete` turns into an empty website. `largo check` called the broken
//! page green, `largo test` never ran its tests, `largo fmt` never formatted it, a bare `Import`
//! never found a `.logos` module, the dev server never saw an edit to one, and every message that
//! did mention a page named a `.lg` file that is not on disk.
//!
//! Every test here drives the real binary on a project written by hand, the way a person lays one
//! out, and `.lg` keeps working beside it.

#[path = "cli_common/mod.rs"]
mod common;
use common::http::{body, Server};
use common::{largo_in, stderr, stdout};

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A project at `dir/name` holding `files` (path, contents), plus a manifest. `entry` of `None`
/// leaves the manifest's `entry` out, so the default applies.
fn project(dir: &Path, name: &str, entry: Option<&str>, files: &[(&str, &str)]) -> PathBuf {
    let root = dir.join(name);
    let entry_line = entry.map(|e| format!("entry = \"{e}\"\n")).unwrap_or_default();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("Largo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n{entry_line}\n[dependencies]\n"),
    )
    .unwrap();
    for (rel, text) in files {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    root
}

const MAIN: &str = "# Site\n\n## Main\n\nShow \"ok\".\n";

const FRONT: &str =
    "## Page /\n\nHead:\n    <title>Acme</title>\n\nView:\n    <main><h1>Acme front</h1></main>\n";

const ABOUT: &str = "## Page /about\n\nView:\n    <main><h1>About Acme</h1></main>\n";

/// A page that cannot compile: its `<div>` never closes.
const BROKEN: &str = "## Page /\n\nView:\n    <div><p>open</p>\n";

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{} must exist: {e}", path.display()))
}

/// ★ THE LOAD-BEARING TEST — a site written in `.logos` exports every page.
#[test]
fn a_logos_site_exports_every_page() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(
        dir.path(),
        "acme",
        Some("src/main.logos"),
        &[("src/main.logos", MAIN), ("pages/app.logos", FRONT), ("pages/about.logos", ABOUT)],
    );
    let out = largo_in(&root, &["build", "--static"]);
    assert!(out.status.success(), "`largo build --static` failed:\n{}", stderr(&out));
    assert!(
        read(&root.join("dist/index.html")).contains("Acme front"),
        "pages/app.logos is the front page"
    );
    assert!(
        read(&root.join("dist/about/index.html")).contains("About Acme"),
        "pages/about.logos serves /about"
    );
}

/// `.lg` keeps working, beside `.logos`, in one site.
#[test]
fn lg_and_logos_pages_export_side_by_side() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(
        dir.path(),
        "mixed",
        Some("src/main.logos"),
        &[("src/main.logos", MAIN), ("pages/app.lg", FRONT), ("pages/about.logos", ABOUT)],
    );
    let out = largo_in(&root, &["build", "--static"]);
    assert!(out.status.success(), "`largo build --static` failed:\n{}", stderr(&out));
    assert!(read(&root.join("dist/index.html")).contains("Acme front"), "the .lg page");
    assert!(read(&root.join("dist/about/index.html")).contains("About Acme"), "the .logos page");
}

/// A page's name is its file's stem, so `app.lg` beside `app.logos` is one page written twice.
/// Which one shipped would depend on the order a directory is read in; the build says so instead,
/// naming both files.
#[test]
fn a_page_written_twice_is_refused_naming_both_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(
        dir.path(),
        "twins",
        Some("src/main.logos"),
        &[("src/main.logos", MAIN), ("pages/app.lg", FRONT), ("pages/app.logos", ABOUT)],
    );
    let out = largo_in(&root, &["build", "--static"]);
    let err = stderr(&out);
    assert!(!out.status.success(), "two files for one page must not build:\n{err}");
    assert!(
        err.contains("pages/app.lg") && err.contains("pages/app.logos"),
        "the refusal names both files: {err}"
    );
}

/// A page's own mistake names the file it is in — `pages/about.logos`, not a `.lg` that is not
/// on disk.
#[test]
fn a_bad_page_path_names_the_logos_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(
        dir.path(),
        "badpath",
        Some("src/main.logos"),
        &[
            ("src/main.logos", MAIN),
            ("pages/app.logos", FRONT),
            ("pages/about.logos", "## Page about\n\nView:\n    <p>About</p>\n"),
        ],
    );
    let out = largo_in(&root, &["build", "--static"]);
    let err = stderr(&out);
    assert!(!out.status.success(), "a relative page path must not build:\n{err}");
    assert!(err.contains("pages/about.logos"), "the message names the file on disk: {err}");
    assert!(!err.contains("about.lg"), "and no file that is not there: {err}");
}

#[test]
fn largo_check_fails_on_a_broken_logos_page_and_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(
        dir.path(),
        "checked",
        Some("src/main.logos"),
        &[("src/main.logos", MAIN), ("pages/app.logos", BROKEN)],
    );
    let out = largo_in(&root, &["check"]);
    let err = stderr(&out);
    assert!(!out.status.success(), "`largo check` passed a page that does not compile:\n{err}");
    assert!(err.contains("app.logos"), "the failure names the page's file: {err}");
    assert!(!err.contains("app.lg"), "and no file that is not there: {err}");
}

#[test]
fn largo_test_runs_the_tests_written_in_a_logos_page() {
    let page = "## Page /\n\nState:\n    Let clicks be 0.\n\nActions:\n    To bump:\n        Set clicks to clicks + 1.\n\nView:\n    <main>\n      <button id=\"more\" on-click={bump}>Click me</button>\n      <p id=\"count\">{clicks}</p>\n    </main>\n\n## Test: the logos button counts\n\nClick \"#more\".\nExpect that the text of \"#count\" is equal to \"1\".\n";
    let dir = tempfile::tempdir().unwrap();
    let root = project(
        dir.path(),
        "tested",
        Some("src/main.logos"),
        &[("src/main.logos", MAIN), ("pages/app.logos", page)],
    );
    let out = largo_in(&root, &["test"]);
    let all = format!("{}{}", stdout(&out), stderr(&out));
    assert!(all.contains("the logos button counts"), "the page's own test runs:\n{all}");
    assert!(out.status.success(), "and passes:\n{all}");
}

#[test]
fn largo_fmt_reaches_logos_pages_and_modules() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(
        dir.path(),
        "formatted",
        Some("src/main.logos"),
        &[
            ("src/main.logos", MAIN),
            ("src/shared.logos", "# Shared\n\n## To twice (n: Int) -> Int:   \n    Return n times 2.\n"),
            ("pages/app.logos", "## Page /   \n\nView:\n    <p>hi</p>\n"),
        ],
    );
    let out = largo_in(&root, &["fmt"]);
    assert!(out.status.success(), "largo fmt failed: {}", stderr(&out));
    assert!(
        !read(&root.join("pages/app.logos")).contains("## Page /   "),
        "`largo fmt` left pages/app.logos unformatted"
    );
    assert!(
        !read(&root.join("src/shared.logos")).contains("-> Int:   "),
        "`largo fmt` left src/shared.logos unformatted"
    );
}

/// A page names a module the way a program does — `Import Shared.` finds `src/shared.logos`.
#[test]
fn a_page_imports_a_logos_module_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(
        dir.path(),
        "imports",
        Some("src/main.logos"),
        &[
            ("src/main.logos", MAIN),
            ("src/shared.logos", "# Shared\n\n## To greeting -> Text:\n    Return \"hello from shared\".\n"),
            ("pages/app.logos", "# App\n\nImport Shared.\n\n## Page /\n\nState:\n    Let line be greeting().\n\nView:\n    <p>{line}</p>\n"),
        ],
    );
    let out = largo_in(&root, &["check"]);
    assert!(
        out.status.success(),
        "`largo check` could not build a page that imports src/shared.logos:\n{}",
        stderr(&out)
    );
    let out = largo_in(&root, &["build", "--static"]);
    assert!(out.status.success(), "`largo build --static` failed:\n{}", stderr(&out));
    assert!(
        read(&root.join("dist/index.html")).contains("hello from shared"),
        "the page that imports src/shared.logos was exported with the module's answer"
    );
}

/// A program imports a `.logos` module beside it, and a module that is not there is reported with
/// every file name that was looked for.
#[test]
fn a_program_imports_a_logos_module_and_a_missing_one_names_every_extension() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(
        dir.path(),
        "program",
        Some("src/main.logos"),
        &[
            ("src/main.logos", "# Main\n\nImport Shared.\n\n## Main\n\nShow greeting().\n"),
            ("src/shared.logos", "# Shared\n\n## To greeting -> Text:\n    Return \"hello from shared\".\n"),
        ],
    );
    let out = largo_in(&root, &["run", "--interpret"]);
    let all = format!("{}{}", stdout(&out), stderr(&out));
    assert!(out.status.success(), "`largo run --interpret` failed:\n{all}");
    assert!(all.contains("hello from shared"), "the module answered:\n{all}");

    std::fs::write(root.join("src/main.logos"), "# Main\n\nImport Missing.\n\n## Main\n\nShow 1.\n")
        .unwrap();
    let out = largo_in(&root, &["check"]);
    let err = format!("{}{}", stdout(&out), stderr(&out));
    assert!(!out.status.success(), "a missing module must not check:\n{err}");
    for want in ["`missing.logos`", "`missing.lg`", "`missing.md`"] {
        assert!(err.contains(want), "the message names {want}:\n{err}");
    }
}

/// A manifest with no `entry` means `src/main`, whichever source extension it is written in.
#[test]
fn the_default_entry_may_be_written_in_logos() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(
        dir.path(),
        "plain",
        None,
        &[("src/main.logos", "## Main\n\nShow \"from main.logos\".\n")],
    );
    let out = largo_in(&root, &["run", "--interpret"]);
    let all = format!("{}{}", stdout(&out), stderr(&out));
    assert!(out.status.success(), "`largo run --interpret` failed:\n{all}");
    assert!(all.contains("from main.logos"), "the default entry ran:\n{all}");
}

/// A published package carries its `.logos` sources.
#[test]
fn a_package_carries_its_logos_sources() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(
        dir.path(),
        "packaged",
        Some("src/main.logos"),
        &[("src/main.logos", MAIN), ("src/geometry.logos", "# Geometry\n")],
    );
    let tarball = logicaffeine_cli::project::create_tarball(&root).expect("a package");
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(&tarball[..]));
    let names: Vec<String> = archive
        .entries()
        .expect("entries")
        .map(|e| e.expect("an entry").path().expect("a path").display().to_string())
        .collect();
    for want in ["src/main.logos", "src/geometry.logos"] {
        assert!(names.iter().any(|n| n == want), "the package holds {want}: {names:?}");
    }
}

/// `largo serve` serves a `.logos` page, and sees an edit to it.
#[test]
fn the_dev_server_serves_a_logos_page_and_sees_its_edits() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(
        dir.path(),
        "served",
        None,
        &[("pages/app.logos", FRONT), ("pages/about.logos", ABOUT)],
    );
    let mut server = Server::start(&root);
    assert!(body(&server.get("/")).contains("Acme front"), "pages/app.logos serves /");
    assert!(body(&server.get("/about")).contains("About Acme"), "pages/about.logos serves /about");

    std::thread::sleep(Duration::from_millis(1100));
    std::fs::write(
        root.join("pages/about.logos"),
        "## Page /about\n\nView:\n    <main><h1>About Acme, edited</h1></main>\n",
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if body(&server.get("/about")).contains("About Acme, edited") {
            break;
        }
        assert!(Instant::now() < deadline, "the dev server never picked up the edit to pages/about.logos");
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// The dev server rebuilds a page when a `.logos` module it imports from `src/` changes, and
/// restarts a backend whose `.logos` source changes.
#[test]
fn the_dev_server_sees_an_edit_to_an_imported_logos_module() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(
        dir.path(),
        "modules",
        None,
        &[
            ("src/shared.logos", "# Shared\n\n## To greeting -> Text:\n    Return \"first words\".\n"),
            ("pages/app.logos", "# App\n\nImport Shared.\n\n## Page /\n\nState:\n    Let line be greeting().\n\nView:\n    <p>{line}</p>\n"),
        ],
    );
    let mut server = Server::start(&root);
    assert!(body(&server.get("/")).contains("first words"), "the page imports src/shared.logos");

    std::thread::sleep(Duration::from_millis(1100));
    std::fs::write(
        root.join("src/shared.logos"),
        "# Shared\n\n## To greeting -> Text:\n    Return \"second words\".\n",
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if body(&server.get("/")).contains("second words") {
            break;
        }
        assert!(Instant::now() < deadline, "the dev server never picked up the edit to src/shared.logos");
        std::thread::sleep(Duration::from_millis(150));
    }
}
