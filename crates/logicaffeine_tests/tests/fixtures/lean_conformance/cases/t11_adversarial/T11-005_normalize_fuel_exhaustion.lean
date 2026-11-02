/-!
@case      T11-005
@tier      11 · Adversarial and soundness traps
@title     `normalize` has a hard fuel of 10000 and returns the PARTIALLY reduced term without
           saying so
@lean      deep computation by `rfl` / `decide`, Nat.pow, Ackermann-ish recursion, long
           unfolding chains, `Nat.rec` over a large literal
@expect    degrade
@target    The faithful term, PLUS a loss record naming the reduction-budget boundary. There is
           no LOGOS knob to raise the fuel — it is a literal in the loop.
@evidence  kernel/src/reduction.rs:52-69 (`normalize`: `let mut fuel = 10000; loop { if fuel ==
           0 { /* If we hit the fuel limit, return what we have */ return current; } fuel -= 1;
           let reduced = reduce_step(ctx, &current); if reduced == current { return current; }
           current = reduced; }` — the exhausted branch and the normal-form branch return the
           SAME shape of value, `Term`, with no flag, no `Result`, and no way for a caller to
           tell them apart); kernel/src/type_checker.rs:1310-1323 (`is_subtype` normalises both
           sides then compares — so an exhausted normalisation makes a TRUE subtyping judgement
           come out FALSE); kernel/src/type_checker.rs:1465-1520 (`def_eq_normalized` likewise);
           kernel/src/reduction.rs:329-345 (the Peano bridge peels ONE `Succ` per step, so a
           `Nat` recursion costs one unit of fuel per unit of the numeral);
           kernel/src/type_checker.rs:890-902 (`substitute`'s fast path avoids recomputing
           `free_vars` when the variable does not occur — but each step that DOES substitute is
           O(size), so fuel is consumed by steps of growing cost, see T11-007)
@loss      REDUCTION_BUDGET. Lean's kernel has no step limit on whnf; it has a
           `maxRecDepth`/`maxHeartbeats` that ERROR when exceeded. LOGOS's `normalize` has a
           step limit that SUCCEEDS when exceeded, returning a partially reduced term. So a
           Lean proof by `rfl` whose reduction takes more than 10000 `reduce_step` calls does
           not translate to a proof by reflexivity in LOGOS — it translates to a term whose
           `def_eq` check answers "not equal". The loss record must state the budget (10000),
           name the declaration, and give the translator's own estimate of the step count.
@loss      NO_DIAGNOSTIC_FROM_TARGET. The boundary is not observable from LOGOS's API: an
           exhausted `normalize` and a completed `normalize` are indistinguishable to the
           caller. The translator must therefore carry its own step accounting; it cannot ask
           the kernel afterwards.
@guard     **TIER 11 THESIS.** LOGOS's kernel accepting a term is not evidence of faithfulness
           — and here the kernel's REJECTION is not evidence of unfaithfulness either. A
           `def_eq` that answers `false` because the fuel ran out looks exactly like a `def_eq`
           that answers `false` because the terms differ. A translator that treats "LOGOS says
           these are not definitionally equal" as a reason to go back and change the
           translation will change a CORRECT translation into an incorrect one that happens to
           normalise faster.
@guard     Catches the silent-truncation failure mode in its worst direction. Because the
           exhausted result is a partially reduced term, `def_eq(a, b)` on two terms that reduce
           to the same normal form in 12000 steps returns `false` — a false NEGATIVE, which
           merely loses a proof. But `def_eq(a, b)` on two DIFFERENT terms that happen to agree
           on their first 10000 steps of reduction returns `true` — a false POSITIVE, which
           accepts a wrong proof. The second is rare and is the reason the loss record must
           name the boundary rather than just "some proofs may fail".
@guard     Catches the "expand it for clarity" move that pushes a case over the line. Emitting
           a `Nat` literal as `Succ^n Zero` (T10-015), inlining a definition at each use site
           instead of emitting one `add_definition` (T03-011), or spelling out a wildcard
           pattern as one case per constructor (T04-010) each multiply the step count, and any
           of them can turn a 3000-step reduction into a 30000-step one. A translator must
           account for the budget when it chooses an encoding, not only when it emits a term.
@guard     Catches assuming the fuel is per-call in a helpful way. It is per `normalize` call,
           and `is_subtype` calls `normalize` twice (reduction.rs:52-69 via
           type_checker.rs:1318-1322), and `def_eq_normalized` recurses structurally calling
           `def_eq` on subterms — so the effective budget for a large term is neither 10000 nor
           a simple multiple of it, and a translator cannot compute it exactly. The honest
           record is "this declaration's reduction is within an order of magnitude of the
           budget", not a precise step count.
@oracle    The emitted term is the faithful one, AND a loss record with category
           REDUCTION_BUDGET is produced for each declaration whose estimated `reduce_step`
           count exceeds a configured fraction of 10000. A harness runs the emitted `def_eq`
           checks against the target and asserts that every one the translator predicted would
           succeed did succeed; a `def_eq` failure that the translator did NOT predict is a
           corpus failure, not a target limitation.
-/

namespace T11_005

/-- A reduction Lean does by `rfl` in one GMP step and an emitted Peano recursion does in
    ~50000 `reduce_step` calls — five times the LOGOS budget. -/
example : (50000 : Nat) - 1 = 49999 := rfl

/-- A recursion whose step count is the numeral itself. -/
def countDown : Nat → Nat
  | 0 => 0
  | n + 1 => countDown n

example : countDown 100 = 0 := rfl

/-- Just inside a plausible budget — and already past Lean's own `maxRecDepth` for `rfl`,
    which is why this needs the compiled evaluator. Lean's limit ERRORS; LOGOS's fuel limit
    SUCCEEDS with a partially reduced term. That difference is the whole case. -/
example : countDown 5000 = 0 := by native_decide

/-- Comfortably outside it. -/
example : countDown 20000 = 0 := by native_decide

/-- Exponential blow-up from a small numeral: the step count is not visible in the source. -/
def tower : Nat → Nat
  | 0 => 1
  | n + 1 => 2 * tower n

example : tower 14 = 16384 := rfl

/-- Two functions that AGREE for a long prefix and then differ. If the divergence lies past the
    budget, an exhausted `normalize` on both sides can make them compare equal — the false
    positive named in the guard above. -/
def f (n : Nat) : Nat := n
def g (n : Nat) : Nat := if n < 100000 then n else n + 1

example : f 5 = g 5 := rfl

/-- Deep delta-unfolding: each `step` costs a delta step plus a beta step, so the budget is
    consumed by definition unfolding rather than by recursion. -/
def s0 (n : Nat) : Nat := n
def s1 (n : Nat) : Nat := s0 (s0 n)
def s2 (n : Nat) : Nat := s1 (s1 n)
def s3 (n : Nat) : Nat := s2 (s2 n)
def s4 (n : Nat) : Nat := s3 (s3 n)

example : s4 7 = 7 := rfl

/-- `decide` routes through the same reduction machinery: the `Decidable` instance must
    normalise to `isTrue`/`isFalse`, and that normalisation is subject to the same budget. -/
example : decide (countDown 3000 = 0) = true := by native_decide

/-- The boundary is on REDUCTION STEPS, not on term size: this term is tiny and its
    normalisation is long. A translator that budgets by AST size will misjudge it. -/
def ack : Nat → Nat → Nat
  | 0, n => n + 1
  | m + 1, 0 => ack m 1
  | m + 1, n + 1 => ack m (ack (m + 1) n)

example : ack 2 3 = 9 := by native_decide

end T11_005
