//! Pass 1 — bounded-accumulator overflow-guard elision (the highest-leverage
//! integer-perf lever). RED-first spec for making a hot integer loop whose
//! intermediates provably fit i64 (given the inputs fit i64) lower to BARE i64
//! arithmetic, instead of paying a per-iteration overflow-guard branch.
//!
//! WHY THIS EXISTS. `Int` is arbitrary-precision, so codegen wraps every add/mul
//! that *could* exceed i64 in `{ if (operands in ±2^k) { raw i64 } else { promote
//! to BigInt } }` (`codegen/expr.rs::try_guarded_dual_chain`). For `isqrt`'s Newton
//! loop `y = (x + n/x)/2` the guard re-checks the loop-invariant `n` EVERY
//! iteration, and hand-C/Rust pay none of this — that is the measured ~9% loss to
//! Rust. The pass proves the loop stays i64-safe once the input fits a compile-time
//! bound `B`, versions the loop (`if input ≤ B { fast raw body } else { original
//! promoting body }`), and marks the fast branch's op nodes proven-raw so codegen
//! emits bare i64. Model: `optimize/closed_form.rs::try_defer_modulus`.
//!
//! SOUNDNESS IS THE WHOLE GAME. Eliding a guard that is actually needed is a silent
//! miscompile on huge inputs, NOT a slowdown. So the shape lock (this pass fired)
//! is paired with adversarial fallback locks: a huge-but-i64 input must still
//! produce the exact value the tree-walker does (the fast path must NOT be taken,
//! or must itself be proven safe). The correctness partners here are GREEN today and
//! MUST stay green when the pass lands — they are the safety net, not the RED spec.

#![allow(clippy::needless_raw_string_hashes)]
mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted, compile_to_rust};

/// The stdlib `isqrt` (integer square root, Newton's method) — the flagship
/// close-loss cap (loses ~9% to hand-Rust purely on the per-iteration guard).
/// Defined inline (not auto-imported) so the emit is self-contained and stable.
const ISQRT_PROGRAM: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## To isqrt (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Let mutable x be n.
    Let mutable y be (x + 1) / 2.
    While y is less than x:
        Set x to y.
        Set y to (x + (n / x)) / 2.
    Return x.
## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Show isqrt(n).
"#;

/// Slice the emitted `fn isqrt …` body out of a full program emit so assertions
/// do not match arithmetic in `main`/other fns. Bodies here close at the first
/// top-level `\n}` (mirrors `codegen_promoted_arith_lock::factorial_fn`).
fn isqrt_fn(rust: &str) -> String {
    let start = rust.find("fn isqrt").expect("isqrt fn present in emit");
    let rest = &rust[start..];
    let end = rest.find("\n}").map(|e| e + 2).unwrap_or(rest.len());
    rest[..end].to_string()
}

/// Every `while (…) { … }` loop body in `src`, brace-matched. Robust to the
/// versioned shape (a fast loop + a fallback loop both appear).
fn while_loop_bodies(src: &str) -> Vec<String> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut search = 0usize;
    while let Some(rel) = src[search..].find("while") {
        let w = search + rel;
        // Find the opening brace of this while's block.
        let Some(brace_rel) = src[w..].find('{') else {
            break;
        };
        let open = w + brace_rel;
        // Brace-match to the close.
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
    out
}

// ---------------------------------------------------------------------------
// RED — the perf-shape spec (fails today, passes when Pass 1 lands).
// ---------------------------------------------------------------------------

/// After Pass 1, `isqrt` has a FAST loop whose Newton step lowers to bare i64 —
/// a `while` body carrying the raw `+`/`/` with NO overflow-promote helper
/// (`logos_add_exact` / `.expect_i64`). Today EVERY isqrt loop body is guarded,
/// so this fails; the versioned fast branch makes it pass. Marker-independent:
/// it only asks that *some* loop body be raw, which the fallback branch (kept
/// guarded) does not spoil.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn isqrt_has_a_raw_i64_fast_loop() {
    let rust = compile_to_rust(ISQRT_PROGRAM).expect("isqrt program compiles to Rust");
    let f = isqrt_fn(&rust);
    let bodies = while_loop_bodies(&f);
    assert!(
        !bodies.is_empty(),
        "isqrt must emit at least one while loop. isqrt emit:\n{f}"
    );
    let has_raw_loop = bodies.iter().any(|b| {
        !b.contains("logos_add_exact")
            && !b.contains("logos_mul_exact")
            && !b.contains(".expect_i64")
    });
    assert!(
        has_raw_loop,
        "Pass 1 (guard elision): isqrt's Newton loop must have a bare-i64 fast branch — \
         a while body with no per-iteration overflow-promote helper (the input bound is \
         proven ONCE, not re-checked every iteration). Every loop body is still guarded. \
         isqrt emit:\n{f}"
    );
}

