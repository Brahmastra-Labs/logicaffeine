//! Comptime table-ization (`optimize::ctfe::tableize_stmts`, gated by
//! `Opt::Comptime`): a pure function called with arguments from a small,
//! statically-bounded domain (`factorial((i mod 12)+1)`, `binomial(30, i mod
//! 16)`, …) is replaced by a lookup into a table precomputed at compile time via
//! CTFE. LOGOS does a load where C recomputes — and the exact CTFE values keep
//! every tier byte-identical. Sound by construction: `eval_binop` bails on i64
//! overflow, so a value that would exceed i64 is never tabled.

mod common;
use common::{assert_all_tiers, compile_to_rust};

const FACTORIAL_BENCH: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## To factorial (n: Int) -> Int:
    Let mutable p be 1.
    Let mutable i be 1.
    While i is less than or equal to n:
        Set p to p * i.
        Set i to i + 1.
    Return p.
## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable sum be 0.
Let mutable i be 1.
While i is at most n:
    Set sum to sum + factorial((i % 12) + 1).
    Set i to i + 1.
Show sum.
"#;

/// A runtime `n` keeps the loop from being constant-folded, so the bounded-domain
/// `factorial((i % 12) + 1)` call must be replaced by a precomputed table — whose
/// last entry is `factorial(12) = 479001600`.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bounded_domain_call_is_tableized() {
    let rust = compile_to_rust(FACTORIAL_BENCH).unwrap();
    assert!(
        rust.contains("479001600"),
        "factorial((i%12)+1) must be replaced by a compile-time table containing \
         factorial(12) = 479001600. Emitted:\n{rust}"
    );
    // The recompute call must be gone from the loop (replaced by the lookup).
    let main = &rust[rust.find("fn _logos_main").unwrap_or(0)..];
    assert!(
        !main.contains("factorial("),
        "the runtime factorial call must be gone from the hot loop (replaced by a \
         table lookup). main:\n{main}"
    );
}

/// The table lookups produce the exact, correct sums on every tier.
/// Σ factorial((i mod 12)+1) for i in 1..=5 = 2+6+24+120+720 = 872.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn tableized_result_is_exact_on_all_tiers() {
    assert_all_tiers(
        r#"## To factorial (n: Int) -> Int:
    Let mutable p be 1.
    Let mutable i be 1.
    While i is less than or equal to n:
        Set p to p * i.
        Set i to i + 1.
    Return p.
## Main
Let mutable sum be 0.
Let mutable i be 1.
While i is at most 5:
    Set sum to sum + factorial((i % 12) + 1).
    Set i to i + 1.
Show sum.
"#,
        "872",
    );
    // A two-argument bounded call (both varying) — powExact-shaped.
    // Σ ((i mod 3)+2) ^ (i mod 3) for i in 1..=3 = 3^1 + 4^2 + 2^0 = 3+16+1 = 20.
    assert_all_tiers(
        r#"## To powi (base: Int) and (e: Int) -> Int:
    Let mutable acc be 1.
    Let mutable k be 0.
    While k is less than e:
        Set acc to acc * base.
        Set k to k + 1.
    Return acc.
## Main
Let mutable sum be 0.
Let mutable i be 1.
While i is at most 3:
    Set sum to sum + powi((i % 3) + 2, i % 3).
    Set i to i + 1.
Show sum.
"#,
        "20",
    );
}

/// A call whose argument is NOT statically bounded (a plain runtime parameter)
/// must be left alone — no table, the real call stays.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn unbounded_argument_is_not_tableized() {
    let code = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## To sq (n: Int) -> Int:
    Return n * n.
## Main
Let n be parseInt(item 2 of args()).
Show sq(n).
"#;
    let rust = compile_to_rust(code).unwrap();
    assert!(
        rust.contains("sq(") || rust.contains("(n * n)"),
        "an unbounded-argument call must not be tableized. Emitted:\n{rust}"
    );
}

/// A remainder takes its dividend's sign, so a call on `x % 6` with a negative `x` reads the function at a negative
/// point: the table covers every remainder the dividend can leave, and every tier prints the function's own values.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_negative_dividend_reads_the_table_at_its_own_remainder() {
    assert_all_tiers(
        r#"## To tri (k: Int) -> Int:
    If k is at most 0:
        Return k.
    Return k + tri(k - 1).

## Main
Let mutable x be 5.
Let mutable c be 0.
Let mutable i be 0.
While i is less than 4:
    Set x to x - 4.
    Set c to tri(x % 6).
    Show c.
    Set i to i + 1.
"#,
        "1\n-3\n-1\n-5",
    );
}
