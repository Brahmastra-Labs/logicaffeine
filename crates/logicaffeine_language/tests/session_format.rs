//! `Session::set_format` — mid-session output-format switching for the
//! logic REPL. The switch must affect ONLY rendering: discourse state
//! (anaphora referents, event counters) carries across untouched.

use logicaffeine_language::{OutputFormat, Session};

const S1: &str = "The boys lifted the piano.";
const S2: &str = "They smiled.";

/// Switching to LaTeX mid-session renders the next sentence exactly as a
/// session that used LaTeX from the start — same discourse, same output.
#[test]
fn set_format_matches_latex_from_the_start() {
    let mut latex_all_along = Session::with_format(OutputFormat::LaTeX);
    latex_all_along.eval(S1).expect("s1 (latex session)");
    let expected = latex_all_along.eval(S2).expect("s2 (latex session)");

    let mut switched = Session::new();
    switched.eval(S1).expect("s1 (unicode session)");
    switched.set_format(OutputFormat::LaTeX);
    let got = switched.eval(S2).expect("s2 after set_format");

    assert_eq!(got, expected, "set_format must only change rendering");
    assert!(got.contains('\\'), "LaTeX output must carry LaTeX markup: {got}");
}

/// The discourse doc-claim, locked as a test: a pronoun in sentence 2
/// resolves against sentence 1's referents (both sentences eval cleanly in
/// one session).
#[test]
fn discourse_pronoun_resolves_across_evals() {
    let mut session = Session::new();
    let out1 = session.eval(S1).expect("s1 must compile");
    let out2 = session.eval(S2).expect("s2 must compile with resolved pronoun");
    assert!(!out1.is_empty() && !out2.is_empty());
    let history = session.history();
    assert!(history.contains(&out1) && history.contains(&out2));
}

/// Switching back and forth keeps working (no one-way latch).
#[test]
fn set_format_round_trip() {
    let mut session = Session::new();
    session.set_format(OutputFormat::LaTeX);
    let latex = session.eval("Every cat sleeps.").expect("latex eval");
    assert!(latex.contains("\\forall"), "latex: {latex}");
    session.set_format(OutputFormat::Unicode);
    let unicode = session.eval("Every dog barks.").expect("unicode eval");
    assert!(unicode.contains('∀'), "unicode: {unicode}");
}
