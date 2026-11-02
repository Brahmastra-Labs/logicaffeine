//! ★ LOCK — every example in `examples/` still builds, and still does what it claims. ★
//!
//! An example is the first LOGOS anybody reads and the first they copy. One that has
//! quietly stopped building is worse than no example at all: it teaches a shape that does
//! not work, and the person who copies it has no way to tell whether the fault is theirs.
//!
//! `examples/fullstack_todo` had a README whose documented run command did not exist and a
//! deployment section describing a path superseded two phases earlier — because nothing
//! ever ran them.
//!
//! ★ DISCOVERED, NOT LISTED. The example set comes from the directory, so a new example is
//! covered the moment it exists and nobody has to remember to add it here. A list would go
//! stale in exactly the way this file exists to prevent — and this campaign has watched a
//! hand-written list of test binaries leave eight of them running nowhere for months.
//!
//! ★ WHAT EACH ONE IS ASKED IS DERIVED FROM WHAT IT IS: every project must `check`; a
//! project with `pages/` must export a site; a project that carries a `## Test` must pass
//! it. So the demands grow with the example rather than being restated per project.

#[path = "cli_common/mod.rs"]
mod common;
mod page_coverage;

use common::http::{body, status, Server};
use common::stderr;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The repository's `examples/` directory.
fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .canonicalize()
        .expect("../../apps/logicaffeine_cli/examples/")
}

/// Every example project, by name — a directory holding a `Largo.toml`.
fn examples() -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(examples_dir())
        .expect("examples/ is readable")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("Largo.toml").is_file())
        .map(|p| (p.file_name().unwrap().to_string_lossy().to_string(), p))
        .collect();
    out.sort();
    out
}

/// Copy an example into a scratch directory.
///
/// ★ A COPY, because these commands WRITE — `largo build --static` leaves a `dist/`, the
/// guestbook leaves its book on disk. A test that dirties the repository it is checking is
/// a test people learn to run with a `git checkout` afterwards.
fn scratch(src: &Path, into: &Path) -> PathBuf {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let p = e.path();
            let name = e.file_name();
            // `target/` and `dist/` are build output; copying them is slow and proves nothing.
            if name == "target" || name == "dist" {
                continue;
            }
            if p.is_dir() {
                copy(&p, &to.join(name));
            } else {
                std::fs::copy(&p, to.join(name)).unwrap();
            }
        }
    }
    let dest = into.join(src.file_name().unwrap());
    copy(src, &dest);
    dest
}

fn largo_in(dir: &Path, args: &[&str]) -> Output {
    common::largo()
        .args(args)
        .current_dir(dir)
        .output()
        .expect("largo runs")
}

/// Does any `.lg` file in the project carry a `## Test` block?
fn has_tests(root: &Path) -> bool {
    fn walk(d: &Path, found: &mut bool) {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() && e.file_name() != "target" && e.file_name() != "dist" {
                walk(&p, found);
            } else if p.extension().and_then(|x| x.to_str()) == Some("lg") {
                if std::fs::read_to_string(&p)
                    .unwrap_or_default()
                    .contains("## Test")
                {
                    *found = true;
                }
            }
        }
    }
    let mut found = false;
    walk(root, &mut found);
    found
}

/// The set is not empty, and the discovery actually found the projects — a sweep over
/// nothing passes silently, which is the one way this whole file could be worthless.
#[test]
fn the_examples_are_discovered() {
    let found: Vec<String> = examples().into_iter().map(|(n, _)| n).collect();
    assert!(
        found.len() >= 4,
        "the sweep must find the example projects; it found {found:?}"
    );
    for expected in [
        "fullstack_todo",
        "guestbook",
        "marketing_site",
        "tic_tac_toe",
    ] {
        assert!(
            found.contains(&expected.to_string()),
            "{expected} is missing from {found:?}"
        );
    }
}

/// ★ THE LOAD-BEARING TEST — every example compiles, on the tier it ships on.
#[test]
fn every_example_checks() {
    for (name, path) in examples() {
        let dir = tempfile::tempdir().unwrap();
        let root = scratch(&path, dir.path());
        let out = largo_in(&root, &["check"]);
        assert!(
            out.status.success(),
            "examples/{name} does not check. An example that has stopped building teaches a \
             shape that does not work, to somebody with no way of telling whose fault it \
             is.\n{}",
            stderr(&out)
        );
    }
}

/// The program an example's manifest names as its entry, when the file exists.
fn entry_program(root: &Path) -> Option<PathBuf> {
    let manifest = std::fs::read_to_string(root.join("Largo.toml")).ok()?;
    let entry = manifest
        .lines()
        .find_map(|l| l.trim().strip_prefix("entry = "))
        .map(|e| e.trim_matches('"').to_string())
        .unwrap_or_else(|| "src/main.lg".to_string());
    Some(root.join(entry)).filter(|p| p.is_file())
}

