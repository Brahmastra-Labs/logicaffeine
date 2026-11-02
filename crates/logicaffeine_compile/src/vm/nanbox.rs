//! An 8-byte NaN-boxed VM value (groundwork for a narrow register file).
//!
//! The VM register file currently holds [`crate::vm::value::Value`], a newtype
//! over the fat tree-walker enum [`RuntimeValue`]. That enum is **16 bytes**
//! (proven by `nanbox_runtime_value_is_16_bytes`): an 8-byte payload word — the
//! `Int(i64)`/`Float(f64)`/`Moment(i64)`/… arms each need a full machine word —
//! beside a discriminant word the compiler cannot niche-pack away (there is no
//! spare bit in an `i64`/`f64` payload). Every register read/write/clone in the
//! dispatch loop copies those 16 bytes; only two registers fit per 32-byte
//! half-cache-line.
//!
//! A *16-byte tagged union* would therefore buy **nothing** on width — it is the
//! same size as the enum it would replace. The only representation that actually
//! narrows the register file is **8 bytes**, and the only way to fit `Int`,
//! `Float`, `Bool`, `Nothing`, and a heap pointer into one 64-bit word is
//! NaN-boxing. [`Narrow`] is that 8-byte value, built and exhaustively tested in
//! isolation so the representation-overhaul wave can swap it into the register
//! file with the encoding already de-risked.
//!
//! # The encoding
//!
//! IEEE-754 `f64` has 2^52 distinct quiet-NaN bit-patterns (`exponent = 0x7FF`,
//! top mantissa bit set, any of the low 51 bits, either sign). Real computation
//! produces exactly one canonical NaN; the other ~2^52 patterns are free real
//! estate. We reserve the quiet-NaN region as our tag space and store every
//! non-float value inside it:
//!
//! ```text
//!  bit: 63        62..52        51..48     47..0
//!       [sign=1] [exp=0x7FF] [tag : 4]  [payload : 48]
//! ```
//!
//! - Any bit pattern that is **not** in this reserved region is a genuine
//!   `f64`, stored verbatim ([`Narrow::float`] / [`Narrow::as_float`]). Floats
//!   are zero-cost: no tagging, no masking on the read.
//! - A real `f64::NAN` is *canonicalised* on the way in to a single bit pattern
//!   that we keep OUTSIDE the tag region (`CANONICAL_NAN`), so it round-trips as
//!   a float and can never be mistaken for a boxed value. This is the classic
//!   NaN-boxing trap, designed out and proven by
//!   `nanbox_nan_payload_is_not_aliased_with_heap`.
//! - The 4 tag bits select `Int`, `Bool`, `Nothing`, or `Heap`; the 48 payload
//!   bits carry the value (a sign-extended small integer, the bool, or a 48-bit
//!   heap pointer).
//!
//! # The i64 tradeoff (justified)
//!
//! LOGOS's hot type is a *full-range* `i64` — `semantics::arith` is wrapping-i64
//! over the whole range and the differential gate pins `i64::MIN`/`MAX`. A full
//! i64 cannot fit in 48 payload bits, and no 64-bit encoding can inline a full
//! i64 *and* a full f64 *and* a tag (that is 64 + 64 + ε bits of information in
//! 64 bits — impossible). So we inline the 48-bit signed range
//! `[-2^47, 2^47)` (±140 trillion — every realistic loop counter, array index,
//! and `% n` value) and **box** integers outside it behind the `Heap` tag. The
//! representation stays lossless for all i64 (`nanbox_int_round_trips_over_edge_
//! grid` covers `i64::MIN`/`MAX`, which box); only the *inline* fast path is
//! range-limited, and the arith-parity test proves the inline path is bit-exact
//! to the kernel for every value that stays inline. Choosing to box large ints
//! (rather than boxing floats, the classic-JS choice) is the right call *for
//! this language*: JS's number is f64, but LOGOS's is i64, and inline small ints
//! are what the dispatch loop touches.
//!
//! # Soundness
//!
//! Every constructor produces a `u64` whose interpretation is determined solely
//! by which numeric region it lands in; every accessor classifies by that same
//! region before reading the payload, so a value is never read as the wrong
//! type. The heap arm stores a `Box<RuntimeValue>` raw pointer in the low 48
//! bits; `Drop`/`Clone` reconstruct that box only for the `Heap` tag, freeing /
//! cloning exactly once. The scalar arms own no resources. The 48-bit pointer
//! assumption (current x86-64 / AArch64 user-space) is asserted at box time.

