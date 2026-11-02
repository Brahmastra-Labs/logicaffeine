//! LOGOS is MIT: everything this tree authors is under the MIT License in the root `LICENSE.md`, and every
//! package that ships carries that text.
//!
//! - `LICENSE.md` is the MIT License word for word, with the project's copyright line.
//! - Every package that ships, each crate `publish.yml` uploads and the VS Code extension, holds a copy
//!   byte-identical to it, and the tree holds no other copy.
//! - Every package manifest in the checkout declares MIT: each `Cargo.toml`, each `package.json`, and the root
//!   entry of each `package-lock.json`.
//! - Every SPDX license identifier in a file we author is MIT.
//! - What we did not write keeps the license its authors gave it. Each such tree is declared in
//!   [`THIRD_PARTY`] with its SPDX identifier and the file that states the license: that file is in the
//!   checkout, the SPDX identifiers inside the tree are the tree's own, the README's license line names it,
//!   and no license file sits outside the declared trees and the copies above.
//! - Nothing we author speaks of the Business Source License the tree carried until 2026-11-01, of the
//!   verification license key that went with it, or of rights reserved. The CHANGELOGs are history and are
//!   not read; [`QUOTING`] names each file that quotes a banned phrase on purpose, and each of its rows must
//!   still be needed.
//!
//! The checkout is what `git ls-files` lists, tracked or untracked but not ignored, with `work/` left out.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test license_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

const COPYRIGHT: &str = "Copyright (c) 2025-2026 Datacenter.Dev";

const MIT_PERMISSION: &str = "\
Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the \"Software\"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
";

/// The VS Code extension: it ships as a VSIX, which carries its own `LICENSE.md`.
const EXTENSION: &str = "editors/vscode/logicaffeine";

const SELF: &str = "crates/logicaffeine_tests/tests/license_lock.rs";

struct ThirdParty {
    /// A directory (ending in `/`) or a file-name prefix, repo-relative.
    path: &'static str,
    /// What the README's license line calls it.
    name: &'static str,
    /// Its license, as an SPDX identifier.
    license: &'static str,
    /// The file that states that license.
    text: &'static str,
}

/// Code this tree carries but did not write, each under the license its authors gave it.
const THIRD_PARTY: &[ThirdParty] = &[
    ThirdParty {
        path: "benchmarks/silicon/onboard/firmware/softfloat/",
        name: "Berkeley SoftFloat",
        license: "BSD-3-Clause",
        text: "benchmarks/silicon/onboard/firmware/softfloat/COPYING.txt",
    },
    ThirdParty {
        path: "crates/logicaffeine_tests/tests/fixtures/riscv-tests/",
        name: "the RISC-V ISA tests",
        license: "BSD-3-Clause",
        text: "crates/logicaffeine_tests/tests/fixtures/riscv-tests/LICENSE",
    },
    ThirdParty {
        path: "crates/logicaffeine_tests/tests/fixtures/lean_arena/",
        name: "the Lean kernel arena's cases",
        license: "Apache-2.0",
        text: "crates/logicaffeine_tests/tests/fixtures/lean_arena/LICENSE",
    },
    ThirdParty {
        path: "scripts/openlane-overlay/",
        name: "OpenLane's flow scripts",
        license: "Apache-2.0",
        text: "scripts/openlane-overlay/LICENSE",
    },
    ThirdParty {
        path: "apps/logicaffeine_web/public/assets/fonts/LogosMathMono",
        name: "the Math editor's font",
        license: "Bitstream-Vera",
        text: "apps/logicaffeine_web/public/assets/fonts/LogosMathMono-LICENSE.txt",
    },
    ThirdParty {
        path: "crates/logicaffeine_compile/src/codegen_sva/terminal/term_font",
        name: "the terminal font",
        license: "MIT",
        text: "crates/logicaffeine_compile/src/codegen_sva/terminal/term_font.LICENSE",
    },
    ThirdParty {
        path: "benchmarks/silicon/term_font",
        name: "the terminal font",
        license: "MIT",
        text: "benchmarks/silicon/term_font.LICENSE",
    },
];

