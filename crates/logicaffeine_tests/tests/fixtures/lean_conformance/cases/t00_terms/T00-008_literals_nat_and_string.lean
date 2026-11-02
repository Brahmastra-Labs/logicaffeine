/-!
@case      T00-008
@tier      0 · Kernel term layer
@title     `Expr.lit` — `Nat` literals and `String` literals land in different LOGOS worlds
@lean      Expr.lit (.natVal n), Expr.lit (.strVal s), OfNat.ofNat elaboration
@expect    translate
@target    `Expr.lit (.natVal n)`  ⟼ `Term::Lit(Literal::Nat(BigInt))`, typed `Global("Nat")`
           `Expr.lit (.strVal s)`  ⟼ `Term::Lit(Literal::Text(String))`, typed `Global("Text")`
@evidence  kernel/src/term.rs:13-44 (`Literal::{Int(i64), Float, Text(String), Duration, Date,
           Moment, BigInt, Nat(BigInt)}` — `Nat` and `Int` are SEPARATE literal constructors, and
           there is no `String`/`Char` literal, only `Text`);
           kernel/src/type_checker.rs:294-312 (`Lit` typing: `Nat(_) ↦ Global("Nat")`,
           `Int(_)`/`BigInt(_) ↦ Global("Int")`, `Text(_) ↦ Global("Text")`; a negative
           `Literal::Nat` is a `CertificationError`);
           kernel/src/prelude.rs (`Nat` is an INDUCTIVE with constructors `Zero`/`Succ`; `Int` is
           an OPAQUE zero-constructor inductive whose ring and order laws are AXIOMS at
           prelude.rs:92-340; `Text` is likewise opaque with no operations at all);
           kernel/src/reduction.rs:330-345 and kernel/src/type_checker.rs:1480-1493 (the Peano
           bridge: `Literal::Nat(n)` is DEFINITIONALLY `Succ^n Zero`, peeled one `Succ` at a time)
@guard     Catches `Nat` literal ⟼ `Literal::Int`. It typechecks — `Literal::Int` is a perfectly
           good literal — but its inferred type is `Global("Int")` (type_checker.rs:296), and in
           LOGOS `Int` is a completely different kind of object from `Nat`: opaque, zero
           constructors, no `Zero`/`Succ`, arithmetic only via the axioms at prelude.rs:92-340.
           The consequence is not a type error at the literal; it is that every later `Match` on
           that value has no constructors to match, every `Fix` over it fails the structural
           guard, and the Peano bridge (type_checker.rs:1480-1493) never fires — so `2 + 2 = 4`
           becomes unprovable by computation and provable only from the ring axioms, if at all.
@guard     Catches translating Lean's `String` to a LOGOS `String`. There is none. The only text
           carrier is `Literal::Text`, typed `Global("Text")`, an opaque prelude inductive with
           ZERO operations — no `++`, no `length`, no decidable equality (LOGOS_FACTS: `String`
           operations absent). A translator that emits `Text` and carries on will produce
           statements it can never prove and cannot even state (`s.length` has no target). The
           literal itself is `translate`; anything that *operates* on it is `reject` — T10-016.
@guard     Catches failing to see through `OfNat`. A numeral in Lean source is elaborated to
           `@OfNat.ofNat Nat 3 instOfNatNat`, i.e. an APPLICATION, not an `Expr.lit`, unless the
           elaborator has already reduced it. A translator that matches only on `Expr.lit` emits
           a four-argument spine referring to `OfNat`, `instOfNatNat` and `Nat.lit` — none of
           which exist in the LOGOS prelude — instead of a literal. It must run `Nat.fromExpr?`
           / `whnf` first. The dual error is *over*-eager: seeing through `OfNat` for a
           non-`Nat` type and emitting a `Literal::Nat` where the Lean type was `Int` or a
           user type with its own `OfNat` instance.
@guard     Catches BigInt canonicalisation. `Literal::Nat` carries a `BigInt`, so a literal
           beyond `i64` is representable — but `Literal::Int` is `i64` (term.rs:14) and
           `Literal::BigInt` is a *third* variant that also types as `Int`. Round-tripping a
           large `Nat` through `Literal::Int` truncates silently. See T10-002.
@oracle    Every translated Lean `Nat` literal emits `Literal::Nat` with the exact `BigInt`
           value and infers `Global("Nat")`; every Lean `String` literal emits `Literal::Text`
           and infers `Global("Text")`; and `def_eq(Lit(Nat(3)), Succ (Succ (Succ Zero)))` holds,
           confirming the Peano bridge is reachable from the emitted form.
-/

namespace T00_008

/-- Small `Nat` literals. In Lean these are `OfNat.ofNat` applications that reduce to
    `Expr.lit (.natVal _)`; the translator must land on `Literal::Nat`. -/
def zeroLit : Nat := 0
def smallLit : Nat := 3
def mediumLit : Nat := 1000000

/-- Beyond `i64`, so `Literal::Int` (an `i64`, term.rs:14) would truncate. -/
def hugeLit : Nat := 123456789012345678901234567890

/-- The Peano bridge: a literal must be definitionally `Succ^n Zero`. Lean proves this by
    `Nat.succ` unfolding; LOGOS proves it by the bridge at type_checker.rs:1480-1493. -/
example : (3 : Nat) = Nat.succ (Nat.succ (Nat.succ Nat.zero)) := rfl

/-- A `Nat` literal used as a *match discriminant*, which is only possible because `Nat` is an
    inductive. If the literal had been emitted as `Literal::Int`, this has no translation at
    all — `Int` has zero constructors. -/
def pred (n : Nat) : Nat :=
  match n with
  | 0 => 0
  | Nat.succ k => k

example : pred 5 = 4 := rfl

/-- An `Int` literal. A DIFFERENT LOGOS world: `Literal::Int`, typed `Global("Int")`, opaque. -/
def intLit : Int := 7
def negIntLit : Int := -7

/-- The conflation trap in one place: the same numeral at two Lean types. -/
def sameNumeralAsNat : Nat := 42
def sameNumeralAsInt : Int := 42

/-- String literals. `Literal::Text`, and nothing may be done with them. -/
def emptyStr : String := ""
def helloStr : String := "hello"
def unicodeStr : String := "λ ∀ ε"

/-- A literal in a dependent position: the index of a `Fin`. The literal must still be a
    `Literal::Nat`, because the `Fin` bound is compared by the Peano bridge. -/
def finAtLit : Fin 5 := ⟨3, by omega⟩

end T00_008
