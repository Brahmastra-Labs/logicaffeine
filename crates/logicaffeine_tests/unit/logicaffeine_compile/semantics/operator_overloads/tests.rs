use std::cell::RefCell;
use std::rc::Rc;

use super::{core_op_text, self_applied, Guard, OpOverload, Refusal, SelfApplied, ALL_OPERATORS};
use crate::ast::stmt::BinaryOpKind;
use crate::interpreter::{ListRepr, RuntimeValue};
use crate::semantics::arith::binary_op;

/// The operators the self-interpreter applies may only grow.
const SELF_APPLIED_OPERATORS: usize = 23;

/// A value of each type an operand can be, as the tree-walker holds it, the name it gives its type, and the table's
/// name for it.
fn values() -> Vec<(RuntimeValue, &'static str)> {
    vec![
        (RuntimeValue::Int(7), "Int"),
        (RuntimeValue::Float(2.5), "Real"),
        (RuntimeValue::Text(Rc::new("7".to_string())), "Text"),
        (RuntimeValue::Bool(true), "Bool"),
        (RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vec![RuntimeValue::Int(1)])))), "Seq"),
        (RuntimeValue::Duration(1_500_000_007), "Duration"),
        (RuntimeValue::Date(20_000), "Date"),
        (RuntimeValue::Moment(1_710_498_645_123_456_789), "Moment"),
        (RuntimeValue::Span { months: 14, days: 3 }, "Span"),
        (RuntimeValue::Time(37_845_500_000_000), "Time"),
        (RuntimeValue::Char('x'), "Char"),
        (RuntimeValue::Nothing, "Nothing"),
    ]
}

fn runtime_name(ty: &str) -> &'static str {
    match ty {
        "Int" => "Int",
        "Real" => "Float",
        "Text" => "Text",
        "Bool" => "Bool",
        "Seq" => "List",
        "Duration" => "Duration",
        "Date" => "Date",
        "Moment" => "Moment",
        "Span" => "Span",
        "Time" => "Time",
        "Char" => "Char",
        other => panic!("no runtime name of `{other}`"),
    }
}

/// Whether an operand typed `ty` takes a value the table calls `of`.
fn takes(ty: &str, of: &str) -> bool {
    ty == "Any" || ty == of
}

fn typed() -> Vec<(BinaryOpKind, &'static [OpOverload], &'static [Refusal])> {
    ALL_OPERATORS
        .iter()
        .filter_map(|op| match self_applied(*op) {
            SelfApplied::Overloads { overloads, refusal, .. } => Some((*op, overloads, refusal)),
            _ => None,
        })
        .collect()
}

/// The operand pairs an overload takes, as the tree-walker holds them.
fn taken(overload: &OpOverload) -> Vec<(RuntimeValue, RuntimeValue)> {
    let mut pairs = Vec::new();
    for (left, left_ty) in values() {
        for (right, right_ty) in values() {
            if takes(overload.params[0], left_ty) && takes(overload.params[1], right_ty) {
                pairs.push((left.clone(), right));
            }
        }
    }
    pairs
}

#[test]
fn every_overload_answers_on_the_tree_walker_as_the_table_says() {
    for (op, overloads, _) in typed() {
        for overload in overloads {
            let pairs = taken(overload);
            assert!(!pairs.is_empty(), "{op:?} {overload:?} takes no operand");
            for (left, right) in pairs {
                let what = format!("{op:?} {overload:?} on {} and {}", left.type_name(), right.type_name());
                let answer = binary_op(op, left, right).unwrap_or_else(|e| panic!("{what}: the tree-walker refuses it: {e}"));
                assert_eq!(answer.type_name(), runtime_name(overload.answer), "{what}");
            }
        }
    }
}

/// The right operand that trips `guard`.
fn tripping(guard: &Guard) -> RuntimeValue {
    match guard.when {
        "{right} equals 0" => RuntimeValue::Int(0),
        "{right} equals 0.0" => RuntimeValue::Float(0.0),
        "{right} is less than 0" => RuntimeValue::Int(-1),
        other => panic!("no operand trips the guard `{other}`"),
    }
}

#[test]
fn every_guard_stops_as_the_tree_walker_stops() {
    let mut guarded = 0;
    for (op, overloads, _) in typed() {
        for overload in overloads {
            for guard in overload.guards {
                guarded += 1;
                let left = values()
                    .into_iter()
                    .find(|(_, ty)| takes(overload.params[0], ty))
                    .map(|(v, _)| v)
                    .expect("a left operand");
                let right = tripping(guard);
                assert!(
                    takes(overload.params[1], match right {
                        RuntimeValue::Float(_) => "Real",
                        _ => "Int",
                    }),
                    "{op:?} {overload:?}: the guard `{}` trips on an operand the overload does not take",
                    guard.when
                );
                assert_eq!(
                    binary_op(op, left, right).err().as_deref(),
                    Some(guard.report),
                    "{op:?} {overload:?}: the guard `{}`",
                    guard.when
                );
            }
        }
    }
    assert!(guarded >= 9, "the table guards its divisions, moduli and powers");
}

#[test]
fn every_application_no_overload_takes_stops_with_the_tree_walkers_refusal() {
    for (op, overloads, refusal) in typed() {
        for (left, left_ty) in values() {
            for (right, right_ty) in values() {
                if overloads.iter().any(|o| takes(o.params[0], left_ty) && takes(o.params[1], right_ty)) {
                    continue;
                }
                let case = refusal
                    .iter()
                    .find(|c| (c.left == "*" || c.left == left.type_name()) && (c.right == "*" || c.right == right.type_name()))
                    .unwrap_or_else(|| panic!("{op:?} on {} and {}: no case of the refusal is for them", left.type_name(), right.type_name()));
                let expected = case.report.replace("{1}", left.type_name()).replace("{2}", right.type_name());
                let what = format!("{op:?} on {} and {}", left.type_name(), right.type_name());
                assert_eq!(
                    binary_op(op, left.clone(), right.clone()).err().as_deref(),
                    Some(expected.as_str()),
                    "{what}: the language applies what the table does not, or refuses it in other words"
                );
            }
        }
    }
}

#[test]
fn every_operator_has_one_core_ir_text() {
    let mut seen: Vec<(&str, BinaryOpKind)> = Vec::new();
    for op in ALL_OPERATORS {
        let text = core_op_text(*op);
        if let Some((_, other)) = seen.iter().find(|(t, _)| *t == text) {
            panic!("{other:?} and {op:?} share the Core IR text `{text}`: the self-interpreter cannot tell them apart");
        }
        seen.push((text, *op));
    }
}

#[test]
fn combined_with_is_not_addition() {
    assert_ne!(
        core_op_text(BinaryOpKind::Concat),
        core_op_text(BinaryOpKind::Add),
        "`1 combined with 2` is `12` and `1 + 2` is `3`: the Core IR keeps them apart"
    );
    let shown = binary_op(BinaryOpKind::Concat, RuntimeValue::Int(1), RuntimeValue::Int(2)).expect("concatenates");
    assert_eq!(shown.to_display_string(), "12");
}

#[test]
fn the_self_interpreter_applies_at_least_its_floor_of_operators() {
    let applied = ALL_OPERATORS.iter().filter(|op| !matches!(self_applied(**op), SelfApplied::Awaits(_))).count();
    assert!(
        applied >= SELF_APPLIED_OPERATORS,
        "the self-interpreter applies {applied} operators, below its floor {SELF_APPLIED_OPERATORS}"
    );
}
