/-!
@case      T08-013
@tier      8 · Tactics
@title     `omega` — the name maps 1:1, but LOGOS's `omega` only REFUTES hypotheses; it never
           decides the goal
@lean      omega on Nat/Int linear goals, omega from contradictory hypotheses, omega with no
           arithmetic hypotheses
@expect    translate
@target    LOGOS script atom `omega` — canonical `omega`, arity 0, aliases `arithmetic`,
           `linarith`, `integers` (proof/src/tactic_script.rs:284, :318) →
           `combinators::omega` (proof/src/tactic.rs:1008-1011) → `ProofState::omega`
           (proof/src/tactic.rs:852-878), closing the goal with `InferenceRule::ExFalso`
@evidence  proof/src/tactic.rs:852-860 (`omega` FIRST calls
           `omega_solve::has_arith_facts` on the hypotheses and returns `DoesNotApply` —
           "omega: no arithmetic (≤ / <) hypotheses in scope" — when there are none. The GOAL is
           never inspected for arithmetic content);
           proof/src/tactic.rs:861-873 (`omega_close(&known)` searches for a refutation of the
           hypotheses; on success the goal — ANY goal — is closed by `InferenceRule::ExFalso`
           over the `⊥` derivation);
           proof/src/tactic.rs:874-877 (`DoesNotApply` when the hypotheses have an integer
           model: "the integer hypotheses are satisfiable (no refutation)");
           proof/src/linarith_solve.rs:1-8 (the decision core: Fourier-Motzkin producing a
           FARKAS CERTIFICATE — multipliers `λᵢ ≥ 0` with `Σ λᵢ·constraintᵢ` a positive
           constant `≤ 0`);
           proof/src/tactic.rs:846-851 (the doc comment: "linear INTEGER arithmetic … using
           discreteness (`a < b ⟹ a+1 ≤ b`) … Mirrors the REFUTATION HALF of Lean's `omega`");
           kernel/src/prelude.rs:202-340 (the eight `Int` order axioms the Farkas certificate is
           reconstructed against: `le_refl`, `le_trans`, `le_add_mono`, `le_mul_nonneg`,
           `le_sub`, `lt_succ_le`, `lt_add1_le`, `le_total`, where `a ≤ b ≔ Eq Bool (le a b) true`)
@guard     THE HALF-TACTIC TRAP. Lean's `omega` is a DECISION PROCEDURE for the goal: it proves
           `n + 0 = n`, `x < 1 → x = 0`, `2 * n ≥ n` from nothing. LOGOS's proves nothing from
           nothing — with no `≤`/`<` hypotheses in scope it declines at tactic.rs:856-859
           before looking at the goal at all, and with satisfiable hypotheses it declines at
           :874. A translator that maps every Lean `omega` to a LOGOS `omega` gets a script that
           fails on the majority of real uses, and the failure message ("no arithmetic
           hypotheses in scope") describes the HYPOTHESES when the problem is that LOGOS has no
           way to attack the goal.
@guard     Catches assuming the goal is negated first. A complete `omega` would negate the goal,
           add it to the hypothesis set, and refute. LOGOS's does not: `known` is built from
           `g.hyps` alone (tactic.rs:861-862) and the goal appears only as the `ExFalso`
           conclusion. So `x ≤ 3 ⊢ x ≤ 4` — trivially valid, and refutable only after negating
           the goal — is DECLINED, because `x ≤ 3` alone is satisfiable.
@guard     Catches the `Nat` / `Int` conflation. LOGOS's order axioms are stated for `Int`
           (prelude.rs:202-340), and `Int` in LOGOS is an OPAQUE type with axiomatised laws
           while `Nat` is an inductive with a literal bridge (see T10-004). Lean's `omega`
           handles both, and crucially knows `Nat` subtraction truncates and that every `Nat`
           is `≥ 0`. Neither fact is available to `omega_close`, which works over ℤ. A `Nat`
           goal whose validity depends on non-negativity or on truncated subtraction is
           outside the fragment even when hypotheses are present.
@guard     Catches over-trusting the ex-falso closure. When `omega` succeeds it closes ANY goal
           (tactic.rs:866-871) — including a FALSE one, soundly, because the hypotheses were
           contradictory. That is correct, but it means a successful `omega` is NO evidence that
           the goal was arithmetic or even meaningful. A harness must not treat "omega closed
           it" as "the arithmetic content was translated".
@note      The index records `translate`, and the script-level name maps exactly: `omega` in,
           `omega` out, aliases included. This file keeps that verdict because the REFUTATION
           fragment corresponds one-for-one. There is a defensible argument for `degrade` with
           loss OMEGA_REFUTATION_ONLY, since the `positiveGoal*` theorems below have no LOGOS
           image whatever — Lean proves them, LOGOS declines. A translator that chooses
           `degrade` and emits the loss record is behaving better than one that chooses
           `translate` and emits nothing; the harness should accept either, and must reject a
           translator that emits `omega` for a positive goal with no loss record at all.
@oracle    Every emitted `omega` atom runs against a goal whose LOGOS hypothesis set satisfies
           `omega_solve::has_arith_facts` and is integer-unsatisfiable; `run_script` closes the
           goal and `qed()` returns `verified == true` with an `ExFalso` root over a Farkas
           certificate. For the `positiveGoal*` theorems the translator must either emit a loss
           record or a diagnostic — emitting a bare `omega` is a failure.
-/

namespace T08_013

/-- The fragment that corresponds exactly: contradictory integer hypotheses, arbitrary goal.
    `x < y` and `y < x + 1` are jointly unsatisfiable over ℤ by discreteness. -/
theorem refuteDiscrete (x y : Int) (q : Prop) (h1 : x < y) (h2 : y < x + 1) : q := by
  omega

/-- Refutation over `Nat`, again from the hypotheses alone. -/
theorem refuteNat (n : Nat) (q : Prop) (h1 : n < 2) (h2 : 3 < n) : q := by
  omega

/-- POSITIVE GOAL, no hypotheses. Lean decides it; LOGOS declines at tactic.rs:856-859. -/
theorem positiveGoalNoHyps (n : Nat) : n + 0 = n := by omega

/-- POSITIVE GOAL with a SATISFIABLE hypothesis. Lean proves it by negating the goal; LOGOS
    declines at tactic.rs:874 because `x ≤ 3` on its own has a model. -/
theorem positiveGoalSatHyps (x : Int) (h : x ≤ 3) : x ≤ 4 := by omega

/-- The classic `omega`-over-`linarith` example from the LOGOS doc comment itself
    (tactic.rs:846-851): rationally satisfiable, integrally not. -/
theorem discretenessMatters (x y : Int) (q : Prop) (h : x < y ∧ y < x + 1) : q := by
  omega

/-- `Nat` non-negativity: implicit in Lean's `omega`, absent from LOGOS's ℤ-based core. -/
theorem natNonneg (n : Nat) : 0 ≤ n := by omega

/-- TRUNCATED `Nat` subtraction — Lean's `omega` models it; `omega_close` works over ℤ. -/
theorem truncatedSub (n : Nat) (h : n ≤ 3) : n - 5 = 0 := by omega

end T08_013
