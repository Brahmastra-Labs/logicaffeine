/-!
@case      T08-009
@tier      8 · Tactics
@title     `induction n` ⟼ zero-argument `induction`, which consumes the goal's OUTERMOST `∀`
@lean      induction n, induction n with | zero | succ k ih, induction on a context variable
@expect    degrade
@target    LOGOS script atom `induction` — canonical `induction`, arity **0**, alias `induct`
           (proof/src/tactic_script.rs:290, :324) → `combinators::induction`
           (proof/src/tactic.rs:1047-1050) → `ProofState::induction`
           (proof/src/tactic.rs:561-607), emitting
           `InferenceRule::StructuralInduction { variable, ind_type: "Nat", step_var }`
@evidence  proof/src/tactic_script.rs:290 (`"induction" => Ok(c::induction())` — `arg()` is
           NEVER called, so the atom takes no argument and a following word is read as the next
           ATOM); proof/src/tactic.rs:562-570 (`induction` reads the target and requires
           `ProofExpr::ForAll { variable, body }`; anything else is `DoesNotApply` — so the
           variable inducted on is whatever the goal's OUTERMOST binder happens to be);
           proof/src/tactic.rs:601 (`ind_type: "Nat".to_string()` — hard-coded);
           proof/src/tactic.rs:583-586 (base target is `body[variable := Constant("Zero")]`,
           step target is `body[variable := Function("Succ", [Constant(step_var)])]`);
           proof/src/tactic.rs:571, :577 (`self.fresh += 1` then
           `step_var = format!("K{}", self.fresh)` — uppercase so the search treats it as a
           rigid eigenconstant); proof/src/tactic.rs:593 (the induction hypothesis is named
           `"ih"`, hard-coded, and placed FIRST in the step's hypothesis list);
           proof/src/tactic.rs:604 (children are `[base_goal, step_goal]`, base first);
           proof/src/tactic.rs:916-941 (`assemble` remaps the eigenconstant back to a bound
           `Variable` via `eigen_to_var_tree`, which is what the certifier's `Match` arm binds)
@loss      TARGET_SELECTION. Lean's `induction n` names its target and works on a variable
           already in the local context; it REVERTS every hypothesis depending on `n`,
           generalises, and re-introduces. LOGOS's `induction` has no argument: it can only
           induct on the goal's outermost `∀`. A Lean proof that does `intro n; …; induction n`
           must be re-ordered so the `∀ n` is the outermost connective at the moment the atom
           runs — i.e. the translator must perform Lean's `revert` itself, at the script level,
           where there is no `revert` atom to perform it with. If any hypothesis mentions `n`,
           the generalisation is not expressible at all and the case escalates to `reject`.
@loss      CASE_NAMING. `induction n with | zero => … | succ k ih => …` names both the cases and
           the step variable and the IH. LOGOS fixes all three: the cases are positional (base
           then step, tactic.rs:604), the step variable is `K{fresh}` (tactic.rs:577), and the
           IH is `ih` (tactic.rs:593). Every reference in the translated step block must be
           rewritten to `ih`, and every reference to Lean's `k` to the generated `K{n}`.
@loss      IH_PLACEMENT. The IH is pushed to the FRONT of the step's hypothesis list
           deliberately (tactic.rs:590-594), so `assumption`/`auto` prefer it over a base fact
           and keep the step variable symbolic. Lean places it last. Any translated step that
           relies on `assumption` picking a particular hypothesis changes meaning.
@guard     Catches emitting `induction n`. The atom's arity is zero (tactic_script.rs:290), so
           the trailing `n` is parsed as the NEXT ATOM and `canonical("n")` is `None`: the whole
           script fails at PARSE time with "unknown tactic `n`", discarding the proof
           (`run_script` leaves the state unchanged on a parse error, tactic_script.rs:358-362).
           This is the single most likely mechanical slip in Tier 8 and it is at least loud.
@guard     Catches inducting on the WRONG variable. With a goal `∀ m n, P m n`, Lean's
           `induction n` inducts on the inner binder; LOGOS's argumentless `induction` takes
           `m`, the outermost. Both produce a base case and a step case, both typecheck, and
           the resulting theorem is a DIFFERENT theorem — proved, certified, and wrong.
           Nothing downstream can detect it, because the FOL layer is untyped and the
           substitution `m := Zero` is perfectly well-formed. A translator MUST verify that the
           outermost binder is the one Lean named.
@guard     Catches the `fresh`-counter coupling. `induction` increments `ProofState::fresh`
           (tactic.rs:571), the same counter that names `∃`-elimination witnesses `W{n}`
           (tactic.rs:228-231). Emitting an induction changes the name of every later witness
           in the same proof. See T08-008.
@guard     Catches assuming the step variable is a `Variable`. It is a `Constant` named
           `K{n}` during the search (tactic.rs:572-577) and only becomes a `Variable` when
           `assemble` runs (tactic.rs:926-930). A translator that emits a step-case reference
           as `Variable("K1")` will not match the goal, which holds `Constant("K1")`.
@oracle    The emitted script contains a bare `induction` atom with NO argument; the goal it
           runs against has the Lean-named variable as its outermost `∀`; every step-case
           hypothesis reference is `ih`; and the loss record names TARGET_SELECTION and
           CASE_NAMING. For `innerBinder` the harness asserts the translator either reordered
           the binders or emitted a diagnostic rather than an `induction` atom.
-/

namespace T08_009

/-- The shape that degrades cleanly: the goal is already `∀ n, P n`. -/
theorem outermost : ∀ n : Nat, n + 0 = n := by
  intro n
  induction n with
  | zero => rfl
  | succ k ih => simp

/-- Lean's `induction` on a variable ALREADY in the context. LOGOS must re-generalise it to the
    outermost `∀` first, and there is no `revert` atom to do so. -/
theorem contextVariable (n : Nat) : 0 + n = n := by
  induction n with
  | zero => rfl
  | succ k ih => simp

/-- THE WRONG-VARIABLE TRAP: Lean inducts on the INNER binder. A zero-argument `induction`
    takes `m`, proves a different theorem, and nothing objects. -/
theorem innerBinder : ∀ m n : Nat, m + n = n + m := by
  intro m n
  induction n with
  | zero => simp
  | succ k ih => omega

/-- A hypothesis MENTIONS the induction variable, so Lean's revert-generalise-reintroduce is
    load-bearing. Not expressible at the LOGOS script layer at all. -/
theorem dependentHypothesis (p : Nat → Prop) (base : p 0) (step : ∀ k, p k → p (k + 1)) :
    ∀ n, p n := by
  intro n
  induction n with
  | zero => exact base
  | succ k ih => exact step k ih

/-- Named case syntax: `zero` / `succ`, the step variable `k`, and the IH `ih` are all user
    choices in Lean and all fixed in LOGOS. -/
theorem namedCases : ∀ n : Nat, n * 1 = n := by
  intro n
  induction n with
  | zero => rfl
  | succ j hj => simp

/-- The IH used in a position where hypothesis ORDER matters: LOGOS puts `ih` first. -/
theorem ihOrdering (p : Nat → Prop) (alwaysTrue : ∀ m, p m) (step : ∀ k, p k → p (k + 1)) :
    ∀ n, p n := by
  intro n
  induction n with
  | zero => exact alwaysTrue 0
  | succ k ih => exact step k ih

end T08_009
