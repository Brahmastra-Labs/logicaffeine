//! ★ LOCK — every place `largo` shows an error, it shows the author's `file:line:col`. ★
//!
//! A mistake is reported on many surfaces: `largo check` and its `--json` envelope, the dev server's
//! terminal and its browser overlay, `largo test`, `largo expand`, `largo emit`, `largo build`. They
//! used to disagree about where the mistake was. A page's error printed with no line at all, the
//! envelope said `"file": null` on line 1 under a `build-failure` code, the dev server told the
//! terminal only "see the browser overlay", `largo test` answered "no tests were discovered" about a
//! page whose tests could not compile, and three commands printed the compiler's error as a raw
//! `ParseError { kind: …, span: Span { … } }` struct.
//!
//! Every fixture puts its mistake on a known line, at or below line 3, of a known file — so a
//! surface that falls back to line 1, to the assembled closure, or to no location at all fails.

#[path = "cli_common/mod.rs"]
mod common;

use common::http::Server;
use common::{largo_in, stderr, stdout};

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A page whose `<p>` is never closed — an error the template parser names — on line 7. The view
/// ends right there (no `</main>`): the parser blames the element still open at end of input,
/// so the caret lands on the line that has the actual mistake, not on some other tag's mismatched
/// closer.
const BROKEN_PAGE: &str = "# App\n\n## Page /\n\nView:\n    <main>\n    <p>unclosed\n";
const BROKEN_LINE: usize = 7;

/// A site: `pages/app.lg` holding `page`, and nothing else.
fn site(dir: &Path, name: &str, page: &str) -> PathBuf {
    let root = dir.join(name);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("Largo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .unwrap();
    std::fs::write(root.join("pages/app.lg"), page).unwrap();
    root
}

/// The `file:line:` pointer a surface must print for `rel` at `line`.
fn pointer(rel: &str, line: usize) -> String {
    format!("{rel}:{line}:")
}

/// No surface prints the compiler's error as a Rust struct.
fn assert_no_debug_dump(surface: &str, text: &str) {
    assert!(
        !text.contains("ParseError {")
            && !text.contains("Span {")
            && !text.contains("kind: Custom("),
        "{surface} printed a Debug dump instead of a diagnostic:\n{text}"
    );
    for name in parse_error_kind_names() {
        let bare = text
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .any(|word| word == name);
        assert!(
            !bare,
            "{surface} printed the bare error kind `{name}` instead of its explanation:\n{text}"
        );
    }
}

/// Every `ParseErrorKind` variant whose name has two or more capitals (`ExpectedStatement`,
/// `UnexpectedToken`) — a name no explanation sentence contains, so one appearing in a surface's
/// output is a Debug-formatted kind. Read from the enum's source so a new variant is covered.
fn parse_error_kind_names() -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/logicaffeine_language/src/error.rs");
    let src = std::fs::read_to_string(&path).expect("error.rs is readable");
    let body = src
        .split("pub enum ParseErrorKind {")
        .nth(1)
        .expect("error.rs declares `pub enum ParseErrorKind`");
    let mut depth = 1usize;
    let mut names = Vec::new();
    for line in body.lines() {
        if depth == 1 {
            let trimmed = line.trim_start();
            let ident: String = trimmed
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            if ident.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && ident.chars().filter(|c| c.is_ascii_uppercase()).count() >= 2
            {
                names.push(ident);
            }
        }
        depth += line.matches('{').count();
        depth = depth.saturating_sub(line.matches('}').count());
        if depth == 0 {
            break;
        }
    }
    assert!(
        names.len() >= 20,
        "the ParseErrorKind census found only {names:?}"
    );
    names
}

fn json_of(out: &std::process::Output) -> Value {
    serde_json::from_str(stdout(out).trim())
        .unwrap_or_else(|e| panic!("one JSON document ({e}):\n{}", stdout(out)))
}

// ── largo check ─────────────────────────────────────────────────────────────────────────────────

#[test]
fn check_places_a_pages_error_at_its_file_line_col() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "chk", BROKEN_PAGE);
    let out = largo_in(&root, &["check"]);
    let err = stderr(&out);
    assert!(!out.status.success(), "{err}");
    assert!(
        err.contains(&pointer("pages/app.lg", BROKEN_LINE)),
        "the page's own line:\n{err}"
    );
    assert!(
        err.contains("<p>unclosed"),
        "and its own text under the caret:\n{err}"
    );
    assert_no_debug_dump("largo check", &err);
}