#[derive(Clone, Copy)]
enum Match {
    /// The phrase as written.
    Exact,
    /// The phrase as written, with no letter, digit or `_` either side.
    Word,
    /// The phrase in any case.
    AnyCase,
}

/// What the tree said while it was under the Business Source License, and the verification license gate
/// that went with it, each with why it is wrong now.
const BANNED: &[(&str, Match, &str)] = &[
    ("BSL", Match::Word, "LOGOS is MIT; the Business Source License is gone"),
    ("Business Source", Match::Exact, "LOGOS is MIT; the Business Source License is gone"),
    ("BUSL", Match::Exact, "LOGOS is MIT; the Business Source License is gone"),
    ("Additional Use Grant", Match::Exact, "a BSL parameter; MIT grants every use"),
    ("Change Date", Match::Exact, "a BSL parameter; MIT applies from the day a version ships"),
    ("Licensed Work", Match::Exact, "a BSL parameter; MIT says the Software"),
    ("Logic Service", Match::Exact, "the one use the BSL reserved; MIT reserves none"),
    ("source-available", Match::AnyCase, "LOGOS is open source"),
    ("source available", Match::AnyCase, "LOGOS is open source"),
    ("all rights reserved", Match::AnyCase, "the MIT License grants the rights; we reserve none"),
    ("license key", Match::AnyCase, "verification takes no key"),
    ("LOGOS_LICENSE", Match::Exact, "verification takes no key"),
    ("license-gated", Match::AnyCase, "nothing is gated by a license"),
    ("license gated", Match::AnyCase, "nothing is gated by a license"),
    ("license gating", Match::AnyCase, "nothing is gated by a license"),
    ("setLicense", Match::Exact, "the extension stores no license"),
    ("Stripe license", Match::AnyCase, "no license is sold"),
    ("Pro and up", Match::Exact, "there are no plans; everything is free"),
];

/// Files that quote a banned phrase on purpose: the file, the phrases it may carry, and why.
const QUOTING: &[(&str, &[&str], &str)] = &[
    (
        "crates/logicaffeine_tests/tests/contact_page.rs",
        &["Business Source", "BUSL", "Logic Service", "license key", "LOGOS_LICENSE", "license-gated", "license gated"],
        "its shipped-surface lock lists the license-gate wording it bans",
    ),
    (
        "crates/logicaffeine_tests/tests/math_font_ships.rs",
        &["all rights reserved"],
        "it checks that the font ships the Bitstream Vera notice, which reads All Rights Reserved",
    ),
    (
        "editors/vscode/logicaffeine/test/integration/suite/server.test.ts",
        &["setLicense"],
        "its suite checks that the removed command stays unregistered",
    ),
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Every file a checkout of this tree holds, repo-relative, `work/` excluded.
fn checkout() -> Vec<String> {
    let root = repo_root();
    let out = Command::new("git")
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .current_dir(&root)
        .output()
        .expect("git ls-files");
    assert!(out.status.success(), "git ls-files failed: {}", String::from_utf8_lossy(&out.stderr));
    let mut paths: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|p| !p.is_empty() && !p.starts_with("work/"))
        .filter(|p| root.join(p).is_file())
        .map(str::to_string)
        .collect();
    paths.sort();
    paths.dedup();
    assert!(paths.len() > 5000, "the checkout lists only {} files; the walk broke, not the tree", paths.len());
    paths
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn third_party_of(path: &str) -> Option<&'static ThirdParty> {
    THIRD_PARTY.iter().find(|t| path.starts_with(t.path))
}

