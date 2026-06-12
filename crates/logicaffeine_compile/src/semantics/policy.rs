//! Security-policy evaluation (`Check …` statements).

use crate::analysis::{PolicyCondition, PolicyRegistry};
use crate::intern::{Interner, Symbol};
use crate::interpreter::RuntimeValue;

use super::compare::values_equal;

/// Evaluate one policy condition against a subject (and optional object).
pub fn evaluate_policy_condition(
    registry: Option<&PolicyRegistry>,
    interner: &Interner,
    condition: &PolicyCondition,
    subject: &RuntimeValue,
    object: Option<&RuntimeValue>,
) -> bool {
    match condition {
        PolicyCondition::FieldEquals { field, value, is_string_literal } => {
            if let RuntimeValue::Struct(s) = subject {
                let field_name = interner.resolve(*field);
                if let Some(field_val) = s.fields.get(field_name) {
                    let expected = interner.resolve(*value);
                    match field_val {
                        RuntimeValue::Text(s) => s.as_str() == expected,
                        RuntimeValue::Int(n) => {
                            if *is_string_literal {
                                false
                            } else {
                                expected.parse::<i64>().map(|e| *n == e).unwrap_or(false)
                            }
                        }
                        RuntimeValue::Bool(b) => {
                            if *is_string_literal {
                                false
                            } else {
                                expected.parse::<bool>().map(|e| *b == e).unwrap_or(false)
                            }
                        }
                        _ => false,
                    }
                } else {
                    false
                }
            } else {
                false
            }
        }
        PolicyCondition::FieldBool { field, value } => {
            if let RuntimeValue::Struct(s) = subject {
                let field_name = interner.resolve(*field);
                if let Some(RuntimeValue::Bool(b)) = s.fields.get(field_name) {
                    *b == *value
                } else {
                    false
                }
            } else {
                false
            }
        }
        PolicyCondition::Predicate { predicate, .. } => {
            if let Some(registry) = registry {
                if let RuntimeValue::Struct(s) = subject {
                    if let Some(subj_type_sym) = interner.lookup(&s.type_name) {
                        if let Some(preds) = registry.get_predicates(subj_type_sym) {
                            if let Some(pred) =
                                preds.iter().find(|p| p.predicate_name == *predicate)
                            {
                                return evaluate_policy_condition(
                                    Some(registry),
                                    interner,
                                    &pred.condition,
                                    subject,
                                    object,
                                );
                            }
                        }
                    }
                }
            }
            false
        }
        PolicyCondition::ObjectFieldEquals { subject: subj_field, object: obj_sym, field } => {
            let obj = match object {
                Some(o) => o,
                None => return false,
            };
            if let (RuntimeValue::Struct(subj_s), RuntimeValue::Struct(obj_s)) = (subject, obj) {
                let subj_field_name = interner.resolve(*subj_field);
                let obj_field_name = interner.resolve(*field);
                if let (Some(subj_val), Some(obj_val)) =
                    (subj_s.fields.get(subj_field_name), obj_s.fields.get(obj_field_name))
                {
                    values_equal(subj_val, obj_val)
                } else {
                    let _obj_sym_name = interner.resolve(*obj_sym);
                    false
                }
            } else {
                false
            }
        }
        PolicyCondition::Or(left, right) => {
            evaluate_policy_condition(registry, interner, left, subject, object)
                || evaluate_policy_condition(registry, interner, right, subject, object)
        }
        PolicyCondition::And(left, right) => {
            evaluate_policy_condition(registry, interner, left, subject, object)
                && evaluate_policy_condition(registry, interner, right, subject, object)
        }
    }
}

/// Run a `Check` statement against already-resolved subject/object values.
/// Every error string lives here — both engines produce the identical text.
pub fn check_policy(
    registry: &PolicyRegistry,
    interner: &Interner,
    subject: &RuntimeValue,
    predicate: Symbol,
    is_capability: bool,
    object: Option<&RuntimeValue>,
    source_text: &str,
) -> Result<(), String> {
    let subj_type_name = match subject {
        RuntimeValue::Struct(s) => s.type_name.clone(),
        _ => {
            return Err(format!(
                "Check subject must be a struct, got {}",
                subject.type_name()
            ));
        }
    };

    let subj_type_sym = match interner.lookup(&subj_type_name) {
        Some(sym) => sym,
        None => {
            return Err(format!("Unknown type '{}' in Check statement", subj_type_name));
        }
    };

    let passed = if is_capability {
        let caps = registry.get_capabilities(subj_type_sym);
        let cap = caps.and_then(|caps| caps.iter().find(|c| c.action == predicate));
        match cap {
            Some(cap) => evaluate_policy_condition(
                Some(registry),
                interner,
                &cap.condition,
                subject,
                object,
            ),
            None => {
                let pred_name = interner.resolve(predicate);
                return Err(format!(
                    "No capability '{}' defined for type '{}'",
                    pred_name, subj_type_name
                ));
            }
        }
    } else {
        let preds = registry.get_predicates(subj_type_sym);
        let pred_def = preds.and_then(|preds| preds.iter().find(|p| p.predicate_name == predicate));
        match pred_def {
            Some(pred) => evaluate_policy_condition(
                Some(registry),
                interner,
                &pred.condition,
                subject,
                None,
            ),
            None => {
                let pred_name = interner.resolve(predicate);
                return Err(format!(
                    "No predicate '{}' defined for type '{}'",
                    pred_name, subj_type_name
                ));
            }
        }
    };

    if !passed {
        return Err(format!("Security Check Failed: {}", source_text));
    }
    Ok(())
}
