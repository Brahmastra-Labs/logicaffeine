//! An 8-byte NaN-boxed VM value (WS-F groundwork).
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

    #[inline]
    pub fn int_add(&self, rhs: &Narrow) -> Option<Narrow> {
        match (self.as_inline_int(), rhs.as_inline_int()) {
            (Some(a), Some(b)) => Some(Narrow::int(a.wrapping_add(b))),
            _ => None,
        }
    }

    #[inline]
    pub fn int_sub(&self, rhs: &Narrow) -> Option<Narrow> {
        match (self.as_inline_int(), rhs.as_inline_int()) {
            (Some(a), Some(b)) => Some(Narrow::int(a.wrapping_sub(b))),
            _ => None,
        }
    }

    #[inline]
    pub fn int_mul(&self, rhs: &Narrow) -> Option<Narrow> {
        match (self.as_inline_int(), rhs.as_inline_int()) {
            (Some(a), Some(b)) => Some(Narrow::int(a.wrapping_mul(b))),
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
mod nanbox_tests {
    use super::*;
    use crate::interpreter::{ListRepr, MapStorage};
    use std::cell::RefCell;
    use std::rc::Rc;

    // The i64 edge grid mirrors value.rs's `fast_path_kernel_differential`.
    // Note: i64::MIN/MAX and the ±2^47 boundary values straddle the inline range
    // on purpose — the round-trip must be lossless for ALL of them.
    const INT_EDGES: [i64; 16] = [
        i64::MIN,
        i64::MIN + 1,
        -4611686018427387904, // i64::MIN / 2
        INT_INLINE_MIN - 1,   // just below inline range → boxes
        INT_INLINE_MIN,       // inline boundary
        -1000003,
        -2,
        -1,
        0,
        1,
        2,
        1000003,
        INT_INLINE_MAX,       // inline boundary
        INT_INLINE_MAX + 1,   // just above inline range → boxes
        4611686018427387903,  // i64::MAX / 2
        i64::MAX,
    ];

    // The subset of INT_EDGES that stays inline — the only values the inline
    // arith fast path applies to.
    fn inline_edges() -> Vec<i64> {
        INT_EDGES
            .iter()
            .copied()
            .filter(|&n| (INT_INLINE_MIN..=INT_INLINE_MAX).contains(&n))
            .collect()
    }

    fn float_edges() -> Vec<f64> {
        vec![
            f64::NAN,
            -f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
            -0.0,
            f64::MIN_POSITIVE,       // smallest normal
            f64::MIN_POSITIVE / 2.0, // a subnormal
            f64::MAX,
            f64::MIN,
            1.0,
            -1.0,
            0.1,
            std::f64::consts::PI,
            -2.5,
        ]
    }

    /// Round-trip equality. RuntimeValue's PartialEq has no arm for the
    /// collection variants (they fall to `_ => false` — see interpreter.rs:376),
    /// so we compare scalars by value (floats by bits, since NaN != NaN) and
    /// collections/structs by their canonical display string.
    fn assert_runtime_eq(a: &RuntimeValue, b: &RuntimeValue, ctx: &str) {
        match (a, b) {
            (RuntimeValue::Float(x), RuntimeValue::Float(y)) => assert_eq!(
                x.to_bits(),
                y.to_bits(),
                "{ctx}: float bits diverged ({x} vs {y})"
            ),
            (RuntimeValue::Int(_), _)
            | (RuntimeValue::Bool(_), _)
            | (RuntimeValue::Nothing, _)
            | (RuntimeValue::Char(_), _)
            | (RuntimeValue::Duration(_), _)
            | (RuntimeValue::Date(_), _)
            | (RuntimeValue::Moment(_), _)
            | (RuntimeValue::Span { .. }, _)
            | (RuntimeValue::Time(_), _)
            | (RuntimeValue::Text(_), _) => assert_eq!(a, b, "{ctx}: round-trip diverged"),
            // Collections/structs: PartialEq is structurally absent, so compare
            // type + display (deterministic for the VM/tree-walker).
            _ => {
                assert_eq!(a.type_name(), b.type_name(), "{ctx}: type diverged");
                assert_eq!(
                    a.to_display_string(),
                    b.to_display_string(),
                    "{ctx}: display diverged"
                );
            }
        }
    }

    // ---- Test 4: size, the headline claim ---------------------------------

    #[test]
    fn nanbox_runtime_value_is_16_bytes() {
        // Documents WHY a 16-byte tagged union would be pointless: the enum it
        // would replace is ALREADY 16 bytes. The win must come from 8.
        assert_eq!(
            std::mem::size_of::<RuntimeValue>(),
            16,
            "RuntimeValue is expected to be 16 bytes"
        );
    }

    #[test]
    fn nanbox_narrow_is_8_bytes() {
        // The whole point of WS-F: a register-file value HALF the width of the
        // fat enum. 8 = one machine word, the NaN-boxed u64.
        assert_eq!(std::mem::size_of::<Narrow>(), 8, "Narrow must be 8 bytes");
        assert_eq!(std::mem::align_of::<Narrow>(), 8, "Narrow must be 8-aligned");
        assert!(
            std::mem::size_of::<Narrow>() < std::mem::size_of::<RuntimeValue>(),
            "Narrow ({}) must be smaller than RuntimeValue ({})",
            std::mem::size_of::<Narrow>(),
            std::mem::size_of::<RuntimeValue>(),
        );
    }

    // ---- Test 1: exhaustive round-trip over every variant -----------------

    #[test]
    fn nanbox_int_round_trips_over_edge_grid() {
        for &n in &INT_EDGES {
            let rv = RuntimeValue::Int(n);
            let narrow = Narrow::from_runtime(rv.clone());
            // Lossless for ALL i64, inline or boxed.
            assert_eq!(narrow.as_int(), Some(n), "as_int({n})");
            let back = narrow.into_runtime();
            assert_runtime_eq(&rv, &back, &format!("Int({n})"));
            // The boundary values land where we claim.
            let inline = (INT_INLINE_MIN..=INT_INLINE_MAX).contains(&n);
            assert_eq!(
                Narrow::int(n).is_inline_int(),
                inline,
                "Int({n}) inline-classification"
            );
        }
    }

    #[test]
    fn nanbox_float_round_trips_bit_exact_incl_nan_zero_inf_subnormal() {
        for f in float_edges() {
            let rv = RuntimeValue::Float(f);
            let n = Narrow::from_runtime(rv.clone());
            assert!(n.as_float().is_some(), "as_float present for {f}");
            // Bit-exactness (NaN canonicalises to a NaN; ±0.0, subnormal exact).
            let got = n.as_float().unwrap();
            if f.is_nan() {
                assert!(got.is_nan(), "NaN must round-trip as a NaN ({f})");
            } else {
                assert_eq!(got.to_bits(), f.to_bits(), "float bits for {f}");
            }
            let back = n.into_runtime();
            match (&rv, &back) {
                (RuntimeValue::Float(a), RuntimeValue::Float(b)) if a.is_nan() => {
                    assert!(b.is_nan(), "NaN round-trip")
                }
                _ => assert_runtime_eq(&rv, &back, &format!("Float({f})")),
            }
        }
    }

    #[test]
    fn nanbox_bool_and_nothing_round_trip() {
        for b in [true, false] {
            let rv = RuntimeValue::Bool(b);
            let back = Narrow::from_runtime(rv.clone()).into_runtime();
            assert_runtime_eq(&rv, &back, &format!("Bool({b})"));
            assert_eq!(Narrow::bool(b).as_bool(), Some(b));
        }
        let back = Narrow::from_runtime(RuntimeValue::Nothing).into_runtime();
        assert_runtime_eq(&RuntimeValue::Nothing, &back, "Nothing");
        assert!(Narrow::nothing().is_nothing());
    }

    #[test]
    fn nanbox_heap_types_round_trip_and_preserve_identity() {
        // Text.
        let text = RuntimeValue::Text(Rc::new("hello world".to_string()));
        let back = Narrow::from_runtime(text.clone()).into_runtime();
        assert_runtime_eq(&text, &back, "Text");

        // List (Ints repr) — reference identity preserved through the box.
        let list = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(vec![1, 2, 3]))));
        let narrow = Narrow::from_runtime(list.clone());
        assert!(narrow.is_heap());
        if let RuntimeValue::List(rc) = &list {
            rc.borrow_mut().push(RuntimeValue::Int(4));
        }
        if let Some(RuntimeValue::List(rc)) = narrow.as_heap() {
            assert_eq!(rc.borrow().len(), 4, "Narrow shares the original List's Rc");
        } else {
            panic!("heap payload was not a List");
        }
        let back = narrow.into_runtime();
        if let RuntimeValue::List(rc) = back {
            assert_eq!(rc.borrow().len(), 4);
        } else {
            panic!("List did not round-trip");
        }

        // Map.
        let mut storage = MapStorage::default();
        storage.insert(RuntimeValue::Int(7), RuntimeValue::Text(Rc::new("v".into())));
        let map = RuntimeValue::Map(Rc::new(RefCell::new(storage)));
        let back = Narrow::from_runtime(map.clone()).into_runtime();
        if let (RuntimeValue::Map(a), RuntimeValue::Map(b)) = (&map, &back) {
            assert_eq!(a.borrow().len(), b.borrow().len());
        } else {
            panic!("Map did not round-trip");
        }

        // Char, the temporal scalars, and the other collections all live on the
        // heap path here.
        for rv in [
            RuntimeValue::Char('x'),
            RuntimeValue::Duration(42),
            RuntimeValue::Date(20260620),
            RuntimeValue::Moment(123456789),
            RuntimeValue::Span { months: 3, days: 14 },
            RuntimeValue::Time(987654321),
            RuntimeValue::Tuple(Rc::new(vec![RuntimeValue::Int(1), RuntimeValue::Bool(true)])),
            RuntimeValue::Set(Rc::new(RefCell::new(vec![RuntimeValue::Int(9)]))),
        ] {
            let back = Narrow::from_runtime(rv.clone()).into_runtime();
            assert_runtime_eq(&rv, &back, "heap-path scalar/collection");
        }
    }

    #[test]
    fn nanbox_to_runtime_matches_into_runtime_without_consuming() {
        let cases = [
            RuntimeValue::Int(-99),
            RuntimeValue::Int(i64::MAX), // boxed large int
            RuntimeValue::Float(std::f64::consts::E),
            RuntimeValue::Bool(true),
            RuntimeValue::Nothing,
            RuntimeValue::Text(Rc::new("abc".into())),
        ];
        for rv in cases {
            let n = Narrow::from_runtime(rv.clone());
            let borrowed = n.to_runtime();
            let consumed = n.into_runtime();
            assert_runtime_eq(&borrowed, &consumed, "to_runtime vs into_runtime");
            assert_runtime_eq(&borrowed, &rv, "to_runtime vs original");
        }
    }

    // ---- Test 2: arithmetic parity with the kernel ------------------------

    #[test]
    fn nanbox_int_arith_is_bit_identical_to_kernel() {
        use crate::ast::stmt::BinaryOpKind;
        use crate::semantics::{arith, compare};
        let edges = inline_edges();
        for &a in &edges {
            for &b in &edges {
                let (na, nb) = (Narrow::int(a), Narrow::int(b));

                let add = na.int_add(&nb).unwrap().into_runtime();
                assert_eq!(
                    add,
                    arith::add(RuntimeValue::Int(a), RuntimeValue::Int(b)).unwrap(),
                    "add({a},{b})"
                );
                let sub = na.int_sub(&nb).unwrap().into_runtime();
                assert_eq!(
                    sub,
                    arith::subtract(RuntimeValue::Int(a), RuntimeValue::Int(b)).unwrap(),
                    "sub({a},{b})"
                );
                let mul = na.int_mul(&nb).unwrap().into_runtime();
                assert_eq!(
                    mul,
                    arith::multiply(RuntimeValue::Int(a), RuntimeValue::Int(b)).unwrap(),
                    "mul({a},{b})"
                );

                let lt = na.int_lt(&nb).unwrap();
                let lt_kernel = compare::compare(
                    BinaryOpKind::Lt,
                    &RuntimeValue::Int(a),
                    &RuntimeValue::Int(b),
                )
                .unwrap();
                assert_eq!(RuntimeValue::Bool(lt), lt_kernel, "lt({a},{b})");

                let eq = na.int_eq(&nb).unwrap();
                assert_eq!(
                    eq,
                    compare::values_equal(&RuntimeValue::Int(a), &RuntimeValue::Int(b)),
                    "eq({a},{b})"
                );
            }
        }
    }

    #[test]
    fn nanbox_arith_wrap_past_inline_range_stays_lossless() {
        // INT_INLINE_MAX + INT_INLINE_MAX overflows the inline range; the result
        // must re-box and still equal the kernel's wrapping-i64 answer.
        use crate::semantics::arith;
        let big = INT_INLINE_MAX;
        let n = Narrow::int(big).int_add(&Narrow::int(big)).unwrap();
        assert!(!n.is_inline_int(), "out-of-range sum must box");
        assert_eq!(
            n.into_runtime(),
            arith::add(RuntimeValue::Int(big), RuntimeValue::Int(big)).unwrap(),
            "wrapped sum equals the kernel"
        );
    }

    #[test]
    fn nanbox_non_inline_int_operands_do_not_take_the_int_fast_path() {
        let i = Narrow::int(2);
        let f = Narrow::float(0.5);
        let t = Narrow::from_runtime(RuntimeValue::Text(Rc::new("x".into())));
        let big = Narrow::int(i64::MAX); // boxed int — must NOT take inline path
        assert!(i.int_add(&f).is_none(), "Int+Float must not take int path");
        assert!(f.int_add(&i).is_none());
        assert!(i.int_mul(&t).is_none(), "Int*Text must not take int path");
        assert!(i.int_add(&big).is_none(), "inline+boxed must not take inline path");
        assert_eq!(i.int_lt(&f), None);
        assert_eq!(i.int_eq(&f), None);
    }

    // ---- Test 3: the NaN-boxing hazard, designed out ----------------------

    #[test]
    fn nanbox_nan_payload_is_not_aliased_with_heap() {
        // The classic NaN-boxing trap: a genuine f64::NAN must be a float, never
        // mistaken for a boxed/tagged value. We canonicalise NaN to a pattern
        // outside the tag region, so:
        let real_nan = Narrow::float(f64::NAN);
        assert!(real_nan.as_float().unwrap().is_nan(), "real NaN reads as a NaN float");
        assert!(!real_nan.is_heap(), "a real NaN must not look like a heap value");
        assert!(real_nan.as_int().is_none(), "a real NaN must not read back as Int");
        assert!(real_nan.as_bool().is_none());
        assert!(!real_nan.is_nothing());
        assert!(real_nan.as_heap().is_none());

        // A boxed value is unambiguously Heap and never reads as a float.
        let heap = Narrow::from_runtime(RuntimeValue::Text(Rc::new("nan".into())));
        assert!(heap.is_heap());
        assert!(heap.as_float().is_none(), "a heap value must never read as Float");
        assert!(heap.as_heap().is_some());

        // Distinct NaN bit-patterns (signaling, negative) all canonicalise to a
        // NaN that round-trips AS a NaN — and crucially never collide with the
        // tag region (which would make them read as Int/Bool/Heap).
        for bits in [0x7FF0_0000_0000_0001u64, 0xFFF0_0000_0000_0001, 0x7FF8_0000_0000_0000] {
            let f = f64::from_bits(bits);
            assert!(f.is_nan());
            let n = Narrow::float(f);
            assert!(n.as_float().unwrap().is_nan(), "{bits:#x} round-trips as NaN");
            assert!(n.as_int().is_none() && n.as_bool().is_none() && !n.is_heap());
        }

        // +inf / -inf are NOT NaN and live on the plain float path untouched.
        assert_eq!(Narrow::float(f64::INFINITY).as_float(), Some(f64::INFINITY));
        assert_eq!(Narrow::float(f64::NEG_INFINITY).as_float(), Some(f64::NEG_INFINITY));
        assert!(!Narrow::float(f64::INFINITY).is_heap());

        // The all-bits-set integer (Int(-1)) is the worst case for collision:
        // its 48-bit payload is all-ones. It must NOT alias CANONICAL_NAN — it
        // is `0xFFF8_FFFF_FFFF_FFFF` (high nibble 0xFFF8), an inline Int, and
        // reads back as -1, not as a NaN float.
        let minus_one = Narrow::int(-1);
        assert_eq!(minus_one.0, 0xFFF8_FFFF_FFFF_FFFF, "Int(-1) bit pattern");
        assert_ne!(minus_one.0, CANONICAL_NAN, "Int(-1) must not alias the NaN");
        assert_eq!(minus_one.as_int(), Some(-1));
        assert!(minus_one.as_float().is_none());

        // Exhaustively: no inline scalar of any tag, with any 48-bit payload
        // extreme, can reach CANONICAL_NAN — the high nibble is bounded by tag.
        for tag in 0u64..=3 {
            let hi = QNAN | (tag << TAG_SHIFT) | PAYLOAD_MASK;
            assert!(hi < CANONICAL_NAN, "tag {tag} max value must stay below the NaN");
        }
    }

    // ---- Memory-safety stress: Drop/Clone over the heap path --------------

    #[test]
    fn nanbox_clone_and_drop_are_memory_safe_for_heap() {
        let rc = Rc::new(RefCell::new(ListRepr::Ints(vec![1, 2, 3])));
        let outer = Rc::clone(&rc); // strong = 2
        let n1 = Narrow::from_runtime(RuntimeValue::List(Rc::clone(&rc))); // strong = 3
        assert_eq!(Rc::strong_count(&rc), 3);

        let n2 = n1.clone(); // RuntimeValue::List clone bumps the Rc → strong = 4
        assert_eq!(Rc::strong_count(&rc), 4);

        drop(n2); // → 3
        assert_eq!(Rc::strong_count(&rc), 3);

        let _ = n1.into_runtime(); // moves box out, drops the List → 2
        assert_eq!(Rc::strong_count(&rc), 2);

        drop(outer); // → 1
        assert_eq!(Rc::strong_count(&rc), 1);
    }

    #[test]
    fn nanbox_boxed_large_int_is_memory_safe_through_clone_and_drop() {
        // A large int boxes; clone allocates a fresh box, drop frees exactly
        // once. Miri-style discipline; here we just exercise the path heavily.
        let n = Narrow::int(i64::MIN);
        assert!(!n.is_inline_int());
        let c = n.clone();
        assert_eq!(c.as_int(), Some(i64::MIN));
        assert_eq!(n.as_int(), Some(i64::MIN));
        drop(c);
        assert_eq!(n.into_runtime(), RuntimeValue::Int(i64::MIN));
    }

    #[test]
    fn nanbox_scalar_clone_is_a_value_copy() {
        let a = Narrow::int(123);
        let b = a.clone();
        assert_eq!(a.as_int(), b.as_int());
        let c = Narrow::float(f64::NAN);
        let d = c.clone();
        assert!(c.as_float().unwrap().is_nan() && d.as_float().unwrap().is_nan());
        let e = Narrow::nothing();
        assert!(e.clone().is_nothing());
    }
}