/// A file whose name says it states a license: `LICENSE`, `COPYING` or `NOTICE` (bare, `.md` or `.txt`), or a
/// name ending in `.LICENSE` or `-LICENSE.txt`.
fn is_license_file(path: &str) -> bool {
    let name = basename(path).to_ascii_lowercase();
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) => (stem.to_string(), ext.to_string()),
        None => (name.clone(), String::new()),
    };
    let bare = ["license", "licence", "copying", "notice"].contains(&stem.as_str()) && ["", "md", "txt"].contains(&ext.as_str());
    bare || name.ends_with(".license") || name.ends_with("-license.txt")
}

/// Every package `publish.yml` uploads: each package directly under `crates/`, `apps/` and `scripts/` whose
/// manifest does not say `publish = false`.
fn published_crates() -> Vec<String> {
    let root = repo_root();
    let mut out = Vec::new();
    for parent in ["crates", "apps", "scripts"] {
        let Ok(entries) = std::fs::read_dir(root.join(parent)) else { continue };
        for entry in entries.flatten() {
            let Ok(text) = std::fs::read_to_string(entry.path().join("Cargo.toml")) else { continue };
            let manifest: toml::Table = text.parse().unwrap_or_else(|e| panic!("{}/Cargo.toml: {e}", entry.path().display()));
            let Some(package) = manifest.get("package") else { continue };
            if package.get("publish").and_then(toml::Value::as_bool) != Some(false) {
                out.push(format!("{parent}/{}", entry.file_name().to_string_lossy()));
            }
        }
    }
    out.sort();
    assert!(out.len() >= 10, "found only {} published crates; the discovery walk broke", out.len());
    out
}

/// The SPDX license identifiers a text states, in order.
fn spdx_identifiers(text: &str) -> Vec<String> {
    let tag = concat!("SPDX-License", "-Identifier:");
    text.match_indices(tag)
        .map(|(at, _)| {
            text[at + tag.len()..]
                .trim_start_matches([' ', '\t'])
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || "-.+".contains(*c))
                .collect()
        })
        .collect()
}

/// Whether `text` (with `lower`, its lowercase) says `phrase` the way `how` reads it.
fn says(text: &str, lower: &str, phrase: &str, how: Match) -> bool {
    match how {
        Match::Exact => text.contains(phrase),
        Match::AnyCase => lower.contains(&phrase.to_lowercase()),
        Match::Word => text.match_indices(phrase).any(|(at, _)| {
            let wordy = |c: char| c.is_ascii_alphanumeric() || c == '_';
            !text[..at].chars().next_back().is_some_and(wordy) && !text[at + phrase.len()..].chars().next().is_some_and(wordy)
        }),
    }
}

/// The banned phrases a text carries.
fn banned_in(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    BANNED.iter().filter(|(phrase, how, _)| says(text, &lower, phrase, *how)).map(|(phrase, _, _)| *phrase).collect()
}

/// The files the authored-text checks read: the checkout less the CHANGELOGs, this lock, and the trees we
/// did not write, with the text of each one that is UTF-8.
fn authored_texts() -> Vec<(String, String)> {
    let root = repo_root();
    let texts: Vec<(String, String)> = checkout()
        .into_iter()
        .filter(|p| basename(p) != "CHANGELOG.md" && p != SELF && third_party_of(p).is_none())
        .filter_map(|p| std::fs::read_to_string(root.join(&p)).ok().map(|text| (p, text)))
        .collect();
    assert!(texts.len() > 5000, "read only {} authored text files; the walk broke, not the tree", texts.len());
    texts
}

#[test]
fn the_license_is_the_mit_license_with_the_projects_copyright_line() {
    assert_eq!(
        read("LICENSE.md"),
        format!("MIT License\n\n{COPYRIGHT}\n\n{MIT_PERMISSION}"),
        "LICENSE.md must be the MIT License word for word, under the line {COPYRIGHT:?}"
    );
}

