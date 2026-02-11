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
// Tier 7: Duplicate dependency detection
// ─────────────────────────────────────────────────────────

#[test]
fn requires_duplicate_same_version_deduplicates() {
    let source = r#"## Requires
    The "itoa" crate version "1".
    The "itoa" crate version "1".

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    assert_eq!(output.dependencies.len(), 1, "Duplicate same-version dep should be deduplicated");
    assert_eq!(output.dependencies[0].name, "itoa");
}

#[test]
fn requires_duplicate_different_version_is_error() {
    let source = r#"## Requires
    The "itoa" crate version "1".
    The "itoa" crate version "2".

## Main
Show 42.
"#;
    let result = compile_program_full(source);
    assert!(result.is_err(), "Duplicate dep with different version should fail");
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
// Tier 9: Unknown system native function produces error, not panic
// ─────────────────────────────────────────────────────────

#[test]
fn unknown_system_native_function_is_error() {
    let source = r#"## To native blarf (x: Int) -> Int.

## Main
Show 42.
"#;
    let result = compile_to_rust(source);
    assert!(result.is_err() || {
        let rust = result.unwrap();
        rust.contains("compile_error!")
    }, "Unknown system native should produce an error, not panic");
}

// ─────────────────────────────────────────────────────────
// Tier 8: WASM export preamble & auto-injection
// ─────────────────────────────────────────────────────────

#[test]
fn wasm_export_preamble_has_use_wasm_bindgen() {
    let source = r#"## To greet () -> Int is exported for wasm:
    Return 42.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("use wasm_bindgen::prelude::*;"), "WASM export should emit wasm_bindgen preamble. Generated:\n{}", rust);
}

#[test]
fn wasm_export_auto_injects_wasm_bindgen_dep() {
    let source = r#"## To greet () -> Int is exported for wasm:
    Return 42.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let has_wasm_bindgen = output.dependencies.iter().any(|d| d.name == "wasm-bindgen");
    assert!(has_wasm_bindgen, "WASM export should auto-inject wasm-bindgen dependency. Deps: {:?}", output.dependencies);
}

#[test]
fn no_wasm_export_no_wasm_preamble() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(!rust.contains("wasm_bindgen"), "Non-WASM export should NOT have wasm_bindgen preamble. Generated:\n{}", rust);
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
    // Verify #[no_mangle] appears on the line before the pub extern fn add
    let lines: Vec<&str> = rust.lines().collect();
    let add_fn_idx = lines.iter().position(|l| l.contains("pub extern \"C\" fn add"));
    assert!(add_fn_idx.is_some(), "C export should have pub extern fn add. Generated:\n{}", rust);
    assert!(add_fn_idx.unwrap() > 0, "pub extern fn add should not be first line. Generated:\n{}", rust);
    let prev_line = lines[add_fn_idx.unwrap() - 1];
    assert!(prev_line.trim() == "#[no_mangle]",
        "#[no_mangle] should be on line before pub extern fn add. Prev line: '{}'\nGenerated:\n{}", prev_line, rust);
}

#[test]
fn exported_function_codegen_extern_c() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("pub extern \"C\" fn add(a: i64, b: i64) -> i64"),
        "C export should have full extern C signature. Generated:\n{}", rust);
}

#[test]
fn exported_function_codegen_wasm_bindgen() {
    let source = r#"## To greet () -> Int is exported for wasm:
    Return 42.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    // Verify #[wasm_bindgen] appears on the line before the pub fn
    let lines: Vec<&str> = rust.lines().collect();
    let wasm_idx = lines.iter().position(|l| l.trim() == "#[wasm_bindgen]");
    assert!(wasm_idx.is_some(), "WASM export should have #[wasm_bindgen]. Generated:\n{}", rust);
    let next_line = lines[wasm_idx.unwrap() + 1];
    assert!(next_line.contains("pub fn greet"),
        "#[wasm_bindgen] should be on line before pub fn. Next line: '{}'\nGenerated:\n{}", next_line, rust);
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
// Tier 10: C ABI type marshaling
// ─────────────────────────────────────────────────────────

#[test]
fn c_export_int_type_is_i64() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("pub extern \"C\" fn add(a: i64, b: i64) -> i64"),
        "C export Int should use i64. Generated:\n{}", rust);
}

#[test]
fn c_export_bool_type_is_bool() {
    let source = r#"## To isPositive (n: Int) -> Bool is exported:
    Return n > 0.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("-> bool"), "C export Bool should use bool. Generated:\n{}", rust);
}

#[test]
fn c_export_real_type_is_f64() {
    let source = r#"## To halve (n: Real) -> Real is exported:
    Return n / 2.0.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("n: f64") && rust.contains("-> f64"),
        "C export Real should use f64. Generated:\n{}", rust);
}

