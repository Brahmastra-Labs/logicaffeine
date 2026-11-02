/-!
@case      T08-007
@tier      8 · Tactics
@title     `left` / `right` — the names coincide, the goal shapes they accept do not
@lean      left, right, Or.inl / Or.inr, left on a general two-constructor inductive
@expect    translate
@target    LOGOS script atoms `left` and `right` — canonical `left` / `right`, arity 0, NO
           aliases (proof/src/tactic_script.rs:287-288, :321-322) →
           `combinators::left` / `combinators::right` (proof/src/tactic.rs:1026-1033) →
           `ProofState::left` / `right` → `ProofState::disjunct`
           (proof/src/tactic.rs:413-439), emitting `InferenceRule::DisjunctionIntro`
@evidence  proof/src/tactic_script.rs:321-322 ("left" ⟼ "left", "right" ⟼ "right" — these are
           the only two entries in `canonical` with a single spelling and no English alias);
           proof/src/tactic.rs:422-439 (`disjunct` matches ONLY `ProofExpr::Or(a, b)`, picks
           `a` or `b` by the `take_left` flag, emits ONE subgoal, and returns `DoesNotApply`
           for every other goal shape, :435-437);
           proof/src/tactic.rs:428-432 (both branches emit the SAME rule,
           `InferenceRule::DisjunctionIntro` — the choice of disjunct is recorded only in the
           subgoal's target, not in the rule);
           proof/src/certifier.rs (the `DisjunctionIntro` arm must recover which side was taken
           by matching the child's conclusion against the conclusion's two disjuncts)
@guard     Catches `left`/`right` on a non-`Or` goal. Lean's `left` applies the FIRST
           constructor of any two-constructor inductive: `Sum.inl`, `Bool.false`... and on an
           `Iff` goal `left` is an error in Lean too, but on `Decidable p` it picks `isTrue`.
           LOGOS's is `ProofExpr::Or`-only (tactic.rs:425). Anything else is `DoesNotApply`.
@guard     Catches the rule-level ambiguity. Because both branches emit
           `InferenceRule::DisjunctionIntro` with no side marker (tactic.rs:428-432), a
           derivation for `p ∨ p` is IDENTICAL whichever branch ran. That is harmless for
           `p ∨ p`, but it means a translator cannot round-trip a derivation back to the tactic
           that produced it, and a harness that checks "the translator emitted `left`" must
           compare the SUBGOAL, not the rule.
@guard     Catches the absent aliases. Every other tactic in the table has English synonyms
           (tactic_script.rs:310-326); `left` and `right` have none. A translator rendering
           Lean's `left` as prose — "take the first disjunct", "choose the left" — produces
           words `canonical` does not know, and the script dies with `unknown tactic`. The
           canonical spelling must be emitted verbatim.
@guard     Catches the FILLER-WORD collision. `parse_atom` calls `skip_filler` before every
           atom, and the filler list (tactic_script.rs:298-303) includes `the`, `on`, `a`,
           `of`, `to`, `and`. A translator emitting "right on" or "and left" has those words
           silently discarded — which happens to work — but emitting `left of h` discards `of`
           and then reads `h` as the NEXT ATOM, so the script fails with "unknown tactic `h`"
           rather than with anything about disjunctions.
@guard     Catches the one-subgoal shape. Lean's `left` leaves one goal; so does LOGOS's. But
           a translator that models `∨`-introduction as "split then discard" (there being a
           `split` atom right next to `left` in the table) would emit `split`, which on an
           `Or` goal is `DoesNotApply` — see T08-006.
@oracle    Each `∨`-goal theorem emits exactly one of `left` / `right`, `run_script` leaves
           `open_goals() == 1`, and the residual target is the corresponding disjunct. The
           non-`Or` theorems must not emit `left` or `right`.
-/

namespace T08_007

/-- `left` on a disjunction — translates exactly. -/
theorem leftIntro (p q : Prop) (hp : p) : p ∨ q := by
  left
  exact hp

/-- `right` on a disjunction. -/
theorem rightIntro (p q : Prop) (hq : q) : p ∨ q := by
  right
  exact hq

/-- Symmetric disjunction: the two branches produce the same derivation, so a harness cannot
    tell which one the translator emitted from the rule alone. -/
theorem symmetricOr (p : Prop) (hp : p) : p ∨ p := by
  left
  exact hp

/-- Nested disjunction — two `right`s then a `left`. -/
theorem nestedOr (p q r : Prop) (hq : q) : p ∨ (q ∨ r) := by
  right
  left
  exact hq

/-- The term spelling; the elaborated `Expr` is the same `Or.inl` application. -/
theorem asTerm (p q : Prop) (hp : p) : p ∨ q := Or.inl hp

/-- `left` on a non-`Or` two-constructor inductive: Lean accepts, LOGOS has no image. -/
inductive Choice where
  | first : Nat → Choice
  | second : Bool → Choice

def choiceLeft : Choice := by
  left
  exact 3

/-- `left` on `Sum` — data, not a proposition, so there is no `ProofExpr` for it at all. -/
def sumLeft : Nat ⊕ Bool := by
  left
  exact 5

end T08_007
