//! ★ LOCK — a linear accumulator runs on raw `i64` only where its guard proves the whole run fits a word. ★
//!
//! The bounded-accumulator pass (`optimize::accum_bound`) versions `While i < limit: Set sum to sum + terms. Set i to
//! i + 1.` into a fast loop of plain `i64` `+` behind a guard on its inputs, and the loop as written for every input
//! the guard turns away. The fast loop is sound only if every input the guard admits keeps the sum inside the word.
//! These rows read the guard of the emitted Rust (`(-B..=B).contains(&x)` for each name `x` it bounds) and evaluate
//! the loop's sum in closed form at every corner of the box it admits: a name it does not bound may be any `Int`.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test accum_bound_guard_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::time::Duration;

use common::RustProfile;
use logicaffeine_compile::compile::{compile_program_full, compile_to_rust};
use logicaffeine_compile::optimization::decorate_source;

/// Every `(-B..=B).contains(&name)` in `code`: the name and its band `B`.
fn guard_bands(code: &str) -> Vec<(String, i128)> {
    let mut bands = Vec::new();
    let mut rest = code;
    while let Some(at) = rest.find("..=") {
        let before = &rest[..at];
        let after = &rest[at + 3..];
        rest = after;
        let number = |text: &str| text.trim_end_matches("_i64").parse::<i128>().ok();
        let Some(open) = before.rfind("(-") else { continue };
        let Some(lo) = number(&before[open + 2..]) else { continue };
        let Some(close) = after.find(").contains(&") else { continue };
        let Some(hi) = number(&after[..close]) else { continue };
        let name: String = after[close + ").contains(&".len()..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if lo == hi && !name.is_empty() {
            bands.push((name, hi));
        }
    }
    bands
}

/// The Rust of the program's function `name`.
fn function<'c>(rust: &'c str, name: &str) -> &'c str {
    let start = rust.find(&format!("fn {name}(")).unwrap_or_else(|| panic!("no `fn {name}` in:\n{rust}"));
    let end = rust[start..].find("\n}\n").map_or(rust.len(), |e| start + e + 3);
    &rust[start..end]
}

/// The values a name takes at the corners of the box a guard admits: its band's ends and the two points where the sum
/// of a run of integers turns, or the word's own ends for a name the guard leaves free.
fn corners(bands: &[(String, i128)], name: &str) -> Vec<i128> {
    let b = bands.iter().find(|(n, _)| n == name).map_or(i64::MAX as i128, |(_, b)| *b);
    vec![-b, -1, 0, 1, b]
}

/// `start + (start + 1) + … + (end - 1)`: the sum the loop leaves from a counter at `start` run up to `end`, exact.
fn run_sum(start: i128, end: i128) -> i128 {
    if end <= start {
        return 0;
    }
    let tri = |x: i128| x * (x - 1) / 2;
    tri(end) - tri(start)
}

/// Every corner of the box the guard in `code` admits where the loop's sum from `start(corner)` to `end(corner)`
/// leaves the word; none for a function that is not versioned.
fn escapes(code: &str, names: &[&str], start: impl Fn(&[i128]) -> i128, end: impl Fn(&[i128]) -> i128) -> Vec<String> {
    let bands = guard_bands(code);
    if bands.is_empty() {
        return Vec::new();
    }
    let axes: Vec<Vec<i128>> = names.iter().map(|n| corners(&bands, n)).collect();
    let mut found = Vec::new();
    let mut at = vec![0usize; axes.len()];
    loop {
        let point: Vec<i128> = at.iter().zip(&axes).map(|(i, a)| a[*i]).collect();
        let sum = run_sum(start(&point), end(&point));
        if sum > i64::MAX as i128 || sum < i64::MIN as i128 {
            found.push(format!("{names:?} = {point:?} sums to {sum}"));
        }
        let mut k = 0;
        while k < at.len() {
            at[k] += 1;
            if at[k] < axes[k].len() {
                break;
            }
            at[k] = 0;
            k += 1;
        }
        if k == at.len() {
            break;
        }
    }
    found
}

const FROM_AN_INPUT: &str = "## To sk (k: Int, n: Int) -> Int:\n    Let mutable total be 0.\n    Let mutable i be k.\n    While i is less than n:\n        Set total to total + i.\n        Set i to i + 1.\n    Return total.\n\n## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n## Main\nLet arguments be args().\nShow sk(parseInt(item 2 of arguments), parseInt(item 3 of arguments)).\n";

