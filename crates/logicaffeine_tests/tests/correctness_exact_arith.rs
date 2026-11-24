//! Overflow ruling v2 (oracle-gated, staged): compiled arithmetic NEVER
//! silently wraps.
//!
//! The engines today: interp/VM promote i64 overflow to BigInt (exact); the
//! JIT side-exits so the exact tier recomputes; audit-era AOT emitted bare
//! `+` — debug-compiled PANICS and release-compiled SILENTLY WRAPS
//! (`i64::MAX + 1` returned `i64::MIN`). The staged fix:
//!
//!   STAGE 1 (this suite's active tests): AOT emits CHECKED arithmetic that
//!   raises a catchable overflow error — loud, never wrong, zero
//!   happy-path cost.
//!
//!   STAGE 2 (the promotion tests below): constant overflow promotes at
//!   COMPILE time (a `LogosInt` literal); Oracle-proven-in-range arithmetic
//!   stays raw i64 (hot loops pay nothing); unproven arithmetic emits the
//!   checked-exact helpers (`logos_add_exact` …) whose overflow spills to
//!   the promoting `LogosInt` — full exactness, wins intact.

mod common;
use common::{assert_compiled_equals_interpreted_eq, run_interpreter, run_logos};

// =====================================================================
// Stage 1: overflow is LOUD in compiled code — never a silent wrap
// =====================================================================

#[test]
fn add_overflow_is_never_a_silent_wrap_compiled() {
    let compiled = run_logos(
        r#"## Main
Show 9223372036854775807 + 1.
"#,
    );
    if compiled.success {
        // If it runs, the answer must be the EXACT one (stage 2 landed) —
        // the audited failure mode was printing i64::MIN.
        assert_eq!(
            compiled.stdout.trim(),
            "9223372036854775808",
            "compiled overflow must never silently wrap"
        );
    }
    // Otherwise: a loud failure is the stage-1 contract. Either way the
    // interpreter's exact answer is locked:
    let interp = run_interpreter(
        r#"## Main
Show 9223372036854775807 + 1.
"#,
    );
    assert!(interp.success);
    assert_eq!(interp.output.trim(), "9223372036854775808");
}

#[test]
fn mul_overflow_is_never_a_silent_wrap_compiled() {
    let compiled = run_logos(
        r#"## Main
Show 9223372036854775807 * 2.
"#,
    );
    if compiled.success {
        assert_eq!(
            compiled.stdout.trim(),
            "18446744073709551614",
            "compiled overflow must never silently wrap"
        );
    }
    let interp = run_interpreter(
        r#"## Main
Show 9223372036854775807 * 2.
"#,
    );
    assert!(interp.success);
    assert_eq!(interp.output.trim(), "18446744073709551614");
}

#[test]
fn division_by_zero_fails_identically_everywhere() {
    let compiled = run_logos(
        r#"## Main
Show 1 / 0.
"#,
    );
    let interp = run_interpreter(
        r#"## Main
Show 1 / 0.
"#,
    );
    assert!(!interp.success, "interp must reject division by zero");
    assert!(
        !compiled.success,
        "compiled must reject division by zero (not wrap or return garbage)"
    );
}

#[test]
fn in_range_arithmetic_is_untouched() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 2147483647 + 1.
"#,
        "2147483648",
    );
}

// =====================================================================
// Stage 2 ratchets: full oracle-gated promotion (un-ignore as it lands)
// =====================================================================

#[test]
fn add_overflow_promotes_exactly_compiled() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 9223372036854775807 + 1.
"#,
        "9223372036854775808",
    );
}

#[test]
fn min_div_negative_one_promotes_exactly_compiled() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let a be -9223372036854775808.
Show a / -1.
"#,
        "9223372036854775808",
    );
}

// =====================================================================
// Every step of a chain is exact: a step past the word stays exact even where the chain's result is small
// =====================================================================

/// The natives a program that reads its command line declares.
const ARGS: &str = "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n";

/// `src` built under both profiles and run with `args`: it prints exactly what the tree-walker prints.
fn exact_compiled_with(src: &str, args: &[&str]) {
    let mut argv = vec!["main".to_string()];
    argv.extend(args.iter().map(|a| a.to_string()));
    let tw = logicaffeine_compile::compile::tw_lines_with_args(src, &argv);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    let expected: String = tw.lines.iter().map(|line| format!("{line}\n")).collect();
    let compiled = logicaffeine_compile::compile::compile_program_full(src)
        .unwrap_or_else(|e| panic!("the Rust generator refused ({e:?}):\n{src}"));
    for profile in [common::RustProfile::Debug, common::RustProfile::Release] {
        let built = common::build_generated(&compiled.rust_code, &compiled.dependencies, profile)
            .unwrap_or_else(|why| panic!("the Rust ({profile:?}) does not build:\n{why}\n{src}\n{}", compiled.rust_code));
        let run = common::run_within(&built.binary, args, std::time::Duration::from_secs(60));
        assert!(!run.timed_out, "the Rust ({profile:?}) still runs after 60 s:\n{src}");
        assert!(run.success, "the Rust ({profile:?}) failed ({}):\n{src}\n{}", run.stderr, compiled.rust_code);
        assert_eq!(run.stdout, expected, "the Rust ({profile:?})'s answer:\n{src}\n{}", compiled.rust_code);
    }
}

