//! A type mismatch says which side is which: EXPECTED is what the position promises (a return
//! type, a parameter, a field, an annotation, a collection's element type), FOUND is the value
//! written there. The editor shows this sentence under a squiggle; read backwards, it tells the
//! author to change the wrong thing.

use logicaffeine_compile::compile::compile_to_rust;
use logicaffeine_language::error::ParseErrorKind;

fn sides(source: &str) -> (String, String) {
    match compile_to_rust(source).map(|_| ()).map_err(|e| e.kind) {
        Err(ParseErrorKind::TypeMismatchDetailed { expected, found, .. })
        | Err(ParseErrorKind::TypeMismatch { expected, found }) => (expected, found),
        other => panic!("expected a type mismatch from:\n{source}\ngot {other:?}"),
    }
}

#[test]
fn a_return_expects_the_declared_type() {
    let source = "## To f () -> Int:\n    Return \"oops\".\n\n## Main\nShow f().\n";
    assert_eq!(sides(source), ("Int".to_string(), "Text".to_string()));
}

#[test]
fn an_argument_expects_its_parameter_type() {
    let source = "## To twice (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nShow twice(\"two\").\n";
    assert_eq!(sides(source), ("Int".to_string(), "Text".to_string()));
}

#[test]
fn a_field_expects_its_declared_type() {
    let source = "## A Point has:\n    x: Int.\n\n## Main\nLet mutable p be a new Point with x 1.\nSet p's x to \"far\".\nShow p's x.\n";
    assert_eq!(sides(source), ("Int".to_string(), "Text".to_string()));
}

#[test]
fn an_annotated_binding_expects_its_annotation() {
    let source = "## Main\nLet n: Int be \"one\".\nShow n.\n";
    assert_eq!(sides(source), ("Int".to_string(), "Text".to_string()));
}

#[test]
fn a_push_expects_the_element_type() {
    let source = "## Main\nLet mutable xs be a new Seq of Int.\nPush \"one\" to xs.\nShow xs.\n";
    assert_eq!(sides(source), ("Int".to_string(), "Text".to_string()));
}