#[test]
fn check_json_names_the_page_and_its_line() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "chkj", BROKEN_PAGE);
    let out = largo_in(&root, &["check", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let v = json_of(&out);
    assert_eq!(v["ok"], false, "{v}");
    let d = &v["diagnostics"][0];
    assert!(
        d["file"]
            .as_str()
            .is_some_and(|f| f.ends_with("pages/app.lg")),
        "the diagnostic names the page: {v}"
    );
    assert_eq!(d["span"]["start_line"], BROKEN_LINE, "at its own line: {v}");
    assert_ne!(
        d["code"], "build-failure",
        "a compile error keeps its own code: {v}"
    );
}

/// ★ THROUGH AN IMPORT. The page is compiled as a closure — every module it imports spliced in
/// ahead of it — and the line must still be the page's own.
#[test]
fn check_places_a_pages_error_at_its_own_line_through_an_import() {
    let dir = tempfile::tempdir().unwrap();
    let page = format!(
        "# App\n\nImport Shared.\n{}",
        &BROKEN_PAGE["# App\n".len()..]
    );
    let root = site(dir.path(), "chki", &page);
    std::fs::write(
        root.join("pages/shared.lg"),
        "# Shared\n\n## To one () -> Int:\n    Return 1.\n\n## To two () -> Int:\n    Return 2.\n\n\
         ## To three () -> Int:\n    Return 3.\n",
    )
    .unwrap();
    let out = largo_in(&root, &["check", "--json"]);
    let v = json_of(&out);
    let d = v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| {
            d["file"]
                .as_str()
                .is_some_and(|f| f.ends_with("pages/app.lg"))
        })
        .unwrap_or_else(|| panic!("a diagnostic about pages/app.lg: {v}"));
    assert_eq!(
        d["span"]["start_line"],
        BROKEN_LINE + 2,
        "the page's line, not the closure's: {v}"
    );
}

#[test]
fn lgx_warnings_print_file_line_col_and_reach_the_json() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(
        dir.path(),
        "lint",
        "# App\n\n## Page /\n\nView:\n    <main>\n    <img src=\"cat.png\">\n    </main>\n",
    );
    let out = largo_in(&root, &["check"]);
    let err = stderr(&out);
    assert!(
        out.status.success(),
        "a warning does not fail the check:\n{err}"
    );
    assert!(err.contains("alt"), "{err}");
    assert!(
        err.contains(&pointer("pages/app.lg", 7)),
        "the warning is placed:\n{err}"
    );

    let out = largo_in(&root, &["check", "--json"]);
    let v = json_of(&out);
    assert_eq!(v["ok"], true, "{v}");
    let w = v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["severity"] == "warning")
        .unwrap_or_else(|| panic!("the warning is in the envelope: {v}"));
    assert_eq!(w["code"], "image-without-alt", "{v}");
    assert!(
        w["file"]
            .as_str()
            .is_some_and(|f| f.ends_with("pages/app.lg")),
        "{v}"
    );
    assert_eq!(w["span"]["start_line"], 7, "{v}");
}

/// A value-semantics copy warning (`crate::lint`) held to the same bar as an LGX one: printed at
/// its own `file:line:col` and present in the `--json` envelope, never failing the check.
///
/// `opens` is bound from `b's opens`, mutated by `Pop`, and never read or written back — the
/// lint's own canonical example, given a REAL declared field (`Box`) so `largo check` also
/// compiles the page to the browser tier cleanly (the warning must survive that, not only a
/// syntax-only parse).
const COPY_LINT_PAGE: &str = "# App\n\n\
     ## A Box has:\n    an opens, which is Seq of Int.\n\n\
     ## To useOpens (b: Box) -> Int:\n\
     \x20   Let opens be b's opens.\n\
     \x20   Pop from opens into at.\n\
     \x20   Return at.\n\n\
     ## Page /\n\nView:\n    <p>hi</p>\n";
const COPY_LINT_LINE: usize = 8;

