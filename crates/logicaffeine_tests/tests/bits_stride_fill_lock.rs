//! A loop that stores one value into a bit-packed `Seq of Bool` at a fixed stride stores it a word at a time.
//!
//! `While j is at most limit: Set item (j + 1) of flags to true. Set j to j + i.` sets one bit per pass, and each
//! set is an `or` into the word that holds it. A stride shorter than a word comes back to the same word before the
//! last `or` has landed, so a sieve's passes for its small primes ran one store-to-load chain per word. When the
//! Oracle proves every store in range and the stride at least 1, the generated Rust hands the whole progression to
//! `LogosBits::fill_stride_through_unchecked`, out of line, which builds each word's mask in a register and writes the
//! word once, then leaves the counter where the loop would have: the last value it stored at, plus the stride, by the
//! loop's own arithmetic. A loop the Oracle cannot prove keeps its stores and their checks.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test bits_stride_fill_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_c_output, assert_compiled_equals_interpreted, compile_to_rust, run_all_tiers};

const FILL: &str = ".fill_stride_through_unchecked(";

fn benchmark(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../benchmarks/programs/{name}/main.lg"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Every tier prints the same, compiled Rust prints the interpreter's output, and C prints it too.
fn answers_alike(source: &str) -> String {
    let agreed = run_all_tiers(source);
    assert!(agreed.error.is_none(), "{:?}\n{source}", agreed.error);
    assert_c_output(source, agreed.output.trim());
    assert_compiled_equals_interpreted(source);
    agreed.output.trim().to_string()
}

/// The sieve benchmark with its `## Main` replaced by `main`.
fn sieve_with_main(main: &str) -> String {
    let source = benchmark("sieve");
    let at = source.find("## Main").expect("the sieve has a Main");
    format!("{}## Main\n{main}", &source[..at])
}

#[test]
fn the_sieve_crosses_off_each_primes_multiples_a_word_at_a_time() {
    let rust = compile_to_rust(&benchmark("sieve")).unwrap();
    assert!(rust.contains(FILL), "the crossing-off loop fills by stride:\n{rust}");
    assert!(
        !rust.contains("set_true_unchecked"),
        "no crossing-off loop is left storing one bit per pass:\n{rust}"
    );
}

/// The prime counts are the known ones, and every tier agrees on the counts and the hash at sizes that end on, just
/// before and just after word and square boundaries.
#[test]
fn the_sieve_counts_the_primes_at_every_size() {
    let sizes = [
        0, 1, 2, 3, 4, 5, 9, 10, 48, 49, 50, 63, 64, 65, 100, 127, 128, 129, 1000, 4099, 65537,
    ];
    let main = format!(
        "Repeat for limit in [{}]:\n    Let (count, h) be sieve(limit).\n    Show \"\" + count + \" \" + h.\n",
        sizes.map(|n| n.to_string()).join(", ")
    );
    let source = sieve_with_main(&main);
    assert!(compile_to_rust(&source).unwrap().contains(FILL));
    let shown = answers_alike(&source);
    let counts: Vec<i64> = shown
        .lines()
        .map(|line| line.split(' ').next().unwrap().parse().unwrap())
        .collect();
    assert_eq!(
        counts,
        [0, 0, 1, 2, 2, 3, 4, 4, 15, 15, 15, 18, 18, 18, 25, 31, 31, 31, 168, 565, 6543],
        "{shown}"
    );
}

/// Strides shorter than, equal to and longer than a word; `at most` and `less than` bounds; stores of `true` and of
/// `false` over earlier fills; an index the counter offsets and one it does not; a stride the Oracle knows from a
/// guard; and loops that never run. Each counter's exit value is shown, and every tier agrees at every size.
const SHAPES: &str = "\
## To native parseInt (s: Text) -> Int

## To shapes (limit: Int, step: Int) -> Text:
    Let mutable flags be a new Seq of Bool.
    Let mutable i be 0.
    While i is at most limit:
        Push false to flags.
        Set i to i + 1.
    Let mutable a be 1.
    While a is at most limit:
        Set item (a + 1) of flags to true.
        Set a to a + 3.
    Let mutable b be 2.
    While b is less than limit:
        Set item b of flags to false.
        Set b to b + 7.
    Let mutable c be 4.
    While c is at most limit:
        Set item (c + 1) of flags to true.
        Set c to c + 64.
    Let mutable d be 9.
    While d is less than limit:
        Set item (d + 2) of flags to true.
        Set d to d + 65.
    Let mutable e be 0.
    If step is at least 1:
        While e is at most limit:
            Set item (e + 1) of flags to true.
            Set e to e + step.
    Let mutable f be limit + 5.
    While f is at most limit:
        Set item f of flags to true.
        Set f to f + 2.
    Let mutable count be 0.
    Let mutable h be 0.
    Let mutable p be 1.
    While p is at most limit + 1:
        If item p of flags equals true:
            Set count to count + 1.
            Set h to (h * 31 + p) % 1000000007.
        Set p to p + 1.
    Return \"\" + count + \" \" + h + \" \" + a + \" \" + b + \" \" + c + \" \" + d + \" \" + e + \" \" + f.

## Main
Repeat for n in [0, 1, 2, 3, 5, 9, 31, 62, 63, 64, 65, 66, 127, 128, 129, 191, 200, 1000, 4099]:
    Show shapes(n, parseInt(\"5\")).
    Show shapes(n, parseInt(\"1\")).
    Show shapes(n, parseInt(\"64\")).
    Show shapes(n, parseInt(\"0\")).
    Show shapes(n, parseInt(\"4611686018427387904\")).
";

#[test]
fn every_strided_shape_fills_by_stride() {
    let rust = compile_to_rust(SHAPES).unwrap();
    let fills = rust.matches(FILL).count();
    assert!(
        fills >= 5,
        "the five strided loops that run fill by stride ({fills} do):\n{rust}"
    );
    assert!(
        !rust.contains(".set_true_unchecked(") && !rust.contains(".set_unchecked("),
        "no strided loop is left storing one bit per pass:\n{rust}"
    );
}

#[test]
fn every_strided_shape_prints_alike_at_every_size() {
    let shown = answers_alike(SHAPES);
    assert_eq!(shown.lines().count(), 19 * 5, "{shown}");
}

/// A stride the Oracle knows only to be at least 0 could be 0, a loop that never ends: its stores are proven in range
/// (the counter never falls below its start), yet there is no fill, and the program still answers.
#[test]
fn an_unproven_stride_keeps_the_loop() {
    let source = "\
## To native parseInt (s: Text) -> Int

## To walk (limit: Int, step: Int) -> Int:
    Let mutable flags be a new Seq of Bool.
    Let mutable i be 0.
    While i is at most limit:
        Push false to flags.
        Set i to i + 1.
    Let mutable j be 1.
    If step is at least 0:
        While j is at most limit:
            Set item (j + 1) of flags to true.
            Set j to j + step.
    Let mutable count be 0.
    Let mutable p be 1.
    While p is at most limit + 1:
        If item p of flags equals true:
            Set count to count + 1.
        Set p to p + 1.
    Return count * 1000 + j.

## Main
Show walk(parseInt(\"100\"), parseInt(\"3\")).
";
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains(FILL), "{rust}");
    assert_eq!(answers_alike(source), "34103");
}

/// A store the Oracle cannot prove in range keeps its check, so the loop fails at the same item, in the
/// interpreter's words, after storing every item before it.
#[test]
fn a_stride_past_the_end_fails_where_every_tier_fails() {
    let source = "\
## To native parseInt (s: Text) -> Int

## To past (limit: Int) -> Int:
    Let mutable flags be a new Seq of Bool.
    Let mutable i be 0.
    While i is less than limit:
        Push false to flags.
        Set i to i + 1.
    Let mutable j be 1.
    While j is at most limit + 6:
        Set item j of flags to true.
        Set j to j + 3.
    Return j.

## Main
Show past(parseInt(\"10\")).
";
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains(FILL), "{rust}");
    let agreed = run_all_tiers(source);
    let want = agreed.error.expect("item 13 of a ten-item list fails");
    let compiled = common::run_logos(source);
    assert!(!compiled.success, "{}\n{}", compiled.stdout, compiled.rust_code);
    assert!(
        compiled.stderr.contains(&want),
        "the compiled failure {:?} is not the interpreter's {want:?}\n{}",
        compiled.stderr,
        compiled.rust_code
    );
}

/// A loop that does anything besides the store and the step is not a fill.
#[test]
fn a_loop_with_more_to_do_keeps_its_stores() {
    let source = "\
## To native parseInt (s: Text) -> Int

## To busy (limit: Int) -> Int:
    Let mutable flags be a new Seq of Bool.
    Let mutable i be 0.
    While i is at most limit:
        Push false to flags.
        Set i to i + 1.
    Let mutable marked be 0.
    Let mutable j be 1.
    While j is at most limit:
        Set item (j + 1) of flags to true.
        Set marked to marked + 1.
        Set j to j + 4.
    Return marked * 1000 + j.

## Main
Show busy(parseInt(\"100\")).
";
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains(FILL), "{rust}");
    assert_eq!(answers_alike(source), "25101");
}
