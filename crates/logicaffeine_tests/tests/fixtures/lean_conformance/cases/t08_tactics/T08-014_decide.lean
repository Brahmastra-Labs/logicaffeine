/-!
@case      T08-014
@tier      8 · Tactics
@title     `decide` — LOGOS's is a hard-wired evaluator router, not a `Decidable`-instance
           driven tactic
@lean      decide, decide on a DecidableEq enum, decide on a bounded quantifier, native_decide
@expect    translate
@target    LOGOS script atom `decide` — canonical `decide`, arity 0, aliases `compute`,
           `evaluate`, `calculate` (proof/src/tactic_script.rs:283, :317) →
           `combinators::decide` (proof/src/tactic.rs:1004-1007) → `ProofState::decide`
           (proof/src/tactic.rs:833-844) → `decide::decide_expr`
           (proof/src/decide.rs:64), closing with `InferenceRule::ArithDecision` or
           `InferenceRule::NativeDecide`
@evidence  proof/src/decide.rs:1-12 (the module header fixing the whole fragment: "ground `Int`
           identities" via `ArithDecision`, "ground comparisons and Bool equalities" via
           `NativeDecide`, "propositional structure recurses through the ordinary intro rules",
           and "A false, open, or unsupported goal yields `None` — decide DECLINES, it never
           guesses");
           proof/src/decide.rs:16-20 (`eval_int` — numeric constants and `add`/`sub`/`mul`/
           `div`/`mod` trees, CHECKED arithmetic, and "overflow declines": the evaluator is
           `i64`, so a goal outside `i64` is not decided even though
           `kernel/src/term.rs:13-44` has a `Literal::BigInt`);
           proof/src/tactic.rs:840-843 (`DoesNotApply` — "decide: not a closed decidable goal
           (or it is false)", one message for two very different situations);
           kernel/src/prelude.rs (the trusted base registers `Decidable`/`isTrue`/`isFalse` and
           exactly two decision procedures, `decEqBool` and `decEqNat` — there is no instance
           SYNTHESIS, no `DecidableEq` derivation, and no way to register a new one from a
           script); kernel/src/prelude.rs:1195-1219 (`reduceBool` / `ofReduceBool` — Lean's
           `native_decide` pair, which is a DIFFERENT route, see T06-014)
@guard     THE INSTANCE-SYNTHESIS TRAP. Lean's `decide` works for ANY proposition with a
           `Decidable` instance — a derived `DecidableEq` on a user enum, `List.all`, a bounded
           `∀ x < 10`, a structural equality on a record. LOGOS's `decide` is a hard-coded
           router over ground `Int`/`Bool` arithmetic (decide.rs:1-12); there is no instance
           resolution in the script layer at all. A translator that maps `decide` to `decide`
           produces a step that fails for every user-defined decidable proposition, and the
           message ("not a closed decidable goal (or it is false)") is indistinguishable from
           the message you get for a FALSE goal.
@guard     Catches the decline/failure conflation in the dangerous direction. `decide_expr`
           returns `None` both when the goal is unsupported and when it is FALSE
           (decide.rs:8-11). A translator that treats a `DoesNotApply` from `decide` as "try
           another tactic" will keep searching past a goal that is actually false, and may
           eventually close it with `auto`'s classical reductio or with `omega`'s ex-falso from
           some unrelated contradiction — producing a certified proof of a statement the Lean
           source never asserted. The decline must be surfaced, not swallowed.
@guard     Catches the `i64` boundary. `eval_int` is checked `i64` arithmetic and declines on
           overflow (decide.rs:16-20), while the kernel's `Literal` has `BigInt` and `Nat`
           (`BigInt`) variants (term.rs:13-44) and `Nat` literals are definitionally
           `Succ^n Zero` (reduction.rs:330-345). So a `decide` that Lean discharges on a large
           numeral has no LOGOS image, and the boundary is at `i64`, not at the kernel's
           representable range — a limit invisible from both the Lean source and the kernel.
@guard     Catches routing `native_decide` here. It is a separate trusted path through
           `reduceBool`/`ofReduceBool` (prelude.rs:1195-1219) with a different TCB; mapping
           Lean's `native_decide` to LOGOS's `decide` silently narrows it to the evaluator
           router, and mapping LOGOS's `NativeDecide` inference rule back to Lean's `decide`
           silently widens the TCB. See T06-014.
@guard     Catches the alias `compute`/`evaluate`/`calculate` (tactic_script.rs:317). These are
           natural renderings of `norm_num`, `native_decide`, `rfl`-by-computation and `simp
           arith` too. Emitting an English rendering rather than the canonical `decide` lands
           on whichever tactic the alias table happens to point at.
@oracle    Every emitted `decide` atom runs against a goal that `decide::decide_expr` accepts —
           closed, `i64`-representable, built only from `Int` identities, comparisons, `Bool`
           equalities and propositional structure over them — and `run_script` closes it with
           an `ArithDecision` or `NativeDecide` leaf. For the enum, bounded-quantifier and
           large-numeral theorems the translator must emit a diagnostic rather than a `decide`
           atom.
-/

namespace T08_014

/-- The fragment that translates: a ground arithmetic identity. -/
theorem groundArith : (2 : Nat) + 2 = 4 := by decide

/-- A ground comparison — the `NativeDecide` route. -/
theorem groundComparison : (3 : Nat) < 5 := by decide

/-- A ground `Bool` equality. -/
theorem groundBool : (true && false) = false := by decide

/-- Propositional structure over ground atoms — recurses through the intro rules. -/
theorem propStructure : (1 : Nat) < 2 ∧ (3 : Nat) < 4 := by decide

/-- A DERIVED `DecidableEq` on a user enum. Lean synthesises the instance; LOGOS has only
    `decEqBool` and `decEqNat` and no synthesis. -/
inductive Colour where
  | red | green | blue
  deriving DecidableEq

theorem enumEq : Colour.red ≠ Colour.blue := by decide

/-- A BOUNDED QUANTIFIER: `Nat.decidableBallLT` makes this decidable in Lean. No LOGOS route. -/
theorem boundedForall : ∀ n : Nat, n < 3 → n * n < 9 := by decide

/-- A `List`-driven decidable proposition. -/
theorem listDecidable : [1, 2, 3].all (· < 4) = true := by decide

/-- A numeral beyond `i64` — `eval_int` declines on overflow even though the kernel has
    `Literal::BigInt`. -/
theorem bigNumeral : (9223372036854775808 : Nat) + 1 = 9223372036854775809 := by decide

/-- `native_decide`: a different trusted path in both systems. -/
theorem viaNative : (100 : Nat) * 100 = 10000 := by native_decide

end T08_014
