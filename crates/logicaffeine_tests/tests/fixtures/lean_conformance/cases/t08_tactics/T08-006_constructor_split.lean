/-!
@case      T08-006
@tier      8 · Tactics
@title     `constructor` on `∧` ⟼ `split` — but LOGOS's `split` is ∧-only and the word
           `constructor` is its ALIAS
@lean      constructor, exact ⟨_, _⟩, constructor on ∨, constructor on Iff, refine ⟨_, _⟩
@expect    translate
@target    LOGOS script atom `split` — canonical `split`, arity 0, aliases `constructor`,
           `both`, `conjunction` (proof/src/tactic_script.rs:286, :320) →
           `combinators::split` (proof/src/tactic.rs:1022-1025) → `ProofState::split`
           (proof/src/tactic.rs:395-410), emitting `InferenceRule::ConjunctionIntro`
@evidence  proof/src/tactic_script.rs:320 ("split"|"constructor"|"both"|"conjunction" ⟼
           "split" — the Lean word `constructor` is accepted, and it means ∧-introduction
           REGARDLESS of the goal);
           proof/src/tactic.rs:395-410 (`ProofState::split` matches ONLY
           `ProofExpr::And(a, b)` and returns `DoesNotApply` for every other shape, :408);
           proof/src/tactic.rs:401-405 (two subgoals, left conjunct FIRST, certified by
           `InferenceRule::ConjunctionIntro`);
           proof/src/tactic.rs:280-298 (`refine` pushes the new subgoals to the FRONT of
           `open`, in order — so the step after `split` targets the LEFT conjunct);
           proof/src/lib.rs:217-231 (`ProofExpr` — `And`, `Or` and `Iff` are three distinct
           constructors; there is no generic "apply the inductive's first constructor")
@guard     THE ALIAS TRAP. A translator that passes Lean's tactic word through unchanged gets
           the right answer for `∧` by luck and the WRONG tactic everywhere else: the script
           word `constructor` is hard-wired to ∧-introduction (tactic_script.rs:320), so
           Lean's `constructor` on an `∨` goal — where Lean picks `Or.inl` — becomes LOGOS
           `split`, which reports `DoesNotApply` ("split expects ∧, got Or(..)"). The error
           names `split`, a tactic the translator never wrote, so the diagnostic points at the
           wrong place.
@guard     Catches `constructor` on an `Iff` goal. Lean's `constructor` applies `Iff.intro`
           and leaves the two implications. LOGOS has `ProofExpr::Iff` (lib.rs:227) and
           `InferenceRule::BicondIntro` in the rule set, but `split` does not handle it
           (tactic.rs:397-409) and NO script atom introduces an `Iff`. An `Iff` goal therefore
           has no step-wise script proof at all; the only route is `auto`.
@guard     Catches `constructor` on a general inductive. Lean's `constructor` applies the first
           constructor of ANY inductive whose conclusion matches — `Exists.intro`, `Or.inl`,
           `Nat.succ`, a user structure's `mk`. LOGOS's script layer works on FOL `ProofExpr`,
           which has exactly three introduction atoms (`split`, `left`/`right`, `exists`); a
           `constructor` on a user structure has no image, and its `⟨…⟩` sugar is equally
           unreachable because `exact` takes only a name (see T08-002).
@guard     Catches the ORDER of the residual goals. `split` pushes the left conjunct to the
           front of `open` (tactic.rs:401-404 then :296-297), matching Lean's `constructor`.
           A translator that reverses them — natural if it folds a goal list — produces a
           script whose subsequent steps are applied to the wrong conjunct and which, for a
           symmetric conjunction like `p ∧ p`, still SUCCEEDS while proving the goals in the
           wrong order. That is undetectable downstream.
@guard     Catches emitting `both` (tactic_script.rs:320) from prose. The alias exists so an
           English proof can read "…and both…"; a translator that emits the English rendering
           of a Lean `And.intro` gets ∧-introduction, which is right — but `both` is also a
           plausible rendering of a two-branch case analysis, which is `cases`, not `split`.
@oracle    Every `∧`-goal theorem emits `split` (or the alias `constructor`) and `run_script`
           leaves exactly two open goals with the LEFT conjunct focused. The `∨` and `Iff`
           theorems must NOT emit `split`; the translator must emit `left`/`right` for the
           former and a diagnostic (or `auto`) for the latter.
-/

namespace T08_006

/-- The shape that translates: `constructor` on a conjunction ⟼ `split`. -/
theorem andIntro (p q : Prop) (hp : p) (hq : q) : p ∧ q := by
  constructor
  · exact hp
  · exact hq

/-- Symmetric conjunction — a translator that swaps the subgoal order still succeeds here,
    which is exactly why the order guard matters. -/
theorem symmetricAnd (p : Prop) (hp : p) : p ∧ p := by
  constructor <;> exact hp

/-- `constructor` on a DISJUNCTION: Lean picks `Or.inl`. LOGOS's `split` refuses. -/
theorem orIntro (p q : Prop) (hp : p) : p ∨ q := by
  constructor
  exact hp

/-- `constructor` on an `Iff`: Lean applies `Iff.intro`. No LOGOS script atom does. -/
theorem iffIntro (p : Prop) : p ↔ p := by
  constructor
  · intro h; exact h
  · intro h; exact h

/-- `constructor` on an `Exists`: Lean applies `Exists.intro` and leaves the witness as a
    metavariable. LOGOS's `exists` needs the witness up front (see T08-017). -/
theorem existsIntro : ∃ n : Nat, n = 0 := by
  constructor
  rfl

/-- `constructor` on a user structure. -/
structure Both where
  l : Nat
  r : Nat

def structIntro : Both := by
  constructor
  · exact 1
  · exact 2

/-- The anonymous-constructor spelling of the same thing — elaborates to the same `Expr`. -/
theorem anonCtor (p q : Prop) (hp : p) (hq : q) : p ∧ q := ⟨hp, hq⟩

end T08_006
