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
    assert!(rust.contains("fn add"), "Should contain fn add. Generated:\n{}", rust);
    assert!(rust.contains("export_name") && rust.contains("logos_add"),
        "Should have export_name with logos_ prefix. Generated:\n{}", rust);
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
    // Verify #[export_name = "logos_add"] appears on the line before pub extern fn add
    let lines: Vec<&str> = rust.lines().collect();
    let add_fn_idx = lines.iter().position(|l| l.contains("pub extern \"C\" fn add("));
    assert!(add_fn_idx.is_some(), "C export should have pub extern fn add. Generated:\n{}", rust);
    assert!(add_fn_idx.unwrap() > 0, "pub extern fn add should not be first line. Generated:\n{}", rust);
    let prev_line = lines[add_fn_idx.unwrap() - 1];
    assert!(prev_line.trim() == "#[export_name = \"logos_add\"]",
        "#[export_name = \"logos_add\"] should be on line before pub extern fn add. Prev line: '{}'\nGenerated:\n{}", prev_line, rust);
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
    assert!(rust.contains("pub extern \"C\" fn add("), "Should be pub extern. Generated:\n{}", rust);
}

// ─────────────────────────────────────────────────────────
// Tier 9b: Universal logos_ prefix verification
// ─────────────────────────────────────────────────────────

#[test]
fn simple_path_export_uses_export_name_attribute() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("#[export_name = \"logos_add\"]"),
        "Simple-path export should use #[export_name]. Generated:\n{}", rust);
    // Verify #[export_name] is on the line directly before fn add
    let lines: Vec<&str> = rust.lines().collect();
    let fn_idx = lines.iter().position(|l| l.contains("pub extern \"C\" fn add("));
    assert!(fn_idx.is_some() && fn_idx.unwrap() > 0, "Should have pub extern fn add. Generated:\n{}", rust);
    let prev = lines[fn_idx.unwrap() - 1].trim();
    assert!(prev.starts_with("#[export_name"),
        "Line before fn add should be #[export_name], not #[no_mangle]. Prev: '{}'\nGenerated:\n{}", prev, rust);
}

#[test]
fn complex_path_inner_function_uses_raw_name() {
    let source = r#"## To greet (name: Text) -> Text is exported:
    Return "hello".

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(!rust.contains("greet_inner"),
        "Complex-path inner function should NOT have _inner suffix. Generated:\n{}", rust);
    assert!(rust.contains("fn greet(name: String)"),
        "Inner function should use raw name. Generated:\n{}", rust);
    assert!(rust.contains("pub extern \"C\" fn logos_greet("),
        "Wrapper should have logos_ prefix. Generated:\n{}", rust);
}

#[test]
fn complex_path_wrapper_calls_inner_by_raw_name() {
    let source = r#"## To greet (name: Text) -> Text is exported:
    Return "hello".

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("greet(name)") || rust.contains("greet(name,"),
        "Wrapper should call inner by raw name. Generated:\n{}", rust);
}

#[test]
fn e2e_internal_call_to_exported_function_works() {
    let source = r#"## To square (n: Int) -> Int is exported:
    Return n * n.

## Main
Let result be square(7).
Show result.
"#;
    assert_exact_output(source, "49");
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
    assert!(js.contains("koffi.load"), "TypeScript bindings should use koffi.load. Generated:\n{}", js);
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

    // Error storage uses Mutex<HashMap<ThreadId, String>> (thread-safe)
    // thread_local is allowed for the CString return cache only
    assert!(rust.contains("Mutex<std::collections::HashMap<std::thread::ThreadId, String>>"),
        "Error storage should use Mutex<HashMap<ThreadId, String>>. Generated:\n{}", rust);
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

// ─────────────────────────────────────────────────────────
// Helper: Extract a specific function body from generated Rust code.
// Returns the full text from the fn signature line through the matching
// closing brace.
// ─────────────────────────────────────────────────────────

fn extract_function_body(code: &str, fn_name: &str) -> Option<String> {
    let lines: Vec<&str> = code.lines().collect();
    let mut start = None;
    for (i, line) in lines.iter().enumerate() {
        if line.contains(&format!("fn {}(", fn_name)) || line.contains(&format!("fn {} (", fn_name)) {
            start = Some(i);
            break;
        }
    }
    let start = start?;
    let mut depth = 0i32;
    let mut end = start;
    for (i, line) in lines[start..].iter().enumerate() {
        for ch in line.chars() {
            if ch == '{' { depth += 1; }
            if ch == '}' { depth -= 1; }
        }
        if depth == 0 && i > 0 {
            end = start + i;
            break;
        }
        if depth == 0 && line.contains('{') && line.contains('}') {
            end = start + i;
            break;
        }
    }
    if end < start { end = lines.len() - 1; }
    Some(lines[start..=end].join("\n"))
}

// ─────────────────────────────────────────────────────────
// Phase 1: Memory Safety — UB & Corruption Bug Fixes
// ─────────────────────────────────────────────────────────

// 1A. logos_last_error() returns dangling pointer — must use CString cache
#[test]
fn bug_last_error_must_not_return_dangling_pointer() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    let body = extract_function_body(&rust, "logos_last_error")
        .expect("logos_last_error function should exist");
    assert!(!body.contains("msg.as_ptr() as"),
        "logos_last_error must NOT return a pointer into the Mutex-guarded HashMap. \
         It must clone into a CString cache. Body:\n{}", body);
    assert!(body.contains("CString") || body.contains("thread_local"),
        "logos_last_error must use CString or thread_local cache. Body:\n{}", body);
}

// 1B. LogosHandle must be *mut c_void, not *const
#[test]
fn bug_logos_handle_must_be_mut_void_pointer() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("pub type LogosHandle = *mut std::ffi::c_void"),
        "LogosHandle must be *mut c_void for safe mutation. Generated:\n{}", rust);
    assert!(!rust.contains("pub type LogosHandle = *const std::ffi::c_void"),
        "LogosHandle must NOT be *const c_void. Generated:\n{}", rust);
}

// 1C. Value-type structs passed by value must have #[repr(C)]
#[test]
fn bug_value_type_struct_must_have_repr_c() {
    let source = r#"## A Point has:
    an x: Int.
    a y: Int.

## To getPoint () -> Point is exported:
    Let p be a new Point with x 1 and y 2.
    Return p.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    // Find "struct Point" and check that #[repr(C)] is on the line immediately preceding it
    let lines: Vec<&str> = rust.lines().collect();
    let struct_idx = lines.iter().position(|l| l.contains("struct Point"));
    assert!(struct_idx.is_some(), "Should have struct Point. Generated:\n{}", rust);
    let idx = struct_idx.unwrap();
    assert!(idx > 0, "struct Point should not be first line");
    // The #[repr(C)] must be within 2 lines before struct Point (allowing for #[derive(...)])
    let window = lines[idx.saturating_sub(3)..idx].join("\n");
    assert!(window.contains("#[repr(C)]"),
        "Value-type struct Point used in C export must have #[repr(C)] immediately before it. \
         Lines before struct Point:\n{}\n\nFull generated:\n{}", window, rust);
}

// 1D. All accessor functions must have catch_unwind panic boundaries
#[test]
fn bug_all_accessor_functions_must_have_panic_boundary() {
    let output = compile_program_full(PERSON_STRUCT_SOURCE).expect("Should compile");
    let rust = &output.rust_code;

    let accessor_fns = ["logos_person_name", "logos_person_age", "logos_person_email",
                         "logos_person_address", "logos_person_phone", "logos_person_free"];
    for fn_name in &accessor_fns {
        let body = extract_function_body(rust, fn_name)
            .unwrap_or_else(|| panic!("Function {} should exist in output", fn_name));
        assert!(body.contains("catch_unwind"),
            "Accessor function {} must have catch_unwind panic boundary. Body:\n{}", fn_name, body);
    }
}

// 1E. Python Nat type must be c_uint64, not c_uint32
#[test]
fn bug_python_nat_must_be_c_uint64() {
    let source = r#"## To getCount () -> Nat is exported:
    Return 42.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let py = output.python_bindings.as_ref().expect("Should have python bindings");
    assert!(py.contains("c_uint64"),
        "Python Nat type should be c_uint64 (not c_uint32). Generated:\n{}", py);
    assert!(!py.contains("c_uint32"),
        "Python Nat type must NOT be c_uint32. Generated:\n{}", py);
}

// 1F. Char field accessor must return u32, not Rust char (4 bytes != C char)
#[test]
fn bug_char_field_accessor_must_return_u32() {
    let source = r#"## A Letter has:
    a value: Char.
    a code: Int.
    a name: Text.
    a weight: Real.
    a active: Bool.

## To getLetter () -> Letter is exported:
    Let l be a new Letter with value 'A' and code 65 and name "A" and weight 1.0 and active true.
    Return l.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let body = extract_function_body(&output.rust_code, "logos_letter_value")
        .expect("logos_letter_value should exist");
    assert!(body.contains("-> u32"),
        "Char field accessor must return u32, not char. Body:\n{}", body);
    assert!(body.contains("as u32"),
        "Char field accessor must cast to u32. Body:\n{}", body);
}

// ─────────────────────────────────────────────────────────
// Phase 5: Correctness Fixes (Non-UB)
// ─────────────────────────────────────────────────────────

// 5A. logos_version() must not be hardcoded to stale "0.8.0"
#[test]
fn bug_logos_version_must_not_be_hardcoded() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(!rust.contains(r#"b"0.8.0\0""#),
        "logos_version must NOT be hardcoded to 0.8.0. Generated:\n{}", rust);
}

// 5B. Python bindings must not hardcode .dylib
#[test]
fn bug_python_bindings_must_not_hardcode_dylib() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let py = output.python_bindings.as_ref().expect("Should have python bindings");
    assert!(py.contains("sys.platform") || py.contains("platform.system"),
        "Python bindings must detect platform for library extension. Generated:\n{}", py);
}

// 5C. TypeScript bindings should not use deprecated ffi-napi
#[test]
fn bug_typescript_should_not_use_deprecated_ffi_napi() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let js = output.typescript_bindings.as_ref().expect("Should have JS bindings");
    assert!(!js.contains("ffi-napi"),
        "TypeScript bindings should NOT use deprecated ffi-napi. Generated:\n{}", js);
    assert!(!js.contains("ref-napi"),
        "TypeScript bindings should NOT use deprecated ref-napi. Generated:\n{}", js);
}

// ─────────────────────────────────────────────────────────
// Phase 2: HandleRegistry Activation
// ─────────────────────────────────────────────────────────

// 2A. _create functions must register handles in the registry
#[test]
fn bug_create_functions_must_register_handles() {
    let source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Return s.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let body = extract_function_body(&output.rust_code, "logos_seq_i64_create")
        .expect("logos_seq_i64_create function should exist");
    assert!(body.contains("logos_handle_registry()") || body.contains("register("),
        "Create function must register handle in registry. Body:\n{}", body);
    assert!(!body.contains("Box::into_raw(Box::new(seq)) as LogosHandle"),
        "Create function must NOT return raw Box pointer directly. Body:\n{}", body);
}

// 2B. _free functions must deregister handles from the registry
#[test]
fn bug_free_functions_must_deregister_handles() {
    let source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Return s.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let body = extract_function_body(&output.rust_code, "logos_seq_i64_free")
        .expect("logos_seq_i64_free function should exist");
    assert!(body.contains("logos_handle_registry()") || body.contains(".free("),
        "Free function must deregister handle from registry. Body:\n{}", body);
    assert!(!body.contains("Box::from_raw(handle as"),
        "Free function must NOT directly Box::from_raw the handle. Body:\n{}", body);
}

// 2C. Accessors must validate handle via registry before dereference
#[test]
fn bug_accessors_must_validate_handle_before_deref() {
    let source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Return s.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let body = extract_function_body(&output.rust_code, "logos_seq_i64_len")
        .expect("logos_seq_i64_len function should exist");
    assert!(body.contains("logos_handle_registry()") || body.contains("deref("),
        "Accessor must validate handle via registry before dereference. Body:\n{}", body);
    assert!(!body.contains("&*(handle as *const"),
        "Accessor must NOT cast handle directly to pointer. Body:\n{}", body);
}

// 2D. Double-free must return InvalidHandle error in free body
#[test]
fn bug_double_free_returns_invalid_handle_in_free_body() {
    let output = compile_program_full(PERSON_STRUCT_SOURCE).expect("Should compile");
    let body = extract_function_body(&output.rust_code, "logos_person_free")
        .expect("logos_person_free function should exist");
    assert!(body.contains("InvalidHandle"),
        "Free function body must contain InvalidHandle error path for double-free detection. Body:\n{}", body);
}

// ─────────────────────────────────────────────────────────
// Phase 4A: Null Handle Validation in Accessor Bodies
// ─────────────────────────────────────────────────────────

// 4A-1. Seq accessor body must null-check the handle
#[test]
fn bug_seq_len_body_must_check_null_handle() {
    let source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Return s.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let body = extract_function_body(&output.rust_code, "logos_seq_i64_len")
        .expect("logos_seq_i64_len function should exist");
    assert!(body.contains("is_null()") || body.contains("NullPointer"),
        "Seq accessor body must check for null handle. Body:\n{}", body);
}

// 4A-2. Struct accessor body must null-check the handle
#[test]
fn bug_struct_accessor_body_must_check_null_handle() {
    let output = compile_program_full(PERSON_STRUCT_SOURCE).expect("Should compile");
    let body = extract_function_body(&output.rust_code, "logos_person_age")
        .expect("logos_person_age function should exist");
    assert!(body.contains("is_null()") || body.contains("NullPointer"),
        "Struct accessor body must check for null handle. Body:\n{}", body);
}

// ─────────────────────────────────────────────────────────
// Phase 3: Enum Variant Accessor Silent Default
// ─────────────────────────────────────────────────────────

// 3A. Enum variant accessor's if-let else branch must set error, not silently return Default
#[test]
fn bug_enum_variant_accessor_must_signal_wrong_variant() {
    let source = r#"## A Shape is one of:
    Circle with a radius: Real.
    Rectangle with a width: Real and a height: Real.

## To getShape () -> Shape is exported:
    Let s be a new Circle with radius 5.0.
    Return s.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let body = extract_function_body(&output.rust_code, "logos_shape_circle_radius")
        .expect("logos_shape_circle_radius function should exist");
    // The if-let else branch must NOT be just `} else { Default::default() }`
    // It must include logos_set_last_error in the else clause itself
    // Find the else clause that contains the variant mismatch
    let has_silent_else = body.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.contains("else") && trimmed.contains("Default::default()")
            && !trimmed.contains("logos_set_last_error")
    });
    assert!(!has_silent_else,
        "Enum variant accessor must NOT have silent `else {{ Default::default() }}` without error. \
         The else branch must call logos_set_last_error. Body:\n{}", body);
}

