//! The Simon STORY must round-trip from English to FOL once the PuzzleBaron
//! boilerplate is stripped. The raw `story` field carries a "Backstory and Goal"
//! header, smart-quotes, and a trailing "If you get stuck …" UI-help sentence that
//! are NOT proper English; `clean_story` removes them, leaving the three meaningful
//! sentences (narrative, goal, all-different), each of which must compile.

use logicaffeine_language::compile;
use serde::Deserialize;

#[derive(Deserialize)]
struct Puzzle {
    story: String,
}

/// Strip PuzzleBaron boilerplate + typography, returning the meaningful sentences.
/// General across the corpus: every PuzzleBaron story has the same header and the
/// same "If you get stuck …" help footer.
fn clean_story(raw: &str) -> Vec<String> {
    let unquoted = raw.replace(['\u{201c}', '\u{201d}', '"'], "");
    let deheadered = unquoted.replace("Backstory and Goal ", "");
    // The all-different rule carries a uniform PuzzleBaron preamble framing it as a
    // reminder; strip it to expose the rule itself ("no option … more than once").
    let depreambled = deheadered.replace("Remember, as with all grid-based logic puzzles, ", "");
    depreambled
        .split(". ")
        .map(|s| s.trim().trim_end_matches('.').trim())
        .filter(|s| !s.is_empty())
        .filter(|s| !s.starts_with("If you get stuck"))
        .map(|s| format!("{s}."))
        .collect()
}

fn load_story() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../puzzles/3cf932ef3e458bf4fcba3081d831f5ae.json"
    );
    let p: Puzzle = serde_json::from_str(&std::fs::read_to_string(path).expect("read"))
        .expect("parse JSON");
    p.story
}

#[test]
fn clean_story_strips_boilerplate_to_three_sentences() {
    let sents = clean_story(&load_story());
    assert_eq!(sents.len(), 3, "narrative + goal + all-different; got: {sents:?}");
    assert!(sents[0].starts_with("Every year Simon takes"), "narrative; got: {:?}", sents[0]);
    assert!(sents[1].starts_with("Determine each trip"), "goal; got: {:?}", sents[1]);
    assert!(sents[2].contains("no option in any category"), "all-different; got: {:?}", sents[2]);
    assert!(!sents.iter().any(|s| s.contains("Clear Errors")), "UI help dropped");
    assert!(!sents.iter().any(|s| s.contains("Backstory")), "header dropped");
}

#[test]
fn story_narrative_parses() {
    let sents = clean_story(&load_story());
    let out = compile(&sents[0]).unwrap_or_else(|e| panic!("narrative {:?}: {e:?}", sents[0]));
    // The indefinite noun-noun compound "adventure holiday" deliberately fuses to
    // `Adventure_holiday` (same rule that keeps "a yoga regimen" → `Yoga_regimen`,
    // pinned by phase_puzzle_parse), so the holiday content is the lowercase tail.
    for needle in ["Simon", "Adventure_holiday", "Friend", "Location"] {
        assert!(out.contains(needle), "narrative lost {needle}; got: {out}");
    }
}

#[test]
fn story_goal_parses() {
    let sents = clean_story(&load_story());
    let out = compile(&sents[1]).unwrap_or_else(|e| panic!("goal {:?}: {e:?}", sents[1]));
    for needle in ["Activity", "State", "Year", "Friend"] {
        assert!(out.contains(needle), "goal lost {needle}; got: {out}");
    }
}

#[test]
fn story_all_different_parses() {
    let sents = clean_story(&load_story());
    let out = compile(&sents[2]).unwrap_or_else(|e| panic!("all-different {:?}: {e:?}", sents[2]));
    assert!(out.contains("Option") || out.contains("Category"), "all-different content; got: {out}");
}
