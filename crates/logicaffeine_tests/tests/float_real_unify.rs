//! `Float` and `Real` are the same type. Decimal literals carry the internal
//! kind `Real`; the surface name `Float` must accept them identically — a
//! `: Float` annotation rejecting `3.14` ("expected 'Float', found 'Real'")
//! was a type-system split the Syntax Guide had to document as a wart.

use logicaffeine_compile::interpret_for_ui_sync;

const PROGRAM: &str = r#"## Main
Let x: Float be 3.14.
Show x.
Let y: Real be 3.14.
Show y.
Show x equals y."#;

#[test]
fn float_annotation_accepts_decimal_literals() {
    let r = interpret_for_ui_sync(PROGRAM);
    assert_eq!(
        r.error, None,
        "`: Float` must accept a decimal literal exactly like `: Real`, got: {:?}",
        r.error
    );
    assert_eq!(r.lines, vec!["3.14", "3.14", "true"]);
}

/// The alias holds across function signatures too — a `Float` parameter/return
/// must interoperate with `Real` values with no cast or error.
const FN_PROGRAM: &str = r#"## To halve (x: Float) -> Float:
    Return x / 2.0.

## Main
Let v: Real be 5.0.
Show halve(v)."#;

#[test]
fn float_and_real_interchange_in_signatures() {
    let r = interpret_for_ui_sync(FN_PROGRAM);
    assert_eq!(
        r.error, None,
        "Float/Real must interchange in signatures, got: {:?}",
        r.error
    );
    assert_eq!(r.lines, vec!["2.5"]);
}
