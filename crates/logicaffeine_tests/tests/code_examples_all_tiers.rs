//! Every shipped Studio Code example runs IDENTICALLY on every execution tier it offers.
//!
//! The in-crate `example_health` lock drives each example through the Studio's own interpreter. This
//! file holds the other tiers to the same bar: the bytecode VM prints the SAME exact output, and the
//! generated Rust (the Studio's 🦀 Compile view, what `largo build` ships) BUILDS with rustc and its
//! binary prints the SAME exact output. `NativeOnlyCompiles` examples (real OS networking) must build.
//!
//! All examples are compiled as the binaries of ONE generated cargo package, so a single `cargo build`
//! shares the dependency graph and builds every example in parallel.

#![cfg(feature = "web-tests")]

use std::path::{Path, PathBuf};
use std::process::Command;

use logicaffeine_web::ui::examples::{ExampleSpec, Expected, ALL_CODE_EXAMPLES};

/// The examples with a byte-exact expected output, and that output.
fn exact_examples() -> Vec<(&'static ExampleSpec, &'static str)> {
    ALL_CODE_EXAMPLES
        .iter()
        .filter_map(|spec| match spec.expected {
            Expected::OutputExact(out) => Some((spec, out.trim_end_matches('\n'))),
            _ => None,
        })
        .collect()
}

