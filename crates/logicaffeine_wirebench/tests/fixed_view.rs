//! The FIXED-stride record-list view (`T_STRUCTS_FVIEW`, the `indexed fast` form): fixed-width
//! rows with NO offset tables, so random access is pure arithmetic. These lock in that it
//! round-trips to the exact logical data, reads any (row, field) in O(1), and is smaller than the
//! variable offset-table view it composes the fixed dial onto.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use logicaffeine_compile::concurrency::marshal::{
    message_from_wire, message_to_wire_with, view_message, with_numerics, with_struct_view, WireCodec,
    WireIntegrity, WireNumerics,
};
use logicaffeine_compile::interpreter::{ListRepr, RuntimeValue, StructValue};

fn rv_list(rows: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(rows))))
}
fn rec(id: i64, name: &str, active: bool) -> RuntimeValue {
    let mut f = HashMap::new();
    f.insert("id".to_string(), RuntimeValue::Int(id));
    f.insert("name".to_string(), RuntimeValue::Text(Rc::new(name.to_string())));
    f.insert("active".to_string(), RuntimeValue::Bool(active));
    RuntimeValue::Struct(Box::new(StructValue { type_name: "Record".to_string(), fields: f }))
}
fn point(x: i64, y: i64) -> RuntimeValue {
    let mut f = HashMap::new();
    f.insert("x".to_string(), RuntimeValue::Int(x));
    f.insert("y".to_string(), RuntimeValue::Int(y));
    RuntimeValue::Struct(Box::new(StructValue { type_name: "Point".to_string(), fields: f }))
}

fn enc_fixed_view(v: &RuntimeValue) -> Vec<u8> {
    with_struct_view(true, || {
        with_numerics(WireNumerics::Fixed, || {
            message_to_wire_with("p", v, WireCodec::Native, WireIntegrity::Raw).unwrap()
        })
    })
}
fn enc_var_view(v: &RuntimeValue) -> Vec<u8> {
    with_struct_view(true, || message_to_wire_with("p", v, WireCodec::Native, WireIntegrity::Raw).unwrap())
}
/// Canonical bytes for a decoded value (no view, default dial) — for logical equality.
fn canon(v: &RuntimeValue) -> Vec<u8> {
    message_to_wire_with("x", v, WireCodec::Native, WireIntegrity::Raw).unwrap()
}

fn as_int(v: Option<RuntimeValue>) -> Option<i64> {
    match v? {
        RuntimeValue::Int(n) => Some(n),
        _ => None,
    }
}
fn as_bool(v: Option<RuntimeValue>) -> Option<bool> {
    match v? {
        RuntimeValue::Bool(b) => Some(b),
        _ => None,
    }
}
fn as_text(v: Option<RuntimeValue>) -> Option<String> {
    match v? {
        RuntimeValue::Text(s) => Some((*s).clone()),
        _ => None,
    }
}

#[test]
fn fixed_view_record_list_round_trips_and_random_accesses() {
    const N: usize = 1000;
    let names = ["alice", "bob", "carol", "dave", "erin"];
    let rv = rv_list((0..N).map(|i| rec(i as i64 * 7 + 1, names[i % 5], i % 2 == 0)).collect());

    let fixed = enc_fixed_view(&rv);
    let variable = enc_var_view(&rv);

    // Round-trip: the fixed view decodes to the SAME logical list as the variable view.
    let (_, df) = message_from_wire(&fixed).expect("fixed view decodes");
    let (_, dv) = message_from_wire(&variable).expect("variable view decodes");
    assert_eq!(canon(&df), canon(&dv), "fixed view carries the same logical data");

    // O(1) random access through the unified reader — incl. a materialized text field.
    let view = view_message(&fixed).expect("fixed view opens");
    assert_eq!(view.structs_len(), Some(N));
    for &i in &[0usize, 1, 7, 333, 999] {
        assert_eq!(as_int(view.structs_row_field_value(i, "id")), Some(i as i64 * 7 + 1), "id row {i}");
        assert_eq!(as_bool(view.structs_row_field_value(i, "active")), Some(i % 2 == 0), "active row {i}");
        assert_eq!(as_text(view.structs_row_field_value(i, "name")), Some(names[i % 5].to_string()), "name row {i}");
    }
    // Out-of-range row and unknown field both refuse.
    assert!(view.structs_row_field_value(N, "id").is_none());
    assert!(view.structs_row_field_value(0, "nope").is_none());
}

#[test]
fn fixed_view_drops_the_offset_tables_so_its_smaller() {
    // An all-fixed-width record (Point {x,y}) is where the fixed stride wins decisively: it has
    // NO row table and NO per-row field table, just n×16 bytes — smaller than the variable view.
    const N: usize = 1000;
    let mut seed = 0x1234_5678u64;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (seed >> 33) as i64 % 200_000 - 100_000
    };
    let rv = rv_list((0..N).map(|_| point(next(), next())).collect());

    let fixed = enc_fixed_view(&rv);
    let variable = enc_var_view(&rv);
    assert!(
        fixed.len() < variable.len(),
        "fixed view must drop the offset tables: fixed {} vs variable {}",
        fixed.len(),
        variable.len()
    );

    // …and still round-trip + random-access exactly.
    let (_, d) = message_from_wire(&fixed).expect("fixed point view decodes");
    let view = view_message(&fixed).expect("opens");
    assert_eq!(view.structs_len(), Some(N));
    // spot-check a row against the decoded list
    if let RuntimeValue::List(l) = &d {
        let row7 = l.borrow().get(7).unwrap();
        if let RuntimeValue::Struct(sv) = row7 {
            let x = match sv.fields.get("x") {
                Some(RuntimeValue::Int(n)) => *n,
                _ => panic!("x"),
            };
            assert_eq!(as_int(view.structs_row_field_value(7, "x")), Some(x), "view x matches decode");
        } else {
            panic!("row is a struct");
        }
    } else {
        panic!("decodes to a list");
    }
}
