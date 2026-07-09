//! Exact numeric comparison for GENERATED code.
//!
//! Mixed Int/Float comparison in LOGOS is EXACT — mathematical values, never
//! a lossy cast (`i as f64` rounds above 2^53, so a cast-based `==` would
//! call `9007199254740993` equal to the float `9007199254740992.0`). The AOT
//! backend emits these helpers for statically-mixed compares; same-type
//! compares are untouched, so hot loops pay nothing.

pub use logicaffeine_base::numeric::cmp_i64_f64_exact as logos_cmp_i64_f64;

/// `i == f`, exactly (NaN is equal to nothing).
#[inline]
pub fn logos_i64_eq_f64(i: i64, f: f64) -> bool {
    logos_cmp_i64_f64(i, f) == Some(core::cmp::Ordering::Equal)
}

/// `a is approximately b` — the TOLERANT float comparison (`==` is IEEE
/// bit-exact). Python's `math.isclose` semantics: relative tolerance 1e-9
/// (nine significant digits agree) with absolute floor 1e-12 (so values near
/// zero compare sanely), plus an exact fast path so `inf is approximately
/// inf` holds. NaN is approximately nothing. ONE definition — the
/// tree-walker, VM, JIT deopt path, AOT, and WASM lowering all share it.
#[inline]
pub fn logos_approx_eq(a: f64, b: f64) -> bool {
    if a == b {
        return true;
    }
    let diff = (a - b).abs();
    diff <= f64::max(1e-9 * f64::max(a.abs(), b.abs()), 1e-12)
}

/// Truthiness for GENERATED code — ONE definition shared with the
/// interpreter's `RuntimeValue::is_truthy` and the VM's `Value::is_truthy`.
/// Falsy: `false`, numeric zero (`-0.0` is zero; NaN is nonzero and truthy),
/// `nothing` (`None`), and empty Text/Seq/Map/Set. Everything else is truthy.
pub trait Truthy {
    fn truthy(&self) -> bool;
}

impl Truthy for bool {
    #[inline]
    fn truthy(&self) -> bool {
        *self
    }
}

impl Truthy for i64 {
    #[inline]
    fn truthy(&self) -> bool {
        *self != 0
    }
}

impl Truthy for f64 {
    #[inline]
    fn truthy(&self) -> bool {
        *self != 0.0
    }
}

impl Truthy for char {
    #[inline]
    fn truthy(&self) -> bool {
        true
    }
}

impl Truthy for String {
    #[inline]
    fn truthy(&self) -> bool {
        !self.is_empty()
    }
}

impl Truthy for &str {
    #[inline]
    fn truthy(&self) -> bool {
        !self.is_empty()
    }
}

impl<T> Truthy for crate::types::LogosSeq<T> {
    #[inline]
    fn truthy(&self) -> bool {
        !self.is_empty()
    }
}

impl<K: Eq + core::hash::Hash, V> Truthy for crate::types::LogosMap<K, V> {
    #[inline]
    fn truthy(&self) -> bool {
        !self.is_empty()
    }
}

impl<T> Truthy for crate::types::FxIndexSet<T> {
    #[inline]
    fn truthy(&self) -> bool {
        !self.is_empty()
    }
}

/// `nothing` is falsy; a present value keeps its own truthiness (the
/// interpreter never sees the `Some` wrapper — `Some(0)` at runtime IS `0`).
impl<T: Truthy> Truthy for Option<T> {
    #[inline]
    fn truthy(&self) -> bool {
        match self {
            Some(v) => v.truthy(),
            None => false,
        }
    }
}

impl Truthy for crate::types::LogosRational {
    #[inline]
    fn truthy(&self) -> bool {
        !self.0.is_zero()
    }
}

impl Truthy for crate::types::LogosDecimal {
    #[inline]
    fn truthy(&self) -> bool {
        !self.0.is_zero()
    }
}

impl Truthy for crate::types::LogosComplex {
    #[inline]
    fn truthy(&self) -> bool {
        !self.0.is_zero()
    }
}

/// The truthiness entry point the AOT backend emits (`logos_truthy(&x)`).
#[inline]
pub fn logos_truthy<T: Truthy>(v: &T) -> bool {
    v.truthy()
}

/// Exact Int arithmetic for GENERATED code (overflow ruling v2, stage 2):
/// the i64 fast path is a single checked op; overflow spills to the
/// promoting [`crate::types::LogosInt`] — the AOT's mirror of the
/// interpreter's Int→BigInt promotion. `impl Into<LogosInt>` lets an
/// already-promoted operand chain through the same helper unchanged.
#[inline(always)]
pub fn logos_add_exact(a: impl Into<crate::types::LogosInt>, b: impl Into<crate::types::LogosInt>) -> crate::types::LogosInt {
    a.into().add(&b.into())
}

#[inline(always)]
pub fn logos_sub_exact(a: impl Into<crate::types::LogosInt>, b: impl Into<crate::types::LogosInt>) -> crate::types::LogosInt {
    a.into().sub(&b.into())
}

#[inline(always)]
pub fn logos_mul_exact(a: impl Into<crate::types::LogosInt>, b: impl Into<crate::types::LogosInt>) -> crate::types::LogosInt {
    a.into().mul(&b.into())
}

/// Truncating division — loud canonical panic on a zero divisor (the same
/// failure the interpreter raises); `i64::MIN / -1` promotes exactly.
#[inline(always)]
pub fn logos_div_exact(a: impl Into<crate::types::LogosInt>, b: impl Into<crate::types::LogosInt>) -> crate::types::LogosInt {
    match a.into().div(&b.into()) {
        Ok(v) => v,
        Err(e) => panic!("{e}"),
    }
}

