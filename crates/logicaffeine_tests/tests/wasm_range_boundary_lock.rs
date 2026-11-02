//! Boundary and nested-control-flow semantics for a constant-storage numeric iterator.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::run_outcome;

fn answer(source: &str, expected: &str) {
    let got = run_outcome(source).expect("valid wasm module");
    assert_eq!(got.failure, None);
    assert_eq!(got.output, expected);
}

#[test]
fn the_last_machine_integer_is_yielded_once_without_incrementing_it() {
    answer(
        "## Main\nRepeat for i from 9223372036854775806 to 9223372036854775807:\n    Show i.\n",
        "9223372036854775806\n9223372036854775807",
    );
}

#[test]
fn a_range_ending_at_the_first_machine_integer_runs_once() {
    answer(
        "## Main\nRepeat for i from -9223372036854775808 to -9223372036854775808:\n    Show i.\n",
        "-9223372036854775808",
    );
}

#[test]
fn an_early_return_from_nested_ranges_does_not_pop_the_callers_iterator() {
    answer(
        r#"## To pick (n: Int) -> Int:
    Repeat for x from n to n + 1:
        Repeat for y from 7 to 8:
            Return x * 10 + y.
    Return 0.

## Main
Repeat for i from 1 to 3:
    Show pick(i).
    Show i.
"#,
        "17\n1\n27\n2\n37\n3",
    );
}
