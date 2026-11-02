//! ★ LOCK — one decision about what a project IS, and every command acts on it. ★
//!
//! A project is a site (pages, no backend), a full-stack site, a backend, or a program. That used
//! to be decided three times, three ways: `largo serve` asked whether the entry contained a
//! `Serve` statement, `largo build` asked the same question with a different fallback, and
//! `check`/`test`/`fmt` each tested for a `pages/` directory on their own. A pure-frontend project
//! — `pages/` beside a placeholder entry, which is what three of the five examples are — was
//! served as the PLACEHOLDER (its pages ignored) and built into a native binary of it, while
//! `check` and `test` read the pages. The first command those examples' READMEs give showed the
//! wrong thing.
//!
//! This lock holds the table — every shape has exactly one serve plan and one build plan — and
//! then drives every command through the real binary on one project of each shape, so a command
//! that re-derives a piece of the decision on its own is caught disagreeing with the rest.

#[path = "cli_common/mod.rs"]
mod common;

use common::http::{body, status, Server};
use common::{largo_in, stderr, stdout};
use logicaffeine_cli::project::shape::{BuildPlan, Project, ProjectShape, ServePlan};

use std::path::{Path, PathBuf};

const PAGE: &str = r##"# Home

## Page /

State:
    Let clicks be 0.

Actions:
    To bump:
        Set clicks to clicks + 1.

View:
    <main>
      <h1>Shape home</h1>
      <button id="more" on-click={bump}>more</button>
      <p id="count">{clicks}</p>
    </main>

## Test: the site page counts a click

Click "#more".
Expect that the text of "#count" is equal to "1".
"##;

const BACKEND: &str = r#"# Backend

## Route GET /api/hello -> Text:
    Return "hello from the backend".

## Test: the backend adds

Expect that 1 + 1 is equal to 2.

## Main

Serve on port 8080.
"#;

const PROGRAM: &str = r#"# Program

## To double (n: Int) -> Int:
    Return n * 2.

## Test: the program doubles

Expect that double(2) is equal to 4.

## Main

Show double(21).
"#;

/// The placeholder entry a pure-frontend project used to need because a manifest named one.
const PLACEHOLDER: &str = "# Placeholder\n\n## Main\n\nShow \"placeholder\".\n";

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// One project of `shape`, written by hand so this lock does not lean on the scaffolds it also
/// checks. A site declares no `entry`: a `## Page` mounts itself and needs no program.
fn project(dir: &Path, shape: ProjectShape) -> PathBuf {
    let name = match shape {
        ProjectShape::Site => "shape_site",
        ProjectShape::FullStack => "shape_fullstack",
        ProjectShape::Backend => "shape_backend",
        ProjectShape::Program => "shape_program",
    };
    let root = dir.join(name);
    match shape {
        ProjectShape::Site => {
            write(
                &root,
                "Largo.toml",
                &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
            );
        }
        _ => write(
            &root,
            "Largo.toml",
            &format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n"
            ),
        ),
    }
    match shape {
        ProjectShape::Site => {
            write(&root, "pages/app.lg", PAGE);
            write(&root, "static/style.css", "h1 { color: teal; }\n");
        }
        ProjectShape::FullStack => {
            write(&root, "src/main.lg", BACKEND);
            write(&root, "pages/app.lg", PAGE);
            write(&root, "static/style.css", "h1 { color: teal; }\n");
        }
        ProjectShape::Backend => write(&root, "src/main.lg", BACKEND),
        ProjectShape::Program => write(&root, "src/main.lg", PROGRAM),
    }
    root
}

fn shape_of(root: &Path) -> ProjectShape {
    Project::load(root)
        .expect("the project loads")
        .shape()
        .expect("the shape is decided")
}

/// The test names each shape's programs carry.
fn tests_of(shape: ProjectShape) -> Vec<&'static str> {
    match shape {
        ProjectShape::Site => vec!["the site page counts a click"],
        ProjectShape::FullStack => vec!["the site page counts a click", "the backend adds"],
        ProjectShape::Backend => vec!["the backend adds"],
        ProjectShape::Program => vec!["the program doubles"],
    }
}

