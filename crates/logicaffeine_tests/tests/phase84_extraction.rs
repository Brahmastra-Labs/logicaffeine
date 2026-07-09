//! Phase 84: Program Extraction (The Forge)
//!
//! Compiles verified kernel terms to executable Rust code.
//! - Inductive types → enum
//! - Fixpoints → recursive fn
//! - Pattern matching → match

use logicaffeine_compile::extraction::{extract_program, extract_programs};
use logicaffeine_kernel::interface::Repl;

// =============================================================================
// BASIC EXTRACTION TESTS
// =============================================================================

#[test]
fn test_extract_nat_enum() {
    let mut repl = Repl::new();

    // StandardLibrary already has Nat, but we define MyNat for isolation
    repl.execute("Inductive MyNat := MZero : MyNat | MSucc : MyNat -> MyNat.")
        .expect("Define MyNat");

    let rust_code = extract_program(repl.context(), "MyNat").expect("Extract MyNat");

    // Should generate enum
    assert!(
        rust_code.contains("enum MyNat {"),
        "Should have enum declaration"
    );
    assert!(rust_code.contains("MZero,"), "Should have MZero constructor");
    assert!(
        rust_code.contains("MSucc(Box<MyNat>)"),
        "Should have MSucc with Box"
    );
}

#[test]
fn test_extract_simple_definition() {
    let mut repl = Repl::new();

    // Define a simple value
    repl.execute("Definition one : Nat := Succ Zero.")
        .expect("Define one");

    let rust_code = extract_program(repl.context(), "one").expect("Extract one");

    // Should reference Nat enum
    assert!(
        rust_code.contains("enum Nat {"),
        "Should include Nat dependency"
    );
    // Should have the definition
    assert!(rust_code.contains("Nat::Succ"), "Should use Nat::Succ");
    assert!(rust_code.contains("Nat::Zero"), "Should use Nat::Zero");
}

// =============================================================================
// FIXPOINT EXTRACTION TESTS
// =============================================================================

#[test]
fn test_extract_add_function() {
    let mut repl = Repl::new();

    // Define add using fix + nested lambdas
    // Note: motive must be a function (fun _ : Nat => ReturnType)
    let add_def = "Definition add : Nat -> Nat -> Nat := \
        fix rec => fun n : Nat => fun m : Nat => \
        match n return (fun _ : Nat => Nat) with \
        | Zero => m \
        | Succ k => Succ (rec k m).";
    repl.execute(add_def).expect("Define add");

    let rust_code = extract_program(repl.context(), "add").expect("Extract add");

    println!("Generated Rust:\n{}", rust_code);

    // Should have Nat enum
    assert!(rust_code.contains("enum Nat {"), "Should include Nat");

    // Should have add function
    assert!(rust_code.contains("fn add("), "Should have fn add");

    // Should have match
    assert!(rust_code.contains("match"), "Should have match expression");
    assert!(rust_code.contains("Nat::Zero"), "Should match Zero");
    assert!(rust_code.contains("Nat::Succ"), "Should match Succ");
}

#[test]
fn test_extract_double_function() {
    let mut repl = Repl::new();

    // add must be defined first
    let add_def = "Definition add : Nat -> Nat -> Nat := \
        fix rec => fun n : Nat => fun m : Nat => \
        match n return (fun _ : Nat => Nat) with \
        | Zero => m \
        | Succ k => Succ (rec k m).";
    repl.execute(add_def).expect("Define add");

    // double uses add
    let double_def = "Definition double : Nat -> Nat := fun n : Nat => add n n.";
    repl.execute(double_def).expect("Define double");

    let rust_code = extract_program(repl.context(), "double").expect("Extract double");

    // Should have all dependencies
    assert!(rust_code.contains("enum Nat {"), "Should include Nat");
    assert!(rust_code.contains("fn add("), "Should include add");
    assert!(rust_code.contains("fn double("), "Should have double");
}

// =============================================================================
// DEPENDENCY TESTS
// =============================================================================

