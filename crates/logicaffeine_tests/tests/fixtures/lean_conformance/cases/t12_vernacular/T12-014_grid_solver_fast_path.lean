/-!
@case      T12-014
@tier      12 · Vernacular surfaces
@title     The grid-solver fast path silently routes `Auto` to a different engine and a different proof object
@lean      a finite-domain entailment: a declared bijection (closure + at-most-one) over
           two small categories, plus cross-pins, concluding one cell
@expect    degrade
@target    The same Surface A `## Theorem` block as T12-001 — no syntax marks the fast
           path — proved through `compile_theorem_for_ui`:

           ```logos
           ## Theorem: Simon
           Given: Alpha is a trip.
           Given: Beta is a trip.
           Given: Alpha is not Beta.
           Given: Every trip is in 2003 or in 2004.
           Given: Every trip is in Florida or in Maine.
           Given: Exactly one trip is in 2003.
           Given: Exactly one trip is in Florida.
           Given: Alpha is in 2003.
           Given: Alpha is in Florida.
           Prove: Beta is in Maine.
           Proof: Auto.
           ```

           whose result carries `verified == true` and a `DerivationTree` built by
           `logicaffeine_proof::grid_solver::grid_prove` — NOT by the backward chainer.
@evidence  tests/phase_studio_grid_proof.rs:17-46 — the block above is quoted verbatim
           from `grid_cell_proves_via_studio_path`, asserted `r.verified` and asserted
           that the rendered tree contains `"Contradiction"` or `"DisjunctionElim"`;
           tests/phase_studio_grid_proof.rs:51-64 — the contrast case: the Socrates
           syllogism through the SAME entry point must keep `"UniversalInst"` in its
           tree, i.e. must NOT be grounded;
           compile/src/ui_bridge.rs:851-905 — the fast path. `if looks_like_grid(&premises)`
           replaces `prove_certify_check` with: `prepare_premises_opts(&premises, true)`,
           then `grid_solver::grid_prove`, then `verify::check_derivation`, then
           `.filter(|vp| vp.verified)`; on failure it consults the RUP tier
           (`rup::entails_certified`) and only then falls back to
           `prove_certify_check_bounded(&grounded, &g, 60)`;
           compile/src/ui_bridge.rs:921-923 — the `else` arm, the ordinary
           `prove_certify_check(&premises, &goal)`;
           compile/src/ui_bridge.rs:3939-3953 — `looks_like_grid`: true when
           `grounding::at_most_one_lemmas(premises)` is non-empty OR any premise is a
           `ForAll` (possibly nested, possibly under `Temporal`) whose body is an
           `Implies` with an `Or` consequent. That is the entire trigger — a structural
           test on the premises, with no flag and no syntax;
           compile/src/ui_bridge.rs:827-831 — a solved grid table is additionally
           attached to the result whenever `is_auto && looks_like_grid(&premises)`;
           compile/src/ui_bridge.rs:3963-3985 — `prepare_premises_opts(.., true)` adds
           `at_most_one_lemmas`, `definite_property_implications` AND
           `functionality_lemmas`, the last of which is given ONLY to the grid solver
           and deliberately withheld from the backward chainer;
           compile/src/ui_bridge.rs:832-845 — a third route on the same `is_auto` arm:
           an `Exists`-shaped goal is answered by `answer_wh`, returning witnesses and
           NO derivation at all (`(None, verified, verr, Some(witnesses))`).
@loss      PROOF_OBJECT_IDENTITY. The theorem is the same and the `verified` flag is the
           same, but the artefact is not: the fast path returns a solver tree
           (`Contradiction` / `DisjunctionElim` over grounded, quantifier-free premises)
           where the general path would return `UniversalInst` / `ModusPonens` over the
           quantified originals. Any translator obligation phrased over the derivation —
           round-tripping it, diffing it against the Lean proof term, counting rule
           applications, asserting a proof *shape* — is measuring a different object
           depending on premise structure it did not choose.
@loss      PREMISE_SET_AUGMENTED. The proof is not of `premises ⊢ goal` as emitted. The
           grid route proves `prepare_premises_opts(premises, true) ⊢ erase_tense(goal)`,
           where the premise set has been extended with at-most-one lemmas, definite-
           description implications and functionality lemmas, and tense has been erased
           from the goal. These are entailed by the originals, so the result is sound —
           but a conformance report that claims "LOGOS proved exactly the premises we
           emitted" is overclaiming.
@loss      ENGINE_NONDETERMINISM_ACROSS_ENTRY_POINTS. `compile_theorem`
           (language/src/compile.rs:1152) has NO grid branch: it always calls
           `prove_certify_check`. `compile_theorem_for_ui` (compile/src/ui_bridge.rs:851)
           does. So the same emitted text proved through the two public entry points
           takes two different engines, and may differ in success as well as in shape —
           the fast path exists precisely because the general chainer is impractical on
           these premises.
@guard     Catches a harness that pins the proof tree. Adding one premise of the form
           "every X is A or B", or one "exactly one X is A", flips `looks_like_grid` and
           changes every subsequent derivation in that theorem from `UniversalInst` to
           `Contradiction`/`DisjunctionElim`. A regression suite that snapshots rendered
           trees will show a large unexplained diff caused by a premise the translator
           emitted for unrelated reasons.