/// Floor division — the quotient rounded toward NEGATIVE INFINITY (`-7 // 2 → -4`),
/// the `//` operator. Loud canonical panic on a zero divisor; exact (promotes to
/// `BigInt`). Distinct from [`logos_div_exact`], which truncates toward zero.
#[inline(always)]
pub fn logos_floordiv_exact(a: impl Into<crate::types::LogosInt>, b: impl Into<crate::types::LogosInt>) -> crate::types::LogosInt {
    match a.into().div_floor(&b.into()) {
        Ok(v) => v,
        Err(e) => panic!("{e}"),
    }
}

/// Fused NARROWED exact ops (overflow ruling v2, stage 2): emitted when a
/// single exact op's result narrows straight back to i64 — the dominant
/// hot-loop shape. The fast path is one checked op on machine integers (no
/// `LogosInt` materialization, so the value lives in a register and LLVM's
/// induction reasoning survives); overflow raises the same loud canonical
/// error the `logos_*_exact(..).expect_i64("Int")` chain raises, byte for
/// byte (the printed value is the exact result).
#[inline(always)]
pub fn logos_add_i64(a: i64, b: i64) -> i64 {
    match a.checked_add(b) {
        Some(v) => v,
        None => narrowed_overflow(logos_add_exact(a, b)),
    }
}

#[inline(always)]
pub fn logos_sub_i64(a: i64, b: i64) -> i64 {
    match a.checked_sub(b) {
        Some(v) => v,
        None => narrowed_overflow(logos_sub_exact(a, b)),
    }
}

#[inline(always)]
pub fn logos_mul_i64(a: i64, b: i64) -> i64 {
    match a.checked_mul(b) {
        Some(v) => v,
        None => narrowed_overflow(logos_mul_exact(a, b)),
    }
}

/// `checked_div` is `None` on exactly the two slow cases (zero divisor,
/// `i64::MIN / -1`); both re-route through the exact helper for the canonical
/// error / exact-promotion narrow.
#[inline(always)]
pub fn logos_div_i64(a: i64, b: i64) -> i64 {
    match a.checked_div(b) {
        Some(v) => v,
        None => logos_div_exact(a, b).expect_i64("Int"),
    }
}

/// `checked_rem` is `None` on a zero divisor (canonical panic via the exact
/// helper) and on `i64::MIN % -1` (which the helper resolves to 0 — cold but
/// correct).
#[inline(always)]
pub fn logos_rem_i64(a: i64, b: i64) -> i64 {
    match a.checked_rem(b) {
        Some(v) => v,
        None => logos_rem_exact(a, b).expect_i64("Int"),
    }
}

#[cold]
#[inline(never)]
fn narrowed_overflow(v: crate::types::LogosInt) -> ! {
    panic!("Integer overflow: {v} does not fit a 64-bit Int")
}

/// Narrow a width-bounded i128 chain result back to i64 (overflow ruling v2,
/// stage 2): the codegen lowers an exact chain whose worst-case bit-width
/// provably fits i128 as native i128 arithmetic (every intermediate exact by
/// construction) and narrows ONCE at the root. Same canonical error text as
/// `expect_i64` — an i128 prints the same digits `LogosInt` would.
#[inline(always)]
pub fn logos_narrow_i128(v: i128) -> i64 {
    match i64::try_from(v) {
        Ok(x) => x,
        Err(_) => narrowed_overflow_i128(v),
    }
}

#[cold]
#[inline(never)]
fn narrowed_overflow_i128(v: i128) -> ! {
    panic!("Integer overflow: {v} does not fit a 64-bit Int")
}

/// Truncating i128 division inside a width-bounded exact chain. The zero
/// divisor raises the canonical error; `i128::MIN / -1` cannot occur (chain
/// operands are width-bounded far below `i128::MIN`).
#[inline(always)]
pub fn logos_div_i128(a: i128, b: i128) -> i128 {
    if b == 0 {
        panic!("Division by zero");
    }
    a / b
}

/// Truncating i128 remainder inside a width-bounded exact chain — the
/// canonical zero-divisor error, sign of the dividend.
#[inline(always)]
pub fn logos_rem_i128(a: i128, b: i128) -> i128 {
    if b == 0 {
        panic!("Modulo by zero");
    }
    a % b
}

/// Truncating remainder — loud canonical panic on a zero divisor;
/// `i64::MIN % -1` is 0 (no overflow, no panic).
#[inline(always)]
pub fn logos_rem_exact(a: impl Into<crate::types::LogosInt>, b: impl Into<crate::types::LogosInt>) -> crate::types::LogosInt {
    match a.into().rem(&b.into()) {
        Ok(v) => v,
        Err(e) => panic!("{e}"),
    }
}

/// Exact integer exponentiation — the i64 fast path promotes to `BigInt` on
/// overflow (`2 ** 100`). Loud canonical panic on a negative exponent (an Int
/// can't hold the fractional result), mirroring the interpreter.
#[inline(always)]
pub fn logos_pow_exact(a: impl Into<crate::types::LogosInt>, b: impl Into<crate::types::LogosInt>) -> crate::types::LogosInt {
    match a.into().pow(&b.into()) {
        Ok(v) => v,
        Err(e) => panic!("{e}"),
    }
}

/// Numeric-unified map key: a `Float` used to index an `Int`-keyed map matches
/// the Int key `i` iff the float is exactly `i` (integral and in i64 range) —
/// the compiled mirror of the interpreter's `1 == 1.0` key coercion. A
/// non-integral float (`1.5`) matches no Int key.
#[inline(always)]
pub fn logos_i64_key_of_f64(f: f64) -> Option<i64> {
    if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
        Some(f as i64)
    } else {
        None
    }
}
