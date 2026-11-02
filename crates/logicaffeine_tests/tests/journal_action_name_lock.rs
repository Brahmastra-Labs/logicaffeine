//! LOCK — an action named in `the journal shows A then B` or `Replay these actions: …` is an
//! action some component in the file actually declares, checked once every component is known.
//! Two defects closed together: a `To <name>:` spelled with a bare directional word (`up`, `down`,
//! `out`, `no`, …) lexes as a phrasal-verb PARTICLE rather than a Noun or Identifier, so it could
//! not be named in a journal assertion at all — the parser refused it as though it were not a
//! word; and a genuinely undeclared name was refused with no guess at what was meant.

#[path = "language_common/mod.rs"]
mod common;

use common::{accepted, refusal};

const APP: &str = "## Component App\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\n    To no:\n        Set n to 0.\nView:\n    <p on-click={bump}>{n}</p>\n\n## Main\nMount App.\n";

#[test]
fn a_particle_named_action_is_a_legal_journal_name() {
    let src = format!("{APP}Replay these actions: bump, no.\n");
    accepted(&src);
}

#[test]
fn a_particle_named_action_is_legal_in_the_journal_shows_form() {
    let src = format!(
        "{APP}## Test: replay order\nClick \"#0\".\nExpect that the journal shows bump then no.\n"
    );
    // Parses cleanly even though `bump then no` is a runtime nonsense assertion here — this test
    // is about the PARSE, not the run.
    let parsed = accepted(&src);
    assert!(!parsed.stmts.is_empty());
}

#[test]
fn an_undeclared_journal_action_is_refused_with_a_guess() {
    let src = format!("{APP}Replay these actions: bump, nosuch.\n");
    let (message, covered) = refusal(&src);
    assert_eq!(covered, "nosuch");
    assert!(message.contains("`nosuch` is not an action"), "{message}");
    assert!(message.contains("its actions are bump, no"), "{message}");
}

#[test]
fn a_near_miss_of_a_declared_action_suggests_it() {
    let src = format!("{APP}Replay these actions: bmup.\n");
    let (message, covered) = refusal(&src);
    assert_eq!(covered, "bmup");
    assert!(message.contains("did you mean `bump`?"), "{message}");
}

/// Every word the lexer or the lexicon gives a class of its own — the words that do NOT lex as a
/// plain identifier: each lowercase word literal in the lexer's source (its keyword arms: `add`,
/// `push`, `show`, …), and every entry of the lexicon's word classes with its listed forms
/// (keywords, articles, pronouns, prepositions, particles, adverbs, verbs, nouns, adjectives, …).
/// Read from the sources themselves, so a word keywordized tomorrow is in the census today.
fn every_classed_word() -> std::collections::BTreeSet<String> {
    let mut words = std::collections::BTreeSet::new();
    let mut add = |w: &str| {
        let w = w.to_lowercase();
        if !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase()) {
            words.insert(w);
        }
    };
    let lexer = include_str!("../../logicaffeine_language/src/lexer.rs");
    for piece in lexer.split('"').skip(1).step_by(2) {
        add(piece);
    }
    let lexicon: serde_json::Value =
        serde_json::from_str(logicaffeine_lexicon::LEXICON_JSON).expect("the lexicon is JSON");
    let lexicon = lexicon
        .as_object()
        .expect("the lexicon is one object of word classes");
    for (class, entries) in lexicon {
        match entries {
            serde_json::Value::Object(map)
                if !matches!(class.as_str(), "morphology" | "ontology" | "axioms") =>
            {
                for key in map.keys() {
                    add(key);
                }
            }
            serde_json::Value::Array(list) => {
                for entry in list {
                    match entry {
                        serde_json::Value::String(w) => add(w),
                        serde_json::Value::Object(e) => {
                            if let Some(lemma) = e.get("lemma").and_then(|l| l.as_str()) {
                                add(lemma);
                            }
                            if let Some(forms) = e.get("forms").and_then(|f| f.as_object()) {
                                for form in forms.values().filter_map(|f| f.as_str()) {
                                    add(form);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    words
}

/// `App`, declaring `name` as one of its actions, plus `tail` after the page's own mount.
fn app_declaring(name: &str, tail: &str) -> String {
    format!(
        "## Component App\nState:\n    Let n be 0.\nActions:\n    To {name}:\n        Set n to n + 1.\nView:\n    <p>{{n}}</p>\n\n## Main\nMount App.\n{tail}"
    )
}

/// ★ CENSUS — an action's name is written the same way wherever it is written. Whatever word an
/// `Actions:` header accepts in `To <name>:` — a keyword (`add`, `remove`, `push`, `show`), an
/// adjective (`double`, `empty`), a particle (`up`), a quantifier (`no`), a noun or a verb — names
/// that action in `Replay these actions: <name>.` and in `the journal shows <name> then <name>`,
/// and it names THE SAME action: the file's own check that every journal name is a declared action
/// accepts it.
#[test]
fn every_declarable_action_name_is_nameable_in_replay() {
    let mut declarable = 0usize;
    let mut unnameable = Vec::new();
    for word in every_classed_word() {
        if common::parse(&app_declaring(&word, "")).is_err() {
            continue;
        }
        declarable += 1;
        let replay = app_declaring(&word, &format!("Replay these actions: {word}.\n"));
        if let Err(e) = common::parse(&replay) {
            unnameable.push(format!(
                "Replay these actions: {word}.  →  {}",
                common::message_of(&e)
            ));
        }
        let shows = app_declaring(
            &word,
            &format!("\n## Test: the journal\nExpect that the journal shows {word} then {word}.\n"),
        );
        if let Err(e) = common::parse(&shows) {
            unnameable.push(format!(
                "the journal shows {word} then {word}  →  {}",
                common::message_of(&e)
            ));
        }
    }
    assert!(
        unnameable.is_empty(),
        "{} of {declarable} declarable action names cannot be named where an action is named:\n{}",
        unnameable.len(),
        unnameable.join("\n")
    );
    for must in [
        "add", "remove", "push", "show", "double", "empty", "up", "no",
    ] {
        assert!(
            common::parse(&app_declaring(must, "")).is_ok(),
            "`To {must}:` is a declarable action name, and the census must cover it"
        );
    }
    assert!(
        declarable >= 2_000,
        "the census checked only {declarable} declarable names"
    );
}

#[test]
fn an_action_declared_below_the_test_still_counts() {
    let src = "## Main\nReplay these actions: bump.\n\n\
        ## Component App\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\nView:\n    <p on-click={bump}>{n}</p>\n";
    accepted(src);
}
