//! E2E Test Harness
//!
//! Provides utilities for compiling LOGOS source and running the generated Rust.
//! Also provides test helpers for parsing and snapshots.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
pub use logicaffeine_compile::compile::compile_to_rust;

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

fn get_shared_target_dir() -> &'static PathBuf {
    SHARED_TARGET_DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join("logos_e2e_cache");
        std::fs::create_dir_all(&dir).expect("Failed to create shared target dir");
        dir
    })
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

/// Compile LOGOS source to a binary without running it.
/// Returns the path to the compiled binary.
pub fn compile_logos(source: &str) -> CompileResult {
    // 1. Compile LOGOS to Rust
    let rust_code = match compile_to_rust(source) {
        Ok(code) => code,
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
"#,
        pkg_name,
        workspace_root.display(),
        workspace_root.display()
    );

    std::fs::create_dir_all(project_dir.join("src")).unwrap();
    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml).unwrap();
    std::fs::write(project_dir.join("src/main.rs"), &rust_code).unwrap();

    // 4. Build (but don't run) - use shared target dir for caching
    let output = Command::new("cargo")
        .args(["build", "--quiet"])
        .current_dir(project_dir)
        .env("CARGO_TARGET_DIR", get_shared_target_dir())
        .output()
        .expect("cargo build");

    let binary_path = get_shared_target_dir().join(format!("debug/{}", pkg_name));

    CompileResult {
        binary_path,
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        success: output.status.success(),
        rust_code,
        _temp_dir: temp_dir,
    }
}

/// Compile LOGOS source and run the generated Rust, returning result.
pub fn run_logos(source: &str) -> E2EResult {
    // 1. Compile LOGOS to Rust
    let rust_code = match compile_to_rust(source) {
        Ok(code) => code,
        Err(e) => {
            return E2EResult {
                stdout: String::new(),
                stderr: format!("LOGOS compile error: {:?}", e),
                success: false,
                rust_code: String::new(),
            };
        }
    };

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
"#,
        pkg_name,
        workspace_root.display(),
        workspace_root.display()
    );

    std::fs::create_dir_all(project_dir.join("src")).unwrap();
    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml).unwrap();
    std::fs::write(project_dir.join("src/main.rs"), &rust_code).unwrap();

    // 4. Run - use shared target dir for caching
    let output = Command::new("cargo")
        .args(["run", "--quiet"])
        .current_dir(project_dir)
        .env("CARGO_TARGET_DIR", get_shared_target_dir())
        .output()
        .expect("cargo run");

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
