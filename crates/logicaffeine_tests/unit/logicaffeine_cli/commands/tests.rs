use super::*;
use crate::project::manifest::Manifest;

fn manifest_with_entry(entry: &str) -> Manifest {
    let mut m = Manifest::new("probe");
    m.package.entry = entry.to_string();
    m
}

#[test]
fn entry_escaping_the_project_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    for bad in ["../outside.lg", "src/../../x.lg", "/etc/hostname"] {
        let err = resolve_entry_path(dir.path(), &manifest_with_entry(bad))
            .expect_err(&format!("{bad} must be rejected"));
        assert!(err.message.contains("escapes"), "{bad}: {}", err.message);
    }
}

#[test]
fn relative_entry_resolves_with_md_fallback() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.md"), "## Main\nShow 1.\n").unwrap();
    let path =
        resolve_entry_path(dir.path(), &manifest_with_entry("src/main.lg")).expect("md fallback");
    assert!(path.ends_with("src/main.md"));
}
