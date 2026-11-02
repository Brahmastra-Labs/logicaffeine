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
