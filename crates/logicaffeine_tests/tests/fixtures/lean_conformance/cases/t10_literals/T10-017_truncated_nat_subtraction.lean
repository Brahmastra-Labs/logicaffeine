/-!
@case      T10-017
@tier      10 · Literals and primitive types
@title     Truncated `Nat` subtraction — Lean saturates at zero; LOGOS has no `Nat` subtraction
           at all, and its `sub` goes negative
@lean      HSub.hSub on Nat, Nat.sub, Nat.pred, `n - m` with `m > n`
@expect    degrade
@target    A translator-emitted structural recursion over the prelude's `Nat`, i.e.
           `add_definition("Nat_sub", Nat → Nat → Nat, Fix{ … Match … })` implementing
           `sub n 0 = n`, `sub 0 (Succ _) = Zero`, `sub (Succ n) (Succ m) = sub n m`.
           The prelude's `sub` is NOT the target.
@evidence  kernel/src/prelude.rs:363-381 (`register_primitives`: `sub` is
           `add_declaration("sub", Int → Int → Int)` — an AXIOM at type `Int`, with no `Nat`
           counterpart anywhere in the prelude); kernel/src/prelude.rs:529-552
           (`register_nat` registers `Nat`, `Zero` and `Succ` and NOTHING else — LOGOS ships no
           `Nat` arithmetic at all); kernel/src/reduction.rs:455-471 (the `sub` reduction is
           `a.checked_sub(*b)` on `Literal::Int` operands, falling through to
           `xb.sub(&yb)` on the exact `BigInt` path at :492 — in BOTH cases an unbounded
           signed subtraction that can and does produce a negative result);
           kernel/src/type_checker.rs:297-301 (`infer_type` REFUSES a negative
           `Literal::Nat`, so a negative result cannot even be re-typed as a `Nat`);
           kernel/src/reduction.rs:337-341 (`extract_constructor` CLAMPS a negative
           `Literal::Nat` to `Zero` "so the peel TERMINATES rather than descending toward −∞" —
           a clamp that is NOT the same as Lean's truncation, see the guard);
           kernel/src/termination.rs:378-400 (the structural guard that the emitted recursion
           must satisfy)
@loss      NAT_ARITHMETIC_IS_EMITTED_NOT_PRIMITIVE. Lean's `Nat.sub` is a kernel-accelerated
           GMP operation with a compiler override; every Lean `Nat` fact about it
           (`Nat.sub_self`, `Nat.sub_lt`, `Nat.succ_sub_succ`, the whole `omega` normal form)
           is stated about that primitive. The LOGOS translation is an ordinary
           `Fix`+`Match` definition, so it computes by ι-reduction at one `Succ` per step:
           `50000 - 1` takes 50000 reduction steps, which exceeds `normalize`'s hard fuel of
           10000 (reduction.rs:52-69). Past that boundary the kernel returns the PARTIALLY
           reduced term and reports no error, so a true equation becomes unprovable with no
           diagnostic. The loss record must name the fuel boundary, not just "slower".
@loss      NO_NAT_LEMMAS. None of Lean's `Nat` subtraction lemmas transfer; each must be
           re-proved against the emitted definition or its use rejected.
@guard     THE CENTRAL BUG OF THIS CASE: translating Lean's `Nat` subtraction to the prelude's
           `sub`. It is enormously tempting — `sub` is right there, it is the only subtraction
           LOGOS has, and it reduces natively on literals (reduction.rs:455-500) instead of
           costing one step per unit. It is also wrong, and wrong in a way nothing detects.
           Lean: `3 - 8 = 0`. LOGOS `sub`: `3 - 8 ⤳ Lit(Int(-5))`. If the translator has also
           conflated `Nat` with `Int` (T10-004) the term typechecks and silently computes `-5`.
           If it has NOT conflated them, the term is ill-typed and is caught — which is the
           argument for keeping the two types apart even though it costs an emitted recursion.
@guard     Catches the "clamp it afterwards" repair: emitting `max 0 (sub n m)` or relying on
           `extract_constructor`'s negative-`Nat` clamp (reduction.rs:337-341). The clamp is a
           REDUCTION-time safety net for malformed input, not a semantic rule: it fires only
           inside `extract_constructor`, so a negative value clamps to `Zero` when it is a
           `Match` scrutinee but stays negative everywhere else — in `def_eq`, in `infer_type`
           (which rejects it outright, type_checker.rs:297-301), and in any arithmetic built on
           top. The result is a term that computes one value under `Match` and another under
           equality. There is no `max` in the prelude to build the repair from either.
@guard     Catches losing the truncation at the point where it MATTERS. `n - m + m = n` is
           FALSE in Lean when `m > n`, and a translator that "fixes" `Nat` subtraction by
           making it total-and-signed makes that identity TRUE — strengthening the theory. Any
           Lean proof that relies on the truncation (`Nat.sub_add_cancel` has the hypothesis
           `m ≤ n` for exactly this reason) becomes a proof of a different statement.
@guard     Catches emitting the recursion with the wrong structural argument. The Giménez guard
           (termination.rs:378-400) accepts a recursive call only when the structural argument
           is headed by a match-bound smaller variable. `sub (Succ n) (Succ m) = sub n m`
           decreases on BOTH arguments; the emitted `Fix` must decrease on the one the guard is
           tracking, or the definition is refused — and a translator that then reaches for
           `termination_by` finds it does not exist (T04-004).
@oracle    The emitted term for `n - m` applies a translator-emitted `Nat`-typed definition, NOT
           `Global("sub")`; `normalize(emitted(3 - 8))` is `Lit(Nat(0))` (or `Global("Zero")`);
           and the loss record contains NAT_ARITHMETIC_IS_EMITTED_NOT_PRIMITIVE. A harness
           asserts over the whole corpus that `Global("sub")` is never applied to a
           `Nat`-typed argument.
-/

namespace T10_017

/-- The defining behaviour. Lean's `Nat` subtraction saturates. -/
example : (3 : Nat) - 8 = 0 := rfl
example : (0 : Nat) - 1 = 0 := rfl
example : (8 : Nat) - 3 = 5 := rfl

/-- The same numerals at `Int`, where LOGOS's `sub` primitive lives. The two results differ,
    and this pair is the whole case. -/
example : (3 : Int) - 8 = -5 := by decide

/-- The identity that truncation BREAKS, and that a signed translation would wrongly make
    provable. -/
example : ¬ (∀ n m : Nat, n - m + m = n) := by
  intro h
  exact absurd (h 3 8) (by decide)

/-- Lean's own statement of when it does hold — note the hypothesis. -/
example (n m : Nat) (h : m ≤ n) : n - m + m = n := Nat.sub_add_cancel h

/-- The structural recursion the translator must emit, written out in Lean so the target shape
    is unambiguous. Note it decreases on both arguments simultaneously. -/
def natSub : Nat → Nat → Nat
  | n, 0 => n
  | 0, _ + 1 => 0
  | n + 1, m + 1 => natSub n m

example : natSub 3 8 = 0 := rfl
example : natSub 8 3 = 5 := rfl
/-- The emitted recursion really is Lean's `Nat` subtraction — so the degrade is a
    performance-and-lemmas loss, not a semantic one, PROVIDED the prelude's `sub` is not used. -/
theorem natSub_eq : ∀ n m : Nat, natSub n m = n - m
  | _, 0 => by simp [natSub]
  | 0, _ + 1 => by simp [natSub]
  | n + 1, m + 1 => by
      show natSub n m = (n + 1) - (m + 1)
      rw [Nat.succ_sub_succ]
      exact natSub_eq n m

/-- `Nat.pred`, the one-step case, which also saturates. -/
example : Nat.pred 0 = 0 := rfl

/-- A subtraction large enough that a one-`Succ`-per-step emitted recursion exceeds
    `normalize`'s 10000-step fuel (reduction.rs:52-69). Lean's GMP-accelerated `Nat.sub`
    does it in one step. -/
example : (50000 : Nat) - 1 = 49999 := rfl

end T10_017
