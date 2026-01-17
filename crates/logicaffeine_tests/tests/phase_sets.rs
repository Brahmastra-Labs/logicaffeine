//! Phase Sets: Set Collection Type
//!
//! Tests for Set type registration.
//! Full parsing and runtime tests are in e2e_sets.rs

use logicaffeine_language::analysis::{TypeRegistry, TypeDef};
use logicaffeine_base::Interner;

// === SET TYPE REGISTRATION ===

#[test]
fn set_is_registered_as_generic() {
    let mut interner = Interner::new();
    let registry = TypeRegistry::with_primitives(&mut interner);

    let set_sym = interner.intern("Set");
    assert!(registry.is_type(set_sym), "Set should be registered as a type");
    assert!(registry.is_generic(set_sym), "Set should be a generic type");

    if let Some(def) = registry.get(set_sym) {
        match def {
            TypeDef::Generic { param_count } => {
                assert_eq!(*param_count, 1, "Set should have 1 type parameter");
            }
            _ => panic!("Set should be a Generic type"),
        }
    } else {
        panic!("Set should be in registry");
    }
}

#[test]
fn char_and_byte_are_primitives() {
    let mut interner = Interner::new();
    let registry = TypeRegistry::with_primitives(&mut interner);

    let char_sym = interner.intern("Char");
    let byte_sym = interner.intern("Byte");

    assert!(registry.is_type(char_sym), "Char should be registered");
    assert!(registry.is_type(byte_sym), "Byte should be registered");
    assert!(!registry.is_generic(char_sym), "Char should not be generic");
    assert!(!registry.is_generic(byte_sym), "Byte should not be generic");
}
