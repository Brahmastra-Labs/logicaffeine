/-!
@case      T08-011
@tier      8 · Tactics
@title     `rw` ⟼ `rewrite`: all-occurrences Leibniz on a GROUND equation, no direction, no
           `at`, no ∀-instantiation, and no binder protection
@lean      rw [h], rw [← h], rw [h] at h', rw with a ∀-quantified equation, subst
@expect    degrade
@target    LOGOS script atom `rewrite <name>` — canonical `rewrite`, arity 1, aliases `rw`,
           `substitute`, `subst` (proof/src/tactic_script.rs:279, :313) →
           `combinators::rewrite` (proof/src/tactic.rs:1043-1047) → `ProofState::rewrite`
           (proof/src/tactic.rs:702-737), emitting
           `InferenceRule::Rewrite { from: rhs, to: lhs }` over an `EqualitySymmetry` premise
@evidence  proof/src/tactic.rs:710-717 (the hypothesis must be `ProofExpr::Identity(l, r)`;
           a `ProofExpr::ForAll` wrapping an equality is `DoesNotApply` —
           "rewrite expects an equality hypothesis");
           proof/src/tactic.rs:718 (`replace_in_expr(&g.target, &lhs, &rhs)` — the direction is
           fixed lhs→rhs, and the SCRUTINEE is `g.target`: the goal, never a hypothesis);
           proof/src/tactic.rs:113-128 and :130-156 (`replace_in_term` / `replace_in_expr` —
           "Replace EVERY occurrence", recursing through `Predicate` args, `Identity` sides,
           every connective, and — critically — into `ForAll`/`Exists` BODIES at :148-153
           without consulting the binder);
           proof/src/tactic.rs:719-723 (fails when `lhs` does not occur);
           proof/src/tactic.rs:724-736 (the certified shape: the equality premise proves
           `rhs = lhs`, the open child is the rewritten goal — so `rewrite` NEVER closes the
           goal itself);
           proof/src/tactic_script.rs:313 ("rewrite"|"rw"|"substitute"|"subst" ⟼ "rewrite")
@loss      QUANTIFIED_REWRITE. Lean's `rw [h]` with `h : ∀ x, f x = g x` finds the FIRST match,
           fixes `x` from it, and rewrites only the occurrences of THAT instance (verified: on
           `f 1 = g 1 ∧ f 2 = g 2` the goal becomes `g 1 = g 1 ∧ f 2 = g 2`). LOGOS's `rewrite`
           refuses a quantified hypothesis outright (tactic.rs:710-717). The translator must
           pre-instantiate — which means computing Lean's first match itself, i.e.
           reimplementing `kabstract`'s traversal order. Acceptable only because the translator
           can read the instantiation off the ELABORATED `Expr` (the `Eq.mpr`/motive
           application Lean already built) instead of re-deriving it from the surface syntax:
           **translate the post-elaboration `Expr`, never the surface `Syntax`.**
@loss      DIRECTION. There is no `← h`. `rewrite` is lhs→rhs only (tactic.rs:718). The
           symmetric hypothesis would have to be constructed — and the script surface has no
           term language to construct it with (`exact` takes a name, T08-002), so `rw [← h]`
           has no script image at all unless the ORIGINAL theorem is restated with the equation
           reversed.
@loss      LOCATION. `rw [h] at h'` rewrites in a hypothesis. `ProofState::rewrite` reads and
           writes only `g.target` (tactic.rs:718, :730). No `at` clause exists.
@loss      TRAILING_RFL. Lean's `rw` attempts `rfl` after rewriting and frequently closes the
           goal outright. LOGOS's `rewrite` always leaves the rewritten subgoal open
           (tactic.rs:730-735). A translator that maps `rw [h]` to `rewrite h` and then expects
           zero goals emits the rest of its script against a goal that is still there — and
           because `refine` pushes subgoals to the front (tactic.rs:296-297), the following
           atom runs on it and usually fails with a confusing shape error.
@guard     THE CAPTURE BUG, and the reason this case exists. `replace_in_expr` recurses into
           `ForAll`/`Exists` bodies (tactic.rs:148-153) and `replace_in_term` compares
           `t == from` structurally (tactic.rs:115) — with NO check that `from` is the binder.
           `ProofExpr` binders are NAMED strings (lib.rs:235-238). So rewriting with
           `h : x = 0` inside a goal `∀ x, P x` replaces the BOUND occurrences of `x`, yielding
           `∀ x, P 0`: a different, usually stronger, statement, silently. Lean's `rw` cannot do
           this — `kabstract` abstracts only occurrences valid in the motive and reports
           "motive is not type correct" instead. A translator must prove the rewritten LHS does
           not collide with any binder name in the goal before emitting `rewrite`.
