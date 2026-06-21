//! E2E Test Harness
//!
//! Provides utilities for compiling LOGOS source and running the generated Rust.
//! Also provides test helpers for parsing and snapshots.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
pub use logicaffeine_compile::compile::{compile_to_rust, compile_program_full};

// ============================================================
// Parse Helper - replaces the logos::parse! macro
// ============================================================

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::ast::{LogicExpr, NounPhrase, Term, ThematicRole};
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::analysis::TypeRegistry;
use logicaffeine_language::{Lexer, Parser};
use logicaffeine_language::view::{ExprView, Resolve};

/// Parse a sentence and return its ExprView representation.
/// This replaces the `parse!` macro with a proper function using tier crates.
pub fn parse_to_view(input: &str) -> ExprView<'static> {
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let world_state: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let expr_arena: &'static Arena<LogicExpr> = Box::leak(Box::new(Arena::new()));
    let term_arena: &'static Arena<Term> = Box::leak(Box::new(Arena::new()));
    let np_arena: &'static Arena<NounPhrase> = Box::leak(Box::new(Arena::new()));
    let sym_arena: &'static Arena<Symbol> = Box::leak(Box::new(Arena::new()));
    let role_arena: &'static Arena<(ThematicRole, Term)> = Box::leak(Box::new(Arena::new()));
    let pp_arena: &'static Arena<&'static LogicExpr> = Box::leak(Box::new(Arena::new()));

    let ctx = AstContext::new(
        expr_arena,
        term_arena,
        np_arena,
        sym_arena,
        role_arena,
        pp_arena,
    );

    let mut lexer = Lexer::new(input, interner);
    let tokens = lexer.tokenize();

    let type_registry = TypeRegistry::default();
    let mut parser = Parser::new(tokens, world_state, interner, ctx, type_registry);

    let ast = parser.parse().unwrap();
    ast.resolve(interner)
}

/// Macro for convenient parsing in tests - delegates to parse_to_view
#[macro_export]
macro_rules! parse {
    ($input:expr) => {
        $crate::common::parse_to_view($input)
    };
}

// ============================================================
// Snapshot Testing
// ============================================================

/// Assert that actual output matches a stored snapshot file.
/// If UPDATE_SNAPSHOTS=1 is set, updates the snapshot instead.
#[macro_export]
macro_rules! assert_snapshot {
    ($name:expr, $actual:expr) => {{
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR not set");
        let snapshot_dir = std::path::Path::new(&manifest_dir)
            .join("tests")
            .join("snapshots");
        let snapshot_path = snapshot_dir.join(format!("{}.txt", $name));

        if !snapshot_dir.exists() {
            std::fs::create_dir_all(&snapshot_dir).expect("Failed to create snapshot dir");
        }

        let actual_str = $actual.trim();
        let force_update = std::env::var("UPDATE_SNAPSHOTS").is_ok();

        if force_update || !snapshot_path.exists() {
            std::fs::write(&snapshot_path, actual_str).expect("Failed to write snapshot");
            println!("Snapshot created/updated: {:?}", snapshot_path);
        } else {
            let expected = std::fs::read_to_string(&snapshot_path)
                .expect("Failed to read snapshot");
            let expected_str = expected.trim();

            if actual_str != expected_str {
                panic!(
                    "\nSnapshot Mismatch: {}\n\nExpected:\n{}\n\nActual:\n{}\n\n\
                    Run `UPDATE_SNAPSHOTS=1 cargo test` to update.\n",
                    $name, expected_str, actual_str
                );
            }
        }
    }};
}

static SHARED_TARGET_DIR: OnceLock<PathBuf> = OnceLock::new();
static COMPILE_COUNTER: AtomicU64 = AtomicU64::new(0);
static RUN_ID: OnceLock<u64> = OnceLock::new();

fn get_run_id() -> u64 {
    *RUN_ID.get_or_init(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
    })
}

/// Shard count for the e2e cargo cache under nextest. Keep in sync with the
/// `e2e-subprocess` group's max-threads in .config/nextest.toml: group slots are
/// 0..max-threads, so concurrently running tests always land in distinct shards
/// and never contend on Cargo's build-dir lock.
const E2E_TARGET_SHARDS: u64 = 12;

