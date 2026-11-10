use super::*;

/// The lines around the next byte of `text`, as the model keeps them (lines end at `\n`).
fn around(text: &[u8]) -> Around {
    let starts: Vec<usize> = std::iter::once(0).chain(text.iter().enumerate().filter(|(_, &b)| b == b'\n').map(|(i, _)| i + 1)).collect();
    let line = starts[starts.len() - 1];
    let previous = if starts.len() >= 2 { starts[starts.len() - 2] } else { 0 };
    let previous2 = if starts.len() >= 3 { starts[starts.len() - 3] } else { 0 };
    let column = text.len() - line;
    let at = |start: usize, end: usize, i: usize| if start + i < end { u64::from(text[start + i]) | 256 } else { 0 };
    Around {
        column,
        above: at(previous, line, column),
        above_left: if column > 0 { at(previous, line, column - 1) } else { 0 },
        above2: at(previous2, previous, column),
        previous_length: (line - previous) as u64,
        previous2_length: (previous - previous2) as u64,
        line_start: line as u64,
        classes: 0,
        capital: false,
    }
}

fn fed(text: &[u8]) -> Lines {
    let mut l = Lines::new();
    for n in 1..=text.len() {
        l.update(text[n - 1], 1, &around(&text[..n]));
    }
    l
}

#[test]
fn a_line_counts_the_bytes_it_has_matched_the_line_above() {
    assert_eq!(fed(b"abcdef\nabc").matched, 3);
    assert!(fed(b"abcdef\nabc").hit, "the last byte matched its own");
    assert_eq!(fed(b"abcdef\nabX").matched, -1, "one byte off and the run is over");
    assert_eq!(fed(b"abcdef\nabXde").matched, -1, "and stays over for the line");
    assert_eq!(fed(b"abcdef\nabX\n").matched, 0, "a new line starts afresh");
    assert_eq!(fed(&[&b"x\n"[..], &[b'q'; 40]].concat()).matched, -1, "past the line above there is nothing to match");
    let long = [&[b'r'; 30][..], b"\n", &[b'r'; 30]].concat();
    assert_eq!(fed(&long).matched, LONGEST_MATCH, "the run is counted up to its cap");
}

#[test]
fn a_line_remembers_the_kind_of_byte_it_began_with() {
    let mut l = Lines::new();
    for (n, (b, class)) in [(b'\n', 0), (b' ', 0), (b' ', 0), (b'D', 2), (b'e', 1)].into_iter().enumerate() {
        let text = &b"\n  De"[..=n];
        l.update(b, class, &around(text));
    }
    assert_eq!(l.first, 3, "the first byte that is not white space, its class plus one");
    l.update(b'\n', 0, &around(b"\n  De\n"));
    assert_eq!(l.first, 0, "a newline forgets it");
}

#[test]
fn the_fixed_column_context_holds_only_where_the_two_lines_above_agree() {
    let keys = |text: &[u8]| fed(text).contexts(*text.last().unwrap(), 1, &around(text));
    let agree = keys(b"ab1\nab1\nab");
    let differ = keys(b"ab1\nab2\nab");
    assert_ne!(agree[3], 0x4C46_4931, "same byte two lines up and one up, same lengths: the column is fixed");
    assert_eq!(differ[3], 0x4C46_4931, "otherwise the context is the constant one");
    assert_eq!(keys(b"ab1x\nab1\nab")[3], 0x4C46_4931, "the same byte above two lines of other lengths is no fixed column");
    assert_ne!(agree[4], differ[4], "the bytes above and two up name their own context either way");
}

#[test]
fn each_line_is_a_context_of_its_own() {
    // Two lines with the same last byte at the same column share the column's context, not the line's: a line is
    // known by where it began.
    let a = fed(b"one\ntwo x").contexts(b'x', 1, &around(b"one\ntwo x"));
    let b = fed(b"ones\ntwo x").contexts(b'x', 1, &around(b"ones\ntwo x"));
    assert_eq!(a[6], b[6], "the column with the last byte is one context");
    assert_ne!(a[10], b[10], "the lines, begun at 4 and at 5, are two");
    assert_ne!(a[11], b[11]);
}
