//! A capacity hint changes where a sequence's memory comes from, and nothing else.
//!
//! `Let dp be a new Seq of Int with capacity n + 1.` says how many items `dp` will hold; it does not change what `dp`
//! is. Every analysis that decides how a sequence is emitted — its element width, whether it lives in a plain `Vec`,
//! which reads keep their bounds checks, whether a fill loop becomes one allocation — must reach the same decision
//! for the hinted form as for the unhinted one. Before, the hint hid the sequence from them: coins' `dp` went from a
//! narrowed `Vec<i32>` with unchecked reads to a checked `Vec<i64>`, and the hinted program ran 1.5× slower through
//! Rust than the one without the hint — a program penalised for saying more about itself.
//!
//! Each shape here is compiled twice, with and without the hint: the optimizations that fire are the same, and so are
//! the element types the emitted Rust declares; both forms print the same thing on every tier.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test with_capacity_transparency_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::collections::BTreeSet;

use common::{assert_all_tiers, assert_exact_output};
use logicaffeine_compile::compile::{compile_to_rust, optimizations_fired};

const HEADER: &str = "## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
";

/// A coins-shaped dynamic program: a fill loop, then in-place updates reading earlier items.
fn coins(hint: &str) -> String {
    format!(
        "{HEADER}
## Main
Let n be 200.
Let mutable dp be a new Seq of Int{hint}.
Push 1 to dp.
Let mutable i be 1.
While i is at most n:
    Push 0 to dp.
    Set i to i + 1.
Let mutable j be 5.
While j is at most n:
    Set item (j + 1) of dp to (item (j + 1) of dp + item (j - 4) of dp) % 1000000007.
    Set j to j + 1.
Let mutable h be 0.
Set i to 1.
While i is at most n + 1:
    Set h to (h * 31 + item i of dp) & 268435455.
    Set i to i + 1.
Show h.
"
    )
}

/// A histogram-shaped table: a fixed-size zero fill, then counted increments at computed indices.
fn histogram(hint: &str) -> String {
    format!(
        "{HEADER}
## Main
Let mutable counts be a new Seq of Int{hint}.
Let mutable idx be 0.
While idx is less than 100:
    Push 0 to counts.
    Set idx to idx + 1.
Let mutable seed be 42.
Let mutable i be 0.
While i is less than 5000:
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Let v be ((seed / 65536) % 32768) % 100.
    Set item (v + 1) of counts to (item (v + 1) of counts) + 1.
    Set i to i + 1.
Let mutable h be 0.
Set i to 1.
While i is at most 100:
    Set h to (h * 31 + item i of counts) & 268435455.
    Set i to i + 1.
Show h.
"
    )
}

/// The element types of the sequences the emitted Rust declares, in order: `Vec<i32>`, `LogosSeq<i64>`, `[i64; 100]`.
fn declared_sequences(rust: &str) -> Vec<String> {
    rust.lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l
                .strip_prefix("let mut ")
                .or_else(|| l.strip_prefix("let "))?;
            let (_, ty) = rest.split_once(':')?;
            let ty = ty.split('=').next()?.trim();
            (ty.starts_with("Vec<") || ty.starts_with("LogosSeq<") || ty.starts_with('['))
                .then(|| ty.to_string())
        })
        .collect()
}

fn same_decisions(plain: &str, hinted: &str) {
    let fired =
        |src: &str| -> BTreeSet<&'static str> { optimizations_fired(src).into_iter().collect() };
    assert_eq!(
        fired(hinted),
        fired(plain),
        "a capacity hint changed which optimizations fire"
    );
    let rust_plain = compile_to_rust(plain).expect("compiles");
    let rust_hinted = compile_to_rust(hinted).expect("compiles");
    assert_eq!(
        declared_sequences(&rust_hinted),
        declared_sequences(&rust_plain),
        "a capacity hint changed a sequence's representation\n--- unhinted:\n{rust_plain}\n--- hinted:\n{rust_hinted}"
    );
}

#[test]
fn a_hinted_dynamic_programming_table_is_emitted_as_the_unhinted_one() {
    let (plain, hinted) = (coins(""), coins(" with capacity n + 1"));
    same_decisions(&plain, &hinted);
    let expected = common::run_interpreter(&plain).output;
    for src in [&plain, &hinted] {
        assert_all_tiers(src, expected.trim());
        assert_exact_output(src, expected.trim());
    }
}

#[test]
fn a_hinted_counting_table_is_emitted_as_the_unhinted_one() {
    let (plain, hinted) = (histogram(""), histogram(" with capacity 100"));
    same_decisions(&plain, &hinted);
    let expected = common::run_interpreter(&plain).output;
    for src in [&plain, &hinted] {
        assert_all_tiers(src, expected.trim());
        assert_exact_output(src, expected.trim());
    }
}
