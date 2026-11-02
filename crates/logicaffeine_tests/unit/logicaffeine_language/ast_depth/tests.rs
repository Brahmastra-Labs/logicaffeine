use super::*;

#[test]
fn override_resolution() {
    assert_eq!(limit_from(None), DEFAULT_MAX_AST_DEPTH);
    assert_eq!(limit_from(Some("1000")), 1000);
    assert_eq!(limit_from(Some(" 2048 ")), 2048);
    assert_eq!(limit_from(Some("3")), MIN_AST_DEPTH, "floor applies");
    assert_eq!(limit_from(Some("potato")), DEFAULT_MAX_AST_DEPTH);
    assert_eq!(limit_from(Some("")), DEFAULT_MAX_AST_DEPTH);
}
