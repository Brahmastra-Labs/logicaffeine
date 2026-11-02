//! Integration tests live in `crates/logicaffeine_tests`, and nowhere else.
//!
//! The suite has one home: `crates/logicaffeine_tests/tests/`, where a test can depend on every
//! crate, its fixtures sit beside it, and nothing is ever packaged for the registry. No other
//! workspace package keeps a `tests/` directory: an integration test written anywhere else belongs
//! here, under the same file name (a crate prefix when the name is taken).
//!
//! Crates `PUBLIC_MANIFEST.toml` holds back are out of scope: the public test crate may never
//! depend on them, so their tests stay with them. White-box unit tests live here too, under
//! `unit/`, behind a one-line stub in their crate (`unit_home_lock`).

#[path = "common/public_manifest.rs"]
mod public_manifest;

use public_manifest::{held_back, packages, repo_root, Classifier};

const HOME: &str = "crates/logicaffeine_tests";

/// Package directories this lock governs: every workspace package but the test crate and the
/// held-back ones.
fn governed() -> Vec<String> {
    let held = held_back(&Classifier::load());
    let all: Vec<String> = packages()
        .into_keys()
        .filter(|dir| dir != HOME && !held.contains_key(dir))
        .collect();
    assert!(
        all.len() >= 10,
        "found only {} governed packages — the discovery walk broke",
        all.len()
    );
    all
}

#[test]
fn no_package_but_the_test_crate_has_a_tests_directory() {
    let root = repo_root();
    let stray: Vec<String> = governed()
        .into_iter()
        .map(|dir| format!("{dir}/tests"))
        .filter(|tests| root.join(tests).exists())
        .collect();
    assert!(
        stray.is_empty(),
        "{} package(s) keep a tests/ directory outside {HOME}:\n  {}\n\n\
         Integration tests live in {HOME}/tests/. Move the file there under the same name (prefix it \
         with the crate's short name if the name is taken); it becomes a binary of package \
         `logicaffeine-tests`, so update anything that scopes it by package: a runner's `-p` \
         (scripts/run-*.sh), a `.config/nextest.toml` `package()` filter, an exclusion list. If the \
         test needs a dependency or feature {HOME} lacks, add it to {HOME}/Cargo.toml, forwarding a \
         feature to the crate it names.",
        stray.len(),
        stray.join("\n  ")
    );
}

/// The lock sees what it claims to: the test crate's own `tests/` exists and held-back packages are
/// excluded rather than silently counted as clean.
#[test]
fn the_lock_sees_the_home_and_the_held_back_crates() {
    let root = repo_root();
    assert!(
        root.join(HOME).join("tests").is_dir(),
        "{HOME}/tests is missing — the lock would pass on nothing"
    );
    let held = held_back(&Classifier::load());
    for dir in held.keys() {
        assert!(
            !governed().contains(dir),
            "held-back {dir} must be out of scope"
        );
    }
}