/// The program files each shape has, relative to its root.
fn programs_of(shape: ProjectShape) -> Vec<&'static str> {
    match shape {
        ProjectShape::Site => vec!["pages/app.lg"],
        ProjectShape::FullStack => vec!["src/main.lg", "pages/app.lg"],
        ProjectShape::Backend | ProjectShape::Program => vec!["src/main.lg"],
    }
}

// ── the decision ─────────────────────────────────────────────────────────────────────────────

#[test]
fn every_fixture_is_the_shape_it_was_written_as() {
    let dir = tempfile::tempdir().unwrap();
    for shape in ProjectShape::ALL {
        let root = project(dir.path(), shape);
        assert_eq!(shape_of(&root), shape, "{}", root.display());
    }
}

#[test]
fn a_placeholder_entry_beside_pages_is_still_a_site() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path(), ProjectShape::Site);
    write(
        &root,
        "Largo.toml",
        "[package]\nname = \"ph\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    );
    write(&root, "src/main.lg", PLACEHOLDER);
    assert_eq!(
        shape_of(&root),
        ProjectShape::Site,
        "an entry with no `Serve` does not make a site a program — its pages are what it serves"
    );
}

#[test]
fn a_serve_in_an_imported_module_makes_a_backend() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path(), ProjectShape::Program);
    write(&root, "src/server.lg", "# Server\n\n## Route GET / -> Text:\n    Return \"hi\".\n\n## To boot ():\n    Serve on port 8080.\n");
    write(
        &root,
        "src/main.lg",
        "# Main\n\nImport Server.\n\n## Main\n\nCall boot.\n",
    );
    assert_eq!(
        shape_of(&root),
        ProjectShape::Backend,
        "the shape reads the assembled closure"
    );
}

#[test]
fn the_word_serve_in_a_string_is_not_a_backend() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path(), ProjectShape::Program);
    write(
        &root,
        "src/main.lg",
        "# Main\n\n## Main\n\nShow \"Serve on port 8080.\".\n",
    );
    assert_eq!(
        shape_of(&root),
        ProjectShape::Program,
        "an AST fact, not a substring"
    );
}

#[test]
fn static_assets_alone_make_a_site() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path(), ProjectShape::Program);
    write(&root, "static/robots.txt", "User-agent: *\n");
    assert_eq!(shape_of(&root), ProjectShape::Site);
    write(&root, "src/main.lg", BACKEND);
    assert_eq!(shape_of(&root), ProjectShape::FullStack);
}

#[test]
fn an_entry_that_does_not_parse_is_reported_not_guessed() {
    // A full-stack site whose backend has a syntax error must not quietly become a site with no
    // API: the shape cannot be decided, and the author gets the compiler's own diagnostic.
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path(), ProjectShape::FullStack);
    write(
        &root,
        "src/main.lg",
        "# Backend\n\n## Main\n\nLet x be item 0 of xs.\nServe on port 8080.\n",
    );
    let err = Project::load(&root)
        .unwrap()
        .shape()
        .expect_err("an unparseable entry has no shape");
    assert!(
        err.message.contains("main.lg:5:"),
        "the diagnostic points at the line:\n{}",
        err.message
    );
}

/// ★ THE TABLE. An exhaustive match: a new shape does not compile until it says what `serve`
/// and `build` do with it.
#[test]
fn every_shape_has_one_serve_plan_and_one_build_plan() {
    for shape in ProjectShape::ALL {
        let (serve, build) = match shape {
            ProjectShape::Site => (ServePlan::Pages { proxy: false }, BuildPlan::StaticExport),
            ProjectShape::FullStack => (
                ServePlan::Pages { proxy: true },
                BuildPlan::Binary { embeds_site: true },
            ),
            ProjectShape::Backend => (ServePlan::Backend, BuildPlan::Binary { embeds_site: false }),
            ProjectShape::Program => (ServePlan::Program, BuildPlan::Binary { embeds_site: false }),
        };
        assert_eq!(shape.serve_plan(), serve, "{shape:?}");
        assert_eq!(shape.build_plan(), build, "{shape:?}");
        assert_eq!(
            shape.has_site(),
            matches!(shape, ProjectShape::Site | ProjectShape::FullStack),
            "{shape:?}"
        );
    }
}

