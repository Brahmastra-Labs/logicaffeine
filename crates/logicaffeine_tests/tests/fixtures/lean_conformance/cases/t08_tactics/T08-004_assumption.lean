/-!
@case      T08-004
@tier      8 · Tactics
@title     `assumption` — structural hypothesis search, not Lean's defeq-and-unification search
@lean      assumption, assumption against a defeq hypothesis, assumption with metavariables
@expect    translate
@target    LOGOS script atom `assumption` — canonical `assumption`, arity 0, aliases `done`,
           `trivial`, `established`, `hypothesis` (proof/src/tactic_script.rs:281, :315) →
           `combinators::assumption` (proof/src/tactic.rs:993-995) → `ProofState::assumption`
           (proof/src/tactic.rs:361-373)
@evidence  proof/src/tactic.rs:361-373 (`assumption` scans `g.hyps` for the FIRST `h` with
           `h.prop == g.target` — derived structural `PartialEq` on `ProofExpr`, no
           normalisation — and closes the goal with THAT hypothesis's recorded
           `DerivationTree`, so a conjunct extracted by `cases` re-emits its
           `ConjunctionElim` projection rather than a dangling name);
           proof/src/tactic_script.rs:315 ("assumption"|"done"|"trivial"|"established"|
           "hypothesis" ⟼ "assumption");
           proof/src/tactic_script.rs:281 (arity 0 — `arg()` is never called, so a trailing word
           after `assumption` is parsed as the NEXT atom);
           proof/src/tactic.rs:183-186 (`direct_hyp` — every in-scope hypothesis carries a
           `PremiseMatch` leaf as its proof, which is what the certifier resolves)
@guard     Catches assuming Lean's search strength. Lean's `assumption` matches a hypothesis
           against the goal up to REDUCIBLE definitional unfolding and instantiates the goal's
           metavariables while doing so. LOGOS's is `h.prop == g.target` on an untyped FOL
           `ProofExpr` (tactic.rs:363). `h : ¬ p` does not close the goal `p → False` — those
           are `Not(p)` and `Implies(p, False)`, different constructors (lib.rs:224, :230).
           `h : p ∧ q` does not close `q ∧ p`. A translator that emits `assumption` wherever
           Lean wrote it will produce scripts that fail on goals Lean discharged, and the
           reported error is `NoSuchHypothesis`, which reads like a naming bug.
@guard     Catches the alias `trivial` (tactic_script.rs:315). Lean's `trivial` is a DIFFERENT
           tactic: it proves `True`, closes `rfl` goals, and runs `assumption` among other
           things. LOGOS's `trivial` IS `assumption` and nothing else — so Lean's
           `trivial` on the goal `True` becomes a LOGOS `assumption` that fails unless `True`
           is literally a hypothesis. Emitting the English word instead of the canonical name
           is how this slips through: the alias table makes the wrong word parse.
@guard     Catches the alias `done` (tactic_script.rs:315). Lean's `done` SUCCEEDS iff there are
           no goals left and proves nothing; LOGOS's `done` closes the focused goal from a
           hypothesis. A translator that emits `done` to assert "the proof is finished" instead
           discharges a goal — and if it happens to succeed, the script is now one step short
           of where the translator thought it was, so every subsequent step targets the wrong
           goal.
@guard     Catches losing the PROJECTION. `assumption` closes with `h.proof`, not with a fresh
           leaf (tactic.rs:364-365). For a conjunct introduced by `cases` the recorded proof is
           a `ConjunctionElim` node over the parent (tactic.rs:505-517), so the certified term
           is the projection applied to the parent's proof. A translator that "optimises" by
           re-emitting a `PremiseMatch` leaf for the conjunct name produces a derivation whose
           leaf cites a hypothesis the certifier's context does not bind, and
           `check_derivation` fails with an unbound reference far from the `cases` that caused it.
@oracle    `ProofState::run_script` on the translated goal closes it, `qed()` returns a
           `VerifiedProof` with `verified == true`, and for `conjunctViaCases` the assembled
           derivation's closing node is a `ConjunctionElim`, not a bare `PremiseMatch`.
-/

namespace T08_004

/-- The shape that translates exactly: the goal IS a hypothesis, structurally. -/
theorem plain (p q : Prop) (_hq : q) (hp : p) : p := by assumption

/-- Several candidates; LOGOS takes the first matching one, Lean the last-declared one.
    Irrelevant here because the propositions are distinct, which is the point: a translator
    must not rely on WHICH hypothesis is chosen when two prove the same thing. -/
theorem firstMatch (p : Prop) (h1 : p) (h2 : p) : p := by
  have _ := h1
  have _ := h2
  assumption

/-- Defeq boundary: Lean closes this because `¬ p` unfolds to `p → False`. -/
theorem defeqBoundary (p : Prop) (h : ¬ p) : p → False := by assumption

/-- Commutation boundary: structurally different, so LOGOS's `==` fails. -/
theorem commBoundary (p q : Prop) (h : p ∧ q) : q ∧ p := by
  exact ⟨h.2, h.1⟩

/-- The projection case: the hypothesis that closes the goal was produced by `cases`, so its
    recorded proof is a projection, not a premise reference. -/
theorem conjunctViaCases (p q : Prop) (h : p ∧ q) : p := by
  cases h with
  | intro hp _hq => assumption

/-- Lean's `trivial` on `True` — NOT the same tactic as LOGOS's `trivial` alias. -/
theorem trivialOnTrue : True := by trivial

end T08_004
