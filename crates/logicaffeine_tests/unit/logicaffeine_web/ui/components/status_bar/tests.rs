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

// ---------------------------------------------------------------------------------------------
// How long the last run took: the status bar says it beside the caret, the Output column under the
// program's last line. The duration is written the way a person reads a stopwatch.
// ---------------------------------------------------------------------------------------------

fn took(millis: f64) -> String {
    RunTime { millis, finished: true }.duration()
}

#[test]
fn a_run_under_ten_milliseconds_is_written_to_a_tenth_of_a_millisecond() {
    assert_eq!(took(0.1), "0.1 ms");
    assert_eq!(took(0.42), "0.4 ms");
    assert_eq!(took(3.26), "3.3 ms");
    assert_eq!(took(9.94), "9.9 ms");
}

#[test]
fn a_run_shorter_than_the_clock_can_tell_says_how_short_it_is_at_most() {
    // A browser clock is coarse: Chrome counts in tenths of a millisecond or finer, Firefox in whole
    // milliseconds, so a reading of zero only says the run took under a millisecond.
    assert_eq!(took(0.0), "under 1 ms");
    assert_eq!(took(0.04), "under 0.1 ms");
    assert_eq!(took(-0.5), "under 1 ms");
    assert_eq!(took(f64::NAN), "under 1 ms");
}

#[test]
fn a_run_from_ten_milliseconds_to_a_second_is_written_in_whole_milliseconds() {
    assert_eq!(took(9.96), "10 ms");
    assert_eq!(took(10.0), "10 ms");
    assert_eq!(took(143.4), "143 ms");
    assert_eq!(took(999.4), "999 ms");
}

#[test]
fn a_run_of_a_second_or_more_is_written_in_seconds() {
    assert_eq!(took(999.6), "1.00 s");
    assert_eq!(took(1_000.0), "1.00 s");
    assert_eq!(took(1_234.0), "1.23 s");
    assert_eq!(took(9_996.0), "10.0 s");
    assert_eq!(took(12_345.0), "12.3 s");
    assert_eq!(took(59_940.0), "59.9 s");
}

#[test]
fn a_run_of_a_minute_or_more_is_written_in_minutes_and_seconds() {
    assert_eq!(took(59_960.0), "1 min 0 s");
    assert_eq!(took(60_000.0), "1 min 0 s");
    assert_eq!(took(83_400.0), "1 min 23 s");
    assert_eq!(took(3_600_000.0), "60 min 0 s");
}

#[test]
fn a_finished_run_and_a_stopped_one_are_told_apart() {
    let finished = RunTime { millis: 2.0, finished: true };
    let stopped = RunTime { millis: 2.0, finished: false };
    assert_eq!(finished.status_label(), "Ran in 2.0 ms");
    assert_eq!(finished.output_label(), "Finished in 2.0 ms");
    assert_eq!(stopped.status_label(), "Stopped after 2.0 ms");
    assert_eq!(stopped.output_label(), "Stopped after 2.0 ms");
}

fn status_bar_html(run: Option<RunTime>) -> String {
    #[derive(Props, Clone, PartialEq)]
    struct Shown {
        run: Option<RunTime>,
    }
    #[allow(non_snake_case)]
    fn Bar(props: Shown) -> Element {
        let status = use_signal(|| EditorStatus { caret: Caret { line: 3, column: 7, selected: 0 }, problems: ProblemCount::default() });
        rsx! { StatusBar { status, language: "LOGOS".to_string(), indent: 4, problems: true, run: props.run } }
    }
    let mut vdom = VirtualDom::new_with_props(Bar, Shown { run });
    vdom.rebuild_in_place();
    dioxus_ssr::render(&vdom)
}

#[test]
fn the_status_bar_says_how_long_the_last_run_took_on_the_caret_s_row() {
    let html = status_bar_html(Some(RunTime { millis: 0.42, finished: true }));
    let run = html.find("data-status=\"run-time\"").expect("the status bar does not show the run time");
    let caret = html.find("data-status=\"position\"").expect("the status bar does not show the caret");
    assert!(html.contains("Ran in 0.4 ms"), "the status bar does not say how long the run took: {html}");
    assert!(run < caret, "the run time is not beside the caret, before it in the row: {html}");
    assert!(html.contains("Ln 3, Col 7"), "{html}");
}

#[test]
fn the_status_bar_says_nothing_about_a_run_before_one_has_happened() {
    let html = status_bar_html(None);
    assert!(!html.contains("data-status=\"run-time\""), "the status bar shows a run time with no run: {html}");
    assert!(!html.contains("Ran in"), "{html}");
}
