//! LLM Heal's pure core, held to work/LLM_HEAL.md: the wire shapes the worker reads and writes,
//! the line diff, the flight's state machine, the endpoint, the pixelation schedule and the words.
use super::*;

// ---- wire -----------------------------------------------------------------------------------

#[test]
fn a_request_serializes_to_the_workers_shape() {
    let request = Request { mode: Mode::Logic, text: "Evry cat sleeps.".to_string(), fragment: false };
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json, serde_json::json!({ "mode": "logic", "text": "Evry cat sleeps." }));
    let fragment = Request { mode: Mode::Code, text: "Shwo x.".to_string(), fragment: true };
    assert_eq!(serde_json::to_value(&fragment).unwrap(), serde_json::json!({ "mode": "code", "text": "Shwo x.", "fragment": true }));
    assert_eq!(serde_json::to_value(Mode::Code).unwrap(), "code");
}

#[test]
fn an_answer_reads_the_workers_shapes() {
    let ok: Answer = serde_json::from_str(r#"{"ok":true,"healed":"Every cat sleeps.\n","model":"m"}"#).unwrap();
    assert_eq!(ok.healed.as_deref(), Some("Every cat sleeps.\n"));
    let bad: Answer = serde_json::from_str(r#"{"ok":false,"error":"text_too_long"}"#).unwrap();
    assert!(!bad.ok && bad.error.as_deref() == Some("text_too_long") && bad.healed.is_none());
}

#[test]
fn an_outcome_is_the_healed_text_or_a_sentence() {
    assert_eq!(outcome(200, r#"{"ok":true,"healed":"Every cat sleeps.\n","model":"m"}"#), Ok("Every cat sleeps.\n".to_string()));
    assert_eq!(outcome(400, r#"{"ok":false,"error":"text_too_long"}"#), Err(reason("text_too_long")));
    assert_eq!(outcome(429, "rate limited"), Err(reason("rate_limited")));
    assert_eq!(outcome(502, r#"{"ok":false,"error":"upstream_failed"}"#), Err(reason("upstream_failed")));
    assert_eq!(outcome(200, r#"{"ok":true,"healed":"  "}"#), Err(reason("empty_answer")));
    assert_eq!(outcome(500, "<html>"), Err(reason("not_json")));
    for text in [reason("text_too_long"), reason("rate_limited"), reason("upstream_failed"), reason("unreachable"), reason("odd_name")] {
        assert!(text.ends_with('.'), "a reason is a sentence: {text:?}");
        assert!(!text.contains("cannot") && !text.contains('\u{2014}') && !text.contains(';'), "{text:?}");
    }
}

// ---- diff -----------------------------------------------------------------------------------

#[test]
fn identical_texts_change_no_lines() {
    assert!(changed_lines("## Main\nShow 1.\n", "## Main\nShow 1.\n").is_empty());
    assert!(changed_lines("", "").is_empty());
}

#[test]
fn a_one_word_fix_changes_that_line_alone() {
    let before = "## Main\nLet x be 5.\nShwo x.\nShow x + 1.\n";
    let after = "## Main\nLet x be 5.\nShow x.\nShow x + 1.\n";
    assert_eq!(changed_lines(before, after), vec![2..3]);
}

#[test]
fn added_lines_are_one_run_and_a_deleted_line_is_an_empty_range() {
    assert_eq!(changed_lines("a\nb\n", "a\nx\ny\nb\n"), vec![1..3]);
    assert_eq!(changed_lines("a\nb\nc\n", "a\nc\n"), vec![1..1]);
    assert_eq!(changed_lines("a\nb\n", "a\nB\n"), vec![1..2]);
}

// ---- flight ---------------------------------------------------------------------------------

#[test]
fn the_flight_goes_idle_flying_mending_review_idle() {
    let before = "## Main\nShwo 1.\n".to_string();
    let flight = Flight::begin(&before);
    assert_eq!(flight, Flight::Flying { to: 0 });
    assert!(flight.busy());
    let flight = flight.arrived();
    assert_eq!(flight, Flight::Mending);
    assert!(flight.busy());
    let flight = Flight::answered(before.clone(), Ok("## Main\nShow 1.\n".to_string()));
    let Flight::Review { before: b, after, lines, diff } = &flight else { panic!("the answer is reviewed: {flight:?}") };
    assert_eq!(diff.before, vec![1..2], "the before side marks the replaced line");
    assert_eq!((b.as_str(), after.as_str(), lines.as_slice()), (before.as_str(), "## Main\nShow 1.\n", &[1..2][..]));
    assert_eq!(flight.line(), 1, "the fairy stands at the first changed line");
    assert!(!flight.busy());
    let (home, kept) = flight.clone().keep();
    assert_eq!((home, kept.as_deref()), (Flight::Idle, Some("## Main\nShow 1.\n")));
    assert_eq!(flight.undo(), Flight::Idle);
}

#[test]
fn a_failure_lands_in_the_reviews_place_and_closes_to_idle() {
    let flight = Flight::answered("x".to_string(), Err(reason("upstream_failed")));
    assert_eq!(flight, Flight::Failed(reason("upstream_failed")));
    assert_eq!(flight.clone().keep(), (Flight::Idle, None), "nothing to keep from a failure");
    assert_eq!(flight.undo(), Flight::Idle);
}

#[test]
fn an_empty_editor_and_an_unchanged_mend_say_so() {
    assert_eq!(Flight::begin("  \n"), Flight::Failed(EMPTY_EDITOR.to_string()));
    let same = Flight::answered("Show 1.\n".to_string(), Ok("Show 1.\n".to_string()));
    assert!(matches!(&same, Flight::Failed(r) if r.contains("Nothing to mend")), "{same:?}");
}

#[test]
fn the_dock_narrates_each_state() {
    assert_eq!(narration(&Flight::Idle), None);
    assert_eq!(narration(&Flight::Flying { to: 3 }).as_deref(), Some("Healing\u{2026}"));
    assert_eq!(narration(&Flight::Mending).as_deref(), Some("Healing\u{2026}"));
    let review = Flight::Review { before: String::new(), after: String::new(), lines: vec![0..2, 4..5], diff: LineDiff::default() };
    assert_eq!(narration(&review).as_deref(), Some("Healed 3 lines. Keep or Undo."));
    let one = Flight::Review { before: String::new(), after: String::new(), lines: vec![2..3], diff: LineDiff::default() };
    assert_eq!(narration(&one).as_deref(), Some("Healed 1 line. Keep or Undo."));
    assert_eq!(narration(&Flight::Failed("Why.".to_string())).as_deref(), Some("Why."));
}

// ---- endpoint and fairy ---------------------------------------------------------------------

#[test]
fn the_endpoint_is_the_build_time_address_else_production() {
    match option_env!("LOGOS_HEAL_URL") {
        Some(url) => assert_eq!(ENDPOINT, url),
        None => assert_eq!(ENDPOINT, "https://api.logicaffeine.com/heal"),
    }
    assert!(ENDPOINT.starts_with("http"), "{ENDPOINT}");
    let served = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("public").join(FAIRY.trim_start_matches('/'));
    assert!(served.is_file(), "the fairy is not served: {}", served.display());
}

// ---- pixels ---------------------------------------------------------------------------------

#[test]
fn the_pixel_schedule_is_a_deterministic_permutation() {
    let steps = pixel_steps(40, 7);
    let mut sorted = steps.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (0..40).collect::<Vec<_>>(), "every character dissolves once");
    assert_eq!(steps, pixel_steps(40, 7), "the same seed gives the same order");
    assert_ne!(steps, pixel_steps(40, 8), "another seed gives another order");
    assert_ne!(steps, (0..40).collect::<Vec<_>>(), "the order is not simply left to right");
    assert!(pixel_steps(0, 1).is_empty() && pixel_steps(1, 1) == vec![0]);
}

// ---- both sides, whitespace, words -----------------------------------------------------------

#[test]
fn the_diff_marks_both_sides_and_ignores_spaces_at_a_lines_end() {
    let d = line_diff("a\nb\nc\n", "a\nB\nc\n");
    assert_eq!((d.before, d.after), (vec![1..2], vec![1..2]));
    let d = line_diff("a\nb\nc\n", "a\nc\n");
    assert_eq!((d.before, d.after), (vec![1..2], vec![1..1]), "a deleted line is marked on the before side");
    let d = line_diff("a\nb\n", "a\nx\ny\nb\n");
    assert_eq!((d.before, d.after), (vec![1..1], vec![1..3]), "added lines are marked on the after side");
    assert!(line_diff("Show x.   \n", "Show x.\n").after.is_empty(), "spaces at a line's end are no change");
    assert!(same_text("Show x.  \n\n", "Show x."), "a mend that only moved whitespace is no mend");
    assert!(!same_text("Shwo x.", "Show x."));
    let same = Flight::answered("Show x.  \n".to_string(), Ok("Show x.\n".to_string()));
    assert!(matches!(&same, Flight::Failed(r) if r.contains("Nothing to mend")), "{same:?}");
}

#[test]
fn a_lines_changed_words_are_marked() {
    let marks = word_marks("If score is at leest 90:", "If score is at least 90:");
    assert_eq!(marks, vec![("If score is at ", false), ("leest ", true), ("90:", false)]);
    let marks = word_marks("Shwo x.", "Show x.");
    assert_eq!(marks, vec![("Shwo ", true), ("x.", false)]);
    assert_eq!(word_marks("same line", "same line"), vec![("same line", false)]);
    assert_eq!(word_marks("", "x"), Vec::<(&str, bool)>::new());
    let whole = word_marks("a b c", "x y z");
    assert_eq!(whole, vec![("a b c", true)]);
}

// ---- only what the person wrote ---------------------------------------------------------------

#[test]
fn only_the_lines_a_person_wrote_since_the_file_opened_are_healed() {
    let opened = "## Main\nLet x be 1.\nShow x.\n";
    assert!(edited_regions(opened, opened).is_empty(), "nothing typed, nothing to heal");
    let typed = "## Main\nLet x be 1.\nShwo x.\nshow x plus one\n";
    assert_eq!(edited_regions(opened, typed), vec![2..4], "the changed line and the added line are one region");
    assert_eq!(region_text(typed, &(2..4)), "Shwo x.\nshow x plus one");
    let two = "## Main\nlet y be 2\nLet x be 1.\nShow x.\nshow y\n";
    assert_eq!(edited_regions(opened, two), vec![1..2, 4..5], "two edits far apart are two regions");
    assert!(edited_regions(opened, "## Main\nShow x.\n").is_empty(), "a line only removed is not healed");
}

#[test]
fn the_mended_lines_replace_their_region_and_nothing_else() {
    let typed = "## Main\nLet x be 1.\nShwo x.\nshow x plus one\n";
    let out = splice(typed, &[2..4], &["Show x.\nShow x + 1.".to_string()]);
    assert_eq!(out, "## Main\nLet x be 1.\nShow x.\nShow x + 1.\n");
    let two = "## Main\nlet y be 2\nLet x be 1.\nShow x.\nshow y\n";
    let out = splice(two, &[1..2, 4..5], &["Let y be 2.".to_string(), "Show y.".to_string()]);
    assert_eq!(out, "## Main\nLet y be 2.\nLet x be 1.\nShow x.\nShow y.\n");
    let grown = splice("a\nb\nc", &[1..2], &["b1\nb2\nb3".to_string()]);
    assert_eq!(grown, "a\nb1\nb2\nb3\nc", "a region may grow, and a text with no final newline keeps none");
    assert_eq!(splice("a\nb\n", &[], &[]), "a\nb\n");
}

#[test]
fn a_mend_that_lost_its_indentation_gets_it_back() {
    assert_eq!(reindent("    Shwo x.\n    show y", "Show x.\n    Show y."), "    Show x.\n    Show y.");
    assert_eq!(reindent("    Shwo x.", "If x:\n        Show x."), "    If x:\n        Show x.", "deeper lines keep their depth");
    assert_eq!(reindent("Shwo x.", "Show x."), "Show x.", "a region at the margin stays at the margin");
    assert_eq!(reindent("    a\n", "b\n\nc"), "    b\n\n    c", "blank lines stay blank");
}