use crate::interpreter::RuntimeValue;

/// An 8-byte NaN-boxed VM value: inline `f64`, inline small `i64`/`Bool`/
/// `Nothing`, boxed pointer for everything else (including out-of-range `i64`).
pub struct Narrow(u64);

// ---- Bit-layout constants ------------------------------------------------

/// The quiet-NaN region marker: sign + all-ones exponent + top mantissa bit.
/// Any `u64` with these bits set (and thus `>= QNAN`) is one of our tagged
/// values; anything below is a genuine non-NaN-or-canonical-NaN `f64`.
const QNAN: u64 = 0xFFF8_0000_0000_0000;

/// The single bit pattern a real `f64::NAN` is folded to: the all-ones pattern.
/// Every tagged value is `QNAN | (tag << 48) | payload` with `tag ∈ 0..=3` (bits
/// 49..48) and a 48-bit payload, so its high three nibbles are `0xFFF8..=0xFFFB`
/// — strictly below `0xFFFF…`. The canonical NaN therefore collides with NO
/// tagged value, which is what makes the float path and the tag space disjoint.
const CANONICAL_NAN: u64 = 0xFFFF_FFFF_FFFF_FFFF;

/// 2-bit tag field in bits 49..48 (just above the 48-bit payload, below QNAN's
/// quiet bit 51). Two bits ⇒ four tags ⇒ tagged values stay in `0xFFF8..=0xFFFB`,
/// the disjointness `CANONICAL_NAN` relies on.
const TAG_SHIFT: u64 = 48;
const TAG_MASK: u64 = 0x3;
const PAYLOAD_MASK: u64 = (1 << 48) - 1;

const TAG_INT: u64 = 0;
const TAG_BOOL: u64 = 1;
const TAG_NOTHING: u64 = 2;
const TAG_HEAP: u64 = 3;

/// Inclusive bounds of the inline signed-integer range `[-2^47, 2^47)`.
const INT_INLINE_MIN: i64 = -(1 << 47);
const INT_INLINE_MAX: i64 = (1 << 47) - 1;

#[inline]
fn is_tagged(bits: u64) -> bool {
    // A genuine float (incl. ±inf and the canonical NaN) is excluded:
    // CANONICAL_NAN is all-ones, and our tags are 0..=3, so a tagged value's
    // high bits are exactly QNAN | (tag<<48) with tag<=3, never all-ones.
    bits >= QNAN && bits != CANONICAL_NAN
}

#[inline]
fn tag_of(bits: u64) -> u64 {
    (bits >> TAG_SHIFT) & TAG_MASK
}

impl Narrow {
    // ---- Inline constructors --------------------------------------------

    /// Encode an `i64`. Values in `[-2^47, 2^47)` go inline; anything outside
    /// boxes (still lossless — see [`Narrow::as_int`]).
    #[inline]
    pub fn int(n: i64) -> Self {
        if (INT_INLINE_MIN..=INT_INLINE_MAX).contains(&n) {
            let payload = (n as u64) & PAYLOAD_MASK;
            Narrow(QNAN | (TAG_INT << TAG_SHIFT) | payload)
        } else {
            Narrow::heap(RuntimeValue::Int(n))
        }
    }

    /// Encode an `f64` verbatim. A real NaN is canonicalised so it round-trips
    /// as a float and never aliases a tag.
    #[inline]
    pub fn float(f: f64) -> Self {
        let bits = f.to_bits();
        if f.is_nan() {
            Narrow(CANONICAL_NAN)
        } else {
            Narrow(bits)
        }
    }