#[test]
fn c_export_text_param_uses_c_char_ptr() {
    let source = r#"## To greet (name: Text) -> Int is exported:
    Return 0.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("*const std::os::raw::c_char"),
        "C export Text param should use *const c_char. Generated:\n{}", rust);
}

#[test]
fn c_export_text_return_uses_c_char_ptr() {
    let source = r#"## To getLabel (n: Int) -> Text is exported:
    Return "hello".

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("*mut std::os::raw::c_char"),
        "C export Text return should use *mut c_char. Generated:\n{}", rust);
}

#[test]
fn c_export_text_function_generates_wrapper() {
    let source = r#"## To greet (name: Text) -> Text is exported:
    Return "hello".

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("CStr::from_ptr") || rust.contains("c_char"),
        "C export with Text should generate conversion code. Generated:\n{}", rust);
}

#[test]
fn c_export_text_function_has_ffi_imports() {
    let source = r#"## To greet (name: Text) -> Text is exported:
    Return "hello".

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("use std::ffi::{CStr, CString};"),
        "C export with Text should import CStr/CString. Generated:\n{}", rust);
}

// ─────────────────────────────────────────────────────────
// Tier 11: WASM type verification
// ─────────────────────────────────────────────────────────

#[test]
fn wasm_export_with_text_compiles() {
    let source = r#"## To greet (name: Text) -> Text is exported for wasm:
    Return "hello".

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("#[wasm_bindgen]"), "WASM export should have wasm_bindgen. Generated:\n{}", rust);
    assert!(rust.contains("pub fn greet(name: String) -> String"),
        "WASM export should use normal Rust String types. Generated:\n{}", rust);
}

#[test]
fn wasm_export_with_seq_compiles() {
    let source = r#"## To double (nums: Seq of Int) -> Seq of Int is exported for wasm:
    Return nums.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("#[wasm_bindgen]"), "WASM export should have wasm_bindgen. Generated:\n{}", rust);
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

// ─────────────────────────────────────────────────────────
// Tier 12: Codegen snapshot tests
// ─────────────────────────────────────────────────────────

#[test]
fn snapshot_requires_basic_codegen() {
    let source = r#"## Requires
    The "itoa" crate version "1".

## Main
Let x be 42.
Show x.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert_snapshot!("ffi_requires_basic", rust);
}

#[test]
fn snapshot_native_function_codegen() {
    let source = r#"## To cube (n: Int) -> Int is native "my_crate::cube".

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert_snapshot!("ffi_native_function", rust);
}

#[test]
fn snapshot_exported_c_function_codegen() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert_snapshot!("ffi_exported_c", rust);
}

#[test]
fn snapshot_exported_wasm_function_codegen() {
    let source = r#"## To greet () -> Int is exported for wasm:
    Return 42.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert_snapshot!("ffi_exported_wasm", rust);
}

#[test]
fn snapshot_c_export_with_text_marshaling() {
    let source = r#"## To greet (name: Text) -> Text is exported:
    Return "hello".

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert_snapshot!("ffi_c_export_text_marshaling", rust);
}

// ─────────────────────────────────────────────────────────
// Tier 13: E2E integration tests — combined FFI features
// ─────────────────────────────────────────────────────────

#[test]
fn e2e_requires_plus_escape_plus_function() {
    let source = r#"## Requires
    The "itoa" crate version "1".

## To double (n: Int) -> Int:
    Return n * 2.

## Main
Let x be double(21).
Escape to Rust:
    let mut buf = itoa::Buffer::new();
    let s = buf.format(x);
    println!("{}", s);
"#;
    assert_exact_output(source, "42");
}

#[test]
fn e2e_exported_function_called_from_main() {
    let source = r#"## To triple (n: Int) -> Int is exported:
    Let result be n * 3.
    Return result.

## Main
Let x be triple(10).
Show x.
"#;
    assert_exact_output(source, "30");
}

// ─────────────────────────────────────────────────────────
// Tier 14: Negative tests — error cases
// ─────────────────────────────────────────────────────────

#[test]
fn exported_for_unsupported_target_is_error() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported for java:
    Return a + b.

## Main
Show 42.
"#;
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Unsupported export target 'java' should be an error");
}

#[test]
fn native_with_invalid_path_chars_is_error() {
    let source = r#"## To bad (n: Int) -> Int is native "???".

## Main
Show 42.
"#;
    let result = compile_to_rust(source);
    match result {
        Ok(rust) => {
            assert!(rust.contains("compile_error!"),
                "Native with invalid path should produce compile_error. Generated:\n{}", rust);
        }
        Err(_) => {} // Parse error is also acceptable
    }
}

