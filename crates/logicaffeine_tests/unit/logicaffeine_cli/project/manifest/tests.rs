use super::*;

#[test]
fn parse_minimal_manifest() {
    let toml = r#"
[package]
name = "myproject"
"#;
    let manifest: Manifest = toml::from_str(toml).expect("Should parse minimal manifest");
    assert_eq!(manifest.package.name, "myproject");
    assert_eq!(manifest.package.version, "0.1.0"); // default
    assert_eq!(manifest.package.entry, "src/main.lg"); // default
}

#[test]
fn parse_full_manifest() {
    let toml = r#"
[package]
name = "myproject"
version = "1.0.0"
description = "A test project"
entry = "src/app.lg"
authors = ["Test Author"]

[dependencies]
std = "logos:std"
"#;
    let manifest: Manifest = toml::from_str(toml).expect("Should parse full manifest");
    assert_eq!(manifest.package.name, "myproject");
    assert_eq!(manifest.package.version, "1.0.0");
    assert_eq!(manifest.package.entry, "src/app.lg");
    assert!(manifest.package.description.is_some());
    assert_eq!(manifest.package.authors.len(), 1);
}

#[test]
fn create_new_manifest() {
    let manifest = Manifest::new("testproject");
    assert_eq!(manifest.package.name, "testproject");
    let toml = manifest.to_toml().expect("Should serialize");
    assert!(toml.contains("name = \"testproject\""));
}

#[test]
fn parse_path_dependency() {
    let toml = r#"
[package]
name = "with_deps"

[dependencies]
math = { path = "./math" }
"#;
    let manifest: Manifest = toml::from_str(toml).expect("Should parse path deps");
    assert!(!manifest.dependencies.is_empty());
    match &manifest.dependencies["math"] {
        DependencySpec::Detailed(d) => {
            assert_eq!(d.path.as_deref(), Some("./math"));
        }
        _ => panic!("Expected detailed dependency"),
    }
}

/// A path dependency resolves to its canonical root; a version/`logos:`
/// dependency is skipped (not linkable from the local closure, never an error).
#[test]
fn resolves_a_path_dependency_and_skips_registry_forms() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("shared/geometry/src")).unwrap();
    let toml = r#"
[package]
name = "app"

[dependencies]
std = "logos:std"
serde = "1.0"
geometry = { path = "shared/geometry" }
"#;
    let manifest: Manifest = toml::from_str(toml).unwrap();
    let resolved = manifest.resolve_dependencies(root.path()).unwrap();
    // Only the path dep is linkable; `std`/`serde` are skipped silently.
    assert_eq!(resolved.len(), 1, "only the path dep links: {resolved:?}");
    assert_eq!(resolved[0].name, "geometry");
    assert_eq!(resolved[0].source, DepSource::Path);
    assert_eq!(
        resolved[0].root,
        root.path().join("shared/geometry").canonicalize().unwrap()
    );
}

/// A git dependency without an exact rev is a floating (false) build.
#[test]
fn a_git_dependency_without_a_rev_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let toml = r#"
[package]
name = "app"

[dependencies]
r = { git = "https://example.com/r.git", branch = "main" }
"#;
    let manifest: Manifest = toml::from_str(toml).unwrap();
    let err = manifest.resolve_dependencies(root.path()).unwrap_err();
    assert!(
        matches!(err, DependencyError::FloatingGit { .. }),
        "{err:?}"
    );
}

/// A git dependency whose rev is not a full 40-hex commit is malformed.
#[test]
fn a_git_dependency_with_a_short_rev_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("vendor/src")).unwrap();
    let toml = r#"
[package]
name = "app"

[dependencies]
r = { git = "https://example.com/r.git", rev = "abc123", path = "vendor" }
"#;
    let manifest: Manifest = toml::from_str(toml).unwrap();
    let err = manifest.resolve_dependencies(root.path()).unwrap_err();
    assert!(
        matches!(err, DependencyError::MalformedRev { .. }),
        "{err:?}"
    );
}

/// An exactly-pinned git dependency resolves against its local checkout.
#[test]
fn an_exactly_pinned_git_dependency_resolves() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("vendor/src")).unwrap();
    let rev = "0123456789abcdef0123456789abcdef01234567";
    let toml = format!(
        r#"
[package]
name = "app"

[dependencies]
r = {{ git = "https://example.com/r.git", rev = "{rev}", path = "vendor" }}
"#
    );
    let manifest: Manifest = toml::from_str(&toml).unwrap();
    let resolved = manifest.resolve_dependencies(root.path()).unwrap();
    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolved[0].source,
        DepSource::Git {
            url: "https://example.com/r.git".into(),
            rev: rev.into()
        }
    );
    assert_eq!(
        resolved[0].root,
        root.path().join("vendor").canonicalize().unwrap()
    );
}

/// A path dependency whose directory does not exist is a hard error.
#[test]
fn a_missing_path_dependency_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let toml = r#"
[package]
name = "app"

[dependencies]
gone = { path = "no/such/place" }
"#;
    let manifest: Manifest = toml::from_str(toml).unwrap();
    let err = manifest.resolve_dependencies(root.path()).unwrap_err();
    assert!(matches!(err, DependencyError::Missing { .. }), "{err:?}");
}
