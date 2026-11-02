//! ★ LOCK — a record declared in its own file decodes a typed fetch in a frontend-only project,
//! through every `largo` surface, and an `Import` that cannot import is refused where it stands. ★
//!
//! A frontend-only project (`Largo.toml`, `pages/`, `src/shared.lg`, no backend) names its record
//! with `Import Shared.`, and `Get … as Seq of Todo` reads the answer through `fromJsonSeq_Todo`,
//! the codec derived from `## A Todo has:`. The loader reaches `src/shared.lg` from `pages/app.lg`
//! through the project root, and the assembled closure every `largo` command compiles carries the
//! declaration, so the derive sees it.
//!
//! ★ WHERE THE IMPORT STANDS DECIDES WHETHER IT IMPORTS. `scan_dependencies` reads `Import`
//! sentences from the prose above the first `##` header and nowhere else (`import_scan_lock` pins
//! both halves). An `Import Shared.` written below a `## Note` heading therefore resolves nothing:
//! the module is never spliced, `Todo` never reaches the type registry, and the project checks
//! clean — the first sign is `Unknown function: fromJsonSeq_Todo` when a test runs, far from the
//! line that caused it. The import sentence is refused at its own line instead.
//!
//! ★ A TYPE NOTHING DECLARES IS NAMED AS SUCH. The reason the misplaced import was silent is that
//! `a new Todo` with no `Todo` anywhere in the program checks clean and runs, showing `Todo`. A
//! type name that no declaration, import or prelude supplies is refused where it is written.

#![cfg(not(target_arch = "wasm32"))]

#[path = "cli_common/mod.rs"]
mod common;
use common::{largo_in, stderr, stdout};

use std::path::Path;

const MANIFEST: &str =
    "[package]\nname = \"feed\"\nversion = \"0.1.0\"\nauthors = []\n\n[dependencies]\n";

const SHARED: &str = "\
# Shared

The record the page and the feed agree on.

## A Todo has:
    an id, which is Int.
    a text, which is Text.
";

/// The page, with its `Import Shared.` in the prose above the first `##` header.
const PAGE: &str = r##"# Feed

The page asks for the feed when it loads and lists what arrived.

Import Shared.

## Page /

State:
    Let todos be a new Seq of Todo.

Actions:
    When the page loads:
        Get "/api/todos" as Seq of Todo, then loaded.

    To loaded (answer: Result of Seq of Todo and HttpError):
        Inspect answer:
            When Ok (list):
                Set todos to list.
            When Err (e):
                Set todos to a new Seq of Todo.

View:
    <p id="count">{length of todos}</p>
    <ul>{for each t in todos:}<li>{t's text}</li>{end}</ul>

## Test: the imported record decodes the feed

Given that GET "/api/todos" answers 200 with "[{{\"id\": 1, \"text\": \"milk\"}}]".
Expect that the text of "#count" is equal to "1".
Expect that the text of "li" is equal to "milk".
"##;

/// The same page with the one change that silences it: the `Import Shared.` sits below a
/// `## Note` heading, past the first `##` header.
fn page_with_the_import_under_a_note() -> String {
    let moved = PAGE.replacen("Import Shared.\n\n", "", 1);
    moved.replacen(
        "## Page /",
        "## Note\n\nThe record comes from its own file.\n\nImport Shared.\n\n## Page /",
        1,
    )
}

fn write_project(root: &Path, page: &str) {
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("Largo.toml"), MANIFEST).unwrap();
    std::fs::write(root.join("src/shared.lg"), SHARED).unwrap();
    std::fs::write(root.join("pages/app.lg"), page).unwrap();
}

/// The 1-based line of the first line of `text` equal to `line`.
fn line_of(text: &str, line: &str) -> usize {
    text.lines()
        .position(|l| l == line)
        .expect("the fixture carries the line")
        + 1
}

/// Whether `out` places something at `file:line` — `line` followed by a column, or by the end of
/// the location, never a longer line number that merely starts with the same digits.
fn names_location(out: &str, file: &str, line: usize) -> bool {
    let at = format!("{file}:{line}");
    out.match_indices(&at).any(|(i, _)| {
        !out[i + at.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
    })
}

fn combined(o: &std::process::Output) -> String {
    format!("{}\n{}", stdout(o), stderr(o))
}

#[test]
fn a_frontend_only_project_decodes_an_imported_record_on_every_largo_surface() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("feed");
    write_project(&root, PAGE);

    let test = largo_in(&root, &["test"]);
    let out = combined(&test);
    assert!(
        test.status.success(),
        "`largo test` must pass the page's own test:\n{out}"
    );
    assert!(
        out.contains("vm_agreed=1") && out.contains("vm_diverged=0"),
        "the page's test must run on the lowered VM too and agree with the tree-walker:\n{out}"
    );
    assert!(
        !out.contains("fromJsonSeq_Todo"),
        "the derived decoder must exist:\n{out}"
    );

    let check = largo_in(&root, &["check"]);
    assert!(
        check.status.success(),
        "`largo check` must pass:\n{}",
        combined(&check)
    );

    let build = largo_in(&root, &["build", "--static"]);
    assert!(
        build.status.success(),
        "`largo build --static` must export:\n{}",
        combined(&build)
    );
    let wasm: Vec<String> = std::fs::read_dir(root.join("dist"))
        .expect("dist/ exists after an export")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".wasm"))
        .collect();
    assert_eq!(
        wasm.len(),
        1,
        "the export carries the page's one compiled module: {wasm:?}"
    );
}

#[test]
fn an_import_below_a_section_header_is_refused_at_its_own_line() {
    let page = page_with_the_import_under_a_note();
    let line = line_of(&page, "Import Shared.");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("feed");
    write_project(&root, &page);

    for args in [&["check"][..], &["test"][..], &["build", "--static"][..]] {
        let o = largo_in(&root, args);
        let out = combined(&o);
        assert!(
            !o.status.success(),
            "`largo {}` must refuse an `Import` that imports nothing, not carry on without it:\n{out}",
            args.join(" ")
        );
        assert!(
            out.contains("Import Shared") && names_location(&out, "app.lg", line),
            "`largo {}` must name the import sentence at pages/app.lg:{line}, where the fix is:\n{out}",
            args.join(" ")
        );
        assert!(
            !out.contains("fromJsonSeq_Todo"),
            "`largo {}` must stop at the import, before a missing decoder is ever reached:\n{out}",
            args.join(" ")
        );
    }
}

#[test]
fn a_type_nothing_declares_is_refused_where_it_is_named() {
    let main = "# Main\n\n## Main\n\nLet t be a new Todo.\nShow t.\n";
    let line = line_of(main, "Let t be a new Todo.");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("undeclared");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Largo.toml"),
        "[package]\nname = \"undeclared\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    )
    .unwrap();
    std::fs::write(root.join("src/main.lg"), main).unwrap();

    let check = largo_in(&root, &["check"]);
    let out = combined(&check);
    assert!(
        !check.status.success(),
        "`a new Todo` with no `Todo` declared anywhere must not check:\n{out}"
    );
    assert!(
        out.contains("Todo") && names_location(&out, "main.lg", line),
        "the refusal must name `Todo` at src/main.lg:{line}:\n{out}"
    );
}