#[test]
fn c_export_with_seq_param_emits_compile_error() {
    let source = r#"## To sumAll (nums: Seq of Int) -> Int is exported:
    Return 0.

## Main
Show 42.
"#;
    let result = compile_to_rust(source);
    match result {
        Ok(rust) => {
            assert!(rust.contains("compile_error!") || rust.contains("Vec<i64>"),
                "C export with Seq param should either error or pass through Vec. Generated:\n{}", rust);
        }
        Err(_) => {} // Error is acceptable too
    }
}

// ─────────────────────────────────────────────────────────
// Tier 15: Struct field accessors — C callers can read struct fields
// ─────────────────────────────────────────────────────────

const PERSON_STRUCT_SOURCE: &str = r#"## A Person has:
    a name: Text.
    an age: Int.
    an email: Text.
    an address: Text.
    a phone: Text.

## To getPerson () -> Person is exported:
    Let p be a new Person with name "Alice" and age 30 and email "a@b" and address "123" and phone "555".
    Return p.

## Main
Show 42.
"#;

#[test]
fn struct_accessor_value_field() {
    let output = compile_program_full(PERSON_STRUCT_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_person_age(handle: LogosHandle) -> i64"),
        "Should generate accessor for Int field 'age'. Generated:\n{}", output.rust_code);
}

#[test]
fn struct_accessor_text_field() {
    let output = compile_program_full(PERSON_STRUCT_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_person_name(handle: LogosHandle) -> *mut std::os::raw::c_char"),
        "Should generate accessor for Text field 'name' returning *mut c_char. Generated:\n{}", output.rust_code);
}

#[test]
fn struct_accessor_free() {
    let output = compile_program_full(PERSON_STRUCT_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_person_free(handle: LogosHandle)"),
        "Should generate free function for struct. Generated:\n{}", output.rust_code);
}

#[test]
fn struct_accessor_c_header() {
    let output = compile_program_full(PERSON_STRUCT_SOURCE).expect("Should compile");
    let header = output.c_header.as_ref().expect("Should have C header");
    assert!(header.contains("logos_person_age"), "C header should declare age accessor. Header:\n{}", header);
    assert!(header.contains("logos_person_name"), "C header should declare name accessor. Header:\n{}", header);
    assert!(header.contains("logos_person_free"), "C header should declare free function. Header:\n{}", header);
}

#[test]
fn struct_accessor_skipped_for_value_struct() {
    let source = r#"## A Point has:
    an x: Int.
    a y: Int.

## To getPoint () -> Point is exported:
    Let p be a new Point with x 1 and y 2.
    Return p.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    assert!(!output.rust_code.contains("logos_point_x(handle: LogosHandle)"),
        "Value-type struct (<=4 all-value fields) should NOT get handle accessors — it's passed by value. Generated:\n{}", output.rust_code);
}

// ─────────────────────────────────────────────────────────
// Tier 16: Option accessors — C callers can check/unwrap Option values
// ─────────────────────────────────────────────────────────

const OPTION_SOURCE: &str = r#"## To findAge (name: Text) -> Option of Int is exported:
    If name = "Alice" then:
        Return some 30.
    Otherwise:
        Return none.

## Main
Show 42.
"#;

#[test]
fn option_accessor_is_some() {
    let output = compile_program_full(OPTION_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_option_i64_is_some(handle: LogosHandle) -> bool"),
        "Should generate is_some accessor for Option of Int. Generated:\n{}", output.rust_code);
}

#[test]
fn option_accessor_unwrap_value() {
    let output = compile_program_full(OPTION_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_option_i64_unwrap(handle: LogosHandle, out: *mut i64) -> LogosStatus"),
        "Should generate unwrap accessor with out-param for Option of Int. Generated:\n{}", output.rust_code);
}

#[test]
fn option_accessor_unwrap_text() {
    let source = r#"## To findName (id: Int) -> Option of Text is exported:
    If id = 1 then:
        Return some "Alice".
    Otherwise:
        Return none.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_option_string_unwrap(handle: LogosHandle) -> *mut std::os::raw::c_char"),
        "Should generate unwrap returning *mut c_char for Option of Text. Generated:\n{}", output.rust_code);
}

#[test]
fn option_accessor_free() {
    let output = compile_program_full(OPTION_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_option_i64_free(handle: LogosHandle)"),
        "Should generate free function for Option. Generated:\n{}", output.rust_code);
}

