use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::analysis::policy::{PolicyRegistry, PredicateDef, CapabilityDef, PolicyCondition};
use crate::intern::{Interner, Symbol};

/// Generate impl blocks with predicate and capability methods for security policies.
pub(super) fn codegen_policy_impls(policies: &PolicyRegistry, interner: &Interner) -> String {
    let mut output = String::new();

    // Collect all types that have policies
    let mut type_predicates: HashMap<Symbol, Vec<&PredicateDef>> = HashMap::new();
    let mut type_capabilities: HashMap<Symbol, Vec<&CapabilityDef>> = HashMap::new();

    for (type_sym, predicates) in policies.iter_predicates() {
        type_predicates.entry(*type_sym).or_insert_with(Vec::new).extend(predicates.iter());
    }

    for (type_sym, capabilities) in policies.iter_capabilities() {
        type_capabilities.entry(*type_sym).or_insert_with(Vec::new).extend(capabilities.iter());
    }

    // Get all types that have any policies
    let mut all_types: HashSet<Symbol> = HashSet::new();
    all_types.extend(type_predicates.keys().copied());
    all_types.extend(type_capabilities.keys().copied());

    // Generate impl block for each type
    for type_sym in all_types {
        let type_name = interner.resolve(type_sym);

        writeln!(output, "impl {} {{", type_name).unwrap();

        // Generate predicate methods
        if let Some(predicates) = type_predicates.get(&type_sym) {
            for pred in predicates {
                let pred_name = interner.resolve(pred.predicate_name).to_lowercase();
                writeln!(output, "    pub fn is_{}(&self) -> bool {{", pred_name).unwrap();
                let condition_code = codegen_policy_condition(&pred.condition, interner);
                writeln!(output, "        {}", condition_code).unwrap();
                writeln!(output, "    }}\n").unwrap();
            }
        }

        // Generate capability methods
        if let Some(capabilities) = type_capabilities.get(&type_sym) {
            for cap in capabilities {
                let action_name = interner.resolve(cap.action).to_lowercase();
                let object_type = interner.resolve(cap.object_type);
                let object_param = object_type.to_lowercase();

                writeln!(output, "    pub fn can_{}(&self, {}: &{}) -> bool {{",
                         action_name, object_param, object_type).unwrap();
                let condition_code = codegen_policy_condition(&cap.condition, interner);
                writeln!(output, "        {}", condition_code).unwrap();
                writeln!(output, "    }}\n").unwrap();
            }
        }

        writeln!(output, "}}\n").unwrap();
    }

    output
}

/// Generate Rust code for a policy condition.
pub(super) fn codegen_policy_condition(condition: &PolicyCondition, interner: &Interner) -> String {
    match condition {
        PolicyCondition::FieldEquals { field, value, is_string_literal } => {
            let field_name = interner.resolve(*field);
            let value_str = interner.resolve(*value);
            if *is_string_literal {
                format!("self.{} == \"{}\"", field_name, value_str)
            } else {
                format!("self.{} == {}", field_name, value_str)
            }
        }
        PolicyCondition::FieldBool { field, value } => {
            let field_name = interner.resolve(*field);
            format!("self.{} == {}", field_name, value)
        }
        PolicyCondition::Predicate { subject: _, predicate } => {
            let pred_name = interner.resolve(*predicate).to_lowercase();
            format!("self.is_{}()", pred_name)
        }
        PolicyCondition::ObjectFieldEquals { subject: _, object, field } => {
            let object_name = interner.resolve(*object).to_lowercase();
            let field_name = interner.resolve(*field);
            format!("self == &{}.{}", object_name, field_name)
        }
        PolicyCondition::Or(left, right) => {
            let left_code = codegen_policy_condition(left, interner);
            let right_code = codegen_policy_condition(right, interner);
            format!("{} || {}", left_code, right_code)
        }
        PolicyCondition::And(left, right) => {
            let left_code = codegen_policy_condition(left, interner);
            let right_code = codegen_policy_condition(right, interner);
            format!("{} && {}", left_code, right_code)
        }
    }
}
