//! Phase 101c: List Operations
//!
//! Defines the List type and basic operations:
//! - append: List A -> List A -> List A
//! - map: (A -> B) -> List A -> List B
//! - length: List A -> Nat
//!
//! Operations are defined using Fix terms for recursion.

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// LIST TYPE DEFINITION
// =============================================================================

#[test]
fn test_define_list() {
    let mut repl = Repl::new();

    // Define polymorphic List (from Phase 101a)
    let result = repl.execute(
        "Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A."
    );

    assert!(result.is_ok(), "Should define List: {:?}", result);

    // Check List type
    let output = repl.execute("Check List.").expect("Check List");
    assert!(output.contains("Type"), "List should be a type constructor: {}", output);
}

#[test]
fn test_list_nat_values() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    // Check empty list
    let output = repl.execute("Check (Nil Nat).").expect("Check Nil Nat");
    assert!(output.contains("List"), "Nil Nat should have type List Nat: {}", output);

    // Check singleton list [0]
    let output = repl.execute("Check (Cons Nat Zero (Nil Nat)).").expect("Check singleton");
    assert!(output.contains("List"), "Singleton should have type List Nat: {}", output);

    // Check [0, 1]
    let output = repl.execute("Check (Cons Nat Zero (Cons Nat (Succ Zero) (Nil Nat))).").expect("Check [0,1]");
    assert!(output.contains("List"), "[0,1] should have type List Nat: {}", output);
}

// =============================================================================
// APPEND OPERATION
// =============================================================================

#[test]
fn test_append_type() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    // Define append using fix
    // append : forall A, List A -> List A -> List A
    // Note: The term parser uses "forall A : Type, ..." syntax (not parentheses)
    // and "fix f => ..." syntax (no type annotation on fix)
    let result = repl.execute(r#"
        Definition append : forall A : Type, List A -> List A -> List A :=
            fun A : Type =>
            fix rec =>
            fun xs : List A =>
            fun ys : List A =>
            match xs return List A with
            | Nil => ys
            | Cons h t => Cons A h (rec t ys)
            end.
    "#);

    assert!(result.is_ok(), "Should define append: {:?}", result);

    // Check type
    let output = repl.execute("Check append.").expect("Check append");
    assert!(output.contains("List") && output.contains("->"), "append should have function type: {}", output);
}

#[test]
fn test_append_nil_left() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    repl.execute(r#"
        Definition append : forall A : Type, List A -> List A -> List A :=
            fun A : Type =>
            fix rec =>
            fun xs : List A =>
            fun ys : List A =>
            match xs return List A with
            | Nil => ys
            | Cons h t => Cons A h (rec t ys)
            end.
    "#).expect("Define append");

    // append Nat (Nil Nat) ys = ys
    // Let's test with ys = Cons Nat Zero (Nil Nat)
    let output = repl.execute("Eval (append Nat (Nil Nat) (Cons Nat Zero (Nil Nat))).").expect("Eval append nil");

    assert!(output.contains("Cons") && output.contains("Zero"),
        "append [] [0] should be [0]: {}", output);
}

#[test]
fn test_append_cons_left() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    repl.execute(r#"
        Definition append : forall A : Type, List A -> List A -> List A :=
            fun A : Type =>
            fix rec =>
            fun xs : List A =>
            fun ys : List A =>
            match xs return List A with
            | Nil => ys
            | Cons h t => Cons A h (rec t ys)
            end.
    "#).expect("Define append");

    // append Nat (Cons Nat Zero (Nil Nat)) (Nil Nat) = Cons Nat Zero (Nil Nat)
    let output = repl.execute("Eval (append Nat (Cons Nat Zero (Nil Nat)) (Nil Nat)).").expect("Eval append");

    assert!(output.contains("Cons") && output.contains("Zero"),
        "append [0] [] should be [0]: {}", output);
}

