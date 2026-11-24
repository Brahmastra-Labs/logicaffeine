//! `IntVec`: machine words until a value past the word is stored, then, once, every element exact.
use super::*;

fn big() -> LogosInt {
    LogosInt::from_literal("1180591620717411303424")
}

#[test]
fn a_list_of_words_stays_words_and_reads_its_words() {
    let mut v = IntVec::from_words(vec![1, 2, 3]);
    v.push_word(4);
    v.set_word(0, 9);
    assert!(!v.is_exact());
    assert_eq!(v.len(), 4);
    assert_eq!((v.get_word(0), v.get_word(3)), (9, 4));
    assert_eq!(v.get_ck(1), Some(2));
    assert_eq!(v.get_int(2), LogosInt::from_i64(3));
}

#[test]
fn a_value_past_the_word_promotes_the_list_once_and_keeps_every_element() {
    let mut v = IntVec::from_words(vec![i64::MAX, -7]);
    v.push_int(LogosInt::from_i64(5));
    assert!(!v.is_exact(), "a value that fits a word does not promote");
    v.push_int(big());
    assert!(v.is_exact());
    assert_eq!(v.len(), 4);
    assert_eq!(v.get_int(0), LogosInt::from_i64(i64::MAX));
    assert_eq!(v.get_int(1), LogosInt::from_i64(-7));
    assert_eq!(v.get_int(3), big());
    assert_eq!(v.get_ck(0), None, "a promoted list leaves the word path");
    v.push_word(8);
    v.set_int(1, big().add(&big()));
    assert_eq!(v.get_int(4), LogosInt::from_i64(8));
    assert_eq!(v.get_int(1), LogosInt::from_literal("2361183241434822606848"));
}

#[test]
fn set_promotes_as_push_does() {
    let mut v = IntVec::from_words(vec![1, 2]);
    v.set_int(0, big());
    assert!(v.is_exact());
    assert_eq!((v.get_int(0), v.get_int(1)), (big(), LogosInt::from_i64(2)));
}

#[test]
#[should_panic(expected = "does not fit a 64-bit Int")]
fn a_word_read_of_a_value_past_the_word_stops_loudly() {
    let mut v = IntVec::new();
    v.push_int(big());
    v.get_word(0);
}

#[test]
fn a_word_read_of_a_promoted_value_within_the_word_is_that_word() {
    let mut v = IntVec::from_words(vec![3]);
    v.push_int(big());
    assert_eq!(v.get_word(0), 3);
}

#[test]
fn contains_compares_by_value_in_either_form() {
    let mut v = IntVec::from_words(vec![1, 2]);
    assert!(v.contains_int(&LogosInt::from_i64(2)));
    assert!(!v.contains_int(&big()));
    v.push_int(big());
    assert!(v.contains_int(&big()), "two equal big integers are equal");
    assert!(v.contains_int(&LogosInt::from_i64(1)));
    assert!(!v.contains_int(&LogosInt::from_i64(3)));
}

#[test]
fn a_clone_is_its_own_list_and_equality_reads_values() {
    let mut a = IntVec::from_words(vec![1]);
    let b = a.clone();
    a.push_int(big());
    assert_eq!(b.len(), 1);
    assert!(!b.is_exact());
    let mut c = IntVec::from_words(vec![1]);
    c.push_int(big());
    assert_eq!(a, c, "equal elements, whatever object holds the big one");
    let mut words = IntVec::from_words(vec![1, 2]);
    let mut promoted = IntVec::from_words(vec![1]);
    promoted.push_int(big());
    promoted.set_int(1, LogosInt::from_i64(2));
    assert!(promoted.is_exact());
    words.set_word(0, 1);
    assert_eq!(words, promoted, "a promoted list equals a word list of the same values");
}

#[test]
fn elements_iterate_as_exact_integers() {
    let mut v = IntVec::from_words(vec![4, 5]);
    v.push_int(big());
    let all: Vec<LogosInt> = v.iter_int().collect();
    assert_eq!(all, vec![LogosInt::from_i64(4), LogosInt::from_i64(5), big()]);
}

#[test]
fn an_item_is_named_as_logos_names_it_from_the_front_or_the_back() {
    let mut v = IntVec::from_words(vec![10, 20, 30]);
    assert_eq!((v.item_word(1), v.item_word(3), v.item_word(-1), v.item_word(-3)), (10, 30, 30, 10));
    assert_eq!(v.item_ck(2), Some(20));
    v.set_item_word(-1, 31);
    v.set_item_int(1, big());
    assert_eq!(v.item_int(1), big());
    assert_eq!(v.item_int(-1), LogosInt::from_i64(31));
    assert_eq!(v.item_ck(2), None, "a promoted list leaves the word path");
}

