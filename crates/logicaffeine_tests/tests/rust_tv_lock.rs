//! Translation validation of the emitted Rust: for every input, each function `compile_to_rust` emits computes what
//! its source computes, or stops with a loud integer overflow where the source holds a value past 64 bits
//! (`ProvenUpToOverflow`: it never computes a wrong answer). The emitter's guarded fast paths, its versioned loops and
//! its loop for a tail call are proved as written; a seeded miscompile of the emitted text is refuted with an input,
//! a silent wrap included; and the Rust executor shows, run concretely, what the tree-walker shows. Requires the
//! `verification` feature (Z3).

#![cfg(feature = "verification")]

use logicaffeine_tv::bytecode::{ExecutorCheck, Verdict};
use logicaffeine_tv::rust::{check_executor, validate, validate_edited};

fn holds(src: &str, function: &str) -> Vec<Vec<String>> {
    let report = validate(src).unwrap_or_else(|e| panic!("{e}\n{src}"));
    match report.verdict(function) {
        Some(Verdict::Proven { invariants, .. } | Verdict::ProvenUpToOverflow { invariants, .. }) => invariants.clone(),
        other => panic!("`{function}` not proven against its Rust: {other:?}\n{src}"),
    }
}

fn refuted(src: &str, function: &str, edit: impl FnOnce(&mut String)) -> (String, String, String) {
    let report = validate_edited(src, edit).expect("compiles");
    match report.verdict(function) {
        Some(Verdict::Refuted { input, source, bytecode, .. }) => {
            assert_ne!(source, bytecode, "a refutation shows the two apart");
            (input.clone(), source.clone(), bytecode.clone())
        }
        other => panic!("a seeded miscompile of `{function}`'s Rust was not refuted: {other:?}\n{src}"),
    }
}

/// Replace the one occurrence of `old` in the emitted text with `new`.
fn swap(old: &'static str, new: &'static str) -> impl FnOnce(&mut String) {
    move |text: &mut String| {
        assert_eq!(text.matches(old).count(), 1, "`{old}` occurs once in the emitted Rust:\n{text}");
        *text = text.replacen(old, new, 1);
    }
}

const POLY: &str = "## To poly (x: Int) -> Int:\n    Return x * x + 3 * x - 7.\n\n## Main\nShow poly(5).\n";

const SIGN: &str = "## To sign (x: Int) -> Int:\n    If x is greater than 0:\n        Return 1.\n    If x is less than 0:\n        Return 0 - 1.\n    Return 0.\n\n## Main\nShow sign(0 - 4).\n";

const GCD_LOOP: &str = "## To gcd (a: Int, b: Int) -> Int:\n    Let mutable x be a.\n    Let mutable y be b.\n    While y is not 0:\n        Let t be y.\n        Set y to x % y.\n        Set x to t.\n    Return x.\n\n## Main\nShow gcd(1071, 462).\n";

const GCD_REC: &str = "## To gcd (a: Int, b: Int) -> Int:\n    If b equals 0:\n        Return a.\n    Return gcd(b, a % b).\n\n## Main\nShow gcd(48, 18).\n";

const COLLATZ: &str = "## To steps (n: Int) -> Int:\n    Let mutable x be n.\n    Let mutable count be 0.\n    While x is greater than 1:\n        If x % 2 equals 0:\n            Set x to x / 2.\n        Otherwise:\n            Set x to 3 * x + 1.\n        Set count to count + 1.\n    Return count.\n\n## Main\nShow steps(27).\n";

const BOOLS: &str = "## To pick (a: Bool, b: Bool, x: Int) -> Int:\n    If a and not b:\n        Return x.\n    If a or b:\n        Return 0 - x.\n    Return 0.\n\n## Main\nShow pick(true, false, 7).\n";

const SEVENS: &str = "## To sev (x: Int) -> Int:\n    If x is at least 0:\n        Return x % 7 + x / 8.\n    Return 0.\n\n## Main\nShow sev(13).\n";

const SUM_RANGE: &str = "## To sum (n: Int) -> Int:\n    Let mutable total be 0.\n    Repeat for i from 1 to n:\n        Set total to total + i.\n    Return total.\n\n## Main\nShow sum(10).\n";