    #[inline]
    pub fn bool(b: bool) -> Self {
        Narrow(QNAN | (TAG_BOOL << TAG_SHIFT) | (b as u64))
    }

    #[inline]
    pub fn nothing() -> Self {
        Narrow(QNAN | (TAG_NOTHING << TAG_SHIFT))
    }

    #[inline]
    fn heap(rv: RuntimeValue) -> Self {
        let ptr = Box::into_raw(Box::new(rv)) as u64;
        debug_assert_eq!(
            ptr & !PAYLOAD_MASK,
            0,
            "heap pointer exceeds 48 bits — NaN-boxing pointer assumption violated"
        );
        Narrow(QNAN | (TAG_HEAP << TAG_SHIFT) | (ptr & PAYLOAD_MASK))
    }

    // ---- Inline accessors ------------------------------------------------

    /// Read back an `i64` whether it is inlined or boxed (lossless for all i64).
    #[inline]
    pub fn as_int(&self) -> Option<i64> {
        if !is_tagged(self.0) {
            return None;
        }
        match tag_of(self.0) {
            TAG_INT => {
                // Sign-extend the 48-bit payload back to i64.
                let payload = (self.0 & PAYLOAD_MASK) as i64;
                Some((payload << 16) >> 16)
            }
            TAG_HEAP => match self.heap_ref() {
                RuntimeValue::Int(n) => Some(*n),
                _ => None,
            },
            _ => None,
        }
    }

    /// Read back an `f64`. Returns `None` for any tagged (non-float) value.
    #[inline]
    pub fn as_float(&self) -> Option<f64> {
        if is_tagged(self.0) {
            None
        } else {
            Some(f64::from_bits(self.0))
        }
    }

    #[inline]
    pub fn as_bool(&self) -> Option<bool> {
        if is_tagged(self.0) && tag_of(self.0) == TAG_BOOL {
            Some((self.0 & PAYLOAD_MASK) != 0)
        } else {
            None
        }
    }

    #[inline]
    pub fn is_nothing(&self) -> bool {
        is_tagged(self.0) && tag_of(self.0) == TAG_NOTHING
    }

    #[inline]
    pub fn is_heap(&self) -> bool {
        is_tagged(self.0) && tag_of(self.0) == TAG_HEAP
    }

    /// True iff the integer lives inline (vs boxed). Diagnostic / test hook —
    /// the inline-vs-boxed classification the round-trip and arith-parity tests
    /// assert against; the production accessors use [`Narrow::as_inline_int`].
    #[inline]
    #[allow(dead_code)]
    pub fn is_inline_int(&self) -> bool {
        is_tagged(self.0) && tag_of(self.0) == TAG_INT
    }

    /// SAFETY-bearing: borrow the boxed `RuntimeValue`. Caller must have
    /// established the `Heap` tag (every call site does).
    #[inline]
    fn heap_ref(&self) -> &RuntimeValue {
        let ptr = (self.0 & PAYLOAD_MASK) as *const RuntimeValue;
        // SAFETY: tag is Heap ⇒ payload is a live `Box<RuntimeValue>` pointer we
        // own; the box outlives this borrow (it lives until our Drop).
        unsafe { &*ptr }
    }

    /// Borrow the heap payload, or `None` for inline scalars.
    #[inline]
    pub fn as_heap(&self) -> Option<&RuntimeValue> {
        if self.is_heap() {
            Some(self.heap_ref())
        } else {
            None
        }
    }

    /// Mutably borrow the boxed `RuntimeValue`, **promoting an inline scalar to
    /// the heap form first** so a stable `&mut RuntimeValue` always exists. This
    /// is the seam the VM's `as_runtime_mut` rides: a scalar register that is
    /// mutated in place (`Set field of struct`, in-place `Text` append) becomes
    /// boxed, and the mutation lands in that box. Lossless — the promoted value
    /// is exactly what `to_runtime()` would have produced.
    #[inline]
    pub fn make_heap_mut(&mut self) -> &mut RuntimeValue {
        if !self.is_heap() {
            let rv = self.to_runtime();
            *self = Narrow::heap(rv);
        }
        let ptr = (self.0 & PAYLOAD_MASK) as *mut RuntimeValue;
        // SAFETY: we just ensured the Heap tag ⇒ payload is the unique owning
        // box pointer; the box outlives this borrow (it lives until our Drop),
        // and `&mut self` proves no other borrow is live.
        unsafe { &mut *ptr }
    }

