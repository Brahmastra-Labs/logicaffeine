use super::*;

fn i32_buf(vals: &[i64]) -> ListRepr {
    let mut r = ListRepr::IntsI32(Vec::new());
    for &v in vals {
        r.push(RuntimeValue::Int(v));
    }
    r
}

#[test]
fn push_and_get_sign_extend() {
    let r = i32_buf(&[-1, 0, 7, i32::MIN as i64, i32::MAX as i64]);
    assert!(
        matches!(r, ListRepr::IntsI32(_)),
        "stays half-width when every value fits i32"
    );
    assert_eq!(
        r.get(0),
        Some(RuntimeValue::Int(-1)),
        "negative sign-extends losslessly"
    );
    assert_eq!(r.get(3), Some(RuntimeValue::Int(i32::MIN as i64)));
    assert_eq!(r.get(4), Some(RuntimeValue::Int(i32::MAX as i64)));
    assert_eq!(r.len(), 5);
}

#[test]
fn push_out_of_range_widens_and_preserves_values() {
    // A value just past i32::MAX forces the whole buffer to full width;
    // every earlier element survives, bit-identical to a never-narrowed run.
    let mut r = i32_buf(&[1, -2, 100]);
    r.push(RuntimeValue::Int(i32::MAX as i64 + 1));
    assert!(
        matches!(r, ListRepr::Ints(_)),
        "an out-of-range push widens to full-width Ints"
    );
    assert_eq!(r.get(0), Some(RuntimeValue::Int(1)));
    assert_eq!(r.get(1), Some(RuntimeValue::Int(-2)));
    assert_eq!(r.get(2), Some(RuntimeValue::Int(100)));
    assert_eq!(r.get(3), Some(RuntimeValue::Int(i32::MAX as i64 + 1)));
}

#[test]
fn set_out_of_range_widens() {
    let mut r = i32_buf(&[5, 5, 5]);
    r.set(1, RuntimeValue::Int(i64::MIN));
    assert!(
        matches!(r, ListRepr::Ints(_)),
        "an out-of-range in-place store widens"
    );
    assert_eq!(r.get(0), Some(RuntimeValue::Int(5)));
    assert_eq!(r.get(1), Some(RuntimeValue::Int(i64::MIN)));
    assert_eq!(r.get(2), Some(RuntimeValue::Int(5)));
}

#[test]
fn set_in_range_truncates_losslessly() {
    let mut r = i32_buf(&[0, 0, 0]);
    r.set(2, RuntimeValue::Int(-12345));
    assert!(
        matches!(r, ListRepr::IntsI32(_)),
        "in-range store stays half-width"
    );
    assert_eq!(r.get(2), Some(RuntimeValue::Int(-12345)));
}

#[test]
fn non_int_push_promotes_to_boxed() {
    // Soundness net: a narrowed buffer that somehow receives a non-Int value
    // boxes rather than dropping the type — never silently wrong.
    let mut r = i32_buf(&[1, 2]);
    r.push(RuntimeValue::Float(3.5));
    assert!(matches!(r, ListRepr::Boxed(_)));
    assert_eq!(r.get(2), Some(RuntimeValue::Float(3.5)));
    assert_eq!(r.get(0), Some(RuntimeValue::Int(1)));
}

#[test]
fn clone_round_trips_to_values() {
    let r = i32_buf(&[-7, 42, i32::MIN as i64]);
    let snap = r.clone();
    assert_eq!(snap.to_values(), r.to_values());
    assert_eq!(
        r.to_values(),
        vec![
            RuntimeValue::Int(-7),
            RuntimeValue::Int(42),
            RuntimeValue::Int(i32::MIN as i64)
        ]
    );
}

#[test]
fn pop_truncate_position_match_full_width() {
    let mut r = i32_buf(&[10, 20, 30]);
    assert_eq!(r.position(&RuntimeValue::Int(20)), Some(1));
    assert_eq!(r.position(&RuntimeValue::Int(99)), None);
    assert_eq!(r.pop(), Some(RuntimeValue::Int(30)));
    r.truncate(1);
    assert_eq!(r.len(), 1);
    assert_eq!(r.get(0), Some(RuntimeValue::Int(10)));
}
