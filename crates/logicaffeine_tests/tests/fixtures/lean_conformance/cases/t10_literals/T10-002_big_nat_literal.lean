/-!
@case      T10-002
@tier      10 · Literals and primitive types
@title     `Nat` literal beyond `i64` — the `BigInt` path and its canonicalisation invariant
@lean      Nat literal exceeding 2^63-1, Nat.pow, large numeral elaboration
@expect    translate
@target    Term::Lit(Literal::Nat(BigInt)) — `Literal::Nat` ALWAYS carries a `BigInt`, so the
           small and large cases are the same constructor and no widening decision arises
@evidence  kernel/src/term.rs:38-44 (`Nat(BigInt)` — one arm, arbitrary precision, "the value is
           non-negative"); kernel/src/term.rs:26-35 (`BigInt` is a PARALLEL *integer*
           representation and is CANONICAL: "it holds only values `to_i64()` cannot, so every
           integer has a unique `Literal` … and definitional equality stays sound");
           kernel/src/term.rs:50-55 (`int_lit` is the ONLY sanctioned way to build an INTEGER
           literal from a `BigInt`, demoting to `Int(i64)` when it fits);
           kernel/src/type_checker.rs:1485 (`(Lit(Nat(x)), Lit(Nat(y))) => x == y` — two `Nat`
           literals are definitionally equal iff their `BigInt`s compare equal);
           base/src/numeric.rs:354-375 (`BigInt::div_rem`, and the limb representation the
           comparison runs over)
@guard     Catches the canonicalisation bug that breaks definitional equality. For INTEGERS the
           kernel has two representations — `Literal::Int(i64)` and `Literal::BigInt` — and the
           soundness of literal `def_eq` rests on `int_lit`'s invariant that a value fitting an
           `i64` is NEVER stored as a `BigInt` (term.rs:26-35, :50-55). A translator that emits
           `Literal::BigInt(BigInt::from_i64(3))` for a small `Int` produces a term that is
           *not* `PartialEq`-equal to `Literal::Int(3)`; `def_eq_normalized`'s literal arms
           compare by `==` on the `Literal` (type_checker.rs:1485 for `Nat`), so `3` fails to
           equal `3`. The symptom is an unprovable `rfl` with two identical-looking printouts.
@guard     Catches the truncating emitter. Reading a Lean `Nat` literal through `u64`/`i64` and
           then calling `BigInt::from_i64` silently wraps for any value ≥ 2^63. The resulting
           term still typechecks (`infer_type` returns `Nat` for any non-negative `Nat`
           literal, type_checker.rs:302) and still computes — at the WRONG value. Nothing in
           the kernel can detect it; the translator must read Lean's literal as arbitrary
           precision end to end.
@guard     Catches emitting a negative value into `Literal::Nat`. `infer_type` rejects it
           (type_checker.rs:297-301), but the more dangerous path is `extract_constructor`,
           which clamps a negative `Nat(n)` to `Zero` so the `Succ`-peel terminates rather than
           descending toward −∞ (reduction.rs:337-341). A mis-signed literal therefore does not
           crash — it silently *computes as zero* inside a `Match`.
@guard     Catches "just spell it in Peano". `Succ^n Zero` for `n = 2^64` is not representable
           at all; even the fuel-bounded normaliser (10000 steps, reduction.rs:52-69) cannot
           peel it. The accelerated literal is mandatory above a few thousand.
@oracle    The emitted `Literal::Nat` payload equals the source value compared as an
           arbitrary-precision decimal string (not as `i64`), and `infer_type` returns
           `Global("Nat")`. A harness additionally asserts the `int_lit` invariant over the
           whole corpus: no emitted `Literal::BigInt` has a `to_i64()` that is `Some`.
-/

namespace T10_002

/-- 2^63 — one past the top of the signed 64-bit range, so an `i64`-based emitter overflows. -/
def justPastI64 : Nat := 9223372036854775808

/-- 2^64 — one past the top of the *unsigned* 64-bit range too, so a `u64`-based emitter
    overflows as well. -/
def justPastU64 : Nat := 18446744073709551616

/-- A value with no relation to any machine-word boundary, to catch an emitter that special-
    cases only the powers of two. -/
def wideDecimal : Nat := 123456789012345678901234567890

/-- Equality of two spellings of the same large value must be `rfl`. This is the test that
    fails when the emitted literal is non-canonical or truncated. -/
example : (18446744073709551616 : Nat) = 18446744073709551615 + 1 := by decide

/-- A large literal that is still *below* the boundary, so it MUST come out as the small form.
    Emitting this as a wide representation is the canonicalisation bug above. -/
def justBelowI64 : Nat := 9223372036854775807

/-- Distinctness: two large literals that differ in their last digit must not be
    definitionally equal. An emitter that truncates to 64 bits makes many such pairs
    accidentally equal. -/
example : (123456789012345678901234567890 : Nat) ≠ 123456789012345678901234567891 := by decide

end T10_002
