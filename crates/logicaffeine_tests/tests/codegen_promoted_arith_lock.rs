//! Hot-path invariants for overflow-promoting exact-`Int` arithmetic (the
//! `LogosInt` accumulators). Ground truth from the emitted AOT-Rust drove these:
//!
//!  1. A *constant-bound* accumulator loop is fully evaluated at compile time —
//!     `Comptime`/`ClosedForm` collapse it to a literal, so there is no runtime
//!     accumulator at all. (No perf tax exists to remove here.)
//!  2. A *genuinely overflowing* accumulator MUST keep exact promotion and print
//!     the full bignum — the soundness floor: representation-narrowing may never
//!     wrap or trap a value the tree-walker would carry exactly.
//!  3. THE LEVER: a *runtime-bound* promoted accumulator (`factorial`) pays a
//!     per-iteration tax that is pure overhead — the accumulator is `.clone()`d
//!     on a read that immediately reassigns it (a dead last-use that must MOVE),
//!     and the exact product is re-wrapped `LogosInt::from(logos_mul_exact(..))`
//!     even though `logos_mul_exact` already returns a `LogosInt` (an identity
//!     wrap). Neither changes the result — both are removable, and removing them
//!     is what closes the gap to C's raw loop while staying exact.

mod common;
use common::{assert_exact_output, compile_to_rust};

/// Slice the emitted `fn factorial …` body out of a full program emit so the
/// hot-path assertions do not accidentally match clones in `main` (e.g. the
/// `argv` read).
fn factorial_fn(rust: &str) -> String {
    let start = rust.find("fn factorial").expect("factorial fn present");
    let rest = &rust[start..];
    // The function ends at the first `\n}` that closes it (bodies here are flat).
    let end = rest.find("\n}").map(|e| e + 2).unwrap_or(rest.len());
    rest[..end].to_string()
}

const FACTORIAL_PROGRAM: &str = r#"## To native args () -> Seq of Text
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
Show factorial(n).
"#;

// 1. Constant-bound accumulator folds to a literal (already optimal).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn constant_bound_accumulator_is_comptime_folded() {
    let code = r#"## Main
Let mutable p be 1.
Let mutable i be 0.
While i is less than 10:
    Set p to p * 2.
    Set i to i + 1.
Show p.
"#;
    let rust = compile_to_rust(code).unwrap();
    assert!(
        rust.contains("1024") && !rust.contains("logos_mul_exact"),
        "a constant-trip doubling loop must comptime-fold to the literal 1024, \
         with no residual accumulator arithmetic. Emitted:\n{rust}"
    );
}

// 2. Soundness floor: a genuinely overflowing accumulator stays exact.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn overflowing_accumulator_stays_exact() {
    assert_exact_output(
        r#"## Main
Let mutable x be 1.
Let mutable i be 0.
While i is less than 80:
    Set x to x * 2.
    Set i to i + 1.
Show x.
"#,
        "1208925819614629174706176",
    );
}

// 3. THE LEVER: the runtime-bound promoted loop must move, not clone, and must
//    not re-wrap the already-`LogosInt` product.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn promoted_accumulator_loop_moves_not_clones() {
    let rust = compile_to_rust(FACTORIAL_PROGRAM).unwrap();
    let f = factorial_fn(&rust);
    // The genuine per-iteration tax: cloning the accumulator on a read that
    // immediately reassigns it (and again at the dead-last-use return). The residual
    // `LogosInt::from(logos_mul_exact(..))` is the *reflexive* `From<LogosInt>` —
    // an identity the compiler elides — so it is intentionally NOT asserted here.
    assert!(
        !f.contains(".clone()"),
        "the promoted accumulator is read then immediately reassigned (and moved \
         at return) — every such dead last-use must MOVE, not `.clone()`. \
         factorial emit:\n{f}"
    );
}

// 3b. The lever must not break exactness: factorial of a large runtime arg still
//     produces the exact bignum (proves the move/unwrap kept promotion intact).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn promoted_factorial_still_exact_for_large_arg() {
    // 25! = 15511210043330985984000000 overflows i64 — must stay exact.
    assert_exact_output(
        r#"## To factorial (n: Int) -> Int:
    Let mutable p be 1.
    Let mutable i be 1.
    While i is less than or equal to n:
        Set p to p * i.
        Set i to i + 1.
    Return p.
## Main
Show factorial(25).
"#,
        "15511210043330985984000000",
    );
}