#[test]
fn test_transitive_dependencies() {
    let mut repl = Repl::new();

    // Build a chain: triple -> double -> add -> Nat
    let add_def = "Definition add : Nat -> Nat -> Nat := \
        fix rec => fun n : Nat => fun m : Nat => \
        match n return (fun _ : Nat => Nat) with \
        | Zero => m \
        | Succ k => Succ (rec k m).";
    let double_def = "Definition double : Nat -> Nat := fun n : Nat => add n n.";
    let triple_def = "Definition triple : Nat -> Nat := fun n : Nat => add n (double n).";

    repl.execute(add_def).expect("Define add");
    repl.execute(double_def).expect("Define double");
    repl.execute(triple_def).expect("Define triple");

    let rust_code = extract_program(repl.context(), "triple").expect("Extract triple");

    // Should have all transitive dependencies
    assert!(rust_code.contains("enum Nat {"), "Should include Nat");
    assert!(rust_code.contains("fn add("), "Should include add");
    assert!(rust_code.contains("fn double("), "Should include double");
    assert!(rust_code.contains("fn triple("), "Should have triple");
}

// =============================================================================
// BOOL EXTRACTION TESTS
// =============================================================================

#[test]
fn test_extract_bool_enum() {
    let mut repl = Repl::new();

    repl.execute("Inductive MyBool := Yes : MyBool | No : MyBool.")
        .expect("Define MyBool");

    let rust_code = extract_program(repl.context(), "MyBool").expect("Extract MyBool");

    assert!(rust_code.contains("enum MyBool {"), "Should have enum");
    assert!(rust_code.contains("Yes,"), "Should have Yes");
    assert!(rust_code.contains("No,"), "Should have No");
    // No Box needed - not recursive
    assert!(!rust_code.contains("Box<MyBool>"), "Should not need Box");
}

#[test]
fn test_extract_is_zero() {
    let mut repl = Repl::new();

    repl.execute("Inductive MyBool := Yes : MyBool | No : MyBool.")
        .expect("Define MyBool");

    let is_zero_def = "Definition is_zero : Nat -> MyBool := \
        fun n : Nat => match n return (fun _ : Nat => MyBool) with \
        | Zero => Yes \
        | Succ k => No.";
    repl.execute(is_zero_def).expect("Define is_zero");

    let rust_code = extract_program(repl.context(), "is_zero").expect("Extract is_zero");

    assert!(rust_code.contains("enum Nat {"), "Should include Nat");
    assert!(rust_code.contains("enum MyBool {"), "Should include MyBool");
    assert!(rust_code.contains("fn is_zero("), "Should have is_zero");
}

// =============================================================================
// ERROR HANDLING TESTS
// =============================================================================

#[test]
fn test_extract_undefined_name() {
    let repl = Repl::new();

    let result = extract_program(repl.context(), "undefined_name");
    assert!(result.is_err(), "Should error on undefined name");
}

#[test]
fn test_extract_inductive_is_extractable() {
    let repl = Repl::new();

    // Nat is an inductive from StandardLibrary, so it IS extractable as an enum
    let result = extract_program(repl.context(), "Nat");
    assert!(result.is_ok(), "Inductives should be extractable");
}

// =============================================================================
// MULTI-ENTRY EXTRACTION TESTS
// =============================================================================

#[test]
fn test_extract_programs_multiple_entries_dedup_shared_deps() {
    let mut repl = Repl::new();

    let add_def = "Definition add : Nat -> Nat -> Nat := \
        fix rec => fun n : Nat => fun m : Nat => \
        match n return (fun _ : Nat => Nat) with \
        | Zero => m \
        | Succ k => Succ (rec k m).";
    let double_def = "Definition double : Nat -> Nat := fun n : Nat => add n n.";
    repl.execute(add_def).expect("Define add");
    repl.execute(double_def).expect("Define double");

    let rust_code =
        extract_programs(repl.context(), &["add", "double"]).expect("Extract add + double");

    // Both functions present.
    assert!(rust_code.contains("fn add("), "Should have fn add");
    assert!(rust_code.contains("fn double("), "Should have fn double");

    // Shared dependencies emitted exactly once (CodeGen dedup across entries).
    assert_eq!(
        rust_code.matches("enum Nat {").count(),
        1,
        "Nat should be emitted exactly once across both entries"
    );
    assert_eq!(
        rust_code.matches("fn add(").count(),
        1,
        "add should be emitted exactly once (double depends on it)"
    );
}

