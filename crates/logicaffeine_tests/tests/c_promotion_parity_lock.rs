//! ★ LOCK — the C tier keeps a machine word wherever the Rust tier does. ★
//!
//! LOGOS has one integer type; each compiled tier carries it as a machine word until a value can outgrow one. The
//! Rust tier proves a masked hash (`(h * 31 + v) & 268435455`) and a remainder LCG (`(x * 1103515245 + 12345) %
//! 2147483648`) stay in `i64`. The C tier promoted both to `LInt`, which left a tuple return type undeclared
//! (sieve), passed an `LInt` where a map wants an `int64_t` key (collect), and put arbitrary-precision arithmetic in
//! the hot loop of every program that hashes this way. The generated C must declare these as `int64_t`, and every
//! tier must print the same bytes.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test c_promotion_parity_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{assert_c_output, assert_exact_output};
use logicaffeine_compile::compile::{compile_to_c, compile_to_rust};

const MASKED_HASH: &str = "## To fold (limit: Int) -> (Int, Int):
    Let mutable count be 0.
    Let mutable h be 0.
    Let mutable i be 2.
    While i is at most limit:
        Let mutable v be 0.
        If i % 3 equals 0:
            Set v to 1.
            Set count to count + 1.
        Set h to (h * 31 + v) & 268435455.
        Set i to i + 1.
    Return (count, h).

## Main
Let (count, h) be fold(5000).
Show \"\" + count + \" \" + h.
";

const LCG_MAP: &str = "## Main
Let m be a new Map of Int to Int.
Let mutable seed be 42.
Let mutable i be 0.
While i is less than 2000:
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Set m at seed to seed * 2.
    Set i to i + 1.
Let mutable hit be 42.
Let mutable found be 0.
Set i to 0.
While i is less than 2000:
    Set hit to (hit * 1103515245 + 12345) % 2147483648.
    If m contains hit:
        Set found to found + 1.
    Set i to i + 1.
Show found.
";

/// Every name in `names` is declared `int64_t` in the generated C and never `LInt`.
fn c_keeps_words(src: &str, names: &[&str]) {
    let c = compile_to_c(src).unwrap_or_else(|e| panic!("compiles to C: {e:?}"));
    for n in names {
        assert!(
            !c.contains(&format!("LInt {n} ")),
            "`{n}` must stay a machine word in C, as in Rust:\n{c}"
        );
        assert!(
            c.contains(&format!("int64_t {n} ")),
            "`{n}` is declared int64_t in C:\n{c}"
        );
    }
    let rust = compile_to_rust(src).unwrap_or_else(|e| panic!("compiles to Rust: {e:?}"));
    assert!(
        !rust.contains("BigInt"),
        "the Rust tier keeps these as i64 too:\n{rust}"
    );
}

#[test]
fn a_masked_hash_stays_a_machine_word_in_c() {
    c_keeps_words(MASKED_HASH, &["h", "count"]);
    assert_exact_output(MASKED_HASH, "1666 33297504");
    assert_c_output(MASKED_HASH, "1666 33297504");
}

#[test]
fn a_remainder_lcg_keyed_map_stays_a_machine_word_in_c() {
    c_keeps_words(LCG_MAP, &["seed", "hit"]);
    assert_exact_output(LCG_MAP, "2000");
    assert_c_output(LCG_MAP, "2000");
}
