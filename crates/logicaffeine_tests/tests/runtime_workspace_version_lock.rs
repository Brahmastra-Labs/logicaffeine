//! A build stages the runtime crates from the workspace `largo` finds (`LOGOS_WORKSPACE` first).
//! Pointed at a checkout of another version, the staged runtime no longer matches the Rust this
//! compiler generates, and rustc's errors were reported as "a LOGOS compiler bug". At the energy
//! hackathon, largo 0.11.0 with `LOGOS_WORKSPACE` on a 0.10.1 checkout failed with "cannot find
//! `failure` in `logicaffeine_system`" under that label. The cause is the version mismatch, so
//! staging now refuses it before anything is built, naming both versions and the variable.
//!
//! One test drives every case, because `LOGOS_WORKSPACE` is process-wide state.

use std::fs;
use std::path::Path;

use logicaffeine_compile::compile::copy_runtime_crates;

/// A checkout skeleton at `version`: the root manifest and the three runtime crates' folders.
fn fake_checkout(root: &Path, version: &str) {
    fs::write(
        root.join("Cargo.toml"),
        format!(
            "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"{version}\"\nedition = \"2021\"\nrust-version = \"1.85\"\n"
        ),
    )
    .expect("root manifest");
    for krate in ["logicaffeine_data", "logicaffeine_system", "logicaffeine_base"] {
        let dir = root.join("crates").join(krate);
        fs::create_dir_all(dir.join("src")).expect("crate folder");
        fs::write(
            dir.join("Cargo.toml"),
            format!("[package]\nname = \"{}\"\nversion.workspace = true\nedition.workspace = true\n", krate.replace('_', "-")),
        )
        .expect("crate manifest");
        fs::write(dir.join("src/lib.rs"), "").expect("crate source");
    }
}

#[test]
fn staging_refuses_a_workspace_of_another_version_and_says_so() {
    let ours = env!("CARGO_PKG_VERSION");

    // A checkout of another version is refused, by naming the mismatch, never as a compiler bug.
    let old = tempfile::tempdir().expect("tempdir");
    fake_checkout(old.path(), "0.10.1");
    let out = tempfile::tempdir().expect("tempdir");
    std::env::set_var("LOGOS_WORKSPACE", old.path());
    let refused = copy_runtime_crates(out.path());
    std::env::remove_var("LOGOS_WORKSPACE");
    let message = format!("{:?}", refused.expect_err("a 0.10.1 runtime must not be staged for this compiler"));
    for part in ["0.10.1", ours, "LOGOS_WORKSPACE", "version"] {
        assert!(message.contains(part), "the refusal must name `{part}`: {message}");
    }
    assert!(!message.contains("compiler bug"), "a version mismatch is not a compiler bug: {message}");
    assert!(
        !out.path().join("crates/logicaffeine_system").exists(),
        "nothing may be staged from a mismatched checkout"
    );

    // The same checkout at this compiler's own version stages.
    let same = tempfile::tempdir().expect("tempdir");
    fake_checkout(same.path(), ours);
    let out = tempfile::tempdir().expect("tempdir");
    std::env::set_var("LOGOS_WORKSPACE", same.path());
    let staged = copy_runtime_crates(out.path());
    std::env::remove_var("LOGOS_WORKSPACE");
    staged.unwrap_or_else(|e| panic!("a checkout at {ours} must stage: {e:?}"));
    assert!(out.path().join("crates/logicaffeine_system/Cargo.toml").exists());
}
