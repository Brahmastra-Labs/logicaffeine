/-!
@case      T10-006
@tier      10 · Literals and primitive types
@title     `Char` — absent from LOGOS, and not reconstructible because its invariant needs
           `UInt32`
@lean      Char, character literal 'a', Char.ofNat, Char.val, Char.valid, Char.isValidCharNat
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/prelude.rs:18-51 (`StandardLibrary::register` — the complete registration
           list; no `Char` anywhere); kernel/src/prelude.rs:352-381 (`register_primitives`
           registers exactly `Int`, `Float`, `Text`, `Duration`, `Date`, `Moment` as opaque
           types — no character type); kernel/src/term.rs:13-44 (the complete `Literal` enum:
           `Int`, `Float`, `Text`, `Duration`, `Date`, `Moment`, `BigInt`, `Nat` — there is NO
           character literal variant, so a `Char` cannot even be *injected*);
           kernel/src/context.rs:271-300 (`add_structure` hard-codes the structure's sort to
           `Type 0`, and it takes `fields` as plain types — there is no facility for a field
           whose type is a PROOF about an earlier field, which is exactly what Lean's `Char`
           needs)
@diag      E_TYPE_ABSENT_FROM_TARGET(Char) — must name `Char`, state that LOGOS has neither the
           type nor a character `Literal` variant, and state that the natural encoding
           (`{ val : UInt32 // val.isValidChar }`) is blocked twice over: `UInt32` is also
           absent (T10-009) and `Subtype` is also absent (T10-013). Must NOT silently re-encode
           `Char` as `Nat`.
@guard     Catches the "a `Char` is just a `Nat`" re-encoding. Emitting `'a'` as
           `Lit(Nat(97))` typechecks and even computes (the Peano bridge makes it
           constructor-form, reduction.rs:330-345). It is wrong in two ways that no LOGOS check
           can see. First, it DROPS THE VALIDITY INVARIANT: Lean's `Char` is a subtype carrying
           a proof that the code point is a scalar value, so `Char.ofNat 0xD800` is NOT the
           surrogate — Lean replaces it with `'\0'` — while the `Nat` encoding happily
           represents it. Second, it makes `Char` and `Nat` the SAME LOGOS type, so a Lean
           function `Nat → Char` becomes `Nat → Nat` and every use site typechecks even when
           the arguments are swapped.
@guard     Catches encoding `Char` as a one-character `Text`. `Text` is opaque with zero
           constructors (prelude.rs:356) and has no operations at all (see T10-005), so the
           encoding is a dead end: `Char.val`, `Char.toNat` and every ordering fact about
           characters become unstatable, and the translator has quietly turned a decidable type
           into an atom while reporting success.
@guard     Catches the translator that reaches this case only through `String`. Almost no real
           Lean source mentions `Char` directly; it arrives via `String.data`, `String.toList`,
           `String.any`, `String.map` or a `for c in s` loop. The rejection must fire on the
           elaborated `Expr`, where `Char` appears as a constant, not on the surface syntax.
@guard     Catches the scale error in the other direction from T06-009: rejecting `Char` is
           cheap for mathematical Lean and fatal for anything that manipulates text. A
           translator must report which of the two kinds of input it is looking at, rather than
           degrading `Char` to keep a success rate up.
@oracle    Translator exits non-zero with E_TYPE_ABSENT_FROM_TARGET(Char) for every declaration
           below, performs zero `add_inductive`/`add_structure` calls, and emits no `Literal`
           for a character. A harness asserts that `"Char"` never appears as a LOGOS global name
           anywhere in the corpus output.
-/

namespace T10_006

/-- The literal itself. In Lean's kernel a `Char` literal is *not* a `Expr.lit` — it elaborates
    to `Char.ofNat 97`, which unfolds to the structure instance. -/
def letterA : Char := 'a'

/-- The field access that exposes the machine-word layer LOGOS also lacks. -/
def codePoint : UInt32 := letterA.val

/-- The validity invariant: this is the content that a `Nat` encoding drops. `Char` is
    `structure Char where val : UInt32; valid : val.isValidChar`. -/
example : letterA.val.isValidChar := letterA.valid

/-- `Char.ofNat` is TOTAL in Lean precisely by clamping invalid code points — observable
    behaviour that a `Nat` re-encoding gets wrong. `0xD800` is a surrogate, not a scalar. -/
example : (Char.ofNat 0xD800).val = 0 := by decide

/-- Ordering and decidable equality on characters. -/
example : ('a' : Char) ≠ 'b' := by decide
example : ('a' : Char) < 'b' := by decide

/-- The realistic entry point: `Char` arrives through `String`, never by name. -/
def firstChar (s : String) : Char := s.get 0

/-- Recursion over a character list — the shape almost all string code has. -/
def countChar (c : Char) : List Char → Nat
  | [] => 0
  | d :: rest => (if c = d then 1 else 0) + countChar c rest

example : countChar 'a' ['a', 'b', 'a'] = 2 := rfl

end T10_006