#[test]
fn a_value_copy_warning_prints_file_line_col_and_reaches_the_json() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "copylint", COPY_LINT_PAGE);
    let out = largo_in(&root, &["check"]);
    let err = stderr(&out);
    assert!(
        out.status.success(),
        "a warning does not fail the check:\n{err}"
    );
    assert!(err.contains("opens"), "{err}");
    assert!(
        err.contains(&pointer("pages/app.lg", COPY_LINT_LINE)),
        "the warning is placed:\n{err}"
    );
    assert_no_debug_dump("largo check", &err);

    let out = largo_in(&root, &["check", "--json"]);
    let v = json_of(&out);
    assert_eq!(v["ok"], true, "{v}");
    let w = v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "orphaned-copy")
        .unwrap_or_else(|| panic!("the copy-lint warning is in the envelope: {v}"));
    assert_eq!(w["severity"], "warning", "{v}");
    assert!(
        w["file"]
            .as_str()
            .is_some_and(|f| f.ends_with("pages/app.lg")),
        "{v}"
    );
    assert_eq!(w["span"]["start_line"], COPY_LINT_LINE, "{v}");
}

// ── largo check, across an import (a "Program" shape: an entry + a module it imports) ──────────

/// A `Program`'s own mistake, with a module imported ahead of it, must still be blamed on the
/// program's own line — not wherever that offset lands when the combined source (the import
/// spliced in first) is misread as the entry's own, unspliced text.
#[test]
fn check_places_the_entrys_own_mistake_correctly_when_it_imports_a_module() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("prog");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Largo.toml"),
        "[package]\nname = \"prog\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/shared.lg"),
        "# Shared\n\n## To greeting () -> Text:\n    Return \"hi\".\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/main.lg"),
        "# Main\n\nImport Shared.\n\n## Main\nShow greeting().\nRetrun 5.\n",
    )
    .unwrap();
    let out = largo_in(&root, &["check"]);
    let err = stderr(&out);
    assert!(!out.status.success(), "{err}");
    assert!(
        err.contains(&pointer("src/main.lg", 7)),
        "the entry's own line, not the spliced offset:\n{err}"
    );
    assert!(
        err.contains("Retrun 5."),
        "and its own text under the caret:\n{err}"
    );
    assert_no_debug_dump("largo check", &err);
}

/// An imported module's own mistake is blamed on THAT module — its own file, at its own line —
/// never on the entry that imported it.
#[test]
fn check_places_an_imported_modules_own_mistake_in_that_module() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("prog");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Largo.toml"),
        "[package]\nname = \"prog\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/shared.lg"),
        "# Shared\n\n## To greeting () -> Text:\n    Retrun \"hi\".\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/main.lg"),
        "# Main\n\nImport Shared.\n\n## Main\nShow greeting().\n",
    )
    .unwrap();
    let out = largo_in(&root, &["check"]);
    let err = stderr(&out);
    assert!(!out.status.success(), "{err}");
    assert!(
        err.contains(&pointer("src/shared.lg", 4)),
        "the module's OWN file and line:\n{err}"
    );
    assert!(
        !err.contains("src/main.lg"),
        "not blamed on the entry that imported it:\n{err}"
    );
    assert!(
        err.contains("Retrun"),
        "and its own text under the caret:\n{err}"
    );
    assert_no_debug_dump("largo check", &err);
}

// ── largo serve ─────────────────────────────────────────────────────────────────────────────────

/// The terminal is where a developer is looking when they save — so the page's error is printed
/// there, placed, and not only in a browser tab they may not have open.
#[test]
fn serve_prints_a_broken_pages_error_in_the_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "srv", BROKEN_PAGE);
    let mut server = Server::start(&root);
    let port = server.port();
    let overlay = common::http::body(&server.get("/"));
    assert!(
        overlay.contains("app.lg"),
        "the overlay names the page:\n{overlay}"
    );
    assert!(
        overlay.contains(&format!("line {BROKEN_LINE}")),
        "and its line:\n{overlay}"
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut said = server.stderr();
    while !said.contains(&pointer("pages/app.lg", BROKEN_LINE)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        said = server.stderr();
    }
    assert!(
        said.contains(&pointer("pages/app.lg", BROKEN_LINE)),
        "the terminal names the line:\n{said}"
    );
    assert!(said.contains("<p>unclosed"), "and shows it:\n{said}");
    assert_no_debug_dump("largo serve", &said);
}

