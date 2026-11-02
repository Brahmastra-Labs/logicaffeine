//! What a published crate ships is exactly what its consumers build with, and everything its
//! production code embeds ships with it.
//!
//! A crate on crates.io is a tarball of its own directory and nothing else: no workspace root, no
//! sibling crates, no `docs/`, no `benchmarks/`. Two failure classes follow, and both are silent in
//! this monorepo because every sibling is always on disk next to every crate:
//!
//! - **An embed that reaches outside the crate.** `include_str!("../../../../benchmarks/…")`
//!   compiles here and fails in every downstream build, for every feature set, because the macro
//!   expands whether or not the function is ever called. The crate is then unbuildable from the
//!   registry, and so is everything that depends on it.
//! - **A package that ships what nobody builds with.** Without an allowlist, cargo packages every
//!   tracked file: test fixtures, fuzz corpora, examples. That is dead weight at best, and past the
//!   registry's 10 MiB upload cap it is a refused release.
//!
//! So every published crate (a workspace member whose manifest has no `publish` key, exactly the
//! set `publish.yml` derives) declares an `include` allowlist drawn from a fixed vocabulary, and
//! every `include_str!`/`include_bytes!`/`include!`/`include_dir!`/`#[path]` reachable from its
//! production module graph must resolve to a file that allowlist ships. Test-only code (items under
//! `#[cfg(test)]` or `#[test]`, and files reached only through a `#[cfg(test)]` module declaration)
//! is exempt: cargo never compiles it for a consumer.
//!
//! The module graph is walked with `syn` from each crate's roots, following `mod` declarations and
//! `#[path]` attributes by the reference's resolution rules, and every macro's token stream is
//! scanned so an embed nested inside `vec![…]` or `concat!(…)` is still seen. This lock is pure
//! parsing, no cargo. `scripts/check-packaging.sh` is its complement: it builds every packaged
//! tarball for real.

#[path = "common/module_graph.rs"]
mod module_graph;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use module_graph::{normalize, Embed};

