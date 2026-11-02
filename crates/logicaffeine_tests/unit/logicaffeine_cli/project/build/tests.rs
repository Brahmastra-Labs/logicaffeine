use super::*;
use tempfile::tempdir;

/// A project root holding a `Largo.toml` whose entry is `src/main.lg`.
fn manifest_in(dir: &Path) {
    std::fs::write(
        dir.join("Largo.toml"),
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    )
    .unwrap();
}

/// A plain (non-`Serve`) program is NEVER a web bundle — even with a stray
/// `pages/` dir, no `Serve` means the embed hook stays inert.
#[test]
fn plan_web_bundle_is_none_for_a_plain_program() {
    let dir = tempdir().unwrap();
    manifest_in(dir.path());
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let entry = dir.path().join("src/main.lg");
    std::fs::write(&entry, "# M\n\n## Main\nShow \"hi\".\n").unwrap();
    // A pages/ dir alone must not flip a non-Serve program into a web build.
    std::fs::create_dir_all(dir.path().join("pages")).unwrap();
    let plan = plan_web_bundle(dir.path()).unwrap();
    assert!(plan.is_none(), "a program with no Serve is never embedded");
}

/// A `Serve` backend with NO `pages/`/`static/` is a backend-only program:
/// no assets to embed, so the hook stays inert and the differential holds.
#[test]
fn plan_web_bundle_is_none_for_a_backend_with_no_zones() {
    let dir = tempdir().unwrap();
    manifest_in(dir.path());
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let entry = dir.path().join("src/main.lg");
    std::fs::write(
        &entry,
        "# S\n\n## To handle (request: Request) -> Response:\n    Return plain(\"hi\").\n\n## Main\nServe on port 8080 with handle.\n",
    )
    .unwrap();
    let plan = plan_web_bundle(dir.path()).unwrap();
    assert!(
        plan.is_none(),
        "a Serve with no pages/ or static/ has nothing to embed"
    );
}

