/-!
@case      T10-009
@tier      10 · Literals and primitive types
@title     `UInt8` / `UInt32` / `UInt64` / `USize` — absent, and wrapping arithmetic has no
           LOGOS counterpart
@lean      UInt8, UInt32, UInt64, USize, UInt64.add (wrapping), UInt32.val, Fin (2^n)
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/prelude.rs:18-51 (`StandardLibrary::register` — no `UInt` of any width);
           kernel/src/prelude.rs:352-381 (`register_primitives`: the complete list of primitive
           types is `Int`, `Float`, `Text`, `Duration`, `Date`, `Moment`, and the complete list
           of arithmetic builtins is `add`/`sub`/`mul`/`div`/`mod : Int → Int → Int` —
           UNBOUNDED integers, with no modulus anywhere);
           kernel/src/reduction.rs:455-471 (the `i64` fast path uses `checked_add`/`checked_sub`/
           `checked_mul`, which return `None` ON OVERFLOW and fall through to the exact
           arbitrary-precision path at :486-500 — LOGOS arithmetic deliberately never wraps);
           kernel/src/term.rs:26-35 (`Literal::BigInt` exists precisely so that "`2.pow(63)` is
           the value `i64` cannot hold" is representable, base/src/numeric.rs:398-399);
           kernel/src/prelude.rs:3470-3525 (`register_bvec`: LOGOS's `BVec` — the only
           fixed-width thing in the kernel — is a hardware bit-vector, not a machine word; see
           T10-010)
@diag      E_TYPE_ABSENT_FROM_TARGET(UInt8|UInt32|UInt64|USize) — must name the width, state that
           LOGOS has no bounded integer type and that its `Int` arithmetic is exact and
           unbounded by construction (reduction.rs:486-500), and must NOT offer `Int` or `Nat`
           as a substitute.
@guard     Catches the "`UInt64` is just a `Nat`" re-encoding. It typechecks, it computes, and
           it is wrong at exactly one place: overflow. `(0xFFFFFFFFFFFFFFFF : UInt64) + 1` is
           `0` in Lean and `18446744073709551616` in the `Nat` encoding. Worse, LOGOS's
           arithmetic is *designed* not to wrap — `checked_add` failing routes to the exact
           `BigInt` path (reduction.rs:459, :491) rather than truncating — so there is no
           configuration of the prelude in which the encoding accidentally agrees. Every
           theorem about a hash, a checksum, a PRNG or a serialiser translated this way is
           silently false.
@guard     Catches encoding `UIntN` as `Fin (2^N)`, which is Lean's own definition. `Fin` is
           itself absent from LOGOS (T10-013), and rebuilding it needs `Subtype`, which is also
           absent — and the bound proof needs a `Nat` `<` relation that LOGOS's order axioms do
           not provide (they are all about `Int`, prelude.rs:202-340). The encoding is blocked
           three layers deep, and a translator that pushes through it ends up declaring all
           three with `add_inductive`/`add_constructor`, which are UNCHECKED (context.rs:253,
           :419) and will accept an ill-formed family without complaint. See T11-013.
@guard     Catches emitting the modulus as an explicit `mod 2^64` wrapper around `Int`
           arithmetic. That is arithmetically faithful, but `mod` in LOGOS is TRUNCATED toward
           zero with the remainder taking the DIVIDEND's sign (base/src/numeric.rs:354-375), so
           it is not the non-negative modulus the encoding needs; on any negative intermediate
           it produces a negative "unsigned" value. See T10-018.
@guard     Catches reaching this case only through `ByteArray`, `String.utf8ByteSize`,
           `Nat.toUInt64`, or `USize` in an array index. The rejection must fire on the
           elaborated `Expr`, where the width appears as a constant.
@oracle    Translator exits non-zero with E_TYPE_ABSENT_FROM_TARGET for every declaration below,
           and emits no LOGOS declaration. A harness asserts that no global named `UInt8`,
           `UInt16`, `UInt32`, `UInt64` or `USize` is ever registered.
-/

namespace T10_009

/-- The literals. Lean elaborates these through `OfNat` at the bounded type. -/
def byteLit : UInt8 := 255
def wordLit : UInt64 := 18446744073709551615

/-- WRAPPING. This is the behaviour no LOGOS encoding reproduces: LOGOS's arithmetic is exact
    and unbounded by construction (reduction.rs:486-500). -/
example : (255 : UInt8) + 1 = 0 := by decide
example : (18446744073709551615 : UInt64) + 1 = 0 := by native_decide

/-- Wrapping in the other direction. -/
example : (0 : UInt8) - 1 = 255 := by decide

/-- The literal is REDUCED MOD 2^N at elaboration, so the surface numeral and the value differ.
    A translator reading the numeral rather than the elaborated value gets this wrong too. -/
example : (256 : UInt8) = 0 := by decide

/-- Lean 4.14's own definition, showing the `BitVec` layer that LOGOS also lacks (T10-010).
    `UInt8` is `structure UInt8 where toBitVec : BitVec 8`. -/
example : (7 : UInt8).toBitVec = (7 : BitVec 8) := rfl

/-- `USize`, whose width is platform-dependent — so even the bound is not a fixed number the
    translator could bake in. -/
def sizeLit : USize := 4096

/-- The realistic entry point: widths arrive through byte-level code, not by name. -/
def checksum (bs : List UInt8) : UInt8 :=
  bs.foldl (fun acc b => acc + b) 0

example : checksum [200, 100] = 44 := by decide

end T10_009