#[test]
fn every_shipped_package_carries_the_license_and_nothing_else_copies_it() {
    let root_text = read("LICENSE.md");
    let mut expected: BTreeSet<String> = published_crates().iter().map(|c| format!("{c}/LICENSE.md")).collect();
    expected.insert(format!("{EXTENSION}/LICENSE.md"));
    expected.insert("LICENSE.md".to_string());
    let present: BTreeSet<String> = checkout()
        .into_iter()
        .filter(|p| basename(p) == "LICENSE.md" && third_party_of(p).is_none())
        .collect();
    let missing: Vec<&String> = expected.difference(&present).collect();
    assert!(missing.is_empty(), "these shipped packages carry no copy of the license: {missing:?}");
    let stray: Vec<&String> = present.difference(&expected).collect();
    assert!(stray.is_empty(), "these copies of the license belong to no shipped package: {stray:?}");
    for copy in &expected {
        assert!(read(copy) == root_text, "{copy} is not byte-identical to LICENSE.md");
    }
}

#[test]
fn every_package_manifest_declares_mit() {
    let root = repo_root();
    let mut problems = Vec::new();
    let mut seen = 0;
    for path in checkout().into_iter().filter(|p| third_party_of(p).is_none()) {
        let declared = match basename(&path) {
            "Cargo.toml" => {
                let manifest: toml::Table = read(&path).parse().unwrap_or_else(|e| panic!("{path}: {e}"));
                let workspace_license = manifest
                    .get("workspace")
                    .and_then(|w| w.get("package"))
                    .and_then(|p| p.get("license"))
                    .and_then(toml::Value::as_str);
                match manifest.get("package").and_then(|p| p.get("license")) {
                    Some(toml::Value::String(s)) => Some(s.clone()),
                    Some(toml::Value::Table(t)) if t.get("workspace").and_then(toml::Value::as_bool) == Some(true) => {
                        let workspace: toml::Table = read("Cargo.toml").parse().expect("the root Cargo.toml");
                        workspace["workspace"]["package"]["license"].as_str().map(str::to_string)
                    }
                    Some(other) => Some(other.to_string()),
                    None if manifest.get("package").is_none() => workspace_license.map(str::to_string),
                    None => None,
                }
            }
            "package.json" => {
                let manifest: serde_json::Value =
                    serde_json::from_str(&read(&path)).unwrap_or_else(|e| panic!("{path}: {e}"));
                manifest.get("license").and_then(serde_json::Value::as_str).map(str::to_string)
            }
            "package-lock.json" => {
                let lock: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(&path)).unwrap())
                    .unwrap_or_else(|e| panic!("{path}: {e}"));
                lock["packages"][""].get("license").and_then(serde_json::Value::as_str).map(str::to_string)
            }
            _ => continue,
        };
        seen += 1;
        if declared.as_deref() != Some("MIT") {
            problems.push(format!("{path} declares {declared:?}"));
        }
    }
    assert!(seen >= 30, "read only {seen} manifests; the walk broke, not the tree");
    assert!(problems.is_empty(), "these manifests do not declare MIT:\n  {}", problems.join("\n  "));
}

#[test]
fn every_spdx_identifier_is_mit_or_its_third_party_trees() {
    let root = repo_root();
    let mut problems = Vec::new();
    for path in checkout().into_iter().filter(|p| p != SELF) {
        let Ok(text) = std::fs::read_to_string(root.join(&path)) else { continue };
        let allowed = third_party_of(&path).map_or("MIT", |t| t.license);
        for id in spdx_identifiers(&text) {
            if id != allowed {
                problems.push(format!("{path} says {id}, not {allowed}"));
            }
        }
    }
    assert!(problems.is_empty(), "SPDX identifiers that are not this tree's:\n  {}", problems.join("\n  "));
}