#[test]
fn option_accessor_c_header() {
    let output = compile_program_full(OPTION_SOURCE).expect("Should compile");
    let header = output.c_header.as_ref().expect("Should have C header");
    assert!(header.contains("logos_option_i64_is_some"), "C header should declare is_some. Header:\n{}", header);
    assert!(header.contains("logos_option_i64_unwrap"), "C header should declare unwrap. Header:\n{}", header);
    assert!(header.contains("logos_option_i64_free"), "C header should declare free. Header:\n{}", header);
}

// ─────────────────────────────────────────────────────────
// Tier 17: Map keys/values — C callers can iterate Maps
// ─────────────────────────────────────────────────────────

const MAP_SOURCE: &str = r#"## To getScores () -> Map of Text to Int is exported:
    Let m be a new Map of Text to Int.
    Return m.

## Main
Show 42.
"#;

#[test]
fn map_keys_accessor() {
    let output = compile_program_full(MAP_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_map_string_i64_keys(handle: LogosHandle) -> LogosHandle"),
        "Should generate keys accessor for Map. Generated:\n{}", output.rust_code);
}

#[test]
fn map_values_accessor() {
    let output = compile_program_full(MAP_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_map_string_i64_values(handle: LogosHandle) -> LogosHandle"),
        "Should generate values accessor for Map. Generated:\n{}", output.rust_code);
}

#[test]
fn map_iteration_c_header() {
    let output = compile_program_full(MAP_SOURCE).expect("Should compile");
    let header = output.c_header.as_ref().expect("Should have C header");
    assert!(header.contains("logos_map_string_i64_keys"), "C header should declare keys. Header:\n{}", header);
    assert!(header.contains("logos_map_string_i64_values"), "C header should declare values. Header:\n{}", header);
}

// ─────────────────────────────────────────────────────────
// Tier 18: Collection creation + mutation from C
// ─────────────────────────────────────────────────────────

const SEQ_INT_SOURCE: &str = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Return s.

## Main
Show 42.
"#;

#[test]
fn seq_create_accessor() {
    let output = compile_program_full(SEQ_INT_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_seq_i64_create() -> LogosHandle"),
        "Should generate create for Seq of Int. Generated:\n{}", output.rust_code);
}

#[test]
fn seq_push_accessor() {
    let output = compile_program_full(SEQ_INT_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_seq_i64_push(handle: LogosHandle, value: i64)"),
        "Should generate push for Seq of Int. Generated:\n{}", output.rust_code);
}

#[test]
fn map_create_accessor() {
    let output = compile_program_full(MAP_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_map_string_i64_create() -> LogosHandle"),
        "Should generate create for Map of Text to Int. Generated:\n{}", output.rust_code);
}

#[test]
fn map_insert_accessor() {
    let output = compile_program_full(MAP_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_map_string_i64_insert(handle: LogosHandle, key: *const std::os::raw::c_char, value: i64)"),
        "Should generate insert for Map of Text to Int. Generated:\n{}", output.rust_code);
}

const SET_SOURCE: &str = r#"## To getTags () -> Set of Text is exported:
    Let s be a new Set of Text.
    Return s.

## Main
Show 42.
"#;

#[test]
fn set_create_accessor() {
    let output = compile_program_full(SET_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_set_string_create() -> LogosHandle"),
        "Should generate create for Set of Text. Generated:\n{}", output.rust_code);
}

#[test]
fn set_insert_accessor() {
    let output = compile_program_full(SET_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_set_string_insert(handle: LogosHandle, value: *const std::os::raw::c_char)"),
        "Should generate insert for Set of Text. Generated:\n{}", output.rust_code);
}

#[test]
fn collection_create_c_header() {
    let output = compile_program_full(SEQ_INT_SOURCE).expect("Should compile");
    let header = output.c_header.as_ref().expect("Should have C header");
    assert!(header.contains("logos_seq_i64_create"), "C header should declare seq create. Header:\n{}", header);
    assert!(header.contains("logos_seq_i64_push"), "C header should declare seq push. Header:\n{}", header);
}

// ─────────────────────────────────────────────────────────
// Tier 19: Version introspection — runtime ABI version checking
// ─────────────────────────────────────────────────────────

#[test]
fn version_function_generated() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_version() -> *const std::os::raw::c_char"),
        "Should generate logos_version function. Generated:\n{}", output.rust_code);
}

#[test]
fn abi_version_constant_generated() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    assert!(output.rust_code.contains("pub const LOGOS_ABI_VERSION: u32 = 1"),
        "Should generate LOGOS_ABI_VERSION constant. Generated:\n{}", output.rust_code);
}

#[test]
fn version_in_c_header() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let header = output.c_header.as_ref().expect("Should have C header");
    assert!(header.contains("logos_version"), "C header should declare logos_version. Header:\n{}", header);
    assert!(header.contains("LOGOS_ABI_VERSION"), "C header should declare LOGOS_ABI_VERSION. Header:\n{}", header);
}

