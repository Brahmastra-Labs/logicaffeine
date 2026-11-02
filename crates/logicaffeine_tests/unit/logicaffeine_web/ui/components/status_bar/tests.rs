use super::*;

const TEXT: &str = "## Main\nLet x be 1.\nShow x.\n";

fn at(offset: usize) -> Caret {
    Caret::of(TEXT, offset, offset, false)
}

#[test]
fn the_start_of_the_text_is_line_one_column_one() {
    assert_eq!(at(0), Caret { line: 1, column: 1, selected: 0 });
    assert_eq!(Caret::of("", 0, 0, false), Caret::default());
}

#[test]
fn columns_count_from_one_within_the_line() {
    assert_eq!(at(3), Caret { line: 1, column: 4, selected: 0 });
    let line3 = TEXT.find("Show").unwrap();
    assert_eq!(at(line3), Caret { line: 3, column: 1, selected: 0 });
    assert_eq!(at(line3 + 3), Caret { line: 3, column: 4, selected: 0 });
}

#[test]
fn the_end_of_a_line_is_past_its_last_character() {
    let newline = TEXT.find('\n').unwrap();
    assert_eq!(at(newline), Caret { line: 1, column: 8, selected: 0 });
    assert_eq!(at(newline + 1), Caret { line: 2, column: 1, selected: 0 });
}

#[test]
fn the_caret_at_the_very_end_is_on_the_last_empty_line() {
    assert_eq!(at(TEXT.len()), Caret { line: 4, column: 1, selected: 0 });
}

#[test]
fn an_offset_past_the_end_stops_at_the_end() {
    assert_eq!(at(TEXT.len() + 50), at(TEXT.len()));
}

#[test]
fn a_forward_selection_puts_the_caret_at_its_end() {
    let line3 = TEXT.find("Show").unwrap();
    assert_eq!(Caret::of(TEXT, line3 + 3, line3 + 5, false), Caret { line: 3, column: 6, selected: 2 });
}

#[test]
fn a_backward_selection_puts_the_caret_at_its_start() {
    let line3 = TEXT.find("Show").unwrap();
    assert_eq!(Caret::of(TEXT, line3 + 3, line3 + 5, true), Caret { line: 3, column: 4, selected: 2 });
}

#[test]
fn a_selection_across_lines_counts_the_line_break() {
    let newline = TEXT.find('\n').unwrap();
    assert_eq!(
        Caret::of(TEXT, newline - 1, newline + 2, false),
        Caret { line: 2, column: 2, selected: 3 },
        "\"n\", the break and \"L\" are selected, and the caret sits after the L"
    );
}

#[test]
fn a_column_counts_characters_not_bytes_or_utf16_units() {
    let text = "∀x ∃y\n😀a";
    assert_eq!(Caret::of(text, 3, 3, false), Caret { line: 1, column: 4, selected: 0 }, "three one-unit symbols in");
    let emoji_then_a = "∀x ∃y\n".encode_utf16().count() + "😀".encode_utf16().count();
    assert_eq!(Caret::of(text, emoji_then_a, emoji_then_a, false), Caret { line: 2, column: 2, selected: 0 });
    assert_eq!(Caret::of(text, emoji_then_a - 2, emoji_then_a + 1, false).selected, 2, "the emoji and the a");
}

#[test]
fn the_label_reads_as_vs_code_writes_it() {
    assert_eq!(Caret { line: 3, column: 4, selected: 0 }.label(), "Ln 3, Col 4");
    assert_eq!(Caret { line: 3, column: 6, selected: 2 }.label(), "Ln 3, Col 6 (2 selected)");
}

fn problem(severity: Severity) -> Problem {
    Problem { start: 0, end: 1, severity, unnecessary: false, message: String::new(), source: None, code: None, docs: None }
}

#[test]
fn errors_and_warnings_are_counted_apart_from_hints() {
    let problems = [problem(Severity::Error), problem(Severity::Warning), problem(Severity::Error), problem(Severity::Hint), problem(Severity::Info)];
    assert_eq!(ProblemCount::of(&problems), ProblemCount { errors: 2, warnings: 1 });
}
