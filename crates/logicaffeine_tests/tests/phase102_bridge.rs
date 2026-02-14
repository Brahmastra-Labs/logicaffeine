//! Phase 102: Kernel-Runtime Bridge
//!
//! Connects kernel inductive types to the imperative interpreter.
//! This enables a unified type system where all sum types are kernel inductives.
//!
//! Architecture:
//! - Compile Time: Types live in Kernel (verification, exhaustiveness, proofs)
//! - Runtime: Types are erased to efficient representations (u8, Box, etc.)
//!
//! The "Dual Life" architecture: Soul (Kernel) + Body (Rust).

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_output;

use logicaffeine_compile::interpreter::{RuntimeValue, InductiveValue};

// =============================================================================
// PHASE 102a: RuntimeValue::Inductive variant
// =============================================================================

#[test]
fn test_runtime_inductive_zero() {
    let val = RuntimeValue::Inductive(Box::new(InductiveValue {
        inductive_type: "Nat".to_string(),
        constructor: "Zero".to_string(),
        args: vec![],
    }));
    assert_eq!(val.type_name(), "Nat");
}

#[test]
fn test_runtime_inductive_succ() {
    let zero = RuntimeValue::Inductive(Box::new(InductiveValue {
        inductive_type: "Nat".to_string(),
        constructor: "Zero".to_string(),
        args: vec![],
    }));
    let one = RuntimeValue::Inductive(Box::new(InductiveValue {
        inductive_type: "Nat".to_string(),
        constructor: "Succ".to_string(),
        args: vec![zero],
    }));
    assert_eq!(one.type_name(), "Nat");
    assert!(one.to_display_string().contains("Succ"));
}

#[test]
fn test_runtime_inductive_list() {
    let nil = RuntimeValue::Inductive(Box::new(InductiveValue {
        inductive_type: "List".to_string(),
        constructor: "Nil".to_string(),
        args: vec![],
    }));
    let cons = RuntimeValue::Inductive(Box::new(InductiveValue {
        inductive_type: "List".to_string(),
        constructor: "Cons".to_string(),
        args: vec![RuntimeValue::Int(42), nil],
    }));
    assert_eq!(cons.type_name(), "List");
    assert!(cons.to_display_string().contains("Cons"));
}

#[test]
fn test_runtime_inductive_display_nested() {
    let zero = RuntimeValue::Inductive(Box::new(InductiveValue {
        inductive_type: "Nat".to_string(),
        constructor: "Zero".to_string(),
        args: vec![],
    }));
    let one = RuntimeValue::Inductive(Box::new(InductiveValue {
        inductive_type: "Nat".to_string(),
        constructor: "Succ".to_string(),
        args: vec![zero],
    }));
    let two = RuntimeValue::Inductive(Box::new(InductiveValue {
        inductive_type: "Nat".to_string(),
        constructor: "Succ".to_string(),
        args: vec![one],
    }));

    let display = two.to_display_string();
    assert!(display.contains("Succ"), "Display: {}", display);
}

#[test]
fn test_runtime_inductive_equality() {
    use logicaffeine_base::Interner;
    use logicaffeine_compile::interpreter::Interpreter;

    let zero1 = RuntimeValue::Inductive(Box::new(InductiveValue {
        inductive_type: "Nat".to_string(),
        constructor: "Zero".to_string(),
        args: vec![],
    }));
    let zero2 = RuntimeValue::Inductive(Box::new(InductiveValue {
        inductive_type: "Nat".to_string(),
        constructor: "Zero".to_string(),
        args: vec![],
    }));
    let one = RuntimeValue::Inductive(Box::new(InductiveValue {
        inductive_type: "Nat".to_string(),
        constructor: "Succ".to_string(),
        args: vec![zero1.clone()],
    }));

    let interner = Interner::new();
    let interp = Interpreter::new(&interner);
    assert!(interp.values_equal_pub(&zero1, &zero2));
    assert!(!interp.values_equal_pub(&zero1, &one));
}

// =============================================================================
// PHASE 102b: Interpreter kernel context integration
// =============================================================================

#[test]
fn test_interpreter_with_kernel_context() {
    use logicaffeine_kernel::Context;
    use logicaffeine_base::Interner;
    use logicaffeine_compile::interpreter::Interpreter;
    use std::sync::Arc;

    let mut ctx = Context::new();
    ctx.add_inductive("Nat", logicaffeine_kernel::Term::Sort(logicaffeine_kernel::Universe::Type(0)));
    ctx.add_constructor("Zero", "Nat", logicaffeine_kernel::Term::Global("Nat".to_string()));

    let interner = Interner::new();
    let interp = Interpreter::new(&interner)
        .with_kernel(Arc::new(ctx));

    assert!(interp.is_kernel_inductive("Nat"));
    assert!(!interp.is_kernel_inductive("NotAType"));
}

#[test]
fn test_interpreter_get_constructors() {
    use logicaffeine_kernel::Context;
    use logicaffeine_base::Interner;
    use logicaffeine_compile::interpreter::Interpreter;
    use std::sync::Arc;

    let mut ctx = Context::new();
    ctx.add_inductive("Bool", logicaffeine_kernel::Term::Sort(logicaffeine_kernel::Universe::Type(0)));
    ctx.add_constructor("True", "Bool", logicaffeine_kernel::Term::Global("Bool".to_string()));
    ctx.add_constructor("False", "Bool", logicaffeine_kernel::Term::Global("Bool".to_string()));

    let interner = Interner::new();
    let interp = Interpreter::new(&interner)
        .with_kernel(Arc::new(ctx));

    let ctors = interp.get_kernel_constructors("Bool");
    assert_eq!(ctors.len(), 2);
    assert!(ctors.iter().any(|(name, _)| name == "True"));
    assert!(ctors.iter().any(|(name, _)| name == "False"));
}

// =============================================================================
// PHASE 102c-d: Pattern matching and constructor evaluation (E2E)
// These require the full compile pipeline
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_is_one_of_creates_inductive() {
    assert_output(
        r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main
Let c be a new Red.
Inspect c:
    When Red: Show "red".
    When Green: Show "green".
    When Blue: Show "blue".
"#,
        "red",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_is_either_creates_inductive() {
    assert_output(
        r#"## A Shape is either:
    A Circle with radius Int.
    A Rectangle with width Int and height Int.

## Main
Let s be a new Circle with radius 10.
Inspect s:
    When Circle (r): Show r.
    When Rectangle (w, h): Show w.
"#,
        "10",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_recursive_inductive() {
    assert_output(
        r#"## A Peano is either:
    A Zero.
    A Succ with pred Peano.

## Main
Let z be a new Zero.
Let n1 be a new Succ with pred z.
Let n2 be a new Succ with pred n1.
Inspect n2:
    When Zero: Show "zero".
    When Succ (p): Show "succ".
"#,
        "succ",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_nested_inductive_inspect() {
    assert_output(
        r#"## A Peano is either:
    A Zero.
    A Succ with pred Peano.

## Main
Let z be a new Zero.
Let n1 be a new Succ with pred z.
Inspect n1:
    When Zero: Show "zero".
    When Succ (p):
        Inspect p:
            When Zero: Show "it is one".
            When Succ (pp): Show "more".
"#,
        "it is one",
    );
}
