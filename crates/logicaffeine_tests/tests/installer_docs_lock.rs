//! Installer-docs ratchet — the one-line install command a user copies must be
//! byte-identical everywhere it's presented, and the platform/flavor facts must
//! not drift between the READMEs, the CLI, the web pages, the VSCode surfaces,
//! and the scripts themselves. One canonical string, checked against every
//! surface, so a domain change or a mistyped flag fails loudly at test time
//! instead of silently shipping a broken `curl … | sh`.
//!
//! Each check is its own `#[test]` so a failure names the violated invariant;
//! the surface lists below are the registry — a moved/renamed doc fails the
//! `every_listed_surface_exists` guard rather than silently dropping coverage.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(installer_docs_lock)'`

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

/// The canonical Unix install command. If this ever changes, every surface in
/// `CURL_SURFACES` changes with it — that is the whole point.
const CURL: &str = "curl -fsSL https://logicaffeine.com/install.sh | sh";

/// The canonical Windows install command.
const PS1: &str = "irm https://logicaffeine.com/install.ps1 | iex";

/// The documented way to ask the installer for the Z3 (full) flavor.
const FULL_FORM: &str = "sh -s -- --full";

/// The install location the docs promise (no sudo, no rc-file edits).
const INSTALL_DIR: &str = ".local/bin";

/// Every surface that presents the Unix install command. Adding a new place the
/// command appears? Add it here so drift there is caught too.
const CURL_SURFACES: &[&str] = &[
    "README.md",
    "docs/getting-started.md",
    "apps/logicaffeine_web/public/llms.txt",
    "docs/cli.md",
    "docs/project/VERSIONING.md",
    "CHANGELOG.md",
    "apps/logicaffeine_cli/README.md",
    "apps/logicaffeine_cli/src/commands/verify.rs",
    "apps/logicaffeine_web/public/install.sh",
    "apps/logicaffeine_web/src/ui/pages/guide/content/part1.rs",
    "apps/logicaffeine_web/src/ui/pages/news/data.rs",
    "apps/logicaffeine_web/src/ui/pages/contact/content.rs",
    "editors/vscode/logicaffeine/README.md",
    "editors/vscode/logicaffeine/media/walkthrough/install-largo.md",
];

/// Every surface that presents the Windows install command.
const PS1_SURFACES: &[&str] = &[
    "README.md",
    "docs/getting-started.md",
    "apps/logicaffeine_web/public/llms.txt",
    "docs/cli.md",
    "docs/project/VERSIONING.md",
    "CHANGELOG.md",
    "apps/logicaffeine_cli/README.md",
    "apps/logicaffeine_web/public/install.ps1",
    "apps/logicaffeine_web/public/install.sh",
    "apps/logicaffeine_web/src/ui/pages/guide/content/part1.rs",
    "apps/logicaffeine_web/src/ui/pages/news/data.rs",
    "apps/logicaffeine_web/src/ui/pages/contact/content.rs",
    "editors/vscode/logicaffeine/README.md",
    "editors/vscode/logicaffeine/media/walkthrough/install-largo.md",
];

/// Surfaces that spell out the full-flavor invocation. They must all use the
/// exact `sh -s -- --full` piping form (a bare `| sh --full` never reaches the
/// script's arg parser — the flag would go to `sh`).
const FULL_SURFACES: &[&str] = &[
    "README.md",
    "apps/logicaffeine_cli/src/commands/verify.rs",
    "apps/logicaffeine_web/public/install.sh",
    "apps/logicaffeine_web/src/ui/pages/guide/content/part1.rs",
];

/// Surfaces that state where the binary lands. They must agree on `.local/bin`.
const INSTALL_DIR_SURFACES: &[&str] = &[
    "docs/getting-started.md",
    "docs/cli.md",
    "apps/logicaffeine_cli/README.md",
    "apps/logicaffeine_web/public/install.sh",
];

/// Substrings that would mean a competing/stale install *channel* slipped in.
/// Scoped to install paths so legitimate `raw.githubusercontent.com` badge URLs
/// don't trip it; a real second install channel must be added deliberately (and
/// this list updated) rather than drift in unnoticed.
const FORBIDDEN_HOSTS: &[&str] = &[
    "logicaffeine.dev/install",
    "logicaffeine.io/install",
    "logicaffeine.org/install",
    "get.logicaffeine",
    "sh.logicaffeine",
    "githubusercontent.com/Brahmastra-Labs/logicaffeine/main/install",
    "githubusercontent.com/Brahmastra-Labs/logicaffeine/refs/heads/main/install",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

/// The union of every surface this lock reasons about — used by the existence
/// guard and the forbidden-host scan.
fn all_surfaces() -> Vec<&'static str> {
    let mut v: Vec<&str> = CURL_SURFACES
        .iter()
        .chain(PS1_SURFACES)
        .chain(FULL_SURFACES)
        .chain(INSTALL_DIR_SURFACES)
        .copied()
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

#[test]
fn every_listed_surface_exists() {
    for rel in all_surfaces() {
        assert!(
            repo_root().join(rel).exists(),
            "installer-docs surface `{rel}` is listed in installer_docs_lock but does not exist \
             — it was moved or renamed; update the surface list so coverage isn't silently lost"
        );
    }
}

#[test]
fn canonical_curl_command_on_every_surface() {
    for rel in CURL_SURFACES {
        assert!(
            read(rel).contains(CURL),
            "`{rel}` does not contain the canonical Unix install command `{CURL}` \
             — installer docs have drifted apart"
        );
    }
}

#[test]
fn canonical_ps1_command_on_every_surface() {
    for rel in PS1_SURFACES {
        assert!(
            read(rel).contains(PS1),
            "`{rel}` does not contain the canonical Windows install command `{PS1}` \
             — installer docs have drifted apart"
        );
    }
}

#[test]
fn full_flavor_uses_the_documented_piping_form() {
    for rel in FULL_SURFACES {
        assert!(
            read(rel).contains(FULL_FORM),
            "`{rel}` mentions the full flavor but not via the documented `{FULL_FORM}` form \
             — a bare `| sh --full` sends the flag to sh, not the installer"
        );
    }
}

#[test]
fn install_scripts_self_document_their_own_command() {
    assert!(
        read("apps/logicaffeine_web/public/install.sh").contains(CURL),
        "install.sh header must document its own `{CURL}` invocation"
    );
    assert!(
        read("apps/logicaffeine_web/public/install.ps1").contains(PS1),
        "install.ps1 header must document its own `{PS1}` invocation"
    );
}

#[test]
fn install_location_fact_is_consistent() {
    for rel in INSTALL_DIR_SURFACES {
        assert!(
            read(rel).contains(INSTALL_DIR),
            "`{rel}` states the install location but not the promised `{INSTALL_DIR}` \
             — the no-sudo install-dir claim has drifted"
        );
    }
}

#[test]
fn no_competing_install_host_slips_in() {
    for rel in all_surfaces() {
        let body = read(rel);
        for bad in FORBIDDEN_HOSTS {
            assert!(
                !body.contains(bad),
                "`{rel}` references a competing/stale install host `{bad}` — \
                 the one canonical channel is `logicaffeine.com/install.sh`"
            );
        }
    }
}