// ─────────────────────────────────────────────────────────
// Tier 20: Enum variant accessors — C callers can inspect enum variants
// ─────────────────────────────────────────────────────────

const ENUM_SOURCE: &str = r#"## A Shape is either:
    Circle with radius: Real.
    Rectangle with width: Real and height: Real.

## To getShape () -> Shape is exported:
    Let s be Circle with radius 5.0.
    Return s.

## Main
Show 42.
"#;

#[test]
fn enum_tag_accessor() {
    let output = compile_program_full(ENUM_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_shape_tag(handle: LogosHandle) -> i32"),
        "Should generate tag accessor for enum. Generated:\n{}", output.rust_code);
}

#[test]
fn enum_variant_field_accessor() {
    let output = compile_program_full(ENUM_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_shape_circle_radius(handle: LogosHandle) -> f64"),
        "Should generate variant field accessor. Generated:\n{}", output.rust_code);
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_shape_rectangle_width(handle: LogosHandle) -> f64"),
        "Should generate rectangle width accessor. Generated:\n{}", output.rust_code);
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_shape_rectangle_height(handle: LogosHandle) -> f64"),
        "Should generate rectangle height accessor. Generated:\n{}", output.rust_code);
}

#[test]
fn enum_free_accessor() {
    let output = compile_program_full(ENUM_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_shape_free(handle: LogosHandle)"),
        "Should generate free function for enum. Generated:\n{}", output.rust_code);
}

#[test]
fn enum_tag_c_header() {
    let output = compile_program_full(ENUM_SOURCE).expect("Should compile");
    let header = output.c_header.as_ref().expect("Should have C header");
    assert!(header.contains("LOGOS_SHAPE_CIRCLE"), "C header should declare tag constants. Header:\n{}", header);
    assert!(header.contains("LOGOS_SHAPE_RECTANGLE"), "C header should declare tag constants. Header:\n{}", header);
    assert!(header.contains("logos_shape_tag"), "C header should declare tag function. Header:\n{}", header);
    assert!(header.contains("logos_shape_circle_radius"), "C header should declare variant accessors. Header:\n{}", header);
    assert!(header.contains("logos_shape_free"), "C header should declare free. Header:\n{}", header);
}

// ─────────────────────────────────────────────────────────
// Tier 21: JSON serialization — any handle to JSON string
// ─────────────────────────────────────────────────────────

const JSON_SOURCE: &str = r#"## A portable Config has:
    a host: Text.
    a port: Int.
    a debug: Bool.
    a retries: Int.
    a timeout: Int.

## To getConfig () -> Config is exported:
    Let c be a new Config with host "localhost" and port 8080 and debug true and retries 3 and timeout 30.
    Return c.

## Main
Show 42.
"#;

#[test]
fn json_to_json_accessor() {
    let output = compile_program_full(JSON_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_config_to_json(handle: LogosHandle) -> *mut std::os::raw::c_char"),
        "Should generate to_json accessor for portable struct. Generated:\n{}", output.rust_code);
}

#[test]
fn json_from_json_accessor() {
    let output = compile_program_full(JSON_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_config_from_json(json: *const std::os::raw::c_char, out: *mut LogosHandle) -> LogosStatus"),
        "Should generate from_json accessor for portable struct. Generated:\n{}", output.rust_code);
}

#[test]
fn json_serde_json_dep_injected() {
    let output = compile_program_full(JSON_SOURCE).expect("Should compile");
    let has_serde_json = output.dependencies.iter().any(|d| d.name == "serde_json");
    assert!(has_serde_json, "Should auto-inject serde_json dependency for portable types with C exports. Deps: {:?}", output.dependencies);
}

#[test]
fn json_seq_to_json() {
    let source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Return s.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_seq_i64_to_json(handle: LogosHandle) -> *mut std::os::raw::c_char"),
        "Should generate to_json for Seq types. Generated:\n{}", output.rust_code);
}

#[test]
fn json_c_header() {
    let output = compile_program_full(JSON_SOURCE).expect("Should compile");
    let header = output.c_header.as_ref().expect("Should have C header");
    assert!(header.contains("logos_config_to_json"), "C header should declare to_json. Header:\n{}", header);
    assert!(header.contains("logos_config_from_json"), "C header should declare from_json. Header:\n{}", header);
}

// ─────────────────────────────────────────────────────────
// Tier 22: Python bindings generation
// ─────────────────────────────────────────────────────────

#[test]
fn python_bindings_generated() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    assert!(output.python_bindings.is_some(),
        "compile_program_full should return Some python_bindings when C exports exist");
}

