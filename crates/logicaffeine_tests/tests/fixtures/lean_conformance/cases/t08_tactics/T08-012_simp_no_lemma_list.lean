/-!
@case      T08-012
@tier      8 · Tactics
@title     `simp only [foo, bar]` — LOGOS's `simp` takes NO lemma list and silently uses every
           hypothesis in scope instead
@lean      simp, simp only [..], simp [..], simp at h, simp with a default @[simp] set
@expect    degrade
@target    LOGOS script atom `simp` — canonical `simp`, arity **0**, aliases `simplify`,
           `normalize`, `simplification` (proof/src/tactic_script.rs:282, :316) →
           `combinators::simp` (proof/src/tactic.rs:996-1002), whose rule set is
           `ProofState::scope_simp_set()` (proof/src/tactic.rs:261-272) →
           `ProofState::simp` (proof/src/tactic.rs:750-827)
@evidence  proof/src/tactic_script.rs:282 (`"simp" => Ok(c::simp())` — `arg()` is never called;
           the atom is nullary and a following word is read as the NEXT atom);
           proof/src/tactic_script.rs:51-105 (the script lexer has `[` and `]` tokens, but
           `parse_atom` (:212-243) accepts a bracket ONLY after the keyword `first` (:221-231),
           so `simp only [h]` cannot be parsed at all);
           proof/src/tactic.rs:261-272 (`scope_simp_set` — the rule set is built by
           `register_lemma` over EVERY in-scope hypothesis, with no way to select);
           proof/src/simp.rs:83-91 (`register_lemma` peels the ∀-prefix and refuses a lemma with
           a shadowed binder, returning `false` — so some hypotheses silently do NOT become
           rules and there is no report of which);
           proof/src/tactic.rs:750-756 (`const BUDGET: usize = 1000` plus a seen-goal set, so
           the fixpoint is bounded and may stop mid-normalisation);
           proof/src/tactic.rs:806-826 (the closing attempts — reflexivity, then `assumption`;
           and `DoesNotApply` when it neither rewrote nor closed);
           proof/src/tactic_script.rs:385-396, :421-437 (`ScriptedTheorem::simp` and the
           `simp_pool` — the ONLY `[simp]`-attribute analogue, and it exists only inside
           `prove_scripted_library`, never for a standalone `## Theorem` script)
@loss      RULE_SET_SELECTION. `simp only [foo, bar]` names its rules precisely; LOGOS's `simp`
           cannot name any. The loss runs in the dangerous direction: LOGOS uses MORE rules
           than asked for, not fewer. Two `simp only` calls in the same Lean proof that
           deliberately normalise to different shapes both become the same LOGOS `simp`, which
           normalises past the first one's intended stopping point.
@loss      DEFAULT_SET. Lean's bare `simp` runs the global `@[simp]` set — thousands of core
           lemmas. LOGOS's bare `simp` runs the LOCAL hypotheses only (tactic.rs:264-272). The
           two spellings are identical and mean almost disjoint things. Every arithmetic
           identity Lean's `simp` discharges from the default set (`Nat.add_zero`,
           `List.append_nil`, …) must be supplied as an explicit premise or a cited lemma, or
           the LOGOS `simp` fails with `DoesNotApply`.
@loss      LOCATION. `simp at h` and `simp_all` normalise hypotheses. `ProofState::simp` reads
           and rewrites only `g.target` (tactic.rs:757, :766). No location clause exists.
@guard     THE CENTRAL TRAP. A translator that maps `simp only [Nat.add_zero]` to `simp`
           produces a step that is *stronger and differently-shaped*. If the extra rewrites
           happen to close the goal, the proof "works" and the translated script no longer
           corresponds to the Lean proof at all — a later step that expected the `simp only`
           residue now runs against `NoOpenGoals`. If they do not close it, the residue differs
           and every downstream `exact`/`rewrite` misses structurally. Both outcomes are
           silent at translation time.
