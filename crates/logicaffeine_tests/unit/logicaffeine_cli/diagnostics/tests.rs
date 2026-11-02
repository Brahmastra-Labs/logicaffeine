use super::*;

#[test]
fn a_finding_code_is_its_kind_in_kebab_case() {
    let f = |kind| Finding {
        kind,
        severity: Severity::Warning,
        span: Span::default(),
        message: String::new(),
    };
    use logicaffeine_language::lgx::check::FindingKind;
    assert_eq!(
        finding_code(&f(FindingKind::ImageWithoutAlt)),
        "image-without-alt"
    );
    assert_eq!(
        finding_code(&f(FindingKind::DuplicateBlock("x".into()))),
        "duplicate-block"
    );
    assert_eq!(
        finding_code(&f(FindingKind::BindOnNonInput("div".into()))),
        "bind-on-non-input"
    );
}

#[test]
fn a_span_past_the_end_or_inside_a_character_is_clamped() {
    let text = "héllo".to_string();
    let at = Placed::in_file(Path::new("x.lg"), text.clone(), Span::new(2, 99));
    assert!(text.is_char_boundary(at.span.start) && text.is_char_boundary(at.span.end));
    assert!(at.span.end <= text.len() && at.span.start <= at.span.end);
    let backwards = Placed::in_file(Path::new("x.lg"), text, Span::new(4, 1));
    assert!(
        backwards.span.start <= backwards.span.end,
        "a backwards span cannot underflow a width"
    );
}

#[test]
fn an_unknown_call_is_found_where_it_is_called_not_where_prose_mentions_it() {
    let source = "# App\n\nMentions `csrf()` up here.\n\n## Note\nAnd csrf() here.\n\n\
                      ## Main\nShow `quoted csrf()`.\nShow csrf().\n";
    let pe = ParseError {
        kind: ParseErrorKind::Custom("Unknown function: csrf".into()),
        span: Span::default(),
    };
    let at = locate_unknown_call(&pe, source)
        .expect("the call is found")
        .span
        .start;
    let line = source[..at].matches('\n').count() + 1;
    assert_eq!(
        line,
        10,
        "the call on line 10, not a mention in prose: {}",
        &source[at..]
    );
}

#[test]
fn the_pointer_is_file_line_col() {
    let at = Placed::in_file(
        Path::new("pages/app.lg"),
        "ab\ncd\n".into(),
        Span::new(4, 5),
    );
    assert_eq!(at.pointer(), "pages/app.lg:2:2");
}

#[test]
fn newline_count_counts_only_before_the_offset() {
    let text = "a\nb\nc\nd";
    assert_eq!(newline_count(text, 0), 0);
    assert_eq!(newline_count(text, 2), 1, "just past the first newline");
    assert_eq!(newline_count(text, text.len()), 3);
    assert_eq!(
        newline_count(text, 9999),
        3,
        "clamped to the text's own end"
    );
}

#[test]
fn nth_line_start_finds_each_lines_first_byte() {
    let text = "one\ntwo\nthree";
    assert_eq!(nth_line_start(text, 0), 0);
    assert_eq!(nth_line_start(text, 1), 4, "right after the first newline");
    assert_eq!(nth_line_start(text, 2), 8);
    assert_eq!(
        nth_line_start(text, 9),
        text.len(),
        "past the last line clamps to the end"
    );
}

#[test]
fn header_start_skips_the_title_and_leading_prose() {
    assert_eq!(
        header_start("# Title\n\nSome prose.\n\n## To f () -> Int:\n"),
        22
    );
    assert_eq!(
        header_start("## To f () -> Int:\n"),
        0,
        "already starts at its own section"
    );
    assert_eq!(header_start("# Title only, no section at all\n"), 0);
}
