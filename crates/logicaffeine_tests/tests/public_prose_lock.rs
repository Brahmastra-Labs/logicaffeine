//! Public documentation cites only what a reader of the public tree can open, and says what it means.
//!
//! Every markdown document the checkout ships (READMEs, `docs/`, `.github/`, the examples, the root documents),
//! and every example project's LOGOS source (literate: its notes are prose), is scanned for citations: a backticked path or link into a private root entry (one git ignores that is not
//! a local build/tool location), a plain mention of such a directory, a repo path that does not exist, or a
//! capitalised document name no public directory holds. The CHANGELOGs have their own lock
//! (`changelog_citation_lock`); `CLAUDE.md` describes the private workspace to agents on purpose; generated
//! copies, stdlib sources and project templates are covered through their sources.
//!
//! The same documents, `llms.txt` and every member's Cargo `description` are held to the house voice: no
//! word that claims a quality instead of showing it (`powerful`, `seamless`, `robust`, …), no internal
//! codename a reader was never introduced to (`WS-G`, `Lever B`, a `rung`), and no "the single … truth"
//! slogan. Code, inline code and link targets are not prose and are not read.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test public_prose_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/public_manifest.rs"]
mod public_manifest;

use public_manifest::{checkout, repo_root, Classifier, LOCAL_LOCATIONS};
use std::collections::BTreeSet;

/// The withheld root directories, by name.
fn private_roots() -> BTreeSet<String> {
    Classifier::load()
        .withheld_dirs()
        .into_iter()
        .map(|d| d.trim_end_matches('/').to_string())
        .collect()
}

/// `a/./b/../c` → `a/c`; `None` when the path climbs out of the repo.
fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// `text` without its fenced code blocks: a fence holds commands and examples, not citations.
fn prose(text: &str) -> String {
    let mut out = String::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// (span, is a link target) for every backticked span and markdown link target in the prose of `text`.
fn spans(text: &str) -> Vec<(String, bool)> {
    let text = &prose(text);
    let mut out: Vec<(String, bool)> = text
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|s| !s.contains('\n'))
        .map(|s| (s.to_string(), false))
        .collect();
    let unticked: String = text.split('`').step_by(2).collect::<Vec<_>>().join(" ");
    out.extend(
        unticked
            .split("](")
            .skip(1)
            .filter_map(|rest| rest.split(')').next())
            .map(|s| (s.to_string(), true)),
    );
    out
}

fn looks_like_a_path(s: &str) -> bool {
    !s.is_empty()
        && !s.contains("://")
        && !s.starts_with("mailto:")
        && !s.chars().any(|c| " *<>{}…$|\\\"'=,;()[]".contains(c))
}

/// A bare document name in capitals (`FAQ.md`): a claim that such a document exists somewhere a reader looks.
fn is_bare_document(s: &str) -> bool {
    s.strip_suffix(".md").is_some_and(|stem| {
        !stem.is_empty()
            && stem
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    })
}