fn get_shared_target_dir() -> &'static PathBuf {
    SHARED_TARGET_DIR.get_or_init(|| {
        let shard = std::env::var("NEXTEST_TEST_GROUP_SLOT")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .or_else(|| {
                std::env::var("NEXTEST_TEST_GLOBAL_SLOT")
                    .ok()
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(|n| n % E2E_TARGET_SHARDS)
            });
        let dir = match shard {
            Some(s) => std::env::temp_dir().join(format!("logos_e2e_cache_{s}")),
            None => std::env::temp_dir().join("logos_e2e_cache"),
        };
        std::fs::create_dir_all(&dir).expect("Failed to create shared target dir");
        // Safety valve: per-project artifacts are pruned after every test
        // (see prune_project_artifacts), so the cache holds only shared
        // dependency builds and stays a few GB. Anything that slips through
        // (interrupted tests, binaries built before the pruning landed) is
        // collected here BY AGE — generated-project artifacts older than an
        // hour cannot belong to a live test, while a whole-directory wipe
        // would delete dependency rmeta out from under CONCURRENT builds in
        // the same shard (global-slot tests share shards modulo the count).
        let stale = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        for sub in ["debug", "debug/deps", "debug/incremental", "debug/.fingerprint"] {
            let Ok(rd) = std::fs::read_dir(dir.join(sub)) else { continue };
            for e in rd.flatten() {
                let name = e.file_name();
                let n = name.to_string_lossy();
                let generated = n.starts_with("logos_e2e_")
                    || n.starts_with("logos_clink_")
                    || n.starts_with("logos_run_")
                    || n.starts_with("liblogos_e2e_")
                    || n.starts_with("liblogos_clink_");
                if !generated {
                    continue;
                }
                let old_enough = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .map(|t| t < stale)
                    .unwrap_or(false);
                if old_enough {
                    let path = e.path();
                    let _ = if path.is_dir() {
                        std::fs::remove_dir_all(&path)
                    } else {
                        std::fs::remove_file(&path)
                    };
                }
            }
        }
        dir
    })
}

/// Delete one generated project's artifacts from the shared cache,
/// KEEPING the third-party dependency builds (the entire speed win).
/// Every project has a unique name, so without this the cache accretes a
/// binary + deps/incremental/fingerprint entries per test forever —
/// hundreds of GB per shard across repeated suite runs.
fn prune_project_artifacts(target: &std::path::Path, pkg_name: &str) {
    let stem = pkg_name.replace('-', "_");
    let debug = target.join("debug");
    let _ = std::fs::remove_file(debug.join(pkg_name));
    let _ = std::fs::remove_file(debug.join(format!("{pkg_name}.d")));
    let _ = std::fs::remove_file(debug.join(format!("lib{stem}.a")));
    let _ = std::fs::remove_file(debug.join(format!("lib{stem}.so")));
    for sub in ["deps", "incremental", ".fingerprint", "build"] {
        let Ok(rd) = std::fs::read_dir(debug.join(sub)) else { continue };
        for e in rd.flatten() {
            let name = e.file_name();
            let n = name.to_string_lossy();
            if n.starts_with(&format!("{stem}-"))
                || n.starts_with(&format!("lib{stem}-"))
                || n.starts_with(&format!("{pkg_name}-"))
            {
                let path = e.path();
                let _ = if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
            }
        }
    }
}

/// Seed a generated project with the workspace's Cargo.lock so dependency
/// resolution is pinned to the exact versions the workspace tests against.
/// Without this every project re-resolves from the registry cache, and a newly
/// published transitive version (e.g. `time` 0.3.48 vs `rcgen` 0.11.3) can
/// break every e2e build mid-run.
fn seed_lockfile(workspace_root: &std::path::Path, project_dir: &std::path::Path) {
    std::fs::copy(
        workspace_root.join("Cargo.lock"),
        project_dir.join("Cargo.lock"),
    )
    .expect("Failed to seed Cargo.lock into generated project");
}

pub struct E2EResult {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub rust_code: String,
}

pub struct CompileResult {
    pub binary_path: std::path::PathBuf,
    pub stderr: String,
    pub success: bool,
    pub rust_code: String,
    pub _temp_dir: tempfile::TempDir,  // Keep alive to prevent cleanup
}

impl Drop for CompileResult {
    fn drop(&mut self) {
        // The test has finished with the binary — drop its artifacts from
        // the shared cache (dependency builds stay).
        if let (Some(dir), Some(name)) =
            (self.binary_path.parent(), self.binary_path.file_name())
        {
            if let Some(target) = dir.parent() {
                prune_project_artifacts(target, &name.to_string_lossy());
            }
        }
    }
}

