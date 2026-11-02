//! A context that declares an exact type gives `sqrt`, `exp`, `ln`, `pi`, `e` and `omega` their exact
//! meaning: in `Let x: Algebraic be sqrt(2).` the root is `√2`, not a float near it; `pi` and `e`
//! are the constants themselves; in an `Ordinal` context `omega` is the first infinite ordinal. Code
//! that declares no exact type keeps the meaning it had (`sqrt(2)` is a `Real`), and a function the
//! program defines, or a name it binds, is never replaced. Every program runs alike on the
//! tree-walker, the VM, the optimized VM and the native tier.

mod common;
use common::run_all_tiers;

fn shows(source: &str, printed: &str) {
    let out = run_all_tiers(source);
    assert_eq!(out.error, None, "{source}");
    assert_eq!(out.output.trim(), printed, "{source}");
}

fn stops_with(source: &str, printed: &str, why: &str) {
    let out = run_all_tiers(source);
    assert_eq!(out.output.trim(), printed, "printed before the stop:\n{source}");
    assert_eq!(out.error.as_deref(), Some(why), "{source}");
}

fn main(body: &str) -> String {
    let indented: String = body.lines().map(|l| format!("    {l}\n")).collect();
    format!("## Main\n{indented}")
}

#[test]
fn a_square_root_in_an_exact_context_is_exact() {
    shows(&main("Let x: Algebraic be sqrt(2).\nShow x."), "√2");
    shows(&main("Let x: Algebraic be sqrt(2) * sqrt(2).\nShow x."), "2");
    shows(&main("Let x: ExactReal be sqrt(16).\nShow x."), "4");
    shows(&main("Let x: Algebraic be sqrt(1 / 2).\nShow x."), "√2/2");
    shows(&main("Let x: Algebraic be sqrt(3 / 4).\nShow x."), "√3/2");
    shows(&main("Let x: Algebraic be sqrt(2.25).\nShow x."), "3/2");
}

#[test]
fn a_square_root_outside_an_exact_context_is_a_real() {
    shows(&main("Show sqrt(2.0)."), "1.4142135623730951");
    shows(&main("Let y be sqrt(2.0).\nShow y * y is equal to 2.0."), "false");
}

#[test]
fn the_exponential_and_the_logarithm_are_exact_in_an_exact_context() {
    shows(&main("Let x: ExactReal be exp(1).\nShow x."), "e");
    shows(&main("Let x: ExactReal be exp(0).\nShow x."), "1");
    shows(&main("Let x: ExactReal be ln(exp(2)).\nShow x."), "2");
    shows(&main("Let x: ExactReal be ln(1).\nShow x."), "0");
    shows(&main("Let x: ExactReal be exp(1) * exp(-1).\nShow x."), "1");
}

#[test]
fn pi_and_e_are_the_constants_in_an_exact_context() {
    shows(&main("Let x: ExactReal be pi.\nShow x."), "π");
    shows(&main("Let x: ExactReal be π.\nShow x."), "π");
    shows(&main("Let x: ExactReal be e.\nShow x."), "e");
    shows(&main("Let x: ExactReal be pi * pi + 1.\nShow x."), "π^2 + 1");
    shows(&main("Let x: ExactReal be ln(e).\nShow x."), "1");
    shows(&main("Let x: ExactReal be pi.\nShow x is greater than 3.\nShow x is less than 22 / 7."), "true\ntrue");
}

#[test]
fn a_bound_name_or_a_function_of_the_program_is_never_replaced() {
    shows(&main("Let e be 5.\nLet x: Rational be e / 2.\nShow x."), "5/2");
    shows(&main("Let pi be 3.\nLet x: Rational be pi / 2.\nShow x."), "3/2");
    shows(
        "## To exp (x: Rational) -> Rational:\n    Return x + 1.\n\n## Main\nLet y: Rational be exp(2).\nShow y.\n",
        "3",
    );
    shows(
        "## To sqrt (x: Rational) -> Rational:\n    Return x / 2.\n\n## Main\nLet y: Rational be sqrt(3).\nShow y.\n",
        "3/2",
    );
}

