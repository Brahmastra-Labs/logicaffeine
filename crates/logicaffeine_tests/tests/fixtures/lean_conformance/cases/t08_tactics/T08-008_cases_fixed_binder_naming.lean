/-!
@case      T08-008
@tier      8 · Tactics
@title     `cases` / `obtain` — LOGOS binder names are FIXED (`h_1`,`h_2`,`h_l`,`h_r`,`h_w`)
           and the ∃-witness name depends on unrelated earlier tactics
@lean      cases, cases … with | intro a b, obtain ⟨a, b⟩ := h, rcases, ∃-elimination
@expect    degrade
@target    LOGOS script atom `cases <name>` — canonical `cases`, arity 1, aliases `destruct`,
           `consider`, `casework` (proof/src/tactic_script.rs:278, :312) →
           `combinators::cases` (proof/src/tactic.rs:1038-1042) → `ProofState::cases`
           (proof/src/tactic.rs:495-554)
@evidence  proof/src/tactic.rs:486-494 (the doc comment fixing the three shapes and the naming
           scheme); proof/src/tactic.rs:516-517 (`∧`: pushes `format!("{name}_1")` and
           `format!("{name}_2")`, each backed by a `ConjunctionElim` projection of the parent);
           proof/src/tactic.rs:523, :526 (`∨`: `format!("{name}_l")` and `format!("{name}_r")`,
           two subgoals, LEFT first, `InferenceRule::DisjunctionCases`);
           proof/src/tactic.rs:536-541 (`∃`: a fresh witness CONSTANT from `fresh_witness`, and
           the hypothesis `format!("{name}_w")`);
           proof/src/tactic.rs:228-231 (`fresh_witness` — `self.fresh += 1; format!("W{}",
           self.fresh)`, a counter on the WHOLE proof state);
           proof/src/tactic.rs:571 (`ProofState::induction` ALSO bumps `self.fresh`, and
           tactic.rs:651 shows `induction_over` bumping it once per constructor argument);
           proof/src/tactic.rs:515-518 (the `∧` branch CLONES `g.hyps` and pushes — the parent
           `h` REMAINS in scope);
           proof/src/tactic.rs:550-552 (every other hypothesis shape — a `Nat`, a list, a user
           inductive — is `DoesNotApply`: "cases expects ∧/∨/∃ hypothesis")
@loss      BINDER_NAMING. Lean's `obtain ⟨a, b⟩ := h` / `cases h with | intro a b` name the
           components; LOGOS names them `h_1`/`h_2` after the PARENT. The translator must
           rewrite every downstream reference from the Lean name to the LOGOS name, and must
           record that the user's chosen names are gone. Acceptable because the names are not
           semantically load-bearing — but only if the rewrite is total: one missed reference
           becomes `NoSuchHypothesis` in the middle of a script.
@loss      DESTRUCTURING_DEPTH. Lean's `obtain ⟨⟨a, b⟩, c⟩ := h` destructures in one step.
           LOGOS needs `cases h . cases h_1`, and the inner components are then `h_1_1` /
           `h_1_2`. Every nesting level costs an atom and lengthens the generated name.
@loss      SCRUTINEE_CLASS. LOGOS's `cases` eliminates a HYPOTHESIS of propositional shape.
           `cases n` for `n : Nat`, `cases xs` for a list, `cases c` for a user inductive are
           all outside it (tactic.rs:550-552); the generic route is `induction_over`
           (tactic.rs:623-695), which is Rust-only — see T08-010 and T08-020.
@guard     THE WITNESS-NAME TRAP, and the most dangerous part of this case. `cases h` on an
           `∃` introduces a fresh constant named `W{n}` where `n` is `ProofState::fresh`
           (tactic.rs:228-231) — a counter shared with `induction` (tactic.rs:571) and
           `induction_over` (tactic.rs:651). The witness's name therefore depends on HOW MANY
           EARLIER TACTICS IN THE SAME PROOF bumped the counter, not on anything local. A
           translator that predicts `W1` by looking at the `∃`-elimination in isolation is
           right only when no induction preceded it. Because the witness constant appears
           inside the residual goal, a wrong prediction makes every later `exact`/`rewrite`
           reference the wrong term — and since `ProofExpr` is untyped, the mismatch surfaces
           as a structural inequality, not as a name error.
@guard     Catches assuming `cases` CLEARS the scrutinee. Lean's `cases h` consumes `h`.
           LOGOS's `∧` branch clones the hypothesis list and pushes the two projections
           (tactic.rs:515-518), so `h` is still there. A translator that emits `cases h` twice
           — e.g. because it re-destructures in a later branch — gets `h_1`/`h_2` pushed a
           SECOND time, and `exact h_1` then resolves against whichever copy `find` reaches
           first (tactic.rs:379, `iter().find`), i.e. the earlier one. Silent, and correct
           only by accident.
