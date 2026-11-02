//! ★ LOCK — `Call <name>.` accepts ANY declared function name, whatever word class the lexer
//! gives it. ★
//!
//! `crates/logicaffeine_language/src/parser/mod.rs::parse_call_statement`'s callee position used
//! to match only `Noun`/`Adjective`/`Verb`/`Ambiguous` tokens — a NARROWER set than the "identifier
//! freedom" the expression-position primary-expr parser already grants `Performative`/`Focus`/
//! `Particle`/`CalendarUnit` tokens (`X()` already worked for the same names). A `## To <name>:`
//! header accepts any of these words as a function name; `Call <name>.` refused the ones that
//! happen to lex as a speech-act verb (`announce`, `declare`, `warn` — `TokenType::Performative`)
//! or a focus particle (`only` — `TokenType::Focus`) with `ExpectedIdentifier`.
//!
//! **The census, not a hand list**: every verb `crates/logicaffeine_lexicon/assets/lexicon.json`
//! tags `"Performative"` (read directly from the SAME JSON `build.rs` derives `is_performative`
//! from — a verb added there gets covered here automatically, with no list to update), plus every
//! word that lexes as a `Focus` particle (`only`/`even`/`just` — `FocusKind::Cleft` has no single
//! lexical trigger of its own, so it is not a `Call`-callee case at all and is excluded, noted
//! rather than silently dropped).
//!
//! **The second, independent bug** this surfaced: `aOnly`/`bOnly` (ordinary camelCase function
//! names) refused too, not because of `Performative`/`Focus` at all — probed the lexer directly
//! and found `TokenType::Adverb`. `crates/logicaffeine_language/src/lexer.rs`'s prose "any word
//! ending in `-ly` is probably an adverb" heuristic (guarded against capitalized PROPER names like
//! "Billy", but not against an INTERNAL capital) fired on `bonly`/`aonly` (both end in `ly`, len >
//! 4) exactly as it would for a real English adverb — `onlyB` never reaches that heuristic at all
//! (it does not end in `ly`), so it "worked" only by accident of spelling, never by a real rule.
//! The fix adds a camelCase guard (an internal capital letter is never a natural English word) so
//! `aOnly`/`bOnly` fall through to the SAME plain-identifier classification `onlyB` already gets —
//! by the SAME rule this time, not by accident.

mod common;

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

fn both_tiers(src: &str, want: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "VM:\n{src}");
    assert_eq!(tw.output.trim(), want, "tree-walker:\n{src}");
    assert_eq!(vm.output.trim(), want, "VM:\n{src}");
}

/// `Call <name>.` on a `## To <name>:` declaring it, on both tree-walker and VM.
fn call_statement_form(name: &str) {
    let src = format!("## To {name}:\n    Show \"ok\".\n\n## Main\nCall {name}.\n");
    both_tiers(&src, "ok");
}

/// The expression form, `Show <name>().` — already worked before this fix; pinned alongside the
/// statement form so the two can never drift apart again.
fn call_expression_form(name: &str) {
    // A bare `Show "ok".` function has no explicit `Return`, so calling it as an EXPRESSION also
    // prints its own implicit `Nothing` result — correct, and irrelevant to what this checks
    // (that the callee parses and runs at all, which is the whole property in question).
    let src = format!(
        "## To {name} () -> Int:\n    Show \"ok\".\n    Return 1.\n\n## Main\nShow {name}().\n"
    );
    both_tiers(&src, "ok\n1");
}

/// Every verb the lexicon's own data tags `"Performative"` — read straight from
/// `logicaffeine_lexicon::LEXICON_JSON`, the exact source `build.rs` derives `is_performative`
/// from, so a verb added there is covered here with no list to edit.
fn performative_verbs() -> Vec<String> {
    let data: serde_json::Value =
        serde_json::from_str(logicaffeine_lexicon::LEXICON_JSON).expect("lexicon.json parses");
    data["verbs"]
        .as_array()
        .expect("lexicon.json has a `verbs` array")
        .iter()
        .filter(|v| {
            v["features"]
                .as_array()
                .is_some_and(|fs| fs.iter().any(|f| f == "Performative"))
        })
        .map(|v| {
            v["lemma"]
                .as_str()
                .expect("verb has a lemma")
                .to_lowercase()
        })
        .collect()
}

/// Every single WORD that lexes as a `Focus` particle — `FocusKind::Cleft` has no lexical
/// trigger of its own (it-cleft is a SYNTACTIC pattern, "It was X who…", never one word), so it
/// is not a `Call`-callee case and is deliberately not in this list.
const FOCUS_WORDS: &[&str] = &["only", "even", "just"];

/// Named directly in the bug report: refused before this fix.
const ORIGINALLY_REFUSED: &[&str] = &["announce", "declare", "warn", "only", "aOnly", "bOnly"];

/// Named directly in the bug report: already fine before this fix — pinned so the fix cannot
/// narrow what already worked.
const ALREADY_FINE: &[&str] = &[
    "greet",
    "report",
    "helper",
    "chime",
    "notify",
    "tick",
    "bump",
    "wait",
    "run",
    "print",
    "describe",
    "summarize",
    "bAlso",
    "onlyB",
];

#[test]
fn every_performative_verb_the_lexicon_tags_can_be_called() {
    let verbs = performative_verbs();
    assert!(verbs.len() > 15, "the census must be real, got {verbs:?}");
    for v in &verbs {
        call_statement_form(v);
    }
}

#[test]
fn every_focus_word_can_be_called() {
    for w in FOCUS_WORDS {
        call_statement_form(w);
    }
}

#[test]
fn a_camelcase_name_ending_in_a_focus_words_spelling_can_be_called() {
    call_statement_form("aOnly");
    call_statement_form("bOnly");
}

#[test]
fn the_already_fine_names_stay_fine() {
    for name in ALREADY_FINE {
        call_statement_form(name);
    }
}

#[test]
fn the_expression_form_is_unaffected_for_every_originally_refused_name() {
    for name in ORIGINALLY_REFUSED {
        call_expression_form(name);
    }
}

#[test]
fn a_camelcase_only_suffixed_name_lexes_as_a_plain_identifier_not_an_adverb() {
    // The lexer-level regression, independent of `Call`: `bOnly`/`aOnly` must classify the SAME
    // way `onlyB` already does (a plain identifier-like token), not as `TokenType::Adverb`.
    use logicaffeine_base::Interner;
    use logicaffeine_language::{Lexer, TokenType};
    for word in ["aOnly", "bOnly", "onlyB"] {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new(word, &mut interner);
        let tokens = lexer.tokenize();
        assert!(
            !matches!(tokens[0].kind, TokenType::Adverb(_)),
            "{word:?} lexed as Adverb: {:?}",
            tokens[0].kind
        );
    }
}

/// AOT, where it applies — the originally-refused set specifically (a full-JSON-census AOT sweep
/// would spin up dozens of `cargo build`s; the statement/expression tests above already cover
/// every performative verb and focus word on the tree-walker and VM).
#[test]
fn the_originally_refused_names_also_compile_and_run_through_aot() {
    for name in ORIGINALLY_REFUSED {
        let src = format!("## To {name}:\n    Show \"ok\".\n\n## Main\nCall {name}.\n");
        common::assert_compiled_equals_interpreted(&src);
    }
}
