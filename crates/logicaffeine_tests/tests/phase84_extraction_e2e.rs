//! Phase 84 E2E: Compile and execute extracted code

#[cfg(not(target_arch = "wasm32"))]
mod extraction_common;

#[cfg(not(target_arch = "wasm32"))]
use extraction_common::{assert_extracted_output, run_extracted};

use logicaffeine_compile::extraction::{extract_program, extract_programs};
use logicaffeine_kernel::interface::Repl;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_e2e_nat_values() {
    let mut repl = Repl::new();

    repl.execute("Inductive MyNat := MZero : MyNat | MSucc : MyNat -> MyNat.")
        .expect("Define MyNat");

    let rust_code = extract_program(repl.context(), "MyNat").expect("Extract");

    let main_code = r#"
fn main() {
    let zero = MyNat::MZero;
    let one = MyNat::MSucc(Box::new(MyNat::MZero));
    let two = MyNat::MSucc(Box::new(MyNat::MSucc(Box::new(MyNat::MZero))));
    println!("Created 0, 1, 2");
}
"#;

    assert_extracted_output(&rust_code, main_code, "Created 0, 1, 2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_e2e_add_execution() {
    let mut repl = Repl::new();

    repl.execute("Inductive MyNat := MZero : MyNat | MSucc : MyNat -> MyNat.")
        .expect("Define MyNat");

    let add_def = "Definition my_add : MyNat -> MyNat -> MyNat := \
        fix rec => fun n : MyNat => fun m : MyNat => \
        match n return (fun _ : MyNat => MyNat) with \
        | MZero => m \
        | MSucc k => MSucc (rec k m).";
    repl.execute(add_def).expect("Define my_add");

    let rust_code = extract_program(repl.context(), "my_add").expect("Extract");

    let main_code = r#"
fn nat_to_int(n: &MyNat) -> u32 {
    match n {
        MyNat::MZero => 0,
        MyNat::MSucc(k) => 1 + nat_to_int(k),
    }
}

fn int_to_nat(n: u32) -> MyNat {
    if n == 0 { MyNat::MZero }
    else { MyNat::MSucc(Box::new(int_to_nat(n - 1))) }
}

fn main() {
    let two = int_to_nat(2);
    let three = int_to_nat(3);
    let result = my_add(two, three);
    println!("{}", nat_to_int(&result));
}
"#;

    assert_extracted_output(&rust_code, main_code, "5");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_e2e_is_zero_execution() {
    let mut repl = Repl::new();

    repl.execute("Inductive MyNat := MZero : MyNat | MSucc : MyNat -> MyNat.")
        .expect("Define MyNat");
    repl.execute("Inductive MyBool := Yes : MyBool | No : MyBool.")
        .expect("Define MyBool");

    let is_zero_def = "Definition is_zero : MyNat -> MyBool := \
        fun n : MyNat => match n return (fun _ : MyNat => MyBool) with \
        | MZero => Yes \
        | MSucc k => No.";
    repl.execute(is_zero_def).expect("Define is_zero");

    let rust_code = extract_program(repl.context(), "is_zero").expect("Extract");

    let main_code = r#"
fn main() {
    let zero = MyNat::MZero;
    let one = MyNat::MSucc(Box::new(MyNat::MZero));

    match is_zero(zero) {
        MyBool::Yes => print!("zero:yes "),
        MyBool::No => print!("zero:no "),
    }
    match is_zero(one) {
        MyBool::Yes => print!("one:yes"),
        MyBool::No => print!("one:no"),
    }
    println!();
}
"#;

    assert_extracted_output(&rust_code, main_code, "zero:yes one:no");
}

// =============================================================================
// E2E for the Studio "🦀 Compile" path: extract_math_rust → rustc → run.
//
// These exercise the EXACT function the Math Compile button calls, prove the
// emitted Rust actually compiles and runs, and pin the opaque-primitive (`Int`)
// fix end-to-end (it previously failed with `Not found: Int`).
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_e2e_extract_math_rust_button_path_add() {
    let mut repl = Repl::new();

    repl.execute("Inductive MyNat := MZero : MyNat | MSucc : MyNat -> MyNat.")
        .expect("Define MyNat");
    let add_def = "Definition my_add : MyNat -> MyNat -> MyNat := \
        fix rec => fun n : MyNat => fun m : MyNat => \
        match n return (fun _ : MyNat => MyNat) with \
        | MZero => m \
        | MSucc k => MSucc (rec k m).";
    repl.execute(add_def).expect("Define my_add");

    // The Studio Compile button calls exactly this — and the emitted module is a
    // self-contained "compiled mathematical object": its own `main` runs my_add on
    // kernel-built samples and `assert_eq!`s the result against the kernel.
    let rust_code =
        logicaffeine_compile::extract_math_rust(repl.context()).expect("extract_math_rust");
    assert!(
        rust_code.contains("fn my_add(") && rust_code.contains("fn main("),
        "self-contained compiled object:\n{rust_code}"
    );
    let result = run_extracted(&rust_code, "");
    assert!(
        result.success,
        "self-verifying object must compile and run (kernel-checked asserts):\n--- rust ---\n{}\n--- stderr ---\n{}",
        result.rust_code, result.stderr
    );
    assert!(
        result.stdout.contains("my_add(..) ="),
        "should demo my_add; got: {}",
        result.stdout
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_e2e_extract_math_rust_int_primitive_runs() {
    // Regression: a definition over the opaque primitive `Int` used to fail
    // extraction with `Not found: Int`. It must now extract to runnable Rust.
    let mut repl = Repl::new();
    repl.execute("Definition ident_int : Int -> Int := fun n : Int => n.")
        .expect("Define ident_int");

    let rust_code =
        logicaffeine_compile::extract_math_rust(repl.context()).expect("extract_math_rust");
    assert!(
        rust_code.contains("fn ident_int(") && rust_code.contains("fn main("),
        "self-contained compiled object:\n{rust_code}"
    );
    let result = run_extracted(&rust_code, "");
    assert!(
        result.success,
        "self-verifying object must compile and run:\n--- rust ---\n{}\n--- stderr ---\n{}",
        result.rust_code, result.stderr
    );
    assert!(
        result.stdout.contains("ident_int(..) ="),
        "should demo ident_int; got: {}",
        result.stdout
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_e2e_polymorphic_list_compiles_and_runs() {
    // The polymorphic List example from the studio: must extract to VALID,
    // runnable generic Rust (was producing `MyList<()>`, `MyNil(())`, and types
    // passed as values).
    let mut repl = Repl::new();
    repl.execute(
        "Inductive MyList (A : Type) := \
         MyNil : MyList A \
       | MyCons : A -> MyList A -> MyList A.",
    )
    .expect("define MyList");
    repl.execute(
        "Definition nat_list : MyList Nat := \
         MyCons Nat Zero (MyCons Nat (Succ Zero) (MyNil Nat)).",
    )
    .expect("define nat_list");

    // Module only (no demo main) so this test can append its own len() + main.
    let rust_code = extract_programs(repl.context(), &["nat_list"]).expect("extract nat_list");

    let main_code = r#"
fn len<A>(l: &MyList<A>) -> u32 {
    match l {
        MyList::MyNil => 0,
        MyList::MyCons(_, t) => 1 + len(t),
    }
}
fn main() {
    println!("{}", len(&nat_list()));
}
"#;

    assert_extracted_output(&rust_code, main_code, "2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_e2e_logic_model_checker_theorem_compiles_and_runs() {
    // A logical theorem (Socrates syllogism) now compiles to a runnable FOL
    // model-checker — `holds(&Model) -> bool` over a demo world — and that Rust
    // actually compiles and runs.
    let input = "## Theorem: Socrates_Mortality\n\
        Given: All men are mortal.\n\
        Given: Socrates is a man.\n\
        Prove: Socrates is mortal.\n\
        Proof: Auto.";
    let rust = logicaffeine_compile::extract_logic_rust(input).expect("extract_logic_rust");
    assert!(
        rust.contains("fn holds(") && rust.contains("fn main("),
        "theorem should compile to a model-checker:\n{rust}"
    );
    // The emitted program is complete (its own main) — append nothing.
    let result = run_extracted(&rust, "");
    assert!(
        result.success,
        "model-checker must compile and run.\n--- rust ---\n{}\n--- stderr ---\n{}",
        result.rust_code, result.stderr
    );
    assert!(
        result.stdout.contains("holds ="),
        "should print the result; got: {}",
        result.stdout
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_e2e_logic_relational_quantifiers_compile_and_run() {
    // Relational quantifier-scope sentences parse as Neo-Davidsonian events; the
    // model-checker must emit a real relation (∃ event with roles), not `true`,
    // and the result must compile and run.
    for sentence in [
        "Every dog chased some cat.",
        "Every student read a book.",
        "A professor supervises every student.",
        "No student failed every exam.",
    ] {
        let rust = logicaffeine_compile::extract_logic_rust(sentence)
            .unwrap_or_else(|e| panic!("{sentence}: {e}"));
        assert!(rust.contains("fn holds("), "{sentence}: model-checker:\n{rust}");
        assert!(
            !rust.contains("unsupported: event"),
            "{sentence}: events must be handled, not dropped to true:\n{rust}"
        );
        assert!(
            rust.contains("\"Agent\""),
            "{sentence}: a Davidsonian role should appear:\n{rust}"
        );
        let result = run_extracted(&rust, "");
        assert!(
            result.success,
            "{sentence}: must compile and run.\n--- rust ---\n{}\n--- stderr ---\n{}",
            result.rust_code, result.stderr
        );
        assert!(result.stdout.contains("holds ="), "{sentence}: got {}", result.stdout);
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_e2e_logic_temporal_monitor_compiles_and_runs() {
    // Temporal sentences compile to a finite-trace monitor (holds over &[World] +
    // an incremental Monitor) — WASM-safe, no Z3, no std::time.
    for sentence in ["Always, every dog runs.", "Eventually, John runs."] {
        let rust = logicaffeine_compile::extract_logic_rust(sentence)
            .unwrap_or_else(|e| panic!("{sentence}: {e}"));
        assert!(
            rust.contains("fn holds(trace") && rust.contains("struct Monitor"),
            "{sentence}: should emit a trace monitor + Monitor:\n{rust}"
        );
        assert!(
            !rust.contains("unsupported: temporal"),
            "{sentence}: temporal operators must be handled:\n{rust}"
        );
        let result = run_extracted(&rust, "");
        assert!(
            result.success,
            "{sentence}: must compile and run.\n--- rust ---\n{}\n--- stderr ---\n{}",
            result.rust_code, result.stderr
        );
        assert!(result.stdout.contains("holds ="), "{sentence}: got {}", result.stdout);
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_e2e_logic_model_checker_quantified_sentence_runs() {
    // Universal quantifier + a predicate (and an event the checker degrades to
    // `true`): exercises `m.domain.iter().all(..)` and must still compile/run.
    let rust = logicaffeine_compile::extract_logic_rust("Every cat sleeps.")
        .expect("extract_logic_rust");
    assert!(rust.contains("fn holds("), "quantified sentence => model-checker:\n{rust}");
    let result = run_extracted(&rust, "");
    assert!(
        result.success,
        "model-checker must compile and run.\n--- rust ---\n{}\n--- stderr ---\n{}",
        result.rust_code, result.stderr
    );
    assert!(result.stdout.contains("holds ="), "should print the result; got: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_e2e_logic_model_checker_plain_sentence_runs() {
    let rust =
        logicaffeine_compile::extract_logic_rust("Socrates is a man.").expect("extract_logic_rust");
    assert!(rust.contains("fn holds("), "plain sentence => model-checker:\n{rust}");
    let result = run_extracted(&rust, "");
    assert!(
        result.success,
        "model-checker must compile and run.\n--- rust ---\n{}\n--- stderr ---\n{}",
        result.rust_code, result.stderr
    );
    assert!(result.stdout.contains("holds ="), "should print the result; got: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_e2e_math_theorem_becomes_runnable_property_check() {
    // A proven theorem (`∀n. add Zero n = n`, provable by refl since it holds by
    // computation) compiles to a runnable property `fn check_..(n) -> bool` over
    // the EXTRACTED `add`, and the self-verifying main runs and asserts it.
    let mut repl = Repl::new();
    repl.execute("Inductive Nat2 := Z2 : Nat2 | S2 : Nat2 -> Nat2.")
        .expect("Nat2");
    repl.execute(
        "Definition add2 : Nat2 -> Nat2 -> Nat2 := \
         fix rec => fun n : Nat2 => fun m : Nat2 => \
         match n return (fun _ : Nat2 => Nat2) with | Z2 => m | S2 k => S2 (rec k m).",
    )
    .expect("add2");
    repl.execute(
        "Definition add2_zero_l : (forall n : Nat2, Eq Nat2 (add2 Z2 n) n) := \
         fun n : Nat2 => refl Nat2 n.",
    )
    .expect("add2_zero_l");

    let rust = logicaffeine_compile::extract_math_rust(repl.context()).expect("extract_math_rust");

    // The theorem became a runnable property check over the extracted function.
    assert!(
        rust.contains("fn check_add2_zero_l(") && rust.contains("add2("),
        "theorem should become a property check over add2:\n{rust}"
    );
    let result = run_extracted(&rust, "");
    assert!(
        result.success,
        "compiled mathematical object must compile and run (property check asserted):\n--- rust ---\n{}\n--- stderr ---\n{}",
        result.rust_code, result.stderr
    );
    assert!(
        result.stdout.contains("add2_zero_l holds"),
        "should report the proven property running; got: {}",
        result.stdout
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_e2e_logic_gates_circuit_compiles_and_runs() {
    // The runnable kind of "theorem": a circuit encoded as computational kernel
    // definitions. Extract the gates to Rust and check the XOR truth table — this
    // is the "encode it, extract it, run it (in WASM / as a circuit)" path.
    let mut repl = Repl::new();
    repl.execute("Inductive MyBit := Lo : MyBit | Hi : MyBit.")
        .expect("MyBit");
    repl.execute(
        "Definition not1 : MyBit -> MyBit := fun a : MyBit => \
         match a return (fun _ : MyBit => MyBit) with | Lo => Hi | Hi => Lo.",
    )
    .expect("not1");
    repl.execute(
        "Definition and2 : MyBit -> MyBit -> MyBit := fun a : MyBit => fun b : MyBit => \
         match a return (fun _ : MyBit => MyBit) with | Lo => Lo | Hi => b.",
    )
    .expect("and2");
    repl.execute(
        "Definition or2 : MyBit -> MyBit -> MyBit := fun a : MyBit => fun b : MyBit => \
         match a return (fun _ : MyBit => MyBit) with | Lo => b | Hi => Hi.",
    )
    .expect("or2");
    repl.execute(
        "Definition xor2 : MyBit -> MyBit -> MyBit := fun a : MyBit => fun b : MyBit => \
         or2 (and2 a (not1 b)) (and2 (not1 a) b).",
    )
    .expect("xor2");

    // Module only (no demo main) so this test can append its own truth-table main.
    let rust_code = extract_programs(repl.context(), &["xor2"]).expect("extract xor2");

    let main_code = r#"
fn bit(b: &MyBit) -> u8 { match b { MyBit::Lo => 0, MyBit::Hi => 1 } }
fn main() {
    // XOR truth table for (Lo,Lo) (Lo,Hi) (Hi,Lo) (Hi,Hi) = 0 1 1 0
    print!(
        "{}{}{}{}",
        bit(&xor2(MyBit::Lo, MyBit::Lo)),
        bit(&xor2(MyBit::Lo, MyBit::Hi)),
        bit(&xor2(MyBit::Hi, MyBit::Lo)),
        bit(&xor2(MyBit::Hi, MyBit::Hi)),
    );
}
"#;

    assert_extracted_output(&rust_code, main_code, "0110");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_e2e_polymorphic_pair_compiles_and_runs() {
    let mut repl = Repl::new();
    repl.execute("Inductive MyBool := Yes : MyBool | No : MyBool.")
        .expect("define MyBool");
    repl.execute("Inductive MyPair (A : Type) (B : Type) := MkPair : A -> B -> MyPair A B.")
        .expect("define MyPair");
    repl.execute("Definition nb : MyPair Nat MyBool := MkPair Nat MyBool Zero Yes.")
        .expect("define nb");

    // Module only (no demo main) so this test can append its own match main.
    let rust_code = extract_programs(repl.context(), &["nb"]).expect("extract nb");

    let main_code = r#"
fn main() {
    match nb() {
        MyPair::MkPair(_, b) => match b {
            MyBool::Yes => println!("yes"),
            MyBool::No => println!("no"),
        },
    }
}
"#;

    assert_extracted_output(&rust_code, main_code, "yes");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_e2e_extract_programs_multi_entry_runs() {
    let mut repl = Repl::new();

    repl.execute("Inductive MyNat := MZero : MyNat | MSucc : MyNat -> MyNat.")
        .expect("Define MyNat");
    let add_def = "Definition my_add : MyNat -> MyNat -> MyNat := \
        fix rec => fun n : MyNat => fun m : MyNat => \
        match n return (fun _ : MyNat => MyNat) with \
        | MZero => m \
        | MSucc k => MSucc (rec k m).";
    repl.execute(add_def).expect("Define my_add");
    repl.execute("Definition my_double : MyNat -> MyNat := fun n : MyNat => my_add n n.")
        .expect("Define my_double");

    // Multiple entries sharing MyNat + my_add — emitted once each.
    let rust_code =
        extract_programs(repl.context(), &["my_add", "my_double"]).expect("extract multi");

    let main_code = r#"
fn nat_to_int(n: &MyNat) -> u32 {
    match n {
        MyNat::MZero => 0,
        MyNat::MSucc(k) => 1 + nat_to_int(k),
    }
}
fn int_to_nat(n: u32) -> MyNat {
    if n == 0 { MyNat::MZero } else { MyNat::MSucc(Box::new(int_to_nat(n - 1))) }
}
fn main() {
    println!("{}", nat_to_int(&my_double(int_to_nat(3))));
}
"#;

    assert_extracted_output(&rust_code, main_code, "6");
}
