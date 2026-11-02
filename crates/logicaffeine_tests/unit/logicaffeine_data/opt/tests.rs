use super::*;

fn stop_words(f: impl FnOnce() + std::panic::UnwindSafe) -> String {
    let payload = std::panic::catch_unwind(f).expect_err("the program stops");
    match payload.downcast::<String>() {
        Ok(words) => *words,
        Err(payload) => payload.downcast_ref::<&str>().expect("a sentence").to_string(),
    }
}

#[test]
fn a_present_value_is_read_where_it_is() {
    let mut place = LogosOpt::from(vec![1_i64, 2]);
    assert!(!place.is_nothing());
    assert_eq!(place.held("unused"), &vec![1, 2]);
    place.held_mut("unused").push(3);
    assert_eq!(place.into_held("unused"), vec![1, 2, 3]);
}

#[test]
fn reading_nothing_stops_in_the_words_given() {
    assert_eq!(
        stop_words(|| {
            LogosOpt::<i64>::NOTHING.held("Can only push to a List");
        }),
        "Can only push to a List"
    );
    assert_eq!(
        stop_words(|| {
            let mut place = LogosOpt::<Vec<i64>>::NOTHING;
            place.held_mut("Cannot pop from Nothing").pop();
        }),
        "Cannot pop from Nothing"
    );
    assert_eq!(
        stop_words(|| {
            LogosOpt::<String>::NOTHING.into_held("Cannot take the length of Nothing");
        }),
        "Cannot take the length of Nothing"
    );
}

#[test]
fn nothing_is_nothing_and_no_value() {
    let nothing = LogosOpt::<i64>::NOTHING;
    assert!(nothing.is_nothing());
    assert_eq!(nothing, LogosOpt(None));
    assert_ne!(nothing, LogosOpt::from(0));
}

#[test]
fn two_places_are_one_value_when_both_hold_nothing_or_equal_values() {
    let same = |a: &i64, b: &f64| (*a as f64) == *b;
    assert!(LogosOpt::<i64>::NOTHING.equals_with(&LogosOpt::<f64>::NOTHING, same));
    assert!(LogosOpt::from(4_i64).equals_with(&LogosOpt::from(4.0), same));
    assert!(!LogosOpt::from(4_i64).equals_with(&LogosOpt::from(4.5), same));
    assert!(!LogosOpt::from(4_i64).equals_with(&LogosOpt::<f64>::NOTHING, same));
    assert!(!LogosOpt::<i64>::NOTHING.equals_with(&LogosOpt::from(4.0), same));
}

#[test]
fn equality_is_not_asked_of_values_when_one_side_is_nothing() {
    let never = |_: &i64, _: &i64| -> bool { panic!("compared a value against nothing") };
    assert!(!LogosOpt::from(1_i64).equals_with(&LogosOpt::NOTHING, never));
    assert!(LogosOpt::<i64>::NOTHING.equals_with(&LogosOpt::NOTHING, never));
}

#[test]
fn a_place_starts_holding_its_types_first_value() {
    assert_eq!(LogosOpt::<i64>::default(), LogosOpt::from(0));
    assert_eq!(LogosOpt::<String>::default(), LogosOpt::from(String::new()));
    assert_eq!(LogosOpt::<Vec<i64>>::default(), LogosOpt::from(Vec::new()));
}

#[test]
fn nothing_hashes_apart_from_every_value() {
    use std::collections::HashSet;
    let places: HashSet<LogosOpt<i64>> = [LogosOpt::NOTHING, LogosOpt::from(0), LogosOpt::NOTHING].into_iter().collect();
    assert_eq!(places.len(), 2);
}

#[test]
fn two_operands_are_read_together_and_stop_naming_which_is_nothing() {
    let words = ["Cannot add Nothing and Int", "Cannot add Int and Nothing", "Cannot add Nothing and Nothing"];
    assert_eq!(LogosOpt::pair(LogosOpt::from(2_i64), LogosOpt::from(3_i64), words), (2, 3));
    assert_eq!(
        stop_words(move || {
            LogosOpt::pair(LogosOpt::<i64>::NOTHING, LogosOpt::from(3_i64), words);
        }),
        words[0]
    );
    assert_eq!(
        stop_words(move || {
            LogosOpt::pair(LogosOpt::from(2_i64), LogosOpt::<i64>::NOTHING, words);
        }),
        words[1]
    );
    assert_eq!(
        stop_words(move || {
            LogosOpt::pair(LogosOpt::<i64>::NOTHING, LogosOpt::<i64>::NOTHING, words);
        }),
        words[2]
    );
}

#[test]
fn nothing_is_false_and_a_value_is_as_true_as_it_is() {
    use crate::ops::Truthy;
    assert!(!LogosOpt::<bool>::NOTHING.truthy());
    assert!(!LogosOpt::<i64>::NOTHING.truthy());
    assert!(LogosOpt::from(true).truthy());
    assert!(!LogosOpt::from(false).truthy());
    assert!(LogosOpt::from(4_i64).truthy());
    assert!(!LogosOpt::from(0_i64).truthy());
}