/// Format user-declared dependencies as Cargo.toml lines.
fn format_user_deps(deps: &[logicaffeine_compile::CrateDependency]) -> String {
    let mut out = String::new();
    for dep in deps {
        if dep.features.is_empty() {
            out.push_str(&format!("{} = \"{}\"\n", dep.name, dep.version));
        } else {
            let feats = dep.features.iter()
                .map(|f| format!("\"{}\"", f))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!(
                "{} = {{ version = \"{}\", features = [{}] }}\n",
                dep.name, dep.version, feats
            ));
        }
    }
    out
}

/// Compile LOGOS source to a binary without running it.
/// Returns the path to the compiled binary.
pub fn compile_logos(source: &str) -> CompileResult {
    // 1. Compile LOGOS to Rust (with dependency extraction)
    let compile_output = match compile_program_full(source) {
        Ok(out) => out,
        Err(e) => {
            return CompileResult {
                binary_path: std::path::PathBuf::new(),
                stderr: format!("LOGOS compile error: {:?}", e),
                success: false,
                rust_code: String::new(),
                _temp_dir: tempfile::tempdir().unwrap(),
            };
        }
    };
    let rust_code = compile_output.rust_code;
    let user_deps = format_user_deps(&compile_output.dependencies);

    // 2. Create temp project
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let project_dir = temp_dir.path();

    // 3. Write Cargo.toml with unique package name (includes run ID to avoid stale binary issues)
    let pkg_id = COMPILE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pkg_name = format!("logos_e2e_{}_{}", get_run_id(), pkg_id);
    // CARGO_MANIFEST_DIR points to crates/logicaffeine_tests, go up twice to get workspace root
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap());
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let cargo_toml = format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[dependencies]
logicaffeine-data = {{ path = "{}/crates/logicaffeine_data" }}
logicaffeine-system = {{ path = "{}/crates/logicaffeine_system", features = ["full"] }}
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
serde = {{ version = "1", features = ["derive"] }}
rayon = "1"
{}"#,
        pkg_name,
        workspace_root.display(),
        workspace_root.display(),
        user_deps
    );

    std::fs::create_dir_all(project_dir.join("src")).unwrap();
    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml).unwrap();
    std::fs::write(project_dir.join("src/main.rs"), &rust_code).unwrap();
    seed_lockfile(workspace_root, project_dir);

    // 4. Build (but don't run) - use shared target dir for caching
    //    Try --offline first, fall back to online if cache isn't warm.
    let output = Command::new("cargo")
        .args(["build", "--quiet", "--offline"])
        .current_dir(project_dir)
        .env("CARGO_TARGET_DIR", get_shared_target_dir())
        .output()
        .expect("cargo build");
    let output = if !output.status.success() && String::from_utf8_lossy(&output.stderr).contains("--offline") {
        Command::new("cargo")
            .args(["build", "--quiet"])
            .current_dir(project_dir)
            .env("CARGO_TARGET_DIR", get_shared_target_dir())
            .output()
            .expect("cargo build")
    } else {
        output
    };

    let binary_path = get_shared_target_dir().join(format!("debug/{}", pkg_name));

    CompileResult {
        binary_path,
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        success: output.status.success(),
        rust_code,
        _temp_dir: temp_dir,
    }
}

/// Compile LOGOS source to optimized LLVM IR (release profile) and return
/// the .ll text. The bounds-hint pass/fail check greps `_logos_main` for
/// `panic_bounds_check` in this output.
#[allow(dead_code)]
pub fn compile_logos_llvm_ir(source: &str) -> String {
    let compile_output = compile_program_full(source)
        .unwrap_or_else(|e| panic!("LOGOS compile error: {:?}\n\nSource:\n{}", e, source));
    let rust_code = compile_output.rust_code;
    let user_deps = format_user_deps(&compile_output.dependencies);

    let temp_dir = tempfile::tempdir().expect("temp dir");
    let project_dir = temp_dir.path();

    let pkg_id = COMPILE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pkg_name = format!("logos_ir_{}_{}", get_run_id(), pkg_id);
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap());
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let cargo_toml = format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[profile.release]
codegen-units = 1