/// A counter that starts at an input reaches the fast loop only where the guard bounds that start: at every corner of
/// the box the guard admits, the sum fits the word.
#[test]
fn a_counter_that_starts_at_an_input_is_fast_only_where_its_start_is_bounded() {
    let rust = compile_to_rust(FROM_AN_INPUT).expect("Rust for the program");
    let sk = function(&rust, "sk");
    // The counter's start is `k`, read into `i` before the loop: a band on either bounds it.
    let bands = guard_bands(sk);
    let start_band = bands.iter().find(|(n, _)| n == "k" || n == "i").map(|(_, b)| *b);
    let mut named = bands.clone();
    if let Some(b) = start_band {
        named.push(("start".to_string(), b));
    }
    let escaping = if bands.is_empty() {
        Vec::new()
    } else {
        let named_code: String =
            named.iter().map(|(n, b)| format!("(-{b}..={b}).contains(&{n}) ")).collect();
        escapes(&named_code, &["start", "n"], |p| p[0], |p| p[1])
    };
    assert!(escaping.is_empty(), "the fast loop runs past the word where its guard admits:\n{}\n{sk}", escaping.join("\n"));
}

/// A limit that reaches past the band takes the counter with it: the fast loop runs only where the sum to that limit
/// fits the word.
#[test]
fn a_limit_past_the_band_is_fast_only_where_the_run_to_it_fits() {
    let src = "## To twice (n: Int) -> Int:\n    Let mutable total be 0.\n    Let mutable i be 0.\n    While i is less than n + n:\n        Set total to total + i.\n        Set i to i + 1.\n    Return total.\n\n## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n## Main\nLet arguments be args().\nShow twice(parseInt(item 2 of arguments)).\n";
    let rust = compile_to_rust(&decorate_source(src, &["closedform"])).expect("Rust for the program");
    let twice = function(&rust, "twice");
    let escaping = escapes(twice, &["n"], |_| 0, |p| 2 * p[0]);
    assert!(escaping.is_empty(), "the fast loop runs past the word where its guard admits:\n{}\n{twice}", escaping.join("\n"));
}

/// The common shape — a counter from a literal, an input limit — keeps the guard it had, so its fast loop is as fast
/// and as often taken: the term's input and the limit each within 2e9, the counter not bounded.
#[test]
fn a_counter_from_a_literal_keeps_its_guard() {
    let src = "## To spread (x: Int, n: Int) -> Int:\n    Let mutable total be 0.\n    Let mutable i be 1.\n    While i is at most n:\n        Set total to total + abs(x - i).\n        Set i to i + 1.\n    Return total.\n\n## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n## Main\nLet arguments be args().\nShow spread(parseInt(item 2 of arguments), parseInt(item 3 of arguments)).\n";
    let rust = compile_to_rust(&decorate_source(src, &["closedform"])).expect("Rust for the program");
    let spread = function(&rust, "spread");
    assert_eq!(
        guard_bands(spread),
        [("x".to_string(), 2_000_000_000), ("n".to_string(), 2_000_000_000)],
        "the guard of the common shape:\n{spread}"
    );
    assert!(spread.contains("black_box"), "the common shape is versioned onto its fast loop:\n{spread}");
}

/// The receipt: a counter started at -2^32 and run to 0 sums to -9223372039002259456, past the word. AOT Rust prints
/// that or stops with the overflow sentence; it never prints a wrapped number.
#[test]
fn a_counter_started_below_zero_never_wraps_on_aot_rust() {
    let out = compile_program_full(FROM_AN_INPUT).expect("Rust for the program");
    let built = common::build_generated(&out.rust_code, &out.dependencies, RustProfile::Release)
        .unwrap_or_else(|why| panic!("the Rust does not build:\n{why}"));
    let run = common::run_within(&built.binary, &["-4294967296", "0"], Duration::from_secs(300));
    assert!(!run.timed_out, "still running after 300 s");
    let exact = run_sum(-4_294_967_296, 0);
    assert_eq!(exact, -9_223_372_039_002_259_456);
    let printed_exact = run.success && run.stdout.trim() == exact.to_string();
    let stopped = !run.success && run.stdout.trim().is_empty() && run.stderr.contains("Integer overflow");
    assert!(
        printed_exact || stopped,
        "AOT Rust printed {:?} (stderr {:?}); the sum is {exact}",
        run.stdout,
        run.stderr
    );
}

/// The reader and the corner check are checks that can fail: a band is read, an unbounded start escapes.
#[test]
fn the_guard_reader_and_the_corner_check_can_fail() {
    let code = "if (-3037000499_i64..=3037000499_i64).contains(&n) && (-2000000000..=2000000000).contains(&i) {";
    assert_eq!(guard_bands(code), [("n".to_string(), 3_037_000_499), ("i".to_string(), 2_000_000_000)]);
    assert!(!escapes("(-3037000499..=3037000499).contains(&n)", &["start", "n"], |p| p[0], |p| p[1]).is_empty());
    assert!(escapes("(-2000000000..=2000000000).contains(&start) (-2000000000..=2000000000).contains(&n)", &["start", "n"], |p| p[0], |p| p[1]).is_empty());
}