#[test]
#[should_panic(expected = "Index 4 out of bounds")]
fn an_item_past_the_end_fails_with_every_tiers_sentence() {
    IntVec::from_words(vec![1, 2, 3]).item_word(4);
}

#[test]
#[should_panic(expected = "Index 0 out of bounds")]
fn item_zero_fails_on_a_promoted_list_too() {
    let mut v = IntVec::new();
    v.push_int(big());
    v.item_int(0);
}

#[test]
fn a_wide_value_is_stored_as_a_word_while_it_fits_one_and_promotes_once_it_does_not() {
    let mut v = IntVec::from_words(vec![1]);
    v.push_i128(i64::MAX as i128);
    v.push_i128(i64::MIN as i128);
    assert!(!v.is_exact(), "a wide value within the word is a word");
    v.push_i128(i64::MAX as i128 + 1);
    assert!(v.is_exact());
    assert_eq!(v.get_int(3), LogosInt::from_literal("9223372036854775808"));
    assert_eq!(v.get_int(2), LogosInt::from_i64(i64::MIN));
    v.push_i128(-5);
    assert_eq!(v.get_int(4), LogosInt::from_i64(-5));
}

#[test]
fn an_item_set_to_a_wide_value_promotes_as_a_push_does() {
    let mut v = IntVec::from_words(vec![1, 2, 3]);
    v.set_item_i128(2, 40);
    assert!(!v.is_exact());
    assert_eq!(v.item_word(2), 40);
    v.set_item_i128(-1, (i64::MIN as i128) - 1);
    assert!(v.is_exact());
    assert_eq!(v.item_int(3), LogosInt::from_literal("-9223372036854775809"));
    assert_eq!((v.item_int(1), v.item_int(2)), (LogosInt::from_i64(1), LogosInt::from_i64(40)));
}

#[test]
fn elements_iterate_as_words_where_each_fits_one() {
    let mut v = IntVec::from_words(vec![4, 5]);
    assert_eq!(v.iter_word().collect::<Vec<i64>>(), vec![4, 5]);
    v.push_int(big());
    v.set_int(1, LogosInt::from_i64(6));
    let mut walk = v.iter_word();
    assert_eq!((walk.next(), walk.next()), (Some(4), Some(6)), "a promoted list's words read as words");
}

#[test]
#[should_panic(expected = "does not fit a 64-bit Int")]
fn a_word_walk_reaching_a_value_past_the_word_stops_loudly() {
    let mut v = IntVec::from_words(vec![1]);
    v.push_int(big());
    for _ in v.iter_word() {}
}

#[test]
fn a_word_is_found_in_either_form() {
    let mut v = IntVec::from_words(vec![1, 2]);
    assert!(v.contains_word(2));
    assert!(!v.contains_word(3));
    v.push_int(big());
    assert!(v.contains_word(1));
    assert!(!v.contains_word(3));
}

#[test]
fn a_list_written_with_values_past_the_word_holds_them_and_one_within_holds_words() {
    let within = IntVec::from_ints(vec![LogosInt::from_i64(1), LogosInt::from_i64(2)]);
    assert!(!within.is_exact());
    let past = IntVec::from_ints(vec![big(), LogosInt::from_i64(2)]);
    assert!(past.is_exact());
    assert_eq!((past.item_int(1), past.item_word(2)), (big(), 2));
}

#[test]
fn a_wide_value_keeps_every_digit_to_the_ends_of_its_range() {
    let mut v = IntVec::new();
    for x in [i128::MAX, i128::MIN, (1i128 << 64) + 7, -(1i128 << 96) - 1, 1i128 << 63, -(1i128 << 63) - 1] {
        v.push_i128(x);
    }
    let shown: Vec<String> = v.iter_int().map(|x| x.to_string()).collect();
    assert_eq!(
        shown,
        [
            i128::MAX.to_string(),
            i128::MIN.to_string(),
            ((1i128 << 64) + 7).to_string(),
            (-(1i128 << 96) - 1).to_string(),
            (1i128 << 63).to_string(),
            (-(1i128 << 63) - 1).to_string(),
        ]
    );
}