/// Every pattern a published crate may ship. Anything a consumer needs to build, read the docs, run
/// the build script or know the license it holds the code under; nothing else. `include` is gitignore syntax, where a pattern with no slash
/// matches at ANY depth (`README.md` would ship every README under the crate), so root files are
/// anchored with a leading `/`.
const VOCABULARY: &[&str] = &[
    "src/**",
    "/build.rs",
    "/README.md",
    "/CHANGELOG.md",
    "/LICENSE.md",
    "assets/**",
    "stencils/**",
    "templates/**",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

struct Package {
    /// Repo-relative directory, e.g. `crates/logicaffeine_kernel`.
    rel: String,
    dir: PathBuf,
    manifest: toml::Table,
}

impl Package {
    fn published(&self) -> bool {
        match self.manifest.get("package").and_then(|p| p.get("publish")) {
            None => true,
            Some(toml::Value::Boolean(b)) => *b,
            Some(_) => true,
        }
    }

    fn name(&self) -> String {
        self.manifest["package"]["name"]
            .as_str()
            .expect("package.name")
            .to_string()
    }

    fn include(&self) -> Option<Vec<String>> {
        let list = self.manifest.get("package")?.get("include")?.as_array()?;
        Some(
            list.iter()
                .map(|v| v.as_str().expect("include entries are strings").to_string())
                .collect(),
        )
    }

    /// The patterns that decide what ships: the declared allowlist, or the whole vocabulary while
    /// the allowlist is missing (so an escape is still reported on its own terms).
    fn shipping_patterns(&self) -> Vec<String> {
        self.include()
            .unwrap_or_else(|| VOCABULARY.iter().map(|s| s.to_string()).collect())
    }

    fn ships(&self, rel_in_crate: &str) -> bool {
        self.shipping_patterns()
            .iter()
            .any(|pat| match pat.strip_suffix("/**") {
                Some(dir) => rel_in_crate.starts_with(&format!("{dir}/")),
                None => rel_in_crate == pat.trim_start_matches('/'),
            })
    }
}

/// Every package directly under `crates/`, `apps/` and `scripts/`.
fn packages() -> Vec<Package> {
    let root = repo_root();
    let mut out = Vec::new();
    for parent in ["crates", "apps", "scripts"] {
        let Ok(entries) = std::fs::read_dir(root.join(parent)) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        dirs.sort();
        for dir in dirs {
            let manifest_path = dir.join("Cargo.toml");
            let Ok(text) = std::fs::read_to_string(&manifest_path) else {
                continue;
            };
            let manifest: toml::Table = text
                .parse()
                .unwrap_or_else(|e| panic!("{} does not parse: {e}", manifest_path.display()));
            if manifest.get("package").is_none() {
                continue;
            }
            let rel = format!("{parent}/{}", dir.file_name().unwrap().to_string_lossy());
            out.push(Package { rel, dir, manifest });
        }
    }
    out
}

fn published() -> Vec<Package> {
    let all: Vec<Package> = packages().into_iter().filter(Package::published).collect();
    assert!(
        all.len() >= 10,
        "found only {} published packages — the discovery walk broke, not the workspace",
        all.len()
    );
    all
}

/// Every embed reachable from a crate's roots, with its test-only status.
fn embeds_of(pkg: &Package) -> Vec<Embed> {
    module_graph::walk(&pkg.dir).embeds
}

fn line_of(file: &Path, literal: &str) -> usize {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let needle = literal
        .split('"')
        .find(|s| !s.is_empty() && s.contains('/'))
        .unwrap_or(literal);
    text.lines()
        .position(|l| l.contains(needle))
        .map(|i| i + 1)
        .unwrap_or(0)
}

fn rel_to_repo(p: &Path) -> String {
    p.strip_prefix(repo_root())
        .unwrap_or(p)
        .display()
        .to_string()
}

// ─── the locks ─────────────────────────────────────────────────────────────────────────────────

#[test]
fn every_published_crate_declares_the_files_it_ships() {
    let missing: Vec<String> = published()
        .iter()
        .filter(|p| p.include().is_none())
        .map(|p| p.rel.clone())
        .collect();
    assert!(
        missing.is_empty(),
        "these published crates have no `include` allowlist, so cargo packages every tracked file in \
         them (tests, fixtures, fuzz corpora, examples): {missing:?}\n\
         Add `include = [...]` under [package], drawn from {VOCABULARY:?}."
    );
}

#[test]
fn a_published_crate_ships_only_what_a_consumer_builds_with() {
    let mut problems = Vec::new();
    for pkg in published() {
        let Some(include) = pkg.include() else {
            continue;
        };
        for pat in &include {
            if !VOCABULARY.contains(&pat.as_str()) {
                problems.push(format!(
                    "{}: `{pat}` is not in the shipping vocabulary {VOCABULARY:?}",
                    pkg.rel
                ));
                continue;
            }
            let exists = match pat.strip_suffix("/**") {
                Some(dir) => pkg.dir.join(dir).is_dir(),
                None => pkg.dir.join(pat.trim_start_matches('/')).is_file(),
            };
            if !exists {
                problems.push(format!("{}: `{pat}` names nothing on disk", pkg.rel));
            }
        }
        for required in ["src/**", "/README.md", "/LICENSE.md"] {
            if !include.iter().any(|p| p == required) {
                problems.push(format!("{}: `{required}` must ship", pkg.rel));
            }
        }
        for optional in ["/build.rs", "/CHANGELOG.md"] {
            let on_disk = pkg.dir.join(optional.trim_start_matches('/')).is_file();
            let listed = include.iter().any(|p| p == optional);
            if on_disk != listed {
                problems.push(format!(
                    "{}: `{optional}` is {} on disk but {} in `include`",
                    pkg.rel,
                    if on_disk { "present" } else { "absent" },
                    if listed { "listed" } else { "not listed" }
                ));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{} packaging allowlist problem(s):\n  {}",
        problems.len(),
        problems.join("\n  ")
    );
}

#[test]
fn production_code_embeds_only_files_its_crate_ships() {
    let mut problems = Vec::new();
    let mut seen = 0usize;
    for pkg in published() {
        for e in embeds_of(&pkg) {
            seen += 1;
            if e.test_only {
                continue;
            }
            let at = format!("{}:{}", rel_to_repo(&e.from), line_of(&e.from, &e.literal));
            match e.target.strip_prefix(&pkg.dir) {
                Err(_) => problems.push(format!(
                    "{at}: embeds `{}` → {} — OUTSIDE {}. The registry tarball has no such file, so the \
                     crate cannot build for anyone. Move the file into the crate that embeds it and make \
                     any other copy a generated golden.",
                    e.literal,
                    rel_to_repo(&e.target),
                    pkg.rel
                )),
                Ok(inside) => {
                    let inside = inside.to_string_lossy().replace('\\', "/");
                    if !pkg.ships(&inside) {
                        problems.push(format!(
                            "{at}: embeds `{}` → {inside}, which {}'s `include` does not ship",
                            e.literal, pkg.rel
                        ));
                    } else if !e.target.exists() {
                        problems.push(format!("{at}: embeds `{}` → {inside}, which does not exist", e.literal));
                    }
                }
            }
        }
    }
    assert!(
        seen >= 20,
        "found only {seen} embeds across the published crates — the walker broke"
    );
    assert!(
        problems.is_empty(),
        "{} production embed(s) a registry build cannot resolve:\n  {}",
        problems.len(),
        problems.join("\n  ")
    );
}

/// The walker must see what it claims to see, or the lock above passes on an empty set.
#[test]
fn the_walker_sees_test_gating_and_nested_embeds() {
    let dir = std::env::temp_dir().join(format!("packaging_lock_probe_{}", std::process::id()));
    let src = dir.join("src");
    std::fs::create_dir_all(src.join("gated")).unwrap();
    std::fs::write(
        src.join("lib.rs"),
        "#![doc = include_str!(\"../README.md\")]\n\
         pub mod plain;\n\
         #[cfg(test)]\n#[path = \"gated/controls.rs\"]\nmod controls;\n\
         #[cfg(test)]\nmod tests { const F: &str = include_str!(\"../../outside_test.txt\"); }\n\
         #[cfg(all(test, feature = \"x\"))]\nconst G: &str = include_str!(\"../../outside_all_test.txt\");\n\
         #[cfg(any(test, feature = \"x\"))]\nconst H: &str = include_str!(\"../../outside_any.txt\");\n",
    )
    .unwrap();
    std::fs::write(
        src.join("plain.rs"),
        "pub fn f() -> Vec<&'static str> { vec![include_str!(\"../../outside_nested.txt\")] }\n",
    )
    .unwrap();
    std::fs::write(src.join("gated/controls.rs"), "const C: &str = include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/tests/fixture.txt\"));\n").unwrap();
    let pkg = Package {
        rel: "probe".into(),
        dir: dir.clone(),
        manifest: toml::Table::new(),
    };

    let embeds = embeds_of(&pkg);
    let by_literal: BTreeMap<String, bool> = embeds
        .iter()
        .map(|e| (e.literal.clone(), e.test_only))
        .collect();
    std::fs::remove_dir_all(&dir).ok();

    assert_eq!(
        by_literal.get("../README.md"),
        Some(&false),
        "crate-doc include is production: {by_literal:?}"
    );
    assert_eq!(
        by_literal.get("../../outside_nested.txt"),
        Some(&false),
        "an embed nested in vec![] is found: {by_literal:?}"
    );
    assert_eq!(
        by_literal.get("../../outside_test.txt"),
        Some(&true),
        "#[cfg(test)] mod is test-only: {by_literal:?}"
    );
    assert_eq!(
        by_literal.get("../../outside_all_test.txt"),
        Some(&true),
        "cfg(all(test, ..)) is test-only: {by_literal:?}"
    );
    assert_eq!(
        by_literal.get("../../outside_any.txt"),
        Some(&false),
        "cfg(any(test, ..)) still compiles for consumers: {by_literal:?}"
    );
    let gated = embeds
        .iter()
        .find(|e| e.literal.contains("/tests/fixture.txt"))
        .expect("the #[path]-declared gated file is walked");
    assert!(
        gated.test_only,
        "a file reached only through `#[cfg(test)] mod` is test-only"
    );
    assert!(
        gated.target.ends_with("tests/fixture.txt"),
        "concat!(env!(CARGO_MANIFEST_DIR), ..) resolves from the crate root"
    );
}

#[test]
fn a_published_crate_depends_only_on_published_crates() {
    let all = packages();
    let root: toml::Table = std::fs::read_to_string(repo_root().join("Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let ws_deps = root
        .get("workspace")
        .and_then(|w| w.get("dependencies"))
        .and_then(|d| d.as_table())
        .cloned()
        .unwrap_or_default();
    let by_dir: BTreeMap<PathBuf, &Package> = all.iter().map(|p| (normalize(&p.dir), p)).collect();

    let resolve = |pkg: &Package, spec: &toml::Value| -> Option<(PathBuf, bool)> {
        let t = spec.as_table()?;
        if t.get("workspace").and_then(|w| w.as_bool()) == Some(true) {
            return None; // resolved by name below
        }
        let path = t.get("path")?.as_str()?;
        Some((normalize(&pkg.dir.join(path)), t.contains_key("version")))
    };

    let mut problems = Vec::new();
    for pkg in all.iter().filter(|p| p.published()) {
        let mut tables: Vec<(&str, &toml::Table)> = Vec::new();
        for key in ["dependencies", "build-dependencies", "dev-dependencies"] {
            if let Some(t) = pkg.manifest.get(key).and_then(|t| t.as_table()) {
                tables.push((key, t));
            }
        }
        if let Some(targets) = pkg.manifest.get("target").and_then(|t| t.as_table()) {
            for (_, tt) in targets {
                for key in ["dependencies", "build-dependencies", "dev-dependencies"] {
                    if let Some(t) = tt.get(key).and_then(|t| t.as_table()) {
                        tables.push((key, t));
                    }
                }
            }
        }
        for (kind, table) in tables {
            for (dep, spec) in table {
                let dep_name = spec.get("package").and_then(|p| p.as_str()).unwrap_or(dep);
                let (dir, versioned) =
                    if spec.get("workspace").and_then(|w| w.as_bool()) == Some(true) {
                        match ws_deps
                            .get(dep_name)
                            .and_then(|d| d.get("path"))
                            .and_then(|p| p.as_str())
                        {
                            Some(p) => (normalize(&repo_root().join(p)), true),
                            None => continue,
                        }
                    } else {
                        match resolve(pkg, spec) {
                            Some(r) => r,
                            None => continue,
                        }
                    };
                // cargo strips a path-only dev-dependency from the published manifest.
                if kind == "dev-dependencies" && !versioned {
                    continue;
                }
                if let Some(target) = by_dir.get(&dir) {
                    if !target.published() {
                        problems.push(format!(
                            "{} [{kind}] `{dep}` → {} is `publish = false`",
                            pkg.rel,
                            target.name()
                        ));
                    }
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "published crates that name unpublished ones (the registry cannot resolve them):\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn docs_rs_builds_only_features_that_exist() {
    let mut problems = Vec::new();
    for pkg in published() {
        let Some(features) = pkg
            .manifest
            .get("package")
            .and_then(|p| p.get("metadata"))
            .and_then(|m| m.get("docs"))
            .and_then(|d| d.get("rs"))
            .and_then(|r| r.get("features"))
            .and_then(|f| f.as_array())
        else {
            continue;
        };
        let declared: BTreeSet<String> = pkg
            .manifest
            .get("features")
            .and_then(|f| f.as_table())
            .map(|t| t.keys().cloned().collect())
            .unwrap_or_default();
        for f in features {
            let f = f.as_str().unwrap_or_default();
            if !declared.contains(f) {
                problems.push(format!(
                    "{}: docs.rs builds feature `{f}`, which [features] does not declare",
                    pkg.rel
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The release uploads only what the packaging check just built. `publish.yml` runs
/// `scripts/check-packaging.sh` before its `cargo publish` step and never passes `--no-verify` (the
/// flag that let unbuildable tarballs reach the registry), and per-push CI runs the same check, so a
/// packaging break surfaces on the pull request that causes it, not on docs.rs after a release.
#[test]
fn the_release_uploads_only_what_the_packaging_check_built() {
    let root = repo_root();
    let script = root.join("scripts/check-packaging.sh");
    assert!(script.is_file(), "scripts/check-packaging.sh is missing");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&script).unwrap().permissions().mode();
        assert!(
            mode & 0o111 != 0,
            "scripts/check-packaging.sh is not executable (mode {mode:o})"
        );
    }

    let publish = std::fs::read_to_string(root.join(".github/workflows/publish.yml"))
        .expect("read publish.yml");
    assert!(
        !publish.contains("--no-verify"),
        "publish.yml passes --no-verify — every tarball must be built before it uploads"
    );
    let check_at = publish
        .find("./scripts/check-packaging.sh")
        .expect("publish.yml must run ./scripts/check-packaging.sh");
    let publish_at = publish
        .find("cargo publish")
        .expect("publish.yml has no `cargo publish` step");
    assert!(
        check_at < publish_at,
        "publish.yml must run the packaging check BEFORE `cargo publish`"
    );

    let ci =
        std::fs::read_to_string(root.join(".github/workflows/test.yml")).expect("read test.yml");
    assert!(
        ci.contains("./scripts/check-packaging.sh"),
        "per-push CI (test.yml) must run ./scripts/check-packaging.sh"
    );

    // One rule decides the published set everywhere: a member whose manifest has no `publish` key.
    let rule = "p[\"publish\"] is None";
    assert!(
        publish.contains(rule),
        "publish.yml must derive the published set with `{rule}`"
    );
    let script_text = std::fs::read_to_string(&script).unwrap();
    assert!(
        script_text.contains("p[\"publish\"] is not None"),
        "check-packaging.sh must derive the published set by the same rule"
    );
}

/// The minimum Rust the workspace declares is the toolchain CI proves it on. `rust-version` is what a
/// downstream user trusts when pinning a toolchain; a value no job builds on is a claim, not a fact.
#[test]
fn the_declared_minimum_rust_is_the_toolchain_ci_checks_on() {
    let root = repo_root();
    let manifest: toml::Table = std::fs::read_to_string(root.join("Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let declared = manifest["workspace"]["package"]["rust-version"]
        .as_str()
        .expect("[workspace.package] rust-version")
        .to_string();

    let ci: workflow_yaml::Workflow =
        workflow_yaml::Workflow::read(&root.join(".github/workflows/test.yml"));
    let toolchain = ci
        .job_step_input("msrv", "toolchain")
        .expect("test.yml needs an `msrv` job whose rust-toolchain step names a `toolchain`");
    assert_eq!(
        toolchain, declared,
        "the msrv job builds on {toolchain} but the workspace declares rust-version {declared}"
    );
    assert!(
        ci.job_runs("msrv", "cargo check --locked"),
        "the msrv job must `cargo check --locked` the published crates"
    );

    for pkg in published() {
        let inherits = pkg
            .manifest
            .get("package")
            .and_then(|p| p.get("rust-version"))
            .and_then(|v| v.get("workspace"))
            .and_then(|w| w.as_bool());
        assert_eq!(
            inherits,
            Some(true),
            "{} must inherit `rust-version.workspace = true`, not declare its own",
            pkg.rel
        );
    }
}

/// Just enough YAML reading for one workflow file: a job's step inputs and run scripts, found by
/// indentation, so the lock needs no YAML dependency.
mod workflow_yaml {
    pub struct Workflow {
        text: String,
    }

    impl Workflow {
        pub fn read(path: &std::path::Path) -> Workflow {
            Workflow {
                text: std::fs::read_to_string(path)
                    .unwrap_or_else(|e| panic!("read {}: {e}", path.display())),
            }
        }

        /// The lines of `jobs.<job>`: from `  <job>:` to the next line indented two spaces or less.
        fn job(&self, job: &str) -> Vec<&str> {
            let header = format!("  {job}:");
            let mut lines = self.text.lines().skip_while(|l| *l != header);
            let Some(_) = lines.next() else {
                return Vec::new();
            };
            lines
                .take_while(|l| {
                    l.is_empty() || l.starts_with("   ") || l.trim_start().starts_with('#')
                })
                .collect()
        }

        pub fn job_step_input(&self, job: &str, input: &str) -> Option<String> {
            let key = format!("{input}:");
            self.job(job).iter().find_map(|l| {
                let t = l.trim();
                t.strip_prefix(&key)
                    .map(|v| v.trim().trim_matches('"').to_string())
            })
        }

        pub fn job_runs(&self, job: &str, command: &str) -> bool {
            self.job(job).iter().any(|l| l.contains(command))
        }
    }
}