[dependencies]
logicaffeine-data = {{ path = "{}/crates/logicaffeine_data" }}
logicaffeine-system = {{ path = "{}/crates/logicaffeine_system", features = ["full"] }}
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
serde = {{ version = "1", features = ["derive"] }}
rayon = "1"
{}"#,
        pkg_name,
        workspace_root.display(),
        workspace_root.display(),
        user_deps
    );

    std::fs::create_dir_all(project_dir.join("src")).unwrap();
    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml).unwrap();
    std::fs::write(project_dir.join("src/main.rs"), &rust_code).unwrap();
    seed_lockfile(workspace_root, project_dir);

    let output = Command::new("cargo")
        .args(["rustc", "--release", "--quiet", "--offline", "--", "--emit=llvm-ir"])
        .current_dir(project_dir)
        .env("CARGO_TARGET_DIR", get_shared_target_dir())
        .output()
        .expect("cargo rustc --emit=llvm-ir");
    let output = if !output.status.success() && String::from_utf8_lossy(&output.stderr).contains("--offline") {
        Command::new("cargo")
            .args(["rustc", "--release", "--quiet", "--", "--emit=llvm-ir"])
            .current_dir(project_dir)
            .env("CARGO_TARGET_DIR", get_shared_target_dir())
            .output()
            .expect("cargo rustc --emit=llvm-ir")
    } else {
        output
    };
    assert!(
        output.status.success(),
        "release IR build failed.\nstderr: {}\n\nGenerated Rust:\n{}",
        String::from_utf8_lossy(&output.stderr),
        rust_code
    );

    let deps_dir = get_shared_target_dir().join("release/deps");
    let prefix = pkg_name.replace('-', "_");
    let ll_path = std::fs::read_dir(&deps_dir)
        .expect("read deps dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| {
            p.extension().map_or(false, |x| x == "ll")
                && p.file_stem()
                    .and_then(|s| s.to_str())
                    .map_or(false, |s| s.starts_with(&prefix))
        })
        .unwrap_or_else(|| panic!("no .ll emitted for {} in {}", prefix, deps_dir.display()));
    let ir = std::fs::read_to_string(&ll_path).expect("read .ll");
    let _ = std::fs::remove_file(&ll_path);
    prune_project_artifacts(get_shared_target_dir(), &pkg_name);
    ir
}

/// Extract one function's definition body from LLVM IR text by symbol
/// substring (e.g. "_logos_main").
#[allow(dead_code)]
pub fn llvm_ir_function<'a>(ir: &'a str, symbol: &str) -> &'a str {
    let def_start = ir
        .match_indices("\ndefine ")
        .find(|(i, _)| {
            let line_end = ir[*i + 1..].find('\n').map(|j| i + 1 + j).unwrap_or(ir.len());
            ir[*i..line_end].contains(symbol)
        })
        .map(|(i, _)| i + 1)
        .unwrap_or_else(|| panic!("no define line containing `{}` in IR", symbol));
    let body_end = ir[def_start..]
        .find("\n}")
        .map(|j| def_start + j + 2)
        .unwrap_or(ir.len());
    &ir[def_start..body_end]
}

/// Compile LOGOS source and run the generated Rust, returning result.
pub fn run_logos(source: &str) -> E2EResult {
    // 1. Compile LOGOS to Rust (with dependency extraction)
    let compile_output = match compile_program_full(source) {
        Ok(out) => out,
        Err(e) => {
            return E2EResult {
                stdout: String::new(),
                stderr: format!("LOGOS compile error: {:?}", e),
                success: false,
                rust_code: String::new(),
            };
        }
    };
    let rust_code = compile_output.rust_code;
    let user_deps = format_user_deps(&compile_output.dependencies);

    // 2. Create temp project
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let project_dir = temp_dir.path();

    // 3. Write Cargo.toml with unique package name (includes run ID to avoid stale binary issues)
    let pkg_id = COMPILE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pkg_name = format!("logos_e2e_{}_{}", get_run_id(), pkg_id);
    // CARGO_MANIFEST_DIR points to crates/logicaffeine_tests, go up twice to get workspace root
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap());
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let cargo_toml = format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[dependencies]
logicaffeine-data = {{ path = "{}/crates/logicaffeine_data" }}
logicaffeine-system = {{ path = "{}/crates/logicaffeine_system", features = ["full"] }}
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
serde = {{ version = "1", features = ["derive"] }}
rayon = "1"
{}"#,
        pkg_name,
        workspace_root.display(),
        workspace_root.display(),
        user_deps
    );

    std::fs::create_dir_all(project_dir.join("src")).unwrap();
    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml).unwrap();
    std::fs::write(project_dir.join("src/main.rs"), &rust_code).unwrap();
    seed_lockfile(workspace_root, project_dir);

    // 4. Run - use shared target dir for caching
    //    Try --offline first to avoid transient crates.io failures, fall back to online.
    //    Set RUST_MIN_STACK=64MB so deeply-recursive PE/cogen programs
    //    don't overflow on structural AST walks.
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--offline"])
        .current_dir(project_dir)
        .env("CARGO_TARGET_DIR", get_shared_target_dir())
        .env("RUST_MIN_STACK", "268435456")
        .output()
        .expect("cargo run");
    // Fall back to online if --offline failed (e.g. first run, cache not warm)
    let output = if !output.status.success() && String::from_utf8_lossy(&output.stderr).contains("--offline") {
        Command::new("cargo")
            .args(["run", "--quiet"])
            .current_dir(project_dir)
            .env("CARGO_TARGET_DIR", get_shared_target_dir())
            .env("RUST_MIN_STACK", "268435456")
            .output()
            .expect("cargo run")
    } else {
        output
    };

    prune_project_artifacts(get_shared_target_dir(), &pkg_name);

    E2EResult {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        success: output.status.success(),
        rust_code,
    }
}