const SUM_WHILE: &str = "## To sum (n: Int) -> Int:\n    Let mutable total be 0.\n    Let mutable i be 1.\n    While i is at most n:\n        Set total to total + i.\n        Set i to i + 1.\n    Return total.\n\n## Main\nShow sum(10).\n";

const FIB: &str = "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(10).\n";

const BREAK: &str = "## To first_square_above (n: Int) -> Int:\n    Let mutable i be 0.\n    While true:\n        If i * i is greater than n:\n            Break.\n        Set i to i + 1.\n    Return i.\n\n## Main\nShow first_square_above(50).\n";

const SQUARES: &str = "## To sq (n: Int) -> Int:\n    Let mutable total be 0.\n    Let mutable i be 1.\n    While i is at most n:\n        Set total to total + i * i.\n        Set i to i + 1.\n    Return total.\n\n## Main\nShow sq(10).\n";

const CUBES: &str = "## To cu (n: Int) -> Int:\n    Let mutable total be 0.\n    Let mutable i be 1.\n    While i is at most n:\n        Set total to total + i * i * i.\n        Set i to i + 1.\n    Return total.\n\n## Main\nShow cu(10).\n";

const MAIN_SUM: &str = "## Main\nLet mutable total be 0.\nLet mutable i be 1.\nWhile i is at most 100:\n    Set total to total + i.\n    Set i to i + 1.\nShow total.\n";

const FROM_K: &str = "## To sk (k: Int, n: Int) -> Int:\n    Let mutable total be 0.\n    Let mutable i be k.\n    While i is less than n:\n        Set total to total + i.\n        Set i to i + 1.\n    Return total.\n\n## Main\nShow sk(3, 10).\n";

const AFFINE: &str = "## To af (n: Int) -> Int:\n    Let mutable total be 0.\n    Let mutable i be 1.\n    While i is at most n:\n        Set total to total + 3 * i + 2.\n        Set i to i + 1.\n    Return total.\n\n## Main\nShow af(10).\n";

const UNTIL: &str = "## To f (n: Int) -> Int:\n    Let mutable i be 0.\n    While i is not n:\n        Set i to i + 1.\n    Return i.\n\n## Main\nShow f(3).\n";

const COUNT: &str = "## To cnt (a: Int, b: Int) -> Int:\n    Let mutable c be 0.\n    Let mutable i be a.\n    While i is at most b:\n        Set c to c + 1.\n        Set i to i + 1.\n    Return c.\n\n## Main\nShow cnt(3, 10).\n";

const SHOWN_SUM: &str = "## To shown_sum (n: Int) -> Int:\n    Show n.\n    Let mutable total be 0.\n    Let mutable i be 1.\n    While i is at most n:\n        Set total to total + i.\n        Set i to i + 1.\n    Return total.\n\n## Main\nShow shown_sum(10).\n";

const MAIN_LOOP: &str = "## Main\nLet mutable total be 0.\nLet mutable i be 1.\nWhile i is at most 20:\n    If i % 3 equals 0:\n        Show i.\n    Set total to total + i * i.\n    Set i to i + 1.\nShow total.\n";

#[test]
fn a_guarded_fast_path_is_proven_never_to_compute_a_wrong_answer() {
    holds(POLY, "poly");
    holds(SEVENS, "sev");
}

#[test]
fn branches_are_proven() {
    holds(SIGN, "sign");
    holds(BOOLS, "pick");
}

#[test]
fn a_function_that_cannot_overflow_is_proven_outright_and_one_that_can_says_so() {
    let sign = validate(SIGN).unwrap();
    assert!(matches!(sign.verdict("sign"), Some(Verdict::Proven { .. })), "`sign` computes no value past 64 bits: {sign:?}");
    let pick = validate(BOOLS).unwrap();
    assert!(
        matches!(pick.verdict("pick"), Some(Verdict::ProvenUpToOverflow { .. })),
        "`0 - x` at the bottom of i64 needs 64 bits and one: {pick:?}"
    );
}

