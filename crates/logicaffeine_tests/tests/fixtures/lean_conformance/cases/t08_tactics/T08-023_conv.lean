/-!
@case      T08-023
@tier      8 · Tactics
@title     `conv` — positional rewriting is exactly what LOGOS's all-occurrences `rewrite`
           cannot express
@lean      conv => lhs; rw [..], conv at h, conv in <pattern>, nth_rewrite
@expect    reject
@target    (none — no sound translation)
@evidence  a grep of `crates/logicaffeine_proof/src` and `crates/logicaffeine_kernel/src` for
           `conv` returns NOTHING — there is no conversion-mode state, no zipper, no subterm
           navigator anywhere in LOGOS; proof/src/tactic_script.rs:308-327 (`canonical` — no arm
           for `conv`, `lhs`, `rhs`, `enter`, `congr` or `nth_rewrite`);
           proof/src/tactic.rs:113-128 (`replace_in_term` — the ONLY rewriting primitive:
           `if t == from { return to.clone() }` then recurse into every argument, with no
           position argument, no occurrence counter, and no way to stop early);
           proof/src/tactic.rs:130-156 (`replace_in_expr` — same, over the whole formula);
           proof/src/tactic.rs:702-737 (`ProofState::rewrite` calls `replace_in_expr` once on
           `g.target` and fails only if NOTHING changed, :719-723 — there is no "rewrite the
           second occurrence" mode and no way to observe how many occurrences changed);
           proof/src/tactic.rs:750-826 (`simp` — the other rewriting tactic — likewise rewrites
           through `replace_in_expr` at :766, so it is all-occurrences too);
           proof/src/lib.rs:488-728 (`InferenceRule::Rewrite { from, to }` carries only the two
           TERMS — there is no position, path, or occurrence index in the certified record, so
           even a Rust-level translator has nowhere to write one)
@diag      E_NO_CONVERSION_MODE — must name the theorem, report the conversion path the `conv`
           block navigates (`lhs`, `rhs`, `enter [2]`, a pattern), and state that LOGOS's only
           rewriting primitive is all-occurrences `replace_in_expr` (tactic.rs:113-156) with no
           positional form in the certified `InferenceRule::Rewrite` either. Must NOT emit a
           `rewrite` atom.
@guard     THE CENTRAL POINT, and the reason this case sits next to T08-011. `conv` exists in
           Lean *because* `rw` is not positional enough. Translating a `conv` block by dropping
           the navigation and emitting the inner `rw` as a LOGOS `rewrite` produces exactly the
           behaviour the user wrote `conv` to avoid: every occurrence is rewritten. The result
           is a well-typed term, a well-formed derivation, and a different goal. Nothing in the
           kernel can object, because the kernel sees only the finished `Rewrite` node and its
           two terms.
@guard     Catches the "it's the same when there is one occurrence" rationalisation. A `conv`
           block whose target occurs exactly once IS equivalent to a plain rewrite — and the
           translator cannot know that in general, because deciding it requires counting
           occurrences in the TRANSLATED goal, after every earlier step has run. A translator
           that counts occurrences in the LEAN goal instead is counting in a different term:
           Lean's `Expr` has `mdata`, implicit arguments, and instance arguments that LOGOS's
           `ProofExpr` does not, so occurrence counts differ. If a translator wants to admit the
           single-occurrence case it must count on the LOGOS side, at emission time, and record
           the check.
@guard     Catches `conv at h`. Conversion mode can target a HYPOTHESIS; `ProofState::rewrite`
           reads and writes `g.target` only (tactic.rs:718, :730). Both the positional aspect
           and the location aspect are missing, and a translator that "fixes" only one of them
           has fixed nothing.
@guard     Catches Mathlib's `nth_rewrite n [h]`. It is the compact spelling of the same
           request and must be rejected by the same rule. A translator that special-cases
           `nth_rewrite 1 [h]` as "the first occurrence, which is what `rw` does anyway" is
           wrong twice: Lean's `rw` rewrites all occurrences of the instantiated pattern
           (verified in T08-011), not the first, and LOGOS's does the same — so `nth_rewrite 1`
           is NOT the default behaviour of either.
@guard     Catches `conv` used for its OTHER purpose — entering under a binder to rewrite inside
           a `∀`/`λ` body. LOGOS's `replace_in_expr` already recurses into `ForAll`/`Exists`
           bodies (tactic.rs:148-153), so the navigation is unnecessary there — and, per
           T08-011, it does so WITHOUT binder protection, which is a capture bug rather than a
           feature. A translator must not read "LOGOS already rewrites under binders" as "the
           `conv` was redundant".
@oracle    Translator exits non-zero with E_NO_CONVERSION_MODE for every theorem below and emits
           no script containing a `rewrite` atom derived from a `conv` block. A harness
           additionally asserts that every emitted `rewrite` in the corpus corresponds to a Lean
           `rw`/`subst` whose pattern occurs exactly once in the translated goal, or carries the
           T08-011 loss record.
-/

namespace T08_023

/-- The canonical positional rewrite: `a` occurs on BOTH sides, and only the left one is meant
    to change. LOGOS's `rewrite h` would change both and close the goal for the wrong reason. -/
theorem lhsOnly (a b : Nat) (h : a = b) : a + a = b + a := by
  conv =>
    lhs
    lhs
    rw [h]

/-- Entering a specific ARGUMENT position. -/
theorem enterArgument (f : Nat → Nat → Nat) (a b : Nat) (h : a = b) :
    f a a = f b a := by
  conv =>
    lhs
    enter [1]
    rw [h]

/-- `conv` under a BINDER — Lean must navigate in; LOGOS's rewrite recurses in unconditionally
    (and unsafely, see T08-011). -/
theorem underBinder (f g : Nat → Nat) (h : ∀ x, f x = g x) :
    (fun x => f x) = (fun x => g x) := by
  conv =>
    lhs
    ext x
    rw [h]

/-- `conv at h` — conversion inside a HYPOTHESIS. -/
theorem convAtHyp (a b : Nat) (p : Nat → Nat → Prop) (h : a = b) (hp : p a a) : p b a := by
  conv at hp =>
    lhs
    rw [h]
  exact hp

/-- The single-occurrence case, where a plain rewrite happens to agree — and which a
    translator may admit only after counting occurrences on the LOGOS side. -/
theorem singleOccurrence (a b c : Nat) (h : a = b) : a + c = b + c := by
  conv =>
    lhs
    rw [h]

/-- Mathlib: `by nth_rewrite 2 [h]`. The compact spelling; same rejection. -/
theorem nthOccurrence (a b : Nat) (h : a = b) : a + a = a + b := by
  conv =>
    rhs
    rw [← h]

end T08_023