/// Assert that LOGOS code runs and produces expected output.
pub fn assert_output(source: &str, expected: &str) {
    let result = run_logos(source);
    assert!(
        result.success,
        "Code should run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source,
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains(expected),
        "Expected '{}' in output.\nGot: '{}'\n\nSource:\n{}\n\nGenerated Rust:\n{}",
        expected,
        result.stdout.trim(),
        source,
        result.rust_code
    );
}

/// Assert that LOGOS code runs and produces exactly the expected output (trimmed).
#[allow(dead_code)]
pub fn assert_exact_output(source: &str, expected: &str) {
    let result = run_logos(source);
    assert!(
        result.success,
        "Code should run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source,
        result.rust_code,
        result.stderr
    );
    assert_eq!(
        result.stdout.trim(),
        expected,
        "\nSource:\n{}\n\nGenerated Rust:\n{}",
        source,
        result.rust_code
    );
}

/// Assert that LOGOS code runs and produces exactly the expected lines (trimmed, line-by-line).
#[allow(dead_code)]
pub fn assert_output_lines(source: &str, expected_lines: &[&str]) {
    let result = run_logos(source);
    assert!(
        result.success,
        "Code should run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source,
        result.rust_code,
        result.stderr
    );
    let actual_lines: Vec<&str> = result.stdout.trim().lines().collect();
    assert_eq!(
        actual_lines.len(),
        expected_lines.len(),
        "Line count mismatch.\nExpected {} lines: {:?}\nGot {} lines: {:?}\n\nSource:\n{}\n\nGenerated Rust:\n{}",
        expected_lines.len(),
        expected_lines,
        actual_lines.len(),
        actual_lines,
        source,
        result.rust_code
    );
    for (i, (actual, expected)) in actual_lines.iter().zip(expected_lines.iter()).enumerate() {
        assert_eq!(
            actual.trim(),
            *expected,
            "Line {} mismatch.\nExpected: '{}'\nGot:      '{}'\n\nFull output:\n{}\n\nSource:\n{}\n\nGenerated Rust:\n{}",
            i + 1,
            expected,
            actual.trim(),
            result.stdout.trim(),
            source,
            result.rust_code
        );
    }
}

/// Assert that LOGOS code runs and output contains all specified substrings (order-independent).
/// Use for non-deterministic output (e.g., concurrent tasks).
#[allow(dead_code)]
pub fn assert_output_contains_all(source: &str, parts: &[&str]) {
    let result = run_logos(source);
    assert!(
        result.success,
        "Code should run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source,
        result.rust_code,
        result.stderr
    );
    let output = result.stdout.trim();
    for part in parts {
        assert!(
            output.contains(part),
            "Expected '{}' in output.\nGot: '{}'\n\nSource:\n{}\n\nGenerated Rust:\n{}",
            part,
            output,
            source,
            result.rust_code
        );
    }
}

/// Assert that LOGOS code runs successfully (no output check).
#[allow(dead_code)]
pub fn assert_runs(source: &str) {
    let result = run_logos(source);
    assert!(
        result.success,
        "Code should run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source,
        result.rust_code,
        result.stderr
    );
}

/// Assert that LOGOS code panics at runtime (e.g., debug_assert! failure).
#[allow(dead_code)]
pub fn assert_panics(source: &str, expected_msg: &str) {
    let result = run_logos(source);
    assert!(
        !result.success,
        "Code should panic but succeeded.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstdout: {}",
        source,
        result.rust_code,
        result.stdout
    );
    assert!(
        result.stderr.contains(expected_msg),
        "Expected '{}' in panic message.\nGot stderr:\n{}\n\nSource:\n{}\n\nGenerated Rust:\n{}",
        expected_msg,
        result.stderr,
        source,
        result.rust_code
    );
}