/// A program's bundle that does not build refuses to serve — at the author's line. WITHOUT
/// `--watch` there is no watcher that could ever fix a broken build, so `cmd_serve_dom` refuses
/// loudly up front rather than binding a port that could only ever answer the same overlay
/// (`serve_dom.rs::cmd_serve_dom`'s doc comment). `--watch` deliberately does the opposite —
/// even a first build that fails still binds the port and serves the located overlay, so a
/// fixing save can recover it with no restart — and that half of the property is proven by
/// `serve_watch.rs::serve_watch_starts_on_a_broken_page_and_recovers_on_the_fixing_save` (a
/// spawn-and-poll test, since a `--watch` server never exits for `.output()` to observe).
#[test]
fn serve_refuses_a_bundle_that_does_not_build_at_its_line() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("bundle");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Largo.toml"),
        "[package]\nname = \"bundle\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/main.lg"),
        "# Main\n\n## Component App\n\nView:\n    <main>\n    <p>unclosed\n\n## Main\n\nMount App.\n",
    )
    .unwrap();
    let out = largo_in(&root, &["serve", "--port", "0"]);
    let err = stderr(&out);
    assert!(!out.status.success(), "{err}");
    assert!(err.contains(&pointer("src/main.lg", 7)), "{err}");
    assert_no_debug_dump("largo serve", &err);
}

// ── largo test ──────────────────────────────────────────────────────────────────────────────────

const TESTED_PAGE: &str = "# Counter\n\n## Page /\n\nState:\n    Let n be 0.\n\nActions:\n    To bump:\n        \
                           Set n to n + 1.\n\nView:\n    <button id=\"b\" on-click={bump}>+</button>\n    \
                           <p id=\"n\">{n}</p>\n\n## Test: the button counts\n\nClick \"#b\".\n\
                           Expect that the text of \"#n\" is equal to \"1\".\n";

#[test]
fn test_names_the_page_file_its_results_came_from() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "tst", TESTED_PAGE);
    let out = largo_in(&root, &["test"]);
    let all = format!("{}{}", stdout(&out), stderr(&out));
    assert!(out.status.success(), "{all}");
    assert!(
        all.contains("pages/app.lg › the button counts"),
        "the result names its file:\n{all}"
    );
}

/// ★ A PAGE WITH TESTS THAT DOES NOT COMPILE IS TESTS THAT DID NOT RUN, and the run says so — the
/// file, how many tests, and why, at the line — instead of "no tests were discovered".
#[test]
fn test_says_a_page_with_tests_did_not_compile() {
    let dir = tempfile::tempdir().unwrap();
    let broken = TESTED_PAGE.replace("<p id=\"n\">{n}</p>", "<p id=\"n\">{n}");
    assert_ne!(broken, TESTED_PAGE);
    let root = site(dir.path(), "tstb", &broken);
    let out = largo_in(&root, &["test"]);
    let err = stderr(&out);
    assert!(!out.status.success(), "{err}");
    assert!(
        !err.contains("no tests were discovered"),
        "the tests were discovered:\n{err}"
    );
    assert!(err.contains("pages/app.lg did not compile"), "{err}");
    assert!(err.contains("1 test(s) did not run"), "{err}");
    assert!(
        err.contains(&pointer("pages/app.lg", 14)),
        "at the line:\n{err}"
    );
}

// ── run --interpret ─────────────────────────────────────────────────────────────────────────────

/// A program whose fourth line does not parse.
const BROKEN_PROGRAM: &str = "# App\n\n## Main\nShow 1 +.\n";

/// A project whose entry, `src/main.lg`, holds `main`.
fn program(dir: &Path, name: &str, main: &str) -> PathBuf {
    let root = dir.join(name);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Largo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n"),
    )
    .unwrap();
    std::fs::write(root.join("src/main.lg"), main).unwrap();
    root
}

#[test]
fn run_interpret_places_a_parse_error_at_its_file_line_col() {
    let dir = tempfile::tempdir().unwrap();
    let root = program(dir.path(), "runi", BROKEN_PROGRAM);
    let out = largo_in(&root, &["run", "--interpret"]);
    let err = stderr(&out);
    assert!(!out.status.success(), "{err}");
    assert!(
        err.contains(&pointer("src/main.lg", 4)),
        "the program's own line:\n{err}"
    );
    assert!(
        err.contains("Show 1 +."),
        "and its own text under the caret:\n{err}"
    );
    assert_no_debug_dump("largo run --interpret", &err);
}