/// The SAME pass generalizes: `powMod`'s modular-exponentiation loop
/// (`acc=(acc*b) mod m`, `b=(b*b) mod m`) proves i64-safe under `B_MUL_SAFE`
/// (`acc*b <= B² < i64::MAX`, `% m` shrinks back into `[-B,B]`), so its loop lowers
/// to bare i64 too. One pass, multiple caps.
const POWMOD_PROGRAM: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## To powMod (base: Int, exp: Int, m: Int) -> Int:
    Let mutable acc be 1.
    Let mutable b be base mod m.
    Let mutable e be exp.
    While e is greater than 0:
        If (e mod 2) is equal to 1:
            Set acc to (acc * b) mod m.
        Set e to e / 2.
        Set b to (b * b) mod m.
    Return acc.
## Main
Let arguments be args().
Let base be parseInt(item 2 of arguments).
Show powMod(base, 65537, 1000000007).
"#;

fn named_fn(rust: &str, sig: &str) -> String {
    let start = rust
        .find(sig)
        .unwrap_or_else(|| panic!("{sig} present in emit"));
    let rest = &rust[start..];
    let end = rest.find("\n}").map(|e| e + 2).unwrap_or(rest.len());
    rest[..end].to_string()
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn powmod_has_a_raw_i64_fast_loop() {
    let rust = compile_to_rust(POWMOD_PROGRAM).expect("powMod program compiles to Rust");
    let f = named_fn(&rust, "fn powMod");
    let bodies = while_loop_bodies(&f);
    assert!(
        !bodies.is_empty(),
        "powMod must emit a while loop. powMod emit:\n{f}"
    );
    let has_raw_loop = bodies.iter().any(|b| {
        !b.contains("logos_mul_exact")
            && !b.contains("logos_add_exact")
            && !b.contains(".expect_i64")
    });
    assert!(
        has_raw_loop,
        "Pass 1 must also version powMod's modular-exponentiation loop to bare i64. powMod emit:\n{f}"
    );
}

// ---------------------------------------------------------------------------
// SOUNDNESS PARTNERS — green today, MUST stay green when Pass 1 lands.
// A guard elided where it was actually needed = a miscompile on huge inputs.
// ---------------------------------------------------------------------------

/// powMod on the FAST path: with `m = 10^9+7 < B_MUL_SAFE`, every `acc*b`/`b*b` stays
/// `< 10^18 < i64::MAX`, so the pass's raw-i64 fast branch runs — and must compute the
/// modular exponentiation exactly. (The FALLBACK path for a huge modulus shares the
/// exact version-guard mechanism proven by `isqrt_i64_max_intermediate_stays_exact`;
/// a huge *constant* modulus additionally hits a pre-existing, accumbound-independent
/// codegen limit — confirmed: it fails to compile with the pass disabled too — so the
/// fast-path correctness is what this test pins.)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn powmod_fast_path_is_exact() {
    let src = r#"## To powMod (base: Int, exp: Int, m: Int) -> Int:
    Let mutable acc be 1.
    Let mutable b be base mod m.
    Let mutable e be exp.
    While e is greater than 0:
        If (e mod 2) is equal to 1:
            Set acc to (acc * b) mod m.
        Set e to e / 2.
        Set b to (b * b) mod m.
    Return acc.
## Main
Show powMod(7, 100, 1000000007).
"#;
    // 7^100 mod (10^9+7) = 946501044.
    assert_compiled_equals_interpreted(src);
    assert_all_tiers(src, "946501044");
}

/// isqrt on a small input is correct across tiers (tree-walker ≡ VM).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn isqrt_small_is_correct_all_tiers() {
    let src = r#"## To isqrt (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Let mutable x be n.
    Let mutable y be (x + 1) / 2.
    While y is less than x:
        Set x to y.
        Set y to (x + (n / x)) / 2.
    Return x.
## Main
Show isqrt(1000000).
"#;
    // isqrt(1_000_000) = 1000.
    assert_all_tiers(src, "1000");
}

/// THE adversarial fallback lock: a huge-but-i64 input whose intermediates
/// (`x + n/x`, and `n + 1` at n = i64::MAX-ish) exceed i64 must still be EXACT —
/// the AOT-compiled result must equal the interpreter's. If Pass 1 elides the
/// guard where it was needed, the AOT tier wraps/traps and this reddens.
/// `9223372036854775807` = i64::MAX; `isqrt` of it is `3037000499`, but the
/// intermediate `n + 1` overflows i64 and must promote.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn isqrt_i64_max_intermediate_stays_exact() {
    let src = r#"## To isqrt (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Let mutable x be n.
    Let mutable y be (x + 1) / 2.
    While y is less than x:
        Set x to y.
        Set y to (x + (n / x)) / 2.
    Return x.
## Main
Show isqrt(9223372036854775807).
"#;
    // The result fits i64 (3037000499) but the `n + 1` intermediate overflows —
    // the promoting path must carry it exactly. AOT must equal the interpreter.
    assert_compiled_equals_interpreted(src);
    assert_all_tiers(src, "3037000499");
}