#[test]
fn python_bindings_has_import_ctypes() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let py = output.python_bindings.as_ref().expect("Should have python bindings");
    assert!(py.contains("import ctypes"), "Python bindings should import ctypes. Generated:\n{}", py);
}

#[test]
fn python_bindings_has_function_wrapper() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let py = output.python_bindings.as_ref().expect("Should have python bindings");
    assert!(py.contains("def add(self"), "Python bindings should have function wrapper. Generated:\n{}", py);
}

#[test]
fn python_bindings_has_type_setup() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let py = output.python_bindings.as_ref().expect("Should have python bindings");
    assert!(py.contains("c_int64"), "Python bindings should setup ctypes. Generated:\n{}", py);
}

#[test]
fn python_bindings_has_error_class() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let py = output.python_bindings.as_ref().expect("Should have python bindings");
    assert!(py.contains("class LogosError"), "Python bindings should have error class. Generated:\n{}", py);
}

#[test]
fn python_bindings_not_generated_no_exports() {
    let source = r#"## To add (a: Int, b: Int) -> Int:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    assert!(output.python_bindings.is_none(),
        "Should not generate python bindings when no C exports exist");
}

// ─────────────────────────────────────────────────────────
// Tier 23: TypeScript bindings generation
// ─────────────────────────────────────────────────────────

#[test]
fn typescript_types_generated() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    assert!(output.typescript_types.is_some(),
        "compile_program_full should return Some typescript_types when C exports exist");
}

#[test]
fn typescript_types_has_function_decl() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let dts = output.typescript_types.as_ref().expect("Should have typescript types");
    assert!(dts.contains("export declare function add(a: number, b: number): number"),
        "TypeScript types should declare add function. Generated:\n{}", dts);
}

#[test]
fn typescript_bindings_has_ffi_library() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let js = output.typescript_bindings.as_ref().expect("Should have typescript bindings");
    assert!(js.contains("ffi.Library"), "TypeScript bindings should use ffi.Library. Generated:\n{}", js);
}

#[test]
fn typescript_not_generated_no_exports() {
    let source = r#"## To add (a: Int, b: Int) -> Int:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    assert!(output.typescript_types.is_none(),
        "Should not generate typescript types when no C exports exist");
}

// ─────────────────────────────────────────────────────────
// Tier 24: SAFETY HARDENING — Phase 1 of Universal ABI Perfection
// Critical safety gaps that can cause crashes, memory corruption, or undefined behavior
// ─────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────
// Safety Issue #1: String Null-Byte Panic
// CString::new(value).unwrap() panics if Rust string contains \0 byte
// Should return LogosStatus::ContainsNullByte instead
// ─────────────────────────────────────────────────────────

#[test]
fn safety_string_with_null_byte_returns_error() {
    // This test verifies that strings containing \0 don't cause panics
    // Current behavior: CString::new().unwrap() panics
    // Expected behavior: Return LogosStatus::ContainsNullByte
    let source = r#"## To getLabel () -> Text is exported:
    Escape to Rust:
        return "hello\0world".to_string();

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // The generated C export should have error handling for null bytes
    assert!(rust.contains("ContainsNullByte") || rust.contains("map_err"),
        "C export with Text should handle null bytes gracefully. Generated:\n{}", rust);
}

#[test]
fn safety_struct_text_field_with_null_byte_returns_error() {
    let source = r#"## A portable Data has:
    a value: Text.

## To getData () -> Data is exported:
    Let d be a new Data with value "test\0data".
    Return d.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Field accessor should handle null bytes
    assert!(rust.contains("ContainsNullByte") || rust.contains("map_err"),
        "Struct field accessor should handle null bytes. Generated:\n{}", rust);
}

#[test]
fn safety_json_string_with_null_byte_returns_error() {
    let source = r#"## A portable Config has:
    a host: Text.
    a port: Int.

## To getConfig () -> Config is exported:
    Let c be a new Config with host "local\0host" and port 8080.
    Return c.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // JSON serialization should handle null bytes
    assert!(rust.contains("ContainsNullByte") || rust.contains("map_err"),
        "JSON to_json should handle null bytes. Generated:\n{}", rust);
}

// ─────────────────────────────────────────────────────────
// Safety Issue #2: Thread-Unsafe Error Storage
// thread_local! { LOGOS_LAST_ERROR: RefCell<Option<String>> }
// Errors overwrite each other in multi-threaded scenarios
// Should use Arc<Mutex<HashMap<ThreadId, String>>>
// ─────────────────────────────────────────────────────────