/// A program that reaches the standard library parses with the library spliced in ahead of its own
/// code, so a mistake below the splice sits past thousands of bytes the author never wrote — and is
/// still placed at the author's own line.
#[test]
fn run_interpret_places_a_parse_error_below_the_stdlib_splice_at_its_file_line_col() {
    let dir = tempfile::tempdir().unwrap();
    let root = program(
        dir.path(),
        "runs",
        "# App\n\n## Main\nShow htmlEscape(\"<b>\").\nShow 1 +.\n",
    );
    let out = largo_in(&root, &["run", "--interpret"]);
    let err = stderr(&out);
    assert!(!out.status.success(), "{err}");
    assert!(
        err.contains(&pointer("src/main.lg", 5)),
        "the program's own line:\n{err}"
    );
    assert!(
        err.contains("Show 1 +."),
        "and its own text under the caret:\n{err}"
    );
    assert_no_debug_dump("largo run --interpret", &err);
}

#[test]
fn run_interpret_places_a_pages_retired_view_line_at_its_file_line_col() {
    let dir = tempfile::tempdir().unwrap();
    let root = program(
        dir.path(),
        "runv",
        "# App\n\n## Page /\n\nView:\n    <main>\n    div: card(\"Hello\")\n    </main>\n",
    );
    let out = largo_in(&root, &["run", "--interpret"]);
    let err = stderr(&out);
    assert!(!out.status.success(), "{err}");
    assert!(
        err.contains(&pointer("src/main.lg", 7)),
        "the retired line's own line:\n{err}"
    );
    assert!(
        err.contains("retired `Render:`"),
        "named for what it is:\n{err}"
    );
    assert_no_debug_dump("largo run --interpret", &err);
}

// ── expand, emit, build ─────────────────────────────────────────────────────────────────────────

#[test]
fn expand_places_a_parse_error_at_its_line() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("page.lg");
    std::fs::write(
        &file,
        "# Page\n\n## Component App\n\nView:\n    <main>\n    <p>unclosed\n",
    )
    .unwrap();
    let out = largo_in(dir.path(), &["expand", file.to_str().unwrap()]);
    let err = stderr(&out);
    assert!(!out.status.success(), "{err}");
    assert!(err.contains(&pointer("page.lg", 7)), "{err}");
    assert_no_debug_dump("largo expand", &err);
}

#[test]
fn emit_places_a_refusal_at_its_line() {
    let dir = tempfile::tempdir().unwrap();
    common::scaffold(dir.path(), "emitted");
    std::fs::write(
        dir.path().join("src/main.lg"),
        "# Main\n\n## Main\n\nLet xs be [1].\nShow item 0 of xs.\n",
    )
    .unwrap();
    let out = largo_in(dir.path(), &["emit", "wasm"]);
    let err = stderr(&out);
    assert!(!out.status.success(), "{err}");
    assert!(err.contains(&pointer("src/main.lg", 6)), "{err}");
    assert_no_debug_dump("largo emit", &err);
}

#[test]
fn build_static_places_a_broken_pages_error_at_its_line() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "stat", BROKEN_PAGE);
    let out = largo_in(&root, &["build", "--static"]);
    let err = stderr(&out);
    assert!(!out.status.success(), "{err}");
    assert!(err.contains(&pointer("pages/app.lg", BROKEN_LINE)), "{err}");
    assert_no_debug_dump("largo build --static", &err);
}