@guard     Catches choosing an entry point by convenience. `compile_theorem` returns a
           formatted `String` and is the obvious API; `compile_theorem_for_ui` returns a
           `TheoremCompileResult` with a `verified` field (T12-013) and is the one the
           suite's own tests assert on. They are not interchangeable: the former lacks
           the grid route, the wh-answer route, and the structured trust flag. A
           translator that develops against one and reports against the other will see
           theorems that "regress" with no change to the emitted text.
@guard     Catches assuming a returned derivation exists. On the `Exists`-goal route
           (ui_bridge.rs:832-841) the result is `verified == true` with
           `derivation: None` and an `answer` list of witnesses. A harness that requires
           `derivation.is_some()` as part of its pass predicate will fail every
           existential goal that LOGOS actually answered.
@guard     Catches reading the fast path as an optimisation that cannot change verdicts.
           It can: the grid route's fallback is `prove_certify_check_bounded(&grounded,
           &g, 60)` — depth 60 over GROUNDED premises — whereas the general route is
           `prove_certify_check`, i.e. depth 100 over the originals
           (proof/src/verify.rs:70-72). A goal reachable at depth 80 in the general
           engine is not reachable in the grid route's fallback.
@oracle    `compile_theorem_for_ui(<emitted text>)` returns `r.verified == true`, AND the
           loss record names PROOF_OBJECT_IDENTITY whenever
           `looks_like_grid` would fire — which a translator can predict from its own
           emitted premises without calling LOGOS: any premise rendering as `∀x(P x → (A x ∨ B x))`,
           or any "exactly one" premise. A harness must NOT assert on derivation shape
           for such cases, and must not compare trees across entry points.
-/

namespace T12_014

/- The premise lists below are deliberately COMPLETE: each Lean hypothesis mirrors one
   `Given:` line of the LOGOS block, including the premises Lean's proof happens not to
   need. That completeness is the point of the case — it is what makes
   `looks_like_grid` fire — so the unused-variable linter is off here. -/
set_option linter.unusedVariables false

axiom Thing : Type
axiom Trip : Thing → Prop
axiom In2003 : Thing → Prop
axiom In2004 : Thing → Prop
axiom InFlorida : Thing → Prop
axiom InMaine : Thing → Prop
axiom alpha : Thing
axiom beta : Thing

/-- The two-category Simon slice, premise for premise.

    `hYear` and `hState` are the DISJUNCTIVE CLOSURES — `∀x(Trip x → (A x ∨ B x))` — that
    `looks_like_grid` (ui_bridge.rs:3940-3950) pattern-matches on. `hOne2003` and
    `hOneFlorida` are the at-most-one halves of "exactly one", which is the other
    trigger (ui_bridge.rs:3951). Either one alone reroutes the whole theorem. -/
theorem betaInMaine
    (hAlphaTrip : Trip alpha)
    (hBetaTrip : Trip beta)
    (hDistinct : alpha ≠ beta)
    (hYear : ∀ t, Trip t → (In2003 t ∨ In2004 t))
    (hState : ∀ t, Trip t → (InFlorida t ∨ InMaine t))
    (hOne2003 : ∀ t u, In2003 t → In2003 u → t = u)
    (hOneFlorida : ∀ t u, InFlorida t → InFlorida u → t = u)
    (hAlpha2003 : In2003 alpha)
    (hAlphaFlorida : InFlorida alpha) :
    InMaine beta := by
  cases hState beta hBetaTrip with
  | inl hb => exact absurd (hOneFlorida alpha beta hAlphaFlorida hb) hDistinct
  | inr hb => exact hb

/-- The same conclusion for the year category, to show the trigger is structural and not
    about any particular predicate. -/
theorem betaIn2004
    (hBetaTrip : Trip beta)
    (hDistinct : alpha ≠ beta)
    (hYear : ∀ t, Trip t → (In2003 t ∨ In2004 t))
    (hOne2003 : ∀ t u, In2003 t → In2003 u → t = u)
    (hAlpha2003 : In2003 alpha) :
    In2004 beta := by
  cases hYear beta hBetaTrip with
  | inl hb => exact absurd (hOne2003 alpha beta hAlpha2003 hb) hDistinct
  | inr hb => exact hb

/-- The contrast case (phase_studio_grid_proof.rs:51-64): no closure premise, no
    "exactly one" premise, so `looks_like_grid` is false and the ordinary
    `prove_certify_check` route runs, keeping a `UniversalInst` trace. The Lean source
    is nearly identical in size and shape — nothing about the theorem tells a translator
    which engine it will get. -/
axiom Man : Thing → Prop
axiom Mortal : Thing → Prop

theorem socratesMortality
    (hAll : ∀ x, Man x → Mortal x) (hMan : Man alpha) : Mortal alpha :=
  hAll alpha hMan

/-- An existential goal, which on the same `is_auto` arm takes the THIRD route
    (`answer_wh`, ui_bridge.rs:832-841): `verified == true` with `derivation == None`. -/
theorem someoneInMaine (hBetaTrip : Trip beta) (hb : InMaine beta) :
    ∃ t, Trip t ∧ InMaine t :=
  ⟨beta, hBetaTrip, hb⟩

end T12_014
