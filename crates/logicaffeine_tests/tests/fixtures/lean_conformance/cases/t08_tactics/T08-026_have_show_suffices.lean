/-!
@case      T08-026
@tier      8 · Tactics
@title     `have` / `show` / `suffices` in tactic mode — a local `have` must be LIFTED to a
           global scripted theorem, and cannot be lifted at all if it mentions a local
@lean      have h : T := e, have h : T := by tac, show T, suffices h : T by tac, obtain from have
@expect    degrade
@target    For `have`: a separate `ScriptedTheorem { name, premises, goal, script, cites, simp }`
           discharged by `prove_scripted_library`
           (proof/src/tactic_script.rs:385-396, :403-463), whose conclusion becomes a citable
           premise for the theorem that needed it (`cites`, tactic_script.rs:427-432).
           For `show`: NOTHING — the step is dropped, and dropping it is sound only when the
           restatement is structurally identical.
@evidence  proof/src/tactic_script.rs:308-327 (`canonical` — no arm for `have`, `show`,
           `suffices`, `let` in its Lean sense, `set`, or `replace`. The word `let` IS in the
           table but maps to `intro` (:310), which is a different tactic entirely);
           proof/src/tactic_script.rs:275-292 (the fifteen primitives — none introduces a
           hypothesis with an accompanying proof; `intro` introduces one from the GOAL's
           antecedent, `cases` from an existing hypothesis's shape, and that is the complete
           set of ways a name enters `Goal::hyps`);
           proof/src/tactic.rs:183-186 (`direct_hyp` — the only constructor of a `Hyp`, and it
           is private to the module);
           proof/src/tactic_script.rs:385-396 (`ScriptedTheorem` — name, premises, goal, script,
           cites, simp);
           proof/src/tactic_script.rs:424-437 (`prove_scripted_library` — premises are
           `axioms ++ t.premises ++ (conclusions of t.cites) ++ simp_pool`, all CLOSED formulas
           assembled before `ProofState::start`);
           proof/src/tactic.rs:198-214 (`start_with_names` — the hypothesis list is fixed at
           construction; nothing adds to it afterwards except `intro` and `cases`);
           proof/src/tactic.rs:363, :380 (`assumption`/`exact` compare `ProofExpr`s
           structurally, so a `show` that changes the goal's SPELLING changes what matches)
@loss      LOCALITY. A Lean `have` is local: its statement may mention the enclosing theorem's
           bound variables and hypotheses, and it disappears at the end of the block. A LOGOS
           `ScriptedTheorem` is global, closed, and citable by name. Lifting therefore requires
           closing the `have`'s statement over every local it mentions — and the script surface
           has no way to state a quantified premise that re-binds those locals, because
           `premises` are `ProofExpr`s fixed at `ProofState::start` (tactic.rs:198-214) and the
           locals were introduced by `intro` AFTER that. A `have` mentioning an `intro`-bound
           variable cannot be lifted, and the case escalates to `reject`.
@loss      PROOF_TERM_SHAPE. `have h : T := e` supplies a TERM. The script surface has no term
           language (T08-002), so the lifted theorem must be proved by its own script, and the
           translator must find one. `have h : T := by tac` is the tractable case: `tac` becomes
           the lifted theorem's script.
@loss      SHOW_ERASURE. `show T` is a no-op up to definitional equality in Lean. LOGOS has no
           step that changes a goal's spelling without changing the goal, so `show` is dropped.
           That is sound when `T` is structurally the translation of the current goal, and
           WRONG when Lean's `show` performed a defeq change — unfolding a definition, reducing
           an `if`, or eta-expanding — because every later atom then runs against the
           un-unfolded form and `exact`/`assumption` (which compare structurally,
           tactic.rs:363, :380) will not match.
@guard     THE LIFT-AND-LOSE-THE-LOCALS TRAP. The lift looks mechanical: turn each `have` into a
           `ScriptedTheorem` and cite it. It is only sound for a `have` whose statement is
           CLOSED. `prove_scripted_library` assembles each theorem's premises as a flat list of
           `ProofExpr`s before the proof starts (tactic_script.rs:424-437); there is no
           parameterisation, no context capture, and no way for a cited lemma to refer to the
           citing theorem's `intro`-bound variables. A translator that lifts a `have` mentioning
           a local will produce a lemma whose free variables are `ProofTerm::Variable`s that
           mean nothing in the lemma's own context, and the lemma will either fail to prove or
           prove something vacuous — and then be cited as though it said what the `have` said.
@guard     Catches the ALIAS `let` (tactic_script.rs:310). Lean's `let x := e` in tactic mode
           introduces a local DEFINITION; LOGOS's `let` is an `intro` alias. A translator that
           passes Lean's tactic word through gets ∀/→ introduction on the goal, which either
           fails with `DoesNotApply` or — if the goal happens to be an implication — consumes a
           hypothesis the source never introduced, shifting everything after it.
@guard     Catches dropping `show` unconditionally. The drop is safe only under a structural
           check. A translator must compare the translation of `T` with the current LOGOS goal
           and, if they differ, either refuse or record the divergence — it cannot rely on
           LOGOS's definitional equality to bridge the gap, because the script layer never
           calls the kernel's `def_eq` at all (it compares `ProofExpr`s, not kernel `Term`s).
@guard     Catches `suffices`. `suffices h : T by tac` REPLACES the goal with `T` and leaves
           `tac`'s obligation. It is a goal transformation, and the only goal-transforming
           script atoms are `intro`, `split`, `left`/`right`, `exists`, `cases` and `rewrite` —
           none of which can substitute an arbitrary new goal. The lift is the same as for
           `have`, with the same locality restriction, plus the residual proof of
           `T → original` which must ALSO be lifted.
@guard     Catches the `simp` POOL side effect. A lifted theorem marked `simp: true` joins the
           `simp_pool` (tactic_script.rs:392-395, :432-437) and is then in scope for every LATER
           theorem's `simp` in the same library call — a global effect a local `have` never had.
           Lifting must default `simp` to `false`.
@oracle    Every Lean `have` becomes a distinct `ScriptedTheorem` whose `goal` is closed, is
           named in the citing theorem's `cites`, and has `simp: false`; every `show` is dropped
           only after a structural equality check against the current LOGOS goal; and the loss
           record names LOCALITY, PROOF_TERM_SHAPE and SHOW_ERASURE as applicable. For
           `haveMentionsLocal` the translator must REFUSE rather than lift.
-/

namespace T08_026

/-- A CLOSED `have`: its statement mentions no local, so it lifts to a `ScriptedTheorem`. -/
theorem closedHave (p : Prop) (hp : p) : p ∧ (2 + 2 = 4) := by
  have h4 : (2 : Nat) + 2 = 4 := by decide
  exact ⟨hp, h4⟩

/-- A `have` proved by a TERM rather than a tactic — the lifted theorem needs a script the
    translator must invent. -/
theorem haveByTerm (p q : Prop) (hpq : p → q) (hp : p) : q := by
  have hq : q := hpq hp
  exact hq

/-- THE LOCALITY FAILURE: the `have`'s statement mentions `n`, which was introduced by `intro`.
    There is no way to state it as a premise of a separate `ScriptedTheorem`. -/
theorem haveMentionsLocal (p : Nat → Prop) (h : ∀ m, p m) : ∀ n : Nat, p n ∧ p n := by
  intro n
  have hn : p n := h n
  exact ⟨hn, hn⟩

/-- `show` as a pure RESTATEMENT — structurally identical, so dropping it is sound. -/
theorem showIdentical (n : Nat) : n = n := by
  show n = n
  rfl

/-- `show` performing a DEFEQ CHANGE: `Not p` becomes `p → False`. Dropping it leaves the LOGOS
    goal as a `ProofExpr::Not`, on which `intro` does not apply (see T08-001). -/
theorem showDefeq (p : Prop) (h : p → False) : ¬ p := by
  show p → False
  exact h

/-- `show` UNFOLDING a definition. -/
def twice (n : Nat) : Nat := n + n

theorem showUnfolds (n : Nat) : twice n = n + n := by
  show n + n = n + n
  rfl

/-- `suffices`: the goal is replaced, and the residual obligation must be lifted too. -/
theorem sufficesStep (p q : Prop) (hpq : p → q) (hp : p) : q := by
  suffices h : p by exact hpq h
  exact hp

/-- Lean's tactic-mode `let`, which the LOGOS alias table maps onto `intro`. -/
theorem tacticLet (p : Nat → Prop) (h : p 3) : p (1 + 2) := by
  let k := 1 + 2
  show p k
  exact h

end T08_026
