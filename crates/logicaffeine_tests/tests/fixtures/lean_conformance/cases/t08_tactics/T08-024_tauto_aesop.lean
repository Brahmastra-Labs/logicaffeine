/-!
@case      T08-024
@tier      8 · Tactics
@title     `tauto` / `aesop` ⟼ `auto` — a search of a different shape, with a different axiom
           footprint, whose success is not yet a proof
@lean      tauto, aesop, trivial-by-automation, simp-driven closure
@expect    degrade
@target    LOGOS script atom `auto` — canonical `auto`, arity 0, aliases `tauto`,
           `automatically`, `automation`, `trivially`, `directly`
           (proof/src/tactic_script.rs:289, :323) → `combinators::auto`
           (proof/src/tactic.rs:1059-1062) → `ProofState::auto` (proof/src/tactic.rs:901-914) →
           `BackwardChainer::prove` (proof/src/engine.rs)
@evidence  proof/src/tactic_script.rs:323 ("auto"|"tauto"|"automatically"|"automation"|
           "trivially"|"directly" ⟹ "auto" — Lean's `tauto` and four English renderings all
           land on the same tactic);
           proof/src/tactic.rs:901-914 (`auto` builds a fresh `BackwardChainer`, adds every
           in-scope hypothesis as an AXIOM, and calls `prove` — it either CLOSES the goal or
           returns `TacticError::AutoFailed`; it never produces a subgoal);
           proof/src/engine.rs:2337-2350 (Strategy 8 — classical reductio via `dne`, tried at
           `depth == 0` for a positive goal: so `auto` may reach for the classical AXIOM where
           Lean's `tauto` would not);
           proof/src/engine.rs:2352-2356 (Strategy 9 — "Oracle fallback (Z3) — the ABSOLUTE last
           resort … it runs only at the TOP LEVEL (`depth == 0`)", certified as
           `InferenceRule::OracleVerification`, proof/src/lib.rs:488-728);
           proof/src/tactic.rs:966-972 (`qed` — the derivation becomes a `VerifiedProof` only
           after `check_derivation`); proof/src/verify.rs:31-43
           (`VerifiedProof.verified == true` IFF a kernel term was produced and typechecked);
           logicaffeine_language/src/compile.rs:1163 ("derivation found but NOT kernel-certified
           (this is not a proof)");
           proof/src/lib.rs:488-728 (46 `InferenceRule` variants, SEVEN of them dead —
           `ModalAccess`, `ModalGeneralization`, `TemporalTransitivity`, `TemporalInduction`,
           `TemporalUnfolding`, `EventualityProgress`, `UntilInduction` — so a tree carrying one
           fails certification with `KernelError::CertificationError`)
@loss      SEARCH_SHAPE. Lean's `tauto` is a propositional decision procedure and `aesop` is a
           configurable best-first search with a rule set, `simp` integration, and safe/unsafe
           rule phases. LOGOS's `auto` is a backward chainer over untyped FOL `ProofExpr` whose
           knowledge base is the in-scope hypotheses (tactic.rs:904-906). The three succeed on
           overlapping but different goal sets, and none of them is a subset of another. Every
           translated `aesop` is a bet that the LOGOS search happens to find A proof.
@loss      AXIOM_FOOTPRINT. `auto`'s Strategy 8 uses the `dne` axiom (engine.rs:2346,
           certifier.rs:1449-1473). Lean's `tauto` on a constructively provable goal produces a
           constructive term. So translating `tauto` to `auto` can turn an axiom-free Lean proof
           into one that depends on LOGOS's single classical axiom — a change a
           `#print axioms`-style audit is supposed to be able to see, and which the script text
           does not record. The loss record must name it.
@loss      NONDETERMINISM. `auto` returns whichever derivation the search finds first. Adding an
           unrelated hypothesis to the theorem changes the knowledge base
           (tactic.rs:904-906) and can change the derivation — so the translated artefact is
           not a function of the Lean proof alone, and two runs over a slightly-edited source
           produce different certified terms for the same theorem.
@guard     THE TRUST-DOOR TRAP, and the most important guard here. `ProofState::auto` returning
           `Ok` means a DERIVATION was built, not that a proof exists. Only `qed()` →
           `check_derivation` (tactic.rs:966-972) consults the kernel, and it can return a
           `VerifiedProof` with `verified == false` (verify.rs:31-43), which `compile_theorem`
           reports as "derivation found but NOT kernel-certified (this is not a proof)"
           (compile.rs:1163). A translator that reports success when the script RAN — rather
           than when `verified == true` — will mark theorems proved that are not. Because
           `auto` is the tactic most likely to be emitted for a hard goal, it is also the one
           most likely to produce an uncertifiable derivation.
@guard     Catches the DEAD-RULE failure mode. Seven of the 46 `InferenceRule` variants are
           emitted by no engine and have no certifier arm (lib.rs:488-728); a derivation
           carrying one fails with `KernelError::CertificationError`. A translator that
           constructs derivation trees directly — rather than letting `auto` build them — must
           never emit a modal or temporal rule. See T11-021.
@guard     Catches the Z3 ORACLE. Strategy 9 (engine.rs:2352-2356) falls back to an external
           SMT solver at the top level, certified as `InferenceRule::OracleVerification`. That
           moves the trust boundary outside the LOGOS kernel for that step. A translator that
           reports "kernel-certified" for a theorem whose derivation contains an
           `OracleVerification` node is overstating the guarantee, and must record it.
@guard     Catches the ALIAS SPRAY. `trivially`, `directly`, `automatically` and `automation`
           all mean `auto` (tactic_script.rs:323), while `trivial` means `assumption`
           (tactic_script.rs:315) and `grind`/`blast` mean `crush` (tactic_script.rs:319). Four
           near-synonyms, three different tactics. A translator that renders Lean tactics into
           English prose will hit the wrong one; the canonical names must be emitted.
@note      `tauto` and `aesop` are MATHLIB tactics; bare Lean 4.14 has neither. Each theorem
           below carries the Mathlib invocation in a comment and is proved with core Lean
           (`simp`, `simp_all`, explicit intro/cases, or `Classical.em`). The CONSTRUCT under
           test is the commented Mathlib call.
@oracle    Every Lean `tauto`/`aesop` maps to a single `auto` atom with a loss record naming
           SEARCH_SHAPE, AXIOM_FOOTPRINT and NONDETERMINISM; `qed()` is called and
           `verified == true` is asserted (not merely that `run_script` returned `Ok`); and the
           assembled derivation contains no dead `InferenceRule` variant. If it contains an
           `OracleVerification` node, the translator must say so in its report.
-/

namespace T08_024

/-- Mathlib: `by tauto`. Purely propositional, constructively provable — Lean's `tauto`
    produces an axiom-free term; LOGOS's `auto` may reach Strategy 8 and use `dne`. -/
theorem propTauto (p q : Prop) (hp : p) (hq : q) : p ∧ q := ⟨hp, hq⟩

/-- Mathlib: `by tauto`. A disjunctive case analysis. -/
theorem orTauto (p q : Prop) (h : p ∨ q) : q ∨ p := by
  cases h with
  | inl hp => right; exact hp
  | inr hq => left; exact hq

/-- Mathlib: `by tauto`. A CLASSICAL tautology — Lean's `tauto` uses `Classical.em` here too,
    so the axiom footprints happen to agree. -/
theorem classicalTauto (p : Prop) : p ∨ ¬p := Classical.em p

/-- Mathlib: `by aesop`. Mixed: a case split, a hypothesis use, and a simp step. -/
theorem aesopMixed (p q : Prop) (h : p ∨ q) (hpq : p → q) : q := by
  cases h with
  | inl hp => exact hpq hp
  | inr hq => exact hq

/-- Mathlib: `by aesop`. `aesop`'s `simp` integration is doing the work; LOGOS's `auto` has no
    simp phase, and LOGOS's `simp` has no default rule set (see T08-012). -/
theorem aesopViaSimp (n : Nat) (h : n + 0 = 3) : n = 3 := by
  simp at h
  exact h

/-- Mathlib: `by aesop`. A goal with an EXTRA irrelevant hypothesis — which changes `auto`'s
    knowledge base (tactic.rs:904-906) and so can change the derivation it finds. -/
theorem irrelevantHypothesis (p q r : Prop) (hp : p) (_hr : r) (hpq : p → q) : q := hpq hp

/-- Mathlib: `by tauto`. A goal `auto` will likely FAIL on, so the translator must not treat
    "emitted `auto`" as "translated". Proved here explicitly. -/
theorem hardForAuto (p q r : Prop) (h1 : p → q) (h2 : q → r) (_h3 : r → p) (hp : p) :
    p ∧ q ∧ r :=
  ⟨hp, h1 hp, h2 (h1 hp)⟩

end T08_024
