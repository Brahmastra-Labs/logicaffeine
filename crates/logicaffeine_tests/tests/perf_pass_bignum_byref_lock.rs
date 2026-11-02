//! Pass — by-reference bignum arithmetic (clone elision on promoted `Int` operands).
//!
//! WHY. A hot loop that REUSES a promoted (`LogosInt`) operand — the canonical shape is
//! exponentiation-by-squaring, `Set b to b * b` with `b` also read by `Set acc to acc * b` —
//! emits `logos_mul_exact(acc, b.clone())` and `logos_mul_exact(b.clone(), b.clone())`. Each
//! `.clone()` DEEP-COPIES the accumulator's heap `BigInt` buffer, once (or twice) per iteration,
//! purely to satisfy the by-VALUE `logos_*_exact(impl Into<LogosInt>, …)` signature. The value
//! it consumes is only ever READ.
//!
//! THE LEVER. `LogosInt::{add,sub,mul}` are already `(&self, rhs: &LogosInt) -> LogosInt`, and
//! `logos_mul_exact(a, b)` is DEFINED as `a.into().mul(&b.into())` — so `(&x).mul(&y)` is the
//! SAME computation with the SAME overflow-promotion, minus the clone. This pass lowers a
//! promoted-operand `+`/`-`/`*` to the by-ref method form: a reused bignum variable is BORROWED
//! (`&b`), never cloned. An i64 / temporary operand is bound once to a `LogosInt` (`(x).into()`)
//! and borrowed. Scoped to `has_bigint_operand` ops, so every plain-i64 emission is untouched.
//!
//! SOUNDNESS IS THE WHOLE GAME. Borrowing instead of cloning may not change a single value the
//! tree-walker would compute — the by-ref method IS what `logos_*_exact` calls. The partners
//! below prove the promoted path still yields the exact bignum at values that overflow i64, and
//! that every tier agrees.

mod common;
use common::{assert_all_tiers, assert_exact_output, compile_to_rust};

/// The exponentiation-by-squaring stdlib shape, self-contained. `b` is read by `acc * b` and
/// then reassigned by `b * b` — the reuse that forces a clone today.
const POW_EXACT: &str = r#"## To powExact (base: Int, exp: Int) -> Int:
    Let mutable acc be 1.
    Let mutable b be base.
    Let mutable e be exp.
    While e is greater than 0:
        If (e mod 2) is equal to 1:
            Set acc to acc * b.
        Set e to e / 2.
        Set b to b * b.
    Return acc.
## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Show powExact(2, n).
## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
"#;

/// Slice the emitted `fn powExact …` body so hot-path assertions do not match `main`.
fn pow_exact_fn(rust: &str) -> String {
    let start = rust.find("fn powExact").expect("powExact fn present");
    let rest = &rust[start..];
    let end = rest.find("\n}").map(|e| e + 2).unwrap_or(rest.len());
    rest[..end].to_string()
}

// 1. THE LEVER: no clone of a reused promoted operand survives.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bignum_reused_operand_is_borrowed_not_cloned() {
    let rust = compile_to_rust(POW_EXACT).expect("powExact compiles to Rust");
    let f = pow_exact_fn(&rust);
    assert!(
        !f.contains(".clone()"),
        "a reused promoted `Int` operand (`b`, read by `acc * b` then reassigned by `b * b`) \
         must be BORROWED for the multiply, never `.clone()`d — the clone deep-copies the heap \
         BigInt buffer every iteration for a value that is only read. powExact emit:\n{f}"
    );
}

// 1b. Positive form: the promoted `*` lowers to the by-ref method call.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bignum_multiply_uses_by_ref_method() {
    let rust = compile_to_rust(POW_EXACT).expect("powExact compiles to Rust");
    let f = pow_exact_fn(&rust);
    assert!(
        f.contains(".mul(&"),
        "a promoted `Int` multiply must emit the by-ref `(&x).mul(&y)` method form. \
         powExact emit:\n{f}"
    );
}

// 2. SOUNDNESS FLOOR: the by-ref multiply still promotes exactly past i64.
//    2^70 = 1180591620717411303424 overflows i64 — must stay exact.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn by_ref_multiply_still_exact_past_i64() {
    assert_exact_output(
        r#"## To powExact (base: Int, exp: Int) -> Int:
    Let mutable acc be 1.
    Let mutable b be base.
    Let mutable e be exp.
    While e is greater than 0:
        If (e mod 2) is equal to 1:
            Set acc to acc * b.
        Set e to e / 2.
        Set b to b * b.
    Return acc.
## Main
Show powExact(2, 70).
"#,
        "1180591620717411303424",
    );
}

// 2b. A genuinely large exponent whose result is a many-limb bignum stays exact.
//     3^100 = 515377520732011331036461129765621272702107522001.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn by_ref_multiply_exact_many_limbs() {
    assert_exact_output(
        r#"## To powExact (base: Int, exp: Int) -> Int:
    Let mutable acc be 1.
    Let mutable b be base.
    Let mutable e be exp.
    While e is greater than 0:
        If (e mod 2) is equal to 1:
            Set acc to acc * b.
        Set e to e / 2.
        Set b to b * b.
    Return acc.
## Main
Show powExact(3, 100).
"#,
        "515377520732011331036461129765621272702107522001",
    );
}