#[test]
fn test_extract_program_delegates_to_extract_programs() {
    let mut repl = Repl::new();
    repl.execute("Inductive MyNat := MZero : MyNat | MSucc : MyNat -> MyNat.")
        .expect("Define MyNat");

    let single = extract_program(repl.context(), "MyNat").expect("single");
    let multi = extract_programs(repl.context(), &["MyNat"]).expect("multi");

    assert_eq!(single, multi, "extract_program must equal extract_programs of one entry");
}

#[test]
fn test_extract_programs_unknown_entry_errors() {
    let repl = Repl::new();
    let result = extract_programs(repl.context(), &["undefined_name"]);
    assert!(result.is_err(), "Should error on undefined entry");
}

// =============================================================================
// UI BRIDGE: MATH / LOGIC → RUST
// =============================================================================

#[test]
fn test_extract_math_rust_emits_user_defs_pulls_deps_once() {
    let mut repl = Repl::new();
    let add_def = "Definition add : Nat -> Nat -> Nat := \
        fix rec => fun n : Nat => fun m : Nat => \
        match n return (fun _ : Nat => Nat) with \
        | Zero => m \
        | Succ k => Succ (rec k m).";
    let double_def = "Definition double : Nat -> Nat := fun n : Nat => add n n.";
    repl.execute(add_def).expect("Define add");
    repl.execute(double_def).expect("Define double");

    let rust = logicaffeine_compile::extract_math_rust(repl.context())
        .expect("extract_math_rust ok");

    assert!(rust.contains("fn add("), "user def add present");
    assert!(rust.contains("fn double("), "user def double present");
    // Nat is a StandardLibrary inductive pulled in transitively — exactly once.
    assert_eq!(
        rust.matches("enum Nat {").count(),
        1,
        "transitively-needed Nat emitted once"
    );
}

#[test]
fn test_extract_math_rust_empty_when_nothing_defined() {
    // A fresh REPL has only StandardLibrary — extracting must NOT dump it.
    let repl = Repl::new();
    let rust = logicaffeine_compile::extract_math_rust(repl.context())
        .expect("extract_math_rust ok");
    assert!(
        rust.contains("nothing defined yet"),
        "no user defs => honest note, not a stdlib dump; got: {rust}"
    );
}

#[test]
fn test_extract_logic_rust_grid_puzzle_bails_fast() {
    // A finite-domain grid premise: Compile must bail with the puzzle note
    // *without* running the solver (which would freeze the UI on the main thread).
    let input = "## Theorem: MiniGrid\n\
        Given: Every trip is in Florida or in Maine.\n\
        Prove: Every trip is in Florida or in Maine.\n\
        Proof: Auto.";
    let rust = logicaffeine_compile::extract_logic_rust(input).expect("extract_logic_rust ok");
    assert!(
        rust.contains("finite-domain puzzle"),
        "grid input must bail with the puzzle note (no solve); got: {rust}"
    );
}

#[test]
fn test_extract_logic_rust_emits_usable_world_library() {
    // A relational sentence yields a reusable rules/invariants library: a public
    // `World` with an ergonomic builder, a public `holds`, doc comments echoing
    // the English, and a real event relation (not a `true` stub).
    let rust = logicaffeine_compile::extract_logic_rust("Every dog chased some cat.").expect("ok");
    assert!(rust.contains("pub struct World"), "usable World type:\n{rust}");
    assert!(rust.contains("pub fn fact("), "ergonomic builder:\n{rust}");
    assert!(rust.contains("pub fn holds("), "public rule fn:\n{rust}");
    assert!(rust.contains("// English:"), "doc comment echoes English:\n{rust}");
    assert!(rust.contains(".any("), "event existential emitted:\n{rust}");
    assert!(!rust.contains("unsupported: event"), "event must not be dropped:\n{rust}");
}

