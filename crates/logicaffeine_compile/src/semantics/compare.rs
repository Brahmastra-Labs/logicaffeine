//! Equality and relational comparison.

use crate::ast::stmt::BinaryOpKind;
use crate::interpreter::RuntimeValue;

/// Value equality for the `equals`/`==` operator and for set/list membership.
/// Floats compare with an epsilon (so `0.1 + 0.2 == 0.3`, and `NaN != NaN`);
/// inductive values compare structurally; collections are never equal.
pub fn values_equal(left: &RuntimeValue, right: &RuntimeValue) -> bool {
    match (left, right) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => a == b,
        (RuntimeValue::Float(a), RuntimeValue::Float(b)) => (a - b).abs() < f64::EPSILON,
        (RuntimeValue::Bool(a), RuntimeValue::Bool(b)) => a == b,
        (RuntimeValue::Text(a), RuntimeValue::Text(b)) => **a == **b,
        (RuntimeValue::Char(a), RuntimeValue::Char(b)) => a == b,
        (RuntimeValue::Nothing, RuntimeValue::Nothing) => true,
        (RuntimeValue::Duration(a), RuntimeValue::Duration(b)) => a == b,
        (RuntimeValue::Date(a), RuntimeValue::Date(b)) => a == b,
        (RuntimeValue::Moment(a), RuntimeValue::Moment(b)) => a == b,
        (
            RuntimeValue::Span { months: m1, days: d1 },
            RuntimeValue::Span { months: m2, days: d2 },
        ) => m1 == m2 && d1 == d2,
        (RuntimeValue::Time(a), RuntimeValue::Time(b)) => a == b,
        (RuntimeValue::Inductive(a), RuntimeValue::Inductive(b)) => {
            a.inductive_type == b.inductive_type
                && a.constructor == b.constructor
                && a.args.len() == b.args.len()
                && a.args.iter().zip(b.args.iter()).all(|(x, y)| values_equal(x, y))
        }
        _ => false,
    }
}

/// Relational comparison (`< > <= >=`).
///
/// Floats use IEEE 754 semantics — NaN is unordered (every relational
/// comparison with a NaN is `false`) and `-0.0 == 0.0` — matching Rust's `f64`
/// ordering, which is what the compile-to-Rust path emits. Integer and
/// temporal types use the natural total order; a Moment compares against a
/// Time by its time-of-day.
pub fn compare(
    op: BinaryOpKind,
    left: &RuntimeValue,
    right: &RuntimeValue,
) -> Result<RuntimeValue, String> {
    use std::cmp::Ordering;

    // Map an `Ordering` (or `None` for an unordered/NaN pair) to the operator.
    let rel = |ord: Option<Ordering>| -> bool {
        match ord {
            None => false,
            Some(o) => match op {
                BinaryOpKind::Lt => o == Ordering::Less,
                BinaryOpKind::Gt => o == Ordering::Greater,
                BinaryOpKind::LtEq => o != Ordering::Greater,
                BinaryOpKind::GtEq => o != Ordering::Less,
                _ => false,
            },
        }
    };
    let int_rel = |a: i64, b: i64| rel(Some(a.cmp(&b)));

    match (left, right) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Bool(int_rel(*a, *b))),
        (RuntimeValue::Float(a), RuntimeValue::Float(b)) => {
            Ok(RuntimeValue::Bool(rel(a.partial_cmp(b))))
        }
        (RuntimeValue::Int(a), RuntimeValue::Float(b)) => {
            Ok(RuntimeValue::Bool(rel((*a as f64).partial_cmp(b))))
        }
        (RuntimeValue::Float(a), RuntimeValue::Int(b)) => {
            Ok(RuntimeValue::Bool(rel(a.partial_cmp(&(*b as f64)))))
        }
        (RuntimeValue::Duration(a), RuntimeValue::Duration(b)) => {
            Ok(RuntimeValue::Bool(int_rel(*a, *b)))
        }
        (RuntimeValue::Date(a), RuntimeValue::Date(b)) => {
            Ok(RuntimeValue::Bool(int_rel(*a as i64, *b as i64)))
        }
        (RuntimeValue::Moment(a), RuntimeValue::Moment(b)) => {
            Ok(RuntimeValue::Bool(int_rel(*a, *b)))
        }
        (RuntimeValue::Time(a), RuntimeValue::Time(b)) => Ok(RuntimeValue::Bool(int_rel(*a, *b))),
        // Moment vs Time: extract time-of-day from Moment. Use Euclidean
        // remainder so a pre-epoch (negative) Moment yields a 0..86399 ns
        // time-of-day, not a negative one.
        (RuntimeValue::Moment(m), RuntimeValue::Time(t)) => {
            let nanos_per_day = 86_400_000_000_000i64;
            Ok(RuntimeValue::Bool(int_rel(m.rem_euclid(nanos_per_day), *t)))
        }
        (RuntimeValue::Time(t), RuntimeValue::Moment(m)) => {
            let nanos_per_day = 86_400_000_000_000i64;
            Ok(RuntimeValue::Bool(int_rel(*t, m.rem_euclid(nanos_per_day))))
        }
        _ => Err(format!(
            "Cannot compare {} and {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_equality_is_epsilon_and_nan_unequal() {
        let sum = RuntimeValue::Float(0.1 + 0.2);
        assert!(values_equal(&sum, &RuntimeValue::Float(0.3)));
        assert!(!values_equal(&RuntimeValue::Float(f64::NAN), &RuntimeValue::Float(f64::NAN)));
        assert!(!values_equal(&RuntimeValue::Int(1), &RuntimeValue::Float(1.0)));
    }

    #[test]
    fn collections_are_never_equal() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let a = RuntimeValue::List(Rc::new(RefCell::new(
            crate::interpreter::ListRepr::from_values(vec![RuntimeValue::Int(1)]),
        )));
        let b = RuntimeValue::List(Rc::new(RefCell::new(
            crate::interpreter::ListRepr::from_values(vec![RuntimeValue::Int(1)]),
        )));
        assert!(!values_equal(&a, &b));
    }

    #[test]
    fn nan_relational_comparisons_are_false_not_errors() {
        let nan = RuntimeValue::Float(f64::NAN);
        for op in [BinaryOpKind::Lt, BinaryOpKind::Gt, BinaryOpKind::LtEq, BinaryOpKind::GtEq] {
            let r = compare(op, &nan, &RuntimeValue::Float(1.0)).unwrap();
            assert!(matches!(r, RuntimeValue::Bool(false)));
        }
    }

    #[test]
    fn moment_compares_to_time_by_time_of_day() {
        let nanos_per_day = 86_400_000_000_000i64;
        // A moment at 10:00 into some day vs a time of 11:00.
        let m = RuntimeValue::Moment(3 * nanos_per_day + 10 * 3_600_000_000_000);
        let t = RuntimeValue::Time(11 * 3_600_000_000_000);
        assert!(matches!(compare(BinaryOpKind::Lt, &m, &t).unwrap(), RuntimeValue::Bool(true)));
        assert!(matches!(compare(BinaryOpKind::Gt, &t, &m).unwrap(), RuntimeValue::Bool(true)));
    }

    #[test]
    fn comparison_type_error_message() {
        let e = compare(BinaryOpKind::Lt, &RuntimeValue::Bool(true), &RuntimeValue::Int(1))
            .unwrap_err();
        assert_eq!(e, "Cannot compare Bool and Int");
    }
}