#[test]
fn every_third_party_tree_states_its_license_and_no_other_file_does() {
    let files: BTreeSet<String> = checkout().into_iter().collect();
    for t in THIRD_PARTY {
        assert!(files.contains(t.text), "{} keeps its license in {}, which the checkout lacks", t.path, t.text);
        assert!(files.iter().any(|f| f.starts_with(t.path) && f != t.text), "{} holds nothing but its license", t.path);
    }
    let stray: Vec<&String> = files
        .iter()
        .filter(|f| is_license_file(f) && third_party_of(f).is_none())
        .filter(|f| basename(f) != "LICENSE.md")
        .collect();
    assert!(stray.is_empty(), "license files outside every declared third-party tree: {stray:?}");
}

#[test]
fn the_readme_names_the_license_and_every_third_party_one() {
    let readme = read("README.md");
    let mut lines = readme.lines().skip_while(|l| !l.starts_with("- **License**:"));
    let first = lines.next().expect("README.md has a `- **License**:` item under ## Project");
    let line = std::iter::once(first)
        .chain(lines.take_while(|l| l.starts_with("  ")))
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ");
    assert!(line.starts_with("- **License**: [MIT](LICENSE.md)"), "the README's license line does not lead with MIT: {line}");
    for t in THIRD_PARTY {
        assert!(
            line.contains(&format!("{} ({})", t.name, t.license)),
            "the README's license line does not name {} ({}): {line}",
            t.name,
            t.license
        );
    }
}

#[test]
fn nothing_we_author_speaks_of_the_old_license_or_its_key() {
    let mut problems = Vec::new();
    let mut quoted: BTreeSet<(&str, &str)> = BTreeSet::new();
    for (path, text) in authored_texts() {
        let quoting = QUOTING.iter().find(|(f, _, _)| *f == path);
        for phrase in banned_in(&text) {
            if let Some((file, _, _)) = quoting.filter(|(_, phrases, _)| phrases.contains(&phrase)) {
                quoted.insert((*file, phrase));
                continue;
            }
            let why = BANNED.iter().find(|(p, _, _)| *p == phrase).unwrap().2;
            problems.push(format!("{path} says {phrase:?}: {why}"));
        }
    }
    assert!(problems.is_empty(), "authored files that speak of the old license:\n  {}", problems.join("\n  "));
    let unneeded: Vec<String> = QUOTING
        .iter()
        .flat_map(|(file, phrases, _)| phrases.iter().map(move |p| (*file, *p)))
        .filter(|row| !quoted.contains(row))
        .map(|(file, phrase)| format!("{file} no longer quotes {phrase:?}"))
        .collect();
    assert!(unneeded.is_empty(), "QUOTING rows nothing needs; delete them:\n  {}", unneeded.join("\n  "));
}

#[test]
fn the_scan_sees_each_banned_phrase_and_only_where_it_stands() {
    for (phrase, how, _) in BANNED {
        let shown = match how {
            Match::AnyCase => phrase.to_uppercase(),
            _ => phrase.to_string(),
        };
        assert_eq!(banned_in(&format!("a line that says {shown}.")), vec![*phrase], "the scan misses {phrase:?}");
    }
    for innocent in ["fabsl(x)", "CbSlot", "BSLV", "x_BSL", "a stripe of lines"] {
        assert!(banned_in(innocent).is_empty(), "the scan flags {innocent:?}: {:?}", banned_in(innocent));
    }
    assert_eq!(spdx_identifiers(concat!("// SPDX-License", "-Identifier: Apache-2.0\n")), ["Apache-2.0"]);
    assert_eq!(spdx_identifiers(concat!("<!-- SPDX-License", "-Identifier: MIT -->")), ["MIT"]);
    assert_eq!(spdx_identifiers(concat!("\"// SPDX-License", "-Identifier: MIT\\n\"")), ["MIT"]);
    assert!(is_license_file("x/LICENSE") && is_license_file("x/COPYING.txt") && is_license_file("x/NOTICE"));
    assert!(is_license_file("x/term_font.LICENSE") && is_license_file("x/Font-LICENSE.txt"));
    assert!(!is_license_file("src/license.ts") && !is_license_file("src/license.rs"));
}