    // ---- Lossless round-trip with the fat enum ---------------------------

    /// Encode a [`RuntimeValue`] into the narrow form. Scalar hot types go
    /// inline (large ints box); every other variant is boxed verbatim (the
    /// `Rc`/`Box` payloads move in, preserving reference identity).
    #[inline]
    pub fn from_runtime(rv: RuntimeValue) -> Self {
        match rv {
            RuntimeValue::Int(n) => Narrow::int(n),
            RuntimeValue::Float(f) => Narrow::float(f),
            RuntimeValue::Bool(b) => Narrow::bool(b),
            RuntimeValue::Nothing => Narrow::nothing(),
            other => Narrow::heap(other),
        }
    }

    /// Decode back to a [`RuntimeValue`], consuming `self`. Exactly inverts
    /// [`Narrow::from_runtime`].
    #[inline]
    pub fn into_runtime(self) -> RuntimeValue {
        let bits = self.0;
        if !is_tagged(bits) {
            // Forget the husk (no heap to free) and return the float.
            std::mem::forget(self);
            return RuntimeValue::Float(f64::from_bits(bits));
        }
        match tag_of(bits) {
            TAG_INT => {
                std::mem::forget(self);
                let payload = (bits & PAYLOAD_MASK) as i64;
                RuntimeValue::Int((payload << 16) >> 16)
            }
            TAG_BOOL => {
                std::mem::forget(self);
                RuntimeValue::Bool((bits & PAYLOAD_MASK) != 0)
            }
            TAG_NOTHING => {
                std::mem::forget(self);
                RuntimeValue::Nothing
            }
            TAG_HEAP => {
                let ptr = (bits & PAYLOAD_MASK) as *mut RuntimeValue;
                std::mem::forget(self);
                // SAFETY: Heap tag ⇒ payload is the unique owning box pointer;
                // reconstitute it and move the value out exactly once. Forgetting
                // the husk first prevents our Drop from freeing it again.
                *unsafe { Box::from_raw(ptr) }
            }
            _ => unreachable!("invalid tag"),
        }
    }

    /// Materialise a fresh [`RuntimeValue`] without consuming `self`.
    #[inline]
    pub fn to_runtime(&self) -> RuntimeValue {
        if !is_tagged(self.0) {
            return RuntimeValue::Float(f64::from_bits(self.0));
        }
        match tag_of(self.0) {
            TAG_INT => {
                let payload = (self.0 & PAYLOAD_MASK) as i64;
                RuntimeValue::Int((payload << 16) >> 16)
            }
            TAG_BOOL => RuntimeValue::Bool((self.0 & PAYLOAD_MASK) != 0),
            TAG_NOTHING => RuntimeValue::Nothing,
            TAG_HEAP => self.heap_ref().clone(),
            _ => unreachable!("invalid tag"),
        }
    }

    // ---- Scalar fast-path arithmetic (mirrors value.rs's inlined kernel) --
    //
    // These exist so the arith-parity test can exercise the narrow inline path
    // directly; they are NOT wired into the VM yet. The Int×Int path is the same
    // wrapping-i64 transcription `Value` carries (value.rs:136-247), and the
    // same `fast_path_kernel_differential` proof applies. The result re-boxes
    // through `Narrow::int`, so a wrap past the inline range stays lossless.
    // Non-inline-Int operands return `None` — route through `crate::semantics`.

    // Integer arithmetic is EXACT: the inline-i64 fast path uses `checked_*` and, only
    // on overflow, BOXES the promoted BigInt (via the kernel) behind the Heap tag — so
    // the narrow VM stays bit-identical to the tree-walker at the boundary.
    #[inline]
    pub fn int_add(&self, rhs: &Narrow) -> Option<Narrow> {
        match (self.as_inline_int(), rhs.as_inline_int()) {
            (Some(a), Some(b)) => Some(match a.checked_add(b) {
                Some(s) => Narrow::int(s),
                None => Narrow::from_runtime(
                    crate::semantics::arith::add(RuntimeValue::Int(a), RuntimeValue::Int(b))
                        .unwrap(),
                ),
            }),
            _ => None,
        }
    }