// ── the commands, through the real binary ────────────────────────────────────────────────────

#[test]
fn check_passes_every_shape_and_fails_on_each_of_its_programs() {
    let dir = tempfile::tempdir().unwrap();
    for shape in ProjectShape::ALL {
        let root = project(dir.path(), shape);
        let out = largo_in(&root, &["check"]);
        assert!(
            out.status.success(),
            "{shape:?} checks clean:\n{}",
            stderr(&out)
        );
        for program in programs_of(shape) {
            let file = root.join(program);
            let good = std::fs::read_to_string(&file).unwrap();
            // An unclosed element in a view, or an index of zero in a program: each fails its file.
            let broken = if program.starts_with("pages/") {
                good.replace("<p id=\"count\">{clicks}</p>", "<p id=\"count\">{clicks}")
            } else {
                good.replace("## Main\n", "## Main\n\nLet broken be item 0 of [1].\n")
            };
            assert_ne!(broken, good, "the fixture must actually break {program}");
            std::fs::write(&file, &broken).unwrap();
            let out = largo_in(&root, &["check"]);
            let err = stderr(&out);
            assert!(
                !out.status.success(),
                "{shape:?}: a broken {program} fails `largo check`:\n{err}"
            );
            let name = Path::new(program)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            assert!(
                err.contains(&format!("{name}:")),
                "{shape:?}: the failure points into {program}:\n{err}"
            );
            std::fs::write(&file, &good).unwrap();
        }
    }
}

#[test]
fn test_runs_the_tests_of_every_program_every_shape_has() {
    let dir = tempfile::tempdir().unwrap();
    for shape in ProjectShape::ALL {
        let root = project(dir.path(), shape);
        let out = largo_in(&root, &["test"]);
        let all = format!("{}{}", stdout(&out), stderr(&out));
        assert!(out.status.success(), "{shape:?}: its tests pass:\n{all}");
        for name in tests_of(shape) {
            assert!(all.contains(name), "{shape:?}: `{name}` ran:\n{all}");
        }
    }
}

#[test]
fn fmt_reaches_every_program_every_shape_has() {
    let dir = tempfile::tempdir().unwrap();
    for shape in ProjectShape::ALL {
        let root = project(dir.path(), shape);
        for program in programs_of(shape) {
            let file = root.join(program);
            let text = std::fs::read_to_string(&file).unwrap();
            std::fs::write(&file, text.replacen('\n', "   \n", 1)).unwrap();
        }
        let out = largo_in(&root, &["fmt", "--check"]);
        assert!(
            !out.status.success(),
            "{shape:?}: trailing whitespace is found"
        );
        let listed = stdout(&out);
        for program in programs_of(shape) {
            let name = Path::new(program)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            assert!(
                listed.contains(&name),
                "{shape:?}: `largo fmt --check` lists {program}:\n{listed}"
            );
        }
    }
}

#[test]
fn build_on_a_site_exports_it_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path(), ProjectShape::Site);
    let out = largo_in(&root, &["build"]);
    let said = format!("{}{}", stdout(&out), stderr(&out));
    assert!(out.status.success(), "a site builds:\n{said}");
    let index = std::fs::read_to_string(root.join("dist/index.html")).expect("dist/index.html");
    assert!(
        index.contains("<h1>Shape home</h1>"),
        "the page is pre-rendered into it:\n{index}"
    );
    assert!(
        said.contains("site") && said.contains("dist"),
        "`largo build` says the site was exported, and where:\n{said}"
    );
    assert!(
        !root.join("target").exists(),
        "no cargo project was generated — there is no program"
    );
}

