//! Byte-lock for `public/llms-full.txt` — the one-file LOGOS curriculum.
//!
//! `llms-full.txt` is the llms.txt convention's full-content companion: the
//! quickguide, then every guide the docs index (`docs/README.md`) lists, in the
//! index's reading order, the error catalog among them, concatenated with source
//! separators. The parts are read from the index rather than listed here, so a
//! guide added to the docs is in the file the next time it is generated, and a
//! guide left out of the index fails `docs_index_lock` first. Agents (and AI
//! crawlers) fetch one URL and hold the whole language. `llms.txt`, the short
//! form, names the release it describes. Regenerate with
//! `./scripts/release/gen-agent-artifacts.sh`.

use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

/// The documents, in reading order: `(source-relative path, title)` — the quickguide, then each row of the
/// docs index's `## Guides` table.
fn parts() -> Vec<(String, String)> {
    let index = std::fs::read_to_string(workspace_root().join("docs/README.md"))
        .expect("docs/README.md exists");
    let table = &index[index
        .find("\n## Guides\n")
        .expect("the docs index has a `## Guides` section")..];
    let table = &table[..table[1..].find("\n## ").map_or(table.len(), |e| e + 1)];
    let mut out = vec![(
        "LOGOS_QUICKGUIDE.md".to_string(),
        "LOGOS Quickguide — the whole language on one page".to_string(),
    )];
    for row in table.lines().filter(|l| l.starts_with("| [")) {
        let title = &row[3..row.find("](").expect("a guide row links its guide")];
        let target = &row[row.find("](").unwrap() + 2..];
        let target = &target[..target.find(')').expect("a guide link closes")];
        out.push((format!("docs/{target}"), title.replace('`', "")));
    }
    out
}

fn render_llms_full() -> String {
    let root = workspace_root();
    let mut out = String::new();
    out.push_str(
        "# LOGOS — full documentation for language models\n\n\
         This is the llms-full.txt companion to /llms.txt: the complete LOGOS\n\
         curriculum in one file. LOGOS is the English programming language —\n\
         https://logicaffeine.com — and its toolchain is also an MCP server\n\
         (`claude mcp add logos -- largo mcp`).\n",
    );
    for (path, title) in parts() {
        let text = std::fs::read_to_string(root.join(&path))
            .unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
        out.push_str(&format!(
            "\n\n================================================================\n\
             SOURCE: {path} — {title}\n\
             ================================================================\n\n"
        ));
        out.push_str(&text);
    }
    out
}

#[test]
fn llms_full_matches_concatenation() {
    let want = render_llms_full();
    let path = workspace_root().join("apps/logicaffeine_web/public/llms-full.txt");
    let have = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "{} is missing — run ./scripts/release/gen-agent-artifacts.sh",
            path.display()
        )
    });
    assert_eq!(
        have, want,
        "llms-full.txt is stale — run ./scripts/release/gen-agent-artifacts.sh"
    );
}

#[test]
fn llms_txt_links_llms_full() {
    let path = workspace_root().join("apps/logicaffeine_web/public/llms.txt");
    let text = std::fs::read_to_string(path).expect("llms.txt exists");
    assert!(
        text.contains("/llms-full.txt"),
        "llms.txt must point crawlers at the full-content companion"
    );
}

#[test]
#[ignore = "regenerator — run via ./scripts/release/gen-agent-artifacts.sh (LOGOS_REGEN=1)"]
fn regenerate_llms_full() {
    if std::env::var_os("LOGOS_REGEN").is_none() {
        // The full suite runs #[ignore]d tests too; without the opt-in this
        // is a no-op (the byte-locks above are the actual gate).
        eprintln!("skipping regeneration: set LOGOS_REGEN=1 (use ./scripts/release/gen-agent-artifacts.sh)");
        return;
    }
    let path = workspace_root().join("apps/logicaffeine_web/public/llms-full.txt");
    std::fs::write(path, render_llms_full()).expect("write llms-full.txt");
}

#[test]
fn llms_full_holds_every_guide_in_docs() {
    let root = workspace_root();
    let listed: std::collections::BTreeSet<String> = parts().into_iter().map(|(p, _)| p).collect();
    let mut guides: Vec<String> = std::fs::read_dir(root.join("docs"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".md") && n != "README.md")
        .map(|n| format!("docs/{n}"))
        .collect();
    guides.sort();
    let missing: Vec<&String> = guides.iter().filter(|g| !listed.contains(*g)).collect();
    assert!(
        missing.is_empty(),
        "guides llms-full.txt would leave out (list them in docs/README.md): {missing:?}"
    );
    assert!(listed.contains("docs/lgx-reference.md") && listed.contains("docs/getting-started.md"));
}

#[test]
fn llms_txt_names_the_release_it_describes() {
    let root = workspace_root();
    let manifest: toml::Value = std::fs::read_to_string(root.join("Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let version = manifest["workspace"]["package"]["version"]
        .as_str()
        .unwrap();
    let text = std::fs::read_to_string(root.join("apps/logicaffeine_web/public/llms.txt"))
        .expect("llms.txt exists");
    let summary = text
        .lines()
        .find(|l| l.starts_with("> "))
        .expect("llms.txt opens with a `> ` summary");
    assert!(
        summary.contains(&format!("Current release: {version}.")),
        "llms.txt's summary names release {version}: {summary}"
    );
}