#[test]
fn build_json_on_a_site_names_the_page_and_its_line() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "statj", BROKEN_PAGE);
    let out = largo_in(&root, &["build", "--json"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let v = json_of(&out);
    let d = &v["diagnostics"][0];
    assert!(
        d["file"]
            .as_str()
            .is_some_and(|f| f.ends_with("pages/app.lg")),
        "{v}"
    );
    assert_eq!(d["span"]["start_line"], BROKEN_LINE, "{v}");
}

// ── the census ──────────────────────────────────────────────────────────────────────────────────

/// Debug-formatting sites in the CLI's own source that stay, each with why it never reaches an
/// author as a location-less compiler struct.
const DEBUG_FORMATTING_ALLOWED: &[(&str, &str)] = &[(
    "src/commands/serve_web.rs",
    "the HTTP wire parser's error about a BACKEND's response bytes — a transport fault, not a \
         mistake in any source file",
)];

/// ★ THE CENSUS. Every `{e:?}`-shaped format in the CLI's non-test source is either gone or listed
/// above with its reason, so a new Debug dump of a compiler error cannot land unnoticed.
#[test]
fn no_debug_dump_of_an_error_reaches_the_author() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/logicaffeine_cli/src");
    let mut found = Vec::new();
    let mut stack = vec![src.clone()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().and_then(|x| x.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            // Only the code before the file's unit tests: an assertion message may print anything.
            let code = text.split("#[cfg(test)]").next().unwrap_or(&text);
            let rel = format!("src/{}", p.strip_prefix(&src).unwrap().to_string_lossy());
            for (n, line) in code.lines().enumerate() {
                let dump = ["{e:?}", "{err:?}", "{pe:?}", "{error:?}"]
                    .iter()
                    .any(|d| line.contains(d));
                if dump && !DEBUG_FORMATTING_ALLOWED.iter().any(|(f, _)| *f == rel) {
                    found.push(format!("{rel}:{}: {}", n + 1, line.trim()));
                }
            }
        }
    }
    // The `lean` command family reports its own structured NDJSON diagnostics.
    found.retain(|l| !l.starts_with("src/commands/lean"));
    assert!(
        found.is_empty(),
        "Debug dumps that can reach an author:\n{}",
        found.join("\n")
    );
}

/// The crates whose strings reach an author: the compiler, the language, the CLI, the language
/// server and the web app.
const AUTHOR_FACING_SOURCES: &[&str] = &[
    "crates/logicaffeine_compile/src",
    "crates/logicaffeine_language/src",
    "apps/logicaffeine_cli/src",
    "crates/logicaffeine_lsp/src",
    "apps/logicaffeine_web/src",
];

/// `ParseError` Debug formats that never reach an author, each with the reason.
const PARSE_ERROR_DEBUG_ALLOWED: &[(&str, &str)] = &[(
    "crates/logicaffeine_language/src/parser/lgx_lower/shared.rs",
    "a panic on the compiler's own shipped LOGOS (the shared-store frame algebra), which parses by \
     construction — an invariant of the compiler, not a mistake in any source an author wrote",
)];

/// Every author-facing source file, as `(path from the repo root, its non-test code)`.
fn author_facing_code() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = Vec::new();
    for base in AUTHOR_FACING_SOURCES {
        let mut stack = vec![root.join(base)];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                    let rel = p
                        .strip_prefix(&root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    let text = std::fs::read_to_string(&p).unwrap();
                    out.push((
                        rel,
                        text.split("#[cfg(test)]")
                            .next()
                            .unwrap_or(&text)
                            .to_string(),
                    ));
                }
            }
        }
    }
    out
}

