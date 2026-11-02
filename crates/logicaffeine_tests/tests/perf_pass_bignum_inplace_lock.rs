//! Pass — in-place bignum accumulator mutation (`Set acc to acc * <i64>` / `acc + <i64>`).
//!
//! WHY. Even after clone-elision ([[bignum-byref-arith-pass]]) the flagship bignum loop —
//! `factorial`, `Set p to p * i` — emits `p = LogosInt::from(p.mul(&i.into()))`: the functional
//! `mul` ALLOCATES a fresh magnitude every iteration and drops the old one. C/GMP grow the
//! accumulator's buffer in place. When the accumulator `p` is a promoted `Int` that is dead the
//! instant it is reassigned (`count_target_reads == 1`) and the other operand is a scalar `i64`,
//! `p * i` / `p + i` / `p - i` can mutate `p`'s limbs IN PLACE (`p.mul_assign_i64(i)`), reusing
//! the heap `Vec` — no per-iteration allocation.
//!
//! SOUNDNESS. In-place is a pure implementation detail: `mul_assign_i64(i)` computes the exact
//! same value as `p.mul(&i)` (Small fast-path with overflow-promotion; Big multiplied limb-by-limb
//! with carry), it just writes it back into `p` instead of a fresh cell. The partners below prove
//! the value is byte-identical to the tree-walker at values that overflow i64 (so promotion is
//! intact) and that a scalar which is NOT a compile-time-known i64 (a bignum operand) is left on
//! the by-ref path.

mod common;
use common::{assert_all_tiers, assert_exact_output, compile_to_rust};

const FACTORIAL: &str = r#"## To factorial (n: Int) -> Int:
    Let mutable p be 1.
    Let mutable i be 1.
    While i is less than or equal to n:
        Set p to p * i.
        Set i to i + 1.
    Return p.
## Main
Show factorial(25).
"#;

fn factorial_fn(rust: &str) -> String {
    let start = rust.find("fn factorial").expect("factorial fn present");
    let rest = &rust[start..];
    let end = rest.find("\n}").map(|e| e + 2).unwrap_or(rest.len());
    rest[..end].to_string()
}

// 1. THE LEVER: the scalar accumulator mutates in place — no fresh `LogosInt` product per iter.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn factorial_accumulator_mutates_in_place() {
    let rust = compile_to_rust(FACTORIAL).expect("factorial compiles to Rust");
    let f = factorial_fn(&rust);
    assert!(
        f.contains("mul_assign_i64"),
        "the `Set p to p * i` accumulator (promoted `Int`, dead last-use, i64 scalar) must \
         mutate `p` IN PLACE via `mul_assign_i64` — no fresh magnitude allocated per iteration. \
         factorial emit:\n{f}"
    );
}

// 2. SOUNDNESS FLOOR: in-place multiply still promotes exactly past i64.
//    25! = 15511210043330985984000000 overflows i64 — must stay exact.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn in_place_factorial_still_exact() {
    assert_exact_output(FACTORIAL, "15511210043330985984000000");
}

// 2b. A large factorial (many-limb) stays exact across every tier — proves the in-place limb
//     multiply carries correctly at width. 40! = 815915283247897734345611269596115894272000000000.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn in_place_factorial_many_limbs_all_tiers() {
    assert_all_tiers(
        r#"## To factorial (n: Int) -> Int:
    Let mutable p be 1.
    Let mutable i be 1.
    While i is less than or equal to n:
        Set p to p * i.
        Set i to i + 1.
    Return p.
## Main
Show factorial(40).
"#,
        "815915283247897734345611269596115894272000000000",
    );
}

// 3. A MIXED accumulator (`Set s to s * i` alongside `Set s to s + i`) mutates in place for the
//    scalar MULTIPLY, and the whole loop stays exact across every tier. The scalar `+` still uses
//    the functional add (in-place scalar add/sub is a deliberate follow-up — the op is cheap and
//    its in-place form needs sign-flip + demotion handling in the bignum core); the multiply is
//    the alloc-dominant step this pass targets.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn mixed_scalar_accumulator_multiply_in_place_and_exact() {
    let src = r#"## To bigTriangle (n: Int) -> Int:
    Let mutable s be 1.
    Let mutable i be 1.
    While i is less than or equal to n:
        Set s to s * i.
        Set s to s + i.
        Set i to i + 1.
    Return s.
## Main
Show bigTriangle(30).
"#;
    let rust = compile_to_rust(src).expect("bigTriangle compiles to Rust");
    let f = {
        let start = rust.find("fn bigTriangle").expect("bigTriangle fn present");
        let rest = &rust[start..];
        let end = rest.find("\n}").map(|e| e + 2).unwrap_or(rest.len());
        rest[..end].to_string()
    };
    assert!(
        f.contains("mul_assign_i64"),
        "the scalar multiply in a mixed bignum accumulator must mutate in place. \
         bigTriangle emit:\n{f}"
    );
    // Cross-tier exactness of the whole (mul-in-place + functional-add) loop is the guarantee.
    assert_all_tiers(src, "986284888586464567653944864693700");
}

// 4. GUARD: a Big × Big multiply (both operands bignum) has NO scalar in-place form — it must
//    stay on the by-ref path (`.mul(&`), never mis-lowered to a scalar in-place op.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bignum_times_bignum_stays_by_ref_not_scalar_inplace() {
    let src = r#"## To sq (n: Int) -> Int:
    Let mutable p be 1.
    Let mutable i be 1.
    While i is less than or equal to n:
        Set p to p * i.
        Set i to i + 1.
    Set p to p * p.
    Return p.
## Main
Show sq(25).
"#;
    let rust = compile_to_rust(src).expect("sq compiles to Rust");
    let f = {
        let start = rust.find("fn sq").expect("sq fn present");
        let rest = &rust[start..];
        let end = rest.find("\n}").map(|e| e + 2).unwrap_or(rest.len());
        rest[..end].to_string()
    };
    assert!(
        f.contains(".mul(&"),
        "a Big × Big multiply (`p * p`) has no scalar in-place form and must stay on the \
         by-ref method path. sq emit:\n{f}"
    );
    // (25!)^2 exact.
    assert_exact_output(src, "240597637008332048087335626345604448256000000000000");
}
