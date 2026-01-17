//! Phase 103: User-Defined Polymorphic Inductives
//!
//! Tests the elaboration bridge from LOGOS generics to kernel polymorphic inductives.
//!
//! Architecture:
//! - LOGOS surface: `A List of [T] is either: ...`
//! - TypeRegistry: `Enum { generics: [T], variants: [...] }`
//! - Kernel: `Π(A:Type). Type` with polymorphic constructors

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_output;

// =============================================================================
// PHASE 103a: Parsing polymorphic "is either:" (Should work from Phase 34)
// =============================================================================

#[test]
fn test_parse_polymorphic_list() {
    // Should parse without errors
    let source = r#"
## A MyList of [T] is either:
    An Empty.
    A Cons with head T and tail MyList of T.

## Main
Show "parsed".
"#;
    let result = logicaffeine_compile::compile::compile_to_rust(source);
    assert!(result.is_ok(), "Should parse polymorphic inductive: {:?}", result);
}

#[test]
fn test_parse_multi_param_generic() {
    let source = r#"
## An Either of [L] and [R] is either:
    A Left with value L.
    A Right with value R.

## Main
Show "parsed".
"#;
    let result = logicaffeine_compile::compile::compile_to_rust(source);
    assert!(result.is_ok(), "Should parse multi-param generic: {:?}", result);
}

// =============================================================================
// PHASE 103b: Codegen for polymorphic enums
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_polymorphic_list_int() {
    assert_output(
        r#"## A MyList of [T] is either:
    An Empty.
    A Cons with head T and tail MyList of T.

## Main
Let empty be a new Empty.
Let single be a new Cons with head 42 and tail empty.
Inspect single:
    When Empty: Show "empty".
    When Cons (h, t): Show h.
"#,
        "42",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_polymorphic_list_text() {
    assert_output(
        r#"## A MyList of [T] is either:
    An Empty.
    A Cons with head T and tail MyList of T.

## Main
Let empty be a new Empty.
Let single be a new Cons with head "hello" and tail empty.
Inspect single:
    When Empty: Show "empty".
    When Cons (h, t): Show h.
"#,
        "hello",
    );
}

// =============================================================================
// PHASE 103c: Multi-parameter generics
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_polymorphic_either() {
    assert_output(
        r#"## An Either of [L] and [R] is either:
    A Left with value L.
    A Right with value R.

## Main
Let x be a new Left with value 42.
Inspect x:
    When Left (v): Show v.
    When Right (v): Show "right".
"#,
        "42",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_polymorphic_either_right() {
    assert_output(
        r#"## An Either of [L] and [R] is either:
    A Left with value L.
    A Right with value R.

## Main
Let x be a new Right with value "success".
Inspect x:
    When Left (v): Show "left".
    When Right (v): Show v.
"#,
        "success",
    );
}

// =============================================================================
// PHASE 103d: Recursive polymorphic types
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_recursive_polymorphic_list() {
    assert_output(
        r#"## A MyList of [T] is either:
    An Empty.
    A Cons with head T and tail MyList of T.

## Main
Let e be a new Empty.
Let n1 be a new Cons with head 1 and tail e.
Let n2 be a new Cons with head 2 and tail n1.
Let n3 be a new Cons with head 3 and tail n2.
Inspect n3:
    When Empty: Show "empty".
    When Cons (h, t): Show h.
"#,
        "3",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_nested_polymorphic_inspect() {
    assert_output(
        r#"## A MyList of [T] is either:
    An Empty.
    A Cons with head T and tail MyList of T.

## Main
Let e be a new Empty.
Let n1 be a new Cons with head 1 and tail e.
Inspect n1:
    When Empty: Show "empty".
    When Cons (h, t):
        Inspect t:
            When Empty: Show h.
            When Cons (h2, t2): Show h2.
"#,
        "1",
    );
}

// =============================================================================
// PHASE 103e: Polymorphic with different instantiations
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_polymorphic_option_int() {
    assert_output(
        r#"## A Maybe of [T] is either:
    A Nothing.
    A Just with value T.

## Main
Let x be a new Just with value 42.
Inspect x:
    When Nothing: Show "none".
    When Just (v): Show v.
"#,
        "42",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_polymorphic_option_text() {
    assert_output(
        r#"## A Maybe of [T] is either:
    A Nothing.
    A Just with value T.

## Main
Let x be a new Just with value "hello".
Inspect x:
    When Nothing: Show "none".
    When Just (v): Show v.
"#,
        "hello",
    );
}

// =============================================================================
// PHASE 103f: Polymorphic binary tree
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_polymorphic_tree() {
    assert_output(
        r#"## A Tree of [T] is either:
    A Leaf.
    A Node with value T and left Tree of T and right Tree of T.

## Main
Let leaf be a new Leaf.
Let tree be a new Node with value 42 and left leaf and right leaf.
Inspect tree:
    When Leaf: Show "leaf".
    When Node (v, l, r): Show v.
"#,
        "42",
    );
}
