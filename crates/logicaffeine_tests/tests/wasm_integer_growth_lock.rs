//! Unbounded Int arithmetic remains exact in standalone WebAssembly.
#![cfg(all(feature = "wasm-jit", not(target_arch = "wasm32")))]
#[path = "compile_common/mod.rs"]
mod compile_common;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

fn holds(source: &str, expected: &str) {
    for (name, run) in [("tree walker", tw_outcome(source)), ("VM", vm_outcome(source))] {
        assert_eq!(run.error, None, "{name}: {source}");
        assert_eq!(run.output.lines().collect::<Vec<_>>(), expected.lines().collect::<Vec<_>>(), "{name}: {source}");
    }
    let output = compile_common::wasm_host::run_show(source)
        .unwrap_or_else(|why| panic!("WASM must run exact Int arithmetic: {why}\n{source}"));
    assert_eq!(output.lines().collect::<Vec<_>>(), expected.lines().collect::<Vec<_>>(), "{source}");
}

const LARGE: &str = "11790184577738583171520872861412518665678211592275841109096961";

#[test]
fn small_integer_growth_remains_exact() {
    holds("## Main\nLet mutable a be 3.\nRepeat for i from 1 to 3:\n    Set a to a * a.\nShow a.\n", "6561");
}

#[test]
fn integer_growth_beyond_a_machine_word_remains_exact() {
    holds("## Main\nLet mutable a be 3.\nRepeat for i from 1 to 7:\n    Set a to a * a.\nShow a.\n", LARGE);
}

#[test]
fn grown_integer_and_small_integer_exchange_values() {
    holds("## Main\nLet mutable a be 3.\nLet mutable b be 5.\nRepeat for i from 1 to 7:\n    Set a to a * a.\nSwap a and b.\nShow a.\nShow b.\n", &format!("5\n{LARGE}"));
}

#[test]
fn repeated_addition_crosses_the_machine_word_boundary() {
    holds("## Main\nLet mutable a be 9223372036854775807.\nRepeat for i from 1 to 2:\n    Set a to a + 1.\nShow a.\n", "9223372036854775809");
}
