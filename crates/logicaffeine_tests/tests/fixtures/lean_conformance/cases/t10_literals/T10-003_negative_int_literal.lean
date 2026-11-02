/-!
@case      T10-003
@tier      10 · Literals and primitive types
@title     Negative `Int` literal — `int_lit` canonicalisation and the `negSucc` encoding gap
@lean      Int numeric literal, Neg.neg on Int, Int.negSucc, Int.ofNat
@expect    translate
@target    Term::Lit(int_lit(BigInt)) — i.e. `Literal::Int(n)` when `n` fits an `i64`, else
           `Literal::BigInt(n)`; type `Global("Int")`
@evidence  kernel/src/term.rs:50-55 (`int_lit`: `match n.to_i64() { Some(x) => Literal::Int(x),
           None => Literal::BigInt(n) }`); kernel/src/term.rs:26-35 (the canonicality invariant
           — one `Literal` per integer value, "so … definitional equality stays sound");
           kernel/src/type_checker.rs:296 (`Literal::Int(_) | Literal::BigInt(_)` both infer
           `Global("Int")`); kernel/src/prelude.rs:354 (`ctx.add_inductive("Int",
           Sort(Type 0))` — `Int` is an inductive with NO constructors, i.e. opaque);
           kernel/src/reduction.rs:474-506 (the `Int` primitive reduction hook: literal
           operands only, `i64` fast path then exact `BigInt` path, result re-canonicalised
           through `int_lit` at :499)
@guard     Catches emitting Lean's *constructor* encoding. Lean's `Int` is a two-constructor
           inductive `ofNat : Nat → Int` / `negSucc : Nat → Int`, and `(-5 : Int)` elaborates to
           `Int.negSucc 4`. A structurally faithful translator emits
           `App(Global("Int_negSucc"), Lit(Nat(4)))`. That compiles — `add_inductive` and
           `add_constructor` are UNCHECKED (context.rs:253, :419), so nothing stops you
           declaring `Int_negSucc` — but it is now a DIFFERENT `Int` from the prelude's, which
           is opaque and zero-constructor. The eight ring axioms and eight order axioms
           (prelude.rs:92-340) are all stated about the prelude's `Int`, so none of them apply
           to the shadow type, and the primitive reduction hook (reduction.rs:474-506) fires
           only on `Term::Lit` operands, so `add (Int_negSucc 4) (Int_ofNat 7)` never computes.
           The translation typechecks and proves nothing.
@guard     Catches the sign dropped at the literal boundary. Lean prints `-5` as a single token
           but its `Expr` is `Neg.neg (OfNat.ofNat 5)`; a translator that lowers `OfNat.ofNat 5`
           and then fails to see the enclosing `Neg.neg` emits `Literal::Int(5)`. The term still
           has type `Int` and still reduces, so every downstream arithmetic fact is off by a
           sign with no diagnostic anywhere.
@guard     Catches breaking `int_lit`'s canonicality in the negative direction. `i64::MIN`
           (−9223372036854775808) fits an `i64`, so it MUST be `Literal::Int`, while
           `−9223372036854775809` must be `Literal::BigInt`. An emitter that negates after
           demoting, or that routes every negative through the wide form, produces two distinct
           `Literal`s for one value and definitional equality of literals — which is settled by
           `==` on the `Literal` — silently fails.
@guard     Catches assuming `Int` literals behave like `Nat` literals. There is NO Peano bridge
           for `Int`: `extract_constructor` bridges only `Literal::Nat` (reduction.rs:330-345).
           An `Int` literal is never constructor-form, so it can never be a `Match` scrutinee.
           Any Lean proof that case-splits on `Int` must be rejected, not silently re-encoded.
@oracle    The emitted term is `Term::Lit` with the `int_lit`-canonical payload for the source
           value, `infer_type` returns `Global("Int")`, and the emitted term contains no
           `Int_negSucc`/`Int_ofNat` global. A harness asserts `normalize(add (-5) 3)` reduces
           to `Lit(Int(-2))`.
-/

namespace T10_003

/-- The canonical small negative literal. Elaborates through `Neg.neg`/`OfNat`, then reduces to
    `Int.negSucc 4`. -/
def minusFive : Int := -5

/-- Lean's kernel encoding, written out. These two are definitionally equal in Lean; in LOGOS
    only the literal form exists. -/
example : (-5 : Int) = Int.negSucc 4 := rfl

/-- Zero has one representation in Lean (`Int.ofNat 0`) and one in LOGOS (`Literal::Int(0)`).
    `-0` must canonicalise to it. -/
example : (-0 : Int) = (0 : Int) := rfl

/-- `i64::MIN` — fits the machine form, so `int_lit` must NOT widen it. -/
def i64Min : Int := -9223372036854775808

/-- One below `i64::MIN` — does not fit, so `int_lit` MUST widen it. -/
def belowI64Min : Int := -9223372036854775809

/-- Arithmetic that crosses zero. In LOGOS this is the opaque `sub : Int → Int → Int`
    declaration (prelude.rs:378) reduced by the primitive hook on literal operands. -/
example : (3 : Int) - 8 = -5 := by decide

/-- A case split on `Int`. Lean allows it because `Int` is an inductive with two constructors.
    LOGOS's `Int` has ZERO constructors, so `Match` over it is not expressible — a translator
    must reject this declaration rather than invent constructors for the prelude's `Int`. -/
def signOf (i : Int) : Int :=
  match i with
  | Int.ofNat 0 => 0
  | Int.ofNat (_ + 1) => 1
  | Int.negSucc _ => -1

example : signOf (-5) = -1 := rfl

end T10_003
