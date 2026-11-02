use super::*;

fn nullary(ty: &str, ctor: &str) -> RuntimeValue {
    RuntimeValue::Inductive(std::rc::Rc::new(InductiveValue {
        inductive_type: ty.into(),
        constructor: ctor.into(),
        args: vec![],
    }))
}
fn with_args(ty: &str, ctor: &str, args: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::Inductive(std::rc::Rc::new(InductiveValue {
        inductive_type: ty.into(),
        constructor: ctor.into(),
        args,
    }))
}
fn ctor_of(v: &RuntimeValue) -> String {
    match v {
        RuntimeValue::Inductive(i) => i.constructor.clone(),
        other => panic!("not an inductive: {other:?}"),
    }
}
fn int_arg(v: &RuntimeValue, j: usize) -> i64 {
    match v {
        RuntimeValue::Inductive(i) => match &i.args[j] {
            RuntimeValue::Int(n) => *n,
            other => panic!("arg {j} not an int: {other:?}"),
        },
        other => panic!("not an inductive: {other:?}"),
    }
}

#[test]
fn from_values_nullary_enums_is_columnar() {
    let r = ListRepr::from_values(vec![
        nullary("Color", "Red"),
        nullary("Color", "Green"),
        nullary("Color", "Red"),
    ]);
    assert!(
        matches!(r, ListRepr::Inductives { .. }),
        "a nullary enum list de-boxes to columns"
    );
    assert_eq!(r.len(), 3);
    assert_eq!(ctor_of(&r.get(0).unwrap()), "Red");
    assert_eq!(ctor_of(&r.get(1).unwrap()), "Green");
    assert_eq!(ctor_of(&r.get(2).unwrap()), "Red");
    assert!(r.get(3).is_none());
}

#[test]
fn from_values_uniform_arg_enums_is_columnar() {
    let r = ListRepr::from_values(vec![
        with_args("Boxed", "B", vec![RuntimeValue::Int(1)]),
        with_args("Boxed", "B", vec![RuntimeValue::Int(2)]),
    ]);
    assert!(
        matches!(r, ListRepr::Inductives { .. }),
        "a uniform-arg enum list packs columnar"
    );
    assert_eq!(int_arg(&r.get(0).unwrap(), 0), 1);
    assert_eq!(int_arg(&r.get(1).unwrap(), 0), 2);
}

#[test]
fn from_values_mixed_arity_enums_is_columnar() {
    // Option-like: Some(1), None, Some(2), None, Some(3) — a tagged union, packed
    // as dense per-constructor arg columns.
    let rows = vec![
        with_args("Option", "Some", vec![RuntimeValue::Int(1)]),
        nullary("Option", "None"),
        with_args("Option", "Some", vec![RuntimeValue::Int(2)]),
        nullary("Option", "None"),
        with_args("Option", "Some", vec![RuntimeValue::Int(3)]),
    ];
    let r = ListRepr::from_values(rows);
    assert!(
        matches!(r, ListRepr::Inductives { .. }),
        "a mixed-arity enum list packs columnar"
    );
    assert_eq!(r.len(), 5);
    assert_eq!(ctor_of(&r.get(0).unwrap()), "Some");
    assert_eq!(int_arg(&r.get(0).unwrap(), 0), 1);
    assert_eq!(ctor_of(&r.get(1).unwrap()), "None");
    assert_eq!(ctor_of(&r.get(2).unwrap()), "Some");
    assert_eq!(int_arg(&r.get(2).unwrap(), 0), 2);
    assert_eq!(ctor_of(&r.get(3).unwrap()), "None");
    assert_eq!(ctor_of(&r.get(4).unwrap()), "Some");
    assert_eq!(int_arg(&r.get(4).unwrap(), 0), 3);
}

#[test]
fn enums_to_values_reconstructs_all_rows() {
    let rows = vec![
        with_args("Option", "Some", vec![RuntimeValue::Int(7)]),
        nullary("Option", "None"),
    ];
    let vs = ListRepr::from_values(rows).to_values();
    assert_eq!(vs.len(), 2);
    assert_eq!(ctor_of(&vs[0]), "Some");
    assert_eq!(int_arg(&vs[0], 0), 7);
    assert_eq!(ctor_of(&vs[1]), "None");
}

#[test]
fn enums_mutation_decolumnarizes_and_stays_correct() {
    let mut r = ListRepr::from_values(vec![nullary("Color", "Red"), nullary("Color", "Green")]);
    r.set(1, RuntimeValue::Int(99));
    assert!(
        matches!(r, ListRepr::Boxed(_)),
        "a mutating set de-columnarizes"
    );
    assert_eq!(ctor_of(&r.get(0).unwrap()), "Red");
    assert_eq!(r.get(1), Some(RuntimeValue::Int(99)));
}

#[test]
fn heterogeneous_enums_stay_boxed() {
    let r = ListRepr::from_values(vec![nullary("Color", "Red"), nullary("Suit", "Spade")]);
    assert!(
        matches!(r, ListRepr::Boxed(_)),
        "mixed inductive types stay boxed"
    );
}
