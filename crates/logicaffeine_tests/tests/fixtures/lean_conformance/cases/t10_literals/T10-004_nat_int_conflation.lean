/-!
@case      T10-004
@tier      10 · Literals and primitive types
@title     `Nat`/`Int` conflation — LOGOS's `Nat` is an INDUCTIVE, its `Int` is OPAQUE, and the
           two compute by completely different mechanisms
@lean      Nat, Int, Int.ofNat coercion (↑), Nat.cast, Int.toNat, Int.natAbs
@expect    reject
@target    (none — no sound translation of a term that treats the two as one type)
@evidence  kernel/src/prelude.rs:529-552 (`register_nat`: `Nat : Type 0` inductive, constructors
           `Zero : Nat` and `Succ : Nat → Nat` — a genuine Peano inductive that `Match`
           eliminates); kernel/src/prelude.rs:354 (`ctx.add_inductive("Int", Sort(Type 0))` with
           NO `add_constructor` call anywhere for it — an inductive of arity 0 and zero
           constructors, i.e. an opaque carrier); kernel/src/prelude.rs:363-381 (`add`, `sub`,
           `mul`, `div`, `mod` are `add_declaration`s of type `Int → Int → Int` — AXIOMS, not
           definitions, and there is no `Nat` counterpart anywhere in the prelude);
           kernel/src/prelude.rs:92-191 (8 ring axioms, all about `Int`);
           kernel/src/prelude.rs:202-340 (8 order axioms, all about `Int`);
           kernel/src/reduction.rs:444-506 (`try_primitive_reduce`: `Int` arithmetic computes by
           a PRIMITIVE HOOK on `Term::Lit` operands); kernel/src/reduction.rs:330-345
           (`Nat` computes by the Peano bridge in `extract_constructor`, i.e. by ι-reduction);
           kernel/src/type_checker.rs:296-302 (`Literal::Int`/`BigInt` infer `Int`;
           `Literal::Nat` infers `Nat` — two different globals)
@diag      E_NAT_INT_DISTINCT — must name both LOGOS types, state that `Nat` is a two-
           constructor inductive whose arithmetic must be emitted as `Fix`/`Match` recursion
           while `Int` is a zero-constructor opaque carrier whose arithmetic is an axiom with a
           reduction hook, and refuse to identify them. Must NOT offer a coercion.
@guard     THIS IS THE HIGHEST-VALUE REJECTION IN TIER 10. The tempting wrong move is to notice
           that LOGOS's *arithmetic* lives on `Int` — `add`/`sub`/`mul` are `Int → Int → Int`
           and there is literally no `Nat` addition in the prelude — and therefore to translate
           every Lean `Nat` as LOGOS `Int`. That compiles, reduces, and gives the right answers
           on every non-negative example, which is exactly what makes it lethal: it is wrong
           precisely at `Nat` subtraction (see T10-017), where Lean truncates at zero and
           LOGOS's `sub` goes negative, and at induction, where the Lean source's
           `Nat.rec`/`match` has no LOGOS counterpart on a zero-constructor type.
@guard     Catches the mirror move: translating Lean `Int` as LOGOS `Nat` because "the literal
           bridge is nicer". A negative `Literal::Nat` is refused by `infer_type`
           (type_checker.rs:297-301) — but only when it is a *literal*. A computed negative
           value flows into `extract_constructor`, which CLAMPS it to `Zero` so the `Succ`-peel
           terminates (reduction.rs:337-341). So `-1` silently becomes `0` inside a `Match`,
           with no error at any point.
@guard     Catches translating Lean's coercion `↑n : Int` as the identity. Because the two
           LOGOS types are distinct globals, the identity is ill-typed and would be caught —
           UNLESS the translator has already conflated them, in which case the coercion
           disappears and with it every proof obligation the coercion carried (`Int.toNat_of_nonneg`
           and friends). The coercion needs a genuine emitted function, and there is no
           `Nat → Int` primitive in the prelude to build it from.
@guard     Catches the "both are literals, so both are `Literal`" reasoning. They are different
           `Literal` VARIANTS with different typing rules (type_checker.rs:296 vs :302) and
           different reduction mechanisms: `Literal::Nat` participates in ι-reduction via the
           Peano bridge and is NOT accepted by `try_primitive_reduce` (which matches only
           `Literal::Int`/`BigInt`, reduction.rs:459-500); `Literal::Int` participates in the
           primitive hook and is NOT constructor-form. A term mixing them is stuck in both
           directions at once and `normalize` reports that as "already normal".
@oracle    Translator exits non-zero with E_NAT_INT_DISTINCT for every declaration below and
           emits no LOGOS output. A harness additionally asserts, over the whole corpus, that
           no emitted term applies `Global("add")`/`"sub"`/`"mul"`/`"div"`/`"mod"` to a
           subterm whose inferred type is `Global("Nat")`.
-/

namespace T10_004

/-- The two types are not interchangeable in Lean either; the coercion is explicit. -/
def toInt (n : Nat) : Int := (n : Int)

/-- The coercion is not injective-on-the-nose in the other direction: `Int.toNat` truncates
    every negative integer to `0`. A translator that identified the types would have to make
    this the identity, which is false. -/
def backToNat (i : Int) : Nat := i.toNat

example : backToNat (-7) = 0 := rfl
example : backToNat 7 = 7 := rfl

/-- `Int.natAbs` — the other direction, also not the identity. -/
example : (-7 : Int).natAbs = 7 := rfl

/-- The decisive witness: the SAME surface syntax `3 - 8` means different things at the two
    types, and the difference is observable. Under the conflation, one of these two lines
    becomes false in LOGOS while remaining true in Lean. -/
example : (3 : Nat) - 8 = 0 := rfl
example : (3 : Int) - 8 = -5 := by decide

/-- `Nat` supports structural recursion because it is an inductive with constructors. LOGOS's
    `Nat` does too. LOGOS's `Int` does not — it has zero constructors. -/
def double : Nat → Nat
  | 0 => 0
  | n + 1 => double n + 2

example : double 4 = 8 := rfl

/-- A round-trip that is NOT the identity, pinning the two directions together. -/
example (n : Nat) : ((n : Int)).toNat = n := rfl

/-- A proposition true of `Nat` and false of `Int`: every natural is `≥ 0`. Under the
    conflation this becomes a claim about `Int` that no LOGOS order axiom proves
    (prelude.rs:202-340 contains `le_total` but nothing asserting a lower bound). -/
example (n : Nat) : 0 ≤ n := Nat.zero_le n

end T10_004
