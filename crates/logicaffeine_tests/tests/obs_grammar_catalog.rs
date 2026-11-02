//! W6.1 — `obs_grammar_catalog`: the observation channel's grammar, locked.
//!
//! Logging is
//! a language primitive, not a library: `Note`/`Warn`/`Record that <clause>`
//! are speech-act statements (developer / operator / audit), payloads attach
//! via the prepositional grammar (`with the attempts`), and `During <gerund>:`
//! opens a lexical trace span. This locks the PARSE PRODUCTS (emission & sinks
//! are Wave 7); observation statements are legal anywhere, unlike test-only
//! `Expect`.

use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_language::ast::stmt::{ObsCategory, Stmt};

fn parse_result(source: &str) -> Result<(), String> {
    with_parsed_program(source, |parsed, _| match parsed {
        Ok(_) => Ok(()),
        Err(advice) => Err(advice),
    })
}

/// Collect (category, message, payload-name count) for every Observe in Main.
fn observes(source: &str) -> Vec<(ObsCategory, String, usize)> {
    with_parsed_program(source, |parsed, interner| {
        let (stmts, _t, _p) = parsed.expect("the program parses");
        stmts
            .iter()
            .filter_map(|s| match s {
                Stmt::Observe {
                    category,
                    message,
                    payloads,
                    ..
                } => Some((
                    *category,
                    interner.resolve(*message).to_string(),
                    payloads.len(),
                )),
                _ => None,
            })
            .collect()
    })
}

/// The three speech-act categories parse, each carrying its that-clause message.
#[test]
fn the_three_categories_parse() {
    let src = "## Main\n\
        \x20   Note that the cache missed.\n\
        \x20   Warn that the retry limit was reached.\n\
        \x20   Record that the payment settled.\n";
    let obs = observes(src);
    assert_eq!(obs.len(), 3, "three observation statements");
    assert_eq!(obs[0].0, ObsCategory::Note);
    assert_eq!(obs[1].0, ObsCategory::Warn);
    assert_eq!(obs[2].0, ObsCategory::Record);
    assert_eq!(
        obs[0].1, "the cache missed",
        "the that-clause is the message"
    );
}

/// Payloads attach via the prepositional `with … and …` grammar.
#[test]
fn payloads_attach_with_prepositions() {
    let src = "## Main\n\
        \x20   Let attempts be 3.\n\
        \x20   Let size be 42.\n\
        \x20   Warn that the upload failed with attempts and size.\n";
    let obs = observes(src);
    assert_eq!(obs.len(), 1);
    assert_eq!(
        obs[0].1, "the upload failed",
        "the message ends before `with`"
    );
    assert_eq!(obs[0].2, 2, "two payloads captured");
}

/// `Note once in every N times that …` captures a deterministic sample rate.
#[test]
fn sampling_phrase_captures_the_rate() {
    let src = "## Main\n\
        \x20   Note once in every 1024 times that the packet arrived.\n";
    let rate = with_parsed_program(src, |parsed, interner| {
        let (stmts, _t, _p) = parsed.expect("parses");
        stmts.iter().find_map(|s| match s {
            Stmt::Observe {
                sample, message, ..
            } => {
                assert_eq!(interner.resolve(*message), "the packet arrived");
                Some(*sample)
            }
            _ => None,
        })
    });
    assert_eq!(
        rate,
        Some(Some(1024)),
        "the 1-in-1024 sample rate is captured"
    );
}

/// `During <gerund>:` opens a lexical span whose body is ordinary code.
#[test]
fn during_opens_a_lexical_span() {
    let src = "## Main\n\
        \x20   During parsing the file:\n\
        \x20       Let x be 1.\n\
        \x20       Note that the header was read.\n";
    let count = with_parsed_program(src, |parsed, _| {
        let (stmts, _t, _p) = parsed.expect("parses");
        stmts
            .iter()
            .filter(|s| matches!(s, Stmt::During { .. }))
            .count()
    });
    assert_eq!(count, 1, "one span block");
}

/// Observation statements are legal ANYWHERE (not test-gated like `Expect`).
#[test]
fn observations_are_legal_outside_tests() {
    assert!(
        parse_result("## Main\n\x20   Note that hello happened.\n").is_ok(),
        "Note is a first-class statement, usable in any program"
    );
}

/// `## Note` the DOC BLOCK still parses as prose — the verb-statement and the
/// block header are disjoint surfaces.
#[test]
fn note_doc_block_still_works() {
    let src = "## Note\nThis module does things.\n\n## Main\n\x20   Show 1.\n";
    assert!(
        parse_result(src).is_ok(),
        "the ## Note doc block is untouched: {:?}",
        parse_result(src)
    );
}
