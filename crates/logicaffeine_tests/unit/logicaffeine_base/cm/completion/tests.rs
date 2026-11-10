use super::*;
use crate::entropy::hash2;

/// `text` fed as the model feeds it: each byte into the history, the prefix before it noted, the word updated.
fn fed(text: &[u8]) -> (Completion, Vec<u8>, u64, u32) {
    let mut c = Completion::new();
    let (mut history, mut word, mut len) = (Vec::new(), 0u64, 0u32);
    for &b in text {
        history.push(b);
        c.update(word, &history);
        if b.is_ascii_alphabetic() {
            word = hash2(word, u64::from(b.to_ascii_lowercase()));
            len += 1;
        } else {
            word = 0;
            len = 0;
        }
    }
    (c, history, word, len)
}

/// What the tracker predicts after `text`: the byte that followed the word so far last time, and its run.
fn predicted(text: &[u8]) -> Option<(u8, u8)> {
    let (c, history, word, _) = fed(text);
    let k = Completion::slot(word);
    (word != 0 && c.at[k] > 0).then(|| (history[c.at[k] as usize], c.run[k]))
}

#[test]
fn a_prefix_predicts_the_byte_that_followed_it_last_time() {
    assert_eq!(predicted(b"cat cab ca"), Some((b'b', 0)), "the last continuation, not the first");
    assert_eq!(predicted(b"cat ca"), Some((b't', 0)));
    assert_eq!(predicted(b"Cat ca"), Some((b't', 0)), "prefixes fold their case");
    assert_eq!(predicted(b"cat c"), Some((b'a', 0)));
    assert_eq!(predicted(b"cat do"), None, "a prefix never seen predicts nothing");
    assert_eq!(predicted(b"cat "), None, "between words there is no prefix");
    assert_eq!(predicted(b"cat cat"), Some((b' ', 0)), "a whole word predicts what ended it");
}

#[test]
fn the_run_counts_how_often_the_byte_came_again_and_a_different_one_resets_it() {
    assert_eq!(predicted(b"ab ab ab a"), Some((b'b', 2)));
    assert_eq!(predicted(b"ab ab ac a"), Some((b'c', 0)));
    let long = b"ab ".repeat(40);
    assert_eq!(predicted(&[&long[..], b"a"].concat()).map(|p| p.1), Some(LONGEST_RUN), "up to its cap");
}

#[test]
fn each_context_is_keyed_on_what_it_says() {
    let keys = |text: &[u8]| {
        let (c, history, word, len) = fed(text);
        c.contexts(word, len, &history)
    };
    let (b, t) = (keys(b"cab ca"), keys(b"cat ca"));
    assert!(b[0] != t[0] && b[1] != t[1], "the byte predicted");
    let (once, twice) = (keys(b"cat ca"), keys(b"cat cat ca"));
    assert!(once[0] != twice[0] && once[1] != twice[1], "the run");
    // The same byte predicted with the same run, after a prefix of one letter and of two.
    let (one, two) = (keys(b"ab a"), keys(b"xab xa"));
    assert_eq!((predicted(b"ab a"), predicted(b"xab xa")), (Some((b'b', 0)), Some((b'b', 0))));
    assert_ne!(one[0], two[0], "the word's length");
    let none = keys(b"cat ");
    assert_eq!(none, keys(b"dog "), "no prefix, the same keys");
}
