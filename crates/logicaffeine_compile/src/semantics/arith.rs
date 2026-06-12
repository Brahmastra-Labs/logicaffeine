//! Arithmetic, logical, and bitwise operators.

use std::rc::Rc;

use crate::ast::stmt::BinaryOpKind;
use crate::interpreter::RuntimeValue;

use super::compare::{compare, values_equal};
use super::temporal::date_add_span;

/// Apply a binary operator to two already-evaluated values.
///
/// NOTE on `And`/`Or`: these are the *eager* semantics (both operands already
/// evaluated) — bitwise for Int×Int, truthiness otherwise. Short-circuit
/// evaluation order is the engine's responsibility: evaluate the left operand,
/// and only consult the right when the left is an Int (bitwise) or when
/// truthiness requires it.
pub fn binary_op(
    op: BinaryOpKind,
    left: RuntimeValue,
    right: RuntimeValue,
) -> Result<RuntimeValue, String> {
    match op {
        BinaryOpKind::Add => add(left, right),
        BinaryOpKind::Subtract => subtract(left, right),
        BinaryOpKind::Multiply => multiply(left, right),
        BinaryOpKind::Divide => divide(left, right),
        BinaryOpKind::Modulo => modulo(left, right),
        BinaryOpKind::Eq => Ok(RuntimeValue::Bool(values_equal(&left, &right))),
        BinaryOpKind::NotEq => Ok(RuntimeValue::Bool(!values_equal(&left, &right))),
        BinaryOpKind::Lt | BinaryOpKind::Gt | BinaryOpKind::LtEq | BinaryOpKind::GtEq => {
            compare(op, &left, &right)
        }
        BinaryOpKind::And => match (&left, &right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Int(a & b)),
            _ => Ok(RuntimeValue::Bool(left.is_truthy() && right.is_truthy())),
        },
        BinaryOpKind::Or => match (&left, &right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Int(a | b)),
            _ => Ok(RuntimeValue::Bool(left.is_truthy() || right.is_truthy())),
        },
        BinaryOpKind::Concat => concat(left, right),
        BinaryOpKind::BitXor => match (left, right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Int(a ^ b)),
            _ => Err("Bitwise XOR requires integer operands".to_string()),
        },
        // Shift counts are truncated to u32 and masked mod 64 (the wrapping spec).
        BinaryOpKind::Shl => match (left, right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => {
                Ok(RuntimeValue::Int(a.wrapping_shl(b as u32)))
            }
            _ => Err("Left shift requires integer operands".to_string()),
        },
        BinaryOpKind::Shr => match (left, right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => {
                Ok(RuntimeValue::Int(a.wrapping_shr(b as u32)))
            }
            _ => Err("Right shift requires integer operands".to_string()),
        },
    }
}

