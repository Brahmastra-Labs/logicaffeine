//! Security Policy Registry.
//!
//! Stores predicate and capability definitions parsed from `## Policy` blocks.
//! These are used to generate security methods on structs and enforce them
//! with the `Check` statement.

use std::collections::HashMap;
use logicaffeine_base::Symbol;

/// Condition in a policy definition.
/// Represents the predicate logic for security rules.
#[derive(Debug, Clone)]
pub enum PolicyCondition {
    /// Field comparison: `the user's role equals "admin"`
    FieldEquals {
        field: Symbol,
        value: Symbol,
        /// Whether the value came from a string literal (needs quotes in codegen)
        is_string_literal: bool,
    },
    /// Boolean field: `the user's verified equals true`
    FieldBool {
        field: Symbol,
        value: bool,
    },
    /// Predicate call: `the user is admin`
    Predicate {
        subject: Symbol,
        predicate: Symbol,
    },
    /// Object field comparison: `the user equals the document's owner`
    ObjectFieldEquals {
        subject: Symbol,
        object: Symbol,
        field: Symbol,
    },
    /// Logical OR: `A OR B`
    Or(Box<PolicyCondition>, Box<PolicyCondition>),
    /// Logical AND: `A AND B`
    And(Box<PolicyCondition>, Box<PolicyCondition>),
}

/// A predicate definition: `A User is admin if the user's role equals "admin".`
#[derive(Debug, Clone)]
pub struct PredicateDef {
    /// The type this predicate applies to (e.g., "User")
    pub subject_type: Symbol,
    /// The predicate name (e.g., "admin")
    pub predicate_name: Symbol,
    /// The condition that must be true
    pub condition: PolicyCondition,
}

/// A capability definition: `A User can publish the Document if...`
#[derive(Debug, Clone)]
pub struct CapabilityDef {
    /// The type that has this capability (e.g., "User")
    pub subject_type: Symbol,
    /// The action name (e.g., "publish")
    pub action: Symbol,
    /// The object type the action applies to (e.g., "Document")
    pub object_type: Symbol,
    /// The condition that must be true
    pub condition: PolicyCondition,
}

/// Registry for security policies defined in `## Policy` blocks.
#[derive(Debug, Default, Clone)]
pub struct PolicyRegistry {
    /// Predicates indexed by subject type
    predicates: HashMap<Symbol, Vec<PredicateDef>>,
    /// Capabilities indexed by subject type
    capabilities: HashMap<Symbol, Vec<CapabilityDef>>,
}

impl PolicyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a predicate definition
    pub fn register_predicate(&mut self, def: PredicateDef) {
        self.predicates
            .entry(def.subject_type)
            .or_insert_with(Vec::new)
            .push(def);
    }

    /// Register a capability definition
    pub fn register_capability(&mut self, def: CapabilityDef) {
        self.capabilities
            .entry(def.subject_type)
            .or_insert_with(Vec::new)
            .push(def);
    }

    /// Get predicates for a type
    pub fn get_predicates(&self, subject_type: Symbol) -> Option<&[PredicateDef]> {
        self.predicates.get(&subject_type).map(|v| v.as_slice())
    }

    /// Get capabilities for a type
    pub fn get_capabilities(&self, subject_type: Symbol) -> Option<&[CapabilityDef]> {
        self.capabilities.get(&subject_type).map(|v| v.as_slice())
    }

    /// Check if a type has any predicates
    pub fn has_predicates(&self, subject_type: Symbol) -> bool {
        self.predicates.contains_key(&subject_type)
    }

    /// Check if a type has any capabilities
    pub fn has_capabilities(&self, subject_type: Symbol) -> bool {
        self.capabilities.contains_key(&subject_type)
    }

    /// Iterate over all types with predicates (for codegen)
    pub fn iter_predicates(&self) -> impl Iterator<Item = (&Symbol, &Vec<PredicateDef>)> {
        self.predicates.iter()
    }

    /// Iterate over all types with capabilities (for codegen)
    pub fn iter_capabilities(&self) -> impl Iterator<Item = (&Symbol, &Vec<CapabilityDef>)> {
        self.capabilities.iter()
    }

    /// Check if registry has any policies
    pub fn is_empty(&self) -> bool {
        self.predicates.is_empty() && self.capabilities.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logicaffeine_base::Interner;

    #[test]
    fn registry_stores_predicates() {
        let mut interner = Interner::new();
        let mut registry = PolicyRegistry::new();

        let user = interner.intern("User");
        let admin = interner.intern("admin");
        let role = interner.intern("role");
        let admin_val = interner.intern("admin");

        let def = PredicateDef {
            subject_type: user,
            predicate_name: admin,
            condition: PolicyCondition::FieldEquals {
                field: role,
                value: admin_val,
                is_string_literal: true,
            },
        };

        registry.register_predicate(def);

        assert!(registry.has_predicates(user));
        let preds = registry.get_predicates(user).unwrap();
        assert_eq!(preds.len(), 1);
        assert_eq!(preds[0].predicate_name, admin);
    }

    #[test]
    fn registry_stores_capabilities() {
        let mut interner = Interner::new();
        let mut registry = PolicyRegistry::new();

        let user = interner.intern("User");
        let doc = interner.intern("Document");
        let publish = interner.intern("publish");
        let admin = interner.intern("admin");
        let user_var = interner.intern("user");

        let def = CapabilityDef {
            subject_type: user,
            action: publish,
            object_type: doc,
            condition: PolicyCondition::Predicate {
                subject: user_var,
                predicate: admin,
            },
        };

        registry.register_capability(def);

        assert!(registry.has_capabilities(user));
        let caps = registry.get_capabilities(user).unwrap();
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].action, publish);
        assert_eq!(caps[0].object_type, doc);
    }
}
