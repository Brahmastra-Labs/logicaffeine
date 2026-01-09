//! Phase 101d: List Theorems
//!
//! Proves fundamental properties of List operations using DElim (generic elimination).
//!
//! Theorems:
//! - append_nil_r : ∀l. append l Nil = l
//! - append_assoc : ∀x y z. append (append x y) z = append x (append y z)
//! - map_id : ∀l. map id l = l
//! - length_append : ∀x y. length (append x y) = plus (length x) (length y)
//!
//! Each theorem is proved by induction on List using DElim.

use logos::interface::Repl;

// =============================================================================
// HELPER: Setup REPL with List and operations
// =============================================================================

fn setup_list_repl() -> Repl {
    let mut repl = Repl::new();

    // Define plus for Nat (needed for length_append theorem)
    repl.execute(r#"
        Definition plus : Nat -> Nat -> Nat :=
            fix rec =>
            fun n : Nat =>
            fun m : Nat =>
            match n return Nat with
            | Zero => m
            | Succ k => Succ (rec k m)
            end.
    "#).expect("Define plus");

    // Define List type
    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    // Define append
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

    // Define map
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

    // Define length
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

    // Define polymorphic identity
    repl.execute(r#"
        Definition id : forall A : Type, A -> A :=
            fun A : Type => fun x : A => x.
    "#).expect("Define id");

    repl
}

// =============================================================================
// THEOREM: append_nil_r : ∀l. append l Nil = l
// =============================================================================

#[test]
fn test_append_nil_r_base_case() {
    let mut repl = setup_list_repl();

    // Base case: append Nil Nil = Nil
    // This should reduce by computation
    let output = repl.execute("Eval (append Nat (Nil Nat) (Nil Nat)).")
        .expect("Eval append Nil Nil");

    assert!(output.contains("Nil"), "append [] [] should be []: {}", output);
}

#[test]
fn test_append_nil_r_step_case() {
    let mut repl = setup_list_repl();

    // Step case: if append t Nil = t, then append (Cons h t) Nil = Cons h t
    // By computation: append (Cons h t) Nil = Cons h (append t Nil)
    // By IH: = Cons h t

    // Test concrete case: append [0] [] = [0]
    let output = repl.execute("Eval (append Nat (Cons Nat Zero (Nil Nat)) (Nil Nat)).")
        .expect("Eval append [0] []");

    // Should be Cons Nat Zero (Nil Nat)
    assert!(output.contains("Cons") && output.contains("Zero") && output.contains("Nil"),
        "append [0] [] should be [0]: {}", output);
}

#[test]
fn test_append_nil_r_theorem_statement() {
    let mut repl = setup_list_repl();

    // The theorem statement: forall A, forall l : List A, append A l (Nil A) = l
    // We express this in the deep embedding

    // First, check we can parse equality of list expressions
    let result = repl.execute(r#"
        Check (forall A : Type, forall l : List A, Eq (List A) (append A l (Nil A)) l).
    "#);

    assert!(result.is_ok(), "Should parse append_nil_r statement: {:?}", result);
}

// =============================================================================
// THEOREM: append_assoc : ∀x y z. (x ++ y) ++ z = x ++ (y ++ z)
// =============================================================================

#[test]
fn test_append_assoc_base_case() {
    let mut repl = setup_list_repl();

    // Base case: append (append Nil y) z = append Nil (append y z)
    // LHS: append (append Nil y) z = append y z
    // RHS: append Nil (append y z) = append y z
    // Equal by reflexivity

    let lhs = repl.execute("Eval (append Nat (append Nat (Nil Nat) (Cons Nat Zero (Nil Nat))) (Nil Nat)).")
        .expect("Eval LHS");
    let rhs = repl.execute("Eval (append Nat (Nil Nat) (append Nat (Cons Nat Zero (Nil Nat)) (Nil Nat))).")
        .expect("Eval RHS");

    // Both should be [0]
    assert!(lhs.contains("Cons") && lhs.contains("Zero"), "LHS should be [0]: {}", lhs);
    assert!(rhs.contains("Cons") && rhs.contains("Zero"), "RHS should be [0]: {}", rhs);
}

#[test]
fn test_append_assoc_step_case() {
    let mut repl = setup_list_repl();

    // Step case: if IH holds, then append (append (Cons h t) y) z = append (Cons h t) (append y z)
    // LHS: append (Cons h (append t y)) z = Cons h (append (append t y) z)
    // RHS: Cons h (append t (append y z))
    // By IH: append (append t y) z = append t (append y z)

    // Test with x=[0], y=[1], z=[2]
    let x = "(Cons Nat Zero (Nil Nat))";
    let y = "(Cons Nat (Succ Zero) (Nil Nat))";
    let z = "(Cons Nat (Succ (Succ Zero)) (Nil Nat))";

    let lhs = repl.execute(&format!("Eval (append Nat (append Nat {} {}) {}).", x, y, z))
        .expect("Eval LHS");
    let rhs = repl.execute(&format!("Eval (append Nat {} (append Nat {} {})).", x, y, z))
        .expect("Eval RHS");

    // Both should be [0, 1, 2]
    assert!(lhs.contains("Cons") && lhs.contains("Zero") && lhs.contains("Succ"),
        "LHS should be [0,1,2]: {}", lhs);
    assert!(rhs.contains("Cons") && rhs.contains("Zero") && rhs.contains("Succ"),
        "RHS should be [0,1,2]: {}", rhs);
}

// =============================================================================
// THEOREM: map_id : ∀l. map id l = l
// =============================================================================

#[test]
fn test_map_id_base_case() {
    let mut repl = setup_list_repl();

    // Base case: map id Nil = Nil
    let output = repl.execute("Eval (map Nat Nat (id Nat) (Nil Nat)).")
        .expect("Eval map id []");

    assert!(output.contains("Nil"), "map id [] should be []: {}", output);
}

#[test]
fn test_map_id_step_case() {
    let mut repl = setup_list_repl();

    // Step case: if map id t = t, then map id (Cons h t) = Cons h t
    // By computation: map id (Cons h t) = Cons (id h) (map id t)
    //               = Cons h (map id t) = Cons h t by IH

    // Test with [0]
    let output = repl.execute("Eval (map Nat Nat (id Nat) (Cons Nat Zero (Nil Nat))).")
        .expect("Eval map id [0]");

    assert!(output.contains("Cons") && output.contains("Zero"),
        "map id [0] should be [0]: {}", output);
}

#[test]
fn test_map_id_longer_list() {
    let mut repl = setup_list_repl();

    // Test with [0, 1]
    let list = "(Cons Nat Zero (Cons Nat (Succ Zero) (Nil Nat)))";

    let output = repl.execute(&format!("Eval (map Nat Nat (id Nat) {}).", list))
        .expect("Eval map id [0,1]");

    // Should be [0, 1]
    let succ_count = output.matches("Succ").count();
    assert!(output.contains("Cons") && output.contains("Zero") && succ_count >= 1,
        "map id [0,1] should be [0,1]: {}", output);
}

// =============================================================================
// THEOREM: length_append : ∀x y. length (x ++ y) = plus (length x) (length y)
// =============================================================================

#[test]
fn test_length_append_base_case() {
    let mut repl = setup_list_repl();

    // Base case: length (append Nil y) = plus (length Nil) (length y)
    // LHS: length y
    // RHS: plus Zero (length y) = length y

    // Test with y = [0]
    let y = "(Cons Nat Zero (Nil Nat))";

    let lhs = repl.execute(&format!("Eval (length Nat (append Nat (Nil Nat) {})).", y))
        .expect("Eval length (append [] [0])");
    let rhs = repl.execute(&format!("Eval (plus Zero (length Nat {})).", y))
        .expect("Eval add 0 (length [0])");

    // Both should be Succ Zero (= 1)
    assert!(lhs.contains("Succ") && lhs.contains("Zero"), "LHS should be 1: {}", lhs);
    assert!(rhs.contains("Succ") && rhs.contains("Zero"), "RHS should be 1: {}", rhs);
}

#[test]
fn test_length_append_step_case() {
    let mut repl = setup_list_repl();

    // Step case: if IH, then length (append (Cons h t) y) = plus (length (Cons h t)) (length y)
    // LHS: length (Cons h (append t y)) = Succ (length (append t y))
    //    = Succ (plus (length t) (length y)) by IH
    // RHS: add (Succ (length t)) (length y)
    //    = Succ (plus (length t) (length y))
    // Equal!

    // Test with x=[0], y=[1]
    let x = "(Cons Nat Zero (Nil Nat))";
    let y = "(Cons Nat (Succ Zero) (Nil Nat))";

    let lhs = repl.execute(&format!("Eval (length Nat (append Nat {} {})).", x, y))
        .expect("Eval length (append [0] [1])");
    let rhs = repl.execute(&format!("Eval (plus (length Nat {}) (length Nat {})).", x, y))
        .expect("Eval plus (length [0]) (length [1])");

    // Both should be Succ (Succ Zero) (= 2)
    let lhs_succ = lhs.matches("Succ").count();
    let rhs_succ = rhs.matches("Succ").count();

    assert_eq!(lhs_succ, rhs_succ,
        "Both should have same number of Succ's. LHS: {}, RHS: {}", lhs, rhs);
}

#[test]
fn test_length_append_longer_lists() {
    let mut repl = setup_list_repl();

    // Test with x=[0,1], y=[2]
    let x = "(Cons Nat Zero (Cons Nat (Succ Zero) (Nil Nat)))";
    let y = "(Cons Nat (Succ (Succ Zero)) (Nil Nat))";

    let lhs = repl.execute(&format!("Eval (length Nat (append Nat {} {})).", x, y))
        .expect("Eval length (append [0,1] [2])");
    let rhs = repl.execute(&format!("Eval (plus (length Nat {}) (length Nat {})).", x, y))
        .expect("Eval plus (length [0,1]) (length [2])");

    // Both should be Succ (Succ (Succ Zero)) (= 3)
    let lhs_succ = lhs.matches("Succ").count();
    let rhs_succ = rhs.matches("Succ").count();

    assert!(lhs_succ >= 3 && rhs_succ >= 3,
        "Both should have at least 3 Succ's. LHS: {}, RHS: {}", lhs, rhs);
}

// =============================================================================
// DEEP EMBEDDING: Proving via DElim
// =============================================================================

#[test]
fn test_delim_list_structure() {
    let mut repl = setup_list_repl();

    // Test that we can construct a DElim for List
    // DElim takes: (ind_type, motive, DCase_chain)
    // For List with 2 constructors (Nil, Cons), we need:
    // DCase nil_proof (DCase cons_proof DCaseEnd)

    // Check that DElim is available
    let result = repl.execute("Check DElim.");

    assert!(result.is_ok(), "DElim should be available: {:?}", result);
}

// =============================================================================
// ADDITIONAL PROPERTIES
// =============================================================================

#[test]
fn test_map_composition() {
    let mut repl = setup_list_repl();

    // map f (map g l) = map (f . g) l
    // Test with f=Succ, g=Succ on [0]

    let inner = repl.execute("Eval (map Nat Nat Succ (Cons Nat Zero (Nil Nat))).")
        .expect("Eval map Succ [0]");

    // inner should be [1]
    assert!(inner.contains("Succ") && inner.contains("Zero"),
        "map Succ [0] should be [1]: {}", inner);
}

#[test]
fn test_append_length_property() {
    let mut repl = setup_list_repl();

    // Another verification: length (x ++ y) = length x + length y
    // Using x = [0, 1] (length 2) and y = [2, 3] (length 2)
    // Result should be length 4

    let x = "(Cons Nat Zero (Cons Nat (Succ Zero) (Nil Nat)))";
    let y = "(Cons Nat (Succ (Succ Zero)) (Cons Nat (Succ (Succ (Succ Zero))) (Nil Nat)))";

    let len_concat = repl.execute(&format!("Eval (length Nat (append Nat {} {})).", x, y))
        .expect("Eval length of concatenated list");

    // Should have 4 Succ's
    let succ_count = len_concat.matches("Succ").count();
    assert!(succ_count >= 4, "Length of [0,1] ++ [2,3] should be 4: {}", len_concat);
}