/// Every citation in `text` (the contents of the repo file `file`) the public tree cannot back, given the
/// withheld root directories.
fn violations(file: &str, text: &str, private: &BTreeSet<String>) -> Vec<String> {
    let root = repo_root();
    let dir = file.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let mut out = Vec::new();
    for (span, link) in spans(text) {
        let bare = span.split('#').next().unwrap_or_default().trim();
        let bare = bare
            .split_once(':')
            .filter(|(_, n)| n.chars().all(|c| c.is_ascii_digit() || c == '-'))
            .map_or(bare, |(p, _)| p);
        let bare = bare.trim_end_matches('/');
        if !looks_like_a_path(bare) {
            continue;
        }
        let Some(path) = normalize(&if link {
            format!("{dir}/{bare}")
        } else {
            bare.to_string()
        }) else {
            if link {
                out.push(format!("`{span}` climbs out of the repository"));
            }
            continue;
        };
        let first = path.split('/').next().unwrap_or_default();
        if private.contains(first) {
            out.push(format!("`{span}` names private `{first}`"));
        } else if LOCAL_LOCATIONS.contains(&first) {
        } else if path.contains('/') || link {
            if root.join(first).exists() && !root.join(&path).exists() {
                out.push(format!("`{span}` does not exist"));
            }
        } else if is_bare_document(&path) {
            let found = ["", "docs/", "docs/project/", ".github/"]
                .iter()
                .map(|p| format!("{p}{path}"))
                .chain(std::iter::once(format!("{dir}/{path}")))
                .any(|c| root.join(c.trim_start_matches('/')).is_file());
            if !found {
                out.push(format!(
                    "`{span}` names a document the public tree does not hold"
                ));
            }
        }
    }
    for name in private {
        let needle = format!("{name}/");
        let mut from = 0;
        while let Some(at) = text[from..].find(&needle) {
            let i = from + at;
            let before = text[..i].chars().next_back();
            if !before.is_some_and(|c| c.is_ascii_alphanumeric() || "_-./".contains(c)) {
                let line = text[..i].matches('\n').count() + 1;
                out.push(format!("line {line} mentions private `{needle}`"));
            }
            from = i + needle.len();
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Markdown under these prefixes is not prose a reader navigates, or is checked through its source.
const NOT_PROSE: &[&str] = &[
    "crates/logicaffeine_tests/tests/",
    "crates/logicaffeine_compile/assets/",
    "apps/logicaffeine_cli/assets/",
    "apps/logicaffeine_cli/templates/",
    "benchmarks/parity/",
];

/// Markdown a reader opens, plus the example projects' LOGOS sources: literate files whose notes are prose.
fn documents() -> Vec<String> {
    checkout()
        .into_iter()
        .filter(|p| p.ends_with(".md") || (p.starts_with("examples/") && p.ends_with(".lg")))
        .filter(|p| !p.starts_with("work/"))
        .filter(|p| !NOT_PROSE.iter().any(|n| p.starts_with(n)))
        .filter(|p| !p.ends_with("CHANGELOG.md") && p != "CLAUDE.md")
        .collect()
}

#[test]
fn the_scan_covers_the_documents_a_reader_opens() {
    let docs: BTreeSet<String> = documents().into_iter().collect();
    for must in [
        "README.md",
        "docs/README.md",
        "docs/cli.md",
        "AGENTS.md",
        "crates/logicaffeine_kernel/README.md",
        "examples/todo/pages/app.lg",
    ] {
        assert!(docs.contains(must), "`{must}` fell out of the scan");
    }
}

#[test]
fn the_detector_sees_each_citation_shape() {
    let private = BTreeSet::from(["work".to_string()]);
    let text = "See `work/plan.md`, [notes](../work/x.md), the work/ folder, `crates/nope/missing.rs`, `NOT_A_DOC.md`, \
                network/ and `target/debug` and `crates/logicaffeine_tests/Cargo.toml` and `..`.\n\
                ```bash\ncat [x](link) `crates/gone.rs`\n```\nLink issues: `([#123](link))`.\n";
    let found = violations("docs/guide.md", text, &private);
    let joined = found.join("\n");
    assert!(joined.contains("`work/plan.md` names private"), "{joined}");
    assert!(joined.contains("`../work/x.md` names private"), "{joined}");
    assert!(joined.contains("mentions private `work/`"), "{joined}");
    assert!(
        joined.contains("`crates/nope/missing.rs` does not exist"),
        "{joined}"
    );
    assert!(
        joined.contains("`NOT_A_DOC.md` names a document"),
        "{joined}"
    );
    assert!(
        !joined.contains("network"),
        "a word ending in `work/` is not the directory: {joined}"
    );
    assert!(
        !joined.contains("target/debug"),
        "a local build location may be named: {joined}"
    );
    assert!(
        !joined.contains("logicaffeine_tests/Cargo.toml"),
        "a real public path is fine: {joined}"
    );
    assert!(
        !joined.contains("`..`"),
        "a bare `..` in prose is not a link: {joined}"
    );
    assert!(
        !joined.contains("gone.rs") && !joined.contains("`link`"),
        "a fenced example cites nothing: {joined}"
    );
}

#[test]
fn public_documents_cite_only_the_public_tree() {
    let private = private_roots();
    let root = repo_root();
    let mut report = Vec::new();
    for doc in documents() {
        let text = std::fs::read_to_string(root.join(&doc)).unwrap_or_default();
        for v in violations(&doc, &text, &private) {
            report.push(format!("{doc}: {v}"));
        }
    }
    assert!(
        report.is_empty(),
        "public documents cite what the public tree does not hold (state the fact, or cite a public path):\n{}",
        report.join("\n")
    );
}

/// Words that assert a quality the sentence should demonstrate instead.
const FLUFF: &[&str] = &[
    "powerful",
    "seamless",
    "seamlessly",
    "robust",
    "robustly",
    "blazing",
    "blazingly",
    "cutting-edge",
    "unleash",
    "unleashes",
    "leverage",
    "leverages",
    "leveraging",
    "revolutionary",
    "revolutionize",
    "game-changing",
    "game-changer",
    "effortless",
    "effortlessly",
    "supercharge",
    "supercharged",
    "world-class",
    "state-of-the-art",
    "best-in-class",
    "delve",
    "delves",
];

/// Names from the workshop that a reader of the public docs was never introduced to.
const CODENAMES: &[&str] = &[
    "WS-G",
    "WS-F",
    "Lever B",
    "Mode A",
    "Mode B",
    "Mode-B",
    "(R2)",
    "(R4)",
    "(K3)",
    "(K6)",
    "§0.7",
    "Hotel California",
    "prime directive",
    "covenant",
    "gauntlet",
    "keystone",
    "rung",
    "rungs",
];

/// `text` as a reader hears it, line for line: fenced blocks, inline code and link targets blanked.
fn spoken(text: &str) -> Vec<String> {
    let mut fenced = false;
    text.lines()
        .map(|line| {
            if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
                fenced = !fenced;
                return String::new();
            }
            if fenced {
                return String::new();
            }
            let unticked: String = line.split('`').step_by(2).collect::<Vec<_>>().join(" ");
            let mut out = String::new();
            let mut rest = unticked.as_str();
            while let Some(at) = rest.find("](") {
                out.push_str(&rest[..at + 1]);
                rest = rest[at + 2..]
                    .split_once(')')
                    .map_or("", |(_, after)| after);
            }
            out.push_str(rest);
            out
        })
        .collect()
}

/// Whether `needle` occurs in `hay` as a whole word (no letter, digit or hyphen on either side).
fn has_word(hay: &str, needle: &str) -> bool {
    let glued = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '_');
    let mut from = 0;
    while let Some(at) = hay[from..].find(needle) {
        let i = from + at;
        if !glued(hay[..i].chars().next_back()) && !glued(hay[i + needle.len()..].chars().next()) {
            return true;
        }
        from = i + needle.len();
    }
    false
}

/// The slogan shape: "the single" and "truth" within three words of each other.
fn has_slogan(line: &str) -> bool {
    let words: Vec<String> = line
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric() && c != '-')
                .to_lowercase()
        })
        .collect();
    words.windows(2).enumerate().any(|(i, w)| {
        w[0] == "the" && w[1] == "single" && words[i + 2..].iter().take(3).any(|x| x == "truth")
    })
}