@guard     Catches the `∨` branch's goal ORDER and its effect on a flat script. `cases h` on a
           disjunction produces TWO goals, left first, pushed to the front of `open`
           (tactic.rs:528-532 then :296-297). The script language has no focusing (see
           T08-025), so the tactics after `cases` run on the left branch until it closes and
           then continue into the right branch. A translator that emits Lean's two `·` blocks
           concatenated gets the right behaviour ONLY if the first block closes its goal
           exactly; if it closes two or leaves one open, the blocks silently misalign.
@guard     Catches mapping `rcases h with ⟨a, b⟩ | c` (an alternation pattern) to anything.
           LOGOS `cases` dispatches on the hypothesis's shape alone; there is no pattern
           language, so a mixed ∧/∨ pattern must be decomposed into a `cases` per level with
           the machine-chosen names threaded through.
@oracle    The emitted script contains one `cases` atom per destructuring level, every
           downstream hypothesis reference uses the LOGOS-generated name, and the emitted loss
           record names BINDER_NAMING and (for nested patterns) DESTRUCTURING_DEPTH. For
           `existsElim` the harness asserts the translator did not hard-code `W1` but read the
           witness name back from the resulting `ProofState`, or else refused.
-/

namespace T08_008

/-- `∧` elimination. Lean names the parts; LOGOS calls them `h_1` and `h_2`. -/
theorem andElim (p q : Prop) (h : p ∧ q) : q ∧ p := by
  obtain ⟨hp, hq⟩ := h
  exact ⟨hq, hp⟩

/-- The `cases … with` spelling of the same thing. -/
theorem andElimWith (p q : Prop) (h : p ∧ q) : p := by
  cases h with
  | intro hp _hq => exact hp

/-- NESTED destructuring in one Lean step; two LOGOS `cases`, names `h_1_1` / `h_1_2` / `h_2`. -/
theorem nestedAnd (p q r : Prop) (h : (p ∧ q) ∧ r) : p ∧ r := by
  obtain ⟨⟨hp, _hq⟩, hr⟩ := h
  exact ⟨hp, hr⟩

/-- `∨` elimination: two goals, left first. Lean's `·` blocks have no LOGOS focusing. -/
theorem orElim (p q r : Prop) (h : p ∨ q) (hpr : p → r) (hqr : q → r) : r := by
  cases h with
  | inl hp => exact hpr hp
  | inr hq => exact hqr hq

/-- `∃` elimination. The LOGOS witness is a fresh CONSTANT `W{n}` whose index depends on the
    whole proof state's counter, and the hypothesis is `h_w`. -/
theorem existsElim (p : Nat → Prop) (q : Prop) (h : ∃ n, p n) (hpq : ∀ n, p n → q) : q := by
  obtain ⟨w, hw⟩ := h
  exact hpq w hw

/-- An `∃` elimination PRECEDED by an induction, so the shared `fresh` counter has already
    advanced and the witness is not `W1`. -/
theorem existsAfterInduction (p : Nat → Prop) (q : Prop)
    (h : ∃ n, p n) (hpq : ∀ n, p n → q) (base : p 0) (step : ∀ k, p k → p (k + 1)) : q := by
  have _all : ∀ n, p n := by
    intro n
    induction n with
    | zero => exact base
    | succ k ih => exact step k ih
  obtain ⟨w, hw⟩ := h
  exact hpq w hw

/-- CLEARING. Lean's `obtain` CONSUMES `h`, so re-destructuring needs an explicit copy —
    without the `have` this file does not compile ("unknown identifier 'h'"). LOGOS's `∧`
    branch clones the hypothesis list and pushes (tactic.rs:515-518), so `h` survives and a
    second `cases h` pushes a SECOND `h_1`/`h_2` pair that shadows nothing and resolves by
    first-match. The two languages disagree about what is in scope after this step. -/
theorem clearingDiffers (p q : Prop) (h : p ∧ q) : p ∧ q := by
  have hcopy := h
  obtain ⟨hp, _⟩ := h
  obtain ⟨_, hq⟩ := hcopy
  exact ⟨hp, hq⟩

/-- `cases` on DATA, not a proposition — outside LOGOS's `cases` entirely. -/
def describe (n : Nat) : Nat := by
  cases n with
  | zero => exact 0
  | succ k => exact k

end T08_008
