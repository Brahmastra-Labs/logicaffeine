/-!
@case      T12-009
@tier      12 · Vernacular surfaces
@title     Surface C has no `forall` — a `Statement:` cannot bind a quantified variable
@lean      `∀`-quantified statement, and the Lean distinction between a universally
           quantified theorem and one with section/auto-bound variables
@expect    reject
@target    (none — the quantifier cannot be written; see @note for what a translator may
           do instead and why it is not this case's target)
@evidence  kernel/src/interface/literate_parser.rs:519-560 — `parse_literate_theorem`
           reads `## Theorem:`, a name, `Statement`, `:`, then calls `self.parse_term()`
           at :555. That is the ENTIRE statement grammar.
@evidence  kernel/src/interface/literate_parser.rs:1217-1252 — `parse_term` handles
           exactly two infix words, `equals` and `implies`, over
           `parse_comparison`. Nothing in the chain
           `parse_term → parse_comparison → parse_additive → parse_multiplicative → …`
           binds a variable.
@evidence  kernel/src/interface/literate_parser.rs — the strings `forall`, `for all`,
           `∀` and `Forall` do not occur anywhere in the file. The sole near-hit is the
           comment at :2351 ("Should be forall _ : A, B (non-dependent Pi)"), describing
           the arrow produced by `implies`, which binds nothing.
@evidence  kernel/src/interface/literate_parser.rs:1239-1250 — `implies` builds
           `Term::Pi { param: "_", param_type: lhs, body_type: rhs }`. The binder name is
           hard-coded `"_"`, so even the one Pi-producing construct on this surface
           cannot be given a name for the body to mention.
@evidence  tests/phase_literate_induction.rs:1-7 — the test file's own module doc:
           "Note: These tests focus on the `induction` keyword and bullet syntax. The
           forall quantifier syntax will be addressed separately." It has not been.
@diag      E_SURFACE_C_NO_QUANTIFIER — must state that the `Statement:` grammar has no
           binder form, name the Lean binder(s) that cannot be rendered, and distinguish
           the two failure modes below. Must emit no literate block.
@guard     THE trap: free variables in a `Statement:` are NOT unbound — they are
           effectively universally closed by the tactic that consumes them, and that
           closure is invisible. `Statement: x <= x.` (phase_literate_lia.rs:136-145)
           proves for all `x`, and a translator can therefore be tempted to render
           `∀ x : Nat, x ≤ x` by simply DROPPING the binder. For that one theorem the
           result is right. It is wrong the instant the quantifier is not outermost and
           not universal:
           · `∃ x, P x` — dropping the binder turns an existential into a universal, an
             unsound strengthening in the direction that still typechecks.
           · `(∀ x, P x) → Q` — dropping the binder makes the ANTECEDENT's quantifier
             disappear, and `implies` puts its LHS under a `Pi` binder named `"_"`, so
             the free `x` in the antecedent and the (absent) `x` in the consequent are
             now the same free variable. The rendered proposition is `P x → Q`, which is
             weaker than the source and provable from strictly less.
           · Alternating quantifiers (`∀ x, ∃ y, R x y`) collapse entirely.
           A translator must therefore NOT treat "free variable = ∀" as a general rule.
           It is admissible only for a statement whose Lean form is a prenex `∀` block
           with no other quantifier anywhere, and even then it must be recorded as a
           loss, not a translation.
@guard     Catches assuming the quantifier can be smuggled in as a lambda. The literate
           surface does have binder syntax — `given x: Nat yields …` and
           `|x: Nat| -> …` (literate_parser.rs:29-33) — but those are TERM-level lambdas
           handled by the DEFINITION parser, not reachable from `Statement:`, whose only
           entry point is `parse_term` at :555. A `Statement:` beginning `given x: Nat`
           parses `given` as an identifier and then fails on the `:`.
@guard     Catches assuming `implies` gives a usable Pi. It does — `Pi { param: "_", … }`
           — but with the binder name hard-coded to `"_"`, so the body can never refer to
           the hypothesis. Every Lean implication whose conclusion depends on the
           hypothesis (a dependent function type, i.e. most `∀ h : P, Q h`) is out of
           reach even though the arrow is available.
@guard     Catches thinking the OTHER surfaces cover this. They do not interoperate:
           Surface B has `for all` (T12-006) but no function terms (T12-007), and
           Surface C has function terms (T12-008) but no quantifier. A Lean theorem that
           needs BOTH — which is almost every interesting arithmetic theorem, e.g.
           `∀ n : Nat, n + 0 = n` — is expressible on no vernacular surface at all. That
           is the observation T12-011 turns into a policy.
@oracle    Translator exits non-zero with E_SURFACE_C_NO_QUANTIFIER and emits no
           `## Theorem: … Statement:` text for any of these. If the translator instead
           emits the binder-dropped form, it must (a) record
           QUANTIFIER_ERASURE and (b) refuse for every statement here except
           `leReflQuantified`, whose Lean form is a bare prenex `∀`.
-/

namespace T12_009

/-- The safe-looking case: a bare prenex `∀` whose binder-dropped rendering
    (`Statement: x <= x.`) happens to mean the same thing. This is the ONLY shape for
    which the erasure is admissible, and even here it is a loss, not a translation. -/
theorem leReflQuantified : ∀ x : Nat, x ≤ x :=
  fun x => Nat.le_refl x

/-- An existential. Dropping the binder converts `∃` to an implicit `∀` — a strengthening
    the LOGOS kernel cannot detect, because the rendered statement is simply a different
    (and, here, false) proposition about a free variable. -/
theorem existsSucc : ∃ x : Nat, x > 0 :=
  ⟨1, Nat.zero_lt_one⟩

/-- A quantifier in the ANTECEDENT. `implies` binds its LHS under `param: "_"`, so the
    antecedent's `x` and any `x` elsewhere become one free variable. -/
theorem allImpliesOne (P : Nat → Prop) (h : ∀ x, P x) : P 0 :=
  h 0

/-- Alternating quantifiers. There is no rendering of either binder, and no rendering of
    their order, which is the entire content of the statement. -/
theorem alternating : ∀ x : Nat, ∃ y : Nat, y > x :=
  fun x => ⟨x + 1, Nat.lt_succ_self x⟩

/-- A dependent implication: the conclusion mentions the hypothesis. `Pi { param: "_" }`
    cannot express this even in principle. -/
theorem dependentArrow (P : Nat → Prop) (Q : ∀ n, P n → Prop) :
    ∀ (n : Nat) (h : P n), Q n h → Q n h :=
  fun _ _ hq => hq

end T12_009