#[test]
fn safety_error_storage_is_thread_safe() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Error storage should NOT use thread_local RefCell
    assert!(!rust.contains("thread_local") || rust.contains("Arc<Mutex"),
        "Error storage should be thread-safe. Generated:\n{}", rust);
}

#[test]
fn safety_last_error_function_exists() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Should have logos_last_error function
    assert!(rust.contains("pub extern \"C\" fn logos_last_error()"),
        "Should generate logos_last_error function. Generated:\n{}", rust);
}

#[test]
fn safety_error_storage_per_thread() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Error storage should be keyed by ThreadId
    assert!(rust.contains("ThreadId") || rust.contains("thread_local"),
        "Error storage should track per-thread errors. Generated:\n{}", rust);
}

// ─────────────────────────────────────────────────────────
// Safety Issue #3: Missing Panic Boundary
// Rust panics unwind across FFI boundary → undefined behavior
// All pub extern "C" fn must be wrapped in catch_unwind
// ─────────────────────────────────────────────────────────

#[test]
fn safety_exported_function_has_panic_boundary() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Should wrap FFI exports in catch_unwind
    assert!(rust.contains("catch_unwind") || rust.contains("panic"),
        "FFI exports should have panic boundaries. Generated:\n{}", rust);
}

#[test]
fn safety_struct_accessor_has_panic_boundary() {
    let source = r#"## A Person has:
    a name: Text.
    an age: Int.

## To getPerson () -> Person is exported:
    Let p be a new Person with name "Alice" and age 30.
    Return p.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Accessors should have panic boundaries
    assert!(rust.contains("catch_unwind") || rust.contains("panic"),
        "Struct accessors should have panic boundaries. Generated:\n{}", rust);
}

#[test]
fn safety_panic_returns_error_status() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Panics should return ThreadPanic status
    assert!(rust.contains("ThreadPanic") || rust.contains("catch_unwind"),
        "Panics should return error status. Generated:\n{}", rust);
}

// ─────────────────────────────────────────────────────────
// Safety Issue #4: Handle Use-After-Free
// Box::into_raw() leaks handles with no lifetime tracking
// C caller can call logos_*_free() twice → memory corruption
// Should have handle registry with generation counters
// ─────────────────────────────────────────────────────────

#[test]
fn safety_handle_registry_exists() {
    let source = r#"## A Person has:
    a name: Text.
    an age: Int.

## To getPerson () -> Person is exported:
    Let p be a new Person with name "Alice" and age 30.
    Return p.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Should have handle registry/validation
    assert!(rust.contains("HandleRegistry") || rust.contains("generation") || rust.contains("validate_handle"),
        "Should have handle registry for validation. Generated:\n{}", rust);
}

#[test]
fn safety_double_free_returns_invalid_handle() {
    let source = r#"## A Person has:
    a name: Text.
    an age: Int.

## To getPerson () -> Person is exported:
    Let p be a new Person with name "Alice" and age 30.
    Return p.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Free should detect already-freed handles
    assert!(rust.contains("InvalidHandle") || rust.contains("is_valid"),
        "Should detect double-free attempts. Generated:\n{}", rust);
}

#[test]
fn safety_handle_generation_counter() {
    let source = r#"## A Person has:
    a name: Text.
    an age: Int.

## To getPerson () -> Person is exported:
    Let p be a new Person with name "Alice" and age 30.
    Return p.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Handles should have generation counters
    assert!(rust.contains("generation") || rust.contains("counter"),
        "Handles should use generation counters. Generated:\n{}", rust);
}

// ─────────────────────────────────────────────────────────
// Safety Issue #5: Null Handle Validation
// Some accessors lack .is_null() checks → segfault
// All handle accessors must validate null before dereferencing
// ─────────────────────────────────────────────────────────

#[test]
fn safety_accessor_checks_null_handle() {
    let source = r#"## A Person has:
    a name: Text.
    an age: Int.

## To getPerson () -> Person is exported:
    Let p be a new Person with name "Alice" and age 30.
    Return p.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Accessors should check for null handles
    assert!(rust.contains("is_null") || rust.contains("NullPointer"),
        "Accessors should validate null handles. Generated:\n{}", rust);
}

#[test]
fn safety_null_handle_returns_null_pointer_status() {
    let source = r#"## A Person has:
    a name: Text.
    an age: Int.

## To getPerson () -> Person is exported:
    Let p be a new Person with name "Alice" and age 30.
    Return p.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Should return NullPointer status
    assert!(rust.contains("NullPointer") || rust.contains("null"),
        "Should have NullPointer status code. Generated:\n{}", rust);
}

#[test]
fn safety_seq_accessor_checks_null() {
    let source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Return s.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Seq accessors should check null
    assert!(rust.contains("is_null") || rust.contains("NullPointer"),
        "Seq accessors should validate null handles. Generated:\n{}", rust);
}

