/-!
@case      T08-021
@tier      8 · Tactics
@title     `calc` — no chaining form exists on any LOGOS surface, and the intermediate terms
           are unwritable on the two that could host it
@lean      calc with =, calc with mixed relations (Trans), calc as a term, calc in tactic mode
@expect    reject
@target    (none — no sound translation)
@evidence  a grep of `crates/logicaffeine_proof/src` and `crates/logicaffeine_kernel/src` for
           `calc` returns NOTHING outside unrelated identifiers (`polycalc`), so there is no
           `calc` construct, no `Trans` class, and no transitivity-chaining combinator anywhere
           in LOGOS; proof/src/tactic_script.rs:308-327 (`canonical` — no arm for `calc`);
           proof/src/lib.rs:488-728 (`InferenceRule` — `EqualityTransitivity` and `LeTrans`
           exist as individual STEPS, but nothing composes a chain of them from a list of
           intermediate terms); proof/src/formula.rs:427-432 (Surface B's `parse_term` accepts
           ONLY a bare identifier, so `a * (b + c)` — a typical `calc` waypoint — cannot be
           written as a term on that surface at all);
           LOGOS_FACTS §Surface A ("No term arithmetic exists on this surface" — the English
           theorem block's `Given:`/`Prove:` lines go through the natural-language parser);
           kernel/src/interface/literate_parser.rs:1223-1270 (Surface C DOES have infix terms —
           `<= ≤ >= ≥ < > + - *` and the word `equals` — but its `Proof:` grammar is a single
           tactic (`ring`, `refl`, `lia`, `cc`, `simp`, `omega`, `auto`, `induction`), with no
           step-list form); kernel/src/prelude.rs:202-340 (the only order facts in the trusted
           base are the eight `Int` axioms; there is no generic `Trans` instance mechanism to
           mix `=` and `<` in one chain)
@diag      E_NO_CALC_CHAIN — must name the theorem, report the number of `calc` steps and the
           relations used, and state that no LOGOS surface has a chaining form. If the chain is
           pure `=` it must additionally say that the expansion into `rewrite` steps was
           considered and why it was refused (see the guards). Must NOT emit a script.
@guard     THE `rewrite`-EXPANSION TRAP. A pure-`=` `calc` chain `a = b := h₁; _ = c := h₂` looks
           like `rewrite h1 . rewrite h2`, and for a goal `a = c` that expansion sometimes
           works. It is not sound in general for two reasons, both from T08-011: LOGOS's
           `rewrite` replaces EVERY occurrence of the LHS (tactic.rs:130-156), so a waypoint
           that occurs elsewhere in the goal is rewritten too; and `rewrite` requires a GROUND
           `ProofExpr::Identity` hypothesis (tactic.rs:710-717), so a `calc` step justified by a
           quantified lemma has no image. A translator that performs the expansion has silently
           chosen a different proof, and for a chain whose waypoints repeat, a different
           theorem.
@guard     Catches MIXED RELATIONS. `calc a = b := h₁; _ < c := h₂; _ ≤ d := h₃` is ordinary
           Lean — the `Trans` typeclass composes the relations and computes the chain's overall
           relation. LOGOS has no `Trans`, no relation composition, and the eight order axioms
           (prelude.rs:202-340) are individual `Int` facts stated over
           `a ≤ b ≔ Eq Bool (le a b) true`. There is nothing to compose them with, so a mixed
           chain cannot even be expanded step-by-step: the translator would have to synthesise
           the `lt_succ_le`/`le_trans` plumbing itself, i.e. write the proof rather than
           translate it.
@guard     Catches the TERM/TACTIC split. `calc` is an EXPRESSION in Lean 4, usable outside
           `by`. The translator therefore meets it while translating a term, not a tactic block,
           and must reject at that point — before the script surface is even chosen. A
           translator that only inspects tactic blocks will pass a `calc`-proved theorem through
           as an opaque term and then fail to translate its `Expr`, producing a diagnostic about
           `Trans.trans` rather than about `calc`.
@guard     Catches "put the waypoints in a Surface-C literate theorem". Surface C is the only
           LOGOS surface with infix arithmetic (literate_parser.rs:1223-1270), so it is the only
           place the intermediate terms can be spelled — but its `Proof:` is one tactic, not a
           list of justified steps, and there is no `forall` in its `Statement:` grammar
           (LOGOS_FACTS §Surface C). Each `calc` step would become its own literate theorem, and
           the composition of those theorems back into the original statement has no surface at
           all. The three surfaces do not interoperate.
@guard     Catches the degenerate one-step `calc`. `calc a = b := h` is just `exact h` and DOES
           translate. The diagnostic must be driven by a chain of length ≥ 2, or by the presence
           of a relation other than `=`, so that a translator is not over-rejecting.
@oracle    Translator exits non-zero with E_NO_CALC_CHAIN for `chainEq`, `chainMixed`,
           `chainAsTerm` and `chainWithQuantifiedStep`, and emits no script for any of them. It
           must NOT reject `oneStep`. A harness additionally asserts that no emitted script
           anywhere in the corpus contains a `rewrite` atom that the translator synthesised from
           a `calc` step.
-/

namespace T08_021

/-- A pure-`=` chain of three steps. The `rewrite`-expansion is tempting and wrong. -/
theorem chainEq (a b c d : Nat) (h1 : a = b) (h2 : b = c) (h3 : c = d) : a = d := by
  calc a = b := h1
    _ = c := h2
    _ = d := h3

/-- MIXED RELATIONS: `=` then `<` then `≤`, composed by `Trans`. Nothing in LOGOS composes
    relations. -/
theorem chainMixed (a b c d : Nat) (h1 : a = b) (h2 : b < c) (h3 : c ≤ d) : a < d := by
  calc a = b := h1
    _ < c := h2
    _ ≤ d := h3

/-- `calc` as a TERM, outside any `by` block — the translator meets it during term translation. -/
theorem chainAsTerm (a b c : Nat) (h1 : a = b) (h2 : b = c) : a = c :=
  calc a = b := h1
    _ = c := h2

/-- A step justified by a QUANTIFIED lemma, so the `rewrite` expansion cannot even start:
    `ProofState::rewrite` refuses a `ForAll` hypothesis. -/
theorem chainWithQuantifiedStep (f g : Nat → Nat) (a b : Nat)
    (h1 : a = b) (h2 : ∀ x, f x = g x) : f a = g b := by
  calc f a = f b := by rw [h1]
    _ = g b := h2 b

/-- A waypoint that OCCURS ELSEWHERE in the goal, so an all-occurrences `rewrite` would change
    more than the step intended. -/
theorem chainRepeatedWaypoint (a b : Nat) (p : Nat → Nat → Prop)
    (h1 : a = b) (h2 : p b b) : p a a := by
  rw [h1]
  exact h2

/-- The DEGENERATE one-step chain: this is `exact h` and must NOT be rejected. -/
theorem oneStep (a b : Nat) (h : a = b) : a = b := by
  calc a = b := h

end T08_021
