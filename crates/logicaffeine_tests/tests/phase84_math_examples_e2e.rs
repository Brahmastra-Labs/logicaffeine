//! E2E: every COMPUTATIONAL Studio Math example must extract to Rust that
//! actually compiles. Drives the exact Math "🦀 Compile" pipeline
//! (`extract_math_rust_from_source`) and `rustc`-compiles the result.
//!
//! These example strings mirror `apps/logicaffeine_web/src/ui/examples.rs`
//! (the `MATH_*` consts). Keep them in sync; if an example changes there, change
//! it here so the "does it compile?" guarantee stays honest.

#[cfg(not(target_arch = "wasm32"))]
mod extraction_common;
#[cfg(not(target_arch = "wasm32"))]
use extraction_common::run_extracted;

use logicaffeine_compile::extract_math_rust_from_source;

/// Extract a Math example and assert the generated Rust both *contains real
/// content* and *compiles*.
#[cfg(not(target_arch = "wasm32"))]
fn assert_example_compiles(name: &str, source: &str, must_contain: &[&str]) {
    let rust = extract_math_rust_from_source(source);
    assert!(
        !rust.contains("extraction error"),
        "{name}: extraction errored:\n{rust}"
    );
    assert!(
        !rust.contains("nothing defined"),
        "{name}: nothing was extracted:\n{rust}"
    );
    for needle in must_contain {
        assert!(
            rust.contains(needle),
            "{name}: expected `{needle}` in extracted Rust:\n{rust}"
        );
    }
    // The extracted module is self-contained (its own self-verifying `main`),
    // so append nothing — running it executes the kernel-checked asserts.
    let result = run_extracted(&rust, "");
    assert!(
        result.success,
        "{name}: extracted Rust did NOT compile.\n--- rust ---\n{}\n--- stderr ---\n{}",
        result.rust_code, result.stderr
    );
}

// ---- example sources (mirror examples.rs) -----------------------------------

const MATH_NAT: &str = r#"-- Natural Numbers
Inductive Nat := Zero : Nat | Succ : Nat -> Nat.
Definition one : Nat := Succ Zero.
Definition two : Nat := Succ one.
Definition three : Nat := Succ two.
Check Zero.
Eval three.
"#;

const MATH_BOOL: &str = r#"Inductive MyBool := Yes : MyBool | No : MyBool.
Check Yes.
Eval No.
Definition id_bool : MyBool -> MyBool := fun b : MyBool => b.
Eval id_bool Yes.
"#;

const MATH_FUNCTIONS: &str = r#"-- Simple Functions
Definition id : Nat -> Nat := fun x : Nat => x.
Definition const_zero : Nat -> Nat := fun x : Nat => Zero.
Definition double_succ : Nat -> Nat := fun x : Nat => Succ (Succ x).
Definition one : Nat := Succ Zero.
Definition two : Nat := Succ one.
Eval double_succ one.
"#;

const MATH_PROP_LOGIC: &str = r#"-- Propositional Logic Types
Inductive MyProp :=
    PTrue : MyProp
  | PFalse : MyProp
  | PAnd : MyProp -> MyProp -> MyProp
  | POr : MyProp -> MyProp -> MyProp
  | PNot : MyProp -> MyProp.
Definition p3 : MyProp := PAnd PTrue PTrue.
Definition p5 : MyProp := PNot PFalse.
Eval p3.
"#;

const MATH_LIST_OPS: &str = r#"-- List Operations
Inductive MyList (A : Type) :=
    MyNil : MyList A
  | MyCons : A -> MyList A -> MyList A.
Definition nat_list : MyList Nat := MyCons Nat Zero (MyCons Nat (Succ Zero) (MyNil Nat)).
Eval nat_list.
"#;

const MATH_PAIRS: &str = r#"-- Pairs and Products
Inductive MyBool := Yes : MyBool | No : MyBool.
Inductive MyPair (A : Type) (B : Type) :=
    MkPair : A -> B -> MyPair A B.
Definition nat_bool_pair : MyPair Nat MyBool := MkPair Nat MyBool Zero Yes.
Definition nat_nat_pair : MyPair Nat Nat := MkPair Nat Nat Zero (Succ Zero).
Eval nat_bool_pair.
"#;

const MATH_CIRCUIT: &str = r#"-- Logic Gates as a Circuit
Inductive MyBit := Lo : MyBit | Hi : MyBit.
Definition not1 : MyBit -> MyBit := fun a : MyBit =>
  match a return (fun _ : MyBit => MyBit) with | Lo => Hi | Hi => Lo.
Definition and2 : MyBit -> MyBit -> MyBit := fun a : MyBit => fun b : MyBit =>
  match a return (fun _ : MyBit => MyBit) with | Lo => Lo | Hi => b.
Definition or2 : MyBit -> MyBit -> MyBit := fun a : MyBit => fun b : MyBit =>
  match a return (fun _ : MyBit => MyBit) with | Lo => b | Hi => Hi.
Definition xor2 : MyBit -> MyBit -> MyBit := fun a : MyBit => fun b : MyBit =>
  or2 (and2 a (not1 b)) (and2 (not1 a) b).
Eval xor2 Hi Lo.
"#;

// ---- per-example tests -------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn math_example_nat_compiles() {
    assert_example_compiles("NAT", MATH_NAT, &["enum Nat", "fn one(", "fn three("]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn math_example_bool_compiles() {
    assert_example_compiles("BOOL", MATH_BOOL, &["enum MyBool", "fn id_bool("]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn math_example_functions_compiles() {
    assert_example_compiles(
        "FUNCTIONS",
        MATH_FUNCTIONS,
        &["enum Nat", "fn id(", "fn double_succ("],
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn math_example_prop_logic_compiles() {
    assert_example_compiles("PROP_LOGIC", MATH_PROP_LOGIC, &["enum MyProp", "fn p3("]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn math_example_list_ops_compiles() {
    assert_example_compiles("LIST_OPS", MATH_LIST_OPS, &["enum MyList<A>", "fn nat_list("]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn math_example_pairs_compiles() {
    assert_example_compiles(
        "PAIRS",
        MATH_PAIRS,
        &["enum MyPair<A, B>", "fn nat_nat_pair("],
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn math_example_circuit_compiles() {
    assert_example_compiles("CIRCUIT", MATH_CIRCUIT, &["enum MyBit", "fn xor2("]);
}
