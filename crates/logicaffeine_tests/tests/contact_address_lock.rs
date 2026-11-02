//! Every shipped surface reaches the company at its own domain.
//!
//! The company is DatacenterDotDev Inc., and its contact address is `team@datacenter.dev`. The crates' author address,
//! the workspace manifest's authors field, is `tristen@datacenter.dev`. Any other mail domain in the workspace manifest,
//! the website, the legal pages, the security policy, `llms.txt`, a crate or a guide is
//! published the moment it ships: crates.io keeps a package's authors field forever, and the site deploys from the tree.
//! The search covers exactly what git would commit (tracked files plus untracked files no ignore rule covers), through
//! `git grep`, so local build output never slows it and nothing that ships escapes it. The needle is assembled at compile
//! time so this file never spells the address it bans.

use std::path::PathBuf;
use std::process::Command;

const RETIRED: &str = concat!("brahmastra", "-labs.com");
const COMPANY: &str = "team@datacenter.dev";
const AUTHOR: &str = "tristen@datacenter.dev";

const SURFACES: &[&str] = &[
    ".github/SECURITY.md",
    "apps/logicaffeine_web/terms.html",
    "apps/logicaffeine_web/privacy.html",
    "apps/logicaffeine_web/public/llms.txt",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

/// Every shippable file that holds `needle`, ignoring case, binary files included.
fn files_holding(needle: &str) -> Vec<String> {
    let out = Command::new("git")
        .args(["grep", "--untracked", "-i", "-l", "-F", "-e", needle])
        .current_dir(repo_root())
        .output()
        .expect("run git grep");
    match out.status.code() {
        Some(0) | Some(1) => String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect(),
        _ => panic!("git grep failed: {}", String::from_utf8_lossy(&out.stderr)),
    }
}

#[test]
fn the_search_reads_every_contact_surface() {
    let found = files_holding(COMPANY);
    for rel in SURFACES {
        assert!(found.iter().any(|f| f == rel), "the search never found {COMPANY} in {rel}; a search that misses the surfaces proves nothing");
    }
}

#[test]
fn no_shipped_file_gives_a_retired_mail_domain() {
    let offenders = files_holding(RETIRED);
    assert!(
        offenders.is_empty(),
        "these shipped files give an address at {RETIRED}; the company's is {COMPANY}:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn the_workspace_authors_field_is_tristens_address() {
    let manifest = std::fs::read_to_string(repo_root().join("Cargo.toml")).expect("read Cargo.toml");
    let authors = manifest
        .lines()
        .find(|l| l.trim_start().starts_with("authors"))
        .expect("the workspace manifest declares authors");
    assert!(authors.contains(AUTHOR), "Cargo.toml's authors field is `{authors}`; every crate inherits it and crates.io keeps it");
}