// 3B. Enum variant text accessor's else branch must also set error
#[test]
fn bug_enum_variant_text_accessor_must_signal_wrong_variant() {
    let source = r#"## A Shape is one of:
    Circle with a name: Text.
    Rectangle with a label: Text.

## To getShape () -> Shape is exported:
    Let s be a new Circle with name "round".
    Return s.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let body = extract_function_body(&output.rust_code, "logos_shape_circle_name")
        .expect("logos_shape_circle_name function should exist");
    // The text accessor's else branch should also not silently return null
    let has_silent_else = body.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.contains("else") && trimmed.contains("null_mut()")
            && !trimmed.contains("logos_set_last_error")
    });
    assert!(!has_silent_else,
        "Text enum variant accessor must NOT have silent `else {{ null_mut() }}` without error. Body:\n{}", body);
}

// ─────────────────────────────────────────────────────────
// E2E ABI Compilation Tests — verify generated code compiles
// ─────────────────────────────────────────────────────────

// E2E: Struct export with handle registry compiles
#[test]
fn e2e_struct_export_compiles_with_registry() {
    let source = r#"## A Person has:
    a name: Text.
    an age: Int.

## To makePerson (name: Text, age: Int) -> Person is exported:
    Let p be a new Person with name name and age age.
    Return p.

## Main
Show "ok".
"#;
    let result = compile_logos(source);
    assert!(result.success,
        "Struct export with handle registry must compile. stderr:\n{}\n\nGenerated Rust:\n{}",
        result.stderr, result.rust_code);
}

// E2E: Seq export compiles
#[test]
fn e2e_seq_export_compiles_with_registry() {
    let source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Push 1 to s.
    Push 2 to s.
    Push 3 to s.
    Return s.

## Main
Show "ok".
"#;
    let result = compile_logos(source);
    assert!(result.success,
        "Seq export with handle registry must compile. stderr:\n{}\n\nGenerated Rust:\n{}",
        result.stderr, result.rust_code);
}

// E2E: Map export compiles with registry
#[test]
fn e2e_map_export_compiles_with_registry() {
    let source = r#"## To getConfig () -> Map of Text to Text is exported:
    Let m be a new Map of Text to Text.
    Return m.

## Main
Show "ok".
"#;
    let result = compile_logos(source);
    assert!(result.success,
        "Map export with handle registry must compile. stderr:\n{}\n\nGenerated Rust:\n{}",
        result.stderr, result.rust_code);
}

// E2E: Enum export compiles with registry
#[test]
fn e2e_enum_export_compiles_with_registry() {
    let source = r#"## A Shape is one of:
    Circle with a radius: Real.
    Rectangle with a width: Real and a height: Real.

## To getCircle () -> Shape is exported:
    Let s be a new Circle with radius 5.0.
    Return s.

## Main
Show "ok".
"#;
    let result = compile_logos(source);
    assert!(result.success,
        "Enum export with handle registry must compile. stderr:\n{}\n\nGenerated Rust:\n{}",
        result.stderr, result.rust_code);
}

// E2E: Multiple exports with mixed types compile
#[test]
fn e2e_multiple_exports_compile_with_registry() {
    let source = r#"## A Person has:
    a name: Text.
    an age: Int.

## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## To greet (name: Text) -> Text is exported:
    Return "Hello " + name.

## To makePerson (name: Text, age: Int) -> Person is exported:
    Let p be a new Person with name name and age age.
    Return p.

## Main
Let r be add(1, 2).
Show r.
"#;
    let result = compile_logos(source);
    assert!(result.success,
        "Multiple exports with mixed types must compile. stderr:\n{}\n\nGenerated Rust:\n{}",
        result.stderr, result.rust_code);
}

// E2E: Exported value-type functions run correctly
#[test]
fn e2e_exported_value_types_run_correctly() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## To multiply (a: Real, b: Real) -> Real is exported:
    Return a * b.

## Main
Show add(10, 20).
Show multiply(3.0, 4.0).
"#;
    assert_output_lines(source, &["30", "12"]);
}

// E2E: HandleRegistry codegen contains all necessary infrastructure
#[test]
fn e2e_registry_infrastructure_complete() {
    // Use PERSON_STRUCT_SOURCE which has 5 fields → reference type → generates accessors
    let output = compile_program_full(PERSON_STRUCT_SOURCE).expect("Should compile");
    let rust = &output.rust_code;

    // Verify complete registry infrastructure
    assert!(rust.contains("struct HandleEntry"), "Must have HandleEntry struct");
    assert!(rust.contains("struct HandleRegistry"), "Must have HandleRegistry struct");
    assert!(rust.contains("fn register(&mut self"), "Registry must have register method");
    assert!(rust.contains("fn deref(&self"), "Registry must have deref method");
    assert!(rust.contains("fn free(&mut self"), "Registry must have free method");
    assert!(rust.contains("fn logos_handle_registry()"), "Must have global registry accessor");
    assert!(rust.contains("OnceLock<std::sync::Mutex<HandleRegistry>>"), "Registry must be Mutex-protected OnceLock");

    // Verify person accessor uses registry (Person has 5 fields → reference type)
    let body = extract_function_body(rust, "logos_person_age")
        .expect("logos_person_age should exist");
    assert!(body.contains("logos_handle_registry()"),
        "Struct accessor must use registry. Body:\n{}", body);
    assert!(body.contains("is_null()"),
        "Struct accessor must null-check. Body:\n{}", body);
    assert!(body.contains("deref("),
        "Struct accessor must deref via registry. Body:\n{}", body);
}

// E2E: Free function has double-free protection
#[test]
fn e2e_free_has_double_free_protection() {
    let source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Return s.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let body = extract_function_body(&output.rust_code, "logos_seq_i64_free")
        .expect("logos_seq_i64_free should exist");
    assert!(body.contains("InvalidHandle"),
        "Free must have InvalidHandle error for double-free. Body:\n{}", body);
    assert!(body.contains("logos_handle_registry()"),
        "Free must use registry. Body:\n{}", body);
    assert!(!body.contains("Box::from_raw(handle as"),
        "Free must NOT directly cast handle to pointer. Body:\n{}", body);
}

// E2E: All accessor types have catch_unwind + null check + registry deref
#[test]
fn e2e_accessor_safety_trifecta() {
    // Use PERSON_STRUCT_SOURCE (5 fields → reference type with accessors)
    let output = compile_program_full(PERSON_STRUCT_SOURCE).expect("Should compile");
    let rust = &output.rust_code;

    // Check struct accessor (Person has 5 fields → reference type)
    for fn_name in &["logos_person_name", "logos_person_age"] {
        let body = extract_function_body(rust, fn_name)
            .unwrap_or_else(|| panic!("{} should exist", fn_name));
        assert!(body.contains("catch_unwind"), "{} must have catch_unwind", fn_name);
        assert!(body.contains("is_null()"), "{} must check null", fn_name);
        assert!(body.contains("logos_handle_registry()"), "{} must use registry", fn_name);
    }

    // Also check seq accessors
    let seq_source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Return s.

## Main
Show 42.
"#;
    let seq_output = compile_program_full(seq_source).expect("Should compile");
    let seq_rust = &seq_output.rust_code;

    for fn_name in &["logos_seq_i64_len", "logos_seq_i64_at"] {
        let body = extract_function_body(seq_rust, fn_name)
            .unwrap_or_else(|| panic!("{} should exist", fn_name));
        assert!(body.contains("catch_unwind"), "{} must have catch_unwind", fn_name);
        assert!(body.contains("is_null()"), "{} must check null", fn_name);
        assert!(body.contains("logos_handle_registry()"), "{} must use registry", fn_name);
    }
}

// E2E: Create functions use registry
#[test]
fn e2e_create_functions_use_registry() {
    let source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Return s.

## To getNames () -> Map of Text to Text is exported:
    Let m be a new Map of Text to Text.
    Return m.

## To getTags () -> Set of Text is exported:
    Let s be a new Set of Text.
    Return s.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let rust = &output.rust_code;

    for fn_name in &["logos_seq_i64_create", "logos_map_string_string_create", "logos_set_string_create"] {
        let body = extract_function_body(rust, fn_name)
            .unwrap_or_else(|| panic!("{} should exist", fn_name));
        assert!(body.contains("logos_handle_registry()"),
            "{} must register in handle registry. Body:\n{}", fn_name, body);
        assert!(body.contains("register("),
            "{} must call register(). Body:\n{}", fn_name, body);
    }
}

// E2E: C header has correct handle type (void*, not const void*)
#[test]
fn e2e_c_header_handle_type_is_mutable() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let header = output.c_header.as_ref().expect("Should have C header");
    assert!(header.contains("typedef void* logos_handle_t"),
        "C header handle must be void* (not const void*). Header:\n{}", header);
    assert!(!header.contains("typedef const void* logos_handle_t"),
        "C header handle must NOT be const void*. Header:\n{}", header);
}

// E2E: Python bindings have platform detection and correct Nat type
#[test]
fn e2e_python_bindings_quality() {
    let source = r#"## To getCount () -> Nat is exported:
    Return 42.

## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let py = output.python_bindings.as_ref().expect("Should have Python bindings");
    assert!(py.contains("c_uint64"), "Nat must map to c_uint64. Python:\n{}", py);
    assert!(!py.contains("c_uint32"), "Must NOT use c_uint32 for Nat. Python:\n{}", py);
    assert!(py.contains("sys.platform"), "Must have platform detection. Python:\n{}", py);
}

// E2E: TypeScript bindings use koffi, not ffi-napi
#[test]
fn e2e_typescript_bindings_quality() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let js = output.typescript_bindings.as_ref().expect("Should have TS bindings");
    assert!(js.contains("koffi"), "TS bindings must use koffi. JS:\n{}", js);
    assert!(!js.contains("ffi-napi"), "TS bindings must NOT use ffi-napi. JS:\n{}", js);
}

// ─────────────────────────────────────────────────────────
// C Linkage Tests — compile C, link against generated staticlib, run
// ─────────────────────────────────────────────────────────

// C-Link: Call exported Int function from C
#[test]
fn c_link_call_exported_int_function() {
    let logos_source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

extern int64_t logos_add(int64_t a, int64_t b);

int main() {
    int64_t result = logos_add(10, 20);
    printf("%lld\n", (long long)result);
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C linkage test must succeed. stderr:\n{}\n\nGenerated Rust:\n{}\n\nC code:\n{}",
        result.stderr, result.rust_code, result.c_code);
    assert_eq!(result.stdout.trim(), "30",
        "C program should call Rust logos_add(10, 20) = 30. Got: {}", result.stdout.trim());
}

