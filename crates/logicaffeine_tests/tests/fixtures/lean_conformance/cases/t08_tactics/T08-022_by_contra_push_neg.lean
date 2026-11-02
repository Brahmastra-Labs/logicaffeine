/-!
@case      T08-022
@tier      8 · Tactics
@title     `by_contra` — reachable only as `auto`'s LAST-RESORT strategy at search depth 0,
           never as a step; `push_neg` has no analogue at all
@lean      by_contra, by_contra!, push_neg, not_not, negation normal form
@expect    degrade
@target    LOGOS script atom `auto` — canonical `auto`, arity 0, aliases `tauto`,
           `automatically`, `automation`, `trivially`, `directly`
           (proof/src/tactic_script.rs:289, :323) → `combinators::auto`
           (proof/src/tactic.rs:1059-1062) → `ProofState::auto` (proof/src/tactic.rs:901-914) →
           `BackwardChainer::prove`, whose Strategy 8 emits
           `InferenceRule::ClassicalReductio` (proof/src/engine.rs:2337-2350), certified as
           `dne G (λ hng. <⊥-proof>)` (proof/src/certifier.rs:1447-1473) against the single
           classical axiom `dne` (kernel/src/prelude.rs:78)
@evidence  proof/src/engine.rs:2337-2350 (Strategy 8, verbatim: "Classical reductio (proof by
           contradiction) — LAST RESORT at the top level for a POSITIVE goal G: assume ¬G and
           search for ⊥". The guard is `if depth == 0 && !is_falsum(&goal.target) &&
           !matches!(goal.target, ProofExpr::Not(_))`);
           proof/src/certifier.rs:1449-1473 (the `ClassicalReductio` arm builds
           `dne G (λ(hng : ¬G). <False proof>)` — the ONLY use of `dne` in the whole certifier);
           kernel/src/prelude.rs:78 (`ctx.add_declaration("dne", dne_type)` — the whole
           classical fragment of the trusted base is this one axiom; there is no
           `Classical.choice`, no `propext`, no `funext`);
           proof/src/tactic_script.rs:308-327 (`canonical` — no arm for `by_contra`,
           `contrapose`, `push_neg`, or any negation-normalisation word);
           proof/src/tactic.rs:901-914 (`auto` either CLOSES the goal or fails with
           `TacticError::AutoFailed` — it never produces a subgoal, so it cannot be a step in a
           longer script);
           proof/src/tactic.rs:264-272 and :750-826 (`simp`'s rule set is the in-scope
           hypotheses only — there are no De Morgan lemmas in the LOGOS prelude, so `simp`
           cannot normalise negations either)
@loss      REDUCTIO_IS_NOT_A_STEP. Lean's `by_contra h` is a STEP: it adds `h : ¬G` and leaves
           `False` as the goal, and the rest of the tactic block proves it. LOGOS's reductio is
           a SEARCH STRATEGY inside `auto` — it is tried only when the backward chainer has
           exhausted everything else, only at `depth == 0`, and only for a positive non-`False`
           goal (engine.rs:2340). A Lean proof of the shape `intro x; by_contra h; <20 steps>`
           has no step-wise image: the whole subproof must be handed to `auto` as one opaque
           call, and if `auto` cannot find the contradiction, there is no way to help it.
@loss      NNF_UNAVAILABLE. `push_neg` rewrites `¬∀`, `¬∃`, `¬(p ∧ q)`, `¬(a < b)` into
           positive form. LOGOS has no De Morgan rewriting: `simp`'s rules are the scope's
           hypotheses (tactic.rs:264-272), and the prelude registers no propositional lemmas.
           Every `push_neg` must be discharged by `auto` on the un-normalised goal, or the
           translator must supply the De Morgan equivalence as an explicit premise — which
           changes the theorem's hypothesis set.
@guard     THE `depth == 0` GATE, and the reason a naive mapping fails in the middle. Because
           Strategy 8 fires only at `depth == 0` (engine.rs:2340), classical reductio is NOT
           available for any subgoal the chainer generates internally. A Lean proof whose
           `by_contra` sits under an `intro` translates to a script whose `auto` runs on the
           post-`intro` goal — which IS depth 0 for that `auto` call, so it works — but a Lean
           proof whose `by_contra` is needed for a subgoal of a larger automatic search will
           never reach it. The difference is invisible from the script text: both spellings are
           the word `auto`.
@guard     Catches emitting `crush` instead. `crush` (tactic.rs:884-896) is e-matching plus
           congruence closure; it has no reductio strategy at all. A translator that tries
           `first [ auto | crush ]` to "be safe" will silently prefer whichever succeeds, and
           `crush` succeeding means the proof went through WITHOUT the classical axiom — a
           different, constructively-weaker derivation than the Lean source's, which is a
           difference a `#print axioms`-style audit is supposed to be able to see.
