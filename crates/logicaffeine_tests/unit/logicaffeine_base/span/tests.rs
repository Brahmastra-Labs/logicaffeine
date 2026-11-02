use super::*;

#[test]
fn span_new_stores_positions() {
    let span = Span::new(5, 10);
    assert_eq!(span.start, 5);
    assert_eq!(span.end, 10);
}

#[test]
fn span_default_is_zero() {
    let span = Span::default();
    assert_eq!(span.start, 0);
    assert_eq!(span.end, 0);
}

#[test]
fn span_merge_combines_ranges() {
    let a = Span::new(5, 10);
    let b = Span::new(8, 15);
    let merged = a.merge(b);
    assert_eq!(merged.start, 5);
    assert_eq!(merged.end, 15);
}

#[test]
fn span_len_returns_size() {
    let span = Span::new(5, 10);
    assert_eq!(span.len(), 5);
}

#[test]
fn span_is_empty_for_zero_length() {
    let empty = Span::new(5, 5);
    assert!(empty.is_empty());

    let nonempty = Span::new(5, 10);
    assert!(!nonempty.is_empty());
}
