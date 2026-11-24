//! ★ LOCK — an `Int` that outgrows a machine word is exact on AOT-Rust, wherever it is held: a local, an argument, a
//! return, a record field, a value shown or compared. ★
//!
//! The Rust tier holds an `Int` as a machine word wherever the range oracle proves it stays one, and as an exact
//! integer wherever the shared exactness decision says some arithmetic can carry it past the word: a counter climbing
//! from a value near the word, a sum or product of unbounded terms, a value handed on through a call. A program prints
//! what the tree-walker prints, on a Debug build and a Release build of the generated Rust; it never stops with the
//! overflow sentence where the tree-walker continues.
//!
//! A counter a loop's bound keeps inside the word, and a count only one bounded loop advances, stay machine words:
//! the speed of every benchmark rests on them.
//!
//! The rows that need the shared scalar decision are ignored while it is held off (`exact_scalars::DECIDES_SCALARS`);
//! there, a value past the word stops with the overflow sentence, never wraps.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test rust_exact_scalars_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::time::Duration;

use common::RustProfile;
use logicaffeine_compile::compile::{compile_program_full, compile_to_rust, tw_lines_with_args};

/// The natives a program that reads its command line declares.
const ARGS: &str = "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n";

/// Main's first lines: `n` read from the command line.
const READ_N: &str = "## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\n";

/// What the tree-walker prints for `src` run with `args` on its command line, which must run clean.
fn reference(src: &str, args: &[&str]) -> String {
    let argv: Vec<String> = std::iter::once("main").chain(args.iter().copied()).map(str::to_string).collect();
    let tw = tw_lines_with_args(src, &argv);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    tw.lines.iter().map(|line| format!("{line}\n")).collect()
}

