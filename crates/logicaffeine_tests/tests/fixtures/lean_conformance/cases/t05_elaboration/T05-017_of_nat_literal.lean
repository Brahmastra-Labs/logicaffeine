/-!
@case      T05-017
@tier      5 · Elaboration surface
@title     `OfNat` literal elaboration — LOGOS literals are typed by KIND, not by instance
@lean      OfNat, numeric literals at a user type, `@[default_instance] instOfNatNat`,
           `(5 : Int)` vs `(5 : Nat)` vs `(5 : MyNum)`
@expect    degrade
@target    For a literal Lean elaborated at `Nat`: `Term::Lit(Literal::Nat(BigInt))`.
           For one at `Int`: `Term::Lit(Literal::Int(i64))` / `Literal::BigInt`.
           For one at a USER type: the fully-applied `OfNat.ofNat` term from Lean's `Expr`,
           i.e. the instance's projection applied to the raw literal — never a bare
           `Term::Lit`, because LOGOS has no way to give a literal a user type.
@evidence  kernel/src/term.rs:13-44 (`Literal` has exactly `Int`, `Float`, `Text`, `Duration`,
           `Date`, `Moment`, `BigInt`, `Nat` — a closed set of BUILT-IN kinds);
           kernel/src/type_checker.rs:294-311 (`Term::Lit(lit)`'s type is decided by a `match`
           on the literal's KIND: `Literal::Nat` ⟼ `Global("Nat")`, `Literal::Int`/`BigInt` ⟼
           `Global("Int")`, and so on — the surrounding context is never consulted, and there
           is no `OfNat` hook);
           kernel/src/reduction.rs:329-346 (the Peano bridge: `Literal::Nat(n)` is
           constructor-form `Succ^n Zero` for `Nat`'s OWN registered `Zero`/`Succ` — this is
           what makes `Literal::Nat` specifically a `Nat` and nothing else);
           kernel/src/context.rs:184-188 (`add_instance` is a flat `Vec`, first-match-wins —
           there is no `@[default_instance]` notion, so nothing reproduces Lean's defaulting);
           LOGOS_FACTS: LOGOS's `Nat` is an INDUCTIVE while `Int` is OPAQUE with axiomatised
           ring and order laws — "the two are completely different kinds of thing"
@loss      LITERAL_POLYMORPHISM. In Lean a numeral is `@OfNat.ofNat α n inst`: one surface form
           that denotes a value of any `OfNat` type. In LOGOS a literal's type is its kind, so
           only `Nat`, `Int`, `Float`, `Text`, `Duration`, `Date` and `Moment` have literal
           forms. Every other Lean numeral must be emitted as an explicit application of the
           instance's value to a `Literal::Nat`, which (a) is a bigger term, (b) does not
           reduce the way a literal does, and (c) loses the fact that it was written as a
           numeral.
@loss      DEFAULT_INSTANCE_ABSENCE. Which `OfNat` instance a bare numeral got is a decision
           Lean made by postponement plus `@[default_instance]` (T05-015). LOGOS has no
           default-instance mechanism at all, so the decision is not reproducible on the LOGOS
           side and must be recorded with the literal.
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax`.** The surface
           text `5` is the same in all six examples below and elaborates to six different
           `Expr`s. A translator on `Syntax` must reimplement `OfNat` resolution AND Lean's
           defaulting table to recover which; a translator on `Expr` reads the instance off the
           application.
@guard     Catches emitting `Literal::Nat` for a literal Lean elaborated at `Int`. The emitted
           term then has type `Nat` (type_checker.rs:294-311), which is an INDUCTIVE with a
           `Zero`/`Succ` computation rule, where the source meant `Int`, which is OPAQUE with
           only axiomatised ring laws. Every `rfl` the source proved by computation stops
           holding, and — because LOGOS is cumulative and both are `Type 0` — many surrounding
           terms still typecheck. This is T10-004 reached through the elaborator.
@guard     Catches emitting a bare `Literal::Nat` for a literal at a USER type. There is no
           `Literal` variant for it, so the emitted term has type `Nat` where the source had
           `MyNum`. If `MyNum` is a one-field wrapper around `Nat`, `try_structure_eta`
           (type_checker.rs:1391-1417) may make the two definitionally equal inside `def_eq`
           — so the mistranslation typechecks in exactly the case where it is most tempting.
@guard     Catches emitting a NEGATIVE `Literal::Nat`. `infer_type` rejects it explicitly
           (type_checker.rs:298-303: "a `Nat` literal must be non-negative"), and
           `extract_constructor` (reduction.rs:336-345) collapses a negative literal to `Zero`
           so the Peano peel terminates — so a negative `Nat` literal that slips past a
           different code path computes as `0` rather than failing.
@guard     Catches losing the `BigInt` boundary. `Literal::Int(i64)` and `Literal::BigInt` are
           different variants (term.rs:13-44) that both infer to `Global("Int")`; a translator
           that canonicalises them inconsistently produces two terms that are equal in meaning
           but not syntactically, and LOGOS's `types_equal`/`unify` compare literals by
           equality (elaborate.rs:275) — so they will fail to unify. See T10-002.
@guard     Catches assuming the `OfNat` instance can be *registered* and then forgotten.
           `add_instance` (context.rs:184-188) lets LOGOS-side elaboration find it, but the
           kernel never consults it for a `Term::Lit`: the literal's type is still its kind.
@oracle    Every emitted literal's `infer_type` equals the translation of Lean's type for that
           literal; no emitted `Literal::Nat` is negative; literals at user types appear as
           explicit instance applications, not as `Term::Lit`; and the loss record contains
           LITERAL_POLYMORPHISM naming each user-typed numeral.
-/

namespace T05_017

/-- The same surface numeral at three different types. -/
def atNat : Nat := 5
def atInt : Int := 5
def atFloat : Float := 5

/-- A user `OfNat` instance: the numeral becomes `@OfNat.ofNat MyNum 3 inst`. -/
structure MyNum where
  v : Nat

instance : OfNat MyNum 3 := ⟨⟨3⟩⟩

def atMyNum : MyNum := 3

example : atMyNum.v = 3 := rfl

/-- A `OfNat` instance that is polymorphic in the literal — one instance covering every
    numeral, so the literal value appears as an argument. -/
instance instAllNums (n : Nat) : OfNat MyNum n := ⟨⟨n⟩⟩

def atMyNum7 : MyNum := 7

example : atMyNum7.v = 7 := rfl

/-- A one-field wrapper around `Nat`, where structure eta makes the wrong translation
    definitionally equal to the right one inside `def_eq`. -/
structure WrapNat where
  unwrap : Nat

instance (n : Nat) : OfNat WrapNat n := ⟨⟨n⟩⟩

def wrapped : WrapNat := 42

example : wrapped.unwrap = 42 := rfl

/-- A NEGATIVE literal — `Neg.neg` applied to a `Nat` literal, which is an `Int` in LOGOS and
    has no `Literal::Nat` form at all. -/
def negFive : Int := -5

example : negFive + 5 = 0 := rfl

/-- A literal beyond `i64`, which must use `Literal::BigInt`. -/
def huge : Nat := 123456789012345678901234567890

/-- A bare numeral with NO ascription — Lean's `@[default_instance]` picks `Nat`. -/
def bare := 5

example : bare = (5 : Nat) := rfl

/-- The literal's type decided by a function's parameter type rather than an ascription. -/
def takesInt (n : Int) : Int := n + 1

example : takesInt 5 = 6 := rfl

end T05_017