#[test]
fn test_append_two_lists() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    repl.execute(r#"
        Definition append : forall A : Type, List A -> List A -> List A :=
            fun A : Type =>
            fix rec =>
            fun xs : List A =>
            fun ys : List A =>
            match xs return List A with
            | Nil => ys
            | Cons h t => Cons A h (rec t ys)
            end.
    "#).expect("Define append");

    // [0] ++ [1] = [0, 1]
    let output = repl.execute(r#"
        Eval (append Nat
            (Cons Nat Zero (Nil Nat))
            (Cons Nat (Succ Zero) (Nil Nat))).
    "#).expect("Eval append two lists");

    // Result should contain both Zero and Succ
    assert!(output.contains("Cons") && output.contains("Zero") && output.contains("Succ"),
        "append [0] [1] should be [0,1]: {}", output);
}

// =============================================================================
// LENGTH OPERATION
// =============================================================================

#[test]
fn test_length_type() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    // length : forall A, List A -> Nat
    let result = repl.execute(r#"
        Definition length : forall A : Type, List A -> Nat :=
            fun A : Type =>
            fix rec =>
            fun xs : List A =>
            match xs return Nat with
            | Nil => Zero
            | Cons h t => Succ (rec t)
            end.
    "#);

    assert!(result.is_ok(), "Should define length: {:?}", result);
}

#[test]
fn test_length_nil() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    repl.execute(r#"
        Definition length : forall A : Type, List A -> Nat :=
            fun A : Type =>
            fix rec =>
            fun xs : List A =>
            match xs return Nat with
            | Nil => Zero
            | Cons h t => Succ (rec t)
            end.
    "#).expect("Define length");

    // length Nat (Nil Nat) = Zero
    let output = repl.execute("Eval (length Nat (Nil Nat)).").expect("Eval length nil");

    assert!(output.contains("Zero") && !output.contains("Succ"),
        "length [] should be 0: {}", output);
}

#[test]
fn test_length_singleton() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    repl.execute(r#"
        Definition length : forall A : Type, List A -> Nat :=
            fun A : Type =>
            fix rec =>
            fun xs : List A =>
            match xs return Nat with
            | Nil => Zero
            | Cons h t => Succ (rec t)
            end.
    "#).expect("Define length");

    // length Nat [0] = Succ Zero = 1
    let output = repl.execute("Eval (length Nat (Cons Nat Zero (Nil Nat))).").expect("Eval length [0]");

    assert!(output.contains("Succ") && output.contains("Zero"),
        "length [0] should be 1: {}", output);
}

#[test]
fn test_length_two() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    repl.execute(r#"
        Definition length : forall A : Type, List A -> Nat :=
            fun A : Type =>
            fix rec =>
            fun xs : List A =>
            match xs return Nat with
            | Nil => Zero
            | Cons h t => Succ (rec t)
            end.
    "#).expect("Define length");

    // length Nat [0, 1] = Succ (Succ Zero) = 2
    let output = repl.execute(r#"
        Eval (length Nat (Cons Nat Zero (Cons Nat (Succ Zero) (Nil Nat)))).
    "#).expect("Eval length [0,1]");

    // Should have two Succ's
    let succ_count = output.matches("Succ").count();
    assert!(succ_count >= 2, "length [0,1] should be 2: {}", output);
}

// =============================================================================
// MAP OPERATION
// =============================================================================

#[test]
fn test_map_type() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    // map : forall A B, (A -> B) -> List A -> List B
    let result = repl.execute(r#"
        Definition map : forall A : Type, forall B : Type, (A -> B) -> List A -> List B :=
            fun A : Type =>
            fun B : Type =>
            fun f : A -> B =>
            fix rec =>
            fun xs : List A =>
            match xs return List B with
            | Nil => Nil B
            | Cons h t => Cons B (f h) (rec t)
            end.
    "#);

    assert!(result.is_ok(), "Should define map: {:?}", result);
}

#[test]
fn test_map_nil() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    repl.execute(r#"
        Definition map : forall A : Type, forall B : Type, (A -> B) -> List A -> List B :=
            fun A : Type =>
            fun B : Type =>
            fun f : A -> B =>
            fix rec =>
            fun xs : List A =>
            match xs return List B with
            | Nil => Nil B
            | Cons h t => Cons B (f h) (rec t)
            end.
    "#).expect("Define map");

    // map Nat Nat Succ (Nil Nat) = Nil Nat
    let output = repl.execute("Eval (map Nat Nat Succ (Nil Nat)).").expect("Eval map nil");

    assert!(output.contains("Nil"), "map f [] should be []: {}", output);
}

#[test]
fn test_map_succ() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    repl.execute(r#"
        Definition map : forall A : Type, forall B : Type, (A -> B) -> List A -> List B :=
            fun A : Type =>
            fun B : Type =>
            fun f : A -> B =>
            fix rec =>
            fun xs : List A =>
            match xs return List B with
            | Nil => Nil B
            | Cons h t => Cons B (f h) (rec t)
            end.
    "#).expect("Define map");

    // map Nat Nat Succ [0] = [1]
    let output = repl.execute("Eval (map Nat Nat Succ (Cons Nat Zero (Nil Nat))).").expect("Eval map succ [0]");

    // Should have Succ Zero (which is 1)
    assert!(output.contains("Succ") && output.contains("Zero"),
        "map Succ [0] should be [1]: {}", output);
}

#[test]
fn test_map_preserves_length() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    repl.execute(r#"
        Definition map : forall A : Type, forall B : Type, (A -> B) -> List A -> List B :=
            fun A : Type =>
            fun B : Type =>
            fun f : A -> B =>
            fix rec =>
            fun xs : List A =>
            match xs return List B with
            | Nil => Nil B
            | Cons h t => Cons B (f h) (rec t)
            end.
    "#).expect("Define map");

    repl.execute(r#"
        Definition length : forall A : Type, List A -> Nat :=
            fun A : Type =>
            fix rec =>
            fun xs : List A =>
            match xs return Nat with
            | Nil => Zero
            | Cons h t => Succ (rec t)
            end.
    "#).expect("Define length");

    // length (map Nat Nat Succ [0, 1]) = length [0, 1] = 2
    let list = "(Cons Nat Zero (Cons Nat (Succ Zero) (Nil Nat)))";

    let len_original = repl.execute(&format!("Eval (length Nat {}).", list)).expect("Eval length original");
    let len_mapped = repl.execute(&format!("Eval (length Nat (map Nat Nat Succ {})).", list)).expect("Eval length mapped");

    // Both should have same structure (two Succ's)
    let succ_count_orig = len_original.matches("Succ").count();
    let succ_count_mapped = len_mapped.matches("Succ").count();

    assert_eq!(succ_count_orig, succ_count_mapped,
        "map should preserve length. Original: {}, Mapped: {}", len_original, len_mapped);
}

// =============================================================================
// IDENTITY FUNCTION
// =============================================================================

#[test]
fn test_id_function() {
    let mut repl = Repl::new();

    // Define polymorphic identity
    let result = repl.execute(r#"
        Definition id : forall A : Type, A -> A :=
            fun A : Type => fun x : A => x.
    "#);

    assert!(result.is_ok(), "Should define id: {:?}", result);

    // Check type
    let output = repl.execute("Check id.").expect("Check id");
    assert!(output.contains("->"), "id should be a function: {}", output);

    // Test: id Nat Zero = Zero
    let output = repl.execute("Eval (id Nat Zero).").expect("Eval id Nat Zero");
    assert!(output.contains("Zero") && !output.contains("Succ"),
        "id Nat Zero should be Zero: {}", output);
}

// =============================================================================
// COMBINED OPERATIONS
// =============================================================================

#[test]
fn test_length_append() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    repl.execute(r#"
        Definition append : forall A : Type, List A -> List A -> List A :=
            fun A : Type =>
            fix rec =>
            fun xs : List A =>
            fun ys : List A =>
            match xs return List A with
            | Nil => ys
            | Cons h t => Cons A h (rec t ys)
            end.
    "#).expect("Define append");

    repl.execute(r#"
        Definition length : forall A : Type, List A -> Nat :=
            fun A : Type =>
            fix rec =>
            fun xs : List A =>
            match xs return Nat with
            | Nil => Zero
            | Cons h t => Succ (rec t)
            end.
    "#).expect("Define length");

    // length ([0] ++ [1, 2]) = 3
    let output = repl.execute(r#"
        Eval (length Nat (append Nat
            (Cons Nat Zero (Nil Nat))
            (Cons Nat (Succ Zero) (Cons Nat (Succ (Succ Zero)) (Nil Nat))))).
    "#).expect("Eval length append");

    // Should have 3 Succ's
    let succ_count = output.matches("Succ").count();
    assert!(succ_count >= 3, "length ([0] ++ [1,2]) should be 3: {}", output);
}