/// ★ EVERY EXAMPLE PROGRAM BUILDS ON THE TIER IT SHIPS AS. ★
///
/// `largo check` compiles the entry to Rust and discards it, and that compile returns `Ok` for a
/// program it refused — the refusal is a `compile_error!` inside the Rust — so an example can check
/// clean and still not `largo build`. This reads the Rust `largo build` compiles: the entry's whole
/// project, with the site's assets embedded when it has pages.
#[test]
fn every_example_program_builds_on_the_deploy_tier() {
    let mut built = 0usize;
    for (name, path) in examples() {
        let Some(entry) = entry_program(&path) else {
            continue;
        };
        let compile = || logicaffeine_compile::compile::compile_project(&entry);
        let compiled = if path.join("pages").is_dir() {
            logicaffeine_compile::codegen::with_embed_assets(compile)
        } else {
            compile()
        };
        let rust = compiled
            .unwrap_or_else(|e| panic!("examples/{name} does not compile: {e}"))
            .rust_code;
        let refusals: Vec<&str> = rust
            .lines()
            .filter(|l| l.contains("compile_error!"))
            .map(str::trim)
            .collect();
        assert!(
            refusals.is_empty(),
            "examples/{name} checks clean and does not build: `largo build` compiles {} refusal(s) into \
             its binary.\n{}",
            refusals.len(),
            refusals.join("\n")
        );
        built += 1;
    }
    assert!(
        built >= 3,
        "the sweep must reach the examples that carry a program; it built {built}"
    );
}