@guard     Catches emitting the lemma list anyway. `simp only [h]` written verbatim parses as
           the atom `simp`, then the atom `only` — `canonical("only")` is `None` — so the whole
           script dies at parse time with "unknown tactic `only`". A translator must strip the
           list, and stripping it is exactly the loss above, so the stripping MUST be recorded.
@guard     Catches the silently-dropped rules. `simp::register_lemma` returns `false` for a
           lemma whose ∀-prefix shadows a binder (simp.rs:87-90) and `scope_simp_set`
           (tactic.rs:264-272) IGNORES the return value. So a hypothesis the translator
           supplied as a rewrite rule may simply not be in the set, with no diagnostic, and the
           `simp` then fails for reasons invisible from the script.
@guard     Catches relying on the `[simp]` POOL. `ScriptedTheorem { simp: true }` puts a proved
           conclusion into a pool that later theorems in the SAME `prove_scripted_library` call
           see (tactic_script.rs:392-395, :432-437). It is not an environment attribute: it does
           not persist, it is not visible to a `## Theorem` block, and it is keyed on proof
           order within one Rust call. A translator that maps Lean's `@[simp]` attribute onto it
           has changed a global, persistent fact into a call-local one.
@guard     Catches the budget. 1000 steps plus a seen-goal set (tactic.rs:751-754, :767) means
           `simp` can stop with the goal PARTIALLY normalised and still report success, because
           it returns `Ok` whenever `progress` is true (tactic.rs:820-821). The residual shape
           is then neither the input nor the Lean normal form.
@oracle    No emitted script contains a `[` outside a `first [...]` construct. Every Lean
           `simp only [...]` maps to a `simp` atom accompanied by a loss record naming
           RULE_SET_SELECTION, and the harness asserts that the goal after `run_script`'s `simp`
           is `types_equal` to the translation of Lean's post-`simp` goal — failing the case if
           LOGOS normalised further.
-/

namespace T08_012

/-- Bare `simp`. In Lean this uses the global `@[simp]` set; in LOGOS only local hypotheses. -/
theorem bareSimp (n : Nat) : n + 0 = n := by simp

/-- `simp only` with a NAMED rule — the selection LOGOS cannot express. -/
theorem simpOnly (n : Nat) : n + 0 = n := by simp only [Nat.add_zero]

/-- Two `simp only` calls that deliberately stop at DIFFERENT normal forms. Collapsing both to
    the same nullary `simp` destroys the distinction. -/
theorem stagedNormalisation (a b : Nat) : (a + 0) + (b + 0) = a + b := by
  simp only [Nat.add_zero]

/-- `simp [h]` adding a LOCAL hypothesis to the default set — the one shape whose LOGOS
    counterpart is closest, since local hypotheses are exactly what `scope_simp_set` collects. -/
theorem simpWithHyp (f : Nat → Nat) (a : Nat) (h : f a = a) : f (f a) = a := by
  simp [h]

/-- `simp at h` — normalising a HYPOTHESIS. No LOGOS image. -/
theorem simpAt (n : Nat) (h : n + 0 = 3) : n = 3 := by
  simp at h
  exact h

/-- `simp_all` — everything, everywhere. No LOGOS image. -/
theorem simpAll (n m : Nat) (h : n + 0 = m) : n = m := by
  simp_all

/-- A LOCAL `@[simp]` attribute: a persistent, global fact in Lean; at best a call-local
    `simp_pool` entry in LOGOS (tactic_script.rs:392-395). -/
@[simp] theorem localSimpLemma (n : Nat) : n * 1 = n := Nat.mul_one n

theorem usesLocalSimpLemma (k : Nat) : k * 1 = k := by simp

/-- A `simp` that must NOT over-normalise: the residue is consumed by the next step. -/
theorem residueMatters (p : Nat → Prop) (a : Nat) (hp : p a) : p (a + 0) := by
  simp only [Nat.add_zero]
  exact hp

end T08_012
