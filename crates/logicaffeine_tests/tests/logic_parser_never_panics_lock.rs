//! ★ LOCK — a sentence the logic parser cannot read is a parse error, never a panic. ★
//!
//! `largo logic "Where does the OS a machine installs come from?"` killed the process: a question
//! whose object carries a reduced relative clause (`the OS [that] a machine installs`) reached the
//! parser's verb reader with the relative clause's article where it expected the verb, and the verb
//! reader answered with `panic!("Expected verb, got Article(Indefinite)")`. The parser has a typed
//! error for exactly that (`ParseErrorKind::ExpectedVerb`); a reader that panics instead takes the
//! whole tool down with it, and a web page compiling a learner's sentence takes the tab.
//!
//! The lock is on the CONTRACT, not on one sentence: every logic entry point `largo logic` reaches
//! returns, a reading or an error, for the reported sentence and for the family of shapes around it
//! (other wh-words, other relative-clause subjects, the relative clause in subject position, a
//! declarative twin).

use std::panic::{self, AssertUnwindSafe};

use logicaffeine_language::compile::{
    compile, compile_all_scopes, compile_ambiguous, compile_discourse, compile_forest,
    compile_pragmatic, compile_simple,
};

/// The sentence the finding was reported with.
const REPORTED: &str = "Where does the OS a machine installs come from?";

/// The shapes around it: a reduced relative clause on the object of a question and of a statement.
fn family() -> Vec<String> {
    let mut out = vec![REPORTED.to_string()];
    for wh in ["Where", "When", "Why", "How"] {
        for np in [
            "the OS a machine installs",
            "the book a student reads",
            "the dog a man owns",
            "the program every machine runs",
            "the file the user saved",
        ] {
            out.push(format!("{wh} does {np} come from?"));
            out.push(format!("{wh} did {np} go?"));
        }
    }
    for s in [
        "What does the OS a machine installs need?",
        "Who wrote the book a student reads?",
        "Does the OS a machine installs come from a disk?",
        "The OS a machine installs comes from a disk.",
        "Every OS a machine installs comes from a disk.",
        "A machine installs the OS a vendor ships.",
    ] {
        out.push(s.to_string());
    }
    out
}

/// What one entry point did with one sentence: it returned (a reading or an error), or it panicked
/// with this message.
fn returns<F: FnOnce()>(f: F) -> Result<(), String> {
    panic::catch_unwind(AssertUnwindSafe(f)).map_err(|payload| {
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "a panic with no message".to_string())
    })
}

fn every_entry_point(sentence: &str) -> Vec<(&'static str, Result<(), String>)> {
    vec![
        ("compile", returns(|| drop(compile(sentence)))),
        ("compile_simple", returns(|| drop(compile_simple(sentence)))),
        ("compile_pragmatic", returns(|| drop(compile_pragmatic(sentence)))),
        ("compile_all_scopes", returns(|| drop(compile_all_scopes(sentence)))),
        ("compile_ambiguous", returns(|| drop(compile_ambiguous(sentence)))),
        ("compile_forest", returns(|| drop(compile_forest(sentence)))),
        ("compile_discourse", returns(|| drop(compile_discourse(&[sentence])))),
    ]
}

/// ★ THE LOAD-BEARING TEST — the reported sentence returns from `largo logic`'s own entry point.
#[test]
fn the_reported_question_returns_instead_of_panicking() {
    let outcome = returns(|| drop(compile(REPORTED)));
    assert_eq!(
        outcome,
        Ok(()),
        "`{REPORTED}` must give a reading or a parse error, never a panic"
    );
}

/// The contract on every logic entry point, over the whole family of shapes.
#[test]
fn no_logic_entry_point_panics_on_a_reduced_relative_clause() {
    let mut panics = Vec::new();
    for sentence in family() {
        for (entry, outcome) in every_entry_point(&sentence) {
            if let Err(why) = outcome {
                panics.push(format!("{entry}({sentence:?}) panicked: {why}"));
            }
        }
    }
    assert!(
        panics.is_empty(),
        "every sentence returns a reading or an error from every entry point:\n{}",
        panics.join("\n")
    );
}
