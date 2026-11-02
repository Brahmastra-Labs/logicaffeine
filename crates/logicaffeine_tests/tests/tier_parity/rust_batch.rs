//! The Rust backends for a batch of programs in one build.
//!
//! Every program's generated Rust becomes one binary of a single cargo package, so one `cargo build` shares
//! the dependency graph and builds them all in parallel; [`super::sweep`] builds the corpus a chunk at a time
//! and [`Built::clean`] deletes each chunk's binaries before the next. The package is built twice: under cargo's dev
//! profile, which is what `largo run` and `largo build` produce, and under the release profile `largo build
//! --release` ships. The release profile is read from the manifest the product itself writes
//! ([`logicaffeine_compile::compile::write_cargo_project`]), so it cannot drift from what ships. Two of its
//! keys only change how long the build takes, never what a program does (`lto`, `codegen-units`), and the
//! batch relaxes exactly those; [`RELEASE_KEYS`] classifies every key, and a key the product adds that is
//! not classified fails the build rather than being guessed at.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::Program;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Profile {
    Debug,
    Release,
}

/// Every key the shipped `[profile.release]` may carry: whether it can change what a program does, and the
/// value the batch build uses when it cannot.
pub const RELEASE_KEYS: &[(&str, Semantic)] = &[
    ("opt-level", Semantic::Kept),
    ("panic", Semantic::Kept),
    ("overflow-checks", Semantic::Kept),
    ("debug-assertions", Semantic::Kept),
    ("strip", Semantic::Kept),
    ("lto", Semantic::BuildTimeOnly("false")),
    ("codegen-units", Semantic::BuildTimeOnly("16")),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Semantic {
    /// The key can change behaviour, or costs nothing to keep: the batch uses the shipped value.
    Kept,
    /// The key changes only build time: the batch uses this value.
    BuildTimeOnly(&'static str),
}

/// The dev-profile keys the batch sets beyond cargo's defaults: debuginfo only changes the size of a binary,
/// never what it does, and a corpus-sized batch of debug binaries with full debuginfo does not fit on a disk.
pub const DEV_KEYS: &[(&str, &str)] = &[("debug", "0")];

/// What the batch build produced.
pub struct Built {
    /// Where cargo put the binaries.
    target: PathBuf,
    /// program id → its binary's name.
    bins: BTreeMap<String, String>,
    /// program id → why no Rust was generated for it.
    refused: BTreeMap<String, String>,
    /// The programs among `refused` the Rust pipeline's front end rejected (the others made it panic).
    rejected: BTreeSet<String>,
    /// (binary name, profile) → the first compiler error of that binary.
    failed: BTreeMap<(String, Profile), String>,
    /// (binary name, profile) → the built executable.
    executables: BTreeMap<(String, Profile), PathBuf>,
    /// A build that failed as a whole (the manifest, the dependency graph), per profile.
    broken: BTreeMap<Profile, String>,
}

impl Built {
    /// Whether the Rust pipeline's front end rejected `id`.
    pub fn rejected(&self, id: &str) -> bool {
        self.rejected.contains(id)
    }

    /// The executable of `id` under `profile`, or why there is none.
    pub fn binary(&self, id: &str, profile: Profile) -> Result<PathBuf, String> {
        if let Some(why) = self.refused.get(id) {
            return Err(why.clone());
        }
        let Some(bin) = self.bins.get(id) else {
            return Err("the program was not part of the batch build".to_string());
        };
        if let Some(why) = self.failed.get(&(bin.clone(), profile)) {
            return Err(format!("the emitted Rust does not build: {why}"));
        }
        if let Some(why) = self.broken.get(&profile) {
            return Err(format!("the batch build failed as a whole: {why}"));
        }
        self.executables
            .get(&(bin.clone(), profile))
            .cloned()
            .ok_or_else(|| "cargo reported no executable for the program".to_string())
    }
}

/// The shipped release profile, read from a project the product writes, with each key classified.
pub fn shipped_release_profile(scratch: &Path) -> Result<Vec<(String, String)>, String> {
    let dir = scratch.join("shipped-manifest");
    let _ = std::fs::remove_dir_all(&dir);
    logicaffeine_compile::compile::write_cargo_project("fn main() {}\n", &[], &dir)
        .map_err(|e| format!("{e:?}"))?;
    let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).map_err(|e| e.to_string())?;
    let mut in_release = false;
    let mut keys = Vec::new();
    for line in manifest.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_release = t == "[profile.release]";
            continue;
        }
        if !in_release || t.is_empty() || t.starts_with('#') {
            continue;
        }
        let (k, v) = t
            .split_once('=')
            .ok_or_else(|| format!("an unreadable release-profile line: {t}"))?;
        let (k, v) = (k.trim().to_string(), v.trim().to_string());
        let class = RELEASE_KEYS
            .iter()
            .find(|(name, _)| *name == k)
            .map(|(_, c)| *c)
            .ok_or_else(|| format!("the shipped release profile carries `{k}`, which RELEASE_KEYS does not classify"))?;
        keys.push(match class {
            Semantic::Kept => (k, v),
            Semantic::BuildTimeOnly(batch) => (k, batch.to_string()),
        });
    }
    let _ = std::fs::remove_dir_all(&dir);
    if keys.is_empty() {
        return Err("the shipped manifest has no [profile.release]".to_string());
    }
    Ok(keys)
}

