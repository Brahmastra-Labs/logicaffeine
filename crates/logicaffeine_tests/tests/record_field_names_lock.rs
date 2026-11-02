//! Every word an author can write as a field name is a field, or the declaration fails — a
//! record must never come out QUIETLY MISSING one.
//!
//! ★ THE BUG THIS EXISTS FOR: the field loop consumed the article, failed to accept the name, and
//! then ran NEITHER of its two branches — no field pushed, no token advanced. The next iteration
//! skipped the word, the rest of the line parsed as nothing, and the record simply did not have
//! that field. `an i, which is Int.` and `an it, which is Option of Inner.` both vanished, and
//! `i` (an index) and `it` (an item) are among the most natural short field names there are.
//!
//! ★ AND IT WAS SILENT AT BOTH ENDS: no error at the declaration, and none at the construction
//! either — `a new Row with it 4` set nothing and reported success. The first sign was a value
//! missing from the far end of a wire, which is the worst place to learn it.
//!
//! Written in PLAIN LOGOS, with no JSON anywhere, because that is where the gap is reachable: it
//! was FOUND through the derived encoder, but a lock on the encoder would pin the symptom and
//! leave the cause. The same rule `wasm_record_global_lock` follows.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

fn both(src: &str, want: &[&str]) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker must run it:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM must run it:\n{src}");
    assert_eq!(
        tw.output.lines().collect::<Vec<_>>(),
        want,
        "tree-walker:\n{src}"
    );
    assert_eq!(vm.output.lines().collect::<Vec<_>>(), want, "VM:\n{src}");
}

/// `a <name>, which is Int.` declared, constructed, and read back.
fn round_trip(name: &str) -> String {
    format!(
        "## A Row has:\n\
        \x20   a {name}, which is Int.\n\
        \x20   a flag, which is Bool.\n\
         \n\
         ## Main\n\
         Let r be a new Row with {name} 4 and flag true.\n\
         Show r's {name}.\n"
    )
}

#[test]
fn a_pronoun_is_a_field_name() {
    // `it` is the word an author reaches for when a record holds one of something.
    both(&round_trip("it"), &["4"]);
}

#[test]
fn the_single_letter_index_name_is_a_field_name() {
    // `i` — the most-typed identifier in programming — declared a field that did not exist.
    both(&round_trip("i"), &["4"]);
}

#[test]
fn every_pronoun_the_lexicon_knows_survives_a_declaration() {
    // Swept rather than sampled: the fix accepts the token CLASS, so the lock has to stand on the
    // class. A future pronoun added to the lexicon is covered without anyone remembering to.
    for name in [
        "i", "it", "its", "he", "she", "him", "her", "they", "them", "we", "us", "you",
    ] {
        let src = round_trip(name);
        let tw = tw_outcome(&src);
        assert_eq!(
            tw.error, None,
            "`a {name}, which is Int.` must declare a field:\n{src}"
        );
        assert_eq!(
            tw.output.trim(),
            "4",
            "the field `{name}` must hold what was constructed into it — a record that comes out \
             missing a field its author declared is the silent failure this lock exists for"
        );
    }
}

/// ★ THE INTERROGATIVES ARE PRONOUNS TOO, and the sweep above missed them — it listed the
/// PERSONAL pronouns and stopped, so `who`, `what` and `whom` still vanished exactly as `i`
/// and `it` once did: declared, silently absent, and blamed at a later USE.
///
/// Found by writing a guestbook. `who` is the word anybody reaches for on a record that
/// records a person — a comment, a signature, an audit line, an event — so this is not an
/// exotic corner, it is the first field name of a very ordinary record.
#[test]
fn an_interrogative_pronoun_is_a_field_name() {
    for name in ["who", "what", "whom", "whose", "which"] {
        let src = round_trip(name);
        let tw = tw_outcome(&src);
        assert_eq!(
            tw.error, None,
            "`a {name}, which is Int.` must declare a field — an interrogative is as ordinary a              field name as any other word:\n{src}"
        );
        assert_eq!(tw.output.trim(), "4", "field `{name}`");
    }
}

/// ★ THE FIX IS THE POSITION, NOT A LONGER LIST OF WORDS — so the lock stands on the
/// position too. `i` and `it` were closed once by teaching the acceptor about pronouns;
/// that fixed two words and left the class open, which is how `who` survived to be found
/// by somebody writing a guestbook. After an article, an alphabetic token IS the field
/// name, whatever part of speech the lexer assigned it.
///
/// The sweep is deliberately of words from DIFFERENT token classes — an interrogative, a
/// modal, a quantifier, a temporal word, a logical connective — because a fix that keyed
/// on any one of them would pass a narrower version of this test.
#[test]
fn a_word_of_any_class_can_be_a_field_name() {
    for name in [
        "who", "what", "whom", "why", "must", "all", "some", "then", "not", "and", "of",
    ] {
        let src = round_trip(name);
        let tw = tw_outcome(&src);
        assert_eq!(
            tw.error, None,
            "`a {name}, which is Int.` must declare a field. A word the acceptor does not \
             recognise used to push NO field and consume NO token, so the field vanished and \
             the error surfaced later as `has no field named '{name}'` — blaming a READ for a \
             declaration that reported success.\n{src}"
        );
        assert_eq!(
            tw.output.trim(),
            "4",
            "field `{name}` must hold what was constructed into it"
        );
    }
}

#[test]
fn the_ordinary_names_still_work() {
    // The acceptor got WIDER, and a wider acceptor is only safe if it still accepts everything it
    // used to. Nouns, adjectives, adverbs and verbs were already field names.
    for name in ["id", "text", "count", "next", "item", "x", "n", "of", "to"] {
        let src = round_trip(name);
        let tw = tw_outcome(&src);
        assert_eq!(
            tw.error, None,
            "`a {name}, which is Int.` must still declare a field:\n{src}"
        );
        assert_eq!(tw.output.trim(), "4", "field `{name}`");
    }
}

#[test]
fn a_pronoun_field_carries_a_declared_type_rather_than_unknown() {
    // The name is only half of it: the loop that dropped the field also never reached the `, which
    // is <Type>` half, so a field that DID survive by another route could still land typed
    // `Unknown`. Text is the type that shows it — an `Unknown` field would not concatenate.
    both(
        "## A Row has:\n\
        \x20   an it, which is Text.\n\
         \n\
         ## Main\n\
         Let r be a new Row with it \"hi\".\n\
         Show r's it + \"!\".\n",
        &["hi!"],
    );
}

#[test]
fn a_pronoun_field_is_mutable_like_any_other() {
    // A field that exists must be a field in every way, not a read-only artifact of the fix.
    both(
        "## A Row has:\n\
        \x20   an i, which is Int.\n\
         \n\
         ## Main\n\
         Let mutable r be a new Row with i 1.\n\
         Set r's i to 9.\n\
         Show r's i.\n",
        &["9"],
    );
}

#[test]
fn two_pronoun_fields_stay_distinct() {
    // They are accepted by their LEXEME, so two different pronouns must not collapse into one
    // name — which is exactly what would happen if the fix had used a shared lemma instead.
    both(
        "## A Row has:\n\
        \x20   an i, which is Int.\n\
        \x20   an it, which is Int.\n\
         \n\
         ## Main\n\
         Let r be a new Row with i 1 and it 2.\n\
         Show r's i.\n\
         Show r's it.\n",
        &["1", "2"],
    );
}