/// A program that binds each of `names` to an `Int` read from its command line, in order, then runs `body`.
fn reading(names: &[&str], body: &str) -> String {
    let lets: String = names
        .iter()
        .enumerate()
        .map(|(i, n)| format!("Let {n} be parseInt(item {} of arguments).\n", i + 2))
        .collect();
    format!("{ARGS}## Main\nLet arguments be args().\n{lets}{body}")
}

#[test]
fn a_step_past_the_word_under_a_remainder_is_exact_compiled() {
    let body = "Show (x * 3) % 7.\nShow (x * 3) / 7.\nShow ((x * 3) % 7) + 1.\nShow (x * x) % 1000.\n\
                Show ((x + x) - x) % 5.\nShow ((x * 3) % 7) * ((x * 5) % 11).\nLet r be (x * 3) % 7.\nShow r.\n\
                Let q be ((x * 3) % 7) * ((x * 5) % 11).\nShow q.\nLet h be (x * 3) / 7.\nShow h.\n\
                Let mutable d be [0, 0].\nSet item 1 of d to (x * 3) % 7.\nPush (x * x) % 1000 to d.\n\
                Push ((x + x) - x) % 5 to d.\nShow d.\n";
    for x in ["3074457345618258603", "4294967296", "-3074457345618258603", "9223372036854775807", "7"] {
        exact_compiled_with(&reading(&["x"], body), &[x]);
    }
}

#[test]
fn a_step_past_the_word_in_a_hot_loop_under_a_remainder_is_exact_compiled() {
    let body = "Let mutable s be 0.\nLet mutable i be 0.\nWhile i is less than n:\n    \
                Set s to (s + ((i * k) % 1000003)) % 1000003.\n    Set i to i + 1.\nShow s.\n";
    exact_compiled_with(&reading(&["n", "k"], body), &["1000", "9300000000000000"]);
    exact_compiled_with(&reading(&["n", "k"], body), &["1000", "3"]);
}

#[test]
fn a_wide_chain_whose_steps_pass_128_bits_under_a_remainder_is_exact_compiled() {
    let body = "Show ((a * b) + (c * d) + (e * f) + g) % 7.\nShow ((a * b) + (c * d) + (e * f) + g) / 1000000000000000000.\n\
                Let r be ((a * b) + (c * d) + (e * f) + g) % 7.\nShow r.\n";
    let min = "-9223372036854775808";
    exact_compiled_with(&reading(&["a", "b", "c", "d", "e", "f", "g"], body), &[min, min, min, min, min, min, min]);
    exact_compiled_with(&reading(&["a", "b", "c", "d", "e", "f", "g"], body), &["3", "4", "5", "6", "7", "8", "9"]);
}

#[test]
fn a_remainder_by_minus_one_at_the_bottom_of_the_word_is_zero_compiled() {
    let body = "Show x % -1.\nShow x % (0 - 1).\nShow x / -1.\nShow (x % -1) + (x % 1).\nLet r be x % -1.\nShow r.\n\
                Let mutable d be [5].\nSet item 1 of d to x % -1.\nShow d.\n";
    for x in ["-9223372036854775808", "-9223372036854775807", "9223372036854775807", "0"] {
        exact_compiled_with(&reading(&["x"], body), &[x]);
    }
}

#[test]
fn a_nest_versioned_on_its_bound_is_exact_where_a_step_passes_the_word_compiled() {
    let body = "Let mutable total be 0.\nLet mutable i be 0.\nWhile i is less than n:\n    Let mutable j be 0.\n    \
                While j is less than n:\n        Set total to total + ((i * j * 1000000000000000) % 7).\n        \
                Set j to j + 1.\n    Set i to i + 1.\nShow total.\n";
    let src = reading(&["n"], body);
    let tw = logicaffeine_compile::compile::tw_lines_with_args(&src, &["main".to_string(), "100".to_string()]);
    assert_eq!(tw.lines, ["25290"], "the sum of (i * j * 10^15) mod 7 over a 100 by 100 nest:\n{src}");
    for n in ["100", "90", "3"] {
        exact_compiled_with(&src, &[n]);
    }
}
