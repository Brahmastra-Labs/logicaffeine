//! `Opt::DivisorSieve`: a search, for each number of a counted range, for a divisor no greater than its square root
//! reads one sieve of the range.
//!
//! The primes benchmark asks every `i` up to `n` whether some `d` with `d * d <= i` divides it, `O(n sqrt n)`
//! divisions. The search finds one exactly when `i` has a divisor `d >= 2` with `d * d <= i`, which one sieve marks for
//! the whole range before the loop. The loop then reads a mark where it searched, on every spelling of the search, and
//! prints what it printed on every tier, sieved and unsieved. A search that reads its divisor afterwards, steps by more
//! than one, stops short of the square root, starts below 2, or does anything else keeps its loop, and a range that
//! ends past the sieve's cap runs as written.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_c_output, assert_compiled_equals_interpreted, compile_to_rust, run_all_tiers};
use logicaffeine_compile::optimization::decorate_source;

fn benchmark(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../benchmarks/programs/{name}/main.lg"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// primes with `n` in the source instead of on the command line.
fn primes_at(n: i64) -> String {
    benchmark("primes").replace(
        "Let n be parseInt(item 2 of arguments).",
        &format!("Let n be {n}."),
    )
}

/// The program's generated Rust, and whether a search became a read of the sieve.
fn sieved(source: &str) -> (bool, String) {
    let rust = compile_to_rust(source).unwrap_or_else(|e| panic!("{e:?}\n{source}"));
    (rust.contains("__divsieve"), rust)
}

/// Every tier prints the same, compiled Rust prints the interpreter's output, C prints it too, and so does the
/// program with the sieve switched off.
fn answers_alike(source: &str) {
    let agreed = run_all_tiers(source);
    assert!(agreed.error.is_none(), "{:?}\n{source}", agreed.error);
    assert_c_output(source, agreed.output.trim());
    assert_compiled_equals_interpreted(source);
    let off = decorate_source(source, &["divisorsieve"]);
    assert_eq!(
        run_all_tiers(&off).output,
        agreed.output,
        "sieved and unsieved print alike:\n{source}"
    );
}

#[test]
fn the_primes_benchmark_reads_a_sieve() {
    let (yes, rust) = sieved(&benchmark("primes"));
    assert!(yes, "primes' search is one read of a sieve. Got:\n{rust}");
}

#[test]
fn turning_the_optimization_off_keeps_the_search() {
    let (yes, rust) = sieved(&decorate_source(&benchmark("primes"), &["divisorsieve"]));
    assert!(!yes, "Got:\n{rust}");
}

#[test]
fn the_primes_benchmark_counts_alike_at_every_size() {
    for n in [0, 1, 2, 3, 4, 5, 9, 10, 25, 26, 100, 1000, 10007] {
        answers_alike(&primes_at(n));
    }
}

/// A `Main` over `i` from `from` through `to`, a search for a divisor from `first`, and what it shows per `i`.
fn search(from: i64, to: i64, cond: &str, test: &str, first: i64) -> String {
    format!(
        "## Main\nLet mutable found be 0.\nLet mutable shown be \"\".\nLet mutable i be {from}.\nWhile i is at most {to}:\n    Let mutable flag be false.\n    Let d be {first}.\n    While {cond}:\n        If {test}:\n            Set flag to true.\n            Break.\n        Set d to d + 1.\n    If flag:\n        Set found to found + 1.\n        Set shown to shown + i + \" \".\n    Set i to i + 1.\nShow found.\nShow shown.\n"
    )
}

/// Spellings of the search that read the sieve.
fn sieving_searches() -> Vec<(&'static str, String)> {
    vec![
        ("d * d <= i", search(0, 200, "d * d is at most i", "i % d equals 0", 2)),
        ("d <= i / d", search(0, 200, "d is at most i / d", "i % d equals 0", 2)),
        ("zero on the left", search(0, 200, "d * d is at most i", "0 equals i % d", 2)),
        ("divisors from 3", search(0, 200, "d * d is at most i", "i % d equals 0", 3)),
        ("divisors from 7", search(0, 400, "d * d is at most i", "i % d equals 0", 7)),
        ("a range from below zero", search(-30, 60, "d * d is at most i", "i % d equals 0", 2)),
        (
            "a Repeat loop",
            "## Main\nLet mutable found be 0.\nRepeat for i from 2 to 300:\n    Let mutable isPrime be 1.\n    Let d be 2.\n    While d * d is at most i:\n        If i % d equals 0:\n            Set isPrime to 0.\n            Break.\n        Set d to d + 1.\n    If isPrime equals 1:\n        Set found to found + 1.\nShow found.\n".to_string(),
        ),
        (
            "an exclusive end and two Sets",
            "## Main\nLet mutable composites be 0.\nLet mutable last be 0.\nLet mutable i be 2.\nWhile i is less than 300:\n    Let mutable hit be 0.\n    Let d be 2.\n    While d * d is at most i:\n        If i % d equals 0:\n            Set hit to 1.\n            Set last to i.\n            Break.\n        Set d to d + 1.\n    Set composites to composites + hit.\n    Set i to i + 1.\nShow composites.\nShow last.\n".to_string(),
        ),
        (
            "inside a function",
            "## To countPrimes (n: Int) -> Int:\n    Let mutable count be 0.\n    Let mutable i be 2.\n    While i is at most n:\n        Let mutable isPrime be true.\n        Let d be 2.\n        While d * d is at most i:\n            If i % d equals 0:\n                Set isPrime to false.\n                Break.\n            Set d to d + 1.\n        If isPrime:\n            Set count to count + 1.\n        Set i to i + 1.\n    Return count.\n\n## Main\nShow countPrimes(1000).\nShow countPrimes(1).\nShow countPrimes(2).\nShow countPrimes(0 - 5).\n".to_string(),
        ),
    ]
}

#[test]
fn every_spelling_of_the_search_reads_the_sieve_and_prints_alike() {
    for (why, program) in sieving_searches() {
        let (yes, rust) = sieved(&program);
        assert!(yes, "{why}: one read of a sieve. Got:\n{rust}");
        answers_alike(&program);
    }
}

/// Searches outside the shape: each keeps its loop and prints alike.
fn other_searches() -> Vec<(&'static str, String)> {
    let body = |inner: &str, after: &str| {
        format!(
            "## Main\nLet mutable found be 0.\nLet mutable i be 2.\nWhile i is at most 200:\n    Let mutable flag be 0.\n    Let d be 2.\n{inner}{after}    Set found to found + flag.\n    Set i to i + 1.\nShow found.\n"
        )
    };
    let standard = "    While d * d is at most i:\n        If i % d equals 0:\n            Set flag to 1.\n            Break.\n        Set d to d + 1.\n";
    vec![
        ("the divisor read afterwards", body(standard, "    Set found to found + d.\n")),
        (
            "the divisor kept on a find",
            body("    While d * d is at most i:\n        If i % d equals 0:\n            Set flag to d.\n            Break.\n        Set d to d + 1.\n", ""),
        ),
        (
            "a step of two",
            body("    While d * d is at most i:\n        If i % d equals 0:\n            Set flag to 1.\n            Break.\n        Set d to d + 2.\n", ""),
        ),
        (
            "a bound short of the square root",
            body("    While d * d is less than i:\n        If i % d equals 0:\n            Set flag to 1.\n            Break.\n        Set d to d + 1.\n", ""),
        ),
        (
            "a count of the divisions",
            body("    While d * d is at most i:\n        Set found to found + 1.\n        If i % d equals 0:\n            Set flag to 1.\n            Break.\n        Set d to d + 1.\n", ""),
        ),
        (
            "an otherwise",
            body("    While d * d is at most i:\n        If i % d equals 0:\n            Set flag to 1.\n            Break.\n        Otherwise:\n            Set found to found + 0.\n        Set d to d + 1.\n", ""),
        ),
        (
            "no break",
            body("    While d * d is at most i:\n        If i % d equals 0:\n            Set flag to 1.\n        Set d to d + 1.\n", ""),
        ),
        (
            "divisors from 1",
            body(standard, "").replace("    Let d be 2.\n", "    Let d be 1.\n"),
        ),
        (
            "a statement between the start and the search",
            body(standard, "").replace("    Let d be 2.\n", "    Let d be 2.\n    Set found to found + 0.\n"),
        ),
    ]
}

#[test]
fn a_search_outside_the_shape_keeps_its_loop_and_prints_alike() {
    for (why, program) in other_searches() {
        let (yes, rust) = sieved(&program);
        assert!(!yes, "{why}: must keep its loop. Got:\n{rust}");
        let agreed = run_all_tiers(&program);
        assert!(agreed.error.is_none(), "{why}: {:?}", agreed.error);
        assert_compiled_equals_interpreted(&program);
    }
}

/// A range that ends past the sieve's cap (2^28) is rewritten behind a guard that is false, which constant
/// propagation folds away, and runs as written.
#[test]
fn a_range_past_the_cap_runs_as_written() {
    let program = search(
        268_435_440,
        268_435_470,
        "d * d is at most i",
        "i % d equals 0",
        2,
    );
    assert!(
        logicaffeine_compile::compile::optimizations_fired(&program).contains(&"divisorsieve"),
        "the search is rewritten behind its guard"
    );
    assert!(
        !sieved(&program).0,
        "the false guard leaves no sieve to build"
    );
    answers_alike(&program);
}
