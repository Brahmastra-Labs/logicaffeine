mod common;
use common::*;

// ─────────────────────────────────────────────────────────
// Tier 1: Parser + Codegen — verify parsing and code generation
// ─────────────────────────────────────────────────────────

#[test]
fn requires_single_crate_compiles() {
    let source = r#"## Requires
    The "itoa" crate version "1".

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn main()"), "Generated:\n{}", rust);
}

#[test]
fn requires_crate_emits_no_rust() {
    let source = r#"## Requires
    The "itoa" crate version "1".

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(!rust.contains("itoa"), "Require should not emit Rust code. Generated:\n{}", rust);
}

#[test]
fn requires_multiple_crates_compile() {
    let source = r#"## Requires
    The "itoa" crate version "1".
    The "ryu" crate version "1".

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn main()"), "Generated:\n{}", rust);
}

#[test]
fn requires_with_features_compiles() {
    let source = r#"## Requires
    The "serde" crate version "1.0" with features "derive".

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn main()"), "Generated:\n{}", rust);
}

#[test]
fn requires_with_description_compiles() {
    let source = r#"## Requires
    The "itoa" crate version "1" for integer formatting.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn main()"), "Generated:\n{}", rust);
}

#[test]
fn requires_with_features_and_description_compiles() {
    let source = r#"## Requires
    The "reqwest" crate version "0.11" with features "json" and "blocking" for HTTP requests.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn main()"), "Generated:\n{}", rust);
}

#[test]
fn requires_coexists_with_typedef_and_functions() {
    let source = r#"## Requires
    The "itoa" crate version "1".

## A Point has:
    an x: Int.
    a y: Int.

## To double (n: Int) -> Int:
    Let result be n * 2.
    Return result.

## Main
Let p be a new Point with x 1 and y 2.
Show p's x.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn main()"), "Generated:\n{}", rust);
    assert!(rust.contains("struct Point"), "Has struct:\n{}", rust);
    assert!(rust.contains("fn double"), "Has function:\n{}", rust);
}

// ─────────────────────────────────────────────────────────
// Tier 2: Error handling — malformed dependency declarations
// ─────────────────────────────────────────────────────────

#[test]
fn requires_missing_crate_name_is_error() {
    let source = r#"## Requires
    The crate version "1.0".

## Main
Show 42.
"#;
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail: missing crate name string literal");
}

#[test]
fn requires_missing_version_string_is_error() {
    let source = r#"## Requires
    The "serde" crate version.

## Main
Show 42.
"#;
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail: missing version string literal");
}

#[test]
fn requires_missing_crate_keyword_is_error() {
    let source = r#"## Requires
    The "serde" version "1.0".

## Main
Show 42.
"#;
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail: missing 'crate' keyword");
}

// ─────────────────────────────────────────────────────────
// Tier 3: Dependency extraction — verify CompileOutput
// ─────────────────────────────────────────────────────────

#[test]
fn requires_extracts_single_dependency() {
    let source = r#"## Requires
    The "itoa" crate version "1".

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    assert_eq!(output.dependencies.len(), 1);
    assert_eq!(output.dependencies[0].name, "itoa");
    assert_eq!(output.dependencies[0].version, "1");
    assert!(output.dependencies[0].features.is_empty());
}

#[test]
fn requires_extracts_multiple_dependencies() {
    let source = r#"## Requires
    The "itoa" crate version "1".
    The "ryu" crate version "1".

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    assert_eq!(output.dependencies.len(), 2);
    assert_eq!(output.dependencies[0].name, "itoa");
    assert_eq!(output.dependencies[1].name, "ryu");
}

#[test]
fn requires_extracts_features() {
    let source = r#"## Requires
    The "reqwest" crate version "0.11" with features "json" and "blocking".

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    assert_eq!(output.dependencies.len(), 1);
    assert_eq!(output.dependencies[0].name, "reqwest");
    assert_eq!(output.dependencies[0].version, "0.11");
    assert_eq!(output.dependencies[0].features, vec!["json", "blocking"]);
}

// ─────────────────────────────────────────────────────────
// Tier 4: E2E — Compile + Run with real crate
// ─────────────────────────────────────────────────────────

#[test]
fn e2e_requires_itoa_in_escape_block() {
    let source = r#"## Requires
    The "itoa" crate version "1".

## Main
Escape to Rust:
    let mut buf = itoa::Buffer::new();
    let s = buf.format(12345i64);
    println!("{}", s);
"#;
    assert_exact_output(source, "12345");
}

#[test]
fn e2e_requires_with_features_in_escape_block() {
    let source = r#"## Requires
    The "base64" crate version "0.22" with features "std".

## Main
Escape to Rust:
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode("hello");
    println!("{}", encoded);
"#;
    assert_exact_output(source, "aGVsbG8=");
}

// ─────────────────────────────────────────────────────────
// Tier 5: Interpreter — Require stmts are no-ops
// ─────────────────────────────────────────────────────────

#[test]
fn interpreter_ignores_require_stmts() {
    let source = r#"## Requires
    The "itoa" crate version "1".

## Main
Let x be 42.
Show x.
"#;
    let result = run_interpreter(source);
    assert!(result.success, "Interpreter should succeed. Error: {}", result.error);
    assert_eq!(result.output.trim(), "42");
}

// ─────────────────────────────────────────────────────────
// Tier 6: Edge cases
// ─────────────────────────────────────────────────────────

#[test]
fn requires_empty_block_compiles() {
    let source = r#"## Requires

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn main()"), "Generated:\n{}", rust);
}

#[test]
fn requires_before_typedef_compiles() {
    let source = r#"## Requires
    The "itoa" crate version "1".

## A Counter has:
    a count: Int.

## Main
Let c be a new Counter with count 0.
Show c's count.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn main()"), "Generated:\n{}", rust);
    assert!(rust.contains("struct Counter"), "Has struct:\n{}", rust);
}

