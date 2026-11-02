/-!
@case      T10-001
@tier      10 · Literals and primitive types
@title     Small `Nat` literal — `Literal::Nat(BigInt)`, not `Literal::Int(i64)`
@lean      Nat numeric literal, OfNat.ofNat, Nat.zero, Nat.succ
@expect    translate
@target    Term::Lit(Literal::Nat(BigInt::from_i64(n)))
@evidence  kernel/src/term.rs:38-44 (`Literal::Nat(BigInt)` — "a compact, accelerated form of
           the unary Peano numeral `Succ^n Zero`"); kernel/src/type_checker.rs:302
           (`Literal::Nat(_) => Ok(Term::Global("Nat"))` — a `Nat` literal's inferred type is
           the inductive `Nat`); kernel/src/type_checker.rs:296
           (`Literal::Int(_) | Literal::BigInt(_) => Ok(Term::Global("Int"))` — the OTHER
           literal family infers `Int`, a DIFFERENT type); kernel/src/prelude.rs:529-552
           (`register_nat`: `Nat : Type 0` is an inductive with constructors `Zero` and
           `Succ : Nat → Nat`); kernel/src/reduction.rs:330-345 (`extract_constructor`'s Peano
           bridge makes `Lit(Nat(n))` constructor-form so `Match` computes on it)
@guard     Catches the single most tempting literal bug in the tier: emitting a Lean `Nat`
           literal as `Literal::Int(n)` because the value fits an `i64` and `Int` is "the
           integer literal". `infer_type` then reports `Int` (type_checker.rs:296), not `Nat`.
           `Int` is an OPAQUE inductive with ZERO constructors (prelude.rs:354), so the emitted
           term can never be matched on, `extract_constructor` returns `None` for it, and every
           downstream `Match`/recursor over what the Lean source called a `Nat` is stuck. Worse,
           it is stuck *silently*: `normalize` returns the un-reduced redex rather than erroring
           (reduction.rs:52-69), so a proof by `rfl` degenerates to a definitional-equality
           failure at a site far from the literal.
@guard     Catches the inverse: emitting a Lean `Int` literal as `Literal::Nat`. A negative
           `Literal::Nat` is rejected outright by `infer_type` (type_checker.rs:297-301), but a
           NON-negative `Int` literal mis-emitted as `Nat` typechecks fine and then silently
           acquires Peano computation it should not have — `mod 4 2` on it will not reduce via
           the `Int` primitive hook (reduction.rs:444-466) because the operands are the wrong
           `Literal` variant, and `add`/`mul` become stuck applications of an axiom.
@guard     Catches carrying the literal as a spelled-out `Succ (Succ … Zero)` term. That is
           *definitionally* correct (term.rs:38-44) but is `n` heap nodes; a literal such as
           `1000000` emitted this way exceeds `normalize`'s 10000-step fuel
           (reduction.rs:52-69) during any comparison and the kernel then silently answers with
           a partially reduced term. The accelerated literal exists precisely to avoid this.
@guard     Catches assuming Lean's `OfNat` elaboration survives. `(5 : Nat)` is
           `@OfNat.ofNat Nat 5 (instOfNatNat 5)` in the elaborated `Expr`; the translator must
           see through the instance to the raw `Nat.lit`, not emit an application of a
           `OfNat_ofNat` global that LOGOS does not have. See T10-014.
@oracle    For each declaration below, the emitted term is `Term::Lit(Literal::Nat(v))` with `v`
           the source value, and LOGOS `infer_type` returns `Term::Global("Nat")`. A harness
           additionally asserts that NO `Literal::Int` appears anywhere in the output for this
           file, and that `def_eq(emitted(3), Succ (Succ (Succ Zero)))` holds.
-/

namespace T10_001

/-- The degenerate literal. In Lean this elaborates to `OfNat.ofNat 0`, whose unfolding is
    `Nat.zero`; in LOGOS it is `Lit(Nat(0))`, definitionally `Zero`. -/
def zeroLit : Nat := 0

/-- An ordinary small literal. -/
def fiveLit : Nat := 5

/-- A literal at the top of the `i64` fast path but still comfortably a `Nat`. -/
def largeButMachine : Nat := 9223372036854775807

/-- The literal must be definitionally equal to its Peano spelling — this is exactly what
    the `extract_constructor` bridge (reduction.rs:330-345) buys. -/
example : (3 : Nat) = Nat.succ (Nat.succ (Nat.succ Nat.zero)) := rfl

/-- A literal used as a `Match` scrutinee. This is the use that fails loudly if the literal
    was emitted as `Literal::Int`: `Int` has no constructors to match on. -/
def isZero (n : Nat) : Bool :=
  match n with
  | 0 => true
  | _ + 1 => false

example : isZero 0 = true := rfl
example : isZero 7 = false := rfl

/-- Literal arithmetic. In LOGOS there is NO `Nat` arithmetic primitive at all — the prelude's
    `add`/`sub`/`mul`/`div`/`mod` are all `Int → Int → Int` (prelude.rs:363-381) — so this must
    become a translator-emitted `Fix`/`Match` recursion over `Zero`/`Succ`, computing by the
    literal's Peano bridge. See T10-004 and T10-017. -/
example : (2 : Nat) + 3 = 5 := rfl

end T10_001