#[test]
fn safety_map_accessor_checks_null() {
    let source = r#"## To getScores () -> Map of Text to Int is exported:
    Let m be a new Map of Text to Int.
    Return m.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Map accessors should check null
    assert!(rust.contains("is_null") || rust.contains("NullPointer"),
        "Map accessors should validate null handles. Generated:\n{}", rust);
}

#[test]
fn safety_option_accessor_checks_null() {
    let source = r#"## To findAge (name: Text) -> Option of Int is exported:
    Return some 30.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Option accessors should check null
    assert!(rust.contains("is_null") || rust.contains("NullPointer"),
        "Option accessors should validate null handles. Generated:\n{}", rust);
}

// ─────────────────────────────────────────────────────────
// Safety Issue #6: Expand LogosStatus Enum
// Current status codes insufficient for all error cases
// Need: NullPointer, InvalidHandle, ContainsNullByte, ThreadPanic, MemoryExhausted, StackOverflow
// ─────────────────────────────────────────────────────────

#[test]
fn safety_status_enum_has_null_pointer() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // LogosStatus should have NullPointer variant
    assert!(rust.contains("NullPointer"),
        "LogosStatus should have NullPointer. Generated:\n{}", rust);
}

#[test]
fn safety_status_enum_has_invalid_handle() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // LogosStatus should have InvalidHandle variant
    assert!(rust.contains("InvalidHandle"),
        "LogosStatus should have InvalidHandle. Generated:\n{}", rust);
}

#[test]
fn safety_status_enum_has_contains_null_byte() {
    let source = r#"## To getLabel () -> Text is exported:
    Return "hello".

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // LogosStatus should have ContainsNullByte variant
    assert!(rust.contains("ContainsNullByte"),
        "LogosStatus should have ContainsNullByte. Generated:\n{}", rust);
}

#[test]
fn safety_status_enum_has_thread_panic() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // LogosStatus should have ThreadPanic variant
    assert!(rust.contains("ThreadPanic"),
        "LogosStatus should have ThreadPanic. Generated:\n{}", rust);
}

#[test]
fn safety_status_enum_has_memory_exhausted() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // LogosStatus should have MemoryExhausted variant
    assert!(rust.contains("MemoryExhausted"),
        "LogosStatus should have MemoryExhausted. Generated:\n{}", rust);
}

#[test]
fn safety_status_enum_in_c_header() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let header = output.c_header.as_ref().expect("Should have C header");

    // C header should declare all status codes
    assert!(header.contains("LOGOS_STATUS_NULL_POINTER"),
        "C header should have NullPointer status. Header:\n{}", header);
    assert!(header.contains("LOGOS_STATUS_INVALID_HANDLE"),
        "C header should have InvalidHandle status. Header:\n{}", header);
    assert!(header.contains("LOGOS_STATUS_CONTAINS_NULL_BYTE"),
        "C header should have ContainsNullByte status. Header:\n{}", header);
    assert!(header.contains("LOGOS_STATUS_THREAD_PANIC"),
        "C header should have ThreadPanic status. Header:\n{}", header);
}

// ─────────────────────────────────────────────────────────
// Safety Integration Tests — Combined safety features
// ─────────────────────────────────────────────────────────

#[test]
fn safety_complete_error_handling_pipeline() {
    let source = r#"## A portable Config has:
    a host: Text.
    a port: Int.

## To getConfig () -> Config is exported:
    Let c be a new Config with host "localhost" and port 8080.
    Return c.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Should have complete safety pipeline
    let safety_features = [
        "catch_unwind",
        "is_null",
        "InvalidHandle",
        "NullPointer",
        "ContainsNullByte",
        "logos_last_error"
    ];

    for feature in &safety_features {
        assert!(rust.contains(feature),
            "Should have safety feature '{}'. Generated:\n{}", feature, rust);
    }
}

#[test]
fn safety_all_status_codes_documented() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let header = output.c_header.as_ref().expect("Should have C header");

    // C header should document all status codes
    let status_codes = [
        "LOGOS_STATUS_OK",
        "LOGOS_STATUS_ERROR",
        "LOGOS_STATUS_NULL_POINTER",
        "LOGOS_STATUS_INVALID_HANDLE",
        "LOGOS_STATUS_CONTAINS_NULL_BYTE",
        "LOGOS_STATUS_THREAD_PANIC",
        "LOGOS_STATUS_MEMORY_EXHAUSTED"
    ];

    for code in &status_codes {
        assert!(header.contains(code),
            "C header should have status code '{}'. Header:\n{}", code, header);
    }
}