#[test]
fn a_loop_the_emitter_versions_on_a_guard_is_proven_against_one_source_loop() {
    let inv = holds(GCD_LOOP, "gcd");
    assert_eq!(inv.len(), 2, "one source loop paired with each of its two Rust copies: {inv:?}");
}

#[test]
fn a_tail_call_the_emitter_turns_into_a_loop_is_proven_as_that_loop() {
    let inv = holds(GCD_REC, "gcd");
    assert_eq!(inv.len(), 1, "{inv:?}");
}

#[test]
fn a_loop_with_a_masked_parity_test_is_proven() {
    holds(COLLATZ, "steps");
}

#[test]
fn a_counted_range_is_proven_for_every_bound() {
    let inv = holds(SUM_RANGE, "sum");
    assert!(inv[0].iter().any(|c| c.ends_with("= total")), "{inv:?}");
}

#[test]
fn main_with_a_loop_and_shows_is_proven() {
    holds(MAIN_LOOP, "## Main");
}

/// The emitter replaces the summation loop with `n·(n+1)/2` behind an `n ≥ 1` test and an i64 guard, an exact cold
/// path past it. Each of the Rust's paths that returns is proved against the source loop on its own, under the
/// invariant the source's runs suggest: `i² = 2·total + i` at the head, which the loop's exit turns into the closed
/// form. The path that stops with an overflow is why the verdict is `ProvenUpToOverflow`.
#[test]
fn a_summation_the_emitter_replaces_with_its_closed_form_is_proven_for_every_n() {
    let report = validate(SUM_WHILE).unwrap();
    let Some(Verdict::ProvenUpToOverflow { invariants, .. }) = report.verdict("sum") else {
        panic!("{report:?}")
    };
    let ran = ["n = n₀", "i² = 2·total + i", "i ≥ 1", "total − i ≥ -1", "i − n₀ ≤ 1"];
    let skipped = ["n = n₀", "i² = 2·total + i", "total = 0", "i = 1"];
    for case in invariants {
        let (when, conjuncts) = case.split_first().expect("a case and its conjuncts");
        let want: &[&str] = if when.ends_with("n₀ < 1") { &skipped } else { &ran };
        assert_eq!(conjuncts, want, "{when}");
    }
    assert!(invariants.iter().any(|c| c[0].ends_with("n₀ < 1")), "{invariants:?}");
    assert_eq!(
        invariants.len(),
        2,
        "the path that returns n·(n+1)/2 and the one that returns 0 are proved; the one that stops past i64 shows \
         nothing, so it is only recorded, in the verdict: {invariants:?}"
    );
}

/// Faulhaber's sums of squares and cubes, closed by the emitter, are proved through relations of degree three and
/// four; the cube sum's division by two is exact because `n·(n+1)` is even, which is given to the solver as a lemma.
#[test]
fn the_sums_of_squares_and_cubes_the_emitter_closes_are_proven() {
    for (src, function, power) in [(SQUARES, "sq", "³"), (CUBES, "cu", "⁴")] {
        let inv = holds(src, function);
        assert!(inv.iter().flatten().any(|c| c.contains(power) && c.contains("total")), "{function}: {inv:?}");
    }
}

/// A value shown before the loop is matched against the closed form's output where it falls: the head is reached
/// having shown one value, every path of the Rust shows it first, and the one that stops past `i64` has shown it
/// before stopping, which is why the verdict says the Rust may stop. A changed shown value is refuted.
#[test]
fn a_value_shown_before_a_closed_form_is_matched_where_it_falls() {
    let report = validate(SHOWN_SUM).unwrap();
    assert!(matches!(report.verdict("shown_sum"), Some(Verdict::ProvenUpToOverflow { .. })), "{report:?}");
    let (input, source, rust) = refuted(SHOWN_SUM, "shown_sum", swap("show(&{ n });", "show(&{ n + 1 });"));
    let n: i64 = input.trim_start_matches("n = ").parse().unwrap();
    assert!(source.starts_with(&format!("shows {n} ")) && rust.starts_with(&format!("shows {} ", n + 1)), "{source} / {rust}");
}

/// `## Main` whose loop the compiler runs at compile time, the Rust printing the constant, is proved against the loop.
#[test]
fn a_main_the_compiler_runs_to_a_constant_is_proven_against_its_loop() {
    holds(MAIN_SUM, "## Main");
}

