//! Phase 84 E2E: Compile and execute extracted code

#[cfg(not(target_arch = "wasm32"))]
mod extraction_common;

#[cfg(not(target_arch = "wasm32"))]
use extraction_common::assert_extracted_output;

use logicaffeine_compile::extraction::extract_program;
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
