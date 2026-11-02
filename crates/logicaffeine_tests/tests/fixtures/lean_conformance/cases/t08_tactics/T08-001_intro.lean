/-!
@case      T08-001
@tier      8 · Tactics
@title     `intro` / `intros` — the one script tactic whose arity and goal shapes nearly match
@lean      intro, intro (several binders), intros, intro on ¬p, rintro-free core usage
@expect    translate
@target    LOGOS script atom `intro <name>` — canonical name `intro`, arity 1, aliases
           `assume`, `suppose`, `let`, `introduce`, `given`, `fix`
           (proof/src/tactic_script.rs:276, :310) → `combinators::intro`
           (proof/src/tactic.rs:988-991) → `ProofState::intro` (proof/src/tactic.rs:313-356)
@evidence  proof/src/tactic_script.rs:310 (`canonical`: "intro"|"assume"|"suppose"|"let"|
           "introduce"|"given"|"fix" ⟼ "intro"); proof/src/tactic_script.rs:264-274 (`arg()` —
           an atom's argument is exactly ONE `Tok::Word`; a missing one is the hard error
           "tactic `intro` expects an argument"); proof/src/tactic.rs:313-356 (`ProofState::intro`
           handles EXACTLY two goal shapes, `ProofExpr::Implies` and `ProofExpr::ForAll`, and
           returns `TacticError::DoesNotApply` for every other, tactic.rs:352-354);
           proof/src/tactic.rs:329-340 (the ∀ case *renames* the bound variable to the supplied
           name by capture-unaware `apply_subst_to_expr`, with a special case when the name
           already IS the binder); proof/src/lib.rs:199-238 (`ProofExpr` — `Not` is its OWN
           constructor, NOT sugar for `Implies(p, False)`)
@guard     Catches mapping Lean's `intro h` on a `¬ p` goal to LOGOS `intro h`. In Lean `Not p`
           unfolds to `p → False` and `intro` succeeds. In LOGOS `ProofExpr::Not` is a distinct
           constructor (lib.rs:230) and `ProofState::intro` matches only `Implies`/`ForAll`
           (tactic.rs:315-355), so the script step fails with `DoesNotApply` — at the FIRST
           step of a proof whose remaining steps were all emitted on the assumption that it
           succeeded. The translator must either re-express the goal as `Implies(p, ⊥)` before
           emitting, or refuse.
@guard     Catches emitting Lean's zero-argument `intro` (which names the hypothesis
           inaccessibly, `h✝`) as a bare LOGOS `intro`. `arg()` (tactic_script.rs:264-274) peeks
           for a `Tok::Word` and errors if the next token is a separator — so bare `intro`
           followed by `.` is a PARSE error, and the whole script is rejected before any step
           runs. The translator must synthesise a name. It must further ensure the synthesised
           name cannot collide: `ProofState::intro` on a ∀ goal substitutes the new name into the
           body (tactic.rs:332-337), so choosing a name that already occurs free in the body
           captures it silently.
@guard     Catches emitting Lean's multi-binder `intro a b c` verbatim. The script grammar's
           atom is `NAME ARG?` — exactly one argument (tactic_script.rs:16, :275-292) — so
           `intro a b c` parses as `intro a`, and then `b` is read as the next ATOM, i.e. a
           tactic name; `canonical("b")` is `None` and the script dies with "unknown tactic `b`".
           Lean's `intro a b c` must become `intro a . intro b . intro c`.
@guard     Catches the alias trap in the other direction. `let` is an `intro` alias
           (tactic_script.rs:310). A translator that emits a LOGOS `let`-binding step spelled
           `let x` while meaning a local definition gets ∀/→ introduction instead, silently.
           Likewise `fix` and `given` — `given` is also the surface keyword for a PREMISE
           (`Given:`), so the two readings must not be conflated.
@oracle    For each theorem below the emitted script is a `;`-separated sequence of
           `intro <name>` atoms whose count equals the number of Lean binders introduced, every
           `<name>` is a single `[A-Za-z0-9_]+` word, and `ProofState::run_script` on the
           translated goal leaves `open_goals() == 1` with the expected residual target. The
           `¬`-goal theorem must NOT be emitted as `intro`.
-/

namespace T08_001

/-- Implication introduction — `ProofExpr::Implies`, tactic.rs:316-322. -/
theorem impIntro (p q : Prop) (hq : q) : p → q := by
  intro _hp
  exact hq

/-- Universal introduction — `ProofExpr::ForAll`, tactic.rs:323-351. -/
theorem forallIntro (p : Nat → Prop) (h : ∀ n, p n) : ∀ n, p n := by
  intro n
  exact h n

/-- Several binders in ONE Lean `intro`. Needs three separate LOGOS atoms. -/
theorem multiIntro (p q r : Prop) (hr : r) : p → q → r := by
  intro _hp _hq
  exact hr

/-- Mixed ∀ and → in one `intro`. -/
theorem mixedIntro (p : Nat → Prop) (h : ∀ n, p n) : ∀ n, p 0 → p n := by
  intro n _h0
  exact h n

/-- THE BOUNDARY CASE. Lean's `intro` unfolds `Not`; LOGOS's does not. -/
theorem introOnNot (p : Prop) (h : p → False) : ¬ p := by
  intro hp
  exact h hp

/-- Lean's zero-argument `intro`: the hypothesis is inaccessible (`a✝`). A LOGOS script
    must invent a name, and the name must not capture anything free in the body. -/
theorem anonymousIntro (p : Prop) : p → p := by
  intro
  assumption

/-- The renaming hazard: the LOGOS `∀` case substitutes the supplied name into the body.
    Here the body already mentions a free `m`, so emitting `intro m` would capture it. -/
theorem captureHazard (r : Nat → Nat → Prop) (m : Nat) (h : ∀ k, r k m) : ∀ n, r n m := by
  intro n
  exact h n

end T08_001