fn report(tier: &str, failures: Vec<String>, total: usize) {
    assert!(
        failures.is_empty(),
        "{} of {total} Studio code examples diverge on the {tier}:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn only_networking_examples_are_native_only() {
    // `NativeOnlyCompiles` is the escape hatch for programs that need real OS sockets. Anything else
    // runs in the browser: a CRDT, a hash, a lattice cipher all work in the Studio or are bugs.
    let native_only: Vec<&str> = ALL_CODE_EXAMPLES
        .iter()
        .filter(|s| matches!(s.expected, Expected::NativeOnlyCompiles))
        .map(|s| s.vfs_path)
        .collect();
    assert_eq!(
        native_only,
        [
            "/examples/code/networking/server.logos",
            "/examples/code/networking/client.logos"
        ],
        "only the real-socket networking examples may skip the browser"
    );
}

#[test]
fn no_example_tells_the_reader_it_cannot_run_here() {
    // An example that runs everywhere must not carry a stale "build it to run it" apology.
    let apologizing: Vec<&str> = exact_examples()
        .into_iter()
        .filter(|(spec, _)| {
            let s = spec.source.to_lowercase();
            s.contains("needs the compiled runtime")
                || s.contains("build with `largo build`")
                || s.contains("needs native kernels")
        })
        .map(|(spec, _)| spec.vfs_path)
        .collect();
    assert!(
        apologizing.is_empty(),
        "examples that say they cannot run in the Studio: {apologizing:?}"
    );
}

/// The VM outcome for `src`, driven by whichever VM driver the program needs (plain, scheduler, or
/// the local-network inbox) — never the tree-walker.
fn vm_outcome(src: &str) -> logicaffeine_compile::compile::RunOutcome {
    use logicaffeine_compile::compile::{
        run_strict_vm, vm_outcome_concurrent, vm_outcome_net, RunOutcome,
    };
    let strict = run_strict_vm(src);
    match strict.engine_ran {
        Some(_) => RunOutcome {
            output: strict.output,
            error: strict.error,
        },
        None => {
            let why = strict.error.clone().unwrap_or_default();
            if why.contains("tasks/channels") {
                vm_outcome_concurrent(src)
            } else if why.contains("async runtime") {
                vm_outcome_net(src)
            } else {
                RunOutcome {
                    output: String::new(),
                    error: strict.error,
                }
            }
        }
    }
}

#[test]
fn every_exact_example_prints_the_same_on_the_vm() {
    let examples = exact_examples();
    let failures = examples
        .iter()
        .filter_map(|(spec, expected)| {
            let r = vm_outcome(spec.source);
            (r.error.is_some() || r.output != *expected).then(|| {
                format!(
                    "{}: error {:?}\n--- expected ---\n{expected}\n--- VM ---\n{}",
                    spec.vfs_path, r.error, r.output
                )
            })
        })
        .collect();
    report("bytecode VM", failures, examples.len());
}

// ─── The compiled tier ───────────────────────────────────────────────────────

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn bin_name(vfs_path: &str) -> String {
    let stem = vfs_path
        .trim_start_matches("/examples/code/")
        .trim_end_matches(".logos");
    format!("ex_{}", stem.replace(['/', '-', '.'], "_"))
}

fn dep_line(dep: &logicaffeine_compile::CrateDependency) -> String {
    if dep.features.is_empty() {
        format!("{} = \"{}\"\n", dep.name, dep.version)
    } else {
        let feats: Vec<String> = dep.features.iter().map(|f| format!("\"{f}\"")).collect();
        format!(
            "{} = {{ version = \"{}\", features = [{}] }}\n",
            dep.name,
            dep.version,
            feats.join(", ")
        )
    }
}

struct Built {
    /// Examples whose LOGOS → Rust step failed, with the reason.
    refused: Vec<String>,
    /// The cargo build's stderr when it failed.
    build_error: Option<String>,
    /// bin name → built binary path.
    bins: std::collections::BTreeMap<String, PathBuf>,
}

/// Generate Rust for every compiled-tier example and build them all as the bins of one package.
fn build_all(specs: &[&ExampleSpec]) -> Built {
    let root = workspace_root();
    let project =
        std::env::temp_dir().join(format!("logos_studio_examples_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(project.join("src/bin")).expect("project dir");
    std::fs::write(project.join("src/lib.rs"), "").expect("lib.rs");

    let mut refused = Vec::new();
    let mut deps = std::collections::BTreeSet::new();
    let mut names = Vec::new();
    for spec in specs {
        let compile = if spec.source.contains("Launch a task") {
            logicaffeine_compile::compile::compile_program_full_deterministic
        } else {
            logicaffeine_compile::compile::compile_program_full
        };
        match compile(spec.source) {
            Ok(out) => {
                deps.extend(out.dependencies.iter().map(dep_line));
                let name = bin_name(spec.vfs_path);
                std::fs::write(
                    project.join("src/bin").join(format!("{name}.rs")),
                    &out.rust_code,
                )
                .expect("write bin");
                names.push(name);
            }
            Err(e) => refused.push(format!("{}: LOGOS → Rust refused: {e:?}", spec.vfs_path)),
        }
    }

    let manifest = format!(
        "[package]\nname = \"logos_studio_examples\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [dependencies]\n\
         logicaffeine-data = {{ path = \"{root}/crates/logicaffeine_data\" }}\n\
         logicaffeine-system = {{ path = \"{root}/crates/logicaffeine_system\", features = [\"full\"] }}\n\
         tokio = {{ version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }}\n\
         serde = {{ version = \"1\", features = [\"derive\"] }}\n\
         rayon = \"1\"\nmemchr = \"2\"\n{}",
        deps.into_iter().collect::<String>(),
        root = root.display(),
    );
    std::fs::write(project.join("Cargo.toml"), manifest).expect("Cargo.toml");
    std::fs::copy(root.join("Cargo.lock"), project.join("Cargo.lock")).expect("seed Cargo.lock");

    let target = std::env::temp_dir().join("logos_studio_examples_target");
    let out = Command::new("cargo")
        .args(["build", "--quiet", "--offline", "--bins", "--keep-going"])
        .current_dir(&project)
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .expect("cargo runs");
    let build_error =
        (!out.status.success()).then(|| String::from_utf8_lossy(&out.stderr).to_string());
    let bins = names
        .into_iter()
        .map(|n| {
            let p = target.join("debug").join(&n);
            (n, p)
        })
        .collect();
    Built {
        refused,
        build_error,
        bins,
    }
}

/// The first rustc error of each failing bin, attributed to its example.
fn build_failures(stderr: &str, specs: &[&ExampleSpec]) -> Vec<String> {
    let mut by_example: std::collections::BTreeMap<&str, Vec<String>> = Default::default();
    let mut current_error: Option<String> = None;
    for line in stderr.lines() {
        if line.starts_with("warning") || line.starts_with("error: could not compile") {
            current_error = None;
            continue;
        }
        if line.starts_with("error") {
            current_error = Some(line.to_string());
        }
        if let Some(loc) = line.trim_start().strip_prefix("--> ") {
            if let (Some(err), Some(spec)) = (
                &current_error,
                specs
                    .iter()
                    .find(|s| loc.contains(&format!("src/bin/{}.rs", bin_name(s.vfs_path)))),
            ) {
                let errs = by_example.entry(spec.vfs_path).or_default();
                if errs.len() < 4 {
                    errs.push(format!("{err}  ({})", loc.trim()));
                }
                current_error = None;
            }
        }
    }
    if by_example.is_empty() {
        return vec![format!("the generated package does not build:\n{stderr}")];
    }
    by_example
        .into_iter()
        .map(|(p, errs)| {
            format!(
                "{p}: the generated Rust does not build:\n  {}",
                errs.join("\n  ")
            )
        })
        .collect()
}

#[test]
fn every_code_example_builds_and_runs_identically_when_compiled() {
    let specs: Vec<&ExampleSpec> = ALL_CODE_EXAMPLES
        .iter()
        .filter(|s| {
            matches!(
                s.expected,
                Expected::OutputExact(_)
                    | Expected::NativeOnlyCompiles
                    | Expected::OutputContains(_)
            )
        })
        .collect();
    let built = build_all(&specs);
    let mut failures = built.refused.clone();
    if let Some(stderr) = &built.build_error {
        failures.extend(build_failures(stderr, &specs));
        report("compiled (Rust) tier", failures, specs.len());
        return;
    }
    for (spec, expected) in exact_examples() {
        let bin = &built.bins[&bin_name(spec.vfs_path)];
        let out = Command::new(bin)
            .env("RUST_MIN_STACK", "268435456")
            .output()
            .expect("the example binary runs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stdout = stdout.trim_end_matches('\n');
        if !out.status.success() || stdout != expected {
            failures.push(format!(
                "{}: status {:?}, stderr {:?}\n--- expected ---\n{expected}\n--- compiled ---\n{stdout}",
                spec.vfs_path,
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    report("compiled (Rust) tier", failures, specs.len());
}