pub fn add(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    match (&left, &right) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Int(a.wrapping_add(*b))),
        (RuntimeValue::Float(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(a + b)),
        (RuntimeValue::Int(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(*a as f64 + b)),
        (RuntimeValue::Float(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Float(a + *b as f64)),
        (RuntimeValue::Text(a), RuntimeValue::Text(b)) => {
            Ok(RuntimeValue::Text(Rc::new(format!("{}{}", a, b))))
        }
        (RuntimeValue::Text(a), other) => {
            Ok(RuntimeValue::Text(Rc::new(format!("{}{}", a, other.to_display_string()))))
        }
        (other, RuntimeValue::Text(b)) => {
            Ok(RuntimeValue::Text(Rc::new(format!("{}{}", other.to_display_string(), b))))
        }
        (RuntimeValue::Duration(a), RuntimeValue::Duration(b)) => {
            Ok(RuntimeValue::Duration(a.wrapping_add(*b)))
        }
        (RuntimeValue::Date(days), RuntimeValue::Span { months, days: span_days }) => {
            Ok(RuntimeValue::Date(date_add_span(*days, *months, *span_days)))
        }
        _ => Err(format!("Cannot add {} and {}", left.type_name(), right.type_name())),
    }
}

pub fn concat(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    Ok(RuntimeValue::Text(Rc::new(format!(
        "{}{}",
        left.to_display_string(),
        right.to_display_string()
    ))))
}

pub fn subtract(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    match (&left, &right) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Int(a.wrapping_sub(*b))),
        (RuntimeValue::Float(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(a - b)),
        (RuntimeValue::Int(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(*a as f64 - b)),
        (RuntimeValue::Float(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Float(a - *b as f64)),
        (RuntimeValue::Duration(a), RuntimeValue::Duration(b)) => {
            Ok(RuntimeValue::Duration(a.wrapping_sub(*b)))
        }
        (RuntimeValue::Date(days), RuntimeValue::Span { months, days: span_days }) => {
            Ok(RuntimeValue::Date(date_add_span(*days, -*months, -*span_days)))
        }
        _ => Err(format!(
            "Cannot subtract {} from {}",
            right.type_name(),
            left.type_name()
        )),
    }
}

pub fn multiply(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    match (&left, &right) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Int(a.wrapping_mul(*b))),
        (RuntimeValue::Float(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(a * b)),
        (RuntimeValue::Int(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(*a as f64 * b)),
        (RuntimeValue::Float(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Float(a * *b as f64)),
        _ => Err(format!(
            "Cannot multiply {} and {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

pub fn divide(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    match (&left, &right) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => {
            if *b == 0 {
                return Err("Division by zero".to_string());
            }
            Ok(RuntimeValue::Int(a.wrapping_div(*b)))
        }
        (RuntimeValue::Float(a), RuntimeValue::Float(b)) => {
            if *b == 0.0 {
                return Err("Division by zero".to_string());
            }
            Ok(RuntimeValue::Float(a / b))
        }
        (RuntimeValue::Int(a), RuntimeValue::Float(b)) => {
            if *b == 0.0 {
                return Err("Division by zero".to_string());
            }
            Ok(RuntimeValue::Float(*a as f64 / b))
        }
        (RuntimeValue::Float(a), RuntimeValue::Int(b)) => {
            if *b == 0 {
                return Err("Division by zero".to_string());
            }
            Ok(RuntimeValue::Float(a / *b as f64))
        }
        _ => Err(format!(
            "Cannot divide {} by {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

pub fn modulo(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    match (&left, &right) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => {
            if *b == 0 {
                return Err("Modulo by zero".to_string());
            }
            Ok(RuntimeValue::Int(a.wrapping_rem(*b)))
        }
        _ => Err(format!(
            "Cannot compute modulo of {} and {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// One CRDT counter step: `current + amount` (wrapping), where an absent or
/// Nothing field starts from zero. Decrement passes a negated amount.
pub fn crdt_counter_bump(
    current: RuntimeValue,
    amount: i64,
    field_name: &str,
) -> Result<RuntimeValue, String> {
    match current {
        RuntimeValue::Int(n) => Ok(RuntimeValue::Int(n.wrapping_add(amount))),
        RuntimeValue::Nothing => Ok(RuntimeValue::Int(amount)),
        _ => Err(format!("Field '{}' is not a counter", field_name)),
    }
}

/// GCounter merge for one field: Int+Int adds (wrapping); anything else takes
/// the incoming value.
pub fn crdt_merge_field(current: &RuntimeValue, incoming: RuntimeValue) -> RuntimeValue {
    match (current, &incoming) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => RuntimeValue::Int(a.wrapping_add(*b)),
        _ => incoming,
    }
}

/// `not x` — logical for Bool, bitwise for Int.
pub fn not_value(val: RuntimeValue) -> Result<RuntimeValue, String> {
    match val {
        RuntimeValue::Bool(b) => Ok(RuntimeValue::Bool(!b)),
        RuntimeValue::Int(n) => Ok(RuntimeValue::Int(!n)),
        other => Err(format!("Cannot apply 'not' to {}", other.type_name())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_are_canonical() {
        let e = add(RuntimeValue::Bool(true), RuntimeValue::Nothing).unwrap_err();
        assert_eq!(e, "Cannot add Bool and Nothing");
        let e = subtract(RuntimeValue::Bool(true), RuntimeValue::Nothing).unwrap_err();
        assert_eq!(e, "Cannot subtract Nothing from Bool");
        let e = multiply(RuntimeValue::Bool(true), RuntimeValue::Nothing).unwrap_err();
        assert_eq!(e, "Cannot multiply Bool and Nothing");
        let e = divide(RuntimeValue::Int(1), RuntimeValue::Int(0)).unwrap_err();
        assert_eq!(e, "Division by zero");
        let e = divide(RuntimeValue::Float(1.0), RuntimeValue::Float(0.0)).unwrap_err();
        assert_eq!(e, "Division by zero");
        let e = modulo(RuntimeValue::Int(1), RuntimeValue::Int(0)).unwrap_err();
        assert_eq!(e, "Modulo by zero");
        let e = not_value(RuntimeValue::Nothing).unwrap_err();
        assert_eq!(e, "Cannot apply 'not' to Nothing");
    }

    #[test]
    fn text_add_stringifies_either_side() {
        let r = add(
            RuntimeValue::Text(Rc::new("n=".to_string())),
            RuntimeValue::Int(4),
        )
        .unwrap();
        assert!(matches!(&r, RuntimeValue::Text(s) if **s == "n=4"));
        let r = add(
            RuntimeValue::Int(4),
            RuntimeValue::Text(Rc::new("!".to_string())),
        )
        .unwrap();
        assert!(matches!(&r, RuntimeValue::Text(s) if **s == "4!"));
    }

    #[test]
    fn date_plus_span_is_calendar_aware() {
        // 2024-01-31 (day 19753) + 1 month = 2024-02-29.
        let r = add(
            RuntimeValue::Date(19753),
            RuntimeValue::Span { months: 1, days: 0 },
        )
        .unwrap();
        assert!(matches!(r, RuntimeValue::Date(19782)));
        // And subtraction inverts the span sign.
        let r = subtract(
            RuntimeValue::Date(19782),
            RuntimeValue::Span { months: 0, days: 1 },
        )
        .unwrap();
        assert!(matches!(r, RuntimeValue::Date(19781)));
    }

    #[test]
    fn int_arithmetic_wraps_in_every_build_profile() {
        // The LOGOS Int spec: wrapping i64. These pass in debug AND release.
        let r = add(RuntimeValue::Int(i64::MAX), RuntimeValue::Int(1)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(i64::MIN)));
        let r = subtract(RuntimeValue::Int(i64::MIN), RuntimeValue::Int(1)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(i64::MAX)));
        let r = multiply(RuntimeValue::Int(i64::MAX), RuntimeValue::Int(2)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(-2)));
        // MIN / -1 and MIN % -1 are the division-overflow edges.
        let r = divide(RuntimeValue::Int(i64::MIN), RuntimeValue::Int(-1)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(i64::MIN)));
        let r = modulo(RuntimeValue::Int(i64::MIN), RuntimeValue::Int(-1)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(0)));
        // Duration arithmetic wraps identically.
        let r = add(RuntimeValue::Duration(i64::MAX), RuntimeValue::Duration(1)).unwrap();
        assert!(matches!(r, RuntimeValue::Duration(i64::MIN)));
        let r = subtract(RuntimeValue::Duration(i64::MIN), RuntimeValue::Duration(1)).unwrap();
        assert!(matches!(r, RuntimeValue::Duration(i64::MAX)));
    }

    #[test]
    fn shifts_mask_their_count_modulo_64() {
        // wrapping_shl/shr(b as u32): the count is truncated to u32 then masked
        // mod 64, so `1 << 64 == 1` and a negative count becomes (b as u32) & 63.
        let r = binary_op(BinaryOpKind::Shl, RuntimeValue::Int(1), RuntimeValue::Int(64)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(1)));
        let r = binary_op(BinaryOpKind::Shl, RuntimeValue::Int(1), RuntimeValue::Int(63)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(i64::MIN)));
        // -1 as u32 == u32::MAX; masked mod 64 → 63.
        let r = binary_op(BinaryOpKind::Shl, RuntimeValue::Int(1), RuntimeValue::Int(-1)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(i64::MIN)));
        let r = binary_op(BinaryOpKind::Shr, RuntimeValue::Int(i64::MIN), RuntimeValue::Int(63)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(-1)));
        let r = binary_op(BinaryOpKind::Shr, RuntimeValue::Int(8), RuntimeValue::Int(64)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(8)));
    }

    #[test]
    fn crdt_counter_bump_wraps() {
        let r = crdt_counter_bump(RuntimeValue::Int(i64::MAX), 1, "n").unwrap();
        assert!(matches!(r, RuntimeValue::Int(i64::MIN)));
        let r = crdt_counter_bump(RuntimeValue::Nothing, 5, "n").unwrap();
        assert!(matches!(r, RuntimeValue::Int(5)));
        let e = crdt_counter_bump(RuntimeValue::Bool(true), 1, "score").unwrap_err();
        assert_eq!(e, "Field 'score' is not a counter");
    }

    #[test]
    fn eager_and_or_are_bitwise_for_ints_truthy_otherwise() {
        let r = binary_op(BinaryOpKind::And, RuntimeValue::Int(6), RuntimeValue::Int(3)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(2)));
        let r = binary_op(BinaryOpKind::Or, RuntimeValue::Int(6), RuntimeValue::Int(3)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(7)));
        let r = binary_op(BinaryOpKind::And, RuntimeValue::Int(1), RuntimeValue::Bool(false)).unwrap();
        assert!(matches!(r, RuntimeValue::Bool(false)));
        let r = binary_op(BinaryOpKind::Or, RuntimeValue::Bool(false), RuntimeValue::Bool(true)).unwrap();
        assert!(matches!(r, RuntimeValue::Bool(true)));
    }
}
