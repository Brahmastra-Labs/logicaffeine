//! Every word constructor reduces an unbounded Int into its own finite ring.
#![cfg(not(target_arch = "wasm32"))]
mod common;

const SOURCE: &str = r#"## No Inline
## To emit (n: Int):
    Show word8(n).
    Show word16(n).
    Show word32(n).
    Show word64(n).
## Main
Let large be 2 ** 100 + 19.
Call emit with large.
Call emit with 0 - large.
"#;

const EXPECTED: &str = "19\n19\n19\n19\n237\n65517\n4294967277\n18446744073709551597";

#[test]
fn wide_integer_word_constructors_wrap_on_the_reference_and_vm() {
    let reference = logicaffeine_compile::compile::tw_outcome(SOURCE);
    assert_eq!(reference.error, None);
    assert_eq!(reference.output.trim_end(), EXPECTED);
    let vm = logicaffeine_compile::compile::vm_outcome(SOURCE);
    assert_eq!(vm.error, None);
    assert_eq!(vm.output.trim_end(), EXPECTED);
}

#[test]
fn wide_integer_word_constructors_wrap_on_aot_rust() {
    common::assert_exact_output(SOURCE, EXPECTED);
}

#[test]
fn wide_integer_word_constructors_wrap_on_aot_c() {
    common::assert_c_output(SOURCE, EXPECTED);
}

const CALL_SOURCE: &str = r#"## No Inline
## To keep (n: Int) -> Int:
    Let shifted be n + 1.
    Return shifted - 1.
## To relay (n: Int) -> Int:
    Return keep(n).
## Main
Let n be 2 ** 100 + 19.
Show relay(n).
Show relay(0 - n).
"#;

const CALL_EXPECTED: &str = "1267650600228229401496703205395\n-1267650600228229401496703205395";

#[test]
fn ordinary_calls_preserve_all_integer_bits_on_the_reference_and_vm() {
    for outcome in [
        logicaffeine_compile::compile::tw_outcome(CALL_SOURCE),
        logicaffeine_compile::compile::vm_outcome(CALL_SOURCE),
    ] {
        assert_eq!(outcome.error, None);
        assert_eq!(outcome.output.trim_end(), CALL_EXPECTED);
    }
}

#[test]
fn ordinary_calls_preserve_all_integer_bits_on_aot_rust() {
    common::assert_exact_output(CALL_SOURCE, CALL_EXPECTED);
}

#[test]
fn ordinary_calls_preserve_all_integer_bits_on_aot_c() {
    common::assert_c_output(CALL_SOURCE, CALL_EXPECTED);
}

#[test]
fn integer_power_results_keep_their_exact_return_type_in_c() {
    let source = "## To big () -> Int:\n    Return pow(2, 100).\n## Main\nShow big().\n";
    common::assert_c_output(source, "1267650600228229401496703205376");
}