@guard     Catches the "all occurrences" assumption running the WRONG way. On a GROUND
           equation the two languages agree — Lean's `rw [h]` with `h : a = b` on `f a a = f b b`
           rewrites both `a`s and closes by the trailing `rfl` (verified). The divergence is
           entirely in the quantified and occurrence-selected cases, so a translator that tests
           only ground rewrites will see perfect agreement and ship the bug.
@guard     Catches the ALIAS `subst` (tactic_script.rs:313). Lean's `subst h` is a DIFFERENT
           tactic: it requires one side to be a local free variable, substitutes it everywhere
           INCLUDING in all hypotheses, and clears it. LOGOS's `subst` is `rewrite` — goal-only,
           no clearing. A translator that passes Lean's tactic word through gets a strictly
           weaker step that leaves the hypotheses untouched, and every later reference to a
           hypothesis Lean had rewritten now mismatches.
@guard     Catches occurrence selection. `conv ... => rw [h]` and Mathlib's `nth_rewrite` exist
           precisely to rewrite ONE occurrence. LOGOS's rewrite is all-or-nothing, so those have
           no image at any granularity — see T08-023.
@oracle    Every emitted `rewrite` atom names a hypothesis whose LOGOS proposition is a
           `ProofExpr::Identity` (not a `ForAll`), its `lhs` is not equal to any binder name
           occurring in the goal, the rewrite direction matches the Lean source, and the loss
           record names QUANTIFIED_REWRITE / DIRECTION / LOCATION as applicable. A harness
           re-runs the rewritten goal through `types_equal` against the translation of Lean's
           post-`rw` goal and requires them equal.
-/

namespace T08_011

/-- Ground equation, LHS occurring TWICE. Lean rewrites both and the trailing `rfl` closes it;
    LOGOS's `rewrite` rewrites both and leaves `f b b = f b b` open. -/
theorem groundBoth (a b : Nat) (f : Nat → Nat → Nat) (h : a = b) : f a a = f b b := by
  rw [h]

/-- Ground equation where the rewritten goal is NOT closed by `rfl` — so Lean also leaves a
    goal, matching LOGOS. This is the shape that translates cleanly. -/
theorem groundResidual (a b : Nat) (p : Nat → Prop) (h : a = b) (hp : p b) : p a := by
  rw [h]
  exact hp

/-- QUANTIFIED equation. Lean instantiates at the FIRST match (`x := 1`) and leaves `f 2`
    alone: after `rw [h]` the goal is `g 1 = g 1 ∧ f 2 = g 2`. LOGOS refuses the hypothesis. -/
theorem quantified (f g : Nat → Nat) (h : ∀ x, f x = g x) : f 1 = g 1 ∧ f 2 = g 2 := by
  rw [h]
  rw [h]
  exact ⟨rfl, rfl⟩

/-- REVERSE direction. No LOGOS image. -/
theorem reverse (a b : Nat) (p : Nat → Prop) (h : a = b) (hp : p a) : p b := by
  rw [← h]
  exact hp

/-- Rewriting IN A HYPOTHESIS. No LOGOS image. -/
theorem atHypothesis (a b : Nat) (p : Nat → Prop) (h : a = b) (hp : p a) : p b := by
  rw [h] at hp
  exact hp

/-- THE CAPTURE HAZARD: the rewritten LHS shares a name with the goal's binder. Lean's
    `kabstract` will not abstract a bound occurrence; LOGOS's `replace_in_expr` recurses into
    the body and would replace it. Written here as the Lean proof that does NOT rewrite under
    the binder. -/
theorem binderCollision (x : Nat) (p : Nat → Nat → Prop)
    (h : x = 0) (hp : ∀ y : Nat, p 0 y) : ∀ y : Nat, p x y := by
  rw [h]
  exact hp

/-- Lean's `subst`, which the LOGOS alias table maps onto `rewrite`. It clears `a` and rewrites
    every hypothesis — neither of which `rewrite` does. -/
theorem substDiffers (a b : Nat) (p : Nat → Prop) (h : a = b) (hp : p a) : p b := by
  subst h
  exact hp

/-- A rewrite chain: `rw [h1, h2]` is two LOGOS atoms, and the intermediate goal must match. -/
theorem chain (a b c : Nat) (p : Nat → Prop) (h1 : a = b) (h2 : b = c) (hp : p c) : p a := by
  rw [h1, h2]
  exact hp

end T08_011