/// A `Serve` backend WITH a `pages/`/`static/` zone is a web project: the
/// whole site is planned into the embed table.
#[test]
fn plan_web_bundle_embeds_a_serve_with_pages() {
    let dir = tempdir().unwrap();
    manifest_in(dir.path());
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let entry = dir.path().join("src/main.lg");
    std::fs::write(
        &entry,
        "# S\n\n## To handle (request: Request) -> Response:\n    Return plain(\"hi\").\n\n## Main\nServe on port 8080 with handle.\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("pages")).unwrap();
    std::fs::write(
        dir.path().join("pages/app.lg"),
        "## Component App\nView:\n    <h1>hi</h1>\n\n## Main\nMount App.\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("static")).unwrap();
    std::fs::write(dir.path().join("static/style.css"), b".card{}").unwrap();

    let plan = plan_web_bundle(dir.path())
        .unwrap()
        .expect("a web project embeds");
    let paths: std::collections::HashSet<&str> = plan.iter().map(|a| a.path.as_str()).collect();
    assert!(paths.contains("/"), "front page: {paths:?}");
    // Content-addressed: `/app.<hash>.wasm`, so the module can be cached forever
    // (`asset_cache_lock`). Asserted by shape — the hash is of the bytes, which is the point.
    assert!(
        paths
            .iter()
            .any(|p| p.starts_with("/app.") && p.ends_with(".wasm")),
        "page module: {paths:?}"
    );
    assert!(paths.contains("/style.css"), "static asset: {paths:?}");
}

/// ★ A SHARED `CARGO_TARGET_DIR` IS ONE DIRECTORY FOR EVERY PROJECT, and cargo names a binary
/// after its target — so the target's name is what keeps two projects' binaries apart.
#[test]
fn same_named_projects_build_under_different_identities() {
    let (a, b) = (tempdir().unwrap(), tempdir().unwrap());
    let (ia, ib) = (
        artifact_identity("app", a.path()),
        artifact_identity("app", b.path()),
    );
    assert_ne!(
        ia, ib,
        "two projects called `app` must never write one cargo binary"
    );
    assert!(
        ia.starts_with("app-") && ib.starts_with("app-"),
        "the name stays readable: {ia} {ib}"
    );
}

#[test]
fn a_project_keeps_one_identity_however_its_directory_is_spelled() {
    let dir = tempdir().unwrap();
    assert_eq!(
        artifact_identity("app", dir.path()),
        artifact_identity("app", &dir.path().join(".")),
        "one project, one cargo binary — or every rebuild would accrete a new one"
    );
}

/// The artifact is what cargo SAYS it produced for this target — not a path assumed from a
/// directory layout that `CARGO_TARGET_DIR`, `--target` or a cargo config can move.
#[test]
fn the_artifact_is_read_from_cargos_own_report() {
    let messages = concat!(
        r#"{"reason":"compiler-artifact","target":{"name":"logicaffeine_data","kind":["lib"]},"filenames":["/t/debug/deps/liblogicaffeine_data-1.rlib"],"executable":null}"#,
        "\n",
        r#"{"reason":"compiler-message","message":{"rendered":"warning"}}"#,
        "\n",
        r#"{"reason":"compiler-artifact","target":{"name":"app-00ff","kind":["bin"]},"filenames":["/t/debug/app-00ff"],"executable":"/t/debug/app-00ff"}"#,
        "\n",
        r#"{"reason":"build-finished","success":true}"#,
        "\n",
    );
    assert_eq!(
        built_artifact(messages, "app-00ff", false),
        Some(PathBuf::from("/t/debug/app-00ff"))
    );
    assert_eq!(
        built_artifact(messages, "app-1234", false),
        None,
        "another project's binary is not ours"
    );

    let lib = r#"{"reason":"compiler-artifact","target":{"name":"app_00ff","kind":["cdylib"]},"filenames":["/t/debug/libapp_00ff.so"],"executable":null}"#;
    assert_eq!(
        built_artifact(lib, "app-00ff", true),
        Some(PathBuf::from("/t/debug/libapp_00ff.so"))
    );
}

#[test]
fn requires_components_reject_toml_structure() {
    assert!(requires_component_is_safe("itoa"));
    assert!(requires_component_is_safe("1.0.11"));
    assert!(requires_component_is_safe(">=1.2, <2.0"));
    assert!(requires_component_is_safe("wasm-bindgen"));
    assert!(!requires_component_is_safe("evil\"\n[patch.crates-io]"));
    assert!(!requires_component_is_safe("x = { path"));
    assert!(!requires_component_is_safe(""));
}

#[test]
fn classify_missing_package_is_dependency_resolution() {
    let stderr = "    Updating crates.io index\n\
                      error: no matching package named `nonexistent-xyz` found\n\
                      location searched: registry `crates-io`\n\
                      required by package `demo v0.1.0`\n";
    assert_eq!(
        classify_cargo_failure(stderr),
        CargoFailureKind::DependencyResolution
    );
}

#[test]
fn classify_version_selection_is_dependency_resolution() {
    let stderr = "error: failed to select a version for the requirement `itoa = \"^99\"`\n\
                      candidate versions found which didn't match: 1.0.11, 1.0.10\n\
                      location searched: crates.io index\n";
    assert_eq!(
        classify_cargo_failure(stderr),
        CargoFailureKind::DependencyResolution
    );
}

#[test]
fn classify_network_fetch_is_dependency_resolution() {
    let stderr = "error: failed to get `itoa` as a dependency of package `demo v0.1.0`\n\
                      Caused by:\n  failed to fetch `https://github.com/rust-lang/crates.io-index`\n";
    assert_eq!(
        classify_cargo_failure(stderr),
        CargoFailureKind::DependencyResolution
    );
}

#[test]
fn classify_rustc_error_is_generated_code() {
    let stderr = "   Compiling demo v0.1.0\n\
                      error[E0425]: cannot find value `undefined_x` in this scope\n\
                        --> src/main.rs:10:5\n\
                      error: could not compile `demo` (bin \"demo\") due to 1 previous error\n";
    assert_eq!(
        classify_cargo_failure(stderr),
        CargoFailureKind::GeneratedCode
    );
}

#[test]
fn classify_unknown_output_defaults_to_generated_code() {
    assert_eq!(
        classify_cargo_failure("something exploded"),
        CargoFailureKind::GeneratedCode
    );
}

#[test]
fn find_project_root_finds_largo_toml() {
    let temp = tempdir().unwrap();
    let sub = temp.path().join("a/b/c");
    fs::create_dir_all(&sub).unwrap();
    fs::write(temp.path().join("Largo.toml"), "[package]\nname=\"test\"\n").unwrap();

    let found = find_project_root(&sub);
    assert!(found.is_some());
    assert_eq!(found.unwrap(), temp.path());
}

#[test]
fn find_project_root_returns_none_if_not_found() {
    let temp = tempdir().unwrap();
    let found = find_project_root(temp.path());
    assert!(found.is_none());
}

#[test]
fn strip_main_wrapper_extracts_body() {
    let code = r#"use logicaffeine_data::*;

fn add(a: i64, b: i64) -> i64 {
    a + b
}

fn main() {
    let x = add(1, 2);
    println!("{}", x);
}"#;
    let result = strip_main_wrapper(code);
    assert!(result.contains("fn add(a: i64, b: i64) -> i64"));
    assert!(result.contains("let x = add(1, 2);"));
    assert!(result.contains("println!(\"{}\", x);"));
    assert!(!result.contains("fn main()"));
}

#[test]
fn strip_main_wrapper_preserves_imports() {
    let code = "use logicaffeine_data::*;\nuse logicaffeine_system::*;\n\nfn main() {\n    println!(\"hello\");\n}\n";
    let result = strip_main_wrapper(code);
    assert!(result.contains("use logicaffeine_data::*;"));
    assert!(result.contains("use logicaffeine_system::*;"));
    assert!(result.contains("println!(\"hello\");"));
    assert!(!result.contains("fn main()"));
}

#[test]
fn strip_main_wrapper_no_main_returns_unchanged() {
    let code = "fn add(a: i64, b: i64) -> i64 { a + b }";
    let result = strip_main_wrapper(code);
    assert_eq!(result, code);
}

#[test]
fn strip_main_wrapper_dedents_body() {
    let code = "fn main() {\n    let x = 1;\n    let y = 2;\n}\n";
    let result = strip_main_wrapper(code);
    // Body lines should be dedented by 4 spaces
    assert!(result.contains("let x = 1;"));
    assert!(result.contains("let y = 2;"));
    // Should not have leading 4-space indent
    for line in result.lines() {
        if line.contains("let x") || line.contains("let y") {
            assert!(
                !line.starts_with("    "),
                "Line should be dedented: {}",
                line
            );
        }
    }
}