/// ★ AND THE BINARY IS BUILT. ★ Reading the generated Rust for a refusal is not building it: a
/// program whose functions reach its own bindings once emitted Rust with no `compile_error!` in it
/// and no chance of compiling — every example server with a `## Session`. This hands each example
/// program to `largo build`, the command its README gives, and requires a binary.
#[test]
#[ignore = "compiles an AOT binary per example via cargo (slow); run in the full suite"]
fn every_example_program_builds_into_a_binary() {
    let cache = std::env::temp_dir().join("logos_e2e_cache");
    let scratch_root = tempfile::tempdir().unwrap();
    let mut failed = Vec::new();
    let mut built = 0usize;
    for (name, path) in examples() {
        if entry_program(&path).is_none() {
            continue;
        }
        let root = scratch(&path, scratch_root.path());
        let out = common::largo()
            .arg("build")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", &cache)
            .output()
            .expect("largo runs");
        if out.status.success() {
            built += 1;
        } else {
            failed.push(format!(
                "examples/{name}:\n{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    assert!(
        failed.is_empty(),
        "`largo build` must produce every example's binary:\n{}",
        failed.join("\n\n")
    );
    assert!(
        built >= 3,
        "the sweep must reach the examples that carry a program; it built {built}"
    );
}

/// An example with a `pages/` directory is a website, so it must still export as one.
///
/// ★ THE FRONT PAGE'S OWN URL DECIDES WHERE IT EXPORTS TO, NOT `dist/index.html` UNCONDITIONALLY.
/// `examples/signup_form` deliberately declares `## Page /signup` (its README documents
/// `largo serve` at exactly that path) — a real, working example with no `/` page at all, so its
/// static export has no `dist/index.html` to find and never will. Checking the wrong path made a
/// correctly-exporting example look broken.
#[test]
fn every_example_with_pages_exports_a_site() {
    for (name, path) in examples() {
        if !path.join("pages").is_dir() {
            continue;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = scratch(&path, dir.path());
        let out = largo_in(&root, &["build", "--static"]);
        assert!(
            out.status.success(),
            "examples/{name} has pages/ and does not export: {}",
            stderr(&out)
        );
        let page = front_page(&root).unwrap_or_else(|| {
            panic!("examples/{name}: no front page found to check the export against")
        });
        let index = front_page_export_path(&root, &page);
        assert!(
            index.is_file(),
            "examples/{name}: {} must exist (front page declares `{}`)",
            index.strip_prefix(&root).unwrap_or(&index).display(),
            declared_page_path(&page).as_deref().unwrap_or("/")
        );
        let html = std::fs::read_to_string(&index).unwrap();
        assert!(
            html.contains("<title>"),
            "examples/{name}: the exported front page must be a titled document:\n{html}"
        );
    }
}

/// ★ AN EXAMPLE THAT CARRIES A TEST MUST PASS IT. The `## Test` in an example is the part
/// a reader trusts most — it is the author saying "this is what it does" in a form the
/// machine checks.
///
/// ★ EVERY EXAMPLE RUNS, EVEN AFTER ONE FAILS. An `assert!` inside the sweep stops at the FIRST
/// failing example — a second, unrelated regression behind it in the corpus stays invisible until
/// the first is fixed and this test is run again, one failure at a time. Failures are collected
/// across the WHOLE corpus and reported together, so a single run names everything currently wrong.
#[test]
fn every_example_passes_its_own_tests() {
    // ★ A SWEEP OVER NOTHING PASSES SILENTLY. Every filtered sweep in this file skips
    // projects by design, so at least one has to be exercised or the assertion below is
    // never reached and the test is a green that means nothing. Counted, not assumed.
    let mut ran = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for (name, path) in examples() {
        if !has_tests(&path) {
            continue;
        }
        ran += 1;
        let dir = tempfile::tempdir().unwrap();
        let root = scratch(&path, dir.path());
        let out = largo_in(&root, &["test"]);
        if !out.status.success() {
            failures.push(format!(
                "examples/{name} carries a `## Test` that does not pass: {}",
                stderr(&out)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {ran} examples failed their own tests:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
    assert!(
        ran > 0,
        "no example carried a `## Test` — the sweep proved nothing"
    );
}

/// Every example says what it is for. A directory of LOGOS with no README is a puzzle.
/// Phrases that claim the FRAMEWORK cannot do something, as opposed to noting a choice the
/// example made. A claim of the first kind is the one that rots.
const CAPABILITY_NEGATIONS: &[&str] = &[
    "cannot",
    "is not lowered",
    "does not lower",
    "not supported",
    "not yet",
    "is impossible",
];

/// The headings under which an example writes down what it cannot do.
fn is_limits_heading(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    l.starts_with('#')
        && (l.contains("honest limit")
            || l.contains("honest gap")
            || l.contains("known behavior")
            || l.contains("known behaviour")
            || l.contains("known gap"))
}

/// ★ A LIMITATION AN EXAMPLE CLAIMS MUST NAME THE TEST THAT PROVES IT. ★
///
/// `examples/guestbook/README.md` told readers that `## Route … -> Response` "is not lowered
/// yet", so a POST could not carry its own status and headers and POST/redirect/GET was out of
/// reach. That stopped being true several phases before anyone noticed — `route_request_response_lock`
/// and `route_handler_single_call_lock` both exercise exactly that form. Nothing was wrong with
/// the framework; the README was simply the last place the old truth still lived, and it is the
/// one page an evaluator reads to decide whether LOGOS can do forms.
///
/// An example that UNDERSTATES the framework is as costly as one that overstates it, and it is
/// far harder to notice: everything still builds, every test still passes, and the only symptom
/// is a reader who concludes they cannot do something and leaves.
///
/// ★ THE CITATION IS THE TEETH. Requiring a `*_lock` name means the person who closes a gap has
/// to delete or rename the refusal that proved it — and this test goes red, pointing at the
/// README that still describes the old world. `every_example_explains_itself` could never catch
/// this: it asks whether a README exists and is longer than 400 bytes, which a wrong one is.
///
/// Same shape as `generate_docs_paths_lock::every_lock_the_documentation_cites_exists` and
/// `web_stdlib_docs_ratchet` — a document checked against the tree it describes.
#[test]
fn every_example_states_its_limits_honestly() {
    let locks = lock_names_in_tree();
    let mut offences = Vec::new();
    for (name, path) in examples() {
        let text = std::fs::read_to_string(path.join("README.md")).unwrap();
        let mut in_limits = false;
        for (i, line) in text.lines().enumerate() {
            if line.starts_with('#') {
                in_limits = is_limits_heading(line);
                continue;
            }
            if !in_limits || line.trim().is_empty() {
                continue;
            }
            let lower = line.to_ascii_lowercase();
            if !CAPABILITY_NEGATIONS.iter().any(|n| lower.contains(n)) {
                continue;
            }
            // The claim may cite its proof on this line or anywhere in the paragraph it opens,
            // so the whole section is searched — a bullet often spans several lines.
            let section: String = text
                .lines()
                .skip(i)
                .take_while(|l| !l.starts_with('#'))
                .collect::<Vec<_>>()
                .join(" ");
            let cited: Vec<&str> = section
                .split('`')
                .filter(|t| t.contains("_lock"))
                .map(|t| t.split("::").next().unwrap_or(t).trim())
                .collect();
            if cited.is_empty() {
                offences.push(format!(
                    "examples/{name}/README.md:{} claims a framework limitation with no test \
                     naming it:\n    {}",
                    i + 1,
                    line.trim()
                ));
            }
            for c in cited {
                if !locks.contains(c) {
                    offences.push(format!(
                        "examples/{name}/README.md:{} cites `{c}`, which is not a test in this \
                         tree — the gap it proved was probably closed and the claim outlived it",
                        i + 1
                    ));
                }
            }
        }
    }
    // ★ EVERY offence, not the first: a lock that stops at one under-reports in a way that looks
    // like completeness, and each round trip here is a full build.
    assert!(offences.is_empty(), "{}", offences.join("\n"));
}

/// Every `*_lock` test file name in the workspace, without extension.
fn lock_names_in_tree() -> std::collections::BTreeSet<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root");
    let mut found = std::collections::BTreeSet::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if name != "target" && !name.starts_with('.') {
                    stack.push(p);
                }
            } else if let Some(stem) = name.strip_suffix(".rs") {
                if stem.contains("_lock") {
                    found.insert(stem.to_string());
                }
            }
        }
    }
    found
}

#[test]
fn every_example_explains_itself() {
    for (name, path) in examples() {
        let readme = path.join("README.md");
        assert!(readme.is_file(), "examples/{name} has no README.md");
        let text = std::fs::read_to_string(&readme).unwrap();
        assert!(
            text.len() > 400,
            "examples/{name}'s README is too short to explain anything ({} bytes)",
            text.len()
        );
    }
}

/// Every environment variable a README documents is read by the example.
///
/// ★ THE OTHER DIRECTION FROM `every_example_states_its_limits_honestly`. That test catches a
/// README claiming the framework CANNOT do something it can — a limitation with no lock naming it.
/// This catches the opposite and equally costly error: a README documenting a capability the
/// example does not have.
///
/// `examples/fullstack_todo/README.md` had a whole section titled "State: PostgreSQL (real) or a
/// file (fallback)", explained that `TODO_DB_URL` selects between them, gave a `TODO_DB_URL=…
/// largo serve` command to run it, and documented `TODO_STORE` with a default path. `server.lg`
/// contains **none** of those three strings, has never spoken to a database, and stores its todos
/// in one file whose name is hard-coded. Every test passed the whole time — the code was correct,
/// the README described a different program.
///
/// An env var is the sharpest checkable signal of this class, because it is CONFIGURATION: a
/// reader who is told to set one will set it, and nothing will happen. Prose may legitimately
/// mention machinery an example does not use — this very README now names `postgres.lg` while
/// saying plainly that it does not use it — so the rule is deliberately narrow: a SHOUTING_TOKEN
/// in the README must appear somewhere in the example's own sources.
fn is_env_var_token(t: &str) -> bool {
    t.len() >= 4
        && t.len() <= 40
        && t.contains('_')
        && t.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && t.bytes().next().is_some_and(|b| b.is_ascii_uppercase())
}

/// Every `.lg` file under `root`, joined — the example's own source text.
fn example_sources(root: &Path) -> String {
    fn walk(d: &Path, out: &mut String) {
        let Ok(entries) = std::fs::read_dir(d) else {
            return;
        };
        for e in entries.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                walk(&p, out);
            } else if p.extension().and_then(|x| x.to_str()) == Some("lg") {
                if let Ok(t) = std::fs::read_to_string(&p) {
                    out.push_str(&t);
                    out.push('\n');
                }
            }
        }
    }
    let mut out = String::new();
    walk(root, &mut out);
    out
}

#[test]
fn every_environment_variable_a_readme_documents_is_read_by_the_example() {
    let mut offences = Vec::new();
    for (name, path) in examples() {
        let text = std::fs::read_to_string(path.join("README.md")).unwrap();
        let sources = example_sources(&path);
        let mut seen = std::collections::BTreeSet::new();
        for (i, line) in text.lines().enumerate() {
            for token in line.split('`').skip(1).step_by(2) {
                let token = token.trim();
                let bare = token.split('=').next().unwrap_or(token).trim();
                if !is_env_var_token(bare) || !seen.insert(bare.to_string()) {
                    continue;
                }
                if !sources.contains(bare) {
                    offences.push(format!(
                        "examples/{name}/README.md:{} documents `{bare}`, which appears in NO \
                         `.lg` file in the example — a reader told to set it will set it and \
                         nothing will happen:\n    {}",
                        i + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(
        offences.is_empty(),
        "README(s) documenting configuration the example does not read:\n\n{}",
        offences.join("\n")
    );
}

#[test]
fn the_environment_variable_check_can_fail() {
    // ★ ANTI-VACUITY. The token test is the whole mechanism; if it stopped recognising an env
    // var, the assertion above would pass over any README at all.
    assert!(is_env_var_token("TODO_DB_URL"));
    assert!(is_env_var_token("LOGOS_SESSION_SECRET"));
    assert!(
        !is_env_var_token("Serve"),
        "a capitalised word is not an env var"
    );
    assert!(
        !is_env_var_token("id|text"),
        "punctuation is not an env var"
    );
    assert!(
        !is_env_var_token("PORT"),
        "no underscore: too loose to be worth flagging"
    );
    // And the corpus really is being read — a scan that found no example would assert nothing.
    assert!(
        !examples().is_empty(),
        "no examples discovered; the check above is vacuous"
    );
}

// ── every example serves, and every example page is exercised ──────────────────────────────────

/// The URL a page file declares (`## Page /path`), read from the parsed page — `None` for a file
/// that declares no page at all (a component, a store, a directive).
fn declared_page_path(page: &Path) -> Option<String> {
    use logicaffeine_language::ast::component::ComponentKind;
    use logicaffeine_language::ast::stmt::Stmt;
    let src = logicaffeine_compile::analysis::assemble_project(page)
        .ok()?
        .source;
    logicaffeine_compile::ui_bridge::with_parsed_program(&src, |parsed, interner| {
        let (stmts, _, _) = parsed.ok()?;
        stmts.iter().find_map(|s| match s {
            Stmt::ComponentDef(def) => match &def.kind {
                ComponentKind::Page { path, .. } => Some(interner.resolve(*path).to_string()),
                _ => None,
            },
            _ => None,
        })
    })
}

/// The page files of an example, sorted.
fn page_files(root: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(root.join("pages")) else {
        return Vec::new();
    };
    let mut pages: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("lg"))
        .collect();
    pages.sort();
    pages
}

/// The page file that owns `/`: the one declaring `## Page /`, else an undeclared `pages/app.lg` —
/// read from the parsed page, the rule the router applies.
fn root_page(root: &Path) -> Option<PathBuf> {
    let pages = page_files(root);
    pages
        .iter()
        .find(|p| declared_page_path(p).as_deref() == Some("/"))
        .or_else(|| {
            pages.iter().find(|p| {
                p.file_stem().is_some_and(|s| s == "app") && declared_page_path(p).is_none()
            })
        })
        .cloned()
}

/// The site's front page: the page that owns `/`, else `pages/app.lg` wherever it declares itself
/// (`examples/signup_form` is `## Page /signup` and has no `/` at all).
fn front_page(root: &Path) -> Option<PathBuf> {
    root_page(root).or_else(|| {
        page_files(root)
            .into_iter()
            .find(|p| p.file_stem().is_some_and(|s| s == "app"))
    })
}

/// The exported `dist/`-relative path `front_page`'s own declared URL lands at — `/` exports as
/// `index.html` at the root, and any other static path exports one directory per segment with its
/// own `index.html` (`/signup` → `signup/index.html`), the same layout `largo build --static`
/// gives every page a clean URL under.
fn front_page_export_path(root: &Path, page: &Path) -> PathBuf {
    match declared_page_path(page).as_deref() {
        None | Some("/") => root.join("dist/index.html"),
        Some(url) => root
            .join("dist")
            .join(url.trim_matches('/'))
            .join("index.html"),
    }
}

/// The markup a page renders, on the reference tier — independent of anything `largo serve` does.
fn rendered_markup(page: &Path) -> Result<String, String> {
    let src = logicaffeine_compile::analysis::assemble_project(page)?.source;
    logicaffeine_compile::semantics::dom::reset();
    let out = logicaffeine_compile::compile::tw_outcome(&src);
    if let Some(e) = out.error {
        return Err(e);
    }
    let doc = logicaffeine_compile::semantics::dom::render();
    Ok(doc
        .trim_start_matches("<body>")
        .trim_end_matches("</body>")
        .to_string())
}

/// What a backend's own route walk answers for `GET /`, on the reference tier.
fn routed_markup(entry: &Path) -> Result<String, String> {
    let src = logicaffeine_compile::analysis::assemble_project(entry)?.source;
    let without_main = match src.find("\n## Main") {
        Some(at) => &src[..at + 1],
        None => &src[..],
    };
    let probe = format!("{without_main}\n## Main\n\nShow route(\"GET\", \"/\").\n");
    let out = logicaffeine_compile::compile::tw_outcome(&probe);
    match out.error {
        Some(e) => Err(e),
        None => Ok(out.output.trim_end().to_string()),
    }
}

/// A per-visitor CSRF token differs on every render; everything else must not.
///
/// `value="…"` is not necessarily the attribute right after `name="_csrf"` — attributes render in
/// alphabetical order, so a hidden field's own `type="hidden"` sits between them
/// (`name="_csrf" type="hidden" value="…"`) — this finds `value="` anywhere within the SAME tag as
/// `name="_csrf"` (bounded by the next `>`), not just immediately adjacent to it.
fn without_csrf_tokens(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(at) = rest.find("name=\"_csrf\"") {
        let (head, tail) = rest.split_at(at + "name=\"_csrf\"".len());
        let tag_end = tail.find('>').unwrap_or(tail.len());
        let Some(value_at) = tail[..tag_end].find("value=\"") else {
            out.push_str(head);
            rest = tail;
            continue;
        };
        let (before_value, after_open_quote) = tail.split_at(value_at + "value=\"".len());
        out.push_str(head);
        out.push_str(before_value);
        let end = after_open_quote.find('"').unwrap_or(after_open_quote.len());
        out.push_str("TOKEN");
        rest = &after_open_quote[end..];
    }
    out.push_str(rest);
    out
}

/// `without_csrf_tokens` used to require `value="` immediately after `name="_csrf" ` — real
/// markup renders `name="_csrf" type="hidden" value="…"` (alphabetical attribute order), so the
/// pattern never matched at all and `examples/guestbook`'s own differing per-visitor tokens leaked
/// straight into `every_example_serves`'s comparison as a false mismatch.
#[test]
fn without_csrf_tokens_normalizes_regardless_of_attribute_order() {
    let real = "<input name=\"_csrf\" type=\"hidden\" value=\"9cbe9fa6\">";
    assert_eq!(
        without_csrf_tokens(real),
        "<input name=\"_csrf\" type=\"hidden\" value=\"TOKEN\">"
    );
    // Also still works in the (hypothetical) adjacent ordering.
    let adjacent = "<input name=\"_csrf\" value=\"abc123\">";
    assert_eq!(
        without_csrf_tokens(adjacent),
        "<input name=\"_csrf\" value=\"TOKEN\">"
    );
    // Two different tokens in one document both normalize, so two renders compare equal.
    let a = format!("{real}{real}");
    let b = "<input name=\"_csrf\" type=\"hidden\" value=\"35a2cb6e\"><input name=\"_csrf\" type=\"hidden\" value=\"aa11bb22\">";
    assert_eq!(without_csrf_tokens(&a), without_csrf_tokens(b));
    // A tag with no `_csrf` name at all is untouched.
    assert_eq!(without_csrf_tokens("<p>hello</p>"), "<p>hello</p>");
}

/// ★ `largo serve` ON EVERY EXAMPLE ANSWERS THAT EXAMPLE'S OWN FRONT PAGE, AT THE FRONT PAGE'S URL. ★
///
/// It is the first command three of these READMEs give, and on a pure-frontend project it used to
/// serve the PLACEHOLDER entry (`Show "marketing site"`), never reading `pages/`. The expected
/// markup is computed independently of the server: the front page rendered on the reference tier —
/// asked for at the URL it declares, `/` unless it says otherwise (`examples/signup_form` is
/// `## Page /signup`) — or, for a backend with no pages, its own route walk for `GET /`.
#[test]
fn every_example_serves() {
    let mut served = 0usize;
    for (name, path) in examples() {
        let dir = tempfile::tempdir().unwrap();
        let root = scratch(&path, dir.path());
        let front = front_page(&root);
        let url = front
            .as_deref()
            .and_then(declared_page_path)
            .unwrap_or_else(|| "/".to_string());
        let expected = match front {
            Some(page) => rendered_markup(&page)
                .unwrap_or_else(|e| panic!("examples/{name}: its front page does not render: {e}")),
            None => {
                let manifest = std::fs::read_to_string(root.join("Largo.toml")).unwrap();
                let entry = manifest
                    .lines()
                    .find_map(|l| l.trim().strip_prefix("entry = "))
                    .map(|e| e.trim_matches('"').to_string())
                    .unwrap_or_else(|| "src/main.lg".to_string());
                routed_markup(&root.join(entry))
                    .unwrap_or_else(|e| panic!("examples/{name}: its `GET /` does not render: {e}"))
            }
        };
        assert!(
            !expected.trim().is_empty(),
            "examples/{name}: an empty front page proves nothing"
        );

        let mut server = Server::start(&root);
        let port = server.port();
        let front = server.get(&url);
        let got = body(&front);
        assert_eq!(
            status(&front),
            200,
            "examples/{name}: `{url}` answers:\n{got}\n{}",
            server.stderr()
        );
        assert!(
            without_csrf_tokens(&got).contains(&without_csrf_tokens(&expected)),
            "examples/{name}: `largo serve` answered `{url}` with something other than the example's \
             own front page.\n--- expected markup ---\n{expected}\n--- served ---\n{got}\n--- \
             server stderr ---\n{}",
            server.stderr()
        );
        served += 1;
    }
    assert!(
        served >= 4,
        "the sweep must reach the examples; it served {served}"
    );
}

/// The markup an `## Error page` renders, through a route walk that matches nothing — the path a
/// deployed server answers an unknown URL by.
fn error_page_markup(page: &Path) -> Result<String, String> {
    let src = logicaffeine_compile::analysis::assemble_project(page)?.source;
    let probe = format!(
        "{src}\n## Route GET /__examples_lock_probe -> Text:\n    Return \"x\".\n\n\
         ## Main\n\nShow route(\"GET\", \"/__examples_lock_no_such_page\").\n"
    );
    let out = logicaffeine_compile::compile::tw_outcome(&probe);
    match out.error {
        Some(e) => Err(e),
        None => Ok(out.output.trim_end().to_string()),
    }
}

/// The `## Error page 404` file of an example's `pages/`, if it has one — read from the parse.
fn not_found_page(root: &Path) -> Option<PathBuf> {
    use logicaffeine_language::ast::component::ComponentKind;
    use logicaffeine_language::ast::stmt::Stmt;
    let entries = std::fs::read_dir(root.join("pages")).ok()?;
    entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "lg")).find(|page| {
        let Ok(assembled) = logicaffeine_compile::analysis::assemble_project(page) else { return false };
        logicaffeine_compile::ui_bridge::with_parsed_program(&assembled.source, |parsed, _| {
            parsed.map_or(false, |(stmts, _, _)| {
                stmts.iter().any(|s| {
                    matches!(s, Stmt::ComponentDef(d) if matches!(d.kind, ComponentKind::ErrorPage { status: 404 }))
                })
            })
        })
    })
}

/// ★ A SITE'S UNKNOWN URL IS ITS OWN 404 PAGE, in development as on a static host. An example that
/// declares `## Error page 404` and has no backend answers an address it does not have with that
/// page and a 404 status — the exported `404.html`, served the way a host serves it.
#[test]
fn every_site_example_answers_an_unknown_url_with_its_own_404_page() {
    let mut checked = 0usize;
    for (name, path) in examples() {
        let Some(page) = not_found_page(&path) else {
            continue;
        };
        let project =
            logicaffeine_cli::project::shape::Project::load(&path).expect("the example loads");
        let plan = project.shape().expect("its shape is decided").serve_plan();
        if plan != (logicaffeine_cli::project::shape::ServePlan::Pages { proxy: false }) {
            continue;
        }
        checked += 1;
        let expected = error_page_markup(&page)
            .unwrap_or_else(|e| panic!("examples/{name}: its 404 page: {e}"));
        let dir = tempfile::tempdir().unwrap();
        let root = scratch(&path, dir.path());
        let mut server = Server::start(&root);
        let port = server.port();
        let missing = server.get("/no/such/page/anywhere");
        assert_eq!(
            status(&missing),
            404,
            "examples/{name}: {}",
            server.stderr()
        );
        assert!(
            body(&missing).contains(&expected),
            "examples/{name}: an unknown URL must show the example's own 404 page:\n--- expected ---\n\
             {expected}\n--- served ---\n{}",
            body(&missing)
        );
    }
    assert!(
        checked >= 1,
        "an example site declares a 404 page; the sweep must reach it"
    );
}

/// ★ EVERY EXAMPLE PAGE IS EXERCISED — computed from the parsed page. ★
///
/// An example's `## Test` is the part a reader trusts most: the author saying "this is what it
/// does" in a form the machine checks. `task_board` shipped a bug (it struck two rows and said
/// "2 left") that no test could see, because no example page had to carry one. So every
/// `## Page` carries a `## Test`, and every action, `on-*` handler, trigger and `bind` it declares
/// is set in motion by some step of one — see `page_coverage` for the model and
/// `page_coverage_lock` for its rules, each pinned in both directions.
#[test]
fn every_example_page_is_exercised() {
    let mut offences = Vec::new();
    let mut pages = 0usize;
    for (name, path) in examples() {
        let Ok(entries) = std::fs::read_dir(path.join("pages")) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("lg"))
            .collect();
        files.sort();
        for file in files {
            pages += 1;
            let rel = format!(
                "examples/{name}/pages/{}",
                file.file_name().unwrap().to_string_lossy()
            );
            let coverage = match page_coverage::page_coverage(&file) {
                Ok(c) => c,
                Err(e) => {
                    offences.push(format!("{rel}: {e}"));
                    continue;
                }
            };
            // An `## Error page` has no URL and no test runner mounts it; with nothing on it to drive,
            // what it shows is checked where it is served (`every_example_serves`).
            let exempt = coverage.error_page_only && coverage.items.is_empty();
            if coverage.tests == 0 && !exempt {
                offences.push(format!("{rel} carries no `## Test`"));
            }
            for item in coverage.undriven() {
                offences.push(format!("{rel}: no test step drives the {}", item.label));
            }
        }
    }
    assert!(
        pages >= 6,
        "the sweep must reach the example pages; it read {pages}"
    );
    assert!(offences.is_empty(), "{}", offences.join("\n"));
}

// ── what an example's README and stylesheets claim ─────────────────────────────────────────────

/// The fenced blocks of a README, as `(info string, lines)`.
fn fences(text: &str) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some(info) = line.trim_start().strip_prefix("```") else {
            continue;
        };
        let body: Vec<String> = lines
            .by_ref()
            .take_while(|l| !l.trim_start().starts_with("```"))
            .map(str::to_string)
            .collect();
        out.push((info.trim().to_string(), body));
    }
    out
}

/// ★ EVERY FILE A README'S LAYOUT LISTS EXISTS. `fullstack_todo`'s tree listed a
/// `public/index.html` that nothing served, kept "from an earlier shape of this example"; a layout
/// is a claim about the directory beside it, so it is checked against that directory.
#[test]
fn every_path_a_readme_layout_lists_exists() {
    let mut offences = Vec::new();
    let mut listed = 0usize;
    for (name, path) in examples() {
        let text = std::fs::read_to_string(path.join("README.md")).unwrap();
        for (_, body) in fences(&text) {
            for line in &body {
                let Some(at) = line.find("── ") else {
                    continue;
                };
                // The path, up to the comment that explains it; a directory's children are listed
                // beneath it with their own prefix, so the leaf name is relative to the root.
                let entry = line[at + "── ".len()..].split('#').next().unwrap().trim();
                if entry.is_empty() {
                    continue;
                }
                listed += 1;
                if !path.join(entry.trim_end_matches('/')).exists() {
                    offences.push(format!(
                        "examples/{name}/README.md lists `{entry}`, which is not there"
                    ));
                }
            }
        }
    }
    assert!(
        listed >= 5,
        "the READMEs' layouts must be read: {listed} entries"
    );
    assert!(offences.is_empty(), "{}", offences.join("\n"));
}

/// ★ EVERY `largo` COMMAND A README GIVES IS ONE THE CLI ACCEPTS — including the
/// `cargo run -p logicaffeine-cli -- …` spelling, which is `largo …` from a checkout.
#[test]
fn every_largo_command_a_readme_gives_is_real() {
    use clap::CommandFactory;
    let mut offences = Vec::new();
    let mut checked = 0usize;
    for (name, path) in examples() {
        let text = std::fs::read_to_string(path.join("README.md")).unwrap();
        for (info, body) in fences(&text) {
            if !matches!(info.as_str(), "bash" | "sh" | "shell" | "console" | "") {
                continue;
            }
            for line in &body {
                let command = line.split('#').next().unwrap().trim();
                let args: Vec<&str> =
                    if let Some(rest) = command.strip_prefix("cargo run -p logicaffeine-cli --") {
                        std::iter::once("largo")
                            .chain(rest.split_whitespace())
                            .collect()
                    } else if command.starts_with("largo ") || command == "largo" {
                        command.split_whitespace().collect()
                    } else {
                        continue;
                    };
                checked += 1;
                if let Err(e) = logicaffeine_cli::cli::Cli::command().try_get_matches_from(&args) {
                    offences.push(format!("examples/{name}/README.md: `{command}` is not a command largo accepts:\n{e}"));
                }
                // `largo serve`/`run`/`build`/`test`/`check` read the project in the directory they
                // run in; a path after them is handed to the PROGRAM, not used to find a project.
                if matches!(
                    args.get(1),
                    Some(&("serve" | "run" | "build" | "test" | "check"))
                ) && args
                    .iter()
                    .skip(2)
                    .any(|a| !a.starts_with('-') && a.contains('/'))
                {
                    offences.push(format!(
                        "examples/{name}/README.md: `{command}` passes a path; largo finds the project \
                         from the directory it runs in — `cd` into the example instead"
                    ));
                }
            }
        }
    }
    assert!(
        checked >= 4,
        "the READMEs' commands must be read: {checked}"
    );
    assert!(offences.is_empty(), "{}", offences.join("\n"));
}

/// Every class a page can render — its literal `class` tokens and every `class:name` — across
/// every component and page an example declares, read from the parsed pages.
fn rendered_classes(root: &Path) -> std::collections::BTreeSet<String> {
    use logicaffeine_language::ast::stmt::{Expr, Literal, Stmt};
    use logicaffeine_language::ast::view::{Attr, ViewNode};
    fn walk(
        nodes: &[ViewNode<'_>],
        interner: &logicaffeine_language::Interner,
        out: &mut std::collections::BTreeSet<String>,
    ) {
        for node in nodes {
            match node {
                ViewNode::Element {
                    attrs, children, ..
                } => {
                    for attr in attrs {
                        match attr {
                            Attr::Plain {
                                name,
                                value: Expr::Literal(Literal::Text(t)),
                            } if interner.resolve(*name) == "class" => {
                                out.extend(
                                    interner.resolve(*t).split_whitespace().map(str::to_string),
                                );
                            }
                            Attr::Class { name, .. } => {
                                out.insert(interner.resolve(*name).to_string());
                            }
                            _ => {}
                        }
                    }
                    walk(children, interner, out);
                }
                ViewNode::Component { children, .. } => walk(children, interner, out),
                ViewNode::Each { body, empty, .. } => {
                    walk(body, interner, out);
                    walk(empty, interner, out);
                }
                ViewNode::If {
                    arms, otherwise, ..
                } => {
                    for (_, b) in arms {
                        walk(b, interner, out);
                    }
                    if let Some(o) = otherwise {
                        walk(o, interner, out);
                    }
                }
                ViewNode::Inspect { arms, .. } => {
                    for a in arms {
                        walk(&a.body, interner, out);
                    }
                }
                ViewNode::Block {
                    default: Some(d), ..
                } => walk(d, interner, out),
                _ => {}
            }
        }
    }
    let mut out = std::collections::BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(root.join("pages")) else {
        return out;
    };
    for page in entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "lg"))
    {
        let src = logicaffeine_compile::analysis::assemble_project(&page)
            .expect("the page assembles")
            .source;
        logicaffeine_compile::ui_bridge::with_parsed_program(&src, |parsed, interner| {
            let (stmts, _, _) = parsed.expect("the page parses");
            for stmt in stmts {
                if let Stmt::ComponentDef(def) = stmt {
                    if let Some(view) = &def.view {
                        walk(&view.nodes, interner, &mut out);
                    }
                }
            }
        });
    }
    out
}

/// A stylesheet's selector text: every rule's prelude, comments removed, declarations skipped.
fn selector_text(css: &str) -> String {
    let mut out = String::new();
    let mut rest = css;
    while let Some(at) = rest.find("/*") {
        out.push_str(&rest[..at]);
        rest = rest[at..]
            .find("*/")
            .map(|e| &rest[at + e + 2..])
            .unwrap_or("");
    }
    out.push_str(rest);
    let mut selectors = String::new();
    let mut depth = 0usize;
    let mut prelude = String::new();
    for c in out.chars() {
        match c {
            '{' => {
                // An at-rule's own prelude (`@media …`) names no class; a rule's does.
                if !prelude.trim_start().starts_with('@') {
                    selectors.push_str(&prelude);
                    selectors.push('\n');
                }
                prelude.clear();
                depth += 1;
            }
            '}' => {
                depth = depth.saturating_sub(1);
                prelude.clear();
            }
            ';' if depth > 0 => prelude.clear(),
            _ => prelude.push(c),
        }
    }
    selectors
}

/// The classes a stylesheet's selectors name.
fn styled_classes(css: &str) -> Vec<String> {
    let selectors = selector_text(css);
    let bytes = selectors.as_bytes();
    let mut out = Vec::new();
    for (i, c) in selectors.char_indices() {
        if c != '.'
            || !bytes
                .get(i + 1)
                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_' || *b == b'-')
        {
            continue;
        }
        let name: String = selectors[i + 1..]
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
            .collect();
        out.push(name);
    }
    out
}

/// ★ EVERY CLASS AN EXAMPLE'S STYLESHEET STYLES IS ONE ITS PAGES RENDER. A rule for a class
/// nothing renders is dead weight every visitor downloads, and it misleads whoever edits the page
/// next — `fullstack_todo` shipped `.card` and `.add-row` rules for markup it no longer had.
#[test]
fn every_class_an_example_stylesheet_styles_is_rendered() {
    let mut offences = Vec::new();
    let mut read = 0usize;
    for (name, path) in examples() {
        let Ok(entries) = std::fs::read_dir(path.join("static")) else {
            continue;
        };
        let rendered = rendered_classes(&path);
        for sheet in entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "css"))
        {
            read += 1;
            let css = std::fs::read_to_string(&sheet).unwrap();
            for class in styled_classes(&css) {
                if !rendered.contains(&class) {
                    offences.push(format!(
                        "examples/{name}/static/{}: `.{class}` styles a class no page renders",
                        sheet.file_name().unwrap().to_string_lossy()
                    ));
                }
            }
        }
    }
    assert!(read >= 3, "the examples' stylesheets must be read: {read}");
    assert!(offences.is_empty(), "{}", offences.join("\n"));
}