#[test]
fn an_unchecked_read_is_the_read_at_an_index_it_names() {
    let mut words = IntVec::from_words(vec![3, -7, i64::MAX]);
    for i in 0..words.len() {
        assert_eq!(unsafe { words.get_word_unchecked(i) }, words.get_word(i));
    }
    words.push_i128(i128::from(i64::MAX) + 1);
    assert!(words.is_exact(), "a value past the word promotes the list");
    for i in 0..3 {
        assert_eq!(unsafe { words.get_word_unchecked(i) }, words.get_word(i), "a promoted item that fits reads as its word");
    }
}

#[test]
fn an_unchecked_read_of_an_item_past_the_word_stops_loudly() {
    let list = IntVec::from_ints(vec![LogosInt::from_i128(i128::from(i64::MAX) + 1)]);
    let stopped = std::panic::catch_unwind(|| unsafe { list.get_word_unchecked(0) });
    let why = stopped.expect_err("an item past the word has no word");
    let text = why.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(text.contains("does not fit a 64-bit Int"), "the canonical sentence: {text}");
}

#[test]
fn unchecked_writes_are_the_writes_at_an_index_they_name() {
    let mut a = IntVec::from_words(vec![0, 0, 0]);
    let mut b = IntVec::from_words(vec![0, 0, 0]);
    unsafe { a.set_word_unchecked(0, 5) };
    b.set_word(0, 5);
    unsafe { a.set_i128_unchecked(1, -9) };
    b.set_i128(1, -9);
    assert_eq!(a, b, "words written in place");
    assert!(!a.is_exact(), "a value that fits keeps the words");
    unsafe { a.set_i128_unchecked(2, i128::from(i64::MIN) - 1) };
    b.set_i128(2, i128::from(i64::MIN) - 1);
    assert!(a.is_exact(), "a value past the word promotes the list");
    assert_eq!(a, b, "and the value is exact");
    unsafe { a.set_int_unchecked(0, LogosInt::from_i128(i128::from(i64::MAX) * 4)) };
    b.set_int(0, LogosInt::from_i128(i128::from(i64::MAX) * 4));
    unsafe { a.set_word_unchecked(1, 11) };
    b.set_word(1, 11);
    assert_eq!(a, b, "writes after the list promoted");
    assert_eq!(a.get_int(0), LogosInt::from_i128(i128::from(i64::MAX) * 4));
}

#[test]
fn a_word_updated_in_place_is_the_item_while_the_list_holds_words() {
    let mut list = IntVec::from_words(vec![4, 9, 16]);
    *list.item_word_mut(2).expect("words") -= 1;
    *list.item_word_mut(-1).expect("words") += 4;
    assert_eq!(list, IntVec::from_words(vec![4, 8, 20]), "1-based from the front, from the back when negative");
    *unsafe { list.word_mut_unchecked(0) }.expect("words") = 7;
    assert_eq!(list.get_word(0), 7);
    list.push_i128(i128::from(i64::MAX) + 1);
    assert!(list.item_word_mut(1).is_none(), "a promoted list hands out no word");
    assert!(unsafe { list.word_mut_unchecked(1) }.is_none());
}

#[test]
fn a_word_updated_in_place_at_no_item_fails_with_the_sentence() {
    let mut list = IntVec::from_words(vec![1]);
    let missed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        list.item_word_mut(3);
    }));
    assert!(missed.is_err(), "item 3 of a one-item list names no item");
}

#[test]
fn the_words_view_is_the_list_while_it_holds_words() {
    let mut list = IntVec::with_capacity(4);
    list.words_mut().expect("words").push(3);
    list.push_word(5);
    let words = list.words_mut().expect("words");
    words[1] += 10;
    words.push(-2);
    assert_eq!(list, IntVec::from_words(vec![3, 15, -2]), "reads, writes and pushes land in the list");
    assert_eq!(list.len(), 3);
    list.set_int(0, LogosInt::from_i128(i128::from(i64::MIN) - 1));
    assert!(list.words_mut().is_none(), "a promoted list hands out no words");
    assert_eq!(list.get_int(1), LogosInt::from_i64(15), "and keeps every item");
}

#[test]
fn the_shared_words_view_reads_the_list_while_it_holds_words() {
    let mut list = IntVec::from_words(vec![2, 4]);
    assert_eq!(list.words().map(|w| w.as_slice()), Some(&[2, 4][..]), "the words, in order");
    list.push_int(LogosInt::from_i128(i128::from(i64::MAX) + 1));
    assert!(list.words().is_none(), "a promoted list hands out no words");
}
