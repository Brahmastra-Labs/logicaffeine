//! Byte-locks for the generated error catalog.
//!
//! `docs/ERROR_CATALOG.md` and `apps/logicaffeine_web/public/errors.json` are
//! generated from `logicaffeine_language::diagnostic::catalog()` by the library's
//! own renderers, `catalog_markdown()` and `catalog_json()` — the same functions
//! `largo`'s MCP server serves live. These tests assert the checked-in files match
//! them byte-for-byte, so the catalog can never drift from the enum. Regenerate with
//! `./scripts/release/gen-error-catalog.sh` (which runs the `#[ignore]`d
//! `regenerate_error_catalog` test under `LOGOS_REGEN=1`).

use std::path::PathBuf;

use logicaffeine_language::diagnostic::{catalog_json, catalog_markdown};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn catalog_md_path() -> PathBuf {
    workspace_root().join("docs/ERROR_CATALOG.md")
}

fn errors_json_path() -> PathBuf {
    workspace_root().join("apps/logicaffeine_web/public/errors.json")
}

#[test]
fn error_catalog_md_matches_generator() {
    let want = catalog_markdown();
    let path = catalog_md_path();
    let have = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "{} is missing — run ./scripts/release/gen-error-catalog.sh",
            path.display()
        )
    });
    assert_eq!(
        have, want,
        "docs/ERROR_CATALOG.md is stale — run ./scripts/release/gen-error-catalog.sh"
    );
}

#[test]
fn errors_json_matches_generator() {
    let want = catalog_json();
    let path = errors_json_path();
    let have = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "{} is missing — run ./scripts/release/gen-error-catalog.sh",
            path.display()
        )
    });
    assert_eq!(
        have, want,
        "errors.json is stale — run ./scripts/release/gen-error-catalog.sh"
    );
}

/// Markdown snippets in the catalog must never use `logos` fences — sample
/// error headlines are not compilable programs, and `doc_examples.rs`
/// compiles every `logos` fence under `docs/`.
#[test]
fn error_catalog_md_has_no_logos_fences() {
    let text = catalog_markdown();
    assert!(
        !text.contains("```logos"),
        "ERROR_CATALOG.md must fence samples as text, not logos"
    );
}

#[test]
#[ignore = "regenerator — run via ./scripts/release/gen-error-catalog.sh (LOGOS_REGEN=1)"]
fn regenerate_error_catalog() {
    if std::env::var_os("LOGOS_REGEN").is_none() {
        // The full suite runs #[ignore]d tests too; without the opt-in this
        // is a no-op (the byte-locks above are the actual gate).
        eprintln!(
            "skipping regeneration: set LOGOS_REGEN=1 (use ./scripts/release/gen-error-catalog.sh)"
        );
        return;
    }
    std::fs::write(catalog_md_path(), catalog_markdown()).expect("write ERROR_CATALOG.md");
    std::fs::write(errors_json_path(), catalog_json()).expect("write errors.json");
}