/// Every `var(--token)` a stylesheet or a page's `Style:` uses, and every `--token:` defined.
fn tokens(
    css: &str,
) -> (
    std::collections::BTreeSet<String>,
    std::collections::BTreeSet<String>,
) {
    let name = |s: &str| {
        s.chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect::<String>()
    };
    let used = css
        .match_indices("var(--")
        .map(|(i, _)| name(&css[i + "var(--".len()..]))
        .collect();
    let defined = css
        .match_indices("--")
        .filter(|(i, _)| !css[..*i].ends_with("var("))
        .map(|(i, _)| (i, name(&css[i + 2..])))
        .filter(|(i, n)| css[i + 2 + n.len()..].trim_start().starts_with(':'))
        .map(|(_, n)| n)
        .collect();
    (used, defined)
}

/// ★ EVERY DESIGN TOKEN AN EXAMPLE USES IS DEFINED — by the example, or by the framework's own
/// theme every page's document carries (`web_host::DEFAULT_THEME_CSS`). CSS drops a declaration
/// whose `var(--x)` names nothing, silently — no error, no warning — so an undefined token is a
/// rule that looks written and does nothing. The same rule `web_theme_lock` holds the scaffold to,
/// over every example.
#[test]
fn every_design_token_an_example_uses_is_defined() {
    let mut offences = Vec::new();
    for (name, path) in examples() {
        let mut all = String::new();
        for dir in ["static", "pages"] {
            let Ok(entries) = std::fs::read_dir(path.join(dir)) else {
                continue;
            };
            for f in entries.flatten().map(|e| e.path()) {
                if f.extension().is_some_and(|x| x == "css" || x == "lg") {
                    all.push_str(&std::fs::read_to_string(&f).unwrap());
                    all.push('\n');
                }
            }
        }
        let (used, mut defined) = tokens(&all);
        defined.extend(tokens(logicaffeine_compile::web_host::DEFAULT_THEME_CSS).1);
        for t in used.difference(&defined) {
            offences.push(format!(
                "examples/{name} uses `var(--{t})`, and nothing defines `--{t}`"
            ));
        }
    }
    assert!(offences.is_empty(), "{}", offences.join("\n"));
}

#[test]
fn the_stylesheet_and_token_readers_can_fail() {
    let css = "/* a .comment */ .card h1, .row.done > span { color: var(--accent); background: url(x.png); }\n\
               @media (max-width: 40rem) { .narrow { --gap: 1px; } }";
    assert_eq!(styled_classes(css), vec!["card", "row", "done", "narrow"]);
    let (used, defined) = tokens(css);
    assert!(used.contains("accent") && defined.contains("gap") && !defined.contains("accent"));
}
