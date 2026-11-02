/-!
@case      T08-003
@tier      8 · Tactics
@title     `apply` — no script spelling exists; the Rust `apply` is single-antecedent modus ponens
@lean      apply, apply with several arguments, apply with implicit unification, apply at
@expect    reject
@target    (none — no sound translation)
@evidence  proof/src/tactic_script.rs:308-327 (`canonical` — the COMPLETE alias table. There is
           no arm producing "apply", so the word `apply` in a script reaches
           tactic_script.rs:261-262 and dies with `ScriptError("unknown tactic \`apply\`")`);
           proof/src/tactic_script.rs:275-292 (`parse_primitive`'s dispatch — fifteen arms,
           none of them `apply`; an unmatched canonical name is itself an error at :291);
           proof/src/tactic.rs:984-1147 (module `combinators` — the COMPLETE set of tactic
           VALUES: intro, assumption, simp, decide, omega, crush, exact, split, left, right,
           exists, cases, rewrite, induction, induction_over, auto, seq, first, try_, repeat,
           all_goals, then_all. There is no `combinators::apply`, so `apply` is not even
           composable from Rust the way every other primitive is);
           proof/src/tactic.rs:467-484 (`ProofState::apply`, the only entry point, exists as an
           inherent method — and it is modus ponens on ONE `ProofExpr::Implies(ant, con)`:
           it requires the rule to be a top-level implication, unifies only its CONCLUSION
           against the goal, and emits exactly one subgoal, the antecedent)
@diag      E_TACTIC_NOT_IN_SCRIPT_GRAMMAR — must name `apply`, state that the LOGOS script
           grammar's primitive set is closed (tactic_script.rs:275-292) and that `apply` is not
           in it, and name the Rust-only `ProofState::apply` so the reader knows the rejection
           is about REACHABILITY, not about the absence of the idea. Must NOT emit a script.
@guard     Catches the translator emitting the word `apply` and trusting that "LOGOS has apply
           too". It does — as a Rust inherent method that no script text can invoke and no
           combinator wraps. The script parser's failure is a `ScriptError`, which
           `compile_theorem` surfaces as a proof failure rather than as a translation failure,
           so the bug is reported against the user's theorem instead of against the translator.
@guard     Catches the plausible re-spelling `exact` for `apply`. `apply f` leaves subgoals;
           `exact f` must close the goal. A translator that maps `apply h` to `exact h` produces
           a script that fails whenever `h` is not literally the goal, and — in the one case
           where `h`'s conclusion IS the goal syntactically — closes the goal WITHOUT ever
           discharging the antecedent, because `ProofState::exact` only compares `h.prop` to
           `g.target` (tactic.rs:380). For `hpq : p → q` and goal `q` that comparison fails, so
           this particular slip is caught; but see `applyConclusionIsGoal` below, where the
           rule's conclusion and the goal coincide and the mis-emission is much harder to see.
@guard     Catches "just use `auto`". `ProofState::auto` (tactic.rs:901-914) runs the whole
           backward chainer over every hypothesis; it either closes the goal outright or fails.
           It is not a step-wise `apply`: it produces NO subgoals, so a Lean proof of the shape
           `apply f; <proof of the antecedent>` becomes `auto; <proof of the antecedent>` where
           the second half now runs against whatever goal remained — usually none, making the
           rest of the script a `NoOpenGoals` error, or, if `auto` failed, the first half is a
           hard failure. Neither is a translation.
@guard     Catches the scale error. Even reaching `ProofState::apply` through the Rust API is
           not enough: Lean's `apply f` handles a rule with an arbitrary TELESCOPE of arguments
           (`f : ∀ a b, P a → Q b → R`), instantiates implicit arguments by unification, and
           spawns one goal per unfilled argument. LOGOS's takes a single `Implies(ant, con)`
           and spawns exactly one goal (tactic.rs:469-479). The multi-premise rules below are
           outside that fragment even with full Rust access.
@oracle    Translator exits non-zero with E_TACTIC_NOT_IN_SCRIPT_GRAMMAR for every theorem
           below and emits no `ProofStrategy::Script` text. A harness additionally asserts that
           no emitted script, anywhere in the corpus, contains a word whose `canonical` image
           is `None` — i.e. every emitted atom is one of the fifteen primitives.
-/

namespace T08_003

/-- The canonical `apply`: one implication, one residual subgoal. -/
theorem applyOnePremise (p q : Prop) (hpq : p → q) (hp : p) : q := by
  apply hpq
  exact hp

/-- Two antecedents — two residual subgoals. Outside `ProofState::apply`'s single-antecedent
    shape even from Rust. -/
theorem applyTwoPremises (p q r : Prop) (h : p → q → r) (hp : p) (hq : q) : r := by
  apply h
  · exact hp
  · exact hq

/-- `apply` instantiating an IMPLICIT argument by unification against the goal. LOGOS's
    `unify_exprs` runs on untyped `ProofExpr`, and there are no implicit arguments to solve. -/
theorem applyImplicit (p : Nat → Prop) (h : ∀ {n : Nat}, p n) : p 7 := by
  apply h

/-- `apply` of a global whose conclusion needs higher-order matching. -/
theorem applyGlobal (n m : Nat) (h : n ≤ m) : n ≤ m + 1 := by
  apply Nat.le_succ_of_le
  exact h

/-- The dangerous coincidence: the rule's conclusion IS the goal, so an `apply ⟼ exact`
    mis-emission looks locally plausible. Lean still leaves the antecedent as a goal. -/
theorem applyConclusionIsGoal (p : Prop) (h : p → p) (hp : p) : p := by
  apply h
  exact hp

/-- `apply` used to CHAIN — three steps, three antecedents, one after another. -/
theorem applyChain (p q r : Prop) (h1 : p → q) (h2 : q → r) (hp : p) : r := by
  apply h2
  apply h1
  exact hp

end T08_003
