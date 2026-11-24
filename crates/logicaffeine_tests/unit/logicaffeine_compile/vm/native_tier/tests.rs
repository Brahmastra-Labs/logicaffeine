use super::*;

fn warm_up(arguments: &[Option<ScalarValue>]) -> EntryValue {
    let mut entry = EntryValue::Unseen;
    for &argument in arguments {
        entry.observe(argument);
    }
    entry
}

#[test]
fn a_parameter_passed_one_value_on_every_call_holds_it() {
    let calls = vec![Some(ScalarValue::Int(10)); 100];
    assert_eq!(warm_up(&calls).held(), Some(ScalarValue::Int(10)));
    let flags = vec![Some(ScalarValue::Bool(false)); 3];
    assert_eq!(warm_up(&flags).held(), Some(ScalarValue::Bool(false)));
    assert_eq!(warm_up(&[Some(ScalarValue::Int(i64::MIN))]).held(), Some(ScalarValue::Int(i64::MIN)));
}

#[test]
fn a_parameter_no_call_passed_holds_nothing() {
    assert_eq!(warm_up(&[]), EntryValue::Unseen);
    assert_eq!(warm_up(&[]).held(), None);
}

#[test]
fn one_differing_call_anywhere_in_the_warm_up_holds_nothing_for_good() {
    let held = Some(ScalarValue::Int(7));
    for at in 0..5 {
        let mut calls = vec![held; 5];
        calls[at] = Some(ScalarValue::Int(8));
        assert_eq!(warm_up(&calls), EntryValue::Varied, "a different value at call {at}");
        calls.extend(vec![held; 50]);
        assert_eq!(warm_up(&calls).held(), None, "the value held again after call {at} does not restore it");
    }
}

#[test]
fn an_argument_that_is_not_a_small_scalar_holds_nothing() {
    assert_eq!(warm_up(&[None]), EntryValue::Varied);
    assert_eq!(warm_up(&[Some(ScalarValue::Int(3)), None, Some(ScalarValue::Int(3))]).held(), None);
    assert_eq!(warm_up(&[None, Some(ScalarValue::Int(3))]).held(), None);
}

#[test]
fn an_int_and_a_bool_are_different_values_even_with_the_same_bits() {
    let calls = [Some(ScalarValue::Int(1)), Some(ScalarValue::Bool(true))];
    assert_eq!(ScalarValue::Int(1).bits(), ScalarValue::Bool(true).bits());
    assert_eq!(warm_up(&calls).held(), None);
}

#[test]
fn a_value_carries_the_bits_its_frame_slot_holds() {
    assert_eq!(ScalarValue::Int(-5).bits(), -5);
    assert_eq!(ScalarValue::Int(i64::MAX).bits(), i64::MAX);
    assert_eq!(ScalarValue::Bool(true).bits(), 1);
    assert_eq!(ScalarValue::Bool(false).bits(), 0);
}