/// The Rust tier prints exactly what the tree-walker prints for `src` run with `args`, under both profiles.
fn exact_on_rust(src: &str, args: &[&str]) {
    let expected = reference(src, args);
    let compiled = compile_program_full(src).unwrap_or_else(|e| panic!("the Rust generator refused ({e:?}):\n{src}"));
    for profile in [RustProfile::Debug, RustProfile::Release] {
        let built = common::build_generated(&compiled.rust_code, &compiled.dependencies, profile)
            .unwrap_or_else(|why| panic!("the Rust ({profile:?}) does not build:\n{why}\n{src}\n{}", compiled.rust_code));
        let run = common::run_within(&built.binary, args, Duration::from_secs(60));
        assert!(!run.timed_out, "the Rust ({profile:?}) still runs after 60 s:\n{src}");
        assert!(run.success, "the Rust ({profile:?}) failed ({}):\n{src}\n{}", run.stderr, compiled.rust_code);
        assert_eq!(run.stdout, expected, "the Rust ({profile:?})'s answer:\n{src}\n{}", compiled.rust_code);
    }
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn a_counter_climbing_past_the_word_is_exact() {
    let src = "## Main\nLet mutable x be 9223372036854775557.\nLet mutable i be 0.\nWhile i is less than 400:\n    \
               Set x to x + 1.\n    Set i to i + 1.\nShow x.\n";
    exact_on_rust(src, &[]);
}

#[test]
fn a_counter_its_last_pass_carries_past_the_word_is_exact() {
    let src = "## Main\nLet mutable i be 9223372036854775806.\nWhile i is at most 9223372036854775807:\n    \
               Set i to i + 1.\nShow i.\n";
    exact_on_rust(src, &[]);
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn a_counter_climbing_from_an_input_is_exact() {
    let src = format!(
        "{ARGS}{READ_N}Let mutable x be n.\nLet mutable i be 0.\nWhile i is less than 10:\n    Set x to x + 1.\n    \
         Set i to i + 1.\nShow x.\n"
    );
    exact_on_rust(&src, &["9223372036854775800"]);
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn a_sum_of_terms_past_the_word_is_exact() {
    let src = format!(
        "{ARGS}{READ_N}Let mutable s be 0.\nLet mutable i be 0.\nWhile i is less than 4:\n    Set s to s + n.\n    \
         Set i to i + 1.\nShow s.\n"
    );
    exact_on_rust(&src, &["9223372036854775807"]);
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn a_product_past_the_word_is_exact() {
    let src = format!("{ARGS}{READ_N}Let b be n * n.\nShow b.\nShow b - n.\n");
    exact_on_rust(&src, &["4294967296"]);
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn a_difference_below_the_word_is_exact() {
    let src = format!("{ARGS}{READ_N}Let d be 0 - n - n.\nShow d.\n");
    exact_on_rust(&src, &["9223372036854775807"]);
}

#[test]
fn a_value_past_the_word_shown_or_compared_unbound_is_exact() {
    let src = format!(
        "{ARGS}{READ_N}Show n * n.\nIf n * n is greater than n:\n    Show \"past\".\nShow \"\" + (n + n).\n"
    );
    exact_on_rust(&src, &["4294967296"]);
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn a_value_past_the_word_returned_is_exact() {
    let src = format!(
        "{ARGS}## To bump (x: Int) -> Int:\n    Return x + 1.\n\n{READ_N}Show bump(n).\nShow bump(bump(n)) - n.\n"
    );
    exact_on_rust(&src, &["9223372036854775807"]);
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn a_value_past_the_word_handed_to_a_function_is_exact() {
    let src = format!(
        "{ARGS}## To report (x: Int):\n    Show x.\n    Show x - 1.\n\n{READ_N}report(n * 3).\n"
    );
    exact_on_rust(&src, &["4611686018427387904"]);
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn a_value_past_the_word_a_recursion_carries_is_exact() {
    let src = format!(
        "{ARGS}## To up (x: Int) and (k: Int) -> Int:\n    If k equals 0:\n        Return x.\n    \
         Return up(x + 1, k - 1).\n\n{READ_N}Show up(n, 5).\n"
    );
    exact_on_rust(&src, &["9223372036854775805"]);
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn a_value_past_the_word_in_a_record_field_is_exact() {
    let src = format!(
        "{ARGS}## A Cell has:\n    A v: Int.\n\n{READ_N}Let c be a new Cell with v (n * 3).\nShow c's v.\n\
         Show c's v - n.\n"
    );
    exact_on_rust(&src, &["4611686018427387904"]);
}

#[test]
fn counters_a_bound_keeps_in_the_word_stay_machine_words() {
    let src = format!(
        "{ARGS}{READ_N}Let mutable count be 0.\nLet mutable i be 0.\nWhile i is less than n:\n    \
         If i % 3 equals 0:\n        Set count to count + 1.\n    Set i to i + 1.\nShow count.\n"
    );
    let rust = compile_to_rust(&src).expect("Rust for the program");
    assert!(!rust.contains("LogosInt"), "a counted loop's counter and its bounded count are machine words:\n{rust}");
    exact_on_rust(&src, &["30"]);
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn a_loop_run_again_exactly_restores_the_words_it_assigned() {
    let src = format!(
        "{ARGS}{READ_N}Let mutable x be n.\nLet mutable c be 0.\nLet mutable seen be 0.\nWhile c is less than 10:\n    \
         Set x to x + 1.\n    Set seen to seen + c.\n    Set c to c + 1.\nShow x.\nShow c.\nShow seen.\n"
    );
    let rust = compile_to_rust(&src).expect("Rust for the program");
    assert!(
        rust.contains("failure::speculate"),
        "a loop whose only effects are on locals runs on machine words, again exactly where one leaves the word:\n{rust}"
    );
    exact_on_rust(&src, &["9223372036854775802"]);
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn a_loop_that_shows_is_never_run_twice() {
    let src = format!(
        "{ARGS}{READ_N}Let mutable x be n.\nLet mutable i be 0.\nWhile i is less than 6:\n    Set x to x + 1.\n    \
         Show x.\n    Set i to i + 1.\n"
    );
    let rust = compile_to_rust(&src).expect("Rust for the program");
    assert!(!rust.contains("failure::speculate"), "a loop that shows a value is never run again:\n{rust}");
    exact_on_rust(&src, &["9223372036854775804"]);
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn a_pure_function_with_an_exact_result_runs_its_word_twin_first() {
    let src = format!(
        "{ARGS}## To total (k: Int) and (x: Int) -> Int:\n    If k equals 0:\n        Return x.\n    \
         Return k + total(k - 1, x).\n\n{READ_N}Show total(100, 7).\nShow total(3, n).\n"
    );
    let rust = compile_to_rust(&src).expect("Rust for the program");
    assert!(
        rust.contains("fn total__w(") && rust.contains("failure::speculate"),
        "a pure function on integers runs on machine words, and exactly where a value leaves the word:\n{rust}"
    );
    exact_on_rust(&src, &["9223372036854775800"]);
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn an_item_a_loop_carries_in_a_register_steps_past_the_word_exactly() {
    for body in [
        "Let mutable d be [0, 9223372036854775805, 5].\nLet mutable i be 0.\nLet mutable seen be 0.\n\
         While i is less than n:\n    Set item 2 of d to (item 2 of d) + 1.\n    If item 2 of d is greater than 9223372036854775806:\n        \
         Set seen to seen + 1.\n    Set i to i + 1.\nShow d.\nShow seen.\nShow i.\n",
        "Let mutable d be [9223372036854775800, 1, 2].\nLet mutable e be [0, 0, 0].\nLet mutable i be 0.\n\
         While i is less than n:\n    Set item 1 of d to (item 1 of d) + (item 2 of e).\n    Set item 2 of e to (item 2 of e) + 1.\n    \
         Set i to i + 1.\nShow d.\nShow e.\n",
    ] {
        exact_on_rust(&format!("{ARGS}{READ_N}{body}"), &["7"]);
    }
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn a_pure_function_over_a_list_runs_its_word_twin_and_gives_up_on_a_value_past_the_word() {
    let sum = "## To bumped (src: Seq of Int) -> Int:\n    Let mutable buf be a new Seq of Int.\n    \
               Repeat for i from 1 to 4:\n        Push item i of src to buf.\n    Repeat for i from 1 to 4:\n        \
               Set item i of buf to (item i of buf) + 1.\n    Let mutable total be 0.\n    Repeat for i from 1 to 4:\n        \
               Set total to total + item i of buf.\n    Return total.\n\n";
    let listed = "## To stepped (src: Seq of Int) -> Seq of Int:\n    Let mutable buf be a new Seq of Int.\n    \
                  Repeat for i from 1 to 4:\n        Push item i of src to buf.\n    Repeat for r from 1 to 3:\n        \
                  Repeat for i from 1 to 4:\n            Set item i of buf to (item i of buf) + 1.\n    Return buf.\n\n";
    for (header, main) in [
        (sum, "Let s be [n, 2, 3, 4].\nShow bumped(s).\n"),
        (listed, "Let s be [n, 2, 3, 4].\nLet out be stepped(s).\nShow out.\nShow item 1 of out.\n"),
    ] {
        let src = format!("{ARGS}{header}{READ_N}{main}");
        let rust = compile_to_rust(&src).expect("Rust for the program");
        assert!(rust.contains("__w("), "a pure function over a list of `Int` runs its word twin first:\n{rust}");
        for n in ["5", "9223372036854775806", "-9223372036854775808"] {
            exact_on_rust(&src, &[n]);
        }
    }
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn a_word_twin_takes_and_gives_its_lists_in_the_forms_both_functions_hold_them() {
    let total = "## To total (xs: Seq of Int) and (i: Int) -> Int:\n    If i is greater than length of xs:\n        \
                 Return 0.\n    Return item i of xs + total(xs, i + 1).\n\n";
    let doubled = "## To doubled (xs: Seq of Int) -> Seq of Int:\n    Let mutable out be a new Seq of Int.\n    \
                   Repeat for x in xs:\n        Push x * 2 to out.\n    Return out.\n\n";
    for (header, main) in [
        (total, "Let s be [n, 2, 3].\nShow total(s, 1).\n"),
        (total, "Let mutable s be [n, 2, 3].\nSet item 2 of s to (item 2 of s) * n.\nShow total(s, 1).\nShow s.\n"),
        (doubled, "Let d be doubled([n, 1, 2]).\nShow d.\nShow item 1 of d.\n"),
        (doubled, "Let s be [n, 2, 3].\nLet d be doubled(s).\nShow d.\nShow s.\n"),
    ] {
        let src = format!("{ARGS}{header}{READ_N}{main}");
        let rust = compile_to_rust(&src).expect("Rust for the program");
        assert!(rust.contains("__w("), "a pure function over a list of `Int` runs its word twin first:\n{rust}");
        for n in ["5", "4611686018427387904", "9223372036854775807", "-9223372036854775808"] {
            exact_on_rust(&src, &[n]);
        }
    }
}

#[test]
fn a_function_writing_the_list_it_is_handed_runs_no_twin_on_that_list() {
    let direct = "## To bump (xs: Seq of Int) -> Seq of Int:\n    Set item 1 of xs to item 1 of xs + 1.\n    Return xs.\n\n";
    let copied = "## To bump (xs: Seq of Int) -> Seq of Int:\n    Let mutable d be xs.\n    Set item 1 of d to item 1 of d + 1.\n    \
                  Return d.\n\n";
    for (header, twin_may_run) in [(direct, false), (copied, true)] {
        let src = format!(
            "{ARGS}{header}{READ_N}Let mutable s be [n, 2].\nLet mutable k be 0.\nWhile k is less than 3:\n    \
             Set s to bump(s).\n    Set k to k + 1.\nShow s.\n"
        );
        let rust = compile_to_rust(&src).expect("Rust for the program");
        assert!(
            twin_may_run || !rust.contains("speculate(|| bump__w("),
            "a twin cut short would leave the list it was handed part-written; a copy the body takes is its own:\n{rust}"
        );
        for n in ["5", "9223372036854775805", "9223372036854775807"] {
            exact_on_rust(&src, &[n]);
        }
    }
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn a_product_an_accumulated_recursion_carries_past_the_word_is_exact() {
    for header in [
        "## To factorial (n: Int):\n    If n is less than 2:\n        Return 1.\n    Return n * factorial(n - 1).\n\n",
        "## To factorial (n: Int) -> Int:\n    If n is less than 2:\n        Return 1.\n    Return n * factorial(n - 1).\n\n",
        "## To total (n: Int):\n    If n is less than 1:\n        Return 0.\n    Return n * 4611686018427387904 + total(n - 1).\n\n",
    ] {
        let name = if header.contains("factorial") { "factorial" } else { "total" };
        let src = format!("{ARGS}{header}{READ_N}Show {name}(n).\n");
        for n in ["5", "25", "40"] {
            exact_on_rust(&src, &[n]);
        }
    }
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn the_magnitude_of_the_least_word_is_exact() {
    let src = format!("{ARGS}{READ_N}Let a be abs(n).\nShow a.\nShow abs(n) - 1.\nShow abs(n - 1).\n");
    for n in ["-9223372036854775808", "-9223372036854775807", "-5", "0", "9223372036854775807"] {
        exact_on_rust(&src, &[n]);
    }
}

#[test]
fn a_counted_loop_over_an_exact_bound_adds_its_counter_into_a_word() {
    let src = "## To countTo (n: Int) -> Int:\n    Let mutable s be 0.\n    Repeat for i from 1 to n:\n        Set s to s + i.\n\
               \x20   Return s.\n\n## Main\nLet mutable m be 1.\nLet mutable k be 0.\nWhile k is less than 3:\n\
               \x20   Set m to m * 10.\n    Set k to k + 1.\nShow countTo(m).\nShow countTo(4).\n";
    exact_on_rust(src, &[]);
}

#[test]
fn a_counter_a_range_over_an_exact_bound_binds_is_read_twice() {
    let src = "## Main\nLet mutable len be 16.\nLet mutable total be 0.\nWhile len is at most 128:\n    Let chunks be len / 16.\n\
               \x20   Repeat for b from 0 to 1:\n        Let start be b * 2 * len.\n        Repeat for c from 0 to chunks - 1:\n\
               \x20           Let lowOff be start + 16 * c.\n            Let highOff be start + len + 16 * c.\n\
               \x20           Set total to total + lowOff + highOff.\n    Set len to len * 2.\nShow total.\n";
    exact_on_rust(src, &[]);
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn a_counted_loop_walks_bounds_an_exact_binding_holds_past_the_word() {
    let src = format!(
        "{ARGS}{READ_N}Let big be n + n.\nRepeat for i from big to big + 2:\n    Show i.\nLet mutable seen be 0.\n\
         Repeat for j from 0 - big to 3 - big:\n    Set seen to seen + 1.\n    Show j.\nShow seen.\n\
         Repeat for k from big to big - 1:\n    Show k.\n"
    );
    for n in ["5", "4611686018427387904", "9223372036854775807"] {
        exact_on_rust(&src, &[n]);
    }
}
