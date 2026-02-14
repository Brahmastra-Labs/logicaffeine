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

    // 3. Build the staticlib
    let build_output = Command::new("cargo")
        .args(["build", "--quiet"])
        .current_dir(project_dir)
        .env("CARGO_TARGET_DIR", get_shared_target_dir())
        .output()
        .expect("cargo build");

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
