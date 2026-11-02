//! ★ LOCK — a name that looks like a statement keyword is still a name. ★
//!
//! `add`, `show`, `set`, `check`, `one`, `count` are among the most natural short names a program
//! reaches for, and each is also the first word of a LOGOS sentence. When the parser could not tell
//! them apart, `Let add be 1.` was refused — and refused with "I expected a name here", which sends
//! the reader looking at their name rather than at the word they had collided with.
//!
//! It was recorded as three RED cases in `vm_global_promotion_lock` whose FIXTURES had to be
//! renamed to get past it. They no longer need renaming: this file is the proof, and it exists so
//! the next person adding a statement keyword finds out here rather than in someone's program.
//!
//! ★ THE PAIRING IS THE POINT. Each case uses the name AND the sentence that shares its word in the
//! same program, because a parser that resolved the ambiguity by dropping the STATEMENT would pass
//! every "the name works" test ever written while quietly breaking `Add 1 to xs.`.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// Both tiers run it and agree — a name resolved by the parser must mean the same thing wherever
/// the program is executed.
fn both(src: &str, expected: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "VM:\n{src}");
    assert_eq!(tw.output.trim_end(), expected, "tree-walker answer:\n{src}");
    assert_eq!(vm.output.trim_end(), expected, "VM answer:\n{src}");
}

#[test]
fn a_variable_may_be_called_add_beside_a_real_add_statement() {
    // ★ THE PAIRING: the NAME and the SENTENCE in one program. `Add v to c.` must still add.
    both(
        "## Main\nLet add be 5.\nLet mutable xs be a new Set of Int.\nAdd add to xs.\nShow length of xs.\nShow add.\n",
        "1\n5",
    );
}

#[test]
fn a_variable_may_be_called_show_beside_a_real_show_statement() {
    both("## Main\nLet show be 7.\nShow show.\n", "7");
}

#[test]
fn a_variable_may_be_called_set_beside_a_real_set_statement() {
    both(
        "## Main\nLet mutable set be 1.\nSet set to 2.\nShow set.\n",
        "2",
    );
}

#[test]
fn numeral_words_are_names_too() {
    // `one` and `two` are numeral WORDS the lexer knows; they are also the names a small example
    // reaches for first.
    both(
        "## Main\nLet one be 1.\nLet two be 2.\nShow one + two.\n",
        "3",
    );
}

#[test]
fn a_field_may_be_called_count_and_a_function_may_be_called_check() {
    // The same question in the two other positions a name appears in — a record FIELD and a
    // FUNCTION — because each is parsed by its own walk and they have gone wrong separately before
    // (a field named `i` or `it` silently did not exist until it was fixed).
    both(
        "## A Row has:\n\
        \x20   a count, which is Int.\n\
         \n\
         ## To check (r: Row) -> Int:\n\
        \x20   Return r's count.\n\
         \n\
         ## Main\n\
         Show check(a new Row with count 4).\n",
        "4",
    );
}

#[test]
fn a_keyword_shaped_name_survives_being_promoted_to_a_global() {
    // ★ THE CASE THAT RECORDED THIS. A name a FUNCTION writes is promoted to a module global, and
    // the promotion walk resolves it by name — so a keyword-shaped one is exactly where the two
    // resolutions could disagree. This is the fixture `vm_global_promotion_lock` had to rename.
    both(
        "## To bump ():\n\
        \x20   Set add to add + 1.\n\
         \n\
         ## Main\n\
         Let mutable add be 0.\n\
         bump().\n\
         bump().\n\
         Show add.\n",
        "2",
    );
}