/// The shipped `.cargo/config.toml` a generated project builds under (the target CPU).
fn shipped_cargo_config(scratch: &Path) -> Result<String, String> {
    let dir = scratch.join("shipped-config");
    let _ = std::fs::remove_dir_all(&dir);
    logicaffeine_compile::compile::write_cargo_project("fn main() {}\n", &[], &dir)
        .map_err(|e| format!("{e:?}"))?;
    let config =
        std::fs::read_to_string(dir.join(".cargo/config.toml")).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(config)
}

fn dep_line(dep: &logicaffeine_compile::CrateDependency) -> String {
    if dep.features.is_empty() {
        format!("{} = \"{}\"", dep.name, dep.version)
    } else {
        let feats: Vec<String> = dep.features.iter().map(|f| format!("\"{f}\"")).collect();
        format!(
            "{} = {{ version = \"{}\", features = [{}] }}",
            dep.name,
            dep.version,
            feats.join(", ")
        )
    }
}

/// Generate every program's Rust and build the package under both profiles, `jobs` rustc processes at a time.
pub fn build(programs: &[Program], jobs: usize) -> Result<Built, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let scratch = super::scratch_root();
    let project = scratch.join("rust-batch");
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(project.join("src/bin")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(project.join(".cargo")).map_err(|e| e.to_string())?;

    let mut bins = BTreeMap::new();
    let mut refused = BTreeMap::new();
    let mut rejected = BTreeSet::new();
    let mut deps: BTreeSet<String> = BTreeSet::new();
    let mut http = false;
    // A program's arguments reach its binary at run time, so programs with one source share one binary: the
    // first is compiled, and every later one takes its binary or its refusal.
    let mut first_of: BTreeMap<&str, &str> = BTreeMap::new();
    for (i, program) in programs.iter().enumerate() {
        if let Some(&first) = first_of.get(program.source.as_str()) {
            if let Some(name) = bins.get(first).cloned() {
                bins.insert(program.id.clone(), name);
            }
            if let Some(why) = refused.get(first).cloned() {
                refused.insert(program.id.clone(), why);
            }
            if rejected.contains(first) {
                rejected.insert(program.id.clone());
            }
            continue;
        }
        first_of.insert(program.source.as_str(), program.id.as_str());
        let generated = std::panic::catch_unwind(|| {
            logicaffeine_compile::compile::compile_program_full(&program.source)
        });
        match generated {
            Err(panic) => {
                refused.insert(
                    program.id.clone(),
                    format!("the Rust emitter panicked: {}", super::panic_text(&*panic)),
                );
            }
            Ok(Err(e)) => {
                refused.insert(
                    program.id.clone(),
                    format!("LOGOS → Rust refused: {e:?}")
                        .chars()
                        .take(240)
                        .collect(),
                );
                rejected.insert(program.id.clone());
            }
            Ok(Ok(out)) => {
                let name = format!("p{i:05}");
                std::fs::write(
                    project.join("src/bin").join(format!("{name}.rs")),
                    &out.rust_code,
                )
                .map_err(|e| e.to_string())?;
                deps.extend(out.dependencies.iter().map(dep_line));
                deps.extend(
                    logicaffeine_compile::compile::implied_crate_deps(
                        &out.rust_code,
                        &out.dependencies,
                    )
                    .into_iter()
                    .map(str::to_string),
                );
                http |= out.rust_code.contains("logicaffeine_system::http::");
                bins.insert(program.id.clone(), name);
            }
        }
    }

    let features = if http {
        "\"full\", \"http\""
    } else {
        "\"full\""
    };
    let profile: String = shipped_release_profile(&scratch)?
        .into_iter()
        .map(|(k, v)| format!("{k} = {v}\n"))
        .collect();
    let manifest = format!(
        "[package]\nname = \"logos_parity\"\nversion = \"0.1.0\"\nedition = \"2021\"\nautobins = true\n\n\
         [dependencies]\n\
         logicaffeine-data = {{ path = \"{root}/crates/logicaffeine_data\" }}\n\
         logicaffeine-system = {{ path = \"{root}/crates/logicaffeine_system\", features = [{features}] }}\n\
         tokio = {{ version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }}\n\
         {deps}\n\n\
         [target.'cfg(target_os = \"linux\")'.dependencies]\n\
         logicaffeine-system = {{ path = \"{root}/crates/logicaffeine_system\", features = [\"full\", \"io-uring\"] }}\n\n\
         [profile.dev]\n{dev}\n\n[profile.release]\n{profile}",
        root = root.display(),
        deps = deps.into_iter().collect::<Vec<_>>().join("\n"),
        dev = DEV_KEYS.iter().map(|(k, v)| format!("{k} = {v}")).collect::<Vec<_>>().join("\n"),
    );
    std::fs::write(project.join("Cargo.toml"), manifest).map_err(|e| e.to_string())?;
    std::fs::write(
        project.join(".cargo/config.toml"),
        shipped_cargo_config(&scratch)?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::copy(root.join("Cargo.lock"), project.join("Cargo.lock"))
        .map_err(|e| e.to_string())?;

    let target = std::env::var_os("LOGOS_PARITY_RUST_TARGET")
        .map(PathBuf::from)
        .unwrap_or_else(|| scratch.join("rust-batch-target"));
    let mut failed = BTreeMap::new();
    let mut executables = BTreeMap::new();
    let mut broken = BTreeMap::new();
    for profile in [Profile::Debug, Profile::Release] {
        let mut cmd = Command::new("cargo");
        cmd.args([
            "build",
            "--offline",
            "--bins",
            "--keep-going",
            "--message-format=json",
            "-j",
            &jobs.max(1).to_string(),
        ]);
        if profile == Profile::Release {
            cmd.arg("--release");
        }
        let out = cmd
            .current_dir(&project)
            .env("CARGO_TARGET_DIR", &target)
            .env("CARGO_INCREMENTAL", "0")
            .output()
            .map_err(|e| format!("cargo did not start: {e}"))?;
        let mut attributed = false;
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let name = msg["target"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            match msg["reason"].as_str() {
                Some("compiler-artifact") => {
                    if let Some(exe) = msg["executable"].as_str() {
                        executables.insert((name, profile), PathBuf::from(exe));
                    }
                }
                Some("compiler-message")
                    if msg["message"]["level"] == "error" && name.starts_with('p') =>
                {
                    attributed = true;
                    failed.entry((name, profile)).or_insert_with(|| {
                        msg["message"]["message"]
                            .as_str()
                            .unwrap_or_default()
                            .chars()
                            .take(240)
                            .collect()
                    });
                }
                _ => {}
            }
        }
        if !out.status.success() && !attributed {
            broken.insert(
                profile,
                super::first_line(&String::from_utf8_lossy(&out.stderr)),
            );
        }
    }
    Ok(Built {
        target,
        bins,
        refused,
        rejected,
        failed,
        executables,
        broken,
    })
}

/// The project the sweep's batches were built in, and their target when it is this process's own (no
/// `LOGOS_PARITY_RUST_TARGET` shares one): released when the sweep is done with them, so a sweep leaves no build
/// behind.
pub fn release() {
    let scratch = super::scratch_root();
    let _ = std::fs::remove_dir_all(scratch.join("rust-batch"));
    if std::env::var_os("LOGOS_PARITY_RUST_TARGET").is_none() {
        let _ = std::fs::remove_dir_all(scratch.join("rust-batch-target"));
    }
}

impl Built {
    /// Delete every binary this batch built, and the per-binary artifacts beside it; the shared dependency
    /// graph stays for the next batch.
    pub fn clean(&self) {
        let is_bin = |name: &str| {
            let stem = name.split(['-', '.']).next().unwrap_or("");
            stem.len() == 6
                && stem.starts_with('p')
                && stem[1..].bytes().all(|b| b.is_ascii_digit())
        };
        for profile in ["debug", "release"] {
            for dir in [
                self.target.join(profile),
                self.target.join(profile).join("deps"),
            ] {
                for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                    if entry.file_name().to_str().is_some_and(is_bin) {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
    }
}
