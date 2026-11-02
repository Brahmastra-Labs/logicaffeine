//! B1 — `largo new <name> --web`: the full-stack scaffold.
//!
//! Convention over manifest: a `--web` project carries a backend entry
//! (`src/main.lg` ending in a `Serve` statement), a `pages/` directory of DOM
//! pages compiled to WASM (`pages/app.lg` is the front page by convention),
//! and a `static/` directory served at the site root — with NO new Largo.toml
//! keys. The plain `largo new` output is byte-locked here so `--web` can never
//! drift it.

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo, largo_in, stderr, stdout};

/// The plain scaffold's exact bytes — the byte-lock that keeps `--web` from
/// drifting the default `largo new` output.
const PLAIN_MAIN_LG: &str =
    "# Main\n\nA simple LOGOS program.\n\n## Main\n\nShow \"Hello, world!\".\n";
const PLAIN_GITIGNORE: &str = "/target\n";
/// `Manifest::new(name).to_toml()` as written today, pinned byte-for-byte.
fn plain_largo_toml(name: &str) -> String {
    format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nauthors = []\nentry = \"src/main.lg\"\n\n[dependencies]\n"
    )
}

/// Every file under `dir`, as sorted root-relative slash paths.
fn file_set(dir: &std::path::Path) -> Vec<String> {
    fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<String>) {
        for f in std::fs::read_dir(dir).unwrap().flatten() {
            let p = f.path();
            if p.is_dir() {
                walk(&p, root, out);
            } else {
                out.push(
                    p.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// A page, plus a driving verb to run against it.
///
/// The verb needs a `## Main` to sit in. A `##` block runs until the next `##` header, so a bare
/// sentence appended to a file whose last block is a `## Page` lands INSIDE that page — and the
/// language says so, by name: *"`Click` is not part of a component. Its sections are `State:` …"*.
/// That refusal is right, and it is the harness's job to splice correctly rather than the
/// language's to guess. Written once here so the next driving test does not rediscover it.
fn driven_by(page: &str, verb: &str) -> String {
    format!("{page}\n## Main\n{verb}\n")
}

#[test]
fn plain_new_output_is_byte_locked() {
    let dir = tempfile::tempdir().unwrap();
    let out = largo_in(dir.path(), &["new", "plainx"]);
    assert!(out.status.success(), "largo new: {}", stderr(&out));
    let root = dir.path().join("plainx");

    // Exactly the four classic files — no pages/, no static/, nothing extra.
    assert_eq!(
        file_set(&root),
        vec![".gitignore", "AGENTS.md", "Largo.toml", "src/main.lg"],
        "the plain scaffold's file set is locked"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("Largo.toml")).unwrap(),
        plain_largo_toml("plainx"),
        "plain Largo.toml is byte-locked"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("src/main.lg")).unwrap(),
        PLAIN_MAIN_LG,
        "plain src/main.lg is byte-locked"
    );
    assert_eq!(
        std::fs::read_to_string(root.join(".gitignore")).unwrap(),
        PLAIN_GITIGNORE,
        "plain .gitignore is byte-locked"
    );
}

#[test]
fn web_scaffold_has_backend_pages_and_static() {
    let dir = tempfile::tempdir().unwrap();
    let out = largo_in(dir.path(), &["new", "mysite", "--web"]);
    assert!(out.status.success(), "largo new --web: {}", stderr(&out));
    let root = dir.path().join("mysite");

    // The three-zone shape: backend entry + pages + static, plus the classics.
    //
    // `static/theme.css` joined deliberately: the scaffolded stylesheet referenced four design
    // tokens (`--border`, `--radius`, `--shadow`, `--accent`) that NOTHING defined, so every
    // declaration using one was silently dropped and the card had no border, no shadow and no
    // colour — CSS reports an undefined custom property as nothing at all. The tokens live in
    // their own file because a project owns its theme and may edit or delete it
    // (`web_theme_lock`).
    assert_eq!(
        file_set(&root),
        vec![
            ".gitignore",
            "AGENTS.md",
            "Largo.toml",
            "pages/app.lg",
            "src/main.lg",
            "static/style.css",
            "static/theme.css",
        ],
        "the --web scaffold's file set"
    );

    // NO new manifest keys: the Largo.toml is byte-identical to the plain one.
    assert_eq!(
        std::fs::read_to_string(root.join("Largo.toml")).unwrap(),
        plain_largo_toml("mysite"),
        "--web adds no Largo.toml keys (convention over manifest)"
    );

    // The backend entry is a real web program: routes + the Serve statement that runs them — the
    // canonical `## Route` form, not a hand-written dispatcher.
    let main = std::fs::read_to_string(root.join("src/main.lg")).unwrap();
    assert!(
        main.contains("Serve on port 8080."),
        "the backend ends in a Serve statement:\n{main}"
    );
    assert!(
        !main.contains("with handle"),
        "the backend serves its routes, not a hand-written `## To handle` dispatcher:\n{main}"
    );
    assert!(
        main.matches("## Route ").count() >= 2,
        "the backend carries a few routes to learn from:\n{main}"
    );

    // The front page is a DOM app. Migrated from the retiring dialect's `Render:` to LGX's
    // `View:` — and the assertion is only the shallow half now: the tests at the bottom of this
    // file RUN the scaffolded page on both tiers and click its button, because "it contains a
    // view" was true of the old scaffold whose counter never moved.
    let page = std::fs::read_to_string(root.join("pages/app.lg")).unwrap();
    assert!(
        page.contains("View:"),
        "pages/app.lg renders a view:\n{page}"
    );

    // The stylesheet is real.
    let css = std::fs::read_to_string(root.join("static/style.css")).unwrap();
    assert!(
        css.contains('{') && css.contains('}'),
        "static/style.css is real CSS:\n{css}"
    );
}

#[test]
fn web_scaffold_passes_largo_check() {
    let dir = tempfile::tempdir().unwrap();
    let out = largo_in(dir.path(), &["new", "mysite", "--web"]);
    assert!(out.status.success(), "largo new --web: {}", stderr(&out));

    let check = largo()
        .args(["check"])
        .current_dir(dir.path().join("mysite"))
        .output()
        .expect("largo check spawns");
    assert!(
        check.status.success(),
        "the --web scaffold must check clean\nstdout: {}\nstderr: {}",
        stdout(&check),
        stderr(&check)
    );
}

// ---- The scaffolded page is LGX, and it RUNS. ----

/// ★ `largo new --web` scaffolded the RETIRING DIALECT — `a signal`, `Render:`, `on click:` —
/// so the first thing a new user saw was the surface being deleted. Worse, the audit that started
/// this campaign recorded that the scaffolded counter never re-rendered: the page shipped as an
/// example did not work.
///
/// Locked by RUNNING it, not by reading it. A scaffold checked with `contains()` can be a page
/// that parses and does nothing, which is exactly what it was.
#[test]
fn the_scaffolded_page_is_lgx_and_not_the_retiring_dialect() {
    let dir = tempfile::tempdir().unwrap();
    let out = largo_in(dir.path(), &["new", "site", "--web"]);
    assert!(
        out.status.success(),
        "scaffold must succeed: {}",
        stderr(&out)
    );
    let page = std::fs::read_to_string(dir.path().join("site/pages/app.lg")).unwrap();

    for dead in ["a signal", "Render:", "on click:", "bound to"] {
        assert!(
            !page.contains(dead),
            "the scaffold still teaches `{dead}`, which is the dialect being retired:\n{page}"
        );
    }
    for live in ["State:", "Actions:", "View:"] {
        assert!(
            page.contains(live),
            "the scaffold must be LGX; `{live}` is missing:\n{page}"
        );
    }
    // ★ A PAGE OR A COMPONENT — both are LGX, and this test's subject is the DIALECT, not which
    // header. `## Page /` is what a file in `pages/` should say: it owns a URL and mounts itself,
    // so the scaffold needs no `## Main` and no `Mount`. `## Component` remains first-class — it is
    // the piece a page PLACES (`<Row todo={t}/>`) — so either header is a scaffold that teaches
    // the living dialect. Asserting the literal `## Component` was asserting the ceremony rather
    // than the language.
    assert!(
        page.contains("## Page ") || page.contains("## Component "),
        "the scaffold must declare a page or a component:\n{page}"
    );
}

#[test]
fn the_scaffolded_page_actually_runs_and_its_button_works() {
    // The whole point. The previous scaffold PARSED and its counter never moved — a page shipped
    // as the example of how to write one, that did not work.
    let dir = tempfile::tempdir().unwrap();
    let out = largo_in(dir.path(), &["new", "site", "--web"]);
    assert!(
        out.status.success(),
        "scaffold must succeed: {}",
        stderr(&out)
    );
    let page = std::fs::read_to_string(dir.path().join("site/pages/app.lg")).unwrap();

    // Drive it the way a visitor would, on the reference tier.
    let driven = driven_by(&page, "Click \"#more\".");
    logicaffeine_compile::semantics::dom::reset();
    let run = logicaffeine_compile::compile::tw_outcome(&driven);
    assert_eq!(run.error, None, "the scaffolded page must RUN:\n{driven}");
    let rendered = logicaffeine_compile::semantics::dom::render();
    assert!(
        rendered.contains("Clicks so far: 1"),
        "the scaffolded button must actually count — this is the bug the audit found. \
         Rendered:\n{rendered}"
    );
}

#[test]
fn the_scaffolded_page_runs_on_the_bytecode_tier_too() {
    // A page that only works on the reference tier is a page that does not work in a browser.
    let dir = tempfile::tempdir().unwrap();
    let out = largo_in(dir.path(), &["new", "site", "--web"]);
    assert!(
        out.status.success(),
        "scaffold must succeed: {}",
        stderr(&out)
    );
    let page = std::fs::read_to_string(dir.path().join("site/pages/app.lg")).unwrap();
    let driven = driven_by(&page, "Click \"#more\".");

    logicaffeine_compile::semantics::dom::reset();
    assert_eq!(
        logicaffeine_compile::compile::tw_outcome(&driven).error,
        None
    );
    let expected = logicaffeine_compile::semantics::dom::render();

    logicaffeine_compile::semantics::dom::reset();
    let vm = logicaffeine_compile::compile::vm_outcome(&driven);
    assert_eq!(
        vm.error, None,
        "the scaffolded page must run on the VM:\n{driven}"
    );
    assert_eq!(
        logicaffeine_compile::semantics::dom::render(),
        expected,
        "the two tiers must render the scaffolded page identically"
    );
}

#[test]
fn the_scaffolded_page_compiles_to_wasm() {
    // It is served to a browser, so it has to reach one.
    let dir = tempfile::tempdir().unwrap();
    let out = largo_in(dir.path(), &["new", "site", "--web"]);
    assert!(
        out.status.success(),
        "scaffold must succeed: {}",
        stderr(&out)
    );
    let page = std::fs::read_to_string(dir.path().join("site/pages/app.lg")).unwrap();
    assert!(
        logicaffeine_compile::compile::compile_to_wasm(&page).is_ok(),
        "the scaffolded page must compile to wasm — it is what gets served:\n{page}"
    );
}

// ---- A fresh project is green on every command a README gives. ----

/// Scaffold `largo new <name> <flag>` and hand back its root.
fn fresh(dir: &std::path::Path, name: &str, flag: &str) -> std::path::PathBuf {
    let out = largo_in(dir, &["new", name, flag]);
    assert!(
        out.status.success(),
        "largo new {name} {flag}: {}",
        stderr(&out)
    );
    dir.join(name)
}

/// ★ THE FIRST THREE COMMANDS ANYBODY RUNS ON A NEW SITE. `largo check` passes, `largo test` runs
/// the front page's own test and passes it, and `largo serve` answers `/` with the page — for both
/// site scaffolds. A scaffold that fails any of them teaches the reader the tool is broken before
/// they have written a line.
#[test]
fn a_fresh_site_checks_tests_and_serves_its_front_page() {
    for flag in ["--web", "--site"] {
        let dir = tempfile::tempdir().unwrap();
        let root = fresh(dir.path(), "fresh", flag);

        let check = largo_in(&root, &["check"]);
        assert!(
            check.status.success(),
            "{flag}: `largo check` on a fresh project:\n{}",
            stderr(&check)
        );

        let test = largo_in(&root, &["test"]);
        let said = format!("{}{}", stdout(&test), stderr(&test));
        assert!(
            test.status.success(),
            "{flag}: `largo test` on a fresh project:\n{said}"
        );
        assert!(
            said.contains("the button counts clicks"),
            "{flag}: the page's own test ran:\n{said}"
        );

        let mut server = common::http::Server::start(&root);
        let port = server.port();
        let front = server.get("/");
        let body = common::http::body(&front);
        assert_eq!(
            common::http::status(&front),
            200,
            "{flag}: {}",
            server.stderr()
        );
        assert!(
            body.contains("fresh is alive"),
            "{flag}: `/` is the front page:\n{body}"
        );
        assert!(
            body.contains("Clicks so far: 0"),
            "{flag}: pre-rendered with its state:\n{body}"
        );
    }
}

/// `--site` writes no program: a page mounts itself, and a placeholder entry would be a file that
/// looks like it runs and does not.
#[test]
fn a_site_scaffold_has_pages_and_assets_and_no_program() {
    let dir = tempfile::tempdir().unwrap();
    let root = fresh(dir.path(), "brochure", "--site");
    assert_eq!(
        file_set(&root),
        vec![
            ".gitignore",
            "AGENTS.md",
            "Largo.toml",
            "pages/app.lg",
            "static/style.css",
            "static/theme.css"
        ],
        "the --site scaffold's file set"
    );
    let manifest = std::fs::read_to_string(root.join("Largo.toml")).unwrap();
    assert!(
        !manifest.contains("entry"),
        "a site names no entry program:\n{manifest}"
    );
    assert!(manifest.contains("name = \"brochure\""), "{manifest}");
    let page = std::fs::read_to_string(root.join("pages/app.lg")).unwrap();
    for part in ["## Page /", "## Component ", "## Test: "] {
        assert!(
            page.contains(part),
            "the front page teaches `{part}`:\n{page}"
        );
    }
    assert!(
        !page.contains("## Main") && !page.contains("Mount "),
        "a page mounts itself:\n{page}"
    );
}

#[test]
fn web_and_site_cannot_both_be_asked_for() {
    let dir = tempfile::tempdir().unwrap();
    let out = largo_in(dir.path(), &["new", "both", "--web", "--site"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "a contradiction is a usage error: {}",
        stderr(&out)
    );
    assert!(!dir.path().join("both").exists(), "and nothing is written");
}