// ============================================================
// C Codegen E2E — compile LOGOS to C, build with gcc, run
// ============================================================

/// Compile LOGOS source to C, build with gcc, run, and check exact output.
#[allow(dead_code)]
pub fn assert_c_output(source: &str, expected: &str) {
    use logicaffeine_compile::compile::compile_to_c;

    let c_code = compile_to_c(source).unwrap_or_else(|e| {
        panic!("LOGOS→C compile error: {:?}\n\nSource:\n{}", e, source);
    });

    let temp_dir = tempfile::tempdir().expect("temp dir");
    let c_path = temp_dir.path().join("main.c");
    let bin_path = temp_dir.path().join("main");

    std::fs::write(&c_path, &c_code).expect("write C file");

    let compile = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin_path)
        .arg(&c_path)
        .args(["-lm"])
        .output()
        .expect("run gcc");

    assert!(
        compile.status.success(),
        "gcc should compile successfully.\nstderr: {}\n\nGenerated C:\n{}",
        String::from_utf8_lossy(&compile.stderr),
        c_code
    );

    let run = std::process::Command::new(&bin_path)
        .output()
        .expect("run binary");

    assert!(
        run.status.success(),
        "C binary should run successfully.\nstderr: {}\n\nGenerated C:\n{}",
        String::from_utf8_lossy(&run.stderr),
        c_code
    );

    let stdout = String::from_utf8_lossy(&run.stdout);
    assert_eq!(
        stdout.trim(),
        expected,
        "\nSource:\n{}\n\nGenerated C:\n{}",
        source,
        c_code
    );
}

// ============================================================
// C ABI Linkage Tests — compile Rust staticlib, link with C
// ============================================================

pub struct CLinkResult {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub rust_code: String,
    pub c_code: String,
}

/// Compile LOGOS source to a staticlib, compile C code, link, and run.
/// The generated Rust is built as a staticlib, then a C file is compiled
/// and linked against it to verify the ABI works end-to-end.
#[allow(dead_code)]
pub fn compile_and_link_c(source: &str, c_code: &str) -> CLinkResult {
    // 1. Compile LOGOS to Rust
    let compile_output = match compile_program_full(source) {
        Ok(out) => out,
        Err(e) => {
            return CLinkResult {
                stdout: String::new(),
                stderr: format!("LOGOS compile error: {:?}", e),
                success: false,
                rust_code: String::new(),
                c_code: c_code.to_string(),
            };
        }
    };
    let rust_code = compile_output.rust_code;
    let user_deps = format_user_deps(&compile_output.dependencies);

    // 2. Create temp project configured as staticlib
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let project_dir = temp_dir.path();

    let pkg_id = COMPILE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pkg_name = format!("logos_clink_{}_{}", get_run_id(), pkg_id);
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap());
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();

    let cargo_toml = format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["staticlib"]

