use super::*;

fn fed(text: &[u8]) -> Prose {
    let mut p = Prose::new();
    for &b in text {
        p.update(b);
    }
    p
}

fn w(s: &[u8]) -> u64 {
    s.iter().fold(0, |h, &b| hash2(h, u64::from(b.to_ascii_lowercase())))
}

#[test]
fn a_sentence_keeps_its_own_words_and_a_full_stop_clears_them() {
    let p = fed(b"The cat sat on");
    assert_eq!(p.word, w(b"on"));
    assert_eq!(p.sentence, [w(b"sat"), w(b"cat")]);
    let p = fed(b"The cat sat. A dog");
    assert_eq!(p.sentence, [w(b"a"), 0], "the words before the full stop are gone");
    for end in [b'!', b'?'] {
        let p = fed(&[b"It ran" as &[u8], &[end], b" Then"].concat());
        assert_eq!(p.sentence, [0, 0], "{} ends a sentence", end as char);
    }
}

#[test]
fn a_phrase_runs_over_single_spaces_and_quotes_and_ends_at_any_other_mark() {
    let p = fed(b"over the \"hill\" and far");
    assert_eq!(p.phrase, [w(b"far"), w(b"and"), w(b"hill"), w(b"the"), w(b"over")]);
    let p = fed(b"over the hill, and far");
    assert_eq!(p.phrase, [w(b"far"), w(b"and"), 0, 0, 0], "a comma starts a new phrase");
    let p = fed(b"a b c d e f g");
    assert_eq!(p.phrase, [w(b"g"), w(b"f"), w(b"e"), w(b"d"), w(b"c")], "the five latest words");
}

#[test]
fn a_words_letter_pattern_counts_th_as_one_kind() {
    assert_eq!(fed(b"they").pattern, u32::from_be_bytes([0, b't', b'e', b'y']));
    assert_eq!(fed(b"stray").pattern, u32::from_be_bytes([b'b', b'b', b'a', b'y']), "the last four letters");
    assert_eq!(fed(b"at the").pattern, u32::from_be_bytes([0, 0, b't', b'e']), "a new word starts its own");
    assert_eq!(fed(b"t h").pattern, u32::from(b'b'), "an h after a word break is a consonant of its own");
    assert_eq!(fed(b"they").len, 4);
}

#[test]
fn quotes_and_keywords_are_remembered() {
    assert_eq!(fed(b"he said (quietly").opened, b'(');
    assert_eq!(fed(b"he said (quietly) and").opened, 0);
    assert_eq!(fed(b"\"Come in").opened, b'"');
    assert_eq!(fed(b"name: value").keyword, w(b"name"));
    assert_eq!(fed(b"width=3 height").keyword, w(b"width"));
}

#[test]
fn a_blank_line_starts_a_paragraph_and_a_newline_a_line() {
    let one = fed(b"First words here.\nStill the first.");
    assert_eq!(one.paragraph, 0, "one newline does not start a paragraph");
    let two = fed(b"First words here.\nStill the first.\n\nSecond");
    assert_eq!(two.paragraph, 36, "a blank line does: the paragraph starts after it");
    assert_eq!(fed(b"First words here.\nStill the first.\n \n  Second").paragraph, 37, "white space between is still blank");
    assert_eq!(fed(b"First.\n\nSecond.\n\nThird").paragraph, 17);
    let p = fed(b"ab\n   Cd");
    assert_eq!((p.line_first, p.column), (b'A', 5), "the line began with a capital, five bytes ago");
    assert_eq!(fed(b"ab\n").line_first, 0, "nothing on the new line yet");
    assert_eq!(fed(b"ab\n  ").line_first, 0, "nor white space");
}

#[test]
fn the_group_history_folds_letters_and_digits() {
    let p = fed(b"aB3,\xC3");
    assert_eq!(p.groups & 0xFF_FFFF_FFFF, u64::from_be_bytes([0, 0, 0, b'a', b'A', b'0', b',', 128]));
}

#[test]
fn the_sentence_context_tells_a_word_apart_by_the_sentence_around_it() {
    // One word after one word: in the same sentence the context names the word before; after a full stop it does not.
    let keys = |text: &[u8]| fed(text).contexts(b'x', b'y', 0, 0, 0);
    let a = keys(b"red fox");
    let b = keys(b"red. fox");
    let c = keys(b"blue fox");
    assert_ne!(a[0], b[0]);
    assert_ne!(a[0], c[0]);
    assert_eq!(b[0], keys(b"blue. fox")[0], "after a full stop the sentence so far is the word alone");
}