#[test]
fn a_parameter_declared_exact_makes_its_argument_an_exact_context() {
    shows(
        "## To half (x: ExactReal) -> ExactReal:\n    Return x / 2.\n\n## Main\nShow half(sqrt(2)).\nShow half(pi).\n",
        "√2/2\n(1/2)·π",
    );
    shows(
        "## To tenth (x: Rational) -> Rational:\n    Return x / 10.\n\n## Main\nShow tenth(1 / 3).\n",
        "1/30",
    );
}

#[test]
fn a_comparison_with_an_exact_operand_is_an_exact_context() {
    shows(
        &main("Let r: Rational be 1 / 3.\nIf r is less than 1 / 2:\n    Show \"less\".\nOtherwise:\n    Show \"not less\"."),
        "less",
    );
    shows(&main("Let x: ExactReal be sqrt(2).\nShow x is less than 3 / 2.\nShow x is greater than 7 / 5."), "true\ntrue");
}

#[test]
fn an_exact_operand_makes_the_spine_it_stands_in_exact() {
    shows(&main("Let r: Rational be 1 / 3.\nLet s be r + 1 / 6.\nShow s."), "1/2");
    shows(&main("Let x: Algebraic be sqrt(2).\nLet y be x / 2.\nShow y."), "√2/2");
    shows(&main("Let x: Algebraic be sqrt(2).\nLet y be sqrt(x * x).\nShow y."), "√2");
}

#[test]
fn the_exact_constructors_can_be_called_by_name() {
    shows(&main("Show exactSqrt(2)."), "√2");
    shows(&main("Show exactPi()."), "π");
    shows(&main("Show exactE()."), "e");
    shows(&main("Show exactLn(exactE())."), "1");
    shows(&main("Show exactOmega()."), "ω");
}

#[test]
fn an_exact_function_refuses_what_has_no_exact_value() {
    stops_with(&main("Show 1.\nLet x: Algebraic be sqrt(-4).\nShow x."), "1", "square root of a negative number");
    stops_with(&main("Let x: ExactReal be ln(0).\nShow x."), "", "logarithm of a nonpositive number");
}

#[test]
fn omega_is_the_first_infinite_ordinal_in_an_ordinal_context() {
    shows(&main("Let w: Ordinal be omega.\nShow w."), "ω");
    shows(&main("Let w: Ordinal be omega + 1.\nShow w."), "ω + 1");
    shows(&main("Let w: Ordinal be 1 + omega.\nShow w."), "ω");
    shows(&main("Let w: Ordinal be ω * 2.\nShow w."), "ω·2");
    shows(&main("Let w: Ordinal be omega ** 2.\nShow w."), "ω^2");
    shows(&main("Let w: Ordinal be omega.\nShow w is greater than 1000000."), "true");
}

#[test]
fn the_base_of_a_power_is_an_exact_context_and_its_exponent_an_integer_one() {
    shows(&main("Let r: Rational be (1 / 2) ** 2.\nShow r."), "1/4");
    shows(&main("Let r: Rational be 4 ** (3 / 2).\nShow r."), "4");
    shows(&main("Let x: ExactReal be pi ** 2.\nShow x."), "π^2");
}

#[test]
fn omega_means_nothing_outside_an_ordinal_context() {
    stops_with(&main("Let x: ExactReal be omega.\nShow x."), "", "Undefined variable: omega");
    stops_with(&main("Show omega."), "", "Undefined variable: omega");
}

#[test]
fn untyped_code_keeps_its_meaning() {
    shows(&main("Show 7 / 2."), "3");
    shows(&main("Let pi be 3.\nShow pi * 2."), "6");
    shows(&main("Let n be 10.\nShow n / 4."), "2");
}