[dependencies]
logicaffeine-data = {{ path = "{}/crates/logicaffeine_data" }}
logicaffeine-system = {{ path = "{}/crates/logicaffeine_system", features = ["full"] }}
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
serde = {{ version = "1", features = ["derive"] }}
rayon = "1"
{}"#,
        pkg_name,
        workspace_root.display(),
        workspace_root.display(),
        user_deps
    );

    // Strip fn main() from the generated Rust — staticlib doesn't need it
    let lib_code = strip_main_function(&rust_code);

    std::fs::create_dir_all(project_dir.join("src")).unwrap();
    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml).unwrap();
    std::fs::write(project_dir.join("src/lib.rs"), &lib_code).unwrap();
    seed_lockfile(workspace_root, project_dir);

    // 3. Build the staticlib — try offline first, fall back to online
    let build_output = Command::new("cargo")
        .args(["build", "--quiet", "--offline"])
        .current_dir(project_dir)
        .env("CARGO_TARGET_DIR", get_shared_target_dir())
        .output()
        .expect("cargo build");
    let build_output = if !build_output.status.success() && String::from_utf8_lossy(&build_output.stderr).contains("--offline") {
        Command::new("cargo")
            .args(["build", "--quiet"])
            .current_dir(project_dir)
            .env("CARGO_TARGET_DIR", get_shared_target_dir())
            .output()
            .expect("cargo build")
    } else {
        build_output
    };

    if !build_output.status.success() {
        return CLinkResult {
            stdout: String::new(),
            stderr: format!(
                "Rust staticlib build failed:\n{}\n\nLib code:\n{}",
                String::from_utf8_lossy(&build_output.stderr),
                lib_code
            ),
            success: false,
            rust_code: rust_code.clone(),
            c_code: c_code.to_string(),
        };
    }

    let lib_path = get_shared_target_dir().join(format!("debug/lib{}.a", pkg_name.replace('-', "_")));

    // 4. Write C code
    let c_file = project_dir.join("test.c");
    std::fs::write(&c_file, c_code).unwrap();

    // 5. Compile and link C code against the staticlib
    let binary_path = project_dir.join("test_binary");
    let mut cc_args = vec![
        "-Wall",
        c_file.to_str().unwrap(),
        lib_path.to_str().unwrap(),
        "-o",
        binary_path.to_str().unwrap(),
    ];

    #[cfg(target_os = "macos")]
    {
        cc_args.extend_from_slice(&[
            "-framework", "Security",
            "-framework", "CoreFoundation",
            "-framework", "SystemConfiguration",
            "-lSystem", "-lresolv", "-liconv",
        ]);
    }

    #[cfg(target_os = "linux")]
    {
        cc_args.extend_from_slice(&["-ldl", "-lpthread", "-lm"]);
    }

    let cc_output = Command::new("cc")
        .args(&cc_args)
        .output()
        .expect("cc command");

    if !cc_output.status.success() {
        return CLinkResult {
            stdout: String::new(),
            stderr: format!(
                "C linking failed:\n{}\n\nC code:\n{}\n\nLib path: {:?}",
                String::from_utf8_lossy(&cc_output.stderr),
                c_code,
                lib_path
            ),
            success: false,
            rust_code: rust_code.clone(),
            c_code: c_code.to_string(),
        };
    }

    // 6. Run the linked binary
    let run_output = Command::new(&binary_path)
        .output()
        .expect("run test binary");

    prune_project_artifacts(get_shared_target_dir(), &pkg_name);

    CLinkResult {
        stdout: String::from_utf8_lossy(&run_output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&run_output.stderr).to_string(),
        success: run_output.status.success(),
        rust_code,
        c_code: c_code.to_string(),
    }
}

/// Strip the `fn main() { ... }` block from generated Rust code.
/// Used when building as a staticlib (no main needed).
fn strip_main_function(code: &str) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let mut result = Vec::new();
    let mut skip = false;
    let mut depth = 0i32;

    for line in &lines {
        if line.trim_start().starts_with("fn main()") {
            skip = true;
            depth = 0;
        }
        if skip {
            for ch in line.chars() {
                if ch == '{' { depth += 1; }
                if ch == '}' { depth -= 1; }
            }
            if depth <= 0 && line.contains('}') {
                skip = false;
            }
            continue;
        }
        result.push(*line);
    }

    result.join("\n")
}

// ============================================================
// Interpreter Tests (no Rust compilation)
// ============================================================

pub struct InterpreterTestResult {
    pub output: String,
    pub error: String,
    pub success: bool,
}

/// Run LOGOS source through the interpreter (no Rust compilation).
/// Uses futures::executor::block_on to run the async interpret_for_ui.
pub fn run_interpreter(source: &str) -> InterpreterTestResult {
    use logicaffeine_compile::interpret_for_ui;
    use futures::executor::block_on;

    let result = block_on(interpret_for_ui(source));
    let success = result.error.is_none();

    InterpreterTestResult {
        output: result.lines.join("\n"),
        error: result.error.unwrap_or_default(),
        success,
    }
}

/// Assert that LOGOS code produces exactly the expected output via the interpreter.
/// No Rust compilation — runs directly through interpret_for_ui.
#[allow(dead_code)]
pub fn assert_interpreter_output(source: &str, expected: &str) {
    let result = run_interpreter(source);
    assert!(
        result.success,
        "Interpreter should succeed.\nSource:\n{}\n\nError: {}",
        source,
        result.error
    );
    assert_eq!(
        result.output.trim(),
        expected,
        "\nSource:\n{}",
        source
    );
}

