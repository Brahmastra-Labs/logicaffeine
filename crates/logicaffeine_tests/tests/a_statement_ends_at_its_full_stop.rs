//! The abbreviation rule reads the dot after a known abbreviation ("Mr. Smith",
//! "153 ft.") as part of the word when more text follows — right for prose,
//! wrong for code. In imperative code every word is an identifier and a full
//! stop ends its statement, so a variable named `m`, `no` or `st` ends its
//! sentence like any other: the period is a token, and the statement's span
//! covers it.

use logicaffeine_base::Interner;
use logicaffeine_language::{token::TokenType, Lexer};

fn periods(source: &str) -> usize {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    lexer.tokenize().iter().filter(|t| t.kind == TokenType::Period).count()
}

#[test]
fn a_variable_named_like_an_abbreviation_still_ends_its_statement() {
    for name in ["m", "no", "st", "ft", "dr", "sq", "ml", "mr", "vs", "approx"] {
        let source = format!("## Main\nLet {name} be 5.\nShow {name}.\nShow 1.\n");
        assert_eq!(periods(&source), 3, "every statement keeps its full stop in {source:?}");
    }
}

#[test]
fn a_function_returning_an_abbreviation_named_value_keeps_its_full_stop() {
    let source = "## To f (m: Int) -> Int:\n    Return m.\n\n## Main\nShow f(1).\n";
    assert_eq!(periods(source), 2, "`Return m.` and `Show f(1).`");
}

#[test]
fn prose_still_reads_an_abbreviations_dot_as_part_of_the_word() {
    assert_eq!(periods("Mr. Smith is a doctor."), 1, "one sentence, one full stop");
    assert_eq!(periods("The tower is 153 ft. tall."), 1, "one sentence, one full stop");
}
