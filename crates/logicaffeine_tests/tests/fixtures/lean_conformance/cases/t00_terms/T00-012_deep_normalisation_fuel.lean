/-!
@case      T00-012
@tier      0 · Kernel term layer
@title     Deep normalisation — `normalize`'s 10000-step fuel expires SILENTLY
@lean      definitional equality by `rfl` requiring many reduction steps, Nat GMP acceleration
@expect    degrade
@target    The term translates structurally (there is nothing special about its shape); the
           degradation is in the CHECKING BUDGET. The translator must emit a loss record naming
           NORMALIZATION_FUEL for every declaration whose Lean-side conversion check is known or
           estimated to exceed LOGOS's step budget.
@evidence  kernel/src/reduction.rs:40-41 and :52-69 (`normalize` runs `reduce_step` under a
           hard `let mut fuel = 10000;` and, on exhaustion, `return current` — the PARTIALLY
           reduced term, with no error, no flag and no diagnostic);
           kernel/src/type_checker.rs:1366-1375 (`def_eq` normalizes BOTH sides and then
           compares — two independently truncated normal forms);
           kernel/src/type_checker.rs:1309-1323 (`is_subtype` does the same, after a
           `types_equal` fast path);
           kernel/src/type_checker.rs:1480-1493 (the Peano bridge peels ONE `Succ` per recursive
           call, so a `Nat` literal of size `n` costs `n` steps to compare against a Peano term);
           kernel/src/reduction.rs:330-345 (`extract_constructor`'s literal→`Succ` bridge, the
           other half of the same cost)
@loss      NORMALIZATION_FUEL. A Lean `rfl` whose kernel conversion check Lean discharges with
           GMP-accelerated `Nat` arithmetic in O(1) becomes, in LOGOS, a unary Peano computation
           costing O(n) reduction steps. Above 10000 steps the check does not fail loudly — it
           silently compares two half-reduced terms. The loss is that a definitional equality
           true in Lean is NOT decidable in LOGOS at the declared budget, so the proof must be
           restated propositionally (via the `Int` ring axioms at prelude.rs:92-191, or via
           `reduceBool`/`ofReduceBool` at prelude.rs:1195-1219) rather than left to conversion.
@guard     Catches treating a conversion FAILURE as evidence that the translation is wrong. When
           fuel runs out, `def_eq` returns `false` — indistinguishable from a genuine type
           mismatch. A translator that reacts by "fixing" the term (inserting a coercion,
           weakening a type, or falling back to `add_declaration` to axiomatise the equation)
           converts a budget problem into a soundness problem. `add_declaration`
           (context.rs:434) will accept `theorem big_arith : … = …` as an AXIOM without a
           murmur, and the resulting library is then unsound with respect to nothing — it is
           simply no longer a translation of the Lean source.
@guard     Catches the far more dangerous converse: fuel exhaustion making a check SUCCEED.
           `is_subtype` (type_checker.rs:1309-1316) tries `types_equal` FIRST, and `types_equal`
           treats `Term::Hole` as matching anything (type_checker.rs:1594-1597). If a truncated
           normal form still contains a `Hole` the translator left behind (T00-016), the
           truncation is what stops the `Hole` from ever being reached and reported — so adding
           fuel would have caught the bug and the default budget hides it.
@guard     Catches NON-DETERMINISM. Fuel is counted per `normalize` CALL, not per declaration.
           The same subterm can be compared once inside a small type (succeeds) and once inside
           a large one (truncates), so the same equation is accepted in one declaration and
           rejected in the next. A translator that caches "this equation is definitional" from a
           successful check will emit terms that do not re-check.
@guard     Catches assuming Lean's `Nat` acceleration carries over. Lean's kernel special-cases
           `Nat` literals with GMP; LOGOS accelerates only *equality* of two `Literal::Nat`s
           (type_checker.rs:1485) and otherwise falls back to unary peeling. `2 ^ 20 = 1048576`
           is one Lean kernel step and roughly a million LOGOS steps.
@oracle    For each declaration the harness records LOGOS's `reduce_step` count during checking;
           any declaration whose count reaches 10000 must have a matching NORMALIZATION_FUEL
           loss record. Re-running with an unbounded fuel must not change any accept/reject
           verdict — a verdict that flips is a fuel-hidden bug.
-/

namespace T00_012

/-- Lean discharges this with GMP in one kernel step. In LOGOS the two sides are `Literal::Nat`s
    compared at type_checker.rs:1485 — cheap — but only because NEITHER side had to reduce. -/
example : (1048576 : Nat) = 1048576 := rfl

/-- Now one side must actually COMPUTE. Lean: one GMP multiply chain. LOGOS: `^` is not a
    primitive (prelude.rs:352-515 lists `add, sub, mul, div, mod` on `Int` only), so the
    translator must emit the recursive definition and normalize it. -/
example : 2 ^ 20 = 1048576 := rfl

/-- The Peano-bridge cost made explicit: comparing a large literal against a `Succ`-headed term
    peels one `Succ` per step (type_checker.rs:1486-1492). -/
example : (100000 : Nat) + 1 = 100001 := rfl

/-- A delta/iota chain whose length is controlled by the argument. `tower n` unfolds to `n`
    nested `bump` applications before any of them reduces. -/
def bump (n : Nat) : Nat := n + 1

def tower : Nat → Nat
  | 0 => 0
  | n + 1 => bump (tower n)

example : tower 20 = 20 := by rfl

/-- Exponential duplication: each `dup` doubles the term, so `dup^k 1` normalizes through
    `2^k` copies of the argument. At `k = 14` that is past the 10000-step budget. -/
def dup (x : Nat) : Nat := x + x

example : dup (dup (dup (dup 1))) = 16 := rfl

/-- A conversion check hidden in a TYPE rather than a term: the `Fin` index must reduce before
    the ascription checks. Fuel spent here is invisible in the proof term. -/
def finByComputation : Fin (2 ^ 10) := ⟨1000, by omega⟩

end T00_012