#[test]
fn test_extract_logic_rust_plain_sentence_emits_model_checker() {
    // A plain sentence now compiles to a runnable FOL model-checker (a `Model` +
    // `holds` + demo `main`), not an honest note.
    let rust = logicaffeine_compile::extract_logic_rust("Socrates is a man.")
        .expect("extract_logic_rust ok");
    assert!(
        rust.contains("fn holds(") && rust.contains("fn main("),
        "plain sentence => runnable model-checker; got: {rust}"
    );
}

// =============================================================================
// OPAQUE PRIMITIVE TYPES (Int / Float / Text / Bool ...)
//
// These are registered in the kernel StandardLibrary as inductives with NO
// constructors. Extraction must NOT treat them as missing — they have no enum
// form and map to Rust primitives at use sites.
// =============================================================================

#[test]
fn test_extract_definition_over_int_does_not_choke() {
    let mut repl = Repl::new();
    repl.execute("Definition ident_int : Int -> Int := fun n : Int => n.")
        .expect("define ident_int");

    let rust = extract_program(repl.context(), "ident_int")
        .expect("extraction must not error on opaque primitive Int");

    assert!(rust.contains("fn ident_int("), "function emitted");
    assert!(rust.contains("i64"), "Int maps to i64 at the type level");
    assert!(!rust.contains("enum Int"), "opaque Int must not be emitted as an enum");
}

#[test]
fn test_extract_programs_over_float_and_text_primitives() {
    let mut repl = Repl::new();
    repl.execute("Definition id_f : Float -> Float := fun x : Float => x.")
        .expect("define id_f");
    repl.execute("Definition id_t : Text -> Text := fun s : Text => s.")
        .expect("define id_t");

    let rust = extract_programs(repl.context(), &["id_f", "id_t"])
        .expect("extraction must not choke on Float/Text");

    assert!(rust.contains("f64"), "Float maps to f64");
    assert!(rust.contains("String"), "Text maps to String");
    assert!(!rust.contains("enum Float"));
    assert!(!rust.contains("enum Text"));
}

// =============================================================================
// PARAMETRIC / POLYMORPHIC INDUCTIVES
//
// `Inductive MyList (A : Type) := ...` must extract to a GENERIC Rust enum with
// the type parameters erased from data positions — not treated as value fields.
// =============================================================================

#[test]
fn test_extract_polymorphic_list_is_generic_enum() {
    let mut repl = Repl::new();
    repl.execute(
        "Inductive MyList (A : Type) := \
         MyNil : MyList A \
       | MyCons : A -> MyList A -> MyList A.",
    )
    .expect("define MyList");

    let rust = extract_program(repl.context(), "MyList").expect("extract MyList");

    assert!(rust.contains("enum MyList<A>"), "generic enum header; got:\n{rust}");
    assert!(rust.contains("MyNil,"), "MyNil must be field-less; got:\n{rust}");
    assert!(
        rust.contains("MyCons(A, Box<MyList<A>>)"),
        "MyCons must hold (A, Box<MyList<A>>); got:\n{rust}"
    );
    assert!(!rust.contains("MyNil(("), "no unit-field garbage; got:\n{rust}");
    assert!(!rust.contains("MyList<()>"), "no erased-to-unit generic; got:\n{rust}");
}

#[test]
fn test_extract_polymorphic_pair_two_params() {
    let mut repl = Repl::new();
    repl.execute("Inductive MyPair (A : Type) (B : Type) := MkPair : A -> B -> MyPair A B.")
        .expect("define MyPair");

    let rust = extract_program(repl.context(), "MyPair").expect("extract MyPair");

    assert!(rust.contains("enum MyPair<A, B>"), "two type params; got:\n{rust}");
    assert!(rust.contains("MkPair(A, B)"), "fields A, B; got:\n{rust}");
}

#[test]
fn test_extract_math_rust_over_int_definition() {
    let mut repl = Repl::new();
    repl.execute("Definition ident_int : Int -> Int := fun n : Int => n.")
        .expect("define ident_int");

    let rust = logicaffeine_compile::extract_math_rust(repl.context())
        .expect("extract_math_rust ok");

    assert!(rust.contains("fn ident_int("), "user def present");
    assert!(
        !rust.contains("Not found"),
        "must not surface a NotFound error for opaque Int; got: {rust}"
    );
}