#[test]
fn build_on_a_site_with_a_placeholder_entry_still_exports_the_site() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path(), ProjectShape::Site);
    write(
        &root,
        "Largo.toml",
        "[package]\nname = \"ph\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    );
    write(&root, "src/main.lg", PLACEHOLDER);
    let out = largo_in(&root, &["build"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        root.join("dist/index.html").is_file(),
        "the site, not the placeholder, is what builds"
    );
    assert!(
        !root.join("target").exists(),
        "the placeholder is never compiled to a binary"
    );
}

#[test]
fn serve_on_a_site_answers_its_pages_and_404s_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path(), ProjectShape::Site);
    let mut server = Server::start(&root);
    let port = server.port();
    let front = server.get("/");
    assert_eq!(status(&front), 200, "{}", server.stderr());
    assert!(
        body(&front).contains("<h1>Shape home</h1>"),
        "`/` is the page:\n{}",
        body(&front)
    );
    assert_eq!(status(&server.get("/style.css")), 200, "static/ is served");
    let missing = server.get("/no-such-page");
    assert_eq!(
        status(&missing),
        404,
        "with no backend, what the site lacks is a 404, not a 502"
    );
}

#[test]
fn serve_on_a_site_with_a_placeholder_entry_serves_the_pages() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path(), ProjectShape::Site);
    write(
        &root,
        "Largo.toml",
        "[package]\nname = \"ph\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    );
    write(&root, "src/main.lg", PLACEHOLDER);
    let mut server = Server::start(&root);
    let port = server.port();
    let front = body(&server.get("/"));
    assert!(
        front.contains("<h1>Shape home</h1>"),
        "`/` is the page, not the placeholder:\n{front}"
    );
}

#[test]
fn serve_on_a_full_stack_site_answers_pages_and_proxies_the_backend() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path(), ProjectShape::FullStack);
    let mut server = Server::start(&root);
    let port = server.port();
    let front = server.get("/");
    assert!(
        body(&front).contains("<h1>Shape home</h1>"),
        "{}\n{}",
        body(&front),
        server.stderr()
    );
    let api = server.get("/api/hello");
    assert_eq!(status(&api), 200, "{}", server.stderr());
    assert!(
        body(&api).contains("hello from the backend"),
        "{}",
        body(&api)
    );
}

#[test]
fn serve_on_a_backend_runs_its_serve() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path(), ProjectShape::Backend);
    let mut server = Server::start(&root);
    let port = server.port();
    let api = server.get("/api/hello");
    assert!(
        body(&api).contains("hello from the backend"),
        "{}\n{}",
        body(&api),
        server.stderr()
    );
}

#[test]
fn serve_on_a_program_serves_its_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path(), ProjectShape::Program);
    let mut server = Server::start(&root);
    let port = server.port();
    let module = server.get("/shape_program.wasm");
    assert_eq!(status(&module), 200, "{}", server.stderr());
    assert!(
        body(&module).as_bytes().starts_with(b"\0asm"),
        "the program's own module is served"
    );
}

#[test]
fn a_site_has_no_program_to_run_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path(), ProjectShape::Site);
    let out = largo_in(&root, &["run", "--interpret"]);
    let err = stderr(&out);
    assert!(!out.status.success(), "there is nothing to run");
    assert!(
        err.contains("is a site"),
        "the refusal says what the project is:\n{err}"
    );
    assert!(
        err.contains("largo serve"),
        "and what to run instead:\n{err}"
    );
}

/// ★ THE SCAFFOLDS MAKE THE SHAPE THEY PROMISE. `largo new` is where a project's shape begins, so
/// each flag is held against the one decision every other command reads.
#[test]
fn every_scaffold_is_the_shape_it_says() {
    for (flag, shape) in [
        (None, ProjectShape::Program),
        (Some("--web"), ProjectShape::FullStack),
        (Some("--site"), ProjectShape::Site),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut args = vec!["new", "made"];
        args.extend(flag);
        let out = largo_in(dir.path(), &args);
        assert!(out.status.success(), "largo new {flag:?}: {}", stderr(&out));
        assert_eq!(
            shape_of(&dir.path().join("made")),
            shape,
            "largo new {flag:?}"
        );
    }
}