/// The doors that hand back the parser's own error: `parse_program`, and every function in `code`
/// whose name has an underscore and whose return type names `ParseError`.
fn parse_error_doors(code: &[(String, String)]) -> Vec<String> {
    let mut doors = vec!["parse_program".to_string()];
    for (_, text) in code {
        for (at, _) in text.match_indices("fn ") {
            if at > 0 && text.as_bytes()[at - 1].is_ascii_alphanumeric() {
                continue;
            }
            let rest = &text[at + 3..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            let Some(sig_end) = rest.find(|c| c == '{' || c == ';') else {
                continue;
            };
            let returns = rest[..sig_end].split_once("->").map_or("", |(_, r)| r);
            if name.contains('_')
                && name.starts_with(|c: char| c.is_ascii_lowercase())
                && returns.contains("ParseError")
                && !doors.contains(&name)
            {
                doors.push(name);
            }
        }
    }
    doors
}

/// Whether `text` calls one of `doors`: the name, not the tail of a longer word, then `(`.
fn calls_a_door(text: &str, doors: &[String]) -> bool {
    doors.iter().any(|d| {
        text.match_indices(d.as_str()).any(|(at, _)| {
            let before = text[..at].chars().next_back();
            let after = text[at + d.len()..].trim_start();
            !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') && after.starts_with('(')
        })
    })
}

/// Whether `line` Debug-formats `name`: `{name:?}`, or a `{:?}` with `name` among the arguments.
fn debug_formats(line: &str, name: &str) -> bool {
    if line.contains(&format!("{{{name}:?}}")) || line.contains(&format!("{{{name}:#?}}")) {
        return true;
    }
    (line.contains("{:?}") || line.contains("{:#?}"))
        && line.match_indices(name).any(|(at, _)| {
            let before = line[..at].trim_end();
            let after = line[at + name.len()..].trim_start();
            before.ends_with(',') && (after.starts_with(',') || after.starts_with(')'))
        })
}

/// The error variants that hold the parser's own error.
const PARSE_ERROR_VARIANTS: &[&str] = &["CompileError::Parse(", "ProjectScopeError::Parse("];

/// The binding an error arm or closure gives the error on `line`: `map_err(|e|`, `if let Err(e) =`,
/// `Err(e) =>`, or an arm of a variant holding the parser's own error (`CompileError::Parse(e) =>`),
/// with which of the four it is.
fn error_binding(line: &str) -> Option<(&str, &'static str)> {
    let ident = |s: &str| -> usize {
        s.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(s.len())
    };
    for variant in PARSE_ERROR_VARIANTS {
        if let Some(at) = line.find(variant) {
            let rest = &line[at + variant.len()..];
            let n = ident(rest);
            if n > 0 && rest[n..].starts_with(')') {
                return Some((&rest[..n], "variant"));
            }
        }
    }
    if let Some(at) = line.find("map_err(|") {
        let rest = &line[at + "map_err(|".len()..];
        let n = ident(rest);
        if n > 0 && rest[n..].starts_with('|') {
            return Some((&rest[..n], "map_err"));
        }
    }
    let at = line.find("Err(")?;
    let rest = &line[at + "Err(".len()..];
    let n = ident(rest);
    let tail = rest[n..].strip_prefix(')')?.trim_start();
    if n == 0 {
        return None;
    }
    if line[..at].trim_end().ends_with("if let") && tail.starts_with('=') && !tail.starts_with("=>")
    {
        return Some((&rest[..n], "if let"));
    }
    tail.starts_with("=>").then_some((&rest[..n], "arm"))
}

/// The expression `text` ends with: walked back over balanced groups to the statement boundary — a
/// `;`, an unmatched opening bracket, or a binding `=` — at depth zero.
fn receiver(text: &str) -> &str {
    let b = text.as_bytes();
    let (mut depth, mut i) = (0usize, b.len());
    while i > 0 {
        let c = b[i - 1];
        match c {
            b')' | b']' | b'}' => depth += 1,
            b'(' | b'[' | b'{' => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            b';' if depth == 0 => break,
            b'=' if depth == 0
                && !(i >= 2 && matches!(b[i - 2], b'=' | b'!' | b'<' | b'>'))
                && !(i < b.len() && matches!(b[i], b'=' | b'>')) =>
            {
                break
            }
            _ => {}
        }
        i -= 1;
    }
    &text[i..]
}

/// The Debug formats of a `ParseError` in `code` (a source file's non-test text at `rel`): every
/// Debug format of the error an arm or closure binds from a call of one of `doors`, and every
/// Debug format of the kind inside the error type itself.
fn parse_error_debug_formats_in(rel: &str, code: &str, doors: &[String]) -> Vec<String> {
    let lines: Vec<&str> = code.lines().collect();
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !line.contains(":?}") && !line.contains(":#?}") {
            continue;
        }
        if rel.ends_with("logicaffeine_language/src/error.rs") && line.contains("self.kind") {
            found.push(format!("{rel}:{}: {}", i + 1, line.trim()));
            continue;
        }
        let Some((j, name, kind)) = (i.saturating_sub(8)..=i)
            .rev()
            .find_map(|j| error_binding(lines[j]).map(|(n, k)| (j, n, k)))
        else {
            continue;
        };
        if !debug_formats(line, name) {
            continue;
        }
        if kind == "variant" {
            found.push(format!("{rel}:{}: {}", i + 1, line.trim()));
            continue;
        }
        let produced = match kind {
            "map_err" => {
                let text = lines[j.saturating_sub(20)..=j].join("\n");
                receiver(&text[..text.rfind(".map_err").unwrap_or(text.len())]).to_string()
            }
            "if let" => lines[j]
                .split_once('=')
                .map_or(String::new(), |(_, e)| e.to_string()),
            _ => (j.saturating_sub(40)..j)
                .rev()
                .find_map(|k| lines[k].split_once("match ").map(|(_, e)| e.to_string()))
                .unwrap_or_default(),
        };
        if calls_a_door(&produced, doors) {
            found.push(format!("{rel}:{}: {}", i + 1, line.trim()));
        }
    }
    found
}

/// ★ NO SURFACE SHOWS A PARSE ERROR AS A RUST STRUCT. A `ParseError` reaches the author through
/// its explanation — `socratic_explanation`, the caret block, the `logos-diagnostic/1` object — and
/// never as `ParseError { kind: …, span: Span { … } }`. This reads the non-test SOURCE of every
/// crate whose strings reach an author, finds every door that hands back the parser's own error,
/// and fails on any `{:?}` of the error such a call binds — so a new one cannot land unnoticed.
#[test]
fn no_debug_format_of_a_parse_error_reaches_any_surface() {
    let code = author_facing_code();
    let doors = parse_error_doors(&code);
    assert!(
        [
            "parse_program",
            "compile_to_wasm",
            "encode_program_source",
            "compile_kripke_with"
        ]
        .iter()
        .all(|d| doors.iter().any(|x| x == d)),
        "the door census stopped finding the doors it is built on: {doors:?}"
    );
    let found: Vec<String> = code
        .iter()
        .filter(|(rel, _)| !PARSE_ERROR_DEBUG_ALLOWED.iter().any(|(f, _)| f == rel))
        .flat_map(|(rel, text)| parse_error_debug_formats_in(rel, text, &doors))
        .collect();
    assert!(
        found.is_empty(),
        "Debug formats of a parse error that can reach an author:\n{}",
        found.join("\n")
    );
}

/// The census above sees the shapes it exists for, and not their honest neighbours.
#[test]
fn the_parse_error_census_sees_a_debug_format_of_a_parse_error() {
    let doors = vec![
        "parse_program".to_string(),
        "compile_kripke_with".to_string(),
        "encode_program_source".to_string(),
    ];
    let seen = |code: &str| parse_error_debug_formats_in("crates/x/src/lib.rs", code, &doors).len();
    assert_eq!(
        seen("let stmts = parser.parse_program().map_err(|e| format!(\"{e:?}\"))?;"),
        1
    );
    assert_eq!(seen("let stmts = parser\n    .parse_program()\n    .map_err(|e| format!(\"Parse error: {:?}\", e))?;"), 1);
    assert_eq!(
        seen("compile_kripke_with(spec, |a, i| {\n    let t = f(a);\n    t.go(i)\n})\n.map_err(|e| Hw(format!(\"{:#?}\", e)))"),
        1,
        "the door's call is the receiver, however many lines its closure takes"
    );
    assert_eq!(seen("match encode_program_source(src) {\n    Ok(s) => s,\n    Err(e) => return format!(\"{e:?}\"),\n}"), 1);
    assert_eq!(
        seen("if let Err(e) = encode_program_source(src) {\n    eprintln!(\"{:?}\", e);\n}"),
        1
    );
    assert_eq!(
        seen("CompileError::Parse(e) => write!(f, \"Parse error: {:?}\", e),"),
        1,
        "the variant holds one"
    );
    assert_eq!(
        seen("let stmts = parser.parse_program().map_err(|e| describe(&e))?;"),
        0,
        "an explanation is not a dump"
    );
    assert_eq!(
        seen("let r = Compiler::compile(s).map_err(|e| format!(\"{e:?}\"))?;"),
        0,
        "not a door"
    );
    assert_eq!(
        seen("let shape = format!(\"{shape:?}\");"),
        0,
        "a Debug format of no error at all"
    );
    assert_eq!(
        seen("let n: u16 = t.parse().map_err(|_| format!(\"not a byte: {:?}\", t))?;"),
        0,
        "a Debug format of the text, not of an error"
    );
    assert_eq!(
        parse_error_debug_formats_in(
            "crates/logicaffeine_language/src/error.rs",
            "let k = format!(\"{:?}\", self.kind);",
            &doors
        )
        .len(),
        1,
        "the error type formatting its own kind"
    );
}
