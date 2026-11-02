//! Pass extension — MULTI-TERM (chain) linear accumulator → plain-`+` vectorization.
//!
//! `try_version_linear` (accum_bound.rs) unblocks vectorization for a single-term accumulator
//! `sum = sum + f(i)`. Real numeric loops often accumulate a SUM OF TERMS: `sum = sum + a(i) +
//! b(i)`, which parses left-associatively as `(sum + a(i)) + b(i)` — the current pattern match
//! (sum must be a direct operand of the top `+`) misses it, so it stays a promoting
//! `logos_add_i64` chain that blocks LLVM vectorization. This extends the match to peel the
//! accumulator out of an Add-chain and bound the increment `Σ terms` (each term bounded, summed),
//! versioning to a plain-`+` fast branch. General value: any integer sum-of-terms loop vectorizes.
//!
//! SOUNDNESS: identical discipline to the single-term case — the increment is bounded (`Σ|term_k|
//! ≤ Σ M_k`), the running sum by `iterations × Σ M_k`, and `(B+1)·ΣM ≤ i64::MAX` is required under
//! the entry guard; a huge `n` takes the promoting fallback. Over-approximating intervals ⇒ firing
//! is sound. The fallback (promoting) branch must survive.
#![allow(clippy::needless_raw_string_hashes)]
mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted, compile_to_rust};

/// Two-term integer accumulator: `sum += (i mod 5) + (i mod 7)`, pure arithmetic (no calls).
const CHAIN_SUM: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable sum be 0.
Let mutable i be 1.
While i is at most n:
    Set sum to sum + (i mod 5) + (i mod 7).
    Set i to i + 1.
Show sum.
"#;

fn loop_bodies(src: &str) -> Vec<String> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    for kw in ["while", "for "] {
        let mut search = 0usize;
        while let Some(rel) = src[search..].find(kw) {
            let w = search + rel;
            let Some(brace_rel) = src[w..].find('{') else {
                break;
            };
            let open = w + brace_rel;
            let mut depth = 0i32;
            let mut i = open;
            let mut close = None;
            while i < bytes.len() {
                match bytes[i] {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            match close {
                Some(c) => {
                    out.push(src[open + 1..c].to_string());
                    search = c + 1;
                }
                None => break,
            }
        }
    }
    out
}

/// RED: after the extension, the two-term sum loop has a plain-`+` fast branch (no
/// `logos_add_i64`). Today the chain is not recognised, so every body promotes.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn chain_sum_has_a_plain_i64_accumulator_loop() {
    let rust = compile_to_rust(CHAIN_SUM).expect("chain-sum compiles to Rust");
    let bodies = loop_bodies(&rust);
    assert!(
        !bodies.is_empty(),
        "chain-sum must emit a loop. emit:\n{rust}"
    );
    let has_plain = bodies.iter().any(|b| {
        b.contains("sum") && !b.contains("logos_add_i64") && !b.contains("logos_add_exact")
    });
    assert!(
        has_plain,
        "chain-accum extension: `sum += a(i) + b(i)` must get a plain-i64 `+` fast branch \
         (no logos_add_i64 — it blocks vectorization). Every loop body still promotes. emit:\n{rust}"
    );
}

/// SOUNDNESS PARTNER: the promoting fallback must survive.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn chain_sum_keeps_a_promoting_fallback_loop() {
    let rust = compile_to_rust(CHAIN_SUM).expect("chain-sum compiles to Rust");
    assert!(
        loop_bodies(&rust)
            .iter()
            .any(|b| b.contains("logos_add_i64") || b.contains("logos_add_exact")),
        "the promoting fallback loop must survive for huge n. emit:\n{rust}"
    );
}

/// Correctness across tiers. n=10: Σ_{i=1..10} (i%5 + i%7) =
/// (1+2+3+4+0+1+2+3+4+0) + (1+2+3+4+5+6+0+1+2+3) = 20 + 27 = 47.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn chain_sum_small_is_correct_all_tiers() {
    let src = r#"## Main
Let n be 10.
Let mutable sum be 0.
Let mutable i be 1.
While i is at most n:
    Set sum to sum + (i mod 5) + (i mod 7).
    Set i to i + 1.
Show sum.
"#;
    assert_all_tiers(src, "47");
    assert_compiled_equals_interpreted(src);
}