@guard     Catches the ALIAS `tauto` (tactic_script.rs:323). Lean's `tauto` is a propositional
           decision procedure; LOGOS's `tauto` is `auto`, the full backward chainer WITH
           classical reductio. Emitting the word `tauto` therefore silently enables the `dne`
           axiom for a proof the Lean source intended to be propositional. The axiom footprint
           of the translated theorem is larger than the original's, and nothing records it.
@guard     Catches treating `auto`'s success as a proof. `ProofState::auto` closing the goal
           produces a DERIVATION; only `qed()` → `check_derivation` (tactic.rs:966-972) turns it
           into a `VerifiedProof`, and `verified == false` is a possible outcome
           (proof/src/verify.rs:31-43) that `compile_theorem` reports as "derivation found but
           NOT kernel-certified (this is not a proof)" (compile.rs:1163). A reductio derivation
           is exactly the kind that can fail certification, because the `ClassicalReductio` arm
           must reconstruct `¬G` as a kernel type (certifier.rs:1456-1458) and a `ProofExpr`
           that does not translate to a `Prop` will fail there.
@note      `by_contra` and `push_neg` are MATHLIB tactics; bare Lean 4.14 has neither
           (`by_contra` reports "unknown tactic"). Each theorem below carries the Mathlib
           invocation in a comment and is proved with core Lean — `intro`/`exact` against
           `Classical.byContradiction`, or explicit `Decidable`/`Classical.em` reasoning. The
           CONSTRUCT under test is the commented Mathlib call.
@oracle    Every Lean `by_contra` maps to a single `auto` atom that is given the ENTIRE
           remaining subproof, accompanied by a loss record naming REDUCTIO_IS_NOT_A_STEP; the
           resulting derivation's root (or some node) is `InferenceRule::ClassicalReductio`; and
           `qed()` returns `verified == true`. For `pushNegForall` the translator must record
           NNF_UNAVAILABLE, and the harness asserts no emitted script contains `push_neg`.
-/

namespace T08_022

/-- Mathlib: `by by_contra h; exact h hp`. The core spelling of the same move. -/
theorem contraDirect (p : Prop) (hp : p) : p := by
  apply Classical.byContradiction
  intro hn
  exact hn hp

/-- Double negation elimination — exactly `dne`, the one classical axiom LOGOS trusts. -/
theorem dneCore (p : Prop) (h : ¬¬p) : p := by
  apply Classical.byContradiction
  intro hn
  exact h hn

/-- Mathlib: `by by_contra h; …`. A reductio UNDER an `intro`, so the subproof that follows is
    what must be handed to `auto` wholesale. -/
theorem contraUnderIntro (p q : Prop) (h : ¬p → q) (hnq : ¬q) : p := by
  apply Classical.byContradiction
  intro hnp
  exact hnq (h hnp)

/-- Mathlib: `by push_neg at h`. Negation of a `∀` becomes an `∃`. No LOGOS normalisation. -/
theorem pushNegForall (p : Nat → Prop) (h : ¬ ∀ n, p n) : ∃ n, ¬ p n := by
  apply Classical.byContradiction
  intro hn
  apply h
  intro n
  apply Classical.byContradiction
  intro hnp
  exact hn ⟨n, hnp⟩

/-- Mathlib: `by push_neg`. De Morgan over `∧`. -/
theorem pushNegAnd (p q : Prop) (h : ¬ (p ∧ q)) : ¬p ∨ ¬q := by
  rcases Classical.em p with hp | hnp
  · right
    intro hq
    exact h ⟨hp, hq⟩
  · left
    exact hnp

/-- Excluded middle, which LOGOS derives from `dne` rather than assuming — see T06-008. -/
theorem em (p : Prop) : p ∨ ¬p := Classical.em p

/-- A goal that is ITSELF a negation: Strategy 8's guard
    `!matches!(goal.target, ProofExpr::Not(_))` (engine.rs:2340) excludes it, so reductio is
    not even attempted here. -/
theorem negativeGoal (p : Prop) (h : p → False) : ¬ p := by
  intro hp
  exact h hp

end T08_022
