//! Every guide, project document and example is reachable from the indexes a reader starts at.
//!
//! `docs/README.md` links every `docs/*.md` guide and every `docs/project/*.md` document, and nothing that is
//! not there; the root README's Documentation section links every guide; `examples/README.md` links every
//! example directory, and nothing that is not there; and both the root README and the docs index link the
//! examples index. A new guide or example that no index names goes RED here instead of shipping unfindable.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test docs_index_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::path::PathBuf;

const DOCS_INDEX: &str = "docs/README.md";
const EXAMPLES_INDEX: &str = "examples/README.md";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

/// Every markdown link target in `text`, without its `#fragment`.
fn links(text: &str) -> BTreeSet<String> {
    text.split("](")
        .skip(1)
        .filter_map(|rest| rest.split(')').next())
        .map(|t| t.split('#').next().unwrap_or_default().trim().to_string())
        .filter(|t| !t.is_empty() && !t.contains("://"))
        .collect()
}

/// `docs/<name>.md` guides (not the index itself) and `docs/project/<name>.md` documents, relative to `docs/`.
fn documents() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (dir, prefix) in [("docs", ""), ("docs/project", "project/")] {
        for entry in std::fs::read_dir(repo_root().join(dir))
            .unwrap_or_else(|e| panic!("{dir}: {e}"))
            .flatten()
        {
            let name = entry.file_name().to_string_lossy().to_string();
            if entry.path().is_file()
                && name.ends_with(".md")
                && !(prefix.is_empty() && name == "README.md")
            {
                out.insert(format!("{prefix}{name}"));
            }
        }
    }
    out
}

fn guides() -> BTreeSet<String> {
    documents()
        .into_iter()
        .filter(|d| !d.contains('/'))
        .collect()
}

fn examples() -> BTreeSet<String> {
    std::fs::read_dir(repo_root().join("examples"))
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect()
}

/// The section of `text` under the `## <heading>` line, up to the next `## `.
fn section<'a>(text: &'a str, heading: &str) -> &'a str {
    let start = text
        .find(&format!("\n## {heading}\n"))
        .unwrap_or_else(|| panic!("no `## {heading}` section"));
    let body = &text[start + heading.len() + 5..];
    &body[..body.find("\n## ").unwrap_or(body.len())]
}

#[test]
fn the_link_reader_sees_markdown_links() {
    let got = links(
        "[a](imperative-mode.md) [b](project/FAQ.md#top) [c](https://x.y/z.md) ![i](img.svg)",
    );
    assert_eq!(
        got,
        BTreeSet::from([
            "imperative-mode.md".to_string(),
            "project/FAQ.md".to_string(),
            "img.svg".to_string()
        ])
    );
}

#[test]
fn the_docs_index_links_every_document_and_nothing_else() {
    let linked: BTreeSet<String> = links(&read(DOCS_INDEX))
        .into_iter()
        .filter(|l| l.ends_with(".md") && !l.starts_with("../") && !l.starts_with("./"))
        .collect();
    let docs = documents();
    let unlinked: Vec<&String> = docs.difference(&linked).collect();
    let dangling: Vec<&String> = linked.difference(&docs).collect();
    assert!(
        unlinked.is_empty(),
        "`{DOCS_INDEX}` does not link {unlinked:?}"
    );
    assert!(
        dangling.is_empty(),
        "`{DOCS_INDEX}` links {dangling:?}, which `docs/` does not hold"
    );
}

#[test]
fn the_root_readme_documentation_section_links_every_guide() {
    let readme = read("README.md");
    let linked: BTreeSet<String> = links(section(&readme, "Documentation"))
        .into_iter()
        .filter_map(|l| l.strip_prefix("docs/").map(str::to_string))
        .collect();
    let unlinked: Vec<String> = guides()
        .into_iter()
        .filter(|g| !linked.contains(g))
        .collect();
    assert!(
        unlinked.is_empty(),
        "the root README's Documentation section does not link docs/{unlinked:?}"
    );
}

#[test]
fn the_examples_index_links_every_example_and_nothing_else() {
    let linked: BTreeSet<String> = links(&read(EXAMPLES_INDEX))
        .into_iter()
        .filter_map(|l| {
            let l = l.trim_start_matches("./");
            (!l.starts_with("..")).then(|| l.split('/').next().unwrap_or_default().to_string())
        })
        .filter(|l| !l.is_empty() && !l.ends_with(".md"))
        .collect();
    let all = examples();
    let unlinked: Vec<&String> = all.difference(&linked).collect();
    let dangling: Vec<&String> = linked.difference(&all).collect();
    assert!(
        unlinked.is_empty(),
        "`{EXAMPLES_INDEX}` does not link examples {unlinked:?}"
    );
    assert!(
        dangling.is_empty(),
        "`{EXAMPLES_INDEX}` links {dangling:?}, which `examples/` does not hold"
    );
}

#[test]
fn every_example_directory_carries_a_readme() {
    let bare: Vec<String> = examples()
        .into_iter()
        .filter(|e| {
            !repo_root()
                .join("examples")
                .join(e)
                .join("README.md")
                .is_file()
        })
        .collect();
    assert!(bare.is_empty(), "examples without a README.md: {bare:?}");
}

#[test]
fn the_readme_and_the_docs_index_link_the_examples_index() {
    let readme = links(&read("README.md"));
    assert!(
        readme.contains("examples/README.md") || readme.contains("examples/"),
        "the root README does not link the examples index `{EXAMPLES_INDEX}`"
    );
    let docs = links(&read(DOCS_INDEX));
    assert!(
        docs.contains("../examples/README.md") || docs.contains("../examples/"),
        "`{DOCS_INDEX}` does not link the examples index `{EXAMPLES_INDEX}`"
    );
}