/// DIFFERENTIAL-CORRECTNESS GATE: the compiled binary and the interpreter MUST
/// agree on a program's outcome (both succeed with identical output, or both
/// fail). This is the CI invariant that catches the whole class of bug where a
/// codegen optimization silently changes a program's meaning relative to the
/// reference engine — e.g. an unsound buffer swap, or an aliased SetIndex
/// double-borrow. The interpreter (VM + tree-walker, cross-checked by the debug
/// shadow oracle) is the reference; codegen must match it.
#[allow(dead_code)]
pub fn assert_compiled_equals_interpreted(source: &str) {
    let compiled = run_logos(source);
    let interp = run_interpreter(source);
    assert_eq!(
        compiled.success, interp.success,
        "DIFFERENTIAL MISMATCH: compiled.success={} but interpreter.success={}\n\
         Source:\n{}\n\ncompiled stderr:\n{}\n\ninterpreter error:\n{}\n\nGenerated Rust:\n{}",
        compiled.success, interp.success, source, compiled.stderr, interp.error, compiled.rust_code
    );
    if compiled.success {
        assert_eq!(
            compiled.stdout.trim(),
            interp.output.trim(),
            "DIFFERENTIAL MISMATCH: compiled and interpreter produced different output\n\
             Source:\n{}\n\nGenerated Rust:\n{}",
            source, compiled.rust_code
        );
    }
}

/// Differential gate + an expected value, so a regression in BOTH engines (they
/// agree but on the wrong answer) is still caught.
#[allow(dead_code)]
pub fn assert_compiled_equals_interpreted_eq(source: &str, expected: &str) {
    assert_compiled_equals_interpreted(source);
    let interp = run_interpreter(source);
    assert_eq!(
        interp.output.trim(),
        expected,
        "Both engines agree but not on the expected value.\nSource:\n{}",
        source
    );
}

/// Assert that LOGOS code produces exactly the expected lines via the interpreter.
#[allow(dead_code)]
pub fn assert_interpreter_output_lines(source: &str, expected_lines: &[&str]) {
    let result = run_interpreter(source);
    assert!(
        result.success,
        "Interpreter should succeed.\nSource:\n{}\n\nError: {}",
        source,
        result.error
    );
    let actual_lines: Vec<&str> = result.output.trim().lines().collect();
    assert_eq!(
        actual_lines.len(),
        expected_lines.len(),
        "Line count mismatch.\nExpected {} lines: {:?}\nGot {} lines: {:?}\n\nSource:\n{}",
        expected_lines.len(),
        expected_lines,
        actual_lines.len(),
        actual_lines,
        source
    );
    for (i, (actual, expected)) in actual_lines.iter().zip(expected_lines.iter()).enumerate() {
        assert_eq!(
            actual.trim(),
            *expected,
            "Line {} mismatch.\nExpected: '{}'\nGot:      '{}'\n\nFull output:\n{}\n\nSource:\n{}",
            i + 1,
            expected,
            actual.trim(),
            result.output.trim(),
            source
        );
    }
}

/// Assert that LOGOS code runs successfully via the interpreter (no output check).
#[allow(dead_code)]
pub fn assert_interpreter_runs(source: &str) {
    let result = run_interpreter(source);
    assert!(
        result.success,
        "Interpreter should succeed.\nSource:\n{}\n\nError: {}",
        source,
        result.error
    );
}

/// Assert that LOGOS source fails to compile (parse error, analysis error, etc.)
/// and that the error message contains the expected substring.
#[allow(dead_code)]
pub fn assert_compile_fails(source: &str, expected_error: &str) {
    let result = run_logos(source);
    assert!(
        !result.success,
        "Expected compilation to fail, but it succeeded.\nSource:\n{}\n\nstdout: {}",
        source,
        result.stdout
    );
    let all_errors = format!("{}\n{}", result.stderr, result.stdout);
    assert!(
        all_errors.contains(expected_error),
        "Expected error containing '{}' but got:\n{}\n\nSource:\n{}",
        expected_error,
        all_errors,
        source
    );
}

/// Assert that LOGOS source fails during interpretation
/// and that the error message contains the expected substring.
#[allow(dead_code)]
pub fn assert_interpreter_fails(source: &str, expected_error: &str) {
    let result = run_interpreter(source);
    assert!(
        !result.success,
        "Expected interpreter to fail, but it succeeded.\nSource:\n{}\n\nOutput: {}",
        source,
        result.output
    );
    assert!(
        result.error.contains(expected_error),
        "Expected error containing '{}' but got:\n{}\n\nSource:\n{}",
        expected_error,
        result.error,
        source
    );
}

/// Assert that LOGOS code produces output containing the expected substring via the interpreter.
#[allow(dead_code)]
pub fn assert_interpreter_output_contains(source: &str, expected: &str) {
    let result = run_interpreter(source);
    assert!(
        result.success,
        "Interpreter should succeed.\nSource:\n{}\n\nError: {}",
        source,
        result.error
    );
    assert!(
        result.output.trim().contains(expected),
        "Expected '{}' in output.\nGot: '{}'\n\nSource:\n{}",
        expected,
        result.output.trim(),
        source
    );
}
