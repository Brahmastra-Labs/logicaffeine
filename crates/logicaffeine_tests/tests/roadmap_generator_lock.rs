//! The roadmap generator takes a release's curated `Highlights` lead even when
//! another category appears first, and preserves the legacy fallback for older
//! releases without a Highlights section.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test roadmap_generator_lock`

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn put(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn highlights_are_the_release_headline_even_when_fixed_comes_first() {
    let root = std::env::temp_dir().join(format!("logos_roadmap_generator_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::copy(
        repo_root().join("scripts/generate-roadmap.sh"),
        root.join("scripts/generate-roadmap.sh"),
    )
    .unwrap();
    put(
        &root,
        "Cargo.toml",
        "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"0.11.0\"\n",
    );
    put(
        &root,
        "CHANGELOG.md",
        "# Changelog\n\n\
         ## [Unreleased]\n\n\
         ## [0.11.0] - 2026-10-03\n\n\
         ### Fixed\n\n\
         - A late internal repair.\n\n\
         ### Highlights\n\n\
         - The intended public headline.\n\n\
         ## [0.10.1] - 2026-09-01\n\n\
         ### Fixed\n\n\
         - The legacy fallback headline.\n",
    );
    std::fs::create_dir_all(root.join("apps/logicaffeine_web/src/ui/pages")).unwrap();

    for args in [
        &["init", "-q"][..],
        &["add", "."][..],
        &[
            "-c",
            "user.name=Roadmap Lock",
            "-c",
            "user.email=roadmap@example.invalid",
            "commit",
            "-qm",
            "fixture",
        ][..],
    ] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(&root)
            .status()
            .expect("git")
            .success());
    }

    let output = Command::new("bash")
        .arg(root.join("scripts/generate-roadmap.sh"))
        .current_dir(&root)
        .output()
        .expect("run generate-roadmap.sh");
    assert!(
        output.status.success(),
        "generator failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let history = std::fs::read_to_string(
        root.join("apps/logicaffeine_web/src/ui/pages/roadmap_history.json"),
    )
    .unwrap();
    let intended = history
        .find("\"title\": \"The intended public headline\"")
        .expect("the Highlights lead is emitted");
    let fallback = history
        .find("\"title\": \"The legacy fallback headline\"")
        .expect("a release without Highlights retains its fallback");
    assert!(
        intended < fallback,
        "newest-first release order:\n{history}"
    );

    let _ = std::fs::remove_dir_all(root);
}
