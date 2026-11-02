/-!
@case      T09-014
@tier      9 · Metaprogramming and syntax
@title     Custom BINDER notation — `extBinder`, `binder_predicate` and `∀ x > 0, …` expand to
           binder structure no LOGOS surface can introduce
@lean      notation3-style binders, extBinder, binder_predicate, ∀ x > 0 / ∃ x ∈ s, bounded
           quantifiers, Σ/Π-style user binders
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/term.rs:463, :467 (`Pi { param, param_type, body_type }` and
           `Lambda { param, param_type, body }` — the ONLY binders in the kernel term language.
           Each binds exactly one NAMED variable with one type, and there is no binder
           annotation, no telescope form, and no predicate slot);
           kernel/src/context.rs:201 (`set_binder_kinds` — the only per-binder metadata LOGOS
           records, and it stores `ParamKind::{Explicit, Implicit, Instance}`
           (elaborate.rs:1016-1027): three values, none of which encodes a bound);
           kernel/src/prelude.rs (`Ex`/`witness` — the existential is a `Prop`-valued inductive
           over a predicate; there is no bounded-existential form and no `∃ x ∈ s`);
           proof/src/tactic_script.rs:276, :310 (the script's ONLY binder-introducing atom is
           `intro <name>` — one name, no type, no bound. `ProofState::intro`
           (proof/src/tactic.rs:313-356) handles `Implies` and `ForAll` and nothing else);
           proof/src/tactic_script.rs:280 (`exists` takes one bare word wrapped in
           `ProofTerm::Constant` — so the existential side has no bound either, see T08-017);
           proof/src/formula.rs (Surface B's formula grammar — `for all`/`∀` and
           `there exists`/`∃` over a BARE IDENTIFIER, with `parse_term` accepting only an
           identifier at :427-432: a bounded binder cannot be written);
           kernel/src/interface/literate_parser.rs:1223-1270 and LOGOS_FACTS §Surface C (the
           literate `Statement:` grammar has NO `forall` at all — free variables are
           universally closed implicitly, so there is nowhere to attach a bound)
@note      `binder_predicate` IS a core Lean 4.14 command and is exercised live below. Mathlib's
           `notation3`, the other standard way to declare a custom binder, is NOT available in
           bare Lean 4.14 ("unexpected identifier; expected 'binder_predicate', …"); it is named
           in a comment and its desugared output is written out, exactly as T08-015 and T08-016
           do for Mathlib tactics. The rejection covers both spellings.
@diag      E_NO_CUSTOM_BINDER — must name the binder notation, report the binder variable and
           the predicate or domain it carries, and state that LOGOS's only binders are
           `Term::Pi` and `Term::Lambda` over one named variable and one type, with the script
           surface offering only nullary `intro`. Must emit nothing for the notation.
@guard     THE "IT DESUGARS, SO IT TRANSLATES" TRAP. `∀ x > 0, p x` desugars to
           `∀ x, x > 0 → p x`, which is an ordinary `Pi` over an ordinary implication and DOES
           translate as a TERM. So a translator reading the elaborated `Expr` will handle the
           quantified statement fine — and that is correct, and is why this case is about the
           NOTATION rather than the proposition. What is rejected is the `binder_predicate` /
           `extBinder` declaration itself, which extends Lean's binder parser: it produces no
           `Expr`, it has no LOGOS counterpart, and a translator that emits anything for it puts
           a phantom name into the flat global namespace (T02-020).
@guard     Catches the PROOF-SIDE consequence, which is the part that actually breaks. A term
           may desugar, but a SCRIPT that proves it cannot. `∀ x > 0, p x` desugars to a `Pi`
           whose body is an implication, so proving it needs `intro x . intro hx` — TWO atoms
           where the Lean source wrote one binder — and the translator must synthesise the
           second name, ensure it does not collide with LOGOS's machine-generated namespace
           (`h_1`, `h_2`, `h_l`, `h_r`, `h_w`, `W{n}`, `K{n}`, `ih`, `hp{i}`), and rewrite every
           downstream reference. On the existential side it is worse: `∃ x > 0, p x` desugars to
           `∃ x, x > 0 ∧ p x`, so `exists w` must be followed by a `split`, and LOGOS's `exists`
           takes a bare `Constant` witness that a bound may not even be stateable about
           (T08-017).
@guard     Catches `∃ x ∈ s, …`. It desugars to `∃ x, x ∈ s ∧ …`, and LOGOS has no `Membership`
           class, no `Set`, and no `List` in its prelude (T10-012, T10-013). So the desugared
           form mentions a global that does not exist, and the failure surfaces as an unbound
           name rather than as "this binder notation is unsupported".
