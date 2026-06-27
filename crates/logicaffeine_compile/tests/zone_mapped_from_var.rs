//! Regression: a memory-mapped zone may take its backing path from a runtime
//! `Text` variable, not only a string literal.
//!
//! `mapped from "file.bin"` parsed; `mapped from path` (a parameter) did not — the
//! parser demanded a string literal. But the realistic use is mapping a file whose
//! path is known only at runtime (the guide's `process_file(path: Text)` example),
//! which is exactly what failed to compile.

use logicaffeine_compile::compile_to_rust;

#[test]
fn zone_mapped_from_variable_compiles() {
    let src = r#"## To process_file (path: Text):
    Inside a zone called "Data" mapped from path:
        Show "Processing: " + path.

## Main
process_file("config.bin")."#;
    let rust = compile_to_rust(src)
        .expect("`mapped from <variable>` must parse and compile");
    assert!(
        rust.contains("new_mapped"),
        "expected a memory-mapped zone in codegen, got:\n{rust}"
    );
    // The path must come from the variable, NOT be emitted as the literal "path".
    assert!(
        !rust.contains("new_mapped(\"path\")"),
        "variable path was wrongly emitted as a string literal:\n{rust}"
    );
}

#[test]
fn zone_mapped_from_literal_still_compiles() {
    let src = r#"## Main
Inside a zone called "D" mapped from "x.bin":
    Show "in"."#;
    let rust = compile_to_rust(src).expect("literal mapped-from must still compile");
    assert!(rust.contains("new_mapped(\"x.bin\")"), "literal path lost:\n{rust}");
}