    #[inline]
    pub fn int_sub(&self, rhs: &Narrow) -> Option<Narrow> {
        match (self.as_inline_int(), rhs.as_inline_int()) {
            (Some(a), Some(b)) => Some(match a.checked_sub(b) {
                Some(s) => Narrow::int(s),
                None => Narrow::from_runtime(
                    crate::semantics::arith::subtract(RuntimeValue::Int(a), RuntimeValue::Int(b))
                        .unwrap(),
                ),
            }),
            _ => None,
        }
    }

    #[inline]
    pub fn int_mul(&self, rhs: &Narrow) -> Option<Narrow> {
        match (self.as_inline_int(), rhs.as_inline_int()) {
            (Some(a), Some(b)) => Some(match a.checked_mul(b) {
                Some(p) => Narrow::int(p),
                None => Narrow::from_runtime(
                    crate::semantics::arith::multiply(RuntimeValue::Int(a), RuntimeValue::Int(b))
                        .unwrap(),
                ),
            }),
            _ => None,
        }
    }

    #[inline]
    pub fn int_lt(&self, rhs: &Narrow) -> Option<bool> {
        match (self.as_inline_int(), rhs.as_inline_int()) {
            (Some(a), Some(b)) => Some(a < b),
            _ => None,
        }
    }

    #[inline]
    pub fn int_eq(&self, rhs: &Narrow) -> Option<bool> {
        match (self.as_inline_int(), rhs.as_inline_int()) {
            (Some(a), Some(b)) => Some(a == b),
            _ => None,
        }
    }

    /// The inline-only integer view: `Some` only when the value is an inlined
    /// (not boxed) integer. The arith/compare fast paths key off this so they
    /// never accidentally pull a boxed large int through the inline kernel
    /// (boxed large ints — including the wrapping spec's `i64::MIN`/`MAX` edges —
    /// route through `crate::semantics`, exactly as a non-Int operand would).
    #[inline]
    pub fn as_inline_int(&self) -> Option<i64> {
        if is_tagged(self.0) && tag_of(self.0) == TAG_INT {
            let payload = (self.0 & PAYLOAD_MASK) as i64;
            Some((payload << 16) >> 16)
        } else {
            None
        }
    }
}

impl Drop for Narrow {
    #[inline]
    fn drop(&mut self) {
        if self.is_heap() {
            let ptr = (self.0 & PAYLOAD_MASK) as *mut RuntimeValue;
            // SAFETY: Heap tag ⇒ payload is the unique owning box pointer; free
            // it exactly once. Drop runs once per Narrow, and the consuming
            // paths `forget` the husk before reclaiming the box themselves.
            unsafe { drop(Box::from_raw(ptr)) }
        }
    }
}

impl Clone for Narrow {
    #[inline]
    fn clone(&self) -> Self {
        if self.is_heap() {
            // Clone the pointee into a fresh box (matching RuntimeValue's shallow
            // Clone — Rc payloads bump their refcount, preserving identity).
            Narrow::heap(self.heap_ref().clone())
        } else {
            // Scalar / float: the u64 IS the value.
            Narrow(self.0)
        }
    }
}

impl std::fmt::Debug for Narrow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(n) = self.as_int() {
            write!(f, "Narrow::Int({n})")
        } else if let Some(x) = self.as_float() {
            write!(f, "Narrow::Float({x})")
        } else if let Some(b) = self.as_bool() {
            write!(f, "Narrow::Bool({b})")
        } else if self.is_nothing() {
            write!(f, "Narrow::Nothing")
        } else {
            write!(f, "Narrow::Heap({:?})", self.heap_ref())
        }
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/nanbox/nanbox_tests.rs"]
mod nanbox_tests;