#[test]
fn requires_multiple_blocks_merged() {
    let source = r#"## Requires
    The "itoa" crate version "1".

## Main
Show 42.

## Requires
    The "ryu" crate version "1".
"#;
    let output = compile_program_full(source).expect("Should compile");
    assert_eq!(output.dependencies.len(), 2, "Both blocks should contribute deps");
    assert_eq!(output.dependencies[0].name, "itoa");
    assert_eq!(output.dependencies[1].name, "ryu");
}

#[test]
fn requires_after_main_compiles() {
    let source = r#"## Main
Show 42.

## Requires
    The "itoa" crate version "1".
"#;
    let output = compile_program_full(source).expect("Should compile");
    assert_eq!(output.dependencies.len(), 1);
    assert_eq!(output.dependencies[0].name, "itoa");
}

// ─────────────────────────────────────────────────────────
// Enhancement 1: compile_project returns dependencies
// ─────────────────────────────────────────────────────────

#[test]
fn compile_project_returns_dependencies() {
    let temp_dir = tempfile::tempdir().unwrap();
    let main_lg = temp_dir.path().join("main.lg");
    std::fs::write(&main_lg, r#"## Requires
    The "itoa" crate version "1".

## Main
Show 42.
"#).unwrap();

    use logicaffeine_compile::compile::compile_project;
    let output = compile_project(&main_lg).expect("Should compile");
    assert_eq!(output.dependencies.len(), 1, "compile_project should return dependencies");
    assert_eq!(output.dependencies[0].name, "itoa");
    assert_eq!(output.dependencies[0].version, "1");
}

// ─────────────────────────────────────────────────────────
// Enhancement 2: User-defined native function paths
// ─────────────────────────────────────────────────────────

#[test]
fn native_with_user_path_parses() {
    let source = r#"## To cube (n: Int) -> Int is native "my_crate::cube".

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should parse");
    assert!(output.rust_code.contains("fn cube"), "Generated:\n{}", output.rust_code);
}

#[test]
fn native_with_user_path_codegen() {
    let source = r#"## To cube (n: Int) -> Int is native "my_crate::cube".

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    assert!(output.rust_code.contains("my_crate::cube(n)"), "Should call user path. Generated:\n{}", output.rust_code);
}

#[test]
fn old_native_syntax_still_works() {
    let source = r#"## To native read (path: Text) -> Text.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Old syntax should still work");
    assert!(rust.contains("logicaffeine_system::file::read"), "Generated:\n{}", rust);
}

#[test]
fn native_is_native_without_path_requires_string() {
    let source = r#"## To cube (n: Int) -> Int is native.

## Main
Show 42.
"#;
    let result = compile_to_rust(source);
    assert!(result.is_err(), "is native without path string should be an error");
}

// ─────────────────────────────────────────────────────────
// Enhancement 3: Exported functions (C ABI + WASM)
// ─────────────────────────────────────────────────────────

#[test]
fn exported_function_parses() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should parse exported function");
    assert!(rust.contains("fn add"), "Generated:\n{}", rust);
}

#[test]
fn exported_for_wasm_parses() {
    let source = r#"## To greet () -> Int is exported for wasm:
    Return 42.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should parse exported for wasm");
    assert!(rust.contains("fn greet"), "Generated:\n{}", rust);
}

#[test]
fn exported_function_codegen_no_mangle() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("#[no_mangle]"), "C export should have #[no_mangle]. Generated:\n{}", rust);
}

#[test]
fn exported_function_codegen_extern_c() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("extern \"C\""), "C export should have extern \"C\". Generated:\n{}", rust);
}

#[test]
fn exported_function_codegen_wasm_bindgen() {
    let source = r#"## To greet () -> Int is exported for wasm:
    Return 42.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("#[wasm_bindgen]"), "WASM export should have #[wasm_bindgen]. Generated:\n{}", rust);
}

#[test]
fn exported_function_has_body() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("return (a + b);"), "Exported function should have body. Generated:\n{}", rust);
}

#[test]
fn e2e_exported_c_function_runs() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Let result be add(3, 4).
Show result.
"#;
    assert_exact_output(source, "7");
}

#[test]
fn exported_function_codegen_pub() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("pub extern \"C\" fn add"), "Should be pub extern. Generated:\n{}", rust);
}

// ─────────────────────────────────────────────────────────
// Enhancement 4: Integration tests — Requires + Escape + Native
// ─────────────────────────────────────────────────────────

#[test]
fn e2e_requires_and_escape_regex() {
    let source = r#"## Requires
    The "regex" crate version "1".

## Main
Escape to Rust:
    let re = regex::Regex::new(r"(\d+)").unwrap();
    let caps = re.captures("abc123def").unwrap();
    println!("{}", &caps[1]);
"#;
    assert_exact_output(source, "123");
}

#[test]
fn e2e_requires_multiple_crates_in_escape() {
    let source = r#"## Requires
    The "itoa" crate version "1".
    The "base64" crate version "0.22" with features "std".

## Main
Escape to Rust:
    let mut buf = itoa::Buffer::new();
    let s = buf.format(42i64);
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(s);
    println!("{}", encoded);
"#;
    assert_exact_output(source, "NDI=");
}

#[test]
fn e2e_requires_in_function_escape() {
    let source = r#"## Requires
    The "itoa" crate version "1".

## To formatNum (n: Int) -> Text:
    Escape to Rust:
        let mut buf = itoa::Buffer::new();
        let s = buf.format(n);
        return s.to_string();

## Main
Let result be formatNum(99).
Show result.
"#;
    assert_exact_output(source, "99");
}