// 3. Cross-tier: tree-walker ≡ VM ≡ AOT on a value that stays within i64 (the Small path
//    of the by-ref method) — proves the borrow-not-clone lowering did not perturb the small case.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn by_ref_multiply_all_tiers_agree_small() {
    assert_all_tiers(
        r#"## To powExact (base: Int, exp: Int) -> Int:
    Let mutable acc be 1.
    Let mutable b be base.
    Let mutable e be exp.
    While e is greater than 0:
        If (e mod 2) is equal to 1:
            Set acc to acc * b.
        Set e to e / 2.
        Set b to b * b.
    Return acc.
## Main
Show powExact(3, 20).
"#,
        "3486784401",
    );
}

// 3b. A promoted ADD that reuses its operand also borrows: `Return p + p` where `p` is the
//     factorial accumulator (a many-limb bignum) read twice. Proves the pass covers `+`, not
//     just `*` — the emit is `p.add(&p)` (both reads borrowed, no clone).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn by_ref_add_reused_operand_is_borrowed() {
    let src = r#"## To facDouble (n: Int) -> Int:
    Let mutable p be 1.
    Let mutable i be 1.
    While i is at most n:
        Set p to p * i.
        Set i to i + 1.
    Return p + p.
## Main
Show facDouble(25).
"#;
    let rust = compile_to_rust(src).expect("facDouble compiles to Rust");
    let f = {
        let start = rust.find("fn facDouble").expect("facDouble fn present");
        let rest = &rust[start..];
        let end = rest.find("\n}").map(|e| e + 2).unwrap_or(rest.len());
        rest[..end].to_string()
    };
    assert!(
        f.contains(".add(&") && !f.contains(".clone()"),
        "a promoted `+` with a reused bignum operand (`p + p`) must borrow both reads via \
         `p.add(&p)`, never clone. facDouble emit:\n{f}"
    );
    // 2·25! = 31022420086661971968000000 — many-limb bignum, exact across tiers.
    assert_exact_output(src, "31022420086661971968000000");
}

/// The factorial-accumulator body used by the nested/sub adversarial cases below.
const FAC_BODY: &str = r#"## To fac (n: Int) -> Int:
    Let mutable p be 1.
    Let mutable i be 1.
    While i is at most n:
        Set p to p * i.
        Set i to i + 1.
    Return p."#;

// 4. ADVERSARIAL: a NESTED promoted multiply `(p * p) * p`. The inner `p * p` becomes a
//    `LogosInt` temp (`.into()`), the outer multiplies by a borrowed `p`. Every read of the
//    reused bignum `p` is borrowed — no clone anywhere in the chain — and the value is exact.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn nested_promoted_multiply_is_exact_and_clone_free() {
    let src = format!(
        "{FAC_BODY}\n## To nestedMul (n: Int) -> Int:\n    Let p be fac(n).\n    Return (p * p) * p.\n## Main\nShow nestedMul(20).\n"
    );
    let rust = compile_to_rust(&src).expect("nestedMul compiles to Rust");
    let f = {
        let start = rust.find("fn nestedMul").expect("nestedMul fn present");
        let rest = &rust[start..];
        let end = rest.find("\n}").map(|e| e + 2).unwrap_or(rest.len());
        rest[..end].to_string()
    };
    assert!(
        f.contains(".mul(&") && !f.contains(".clone()"),
        "a nested promoted multiply must borrow every reused bignum read (`.mul(&`), no \
         clone. nestedMul emit:\n{f}"
    );
    // (20!)^3, exact across every tier.
    assert_exact_output(
        &src,
        "14400376622525549608547603031202889616850944000000000000",
    );
}

// 4b. ADVERSARIAL: a promoted SUBTRACT with a reused operand, `q - p` where both are bignums
//     and `p` was also read to build `q`. Proves the `-` lane borrows, exact.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn promoted_subtract_reused_operand_is_exact() {
    let src = format!(
        "{FAC_BODY}\n## To bignumSub (n: Int) -> Int:\n    Let p be fac(n).\n    Let q be p * p.\n    Return q - p.\n## Main\nShow bignumSub(20).\n"
    );
    let rust = compile_to_rust(&src).expect("bignumSub compiles to Rust");
    let f = {
        let start = rust.find("fn bignumSub").expect("bignumSub fn present");
        let rest = &rust[start..];
        let end = rest.find("\n}").map(|e| e + 2).unwrap_or(rest.len());
        rest[..end].to_string()
    };
    assert!(
        f.contains(".sub(&") && !f.contains(".clone()"),
        "a promoted subtract with a reused operand must borrow via `.sub(&`, no clone. \
         bignumSub emit:\n{f}"
    );
    // (20!)^2 − 20! = 20!·(20!−1), exact across every tier.
    assert_exact_output(&src, "5919012181389927682984539681423360000");
}