#[test]
fn a_main_printing_the_wrong_constant_for_its_loop_is_refuted() {
    let (_, source, rust) = refuted(MAIN_SUM, "## Main", swap("show(&5050);", "show(&5051);"));
    assert_eq!((source.as_str(), rust.as_str()), ("shows 5050 and finishes", "shows 5051 and finishes"));
}

#[test]
fn a_main_printing_its_constant_twice_is_refuted() {
    let (_, source, rust) = refuted(MAIN_SUM, "## Main", swap("show(&5050);", "show(&5050); show(&5050);"));
    assert_eq!((source.as_str(), rust.as_str()), ("shows 5050 and finishes", "shows 5050, 5050 and finishes"));
}

#[test]
fn a_closed_form_with_a_changed_coefficient_is_refuted_with_an_input() {
    let (input, source, rust) = refuted(SUM_WHILE, "sum", swap("(n * (n + 1i64)) / 2i64", "(n * (n + 2i64)) / 2i64"));
    let n: i64 = input.trim_start_matches("n = ").parse().unwrap();
    assert!(n >= 1, "only a loop that runs shows it: {input}");
    assert_eq!(source, format!("returns {}", n * (n + 1) / 2));
    assert_eq!(rust, format!("returns {}", n * (n + 2) / 2));
}

/// A closed form wrong only where its cold path runs (n past 2³¹) cannot be shown wrong by a replay of a loop that long,
/// and is never proven either.
#[test]
fn a_closed_form_wrong_only_past_its_guard_is_never_proven() {
    let report = validate_edited(SUM_WHILE, swap("logos_add_exact(n, 1i64)", "logos_add_exact(n, 2i64)")).unwrap();
    assert!(
        !matches!(report.verdict("sum"), Some(Verdict::Proven { .. } | Verdict::ProvenUpToOverflow { .. })),
        "{report:?}"
    );
}

/// A proof against a target that always ends must show the source ends too. `i` counts up from zero to `n` and never
/// meets a negative `n`; with the loop replaced by `n`, the two agree on every run that ends, and the validator still
/// does not prove them alike.
#[test]
fn a_closed_form_for_a_loop_the_source_may_never_leave_is_never_proven() {
    let loop_text = "    let mut i: i64 = 0;\n    while i != n {\n        i = logos_add_i64(i, 1);\n    }\n    i\n";
    let report = validate_edited(UNTIL, swap(loop_text, "    n\n")).unwrap();
    match report.verdict("f") {
        Some(Verdict::Unproven { at, .. }) => assert!(at.contains("may never leave the loop"), "{at}"),
        other => panic!("{other:?}"),
    }
}

/// The emitter versions this loop on `n ∈ [-10⁹, 10⁹]` and accumulates with a plain `+` inside: the loop head keeps the
/// guard it was entered under and the relation its runs suggest, which together bound `total` well inside `i64`.
#[test]
fn a_loop_versioned_on_its_bound_is_proven_through_the_guard_it_was_entered_under() {
    let inv = holds(AFFINE, "af");
    let fast = inv.iter().find(|c| c.iter().any(|x| x == "n₀ ≤ 1000000000")).unwrap_or_else(|| panic!("{inv:?}"));
    assert!(fast.iter().any(|x| x == "3·i² + i = 2·total + 4"), "{fast:?}");
}

/// The emitter leaves this loop by `break` at `i == b`, where the source tests `i ≤ b` once more, and steps `i` with a
/// plain `+= 1`. The Rust's parameters are `i64`, so no caller can pass a `b` past `i64::MAX`, the step below it never
/// wraps, and the source's extra test is the exit the Rust's `break` took.
#[test]
fn a_loop_the_rust_leaves_by_break_at_its_bound_is_proven_for_every_i64_input() {
    holds(COUNT, "cnt");
}

