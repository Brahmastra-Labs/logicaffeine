use super::count_handles;
use crate::interpreter::{ListRepr, RuntimeValue};
use std::cell::RefCell;
use std::rc::Rc;

fn text(s: &str) -> RuntimeValue {
    RuntimeValue::Text(Rc::new(s.to_string()))
}

/// A boxed list, which holds each item's own handle. (`from_values` would pack an
/// all-text list into one flat byte buffer that holds none.)
fn list(items: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(items))))
}

fn addr(v: &RuntimeValue) -> usize {
    match v {
        RuntimeValue::Text(t) => Rc::as_ptr(t) as usize,
        RuntimeValue::List(l) => Rc::as_ptr(l) as usize,
        other => panic!("not a heap value: {other:?}"),
    }
}

#[test]
fn two_roots_on_one_allocation_count_two() {
    let t = text("hi");
    let counts = count_handles([&t, &t.clone()]);
    assert_eq!(counts.get(&addr(&t)), Some(&2));
}

#[test]
fn a_handle_inside_a_container_counts() {
    let t = text("hi");
    let l = list(vec![t.clone(), t.clone()]);
    let counts = count_handles([&t, &l]);
    assert_eq!(counts.get(&addr(&t)), Some(&3), "one root plus two slots in the list");
    assert_eq!(counts.get(&addr(&l)), Some(&1));
}

#[test]
fn a_shared_container_is_entered_once() {
    let t = text("hi");
    let l = list(vec![t.clone()]);
    let counts = count_handles([&l, &l.clone()]);
    assert_eq!(counts.get(&addr(&l)), Some(&2));
    assert_eq!(counts.get(&addr(&t)), Some(&1), "the text sits in ONE list, however many names reach it");
}

#[test]
fn scalars_hold_no_handles() {
    assert!(count_handles([&RuntimeValue::Int(3)]).is_empty());
}

#[test]
fn copies_held_outside_the_roots_do_not_count() {
    // `Rc::strong_count` sees every clone anywhere — the debugger keeps one per step of
    // history. The program's count sees only what its roots reach.
    let t = text("hi");
    let history: Vec<RuntimeValue> = (0..50).map(|_| t.clone()).collect();
    assert_eq!(count_handles([&t]).get(&addr(&t)), Some(&1));
    drop(history);
}
