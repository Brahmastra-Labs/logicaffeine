use super::*;

fn list(items: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(
        crate::interpreter::ListRepr::from_values(items),
    )))
}

#[test]
fn index_is_one_based_with_end_relative_negatives() {
    let xs = list(vec![RuntimeValue::Int(5), RuntimeValue::Int(6)]);
    assert!(matches!(
        index_get(&xs, &RuntimeValue::Int(1)).unwrap(),
        RuntimeValue::Int(5)
    ));
    assert_eq!(
        index_get(&xs, &RuntimeValue::Int(0)).unwrap_err(),
        "Index 0 out of bounds"
    );
    assert_eq!(
        index_get(&xs, &RuntimeValue::Int(3)).unwrap_err(),
        "Index 3 out of bounds"
    );
    // Negative = end-relative: `-1` is the last element, `-2` the first.
    assert!(matches!(
        index_get(&xs, &RuntimeValue::Int(-1)).unwrap(),
        RuntimeValue::Int(6)
    ));
    assert!(matches!(
        index_get(&xs, &RuntimeValue::Int(-2)).unwrap(),
        RuntimeValue::Int(5)
    ));
    // Out of range on the negative side is still loud.
    assert_eq!(
        index_get(&xs, &RuntimeValue::Int(-3)).unwrap_err(),
        "Index -3 out of bounds"
    );
}

#[test]
fn text_indexing_is_chars_but_length_is_bytes() {
    let s = RuntimeValue::Text(Rc::new("héllo".to_string()));
    // 5 chars, 6 bytes.
    let c = index_get(&s, &RuntimeValue::Int(2)).unwrap();
    assert!(matches!(&c, RuntimeValue::Text(t) if **t == "é"));
    assert!(matches!(length_of(&s).unwrap(), RuntimeValue::Int(6)));
}

#[test]
fn slice_is_one_indexed_inclusive_and_oob_is_empty() {
    let xs = list((1..=5).map(RuntimeValue::Int).collect());
    let s = slice(&xs, &RuntimeValue::Int(2), &RuntimeValue::Int(4)).unwrap();
    if let RuntimeValue::List(items) = &s {
        let v: Vec<i64> = items
            .borrow()
            .to_values()
            .iter()
            .map(|x| {
                if let RuntimeValue::Int(n) = x {
                    *n
                } else {
                    panic!()
                }
            })
            .collect();
        assert_eq!(v, vec![2, 3, 4]);
    } else {
        panic!("slice did not return a list");
    }
    let s = slice(&xs, &RuntimeValue::Int(4), &RuntimeValue::Int(99)).unwrap();
    if let RuntimeValue::List(items) = &s {
        assert!(items.borrow().is_empty());
    }
}

#[test]
fn pop_of_empty_list_is_nothing_not_error() {
    let xs = list(vec![]);
    assert!(matches!(list_pop(&xs).unwrap(), RuntimeValue::Nothing));
}

#[test]
fn set_add_dedups_with_ieee_equality() {
    // IEEE equality: 0.1 + 0.2 is NOT 0.3 (the artifact is real), so they
    // are two distinct set elements…
    let s = RuntimeValue::Set(Rc::new(RefCell::new(vec![RuntimeValue::Float(0.3)])));
    set_add(&s, RuntimeValue::Float(0.1 + 0.2)).unwrap();
    if let RuntimeValue::Set(items) = &s {
        assert_eq!(
            items.borrow().len(),
            2,
            "IEEE-distinct floats stay distinct"
        );
    }
    // …while a bit-equal float DOES dedup.
    set_add(&s, RuntimeValue::Float(0.3)).unwrap();
    if let RuntimeValue::Set(items) = &s {
        assert_eq!(items.borrow().len(), 2, "bit-equal float must dedup");
    }
}

#[test]
fn range_requires_int_bounds() {
    assert_eq!(
        range(&RuntimeValue::Int(1), &RuntimeValue::Float(2.5)).unwrap_err(),
        "Range requires Int bounds"
    );
}
