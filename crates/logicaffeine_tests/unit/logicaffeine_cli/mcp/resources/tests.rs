use super::*;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn pack_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/agent")
}

fn all() -> impl Iterator<Item = &'static Resource> {
    DOCS.iter().chain(DATA.iter())
}

/// The files in `docs/` that are not guides to the language, each with why it is not served.
const NOT_GUIDES: &[(&str, &str)] = &[(
    "README.md",
    "the index of the guides, whose links point at files an agent cannot follow",
)];

/// ★ EVERY GUIDE IN `docs/` IS A RESOURCE. The MCP server is how an agent learns the language
/// offline; a guide written and never served teaches nobody who reaches LOGOS that way.
#[test]
fn every_guide_in_docs_is_served() {
    let docs = repo_root().join("docs");
    let mut missing = Vec::new();
    for entry in std::fs::read_dir(&docs)
        .expect("docs/ is readable")
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".md") else {
            continue;
        };
        if NOT_GUIDES.iter().any(|(n, _)| *n == name) {
            continue;
        }
        if !DOCS.iter().any(|r| r.source == format!("docs/{name}")) {
            missing.push(format!(
                "docs/{name} (as logos://docs/{})",
                stem.to_ascii_lowercase().replace('_', "-")
            ));
        }
    }
    assert!(
        missing.is_empty(),
        "guides no MCP resource serves: {missing:?}"
    );
}

/// Every resource reproduces its repository source byte-for-byte: the agent pack is a fresh
/// mirror, and the rendered resources equal the files generated from the same renderers.
#[test]
fn every_resource_is_byte_identical_to_its_source() {
    let root = repo_root();
    let mut stale = Vec::new();
    for r in all() {
        let source = std::fs::read_to_string(root.join(r.source)).unwrap_or_else(|e| {
            panic!(
                "{} names source {} which cannot be read: {e}",
                r.uri, r.source
            )
        });
        if r.text() != source {
            stale.push(format!("{} ≠ {}", r.uri, r.source));
        }
    }
    assert!(
        stale.is_empty(),
        "resources that no longer match their source — run ./scripts/release/gen-agent-artifacts.sh:\n  {}",
        stale.join("\n  ")
    );
}

/// A packed resource embeds exactly `assets/agent/<source>`, so the mirror layout is the table.
#[test]
fn every_pack_entry_embeds_the_mirror_of_its_source() {
    for r in all() {
        if let Body::Pack(text) = r.body {
            let packed = std::fs::read_to_string(pack_dir().join(r.source))
                .unwrap_or_else(|e| panic!("{}: assets/agent/{} is missing: {e}", r.uri, r.source));
            assert_eq!(
                text, packed,
                "{} embeds a file other than assets/agent/{}",
                r.uri, r.source
            );
        }
    }
}

#[test]
fn every_uri_is_served_once() {
    let mut uris: Vec<&str> = all().map(|r| r.uri).collect();
    let before = uris.len();
    uris.sort();
    uris.dedup();
    assert_eq!(uris.len(), before, "one URI, one resource");
}

/// Regenerate `assets/agent/` from the repository sources (`./scripts/release/gen-agent-artifacts.sh`).
#[test]
#[ignore = "regenerator — run via ./scripts/release/gen-agent-artifacts.sh (LOGOS_REGEN=1)"]
fn regenerate_agent_pack() {
    if std::env::var_os("LOGOS_REGEN").is_none() {
        eprintln!("skipping regeneration: set LOGOS_REGEN=1 (use ./scripts/release/gen-agent-artifacts.sh)");
        return;
    }
    let root = repo_root();
    for r in all() {
        if let Body::Pack(_) = r.body {
            let dest = pack_dir().join(r.source);
            std::fs::create_dir_all(dest.parent().unwrap()).expect("create pack dir");
            std::fs::copy(root.join(r.source), &dest)
                .unwrap_or_else(|e| panic!("copy {}: {e}", r.source));
        }
    }
}
