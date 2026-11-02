use super::*;

#[test]
fn spanned_error_display() {
    let err = SpannedError::new("test error", Span::new(5, 10));
    let display = format!("{}", err);
    assert!(display.contains("test error"));
    assert!(display.contains("5..10"));
}
