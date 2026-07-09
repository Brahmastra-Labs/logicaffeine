//! Phase 30: Collections & Iteration
//!
//! Tests for Seq type, list literals, Repeat loops, and ranges.

use logicaffeine_compile::compile::{compile_to_rust, compile_to_dir};
use tempfile::TempDir;

#[test]
fn test_list_literal_codegen() {
    let source = "## Main\nLet numbers be [1, 2, 3].";
    let rust = compile_to_rust(source).expect("Compiles");
    // De-Rc strips the read-only literal list to a plain typed `Vec<i64>` (the canonical Seq
    // representation collapses when the value is never aliased/mutated). `## No Optimize` keeps
    // the `LogosSeq::from_vec(...)` baseline form.
    assert!(rust.contains("let numbers: Vec<i64> = vec![1, 2, 3]"), "Generated: {}", rust);
}

#[test]
fn test_empty_list_codegen() {
    // "empty" is TokenType::Nothing, but valid identifier here
    let source = "## Main\nLet empty be [].";
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("LogosSeq::from_vec(vec![])"), "Generated: {}", rust);
}

#[test]
fn test_repeat_loop_codegen() {
    let source = r#"
## Main
Let sum be 0.
Repeat for x in [1, 2, 3]:
    Set sum to sum + x.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("for x in"), "Generated: {}", rust);
    // The accumulator `sum + x` is unbounded, so exact arithmetic emits a
    // checked narrowed form (overflow ruling v2): the fused i64-native helper
    // or its `LogosInt`-chain equivalent — never a raw wrapping add.
    assert!(
        rust.contains("sum = logos_add_i64(sum, x);")
            || rust.contains("sum = logos_add_exact(sum, x).expect_i64(\"Int\");"),
        "Generated: {}", rust
    );
}

#[test]
fn test_range_loop_codegen() {
    // "i" is a Pronoun in the lexicon, but valid identifier here
    let source = r#"
## Main
Repeat for i from 1 to 10:
    Show i.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("for i in (1..=10)"), "Generated: {}", rust);
}

#[test]
fn test_repeat_with_variable_iterator() {
    // "items" is TokenType::Items, but valid identifier here
    let source = r#"
## Main
Let items be [10, 20, 30].
Repeat for n in items:
    Show n.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("for n in items"), "Generated: {}", rust);
}

#[test]
fn test_runtime_seq_type() {
    let source = "## Main\nLet list: Seq of Int be [10, 20].";
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    compile_to_dir(source, temp_dir.path()).expect("Full compilation");

    let types_rs = std::fs::read_to_string(temp_dir.path().join("crates/logicaffeine_data/src/types.rs")).unwrap();
    assert!(types_rs.contains("LogosSeq"), "types.rs should contain LogosSeq: {}", types_rs);
}

#[test]
fn test_seq_type_annotation_codegen() {
    let source = "## Main\nLet nums: Seq of Int be [1, 2].";
    let rust = compile_to_rust(source).expect("Compiles");
    // `Seq of Int` lowers to a Vec<i64> after de-Rc (the read-only annotated literal de-Rc's).
    assert!(rust.contains("Vec<i64>"), "Generated: {}", rust);
}

#[test]
fn test_showable_trait_exported() {
    let source = "## Main\nReturn.";
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    compile_to_dir(source, temp_dir.path()).expect("Full compilation");

    let lib_rs = std::fs::read_to_string(temp_dir.path().join("crates/logicaffeine_system/src/lib.rs")).unwrap();
    assert!(lib_rs.contains("Showable"), "lib.rs should export Showable: {}", lib_rs);

    let io_rs = std::fs::read_to_string(temp_dir.path().join("crates/logicaffeine_system/src/io.rs")).unwrap();
    assert!(io_rs.contains("pub trait Showable"), "io.rs should define Showable: {}", io_rs);
}

// Phase 30b: Optional "Repeat" keyword - "for" alone should work
#[test]
fn test_for_loop_without_repeat() {
    let source = r#"
## Main
for i from 1 to 5:
    Show i.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("for i in (1..=5)"), "Generated: {}", rust);
}

#[test]
fn test_for_in_without_repeat() {
    let source = r#"
## Main
Let items be [1, 2, 3].
for x in items:
    Show x.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("for x in items"), "Generated: {}", rust);
}
