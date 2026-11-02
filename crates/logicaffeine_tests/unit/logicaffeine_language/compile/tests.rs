use super::*;

#[test]
fn test_compile_simple_sentence() {
    let result = compile("John runs.");
    assert!(result.is_ok());
    let output = result.unwrap();
    assert!(output.contains("Run"));
    assert!(output.contains("John"));
}

#[test]
fn test_compile_with_unicode_format() {
    let options = CompileOptions {
        format: OutputFormat::Unicode,
        pragmatic: false,
    };
    let result = compile_with_options("Every dog barks.", options);
    assert!(result.is_ok());
    let output = result.unwrap();
    assert!(output.contains("∀") || output.contains("Forall"));
}

#[test]
fn test_compile_all_scopes() {
    let result = compile_all_scopes("Every woman loves a man.");
    assert!(result.is_ok());
    let readings = result.unwrap();
    assert!(readings.len() >= 1);
}
