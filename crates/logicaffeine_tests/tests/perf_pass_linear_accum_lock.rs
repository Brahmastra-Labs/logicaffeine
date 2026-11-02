//! Pass — linear-accumulator plain-`+` elision (unblocks LLVM auto-vectorization).
//!
//! WHY. A hot loop `sum = sum + f(i)` over `i in 1..=n` currently emits the PROMOTING
//! `sum = logos_add_i64(sum, f(i))` — because `Int` is arbitrary-precision and codegen
//! can't prove the loop-carried `sum` stays i64 (the interval fixpoint widens it to ±∞).
//! That `logos_add_i64` is a FUNCTION CALL in the hot path, and a call blocks LLVM from
//! vectorizing the whole loop. The winning bit caps (reverse_bits/popcount) emit a PLAIN
//! `sum = (sum + …)` and auto-vectorize — measured 1.1–7.7× over C/Rust/Go. This pass
//! proves the accumulator stays i64 whenever the loop bound `n` fits a compile-time `B`
//! (`|sum| ≤ iterations × max|f| ≤ n·M`, fits i64 when `n·M ≤ i64::MAX`), and VERSIONS the
//! loop: `if n ≤ B { fast: plain + } else { original: promoting }`. The fast branch's plain
//! `+` then vectorizes. MEASURED (standalone): abs 0.41→1.10× vs C, divmod 0.82→1.04×.
//!
//! SOUNDNESS IS THE WHOLE GAME. Emitting a plain (wrapping) `+` where the sum actually
//! overflows i64 is a SILENT MISCOMPILE (wrong answer, not a promotion). The entry guard
//! `n ≤ B` MUST guarantee `Σ|f(i)|` cannot exceed i64::MAX, and a genuinely huge `n` takes
//! the ORIGINAL promoting branch. Because the bound is discharged by INTERVAL arithmetic
//! (which OVER-approximates), a pass that FIRES is sound — it can only fail to fire. The
//! shape lock (pass fired: a plain-`+` fast branch) is paired with a structural lock (the
//! promoting fallback branch still exists for huge `n`) and correctness locks across tiers
//! at small AND large-sum inputs on the fast path.
#![allow(clippy::needless_raw_string_hashes)]
mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted, compile_to_rust};

/// The `math.abs` parity bench shape: `sum += abs(i - n/2)` over `i in 1..=n`. `abs` is a
/// builtin, so this is self-contained. Today emits `logos_add_i64(sum, …)` + a per-iter
/// overflow guard on `i - n/2`; the pass must give it a plain-`+` fast branch.
const ABS_SUM: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable sum be 0.
Let mutable i be 1.
While i is at most n:
    Set sum to sum + abs(i - n / 2).
    Set i to i + 1.
Show sum.
"#;

/// All brace-matched loop bodies (`while … { … }` AND the `for … { … }` a counted While
/// lowers to), so the assertion sees the real hot loop in whichever form it emits.
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

// ---------------------------------------------------------------------------
// RED — the perf-shape spec (fails today, passes when the pass lands).
// ---------------------------------------------------------------------------

/// After the pass, the abs-sum loop has a FAST branch whose accumulation is a PLAIN i64
/// `+` (no `logos_add_i64` call) — the form LLVM vectorizes. Today every loop body carries
/// `logos_add_i64`, so this fails; the versioned fast branch makes it pass. Marker-
/// independent: only asks that SOME loop body accumulate with a plain `+`.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn abs_sum_has_a_plain_i64_accumulator_loop() {
    let rust = compile_to_rust(ABS_SUM).expect("abs-sum program compiles to Rust");
    let bodies = loop_bodies(&rust);
    assert!(
        !bodies.is_empty(),
        "abs-sum must emit a loop. emit:\n{rust}"
    );
    let has_plain = bodies.iter().any(|b| {
        b.contains("sum") && !b.contains("logos_add_i64") && !b.contains("logos_add_exact")
    });
    assert!(
        has_plain,
        "linear-accum pass: the abs-sum loop must have a plain-i64 `+` fast branch (no \
         logos_add_i64 call — that call blocks LLVM vectorization). Every loop body still \
         calls logos_add_i64. emit:\n{rust}"
    );
}

// ---------------------------------------------------------------------------
// SOUNDNESS PARTNERS — green today, MUST stay green when the pass lands.
// ---------------------------------------------------------------------------

/// The versioned shape keeps a PROMOTING fallback: SOME loop body must still carry the
/// arbitrary-precision `logos_add_i64` (the `else` branch a genuinely huge `n` takes). A
/// pass that dropped the fallback — making every branch a wrapping plain `+` — is UNSOUND
/// and reddens this. (Trivially green today: today ALL bodies are promoting.)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn abs_sum_keeps_a_promoting_fallback_loop() {
    let rust = compile_to_rust(ABS_SUM).expect("abs-sum program compiles to Rust");
    let bodies = loop_bodies(&rust);
    assert!(
        bodies
            .iter()
            .any(|b| b.contains("logos_add_i64") || b.contains("logos_add_exact")),
        "the promoting fallback loop (for huge n that overflows i64) must survive — a \
         wrapping-only lowering is a silent miscompile. emit:\n{rust}"
    );
}

/// Small input, correct across tiers (tree-walker ≡ VM ≡ AOT).
/// n=10: Σ_{i=1..10} |i - 5| = 4+3+2+1+0+1+2+3+4+5 = 25.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn abs_sum_small_is_correct_all_tiers() {
    let src = r#"## Main
Let n be 10.
Let mutable sum be 0.
Let mutable i be 1.
While i is at most n:
    Set sum to sum + abs(i - n / 2).
    Set i to i + 1.
Show sum.
"#;
    assert_all_tiers(src, "25");
    assert_compiled_equals_interpreted(src);
}

/// Larger fast-path input whose sum is well into the billions (still i64) — exercises the
/// FAST branch on a big accumulator and pins it exact vs the tree-walker. If the pass's
/// bound `B` were wrong (fast path taken when the sum can overflow), a bigger case like
/// this is where a wrapping `+` would start to disagree. n=100000 → Σ|i-50000| = 2_500_000_000.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn abs_sum_large_fastpath_is_exact() {
    let src = r#"## Main
Let n be 100000.
Let mutable sum be 0.
Let mutable i be 1.
While i is at most n:
    Set sum to sum + abs(i - n / 2).
    Set i to i + 1.
Show sum.
"#;
    assert_all_tiers(src, "2500000000");
    assert_compiled_equals_interpreted(src);
}
