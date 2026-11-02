use super::*;

#[test]
fn source_map_stores_line_mappings() {
    let mut map = SourceMap::new("Let x be 5.".to_string());
    map.line_to_span.insert(1, Span::new(0, 11));

    assert_eq!(map.get_span_for_line(1), Some(Span::new(0, 11)));
    assert_eq!(map.get_span_for_line(2), None);
}

#[test]
fn source_map_builder_tracks_lines() {
    let mut builder = SourceMapBuilder::new("test source");
    assert_eq!(builder.current_line(), 1);

    builder.newline();
    assert_eq!(builder.current_line(), 2);

    builder.add_lines(3);
    assert_eq!(builder.current_line(), 5);
}

#[test]
fn source_map_builder_records_spans() {
    let mut builder = SourceMapBuilder::new("Let x be 5.\nLet y be 10.");
    builder.record_line(Span::new(0, 11));
    builder.newline();
    builder.record_line(Span::new(12, 24));

    let map = builder.build();
    assert_eq!(map.get_span_for_line(1), Some(Span::new(0, 11)));
    assert_eq!(map.get_span_for_line(2), Some(Span::new(12, 24)));
}

#[test]
fn find_nearest_span_searches_nearby() {
    let mut builder = SourceMapBuilder::new("source");
    builder.record_line(Span::new(0, 10));
    builder.add_lines(5);
    // Line 1 has span, lines 2-6 don't

    let map = builder.build();
    // Line 3 should find line 1's span
    assert_eq!(map.find_nearest_span(3), Some(Span::new(0, 10)));
}