@guard     Catches BINDER TELESCOPES introduced by one notation. `notation3`-style binders can
           bind several variables with one token; `Term::Pi` binds exactly one
           (term.rs:463). The expansion is a nested telescope, which is fine as a term — and
           which, per T00-021 and T11-007, makes LOGOS's named, capture-avoiding substitution
           quadratic in the telescope depth. A translator that generates deep telescopes from
           compact binder notations should record that cost.
@guard     Catches the Surface-B and Surface-C dead ends specifically. Surface B's `∀`/`∃`
           range over a bare identifier with no domain (formula.rs:427-432), and Surface C has
           no `forall` in its `Statement:` grammar at all. So a bounded quantifier has no
           spelling on EITHER of the two surfaces that can state a quantified proposition, and
           the only route is the kernel term language via the Rust API — where the desugared
           form works and the notation still does not exist.
@guard     Catches over-rejecting the desugared THEOREM. `boundedForallDesugared` and
           `boundedExistsDesugared` below are ordinary Lean with no custom notation, and must
           translate. The diagnostic must fire on the notation declaration, not on every
           theorem whose statement happens to contain a bounded quantifier.
@oracle    Translator exits non-zero with E_NO_CUSTOM_BINDER for the `binder_predicate` and
           custom-binder `notation` declarations below, emits nothing for them, and DOES emit
           correct LOGOS declarations for every theorem — including the ones written with the
           custom binders, whose elaborated `Expr`s are ordinary. For each such theorem the
           emitted script introduces one `intro` atom per DESUGARED binder, and the harness
           asserts none of the synthesised names lies in LOGOS's machine-generated namespace.
-/

namespace T09_014

def pos (n : Nat) : Prop := 0 < n

-- A `binder_predicate`: it extends Lean's BINDER parser, so `∀ x > 0, …` and `∃ x > 0, …`
-- become available. It declares no constant and produces no `Expr`.
binder_predicate x " ≫ " y:term => `($x > $y)

/-- Using the custom binder. The elaborated `Expr` is an ordinary `Pi` whose body is an
    implication — it translates; the `binder_predicate` above does not. -/
theorem boundedForallCustom : ∀ x ≫ 0, pos x := by
  intro x hx
  exact hx

/-- The hand-desugared form of the same statement. Must translate, and must produce the same
    LOGOS term as the line above. -/
theorem boundedForallDesugared : ∀ x : Nat, x > 0 → pos x := by
  intro x hx
  exact hx

/-- The EXISTENTIAL side desugars to a conjunction, so the proof needs an extra `split` that
    the Lean source never wrote. -/
theorem boundedExistsCustom : ∃ x ≫ 0, pos x :=
  ⟨1, Nat.zero_lt_one, Nat.zero_lt_one⟩

/-- The hand-desugared existential. -/
theorem boundedExistsDesugared : ∃ x : Nat, x > 0 ∧ pos x :=
  ⟨1, Nat.zero_lt_one, Nat.zero_lt_one⟩

-- A user-defined domain predicate, over which Mathlib's `notation3 "∀ᵣ " (...) ", " …` would
-- declare a custom binder. `notation3` is a MATHLIB command and is unavailable here; the
-- predicate is declared so the desugared statements below are well-typed, and the binder
-- notation that would introduce them is named in the comment only.
def inRange (lo hi n : Nat) : Prop := lo ≤ n ∧ n ≤ hi

/-- The DESUGARED form of what such a binder would produce: an ordinary `Pi` over an ordinary
    implication. It translates; the `notation3` command would not. -/
theorem rangeBoundedDesugared : ∀ n : Nat, inRange 0 5 n → n ≤ 5 := by
  intro n hn
  exact hn.2

/-- A TELESCOPE introduced by one binder token: three variables, three nested `Pi`s in the
    `Expr`, and quadratic named substitution on the LOGOS side (T00-021, T11-007). -/
theorem telescope : ∀ a b c : Nat, a + b + c = c + b + a := by
  intro a b c
  omega

/-- A bounded quantifier over a MEMBERSHIP, whose desugaring mentions a class LOGOS has no
    counterpart for. -/
def MyMem (n : Nat) (l : List Nat) : Prop := n ∈ l

theorem boundedMembership (l : List Nat) : ∀ x ∈ l, MyMem x l := by
  intro x hx
  exact hx

end T09_014