/// A counter seeded from an input: the emitter versions `total += i` on a guard over both the limit and the seed
/// (`n, k ∈ [-2·10⁹, 2·10⁹]`), and the fast loop is proven through the sum its runs suggest. Under B24's guard, which
/// bounded the limit alone, the plain `+` wraps for a seed below the band (`sk(-4294967296, 0)` printed a positive
/// number), and that Rust, seeded back, is never proven.
#[test]
fn a_counter_seeded_from_an_input_is_proven_under_a_guard_on_the_seed_and_never_without_it() {
    let inv = holds(FROM_K, "sk");
    assert!(inv.iter().flatten().any(|c| c == "i² + k₀ = k₀² + 2·total + i"), "{inv:?}");
    let limit_alone = swap(
        "if (-2000000000_i64..=2000000000_i64).contains(&n) && (-2000000000_i64..=2000000000_i64).contains(&k) {",
        "if (-3037000499_i64..=3037000499_i64).contains(&n) {",
    );
    match validate_edited(FROM_K, limit_alone).unwrap().verdict("sk") {
        Some(Verdict::Refuted { .. } | Verdict::Unproven { .. }) => {}
        other => panic!("the fast loop's `total += i` wraps for a seed below -3037000499: {other:?}"),
    }
}

#[test]
fn a_memoized_recursion_is_unsupported_never_proven_or_refuted() {
    let report = validate(FIB).unwrap();
    assert!(matches!(report.verdict("fib"), Some(Verdict::Unsupported { .. })), "{report:?}");
}

#[test]
fn a_changed_constant_in_the_fast_path_is_refuted() {
    let (input, _, _) = refuted(POLY, "poly", swap("- 7i64", "- 8i64"));
    assert!(input.starts_with("x = "), "{input}");
}

#[test]
fn a_guard_too_wide_for_its_fast_path_is_refuted_with_a_wrapped_answer() {
    let (input, source, rust) = refuted(POLY, "poly", swap("2147483648_i64)", "4294967296_i64)"));
    let x: i128 = input.trim_start_matches("x = ").parse().unwrap();
    assert!(x.abs() > 1i128 << 31, "only an x the true guard excludes wraps: {input}");
    assert!(source.starts_with("returns ") && rust.starts_with("returns "), "a silent wrong answer, not a stop");
}

#[test]
fn a_checked_subtraction_made_wrapping_is_refuted_at_the_bottom_of_i64() {
    let (input, source, rust) = refuted(BOOLS, "pick", swap("logos_sub_i64(0, x)", "0 - x"));
    assert!(input.contains(&i64::MIN.to_string()), "{input}");
    assert_eq!(source, format!("returns {}", -(i64::MIN as i128)));
    assert_eq!(rust, format!("returns {}", i64::MIN));
}

#[test]
fn a_wrapping_product_in_a_comparison_is_never_proven() {
    let report = validate_edited(BREAK, swap("if (i as i128) * (i as i128) > i128::from(n) {", "if i * i > n {")).unwrap();
    match report.verdict("first_square_above") {
        Some(Verdict::Refuted { .. } | Verdict::Unproven { .. }) => {}
        other => panic!("`i * i` wraps past 3037000499, so the comparison is not the source's: {other:?}"),
    }
}

#[test]
fn a_value_shown_twice_is_refuted() {
    let (_, source, rust) = refuted(MAIN_LOOP, "## Main", swap("show(&{ i });", "show(&{ i }); show(&{ i });"));
    assert!(source.starts_with("shows 3, 6,") && rust.starts_with("shows 3, 3, 6, 6,"), "{source} / {rust}");
}

#[test]
fn swapped_remainder_operands_are_refuted() {
    refuted(GCD_LOOP, "gcd", swap("y = x % y;", "y = y % x;"));
}

#[test]
fn an_inverted_comparison_is_refuted() {
    refuted(SIGN, "sign", swap("if x > 0 {", "if x >= 0 {"));
}

#[test]
fn the_rust_executor_shows_what_the_tree_walker_shows() {
    let all = [POLY, SIGN, GCD_LOOP, GCD_REC, COLLATZ, BOOLS, SEVENS, SUM_RANGE, SUM_WHILE, SQUARES, CUBES, MAIN_SUM];
    for src in all.into_iter().chain([FROM_K, AFFINE, COUNT, SHOWN_SUM, MAIN_LOOP]) {
        assert_eq!(check_executor(src), ExecutorCheck::Agree, "{src}");
    }
}