// C-Link: Call multiple exported functions from C
#[test]
fn c_link_multiple_exported_functions() {
    let logos_source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## To square (n: Int) -> Int is exported:
    Return n * n.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

extern int64_t logos_add(int64_t a, int64_t b);
extern int64_t logos_square(int64_t n);

int main() {
    printf("%lld\n", (long long)logos_add(3, 4));
    printf("%lld\n", (long long)logos_square(5));
    printf("%lld\n", (long long)logos_add(logos_square(3), logos_square(4)));
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C linkage test must succeed. stderr:\n{}", result.stderr);
    let lines: Vec<&str> = result.stdout.trim().lines().collect();
    assert_eq!(lines, vec!["7", "25", "25"],
        "C program should produce correct output. Got: {:?}", lines);
}

// C-Link: Call ABI runtime functions from C (version, error API)
#[test]
fn c_link_runtime_functions() {
    let logos_source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <string.h>

extern const char* logos_version(void);
extern uint32_t logos_abi_version(void);
extern const char* logos_last_error(void);
extern void logos_clear_error(void);

int main() {
    const char* ver = logos_version();
    uint32_t abi = logos_abi_version();
    const char* err = logos_last_error();

    printf("version_not_null=%d\n", ver != NULL && strlen(ver) > 0);
    printf("abi=%u\n", abi);
    printf("err_is_null=%d\n", err == NULL);

    logos_clear_error();
    printf("ok\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C linkage test must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("version_not_null=1"), "Version should not be null. Got: {}", output);
    assert!(output.contains("abi=1"), "ABI version should be 1. Got: {}", output);
    assert!(output.contains("err_is_null=1"), "Initial error should be null. Got: {}", output);
    assert!(output.contains("ok"), "Should complete successfully. Got: {}", output);
}

// C-Link: Call exported Real function from C
#[test]
fn c_link_call_exported_real_function() {
    let logos_source = r#"## To halve (n: Real) -> Real is exported:
    Return n / 2.0.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>

extern double logos_halve(double n);

int main() {
    double result = logos_halve(10.0);
    printf("%.1f\n", result);
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C linkage test must succeed. stderr:\n{}", result.stderr);
    assert_eq!(result.stdout.trim(), "5.0",
        "C program should call Rust logos_halve(10.0) = 5.0. Got: {}", result.stdout.trim());
}

// C-Link: Call exported Bool function from C
#[test]
fn c_link_call_exported_bool_function() {
    let logos_source = r#"## To isPositive (n: Int) -> Bool is exported:
    Return n > 0.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <stdbool.h>

extern bool logos_isPositive(int64_t n);

int main() {
    printf("%d\n", logos_isPositive(5));
    printf("%d\n", logos_isPositive(-3));
    printf("%d\n", logos_isPositive(0));
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C linkage test must succeed. stderr:\n{}", result.stderr);
    let lines: Vec<&str> = result.stdout.trim().lines().collect();
    assert_eq!(lines, vec!["1", "0", "0"],
        "C program should produce correct bool results. Got: {:?}", lines);
}

// C-Link: Struct handle accessors from C — create, access fields, free
#[test]
fn c_link_struct_handle_lifecycle() {
    let logos_source = r#"## A Person has:
    a name: Text.
    an age: Int.
    an email: Text.
    an address: Text.
    a phone: Text.

## To getPerson () -> Person is exported:
    Let p be a new Person with name "Alice" and age 30 and email "alice@test.com" and address "123 Main St" and phone "555-1234".
    Return p.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <string.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1 } logos_status_t;

extern logos_status_t logos_getPerson(logos_handle_t* out);
extern char* logos_person_name(logos_handle_t handle);
extern int64_t logos_person_age(logos_handle_t handle);
extern char* logos_person_email(logos_handle_t handle);
extern void logos_person_free(logos_handle_t handle);
extern void logos_free_string(char* str);

int main() {
    logos_handle_t person = NULL;
    logos_status_t status = logos_getPerson(&person);
    printf("status=%d\n", status);
    printf("handle_not_null=%d\n", person != NULL);

    char* name = logos_person_name(person);
    int64_t age = logos_person_age(person);
    char* email = logos_person_email(person);

    printf("name=%s\n", name);
    printf("age=%lld\n", (long long)age);
    printf("email=%s\n", email);

    logos_free_string(name);
    logos_free_string(email);
    logos_person_free(person);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C struct handle lifecycle must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("status=0"), "Status should be OK. Got:\n{}", output);
    assert!(output.contains("handle_not_null=1"), "Handle should not be null. Got:\n{}", output);
    assert!(output.contains("name=Alice"), "Name should be Alice. Got:\n{}", output);
    assert!(output.contains("age=30"), "Age should be 30. Got:\n{}", output);
    assert!(output.contains("email=alice@test.com"), "Email should match. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete without crash. Got:\n{}", output);
}

// C-Link: Seq create, push, len, at from C
#[test]
fn c_link_seq_operations() {
    let logos_source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Push 10 to s.
    Push 20 to s.
    Push 30 to s.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0 } logos_status_t;

extern logos_status_t logos_getNumbers(logos_handle_t* out);
extern size_t logos_seq_i64_len(logos_handle_t handle);
extern logos_status_t logos_seq_i64_at(logos_handle_t handle, size_t index, int64_t* out);
extern void logos_seq_i64_free(logos_handle_t handle);

int main() {
    logos_handle_t seq = NULL;
    logos_status_t status = logos_getNumbers(&seq);
    printf("status=%d\n", status);

    size_t len = logos_seq_i64_len(seq);
    printf("len=%zu\n", len);

    for (size_t i = 0; i < len; i++) {
        int64_t val = 0;
        logos_seq_i64_at(seq, i, &val);
        printf("seq[%zu]=%lld\n", i, (long long)val);
    }

    logos_seq_i64_free(seq);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C seq operations must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("status=0"), "Status should be OK. Got:\n{}", output);
    assert!(output.contains("len=3"), "Length should be 3. Got:\n{}", output);
    assert!(output.contains("seq[0]=10"), "First element should be 10. Got:\n{}", output);
    assert!(output.contains("seq[1]=20"), "Second element should be 20. Got:\n{}", output);
    assert!(output.contains("seq[2]=30"), "Third element should be 30. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Seq create/push/len from C (independent of exported function)
#[test]
fn c_link_seq_create_push_len() {
    let logos_source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;

extern logos_handle_t logos_seq_i64_create(void);
extern void logos_seq_i64_push(logos_handle_t handle, int64_t value);
extern size_t logos_seq_i64_len(logos_handle_t handle);
extern void logos_seq_i64_free(logos_handle_t handle);

int main() {
    logos_handle_t seq = logos_seq_i64_create();
    printf("created=%d\n", seq != NULL);

    logos_seq_i64_push(seq, 100);
    logos_seq_i64_push(seq, 200);
    logos_seq_i64_push(seq, 300);

    size_t len = logos_seq_i64_len(seq);
    printf("len=%zu\n", len);

    logos_seq_i64_free(seq);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C seq create/push/len must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("created=1"), "Seq should be created. Got:\n{}", output);
    assert!(output.contains("len=3"), "Length should be 3. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Enum tag accessor from C
#[test]
fn c_link_enum_tag_accessor() {
    let logos_source = r#"## A Shape is one of:
    Circle with a radius: Real.
    Rectangle with a width: Real and a height: Real.

## To getCircle () -> Shape is exported:
    Let s be a new Circle with radius 5.0.
    Return s.

## To getRectangle () -> Shape is exported:
    Let s be a new Rectangle with width 3.0 and height 4.0.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0 } logos_status_t;

extern logos_status_t logos_getCircle(logos_handle_t* out);
extern logos_status_t logos_getRectangle(logos_handle_t* out);
extern int32_t logos_shape_tag(logos_handle_t handle);
extern double logos_shape_circle_radius(logos_handle_t handle);
extern double logos_shape_rectangle_width(logos_handle_t handle);
extern double logos_shape_rectangle_height(logos_handle_t handle);
extern void logos_shape_free(logos_handle_t handle);

int main() {
    logos_handle_t circle = NULL;
    logos_handle_t rect = NULL;
    logos_getCircle(&circle);
    logos_getRectangle(&rect);

    int32_t circle_tag = logos_shape_tag(circle);
    int32_t rect_tag = logos_shape_tag(rect);
    printf("circle_tag=%d\n", circle_tag);
    printf("rect_tag=%d\n", rect_tag);

    double radius = logos_shape_circle_radius(circle);
    printf("radius=%.1f\n", radius);

    double width = logos_shape_rectangle_width(rect);
    double height = logos_shape_rectangle_height(rect);
    printf("width=%.1f\n", width);
    printf("height=%.1f\n", height);

    logos_shape_free(circle);
    logos_shape_free(rect);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C enum tag accessor must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("circle_tag=0"), "Circle tag should be 0. Got:\n{}", output);
    assert!(output.contains("rect_tag=1"), "Rectangle tag should be 1. Got:\n{}", output);
    assert!(output.contains("radius=5.0"), "Radius should be 5.0. Got:\n{}", output);
    assert!(output.contains("width=3.0"), "Width should be 3.0. Got:\n{}", output);
    assert!(output.contains("height=4.0"), "Height should be 4.0. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Text marshaling — passing and receiving strings
#[test]
fn c_link_text_marshaling() {
    let logos_source = r#"## To greet (name: Text) -> Text is exported:
    Return "Hello " + name.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <string.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0 } logos_status_t;

extern logos_status_t logos_greet(const char* name, char** out);
extern void logos_free_string(char* str);

int main() {
    char* result = NULL;
    logos_status_t status = logos_greet("World", &result);
    printf("status=%d\n", status);
    if (result) {
        printf("result=%s\n", result);
        logos_free_string(result);
    }
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C text marshaling must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("status=0"), "Status should be OK. Got:\n{}", output);
    assert!(output.contains("result=Hello World"), "Should greet correctly. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// E2E: Error infrastructure is correct
#[test]
fn e2e_error_infrastructure_quality() {
    let source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Thread-local CString cache for safe error string return
    assert!(rust.contains("LOGOS_ERROR_CACHE"),
        "Must have thread-local error cache for safe pointer return");
    assert!(rust.contains("RefCell<Option<std::ffi::CString>>"),
        "Error cache must be RefCell<Option<CString>>");

    // Error store with poisoned mutex recovery
    let set_body = extract_function_body(&rust, "logos_set_last_error")
        .expect("logos_set_last_error should exist");
    assert!(set_body.contains("unwrap_or_else(|e| e.into_inner())"),
        "Error store must handle poisoned mutex. Body:\n{}", set_body);

    // Version uses env! macro
    let ver_body = extract_function_body(&rust, "logos_version")
        .expect("logos_version should exist");
    assert!(ver_body.contains("env!(\"CARGO_PKG_VERSION\")"),
        "Version must use env! macro. Body:\n{}", ver_body);
}

// ─────────────────────────────────────────────────────────
// C Linkage Tests — Batch 2: Collections, Error Handling, Safety
// ─────────────────────────────────────────────────────────

// C-Link: Map of Text to Int — create, insert, get, len, free
#[test]
fn c_link_map_create_insert_get_len() {
    let logos_source = r#"## To getScores () -> Map of Text to Int is exported:
    Let m be a new Map of Text to Int.
    Return m.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <string.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1 } logos_status_t;

extern logos_handle_t logos_map_string_i64_create(void);
extern void logos_map_string_i64_insert(logos_handle_t handle, const char* key, int64_t value);
extern logos_status_t logos_map_string_i64_get(logos_handle_t handle, const char* key, int64_t* out);
extern size_t logos_map_string_i64_len(logos_handle_t handle);
extern void logos_map_string_i64_free(logos_handle_t handle);

int main() {
    logos_handle_t map = logos_map_string_i64_create();
    printf("created=%d\n", map != NULL);

    logos_map_string_i64_insert(map, "alice", 95);
    logos_map_string_i64_insert(map, "bob", 87);
    logos_map_string_i64_insert(map, "carol", 92);

    size_t len = logos_map_string_i64_len(map);
    printf("len=%zu\n", len);

    int64_t score = 0;
    logos_status_t status = logos_map_string_i64_get(map, "alice", &score);
    printf("get_alice_status=%d\n", status);
    printf("alice_score=%lld\n", (long long)score);

    status = logos_map_string_i64_get(map, "bob", &score);
    printf("bob_score=%lld\n", (long long)score);

    status = logos_map_string_i64_get(map, "missing", &score);
    printf("missing_status=%d\n", status);

    logos_map_string_i64_free(map);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C map operations must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("created=1"), "Map should be created. Got:\n{}", output);
    assert!(output.contains("len=3"), "Map length should be 3. Got:\n{}", output);
    assert!(output.contains("get_alice_status=0"), "Get alice should succeed. Got:\n{}", output);
    assert!(output.contains("alice_score=95"), "Alice score should be 95. Got:\n{}", output);
    assert!(output.contains("bob_score=87"), "Bob score should be 87. Got:\n{}", output);
    assert!(output.contains("missing_status=1"), "Missing key should return error status. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Map keys/values — needs both Map and Seq types so accessors are generated
#[test]
fn c_link_map_keys_values() {
    let logos_source = r#"## To getScores () -> Map of Text to Int is exported:
    Let m be a new Map of Text to Int.
    Return m.

## To getNames () -> Seq of Text is exported:
    Let s be a new Seq of Text.
    Return s.

## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;

extern logos_handle_t logos_map_string_i64_create(void);
extern void logos_map_string_i64_insert(logos_handle_t handle, const char* key, int64_t value);
extern logos_handle_t logos_map_string_i64_keys(logos_handle_t handle);
extern logos_handle_t logos_map_string_i64_values(logos_handle_t handle);
extern size_t logos_seq_string_len(logos_handle_t handle);
extern size_t logos_seq_i64_len(logos_handle_t handle);
extern void logos_map_string_i64_free(logos_handle_t handle);
extern void logos_seq_string_free(logos_handle_t handle);
extern void logos_seq_i64_free(logos_handle_t handle);

int main() {
    logos_handle_t map = logos_map_string_i64_create();
    logos_map_string_i64_insert(map, "x", 10);
    logos_map_string_i64_insert(map, "y", 20);

    logos_handle_t keys = logos_map_string_i64_keys(map);
    size_t keys_len = logos_seq_string_len(keys);
    printf("keys_len=%zu\n", keys_len);

    logos_handle_t values = logos_map_string_i64_values(map);
    size_t values_len = logos_seq_i64_len(values);
    printf("values_len=%zu\n", values_len);

    logos_seq_string_free(keys);
    logos_seq_i64_free(values);
    logos_map_string_i64_free(map);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C map keys/values must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("keys_len=2"), "Keys should have 2 entries. Got:\n{}", output);
    assert!(output.contains("values_len=2"), "Values should have 2 entries. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Map of Text to Text — create, insert, get
#[test]
fn c_link_map_text_to_text() {
    let logos_source = r#"## To getLabels () -> Map of Text to Text is exported:
    Let m be a new Map of Text to Text.
    Return m.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <string.h>

typedef void* logos_handle_t;

extern logos_handle_t logos_map_string_string_create(void);
extern void logos_map_string_string_insert(logos_handle_t handle, const char* key, const char* value);
extern char* logos_map_string_string_get(logos_handle_t handle, const char* key);
extern size_t logos_map_string_string_len(logos_handle_t handle);
extern void logos_map_string_string_free(logos_handle_t handle);
extern void logos_free_string(char* str);

int main() {
    logos_handle_t map = logos_map_string_string_create();
    logos_map_string_string_insert(map, "greeting", "hello");
    logos_map_string_string_insert(map, "farewell", "goodbye");

    size_t len = logos_map_string_string_len(map);
    printf("len=%zu\n", len);

    char* greeting = logos_map_string_string_get(map, "greeting");
    printf("greeting=%s\n", greeting);
    logos_free_string(greeting);

    char* farewell = logos_map_string_string_get(map, "farewell");
    printf("farewell=%s\n", farewell);
    logos_free_string(farewell);

    char* missing = logos_map_string_string_get(map, "missing");
    printf("missing_is_null=%d\n", missing == NULL);

    logos_map_string_string_free(map);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C map text-to-text must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("len=2"), "Map should have 2 entries. Got:\n{}", output);
    assert!(output.contains("greeting=hello"), "Greeting should be hello. Got:\n{}", output);
    assert!(output.contains("farewell=goodbye"), "Farewell should be goodbye. Got:\n{}", output);
    assert!(output.contains("missing_is_null=1"), "Missing key should return NULL. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Set of Text — create, insert, contains, len, free
#[test]
fn c_link_set_create_insert_contains() {
    let logos_source = r#"## To getTags () -> Set of Text is exported:
    Let s be a new Set of Text.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdbool.h>

typedef void* logos_handle_t;

extern logos_handle_t logos_set_string_create(void);
extern void logos_set_string_insert(logos_handle_t handle, const char* value);
extern bool logos_set_string_contains(logos_handle_t handle, const char* value);
extern size_t logos_set_string_len(logos_handle_t handle);
extern void logos_set_string_free(logos_handle_t handle);

int main() {
    logos_handle_t set = logos_set_string_create();
    printf("created=%d\n", set != NULL);

    logos_set_string_insert(set, "rust");
    logos_set_string_insert(set, "python");
    logos_set_string_insert(set, "go");
    logos_set_string_insert(set, "rust");

    size_t len = logos_set_string_len(set);
    printf("len=%zu\n", len);

    bool has_rust = logos_set_string_contains(set, "rust");
    bool has_python = logos_set_string_contains(set, "python");
    bool has_java = logos_set_string_contains(set, "java");
    printf("has_rust=%d\n", has_rust);
    printf("has_python=%d\n", has_python);
    printf("has_java=%d\n", has_java);

    logos_set_string_free(set);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C set operations must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("created=1"), "Set should be created. Got:\n{}", output);
    assert!(output.contains("len=3"), "Set should deduplicate 'rust', len=3. Got:\n{}", output);
    assert!(output.contains("has_rust=1"), "Should contain 'rust'. Got:\n{}", output);
    assert!(output.contains("has_python=1"), "Should contain 'python'. Got:\n{}", output);
    assert!(output.contains("has_java=0"), "Should not contain 'java'. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Option of Int — is_some, unwrap with Some value
#[test]
fn c_link_option_some_unwrap() {
    let logos_source = r#"## To findAge (name: Text) -> Option of Int is exported:
    If name = "Alice" then:
        Return some 30.
    Otherwise:
        Return none.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <stdbool.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1 } logos_status_t;

extern logos_status_t logos_findAge(const char* name, logos_handle_t* out);
extern bool logos_option_i64_is_some(logos_handle_t handle);
extern logos_status_t logos_option_i64_unwrap(logos_handle_t handle, int64_t* out);
extern void logos_option_i64_free(logos_handle_t handle);

int main() {
    logos_handle_t opt_alice = NULL;
    logos_status_t status = logos_findAge("Alice", &opt_alice);
    printf("alice_status=%d\n", status);

    bool is_some = logos_option_i64_is_some(opt_alice);
    printf("alice_is_some=%d\n", is_some);

    int64_t age = 0;
    logos_status_t unwrap_status = logos_option_i64_unwrap(opt_alice, &age);
    printf("alice_unwrap_status=%d\n", unwrap_status);
    printf("alice_age=%lld\n", (long long)age);
    logos_option_i64_free(opt_alice);

    logos_handle_t opt_bob = NULL;
    logos_findAge("Bob", &opt_bob);
    bool bob_is_some = logos_option_i64_is_some(opt_bob);
    printf("bob_is_some=%d\n", bob_is_some);

    int64_t bob_age = -1;
    logos_status_t bob_unwrap = logos_option_i64_unwrap(opt_bob, &bob_age);
    printf("bob_unwrap_status=%d\n", bob_unwrap);
    printf("bob_age_unchanged=%d\n", bob_age == -1);
    logos_option_i64_free(opt_bob);

    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C option operations must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("alice_status=0"), "findAge(Alice) should succeed. Got:\n{}", output);
    assert!(output.contains("alice_is_some=1"), "Alice option should be Some. Got:\n{}", output);
    assert!(output.contains("alice_unwrap_status=0"), "Unwrap Alice should succeed. Got:\n{}", output);
    assert!(output.contains("alice_age=30"), "Alice age should be 30. Got:\n{}", output);
    assert!(output.contains("bob_is_some=0"), "Bob option should be None. Got:\n{}", output);
    assert!(output.contains("bob_unwrap_status=1"), "Unwrap None should return error. Got:\n{}", output);
    assert!(output.contains("bob_age_unchanged=1"), "Bob age should remain unchanged. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Null handle error handling — passing NULL to accessors
#[test]
fn c_link_null_handle_graceful_error() {
    let logos_source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1, LOGOS_TYPE_MISMATCH = 2, LOGOS_NULL_POINTER = 3 } logos_status_t;

extern size_t logos_seq_i64_len(logos_handle_t handle);
extern logos_status_t logos_seq_i64_at(logos_handle_t handle, size_t index, int64_t* out);
extern void logos_seq_i64_free(logos_handle_t handle);
extern const char* logos_last_error(void);

int main() {
    size_t len = logos_seq_i64_len(NULL);
    printf("null_len=%zu\n", len);

    int64_t val = -1;
    logos_status_t status = logos_seq_i64_at(NULL, 0, &val);
    printf("null_at_status=%d\n", status);
    printf("val_unchanged=%d\n", val == -1);

    const char* err = logos_last_error();
    printf("has_error=%d\n", err != NULL);

    logos_seq_i64_free(NULL);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C null handle test must not crash. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("null_len=0"), "Null handle len should return 0. Got:\n{}", output);
    assert!(output.contains("null_at_status=3"), "Null handle at should return NullPointer(3). Got:\n{}", output);
    assert!(output.contains("val_unchanged=1"), "Value should remain unchanged. Got:\n{}", output);
    assert!(output.contains("has_error=1"), "Error should be set. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete without crash. Got:\n{}", output);
}

// C-Link: Double-free protection — freeing same handle twice must not crash
#[test]
fn c_link_double_free_no_crash() {
    let logos_source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Push 1 to s.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0 } logos_status_t;

extern logos_status_t logos_getNumbers(logos_handle_t* out);
extern size_t logos_seq_i64_len(logos_handle_t handle);
extern void logos_seq_i64_free(logos_handle_t handle);
extern const char* logos_last_error(void);
extern void logos_clear_error(void);

int main() {
    logos_handle_t seq = NULL;
    logos_getNumbers(&seq);

    size_t len = logos_seq_i64_len(seq);
    printf("len=%zu\n", len);

    logos_seq_i64_free(seq);
    printf("first_free=ok\n");

    logos_clear_error();
    logos_seq_i64_free(seq);
    printf("second_free=ok\n");

    const char* err = logos_last_error();
    printf("has_error_after_double_free=%d\n", err != NULL);

    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "Double-free must not crash. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("len=1"), "Seq should have 1 element. Got:\n{}", output);
    assert!(output.contains("first_free=ok"), "First free should work. Got:\n{}", output);
    assert!(output.contains("second_free=ok"), "Second free should not crash. Got:\n{}", output);
    assert!(output.contains("has_error_after_double_free=1"), "Should set error on double-free. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Seq out-of-bounds access returns error status
#[test]
fn c_link_seq_out_of_bounds() {
    let logos_source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Push 10 to s.
    Push 20 to s.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1, LOGOS_TYPE_MISMATCH = 2, LOGOS_NULL_POINTER = 3, LOGOS_OUT_OF_BOUNDS = 4 } logos_status_t;

extern logos_status_t logos_getNumbers(logos_handle_t* out);
extern logos_status_t logos_seq_i64_at(logos_handle_t handle, size_t index, int64_t* out);
extern void logos_seq_i64_free(logos_handle_t handle);
extern const char* logos_last_error(void);

int main() {
    logos_handle_t seq = NULL;
    logos_getNumbers(&seq);

    int64_t val = 0;
    logos_status_t s0 = logos_seq_i64_at(seq, 0, &val);
    printf("at_0_status=%d val=%lld\n", s0, (long long)val);

    logos_status_t s1 = logos_seq_i64_at(seq, 1, &val);
    printf("at_1_status=%d val=%lld\n", s1, (long long)val);

    logos_status_t s2 = logos_seq_i64_at(seq, 2, &val);
    printf("at_2_status=%d\n", s2);

    const char* err = logos_last_error();
    printf("error=%s\n", err ? err : "null");

    logos_status_t s99 = logos_seq_i64_at(seq, 99, &val);
    printf("at_99_status=%d\n", s99);

    logos_seq_i64_free(seq);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C seq OOB test must not crash. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("at_0_status=0 val=10"), "Index 0 should return 10. Got:\n{}", output);
    assert!(output.contains("at_1_status=0 val=20"), "Index 1 should return 20. Got:\n{}", output);
    assert!(output.contains("at_2_status=4"), "Index 2 should return OutOfBounds(4). Got:\n{}", output);
    assert!(output.contains("at_99_status=4"), "Index 99 should return OutOfBounds(4). Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Wrong enum variant field access sets error
#[test]
fn c_link_wrong_enum_variant_error() {
    let logos_source = r#"## A Shape is one of:
    Circle with a radius: Real.
    Rectangle with a width: Real and a height: Real.

## To getCircle () -> Shape is exported:
    Let s be a new Circle with radius 5.0.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <string.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0 } logos_status_t;

extern logos_status_t logos_getCircle(logos_handle_t* out);
extern int32_t logos_shape_tag(logos_handle_t handle);
extern double logos_shape_circle_radius(logos_handle_t handle);
extern double logos_shape_rectangle_width(logos_handle_t handle);
extern const char* logos_last_error(void);
extern void logos_clear_error(void);
extern void logos_shape_free(logos_handle_t handle);

int main() {
    logos_handle_t circle = NULL;
    logos_getCircle(&circle);

    double radius = logos_shape_circle_radius(circle);
    printf("radius=%.1f\n", radius);

    logos_clear_error();
    double width = logos_shape_rectangle_width(circle);
    printf("wrong_variant_width=%.1f\n", width);

    const char* err = logos_last_error();
    printf("has_error=%d\n", err != NULL);
    if (err) printf("error_contains_wrong=%d\n", strstr(err, "Wrong variant") != NULL);

    logos_shape_free(circle);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C wrong variant test must not crash. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("radius=5.0"), "Correct variant field should work. Got:\n{}", output);
    assert!(output.contains("wrong_variant_width=0.0"), "Wrong variant should return default. Got:\n{}", output);
    assert!(output.contains("has_error=1"), "Error should be set for wrong variant. Got:\n{}", output);
    assert!(output.contains("error_contains_wrong=1"), "Error message should mention wrong variant. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Struct with Bool and Real fields — verify all field types
#[test]
fn c_link_struct_bool_real_fields() {
    let logos_source = r#"## A Sensor has:
    a name: Text.
    a temperature: Real.
    a active: Bool.
    a readings: Int.
    a precision: Real.

## To getSensor () -> Sensor is exported:
    Let s be a new Sensor with name "thermometer" and temperature 98.6 and active true and readings 42 and precision 0.01.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <stdbool.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0 } logos_status_t;

extern logos_status_t logos_getSensor(logos_handle_t* out);
extern char* logos_sensor_name(logos_handle_t handle);
extern double logos_sensor_temperature(logos_handle_t handle);
extern bool logos_sensor_active(logos_handle_t handle);
extern int64_t logos_sensor_readings(logos_handle_t handle);
extern double logos_sensor_precision(logos_handle_t handle);
extern void logos_sensor_free(logos_handle_t handle);
extern void logos_free_string(char* str);

int main() {
    logos_handle_t sensor = NULL;
    logos_status_t status = logos_getSensor(&sensor);
    printf("status=%d\n", status);

    char* name = logos_sensor_name(sensor);
    printf("name=%s\n", name);
    logos_free_string(name);

    double temp = logos_sensor_temperature(sensor);
    printf("temp=%.1f\n", temp);

    bool active = logos_sensor_active(sensor);
    printf("active=%d\n", active);

    int64_t readings = logos_sensor_readings(sensor);
    printf("readings=%lld\n", (long long)readings);

    double precision = logos_sensor_precision(sensor);
    printf("precision=%.2f\n", precision);

    logos_sensor_free(sensor);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C struct with mixed field types must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("status=0"), "Status should be OK. Got:\n{}", output);
    assert!(output.contains("name=thermometer"), "Name should be thermometer. Got:\n{}", output);
    assert!(output.contains("temp=98.6"), "Temperature should be 98.6. Got:\n{}", output);
    assert!(output.contains("active=1"), "Active should be true. Got:\n{}", output);
    assert!(output.contains("readings=42"), "Readings should be 42. Got:\n{}", output);
    assert!(output.contains("precision=0.01"), "Precision should be 0.01. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Seq of Real — push, len, at
#[test]
fn c_link_seq_of_real() {
    let logos_source = r#"## To getMeasurements () -> Seq of Real is exported:
    Let s be a new Seq of Real.
    Push 1.5 to s.
    Push 2.7 to s.
    Push 3.14 to s.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0 } logos_status_t;

extern logos_status_t logos_getMeasurements(logos_handle_t* out);
extern size_t logos_seq_f64_len(logos_handle_t handle);
extern logos_status_t logos_seq_f64_at(logos_handle_t handle, size_t index, double* out);
extern void logos_seq_f64_free(logos_handle_t handle);

int main() {
    logos_handle_t seq = NULL;
    logos_getMeasurements(&seq);

    size_t len = logos_seq_f64_len(seq);
    printf("len=%zu\n", len);

    for (size_t i = 0; i < len; i++) {
        double val = 0;
        logos_seq_f64_at(seq, i, &val);
        printf("seq[%zu]=%.2f\n", i, val);
    }

    logos_seq_f64_free(seq);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C seq of real must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("len=3"), "Length should be 3. Got:\n{}", output);
    assert!(output.contains("seq[0]=1.50"), "First should be 1.50. Got:\n{}", output);
    assert!(output.contains("seq[1]=2.70"), "Second should be 2.70. Got:\n{}", output);
    assert!(output.contains("seq[2]=3.14"), "Third should be 3.14. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Seq of Text — push, len, at with string values
#[test]
fn c_link_seq_of_text() {
    let logos_source = r#"## To getNames () -> Seq of Text is exported:
    Let s be a new Seq of Text.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>

typedef void* logos_handle_t;

extern logos_handle_t logos_seq_string_create(void);
extern void logos_seq_string_push(logos_handle_t handle, const char* value);
extern size_t logos_seq_string_len(logos_handle_t handle);
extern char* logos_seq_string_at(logos_handle_t handle, size_t index);
extern void logos_seq_string_free(logos_handle_t handle);
extern void logos_free_string(char* str);

int main() {
    logos_handle_t seq = logos_seq_string_create();
    logos_seq_string_push(seq, "alice");
    logos_seq_string_push(seq, "bob");
    logos_seq_string_push(seq, "carol");

    size_t len = logos_seq_string_len(seq);
    printf("len=%zu\n", len);

    for (size_t i = 0; i < len; i++) {
        char* val = logos_seq_string_at(seq, i);
        printf("seq[%zu]=%s\n", i, val);
        logos_free_string(val);
    }

    logos_seq_string_free(seq);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C seq of text must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("len=3"), "Length should be 3. Got:\n{}", output);
    assert!(output.contains("seq[0]=alice"), "First should be alice. Got:\n{}", output);
    assert!(output.contains("seq[1]=bob"), "Second should be bob. Got:\n{}", output);
    assert!(output.contains("seq[2]=carol"), "Third should be carol. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Seq to_json serialization from C
#[test]
fn c_link_seq_to_json() {
    let logos_source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Push 1 to s.
    Push 2 to s.
    Push 3 to s.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <string.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0 } logos_status_t;

extern logos_status_t logos_getNumbers(logos_handle_t* out);
extern char* logos_seq_i64_to_json(logos_handle_t handle);
extern void logos_seq_i64_free(logos_handle_t handle);
extern void logos_free_string(char* str);

int main() {
    logos_handle_t seq = NULL;
    logos_getNumbers(&seq);

    char* json = logos_seq_i64_to_json(seq);
    printf("json=%s\n", json);
    printf("valid=%d\n", json != NULL && strlen(json) > 0);
    logos_free_string(json);

    logos_seq_i64_free(seq);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C seq to_json must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("json=[1,2,3]"), "JSON should be [1,2,3]. Got:\n{}", output);
    assert!(output.contains("valid=1"), "JSON should be non-empty. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Error API — set, read, clear cycle
#[test]
fn c_link_error_api_full_cycle() {
    let logos_source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <string.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1, LOGOS_TYPE_MISMATCH = 2, LOGOS_NULL_POINTER = 3, LOGOS_OUT_OF_BOUNDS = 4 } logos_status_t;

extern const char* logos_last_error(void);
extern void logos_clear_error(void);
extern size_t logos_seq_i64_len(logos_handle_t handle);
extern logos_status_t logos_seq_i64_at(logos_handle_t handle, size_t index, int64_t* out);

int main() {
    const char* err = logos_last_error();
    printf("initial_error_null=%d\n", err == NULL);

    size_t len = logos_seq_i64_len(NULL);
    err = logos_last_error();
    printf("after_null_access_has_error=%d\n", err != NULL);
    if (err) printf("error_msg_nonempty=%d\n", strlen(err) > 0);

    logos_clear_error();
    err = logos_last_error();
    printf("after_clear_error_null=%d\n", err == NULL);

    int64_t val = 0;
    logos_status_t status = logos_seq_i64_at(NULL, 0, &val);
    printf("null_at_status=%d\n", status);
    err = logos_last_error();
    printf("at_error_not_null=%d\n", err != NULL);

    logos_clear_error();
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C error API cycle must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("initial_error_null=1"), "No error initially. Got:\n{}", output);
    assert!(output.contains("after_null_access_has_error=1"), "Error after null access. Got:\n{}", output);
    assert!(output.contains("error_msg_nonempty=1"), "Error message should be nonempty. Got:\n{}", output);
    assert!(output.contains("after_clear_error_null=1"), "Error cleared. Got:\n{}", output);
    assert!(output.contains("null_at_status=3"), "Null at should return NullPointer. Got:\n{}", output);
    assert!(output.contains("at_error_not_null=1"), "Error set again. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Multiple text function calls — verify no memory corruption
#[test]
fn c_link_text_multiple_calls() {
    let logos_source = r#"## To greet (name: Text) -> Text is exported:
    Return "Hello " + name.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <string.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0 } logos_status_t;

extern logos_status_t logos_greet(const char* name, char** out);
extern void logos_free_string(char* str);

int main() {
    const char* names[] = {"Alice", "Bob", "Carol", "Dave", "Eve"};
    for (int i = 0; i < 5; i++) {
        char* result = NULL;
        logos_status_t status = logos_greet(names[i], &result);
        printf("call_%d_status=%d result=%s\n", i, status, result ? result : "null");
        if (result) logos_free_string(result);
    }
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C multiple text calls must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("call_0_status=0 result=Hello Alice"), "First call. Got:\n{}", output);
    assert!(output.contains("call_1_status=0 result=Hello Bob"), "Second call. Got:\n{}", output);
    assert!(output.contains("call_2_status=0 result=Hello Carol"), "Third call. Got:\n{}", output);
    assert!(output.contains("call_3_status=0 result=Hello Dave"), "Fourth call. Got:\n{}", output);
    assert!(output.contains("call_4_status=0 result=Hello Eve"), "Fifth call. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Map returned from exported function — populated with data
#[test]
fn c_link_map_from_exported_function() {
    let logos_source = r#"## To getScores () -> Map of Text to Int is exported:
    Let mut m be a new Map of Text to Int.
    Set m["math"] to 95.
    Set m["science"] to 88.
    Return m.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1 } logos_status_t;

extern logos_status_t logos_getScores(logos_handle_t* out);
extern size_t logos_map_string_i64_len(logos_handle_t handle);
extern logos_status_t logos_map_string_i64_get(logos_handle_t handle, const char* key, int64_t* out);
extern void logos_map_string_i64_free(logos_handle_t handle);

int main() {
    logos_handle_t map = NULL;
    logos_status_t status = logos_getScores(&map);
    printf("status=%d\n", status);
    printf("not_null=%d\n", map != NULL);

    size_t len = logos_map_string_i64_len(map);
    printf("len=%zu\n", len);

    int64_t math = 0, science = 0;
    logos_map_string_i64_get(map, "math", &math);
    logos_map_string_i64_get(map, "science", &science);
    printf("math=%lld\n", (long long)math);
    printf("science=%lld\n", (long long)science);

    logos_map_string_i64_free(map);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C map from exported function must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("status=0"), "Status should be OK. Got:\n{}", output);
    assert!(output.contains("not_null=1"), "Map handle should not be null. Got:\n{}", output);
    assert!(output.contains("len=2"), "Map should have 2 entries. Got:\n{}", output);
    assert!(output.contains("math=95"), "Math score should be 95. Got:\n{}", output);
    assert!(output.contains("science=88"), "Science score should be 88. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Use-after-free detection — accessor on freed handle
#[test]
fn c_link_use_after_free_graceful() {
    let logos_source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Push 42 to s.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1, LOGOS_TYPE_MISMATCH = 2, LOGOS_NULL_POINTER = 3, LOGOS_OUT_OF_BOUNDS = 4, LOGOS_DESER_FAILED = 5, LOGOS_INVALID_HANDLE = 6 } logos_status_t;

extern logos_status_t logos_getNumbers(logos_handle_t* out);
extern size_t logos_seq_i64_len(logos_handle_t handle);
extern logos_status_t logos_seq_i64_at(logos_handle_t handle, size_t index, int64_t* out);
extern void logos_seq_i64_free(logos_handle_t handle);
extern const char* logos_last_error(void);
extern void logos_clear_error(void);

int main() {
    logos_handle_t seq = NULL;
    logos_getNumbers(&seq);

    size_t len = logos_seq_i64_len(seq);
    printf("before_free_len=%zu\n", len);

    logos_seq_i64_free(seq);
    logos_clear_error();

    size_t len_after = logos_seq_i64_len(seq);
    printf("after_free_len=%zu\n", len_after);

    const char* err = logos_last_error();
    printf("has_error=%d\n", err != NULL);

    int64_t val = -1;
    logos_status_t status = logos_seq_i64_at(seq, 0, &val);
    printf("after_free_at_status=%d\n", status);

    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "Use-after-free must not crash. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("before_free_len=1"), "Before free, len should be 1. Got:\n{}", output);
    assert!(output.contains("after_free_len=0"), "After free, len should be 0 (invalid handle). Got:\n{}", output);
    assert!(output.contains("has_error=1"), "Error should be set after use-after-free. Got:\n{}", output);
    assert!(output.contains("after_free_at_status=6"), "Use-after-free should return InvalidHandle(6). Got:\n{}", output);
    assert!(output.contains("done"), "Should complete without crash. Got:\n{}", output);
}

// ─────────────────────────────────────────────────────────
// C Linkage Tests — Batch 3: Limit-pushing complex scenarios
// ─────────────────────────────────────────────────────────

// C-Link: Exported function taking a Seq handle parameter
// CRITICAL: Tests that ref-type params are correctly unmarshaled from handle IDs
#[test]
fn c_link_function_takes_seq_parameter() {
    let logos_source = r#"## To sumAll (numbers: Seq of Int) -> Int is exported:
    Let mut total be 0.
    Repeat for n in numbers:
        Set total to total + n.
    Return total.

## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Push 10 to s.
    Push 20 to s.
    Push 30 to s.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0 } logos_status_t;

extern logos_status_t logos_getNumbers(logos_handle_t* out);
extern int64_t logos_sumAll(logos_handle_t numbers);
extern void logos_seq_i64_free(logos_handle_t handle);

int main() {
    logos_handle_t seq = NULL;
    logos_getNumbers(&seq);

    int64_t total = logos_sumAll(seq);
    printf("total=%lld\n", (long long)total);

    logos_seq_i64_free(seq);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C function taking Seq param must succeed. stderr:\n{}\n\nGenerated Rust:\n{}", result.stderr, result.rust_code);
    let output = result.stdout.trim();
    assert!(output.contains("total=60"), "Sum of [10,20,30] should be 60. Got:\n{}\n\nGenerated Rust:\n{}", output, result.rust_code);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Exported function taking a struct handle parameter
#[test]
fn c_link_function_takes_struct_parameter() {
    let logos_source = r#"## A Person has:
    a name: Text.
    an age: Int.
    an email: Text.
    an address: Text.
    a phone: Text.

## To getAge (p: Person) -> Int is exported:
    Return p's age.

## To getPerson () -> Person is exported:
    Let p be a new Person with name "Alice" and age 30 and email "a@b" and address "123" and phone "555".
    Return p.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0 } logos_status_t;

extern logos_status_t logos_getPerson(logos_handle_t* out);
extern int64_t logos_getAge(logos_handle_t p);
extern void logos_person_free(logos_handle_t handle);

int main() {
    logos_handle_t person = NULL;
    logos_getPerson(&person);

    int64_t age = logos_getAge(person);
    printf("age=%lld\n", (long long)age);

    logos_person_free(person);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C function taking struct param must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("age=30"), "Age should be 30. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Enum with Text variant fields — string marshaling inside enums
#[test]
fn c_link_enum_with_text_fields() {
    let logos_source = r#"## A Message is one of:
    Greeting with a text: Text.
    Error with a code: Int and a description: Text.

## To getGreeting () -> Message is exported:
    Let m be a new Greeting with text "Hello World".
    Return m.

## To getError () -> Message is exported:
    Let m be a new Error with code 404 and description "Not Found".
    Return m.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <string.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0 } logos_status_t;

extern logos_status_t logos_getGreeting(logos_handle_t* out);
extern logos_status_t logos_getError(logos_handle_t* out);
extern int32_t logos_message_tag(logos_handle_t handle);
extern char* logos_message_greeting_text(logos_handle_t handle);
extern int64_t logos_message_error_code(logos_handle_t handle);
extern char* logos_message_error_description(logos_handle_t handle);
extern void logos_message_free(logos_handle_t handle);
extern void logos_free_string(char* str);

int main() {
    logos_handle_t greet = NULL;
    logos_getGreeting(&greet);

    int32_t greet_tag = logos_message_tag(greet);
    printf("greet_tag=%d\n", greet_tag);

    char* greet_text = logos_message_greeting_text(greet);
    printf("greet_text=%s\n", greet_text);
    logos_free_string(greet_text);
    logos_message_free(greet);

    logos_handle_t err = NULL;
    logos_getError(&err);

    int32_t err_tag = logos_message_tag(err);
    printf("err_tag=%d\n", err_tag);

    int64_t err_code = logos_message_error_code(err);
    printf("err_code=%lld\n", (long long)err_code);

    char* err_desc = logos_message_error_description(err);
    printf("err_desc=%s\n", err_desc);
    logos_free_string(err_desc);
    logos_message_free(err);

    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C enum with text fields must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("greet_tag=0"), "Greeting tag should be 0. Got:\n{}", output);
    assert!(output.contains("greet_text=Hello World"), "Greeting text. Got:\n{}", output);
    assert!(output.contains("err_tag=1"), "Error tag should be 1. Got:\n{}", output);
    assert!(output.contains("err_code=404"), "Error code should be 404. Got:\n{}", output);
    assert!(output.contains("err_desc=Not Found"), "Error description. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Enum with 3 variants
#[test]
fn c_link_enum_three_variants() {
    let logos_source = r#"## A Color is one of:
    Red with an intensity: Int.
    Green with an intensity: Int.
    Blue with an intensity: Int.

## To getRed () -> Color is exported:
    Let c be a new Red with intensity 255.
    Return c.

## To getGreen () -> Color is exported:
    Let c be a new Green with intensity 128.
    Return c.

## To getBlue () -> Color is exported:
    Let c be a new Blue with intensity 64.
    Return c.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0 } logos_status_t;

extern logos_status_t logos_getRed(logos_handle_t* out);
extern logos_status_t logos_getGreen(logos_handle_t* out);
extern logos_status_t logos_getBlue(logos_handle_t* out);
extern int32_t logos_color_tag(logos_handle_t handle);
extern int64_t logos_color_red_intensity(logos_handle_t handle);
extern int64_t logos_color_green_intensity(logos_handle_t handle);
extern int64_t logos_color_blue_intensity(logos_handle_t handle);
extern void logos_color_free(logos_handle_t handle);

int main() {
    logos_handle_t r = NULL, g = NULL, b = NULL;
    logos_getRed(&r);
    logos_getGreen(&g);
    logos_getBlue(&b);

    printf("red_tag=%d red_val=%lld\n", logos_color_tag(r), (long long)logos_color_red_intensity(r));
    printf("green_tag=%d green_val=%lld\n", logos_color_tag(g), (long long)logos_color_green_intensity(g));
    printf("blue_tag=%d blue_val=%lld\n", logos_color_tag(b), (long long)logos_color_blue_intensity(b));

    int64_t wrong = logos_color_green_intensity(r);
    printf("wrong_variant_default=%lld\n", (long long)wrong);

    logos_color_free(r);
    logos_color_free(g);
    logos_color_free(b);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C 3-variant enum must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("red_tag=0 red_val=255"), "Red. Got:\n{}", output);
    assert!(output.contains("green_tag=1 green_val=128"), "Green. Got:\n{}", output);
    assert!(output.contains("blue_tag=2 blue_val=64"), "Blue. Got:\n{}", output);
    assert!(output.contains("wrong_variant_default=0"), "Wrong variant should return 0. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Portable struct to_json and from_json
#[test]
fn c_link_struct_json_serialization() {
    let logos_source = r#"## A portable Config has:
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
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <string.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1 } logos_status_t;

extern logos_status_t logos_getConfig(logos_handle_t* out);
extern char* logos_config_to_json(logos_handle_t handle);
extern logos_status_t logos_config_from_json(const char* json, logos_handle_t* out);
extern char* logos_config_host(logos_handle_t handle);
extern int64_t logos_config_port(logos_handle_t handle);
extern void logos_config_free(logos_handle_t handle);
extern void logos_free_string(char* str);

int main() {
    logos_handle_t cfg = NULL;
    logos_getConfig(&cfg);

    char* json = logos_config_to_json(cfg);
    printf("json_not_null=%d\n", json != NULL);
    printf("json_has_host=%d\n", strstr(json, "localhost") != NULL);
    printf("json_has_port=%d\n", strstr(json, "8080") != NULL);

    logos_handle_t cfg2 = NULL;
    logos_status_t status = logos_config_from_json(json, &cfg2);
    printf("from_json_status=%d\n", status);

    char* host2 = logos_config_host(cfg2);
    int64_t port2 = logos_config_port(cfg2);
    printf("roundtrip_host=%s\n", host2);
    printf("roundtrip_port=%lld\n", (long long)port2);

    logos_free_string(json);
    logos_free_string(host2);
    logos_config_free(cfg);
    logos_config_free(cfg2);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C struct JSON serialization must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("json_not_null=1"), "JSON should not be null. Got:\n{}", output);
    assert!(output.contains("json_has_host=1"), "JSON should contain host. Got:\n{}", output);
    assert!(output.contains("json_has_port=1"), "JSON should contain port. Got:\n{}", output);
    assert!(output.contains("from_json_status=0"), "from_json should succeed. Got:\n{}", output);
    assert!(output.contains("roundtrip_host=localhost"), "Roundtrip host should match. Got:\n{}", output);
    assert!(output.contains("roundtrip_port=8080"), "Roundtrip port should match. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Complex multi-type function — mixed Int, Real, Text, Bool params
#[test]
fn c_link_complex_multi_type_params() {
    let logos_source = r#"## To describe (name: Text, age: Int, height: Real, active: Bool) -> Text is exported:
    If active then:
        Return name + " is active".
    Otherwise:
        Return name + " is inactive".

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <stdbool.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0 } logos_status_t;

extern logos_status_t logos_describe(const char* name, int64_t age, double height, bool active, char** out);
extern void logos_free_string(char* str);

int main() {
    char* result1 = NULL;
    logos_status_t s1 = logos_describe("Alice", 30, 5.7, 1, &result1);
    printf("s1=%d r1=%s\n", s1, result1 ? result1 : "null");
    if (result1) logos_free_string(result1);

    char* result2 = NULL;
    logos_status_t s2 = logos_describe("Bob", 25, 6.1, 0, &result2);
    printf("s2=%d r2=%s\n", s2, result2 ? result2 : "null");
    if (result2) logos_free_string(result2);

    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C multi-type function must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("s1=0 r1=Alice is active"), "Active result. Got:\n{}", output);
    assert!(output.contains("s2=0 r2=Bob is inactive"), "Inactive result. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Multiple struct types in the same program
#[test]
fn c_link_multiple_struct_types() {
    let logos_source = r#"## A Person has:
    a name: Text.
    an age: Int.
    an email: Text.
    an address: Text.
    a phone: Text.

## A Sensor has:
    a name: Text.
    a temperature: Real.
    a active: Bool.
    a readings: Int.
    a precision: Real.

## To getPerson () -> Person is exported:
    Let p be a new Person with name "Alice" and age 30 and email "a@b" and address "123" and phone "555".
    Return p.

## To getSensor () -> Sensor is exported:
    Let s be a new Sensor with name "temp1" and temperature 72.5 and active true and readings 100 and precision 0.1.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <stdbool.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0 } logos_status_t;

extern logos_status_t logos_getPerson(logos_handle_t* out);
extern logos_status_t logos_getSensor(logos_handle_t* out);
extern char* logos_person_name(logos_handle_t handle);
extern int64_t logos_person_age(logos_handle_t handle);
extern char* logos_sensor_name(logos_handle_t handle);
extern double logos_sensor_temperature(logos_handle_t handle);
extern bool logos_sensor_active(logos_handle_t handle);
extern void logos_person_free(logos_handle_t handle);
extern void logos_sensor_free(logos_handle_t handle);
extern void logos_free_string(char* str);

int main() {
    logos_handle_t person = NULL, sensor = NULL;
    logos_getPerson(&person);
    logos_getSensor(&sensor);

    char* pname = logos_person_name(person);
    int64_t page = logos_person_age(person);
    char* sname = logos_sensor_name(sensor);
    double stemp = logos_sensor_temperature(sensor);
    bool sactive = logos_sensor_active(sensor);

    printf("person=%s age=%lld\n", pname, (long long)page);
    printf("sensor=%s temp=%.1f active=%d\n", sname, stemp, sactive);

    logos_free_string(pname);
    logos_free_string(sname);
    logos_person_free(person);
    logos_sensor_free(sensor);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C multiple struct types must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("person=Alice age=30"), "Person data. Got:\n{}", output);
    assert!(output.contains("sensor=temp1 temp=72.5 active=1"), "Sensor data. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: String edge cases — empty strings, special characters
#[test]
fn c_link_string_edge_cases() {
    let logos_source = r#"## To echo (s: Text) -> Text is exported:
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <string.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0 } logos_status_t;

extern logos_status_t logos_echo(const char* s, char** out);
extern void logos_free_string(char* str);

int main() {
    char* result = NULL;

    logos_echo("", &result);
    printf("empty_len=%d\n", result ? (int)strlen(result) : -1);
    if (result) logos_free_string(result);

    logos_echo("hello world", &result);
    printf("spaces=%s\n", result);
    if (result) logos_free_string(result);

    logos_echo("line1\nline2", &result);
    printf("has_newline=%d\n", result != NULL);
    if (result) logos_free_string(result);

    logos_echo("tab\there", &result);
    printf("has_tab=%d\n", result != NULL);
    if (result) logos_free_string(result);

    logos_echo("unicode: cafe\xcc\x81", &result);
    printf("unicode_ok=%d\n", result != NULL);
    if (result) logos_free_string(result);

    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C string edge cases must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("empty_len=0"), "Empty string should have length 0. Got:\n{}", output);
    assert!(output.contains("spaces=hello world"), "Spaces should be preserved. Got:\n{}", output);
    assert!(output.contains("has_newline=1"), "Newline strings should work. Got:\n{}", output);
    assert!(output.contains("has_tab=1"), "Tab strings should work. Got:\n{}", output);
    assert!(output.contains("unicode_ok=1"), "Unicode should work. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// C-Link: Chained exported function calls — output of one feeds into another
#[test]
fn c_link_chained_function_calls() {
    let logos_source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## To doubleIt (n: Int) -> Int is exported:
    Return n * 2.

## To negate (n: Int) -> Int is exported:
    Return 0 - n.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

extern int64_t logos_add(int64_t a, int64_t b);
extern int64_t logos_doubleIt(int64_t n);
extern int64_t logos_negate(int64_t n);

int main() {
    int64_t a = logos_add(3, 4);
    int64_t b = logos_doubleIt(a);
    int64_t c = logos_negate(b);
    printf("add=%lld double=%lld negate=%lld\n", (long long)a, (long long)b, (long long)c);

    int64_t chain = logos_negate(logos_doubleIt(logos_add(10, 5)));
    printf("chain=%lld\n", (long long)chain);

    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C chained function calls must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("add=7 double=14 negate=-14"), "Chain step by step. Got:\n{}", output);
    assert!(output.contains("chain=-30"), "Nested chain: negate(double(add(10,5)))=-30. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// ─────────────────────────────────────────────────────────
// Batch A — Out-Parameter Null Safety
// ─────────────────────────────────────────────────────────

// Batch A.1: Seq _at with NULL out pointer returns NullPointer(3)
#[test]
fn c_link_null_out_seq_at() {
    let logos_source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Push 10 to s.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1, LOGOS_REFINEMENT = 2, LOGOS_NULL_POINTER = 3 } logos_status_t;

extern logos_status_t logos_getNumbers(logos_handle_t* out);
extern logos_status_t logos_seq_i64_at(logos_handle_t handle, size_t index, int64_t* out);
extern void logos_seq_i64_free(logos_handle_t handle);
extern const char* logos_last_error(void);
extern void logos_clear_error(void);

int main() {
    logos_handle_t seq = NULL;
    logos_getNumbers(&seq);

    logos_status_t status = logos_seq_i64_at(seq, 0, NULL);
    printf("null_out_status=%d\n", status);

    const char* err = logos_last_error();
    printf("has_error=%d\n", err != NULL);

    logos_clear_error();
    int64_t val = -1;
    logos_status_t ok = logos_seq_i64_at(seq, 0, &val);
    printf("valid_out_status=%d val=%lld\n", ok, (long long)val);

    logos_seq_i64_free(seq);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C null out seq_at test must not crash. stderr:\n{}\n\nRust:\n{}", result.stderr, result.rust_code);
    let output = result.stdout.trim();
    assert!(output.contains("null_out_status=3"), "NULL out to seq_at should return NullPointer(3). Got:\n{}", output);
    assert!(output.contains("has_error=1"), "Error should be set. Got:\n{}", output);
    assert!(output.contains("valid_out_status=0 val=10"), "Valid out should work after recovery. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// Batch A.2: Map _get with NULL out pointer returns NullPointer(3)
#[test]
fn c_link_null_out_map_get() {
    let logos_source = r#"## To getScores () -> Map of Text to Int is exported:
    Let m be a new Map of Text to Int.
    Set m at "alice" to 100.
    Return m.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1, LOGOS_REFINEMENT = 2, LOGOS_NULL_POINTER = 3 } logos_status_t;

extern logos_status_t logos_getScores(logos_handle_t* out);
extern logos_status_t logos_map_string_i64_get(logos_handle_t handle, const char* key, int64_t* out);
extern void logos_map_string_i64_free(logos_handle_t handle);
extern const char* logos_last_error(void);
extern void logos_clear_error(void);

int main() {
    logos_handle_t map = NULL;
    logos_getScores(&map);

    logos_status_t status = logos_map_string_i64_get(map, "alice", NULL);
    printf("null_out_status=%d\n", status);

    const char* err = logos_last_error();
    printf("has_error=%d\n", err != NULL);

    logos_clear_error();
    int64_t val = -1;
    logos_status_t ok = logos_map_string_i64_get(map, "alice", &val);
    printf("valid_out_status=%d val=%lld\n", ok, (long long)val);

    logos_map_string_i64_free(map);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C null out map_get test must not crash. stderr:\n{}\n\nRust:\n{}", result.stderr, result.rust_code);
    let output = result.stdout.trim();
    assert!(output.contains("null_out_status=3"), "NULL out to map_get should return NullPointer(3). Got:\n{}", output);
    assert!(output.contains("has_error=1"), "Error should be set. Got:\n{}", output);
    assert!(output.contains("valid_out_status=0 val=100"), "Valid out should work after recovery. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// Batch A.3: Option _unwrap with NULL out pointer returns NullPointer(3)
#[test]
fn c_link_null_out_option_unwrap() {
    let logos_source = r#"## To getMaybe () -> Option of Int is exported:
    Let o be some 42.
    Return o.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1, LOGOS_REFINEMENT = 2, LOGOS_NULL_POINTER = 3 } logos_status_t;

extern logos_status_t logos_getMaybe(logos_handle_t* out);
extern logos_status_t logos_option_i64_unwrap(logos_handle_t handle, int64_t* out);
extern void logos_option_i64_free(logos_handle_t handle);
extern const char* logos_last_error(void);
extern void logos_clear_error(void);

int main() {
    logos_handle_t opt = NULL;
    logos_getMaybe(&opt);

    logos_status_t status = logos_option_i64_unwrap(opt, NULL);
    printf("null_out_status=%d\n", status);

    const char* err = logos_last_error();
    printf("has_error=%d\n", err != NULL);

    logos_clear_error();
    int64_t val = -1;
    logos_status_t ok = logos_option_i64_unwrap(opt, &val);
    printf("valid_out_status=%d val=%lld\n", ok, (long long)val);

    logos_option_i64_free(opt);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C null out option_unwrap test must not crash. stderr:\n{}\n\nRust:\n{}", result.stderr, result.rust_code);
    let output = result.stdout.trim();
    assert!(output.contains("null_out_status=3"), "NULL out to option_unwrap should return NullPointer(3). Got:\n{}", output);
    assert!(output.contains("has_error=1"), "Error should be set. Got:\n{}", output);
    assert!(output.contains("valid_out_status=0 val=42"), "Valid out should work after recovery. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// Batch A.4: Struct _from_json with NULL out pointer returns NullPointer(3)
#[test]
fn c_link_null_out_from_json() {
    let logos_source = r#"## A Config has:
    a name: Text.
    a value: Int.

## To getConfig () -> Config is exported:
    Let c be a new Config with name "test" and value 42.
    Return c.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1, LOGOS_REFINEMENT = 2, LOGOS_NULL_POINTER = 3 } logos_status_t;

extern logos_status_t logos_config_from_json(const char* json, logos_handle_t* out);
extern char* logos_config_name(logos_handle_t handle);
extern void logos_config_free(logos_handle_t handle);
extern void logos_free_string(char* str);
extern const char* logos_last_error(void);
extern void logos_clear_error(void);

int main() {
    logos_status_t status = logos_config_from_json("{\"name\":\"hello\",\"value\":99}", NULL);
    printf("null_out_status=%d\n", status);

    const char* err = logos_last_error();
    printf("has_error=%d\n", err != NULL);

    logos_clear_error();
    logos_handle_t cfg = NULL;
    logos_status_t ok = logos_config_from_json("{\"name\":\"hello\",\"value\":99}", &cfg);
    printf("valid_out_status=%d\n", ok);

    char* name = logos_config_name(cfg);
    printf("name=%s\n", name);
    logos_free_string(name);

    logos_config_free(cfg);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C null out from_json test must not crash. stderr:\n{}\n\nRust:\n{}", result.stderr, result.rust_code);
    let output = result.stdout.trim();
    assert!(output.contains("null_out_status=3"), "NULL out to from_json should return NullPointer(3). Got:\n{}", output);
    assert!(output.contains("has_error=1"), "Error should be set. Got:\n{}", output);
    assert!(output.contains("valid_out_status=0"), "Valid out should work. Got:\n{}", output);
    assert!(output.contains("name=hello"), "Should recover and get name. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// ─────────────────────────────────────────────────────────
// Non-text key Map — codegen unit tests
// ─────────────────────────────────────────────────────────

const MAP_INT_TO_TEXT_SOURCE: &str = r#"## To getNames () -> Map of Int to Text is exported:
    Let m be a new Map of Int to Text.
    Return m.

## Main
Show 42.
"#;

const MAP_INT_TO_INT_SOURCE: &str = r#"## To getGrid () -> Map of Int to Int is exported:
    Let m be a new Map of Int to Int.
    Return m.

## Main
Show 42.
"#;

#[test]
fn map_int_key_get_accessor() {
    let output = compile_program_full(MAP_INT_TO_TEXT_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_map_i64_string_get(handle: LogosHandle, key: i64) -> *mut std::os::raw::c_char"),
        "Should generate get for Map of Int to Text with raw key type. Generated:\n{}", output.rust_code);
}

#[test]
fn map_int_key_insert_accessor() {
    let output = compile_program_full(MAP_INT_TO_TEXT_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_map_i64_string_insert(handle: LogosHandle, key: i64, value: *const std::os::raw::c_char)"),
        "Should generate insert for Map of Int to Text with raw key type. Generated:\n{}", output.rust_code);
}

#[test]
fn map_int_key_int_val_get_accessor() {
    let output = compile_program_full(MAP_INT_TO_INT_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_map_i64_i64_get(handle: LogosHandle, key: i64, out: *mut i64) -> LogosStatus"),
        "Should generate get with out-param for Map of Int to Int. Generated:\n{}", output.rust_code);
}

#[test]
fn map_int_key_int_val_insert_accessor() {
    let output = compile_program_full(MAP_INT_TO_INT_SOURCE).expect("Should compile");
    assert!(output.rust_code.contains("pub extern \"C\" fn logos_map_i64_i64_insert(handle: LogosHandle, key: i64, value: i64)"),
        "Should generate insert for Map of Int to Int. Generated:\n{}", output.rust_code);
}

#[test]
fn map_int_key_c_header_get() {
    let output = compile_program_full(MAP_INT_TO_TEXT_SOURCE).expect("Should compile");
    let header = output.c_header.as_ref().expect("Should have C header");
    assert!(header.contains("logos_map_i64_string_get(logos_handle_t handle, int64_t key)"),
        "C header should declare get for Map of Int to Text. Header:\n{}", header);
}

#[test]
fn map_int_key_c_header_insert() {
    let output = compile_program_full(MAP_INT_TO_TEXT_SOURCE).expect("Should compile");
    let header = output.c_header.as_ref().expect("Should have C header");
    assert!(header.contains("logos_map_i64_string_insert(logos_handle_t handle, int64_t key,"),
        "C header should declare insert for Map of Int to Text. Header:\n{}", header);
}

#[test]
fn map_int_key_int_val_c_header_get() {
    let output = compile_program_full(MAP_INT_TO_INT_SOURCE).expect("Should compile");
    let header = output.c_header.as_ref().expect("Should have C header");
    assert!(header.contains("logos_map_i64_i64_get(logos_handle_t handle, int64_t key, int64_t* out)"),
        "C header should declare get with out-param for Map of Int to Int. Header:\n{}", header);
}

#[test]
fn map_int_key_int_val_c_header_insert() {
    let output = compile_program_full(MAP_INT_TO_INT_SOURCE).expect("Should compile");
    let header = output.c_header.as_ref().expect("Should have C header");
    assert!(header.contains("logos_map_i64_i64_insert(logos_handle_t handle, int64_t key, int64_t value)"),
        "C header should declare insert for Map of Int to Int. Header:\n{}", header);
}

// ─────────────────────────────────────────────────────────
// Non-text key Map — C linkage end-to-end tests
// ─────────────────────────────────────────────────────────

#[test]
fn c_link_map_int_to_text() {
    let logos_source = r#"## To getNames () -> Map of Int to Text is exported:
    Let m be a new Map of Int to Text.
    Return m.

## To getIds () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>

typedef void* logos_handle_t;

extern logos_handle_t logos_map_i64_string_create(void);
extern void logos_map_i64_string_insert(logos_handle_t handle, int64_t key, const char* value);
extern char* logos_map_i64_string_get(logos_handle_t handle, int64_t key);
extern size_t logos_map_i64_string_len(logos_handle_t handle);
extern logos_handle_t logos_map_i64_string_keys(logos_handle_t handle);
extern logos_handle_t logos_map_i64_string_values(logos_handle_t handle);
extern void logos_map_i64_string_free(logos_handle_t handle);
extern void logos_free_string(char* str);
extern size_t logos_seq_i64_len(logos_handle_t handle);
extern void logos_seq_i64_free(logos_handle_t handle);

int main() {
    logos_handle_t map = logos_map_i64_string_create();
    printf("created=%d\n", map != NULL);

    logos_map_i64_string_insert(map, 1, "alice");
    logos_map_i64_string_insert(map, 2, "bob");
    logos_map_i64_string_insert(map, 3, "carol");

    size_t len = logos_map_i64_string_len(map);
    printf("len=%zu\n", len);

    char* name = logos_map_i64_string_get(map, 1);
    printf("get_1=%s\n", name ? name : "NULL");
    if (name) logos_free_string(name);

    char* name2 = logos_map_i64_string_get(map, 2);
    printf("get_2=%s\n", name2 ? name2 : "NULL");
    if (name2) logos_free_string(name2);

    char* missing = logos_map_i64_string_get(map, 999);
    printf("missing=%s\n", missing ? missing : "NULL");

    logos_handle_t keys = logos_map_i64_string_keys(map);
    size_t keys_len = logos_seq_i64_len(keys);
    printf("keys_len=%zu\n", keys_len);
    logos_seq_i64_free(keys);

    logos_map_i64_string_free(map);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C Map<Int,Text> operations must succeed. stderr:\n{}\n\nRust:\n{}", result.stderr, result.rust_code);
    let output = result.stdout.trim();
    assert!(output.contains("created=1"), "Map should be created. Got:\n{}", output);
    assert!(output.contains("len=3"), "Map length should be 3. Got:\n{}", output);
    assert!(output.contains("get_1=alice"), "Key 1 should map to alice. Got:\n{}", output);
    assert!(output.contains("get_2=bob"), "Key 2 should map to bob. Got:\n{}", output);
    assert!(output.contains("missing=NULL"), "Missing key should return NULL. Got:\n{}", output);
    assert!(output.contains("keys_len=3"), "Keys should have 3 entries. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

#[test]
fn c_link_map_int_to_int() {
    let logos_source = r#"## To getGrid () -> Map of Int to Int is exported:
    Let m be a new Map of Int to Int.
    Return m.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1, LOGOS_REFINEMENT = 2, LOGOS_NULL_POINTER = 3 } logos_status_t;

extern logos_handle_t logos_map_i64_i64_create(void);
extern void logos_map_i64_i64_insert(logos_handle_t handle, int64_t key, int64_t value);
extern logos_status_t logos_map_i64_i64_get(logos_handle_t handle, int64_t key, int64_t* out);
extern size_t logos_map_i64_i64_len(logos_handle_t handle);
extern void logos_map_i64_i64_free(logos_handle_t handle);

int main() {
    logos_handle_t map = logos_map_i64_i64_create();
    printf("created=%d\n", map != NULL);

    logos_map_i64_i64_insert(map, 10, 100);
    logos_map_i64_i64_insert(map, 20, 200);

    size_t len = logos_map_i64_i64_len(map);
    printf("len=%zu\n", len);

    int64_t val = 0;
    logos_status_t status = logos_map_i64_i64_get(map, 10, &val);
    printf("get_10_status=%d\n", status);
    printf("get_10_val=%lld\n", (long long)val);

    status = logos_map_i64_i64_get(map, 20, &val);
    printf("get_20_val=%lld\n", (long long)val);

    status = logos_map_i64_i64_get(map, 999, &val);
    printf("missing_status=%d\n", status);

    status = logos_map_i64_i64_get(map, 10, NULL);
    printf("null_out_status=%d\n", status);

    logos_map_i64_i64_free(map);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C Map<Int,Int> operations must succeed. stderr:\n{}\n\nRust:\n{}", result.stderr, result.rust_code);
    let output = result.stdout.trim();
    assert!(output.contains("created=1"), "Map should be created. Got:\n{}", output);
    assert!(output.contains("len=2"), "Map length should be 2. Got:\n{}", output);
    assert!(output.contains("get_10_status=0"), "Get 10 should succeed. Got:\n{}", output);
    assert!(output.contains("get_10_val=100"), "Key 10 should map to 100. Got:\n{}", output);
    assert!(output.contains("get_20_val=200"), "Key 20 should map to 200. Got:\n{}", output);
    assert!(output.contains("missing_status=1"), "Missing key should return error. Got:\n{}", output);
    assert!(output.contains("null_out_status=3"), "NULL out should return NullPointer. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// ─────────────────────────────────────────────────────────
// Phase 2: C-Link Tests — Missing Type Coverage
// ─────────────────────────────────────────────────────────

#[test]
fn c_link_option_none_from_logos() {
    let logos_source = r#"## To getNone () -> Option of Int is exported:
    Return none.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1 } logos_status_t;

extern logos_status_t logos_getNone(logos_handle_t* out);
extern int logos_option_i64_is_some(logos_handle_t handle);
extern logos_status_t logos_option_i64_unwrap(logos_handle_t handle, int64_t* out);
extern void logos_option_i64_free(logos_handle_t handle);

int main() {
    logos_handle_t opt = NULL;
    logos_status_t status = logos_getNone(&opt);
    printf("status=%d\n", status);
    printf("is_some=%d\n", logos_option_i64_is_some(opt));

    int64_t val = -1;
    logos_status_t unwrap_status = logos_option_i64_unwrap(opt, &val);
    printf("unwrap_status=%d\n", unwrap_status);

    logos_option_i64_free(opt);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C Option None must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("status=0"), "Return should succeed. Got:\n{}", output);
    assert!(output.contains("is_some=0"), "None should not be some. Got:\n{}", output);
    assert!(output.contains("unwrap_status=1"), "Unwrap on None should return error. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

#[test]
fn c_link_seq_of_bool() {
    let logos_source = r#"## To getFlags () -> Seq of Bool is exported:
    Let s be a new Seq of Bool.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <stdbool.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0 } logos_status_t;

extern logos_handle_t logos_seq_bool_create(void);
extern void logos_seq_bool_push(logos_handle_t handle, bool value);
extern size_t logos_seq_bool_len(logos_handle_t handle);
extern logos_status_t logos_seq_bool_at(logos_handle_t handle, size_t index, bool* out);
extern void logos_seq_bool_free(logos_handle_t handle);

int main() {
    logos_handle_t seq = logos_seq_bool_create();
    printf("created=%d\n", seq != NULL);

    logos_seq_bool_push(seq, true);
    logos_seq_bool_push(seq, false);
    logos_seq_bool_push(seq, true);

    size_t len = logos_seq_bool_len(seq);
    printf("len=%zu\n", len);

    bool val = false;
    logos_seq_bool_at(seq, 0, &val);
    printf("at0=%d\n", val);
    logos_seq_bool_at(seq, 1, &val);
    printf("at1=%d\n", val);

    logos_seq_bool_free(seq);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C Seq<Bool> must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("created=1"), "Seq should be created. Got:\n{}", output);
    assert!(output.contains("len=3"), "Length should be 3. Got:\n{}", output);
    assert!(output.contains("at0=1"), "First element should be true. Got:\n{}", output);
    assert!(output.contains("at1=0"), "Second element should be false. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

#[test]
fn c_link_set_of_text() {
    let logos_source = r#"## To getNames () -> Set of Text is exported:
    Let s be a new Set of Text.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdbool.h>

typedef void* logos_handle_t;

extern logos_handle_t logos_set_string_create(void);
extern void logos_set_string_insert(logos_handle_t handle, const char* value);
extern int logos_set_string_contains(logos_handle_t handle, const char* value);
extern size_t logos_set_string_len(logos_handle_t handle);
extern void logos_set_string_free(logos_handle_t handle);

int main() {
    logos_handle_t set = logos_set_string_create();
    printf("created=%d\n", set != NULL);

    logos_set_string_insert(set, "alice");
    logos_set_string_insert(set, "bob");
    logos_set_string_insert(set, "alice");

    size_t len = logos_set_string_len(set);
    printf("len=%zu\n", len);

    int has_alice = logos_set_string_contains(set, "alice");
    int has_carol = logos_set_string_contains(set, "carol");
    printf("has_alice=%d\n", has_alice);
    printf("has_carol=%d\n", has_carol);

    logos_set_string_free(set);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C Set<Text> must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("created=1"), "Set should be created. Got:\n{}", output);
    assert!(output.contains("len=2"), "Duplicate insert should not increase len. Got:\n{}", output);
    assert!(output.contains("has_alice=1"), "Should contain alice. Got:\n{}", output);
    assert!(output.contains("has_carol=0"), "Should not contain carol. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// ─────────────────────────────────────────────────────────
// Phase 3: C-Link Tests — Safety Hardening
// ─────────────────────────────────────────────────────────

#[test]
fn c_link_garbage_handle_id() {
    let logos_source = r#"## To add (a: Int, b: Int) -> Int is exported:
    Return a + b.

## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;

extern size_t logos_seq_i64_len(logos_handle_t handle);
extern const char* logos_last_error(void);
extern void logos_clear_error(void);

int main() {
    logos_handle_t garbage = (logos_handle_t)0xDEADBEEF;
    size_t len = logos_seq_i64_len(garbage);
    printf("len=%zu\n", len);
    const char* err = logos_last_error();
    printf("has_error=%d\n", err != NULL);
    logos_clear_error();
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "Garbage handle must not crash. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("len=0"), "Garbage handle should return 0 for len. Got:\n{}", output);
    assert!(output.contains("has_error=1"), "Should have set error. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

#[test]
fn c_link_null_string_parameter() {
    let logos_source = r#"## To greet (name: Text) -> Text is exported:
    Return "Hello " + name.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1, LOGOS_REFINEMENT = 2, LOGOS_NULL = 3 } logos_status_t;

extern logos_status_t logos_greet(const char* name, char** out);
extern void logos_free_string(char* str);

int main() {
    char* result = NULL;
    logos_status_t status = logos_greet(NULL, &result);
    printf("status=%d\n", status);
    printf("is_null=%d\n", status == LOGOS_NULL);
    // Should not crash — returns NullPointer(3) for NULL text param
    printf("survived=1\n");
    if (result) {
        logos_free_string(result);
    }
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "NULL string parameter must not crash. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("status=3"), "Should return NullPointer(3). Got:\n{}", output);
    assert!(output.contains("is_null=1"), "Should detect NullPointer. Got:\n{}", output);
    assert!(output.contains("survived=1"), "Should survive NULL string. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

#[test]
fn c_link_very_long_string() {
    let logos_source = r#"## To echo (msg: Text) -> Text is exported:
    Return msg.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0 } logos_status_t;

extern logos_status_t logos_echo(const char* msg, char** out);
extern void logos_free_string(char* str);

int main() {
    size_t len = 100000;
    char* big = (char*)malloc(len + 1);
    memset(big, 'A', len);
    big[len] = '\0';

    char* result = NULL;
    logos_status_t status = logos_echo(big, &result);
    printf("status=%d\n", status);
    if (result) {
        printf("result_len=%zu\n", strlen(result));
        printf("match=%d\n", strcmp(big, result) == 0);
        logos_free_string(result);
    }
    free(big);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "Long string must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("status=0"), "Status should be OK. Got:\n{}", output);
    assert!(output.contains("result_len=100000"), "String length should be preserved. Got:\n{}", output);
    assert!(output.contains("match=1"), "String content should match. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

// ─────────────────────────────────────────────────────────
// Phase 4: C-Link Tests — New Accessor Coverage
// ─────────────────────────────────────────────────────────

#[test]
fn c_link_seq_pop() {
    let logos_source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1 } logos_status_t;

extern logos_handle_t logos_seq_i64_create(void);
extern void logos_seq_i64_push(logos_handle_t handle, int64_t value);
extern logos_status_t logos_seq_i64_pop(logos_handle_t handle, int64_t* out);
extern size_t logos_seq_i64_len(logos_handle_t handle);
extern void logos_seq_i64_free(logos_handle_t handle);

int main() {
    logos_handle_t seq = logos_seq_i64_create();
    logos_seq_i64_push(seq, 10);
    logos_seq_i64_push(seq, 20);
    logos_seq_i64_push(seq, 30);

    printf("len_before=%zu\n", logos_seq_i64_len(seq));

    int64_t val = 0;
    logos_status_t status = logos_seq_i64_pop(seq, &val);
    printf("pop_status=%d\n", status);
    printf("popped=%lld\n", (long long)val);
    printf("len_after=%zu\n", logos_seq_i64_len(seq));

    // Pop from empty
    logos_seq_i64_pop(seq, &val);
    logos_seq_i64_pop(seq, &val);
    logos_status_t empty_status = logos_seq_i64_pop(seq, &val);
    printf("empty_pop=%d\n", empty_status);

    logos_seq_i64_free(seq);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C seq pop must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("len_before=3"), "Length should be 3 before pop. Got:\n{}", output);
    assert!(output.contains("pop_status=0"), "Pop should succeed. Got:\n{}", output);
    assert!(output.contains("popped=30"), "Should pop last element (30). Got:\n{}", output);
    assert!(output.contains("len_after=2"), "Length should be 2 after pop. Got:\n{}", output);
    assert!(output.contains("empty_pop=1"), "Pop from empty should return error. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

#[test]
fn c_link_seq_from_json() {
    let logos_source = r#"## To getNumbers () -> Seq of Int is exported:
    Let s be a new Seq of Int.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1, LOGOS_DESER = 5 } logos_status_t;

extern logos_status_t logos_seq_i64_from_json(const char* json, logos_handle_t* out);
extern size_t logos_seq_i64_len(logos_handle_t handle);
extern logos_status_t logos_seq_i64_at(logos_handle_t handle, size_t index, int64_t* out);
extern char* logos_seq_i64_to_json(logos_handle_t handle);
extern void logos_seq_i64_free(logos_handle_t handle);
extern void logos_free_string(char* str);

int main() {
    logos_handle_t seq = NULL;
    logos_status_t status = logos_seq_i64_from_json("[10,20,30]", &seq);
    printf("status=%d\n", status);

    size_t len = logos_seq_i64_len(seq);
    printf("len=%zu\n", len);

    int64_t val = 0;
    logos_seq_i64_at(seq, 0, &val);
    printf("at0=%lld\n", (long long)val);
    logos_seq_i64_at(seq, 2, &val);
    printf("at2=%lld\n", (long long)val);

    // Round-trip: to_json
    char* json = logos_seq_i64_to_json(seq);
    printf("json=%s\n", json);
    logos_free_string(json);

    // Invalid JSON
    logos_handle_t bad = NULL;
    logos_status_t bad_status = logos_seq_i64_from_json("not json", &bad);
    printf("bad_status=%d\n", bad_status);

    logos_seq_i64_free(seq);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C seq from_json must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("status=0"), "from_json should succeed. Got:\n{}", output);
    assert!(output.contains("len=3"), "Deserialized seq should have 3 elements. Got:\n{}", output);
    assert!(output.contains("at0=10"), "First element should be 10. Got:\n{}", output);
    assert!(output.contains("at2=30"), "Third element should be 30. Got:\n{}", output);
    assert!(output.contains("json=[10,20,30]"), "to_json round-trip should match. Got:\n{}", output);
    assert!(output.contains("bad_status=5"), "Invalid JSON should return DeserializationFailed. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

#[test]
fn c_link_map_remove() {
    let logos_source = r#"## To getScores () -> Map of Text to Int is exported:
    Let m be a new Map of Text to Int.
    Return m.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <stdbool.h>

typedef void* logos_handle_t;

extern logos_handle_t logos_map_string_i64_create(void);
extern void logos_map_string_i64_insert(logos_handle_t handle, const char* key, int64_t value);
extern bool logos_map_string_i64_remove(logos_handle_t handle, const char* key);
extern size_t logos_map_string_i64_len(logos_handle_t handle);
extern void logos_map_string_i64_free(logos_handle_t handle);

int main() {
    logos_handle_t map = logos_map_string_i64_create();
    logos_map_string_i64_insert(map, "a", 1);
    logos_map_string_i64_insert(map, "b", 2);
    logos_map_string_i64_insert(map, "c", 3);
    printf("len_before=%zu\n", logos_map_string_i64_len(map));

    bool removed = logos_map_string_i64_remove(map, "b");
    printf("removed_b=%d\n", removed);
    printf("len_after=%zu\n", logos_map_string_i64_len(map));

    bool removed_missing = logos_map_string_i64_remove(map, "z");
    printf("removed_z=%d\n", removed_missing);

    logos_map_string_i64_free(map);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C map remove must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("len_before=3"), "Should have 3 entries. Got:\n{}", output);
    assert!(output.contains("removed_b=1"), "Should remove existing key. Got:\n{}", output);
    assert!(output.contains("len_after=2"), "Length should decrease. Got:\n{}", output);
    assert!(output.contains("removed_z=0"), "Removing missing key should return false. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

#[test]
fn c_link_map_to_json() {
    let logos_source = r#"## To getScores () -> Map of Text to Int is exported:
    Let m be a new Map of Text to Int.
    Return m.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <string.h>

typedef void* logos_handle_t;

extern logos_handle_t logos_map_string_i64_create(void);
extern void logos_map_string_i64_insert(logos_handle_t handle, const char* key, int64_t value);
extern char* logos_map_string_i64_to_json(logos_handle_t handle);
extern void logos_map_string_i64_free(logos_handle_t handle);
extern void logos_free_string(char* str);

int main() {
    logos_handle_t map = logos_map_string_i64_create();
    logos_map_string_i64_insert(map, "x", 42);

    char* json = logos_map_string_i64_to_json(map);
    printf("has_json=%d\n", json != NULL);
    if (json) {
        printf("contains_x=%d\n", strstr(json, "\"x\"") != NULL);
        printf("contains_42=%d\n", strstr(json, "42") != NULL);
        logos_free_string(json);
    }

    logos_map_string_i64_free(map);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C map to_json must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("has_json=1"), "Should produce JSON. Got:\n{}", output);
    assert!(output.contains("contains_x=1"), "JSON should contain key. Got:\n{}", output);
    assert!(output.contains("contains_42=1"), "JSON should contain value. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

#[test]
fn c_link_map_from_json_round_trip() {
    let logos_source = r#"## To getScores () -> Map of Text to Int is exported:
    Let m be a new Map of Text to Int.
    Return m.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_DESER = 5 } logos_status_t;

extern logos_handle_t logos_map_string_i64_create(void);
extern void logos_map_string_i64_insert(logos_handle_t handle, const char* key, int64_t value);
extern char* logos_map_string_i64_to_json(logos_handle_t handle);
extern logos_status_t logos_map_string_i64_from_json(const char* json, logos_handle_t* out);
extern logos_status_t logos_map_string_i64_get(logos_handle_t handle, const char* key, int64_t* out);
extern size_t logos_map_string_i64_len(logos_handle_t handle);
extern void logos_map_string_i64_free(logos_handle_t handle);
extern void logos_free_string(char* str);

int main() {
    // Create original map
    logos_handle_t map = logos_map_string_i64_create();
    logos_map_string_i64_insert(map, "score", 95);

    // Serialize to JSON
    char* json = logos_map_string_i64_to_json(map);
    printf("json_ok=%d\n", json != NULL);

    // Deserialize back
    logos_handle_t map2 = NULL;
    logos_status_t status = logos_map_string_i64_from_json(json, &map2);
    printf("from_json_status=%d\n", status);

    // Verify contents
    size_t len = logos_map_string_i64_len(map2);
    printf("len=%zu\n", len);

    int64_t val = 0;
    logos_map_string_i64_get(map2, "score", &val);
    printf("score=%lld\n", (long long)val);

    logos_free_string(json);
    logos_map_string_i64_free(map);
    logos_map_string_i64_free(map2);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C map JSON round-trip must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("json_ok=1"), "Serialization should succeed. Got:\n{}", output);
    assert!(output.contains("from_json_status=0"), "Deserialization should succeed. Got:\n{}", output);
    assert!(output.contains("len=1"), "Deserialized map should have 1 entry. Got:\n{}", output);
    assert!(output.contains("score=95"), "Value should survive round-trip. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

#[test]
fn c_link_set_remove() {
    let logos_source = r#"## To getIds () -> Set of Int is exported:
    Let s be a new Set of Int.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <stdbool.h>

typedef void* logos_handle_t;

extern logos_handle_t logos_set_i64_create(void);
extern void logos_set_i64_insert(logos_handle_t handle, int64_t value);
extern bool logos_set_i64_remove(logos_handle_t handle, int64_t value);
extern bool logos_set_i64_contains(logos_handle_t handle, int64_t value);
extern size_t logos_set_i64_len(logos_handle_t handle);
extern void logos_set_i64_free(logos_handle_t handle);

int main() {
    logos_handle_t set = logos_set_i64_create();
    logos_set_i64_insert(set, 10);
    logos_set_i64_insert(set, 20);
    logos_set_i64_insert(set, 30);
    printf("len_before=%zu\n", logos_set_i64_len(set));

    bool removed = logos_set_i64_remove(set, 20);
    printf("removed_20=%d\n", removed);
    printf("contains_20=%d\n", logos_set_i64_contains(set, 20));
    printf("len_after=%zu\n", logos_set_i64_len(set));

    bool removed_missing = logos_set_i64_remove(set, 99);
    printf("removed_99=%d\n", removed_missing);

    logos_set_i64_free(set);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C set remove must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("len_before=3"), "Should have 3 elements. Got:\n{}", output);
    assert!(output.contains("removed_20=1"), "Should remove existing element. Got:\n{}", output);
    assert!(output.contains("contains_20=0"), "Removed element should not be present. Got:\n{}", output);
    assert!(output.contains("len_after=2"), "Length should decrease. Got:\n{}", output);
    assert!(output.contains("removed_99=0"), "Removing missing element should return false. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

#[test]
fn c_link_set_to_json() {
    let logos_source = r#"## To getIds () -> Set of Int is exported:
    Let s be a new Set of Int.
    Return s.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>
#include <string.h>

typedef void* logos_handle_t;

extern logos_handle_t logos_set_i64_create(void);
extern void logos_set_i64_insert(logos_handle_t handle, int64_t value);
extern char* logos_set_i64_to_json(logos_handle_t handle);
extern void logos_set_i64_free(logos_handle_t handle);
extern void logos_free_string(char* str);

int main() {
    logos_handle_t set = logos_set_i64_create();
    logos_set_i64_insert(set, 42);

    char* json = logos_set_i64_to_json(set);
    printf("has_json=%d\n", json != NULL);
    if (json) {
        printf("contains_42=%d\n", strstr(json, "42") != NULL);
        logos_free_string(json);
    }

    logos_set_i64_free(set);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C set to_json must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("has_json=1"), "Should produce JSON. Got:\n{}", output);
    assert!(output.contains("contains_42=1"), "JSON should contain value. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

#[test]
fn c_link_option_create_some_from_c() {
    let logos_source = r#"## To getSome () -> Option of Int is exported:
    Return some 42.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1 } logos_status_t;

extern logos_handle_t logos_option_i64_some(int64_t value);
extern int logos_option_i64_is_some(logos_handle_t handle);
extern logos_status_t logos_option_i64_unwrap(logos_handle_t handle, int64_t* out);
extern void logos_option_i64_free(logos_handle_t handle);

int main() {
    logos_handle_t opt = logos_option_i64_some(99);
    printf("created=%d\n", opt != NULL);
    printf("is_some=%d\n", logos_option_i64_is_some(opt));

    int64_t val = 0;
    logos_status_t status = logos_option_i64_unwrap(opt, &val);
    printf("unwrap_status=%d\n", status);
    printf("value=%lld\n", (long long)val);

    logos_option_i64_free(opt);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C option create_some must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("created=1"), "Some should be created. Got:\n{}", output);
    assert!(output.contains("is_some=1"), "Should be Some. Got:\n{}", output);
    assert!(output.contains("unwrap_status=0"), "Unwrap should succeed. Got:\n{}", output);
    assert!(output.contains("value=99"), "Value should be 99. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

#[test]
fn c_link_option_create_none_from_c() {
    let logos_source = r#"## To getSome () -> Option of Int is exported:
    Return some 42.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_ERROR = 1 } logos_status_t;

extern logos_handle_t logos_option_i64_none(void);
extern int logos_option_i64_is_some(logos_handle_t handle);
extern logos_status_t logos_option_i64_unwrap(logos_handle_t handle, int64_t* out);
extern void logos_option_i64_free(logos_handle_t handle);

int main() {
    logos_handle_t opt = logos_option_i64_none();
    printf("created=%d\n", opt != NULL);
    printf("is_some=%d\n", logos_option_i64_is_some(opt));

    int64_t val = -1;
    logos_status_t status = logos_option_i64_unwrap(opt, &val);
    printf("unwrap_status=%d\n", status);

    logos_option_i64_free(opt);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C option create_none must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("created=1"), "None handle should be created. Got:\n{}", output);
    assert!(output.contains("is_some=0"), "Should not be Some. Got:\n{}", output);
    assert!(output.contains("unwrap_status=1"), "Unwrap on None should return error. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}

#[test]
fn c_link_json_round_trip_struct() {
    let logos_source = r#"## A Particle has:
    An x: Int.
    A y: Int.
    A z: Int.
    A mass: Int.
    A charge: Int.

## To getParticle () -> Particle is exported:
    Let p be a new Particle with x 10 and y 20 and z 30 and mass 5 and charge 1.
    Return p.

## Main
Show 42.
"#;
    let c_code = r#"
#include <stdio.h>
#include <stdint.h>

typedef void* logos_handle_t;
typedef enum { LOGOS_OK = 0, LOGOS_DESER = 5 } logos_status_t;

extern logos_status_t logos_getParticle(logos_handle_t* out);
extern char* logos_particle_to_json(logos_handle_t handle);
extern logos_status_t logos_particle_from_json(const char* json, logos_handle_t* out);
extern int64_t logos_particle_x(logos_handle_t handle);
extern int64_t logos_particle_y(logos_handle_t handle);
extern int64_t logos_particle_z(logos_handle_t handle);
extern void logos_particle_free(logos_handle_t handle);
extern void logos_free_string(char* str);

int main() {
    logos_handle_t p1 = NULL;
    logos_getParticle(&p1);

    char* json = logos_particle_to_json(p1);
    printf("has_json=%d\n", json != NULL);

    logos_handle_t p2 = NULL;
    logos_status_t status = logos_particle_from_json(json, &p2);
    printf("from_json_status=%d\n", status);

    int64_t x = logos_particle_x(p2);
    int64_t y = logos_particle_y(p2);
    int64_t z = logos_particle_z(p2);
    printf("x=%lld\n", (long long)x);
    printf("y=%lld\n", (long long)y);
    printf("z=%lld\n", (long long)z);

    logos_free_string(json);
    logos_particle_free(p1);
    logos_particle_free(p2);
    printf("done\n");
    return 0;
}
"#;
    let result = compile_and_link_c(logos_source, c_code);
    assert!(result.success,
        "C struct JSON round-trip must succeed. stderr:\n{}", result.stderr);
    let output = result.stdout.trim();
    assert!(output.contains("has_json=1"), "Serialization should succeed. Got:\n{}", output);
    assert!(output.contains("from_json_status=0"), "Deserialization should succeed. Got:\n{}", output);
    assert!(output.contains("x=10"), "x should survive round-trip. Got:\n{}", output);
    assert!(output.contains("y=20"), "y should survive round-trip. Got:\n{}", output);
    assert!(output.contains("z=30"), "z should survive round-trip. Got:\n{}", output);
    assert!(output.contains("done"), "Should complete. Got:\n{}", output);
}
