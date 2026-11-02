//! The named power builtin has the same numeric result contract as exponentiation.
mod common;
#[test]
fn integer_power_keeps_its_exact_declared_return() {
    common::assert_c_output("## No Inline\n## To raise (n: Int) -> Int:\n    Return pow(n, 100).\n## Main\nShow raise(2).\n", "1267650600228229401496703205376");
}
#[test]
fn a_real_operand_makes_the_power_real() {
    common::assert_c_output("## No Inline\n## To raise (n: Real) -> Real:\n    Return pow(n, 3).\n## Main\nShow raise(2.5).\n", "15.625");
}
#[test]
fn a_real_power_is_not_accepted_as_an_integer_return() {
    let source = "## To raise (n: Real) -> Int:\n    Return pow(n, 3).\n## Main\nShow raise(2.5).\n";
    assert!(logicaffeine_compile::compile::compile_to_c(source).is_err(), "typed C must reject a Real result declared Int");
}
#[test]
fn a_program_defined_power_keeps_its_own_signature() {
    common::assert_c_output("## No Inline\n## To pow (n: Int) and (e: Int) -> Text:\n    Return \"declared\".\n## Main\nShow pow(2, 3).\n", "declared");
}
