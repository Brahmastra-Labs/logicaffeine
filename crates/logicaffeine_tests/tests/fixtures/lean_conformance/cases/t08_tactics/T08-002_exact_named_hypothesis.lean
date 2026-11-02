/-!
@case      T08-002
@tier      8 · Tactics
@title     `exact` takes a NAMED HYPOTHESIS in LOGOS, never a term — `exact (f h)` has no image
@lean      exact, exact with an applied term, exact up to definitional unfolding
@expect    translate
@target    LOGOS script atom `exact <name>` — canonical `exact`, aliases `from`, `because`
           (proof/src/tactic_script.rs:277, :311) → `combinators::exact`
           (proof/src/tactic.rs:1017-1021) → `ProofState::exact` (proof/src/tactic.rs:377-391)
@evidence  proof/src/tactic_script.rs:277 (`"exact" => Ok(c::exact(&arg()?))` — the argument is a
           `String`, lexed as one `Tok::Word`, and is used as a HYPOTHESIS NAME);
           proof/src/tactic_script.rs:89-100 (the script lexer produces only `Word`/`Semi`/
           parens/brackets/`|`/`<;>` — there is no application, no parenthesised term, no
           operator, so a term argument cannot even be TOKENISED);
           proof/src/tactic.rs:377-391 (`exact` looks the name up in `g.hyps` by `h.name == name`
           and then demands `h.prop == g.target` — STRUCTURAL equality of `ProofExpr`, with no
           normalisation, no unfolding and no unification);
           proof/src/tactic.rs:183-186 (`direct_hyp` — the only way a name enters `hyps` is a
           premise, an `intro`, or a `cases` branch)
@guard     THE CENTRAL TIER-8 TRAP FOR THIS TACTIC. Lean's `exact e` takes an arbitrary TERM.
           `exact (h₁ h₂)`, `exact ⟨a, b⟩`, `exact Nat.le_refl n`, `exact fun x => f x` are all
           ordinary Lean. NONE of them has a LOGOS script image: `arg()` reads exactly one word
           (tactic_script.rs:266-271) and anything else is a parse error, or — far worse —
           `exact f h` parses as `exact f` followed by the ATOM `h`, so the translator's term
           silently becomes "close the goal with hypothesis `f`, then run the tactic `h`",
           which fails with "unknown tactic `h`" AFTER `exact f` may already have committed.
@guard     Catches assuming `exact` is up to definitional equality. Lean's `exact` elaborates
           the term against the goal with full defeq (delta, iota, beta, eta, proof
           irrelevance). LOGOS's is `h.prop == g.target` (tactic.rs:380) — derived `PartialEq`
           on `ProofExpr`. `hp : p ∧ q` does not close the goal `q ∧ p`; `hp : ¬¬p` does not
           close `p`; a hypothesis stated with the arguments in a different but equivalent
           spelling does not close anything. A translator that emits `exact h` wherever Lean
           wrote `exact h` will produce scripts that fail on goals Lean accepted, and the
           failure is reported as `DoesNotApply`, i.e. as if the hypothesis were the wrong one.
@guard     Catches confusing `exact` with `assumption`. LOGOS's `exact` fails with
           `NoSuchHypothesis` when the NAME is absent and `DoesNotApply` when the name is present
           but the proposition differs (tactic.rs:385-390); `assumption` (tactic.rs:361-373)
           searches all hypotheses. Since LOGOS hypothesis names are MACHINE-CHOSEN in several
           places (`hp0`, `hp1`, … for premises, tactic.rs:211; `h_1`/`h_2`/`h_l`/`h_r`/`h_w`
           after `cases`, tactic.rs:516-541), a translator that carries Lean's user-written
           names into `exact` will name hypotheses that do not exist. `assumption` is the
           robust emission; `exact` is only safe for a name the translator itself created.
@guard     Catches the alias `from`/`because` (tactic_script.rs:311) being emitted as prose.
           `from h` means `exact h`, and `from` is also a Lean keyword in `show T from e` —
           a translator that passes `show … from …` through as script text produces a LOGOS
           `exact` applied to whatever word follows `from`.
@oracle    Every emitted `exact` atom's argument is a single word that names a hypothesis the
           translator itself introduced (a premise name, an `intro` name, or a `cases`-generated
           `h_1`/`h_2`/`h_l`/`h_r`/`h_w`), and `ProofState::exact` closes the goal. Any Lean
           `exact` whose argument is not a bare free variable of the local context must NOT be
           emitted as `exact`; the harness asserts the translator either restructured the proof
           or emitted a diagnostic.
-/

namespace T08_002

/-- The only shape that translates: the argument is a bare local hypothesis. -/
theorem exactBareHyp (p : Prop) (hp : p) : p := by exact hp

/-- Lean's `exact` applied to a TERM. No LOGOS script image: the lexer cannot tokenise it. -/
theorem exactApplied (p q : Prop) (hpq : p → q) (hp : p) : q := by exact hpq hp

/-- Anonymous-constructor term argument. -/
theorem exactAnonCtor (p q : Prop) (hp : p) (hq : q) : p ∧ q := by exact ⟨hp, hq⟩

/-- A global constant as the argument — not a hypothesis at all. -/
theorem exactGlobal (n : Nat) : n ≤ n := by exact Nat.le_refl n

/-- A lambda as the argument. -/
theorem exactLambda (p : Nat → Prop) (h : ∀ n, p n) : ∀ n, p n := by exact fun n => h n

/-- `exact` succeeding only up to DEFEQ: `Not p` and `p → False` are the same type in Lean,
    different `ProofExpr` constructors in LOGOS. -/
theorem exactDefeq (p : Prop) (h : p → False) : ¬ p := by exact h

/-- `exact` up to iota: the goal reduces to the hypothesis's type. -/
theorem exactIota (p : Prop) (hp : p) : (if True then p else p) := by exact hp

/-- The robust emission is `assumption`, which does not depend on the name. -/
theorem viaAssumption (p : Prop) (hp : p) : p := by assumption

end T08_002