/// Every voice rule `text` breaks, by line.
fn voice(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, line) in spoken(text).iter().enumerate() {
        let lower = line.to_lowercase();
        for w in FLUFF {
            if has_word(&lower, w) {
                out.push(format!(
                    "line {}: `{w}` says a quality instead of showing it",
                    i + 1
                ));
            }
        }
        for c in CODENAMES {
            let found = if c.chars().any(char::is_uppercase) {
                line.contains(c)
            } else {
                has_word(&lower, c)
            };
            if found {
                out.push(format!("line {}: `{c}` is an internal name", i + 1));
            }
        }
        if has_slogan(line) {
            out.push(format!("line {}: a \"the single … truth\" slogan", i + 1));
        }
    }
    out
}

/// Every workspace member's Cargo `description`, by manifest.
fn descriptions() -> Vec<(String, String)> {
    let root = repo_root();
    let mut out = Vec::new();
    for parent in ["crates", "apps", "scripts"] {
        let Ok(entries) = std::fs::read_dir(root.join(parent)) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let manifest = entry.path().join("Cargo.toml");
            let Ok(text) = std::fs::read_to_string(&manifest) else {
                continue;
            };
            let Ok(value) = text.parse::<toml::Value>() else {
                continue;
            };
            if let Some(d) = value
                .get("package")
                .and_then(|p| p.get("description"))
                .and_then(|d| d.as_str())
            {
                out.push((
                    format!(
                        "{parent}/{}/Cargo.toml",
                        entry.file_name().to_string_lossy()
                    ),
                    d.to_string(),
                ));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn the_voice_detector_sees_each_shape() {
    let text = "A powerful, Seamless API.\nThe WS-G backend and Mode B.\nIt is the single teaching truth.\n\
                ```text\nrobust in a fence\n```\nA `robust` span, a [link](tests/gauntlet/x.txt), robustness, \
                multi-rung-free, the rungs of a ladder.\n";
    let found = voice(text).join("\n");
    assert!(found.contains("line 1: `powerful`"), "{found}");
    assert!(
        found.contains("line 1: `seamless`"),
        "case does not hide a word: {found}"
    );
    assert!(
        found.contains("line 2: `WS-G`") && found.contains("line 2: `Mode B`"),
        "{found}"
    );
    assert!(
        found.contains("line 3: a \"the single … truth\" slogan"),
        "{found}"
    );
    assert!(
        !found.contains("line 5") && !found.contains("gauntlet"),
        "fences, inline code and link targets are not prose: {found}"
    );
    assert!(
        found.contains("line 7: `rungs`"),
        "a line after a fence keeps its number: {found}"
    );
    assert!(
        !found.contains("`robust`"),
        "`robustness` and code spans are not the word: {found}"
    );
    assert!(
        found.contains("`rungs`") && !found.contains("`rung`"),
        "a hyphenated compound is not the word: {found}"
    );
}

#[test]
fn public_documents_speak_plainly() {
    let root = repo_root();
    let mut report = Vec::new();
    let mut sources = documents();
    sources.push("apps/logicaffeine_web/public/llms.txt".to_string());
    for doc in sources {
        let text =
            std::fs::read_to_string(root.join(&doc)).unwrap_or_else(|e| panic!("read {doc}: {e}"));
        report.extend(voice(&text).into_iter().map(|v| format!("{doc}: {v}")));
    }
    let described = descriptions();
    assert!(
        described.len() >= 18,
        "expected every member's description, found {}",
        described.len()
    );
    for (manifest, d) in described {
        report.extend(voice(&d).into_iter().map(|v| {
            format!(
                "{manifest} description: {}",
                v.trim_start_matches("line 1: ")
            )
        }));
    }
    assert!(
        report.is_empty(),
        "public prose that describes instead of saying:\n{}",
        report.join("\n")
    );
}
