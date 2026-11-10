use super::*;

fn fed(text: &[u8]) -> Code {
    let mut c = Code::new();
    let mut previous = 0;
    for &b in text {
        c.update(b, previous);
        previous = b;
    }
    c
}

/// An identifier's hash as the tracker spells it.
fn ident(word: &[u8]) -> u64 {
    word.iter().fold(0, |h, &b| hash2(h | 1, u64::from(b)))
}

#[test]
fn an_identifier_runs_over_letters_digits_and_underscores_and_keeps_its_case() {
    assert_eq!(fed(b"x = prs_uint32").ident, ident(b"prs_uint32"));
    let call = fed(b"prs_uint32(");
    assert_eq!((call.ident, call.prev), (0, ident(b"prs_uint32")), "a bracket ends it");
    assert_ne!(ident(b"Prs_uint32"), ident(b"prs_uint32"));
    assert_eq!(fed(b"x = SIVAL").ident, ident(b"SIVAL"), "its case is kept");
    let three = fed(b"a b c");
    assert_eq!((three.prev2, three.prev, three.ident), (ident(b"a"), ident(b"b"), ident(b"c")));
}

#[test]
fn a_number_is_its_own_class_but_a_digit_inside_an_identifier_is_not() {
    assert_eq!(fed(b" 4").classes & 0xF, NUMBER);
    assert_eq!(fed(b" 42").classes & 0xFF, NUMBER << 4 | IDENT, "a number's later digits read as an identifier's");
    assert_eq!(fed(b" x4").classes & 0xF, IDENT);
}

#[test]
fn brackets_count_by_kind_and_never_below_zero() {
    let c = fed(b"f(a[1], {b");
    assert_eq!((c.paren, c.square, c.brace), (1, 0, 1));
    assert_eq!(fed(b"f(a)").paren, 0);
    assert_eq!(fed(b"))) (").paren, 1, "a closing bracket with none open counts nothing");
    assert_eq!(fed(b"}}{").brace, 1);
}

#[test]
fn comments_and_strings_hold_their_brackets_and_close_where_they_end() {
    assert_eq!(fed(b"/* ( */ (").paren, 1);
    assert_eq!(fed(b"// ( \n (").paren, 1);
    assert_eq!(fed(b"\"(\" (").paren, 1);
    assert_eq!(fed(b"'(' (").paren, 1);
    assert_eq!(fed(b"\"a\\\"b\" (").paren, 1, "an escaped quote does not close the string");
    assert_eq!(fed(b"a * (b").paren, 1, "a star alone opens no comment");
    assert_eq!(fed(b"/* x").comment, BLOCK_COMMENT);
    assert_eq!(fed(b"/* x */").comment, NO_COMMENT);
    assert_eq!(fed(b"// x").comment, LINE_COMMENT);
    assert_eq!(fed(b"// x\n").comment, NO_COMMENT);
    assert_eq!(fed(b"\"x").string, b'"');
    assert_eq!(fed(b"\"x' y").string, b'"', "the other quote is a byte of the string");
}

#[test]
fn a_line_keeps_its_indentation_and_first_identifier() {
    let c = fed(b"\t  if (x");
    assert_eq!((c.indent, c.first), (10, ident(b"if")));
    let next = fed(b"\t  if (x\n");
    assert_eq!((next.indent, next.first), (0, 0), "a newline forgets both");
    assert_eq!(fed(b"if  x").indent, 0, "spaces after the first identifier are no indentation");
    assert_eq!(fed(b"if (x) y").first, ident(b"if"), "the first stays the first");
}

#[test]
fn a_token_class_enters_on_a_change_and_every_punctuation_enters() {
    assert_eq!(fed(b"ab  ((").classes & 0xFFFF, IDENT << 12 | SPACE << 8 | OPEN << 4 | OPEN);
    assert_eq!(fed(b"a;b,").punct & 0xFFFF, u64::from(b';') << 8 | u64::from(b','));
    assert_eq!(fed(b"a b").punct, 0, "spaces and identifiers are no punctuation");
}

#[test]
fn each_context_is_keyed_on_its_own_fields() {
    let base = fed(b"\tfoo(bar, baz");
    let keys = base.contexts(b'z');
    let changed = |c: &Code| -> Vec<usize> {
        let k = c.contexts(b'z');
        (0..CONTEXTS).filter(|&i| k[i] != keys[i]).collect()
    };
    let with = |f: &dyn Fn(&mut Code)| {
        let mut c = base.clone();
        f(&mut c);
        changed(&c)
    };
    assert_eq!(with(&|c| c.paren += 1), vec![2, 3, 7], "the bracket depths");
    assert_eq!(with(&|c| c.ident ^= 1), vec![0, 1, 4, 6], "the identifier so far");
    assert_eq!(with(&|c| c.prev ^= 1), vec![1, 4, 6, 7], "the last identifier");
    assert_eq!(with(&|c| c.prev2 ^= 1), vec![6], "the one before it");
    assert_eq!(with(&|c| c.first ^= 1), vec![5], "the line's first identifier");
    assert_eq!(with(&|c| c.indent += 1), vec![5], "the indentation");
    assert_eq!(with(&|c| c.comment = BLOCK_COMMENT), vec![0, 2, 7], "a comment open");
    assert_eq!(with(&|c| c.classes ^= 1), vec![2, 5], "the last token classes");
    assert_eq!(with(&|c| c.punct ^= 1), vec![3, 4], "the punctuation lately");
    assert_ne!(base.contexts(b'z')[3], base.contexts(b'y')[3], "and the last byte");
}
