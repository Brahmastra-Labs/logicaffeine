//! ★ LOCK — `the todo's id` is a sentence, and LOGOS should read it as one. ★
//!
//! LOGOS reads as English everywhere else, and English puts an article in front of a possessed
//! noun. Only the BARE possessive parsed:
//!
//! ```text
//! Show todo's id.          →  works
//! Show the todo's id.      →  "I expected a statement here"
//! Set todo's id to 5.      →  works
//! Set the todo's id to 5.  →  "This construction needs the word 'to' here"
//! ```
//!
//! There was no reason behind that, and no decision either — it is a position nobody had covered.
//! The refusals are the worse half: neither message mentions the article, so the author reads
//! "I expected a statement" about a line that is plainly a statement, and the fix (delete a word
//! that reads correctly) is the one thing they will not think to try.
//!
//! ★ THE LOOKAHEAD IS WHAT MAKES IT UNAMBIGUOUS, and it has to be two tokens deep. `the count of
//! xs` is an EXISTING form whose `the` belongs to the form itself, so an article cannot simply be
//! skipped wherever one appears. The two are told apart by what follows the noun — `of` there,
//! `'s` here — so this fires only for `the <name>'s`, which no other form claims.
//! `the_count_of_still_means_what_it_meant` is the half of this file that would catch getting
//! that wrong.

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome};

const TODO: &str = "## A Todo has:\n\x20   an id, which is Int.\n\x20   a text, which is Text.\n\n";

fn prints(subject: &str, src: &str) -> String {
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "`{subject}` must run:\n{src}");
    out.output.trim().to_string()
}

#[test]
fn an_article_may_introduce_a_possessive() {
    let src = format!(
        "{TODO}## Main\nLet todo be a new Todo with id 7 and text \"a\".\nShow the todo's id.\n"
    );
    assert_eq!(prints("the todo's id", &src), "7");
}

#[test]
fn an_article_may_introduce_a_possessive_being_assigned() {
    let src = format!(
        "{TODO}## Main\nLet mutable todo be a new Todo with id 1 and text \"a\".\n\
         Set the todo's id to 9.\nShow todo's id.\n"
    );
    assert_eq!(prints("Set the todo's id", &src), "9");
}

#[test]
fn it_works_wherever_an_expression_does() {
    // Not just after `Show`: an argument, a return, a condition and an operand are all expression
    // positions, and a fix that landed in one of them would be the arbitrary-list failure this
    // codebase keeps finding.
    let src = format!(
        "{TODO}## To twice (n: Int) -> Int:\n\x20   Return n * 2.\n\n\
         ## To idOf (t: Todo) -> Int:\n\x20   Return the t's id.\n\n\
         ## Main\nLet todo be a new Todo with id 4 and text \"a\".\n\
         Show twice(the todo's id).\n\
         Show idOf(todo).\n\
         If the todo's id is greater than 3:\n\x20   Show \"big\".\n\
         Show the todo's id + 1.\n"
    );
    assert_eq!(prints("every expression position", &src), "8\n4\nbig\n5");
}

#[test]
fn a_chain_of_possessives_may_be_introduced_once() {
    let src = "## A Inner has:\n\x20   a v, which is Int.\n\n\
               ## A Outer has:\n\x20   an inner, which is Inner.\n\n\
               ## Main\nLet o be a new Outer with inner (a new Inner with v 3).\n\
               Show the o's inner's v.\n";
    assert_eq!(prints("a chain", src), "3");
}

#[test]
fn the_bare_possessive_is_unchanged() {
    // The floor. Every `x's y` in the standard library and the suite keeps meaning what it meant.
    let src = format!(
        "{TODO}## Main\nLet todo be a new Todo with id 2 and text \"b\".\n\
         Show todo's id.\nShow todo's text.\n"
    );
    assert_eq!(prints("the bare form", &src), "2\nb");
}

#[test]
fn the_count_of_still_means_what_it_meant() {
    // ★ THE ONE THAT WOULD CATCH AN OVER-EAGER SKIP. `the` here is part of the form, not an
    // article in front of a name — swallowing it would break a construct that has always worked.
    let src = "## Main\nLet xs be [1, 2, 3].\nShow the count of xs.\n";
    assert_eq!(prints("the count of", src), "3");
}

#[test]
fn an_article_before_something_that_is_not_a_possessive_is_still_refused() {
    // The skip is deliberately NARROW: `Show the x.` names no form and is not English anybody
    // means, so it keeps its refusal rather than being quietly accepted. Widening the article rule
    // to every expression is a separate decision with its own ambiguities, and taking it silently
    // here would be exactly the kind of unrequested change this codebase pushes back on.
    let out = tw_outcome("## Main\nLet x be 1.\nShow the x.\n");
    assert!(
        out.error.is_some(),
        "`Show the x.` must still be refused, got {:?}",
        out.output
    );
}

#[test]
fn it_builds_on_the_tier_a_server_ships_as() {
    let src = format!(
        "{TODO}## Main\nLet todo be a new Todo with id 7 and text \"a\".\nShow the todo's id.\n"
    );
    let rust = compile_to_rust(&src).unwrap_or_else(|e| panic!("must compile: {e:?}"));
    let refusals: Vec<&str> = rust
        .lines()
        .filter(|l| l.contains("compile_error!"))
        .collect();
    assert!(
        refusals.is_empty(),
        "would not build:\n{}",
        refusals.join("\n")
    );
}
