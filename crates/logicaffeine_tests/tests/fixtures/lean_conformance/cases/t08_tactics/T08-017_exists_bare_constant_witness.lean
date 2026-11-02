/-!
@case      T08-017
@tier      8 · Tactics
@title     `use (n+1)` has no image — the LOGOS witness is always a bare `ProofTerm::Constant`,
           even when the Lean witness was a local VARIABLE
@lean      exists, use, use with an applied term, refine ⟨w, ?_⟩, Exists.intro
@expect    degrade
@target    LOGOS script atom `exists <word>` — canonical `exists`, arity 1, aliases `use`,
           `witness`, `choose`, `provide` (proof/src/tactic_script.rs:280, :314) →
           `combinators::exists` (proof/src/tactic.rs:1034-1037) → `ProofState::exists`
           (proof/src/tactic.rs:443-463), emitting
           `InferenceRule::ExistentialIntro { witness, witness_type: "Entity" }`
@evidence  proof/src/tactic_script.rs:280 (`"exists" => Ok(c::exists(ProofTerm::Constant(arg()?)))`
           — the argument word is wrapped in `ProofTerm::Constant` UNCONDITIONALLY, with no
           lookup against the local context and no alternative term shape);
           proof/src/tactic_script.rs:264-274 (`arg()` accepts exactly one `Tok::Word`; the
           lexer (:89-100) produces a `Word` only for a run of `is_alphanumeric() || '_'`, so a
           parenthesis, an operator, or a second word is not an argument);
           proof/src/tactic.rs:443-463 (`exists` substitutes the witness into the `∃` body and
           leaves the instantiated goal OPEN — it never attempts `rfl`);
           proof/src/tactic.rs:333 (`ProofState::intro` binds an introduced ∀-variable as
           `ProofTerm::Variable(name)` — a DIFFERENT constructor from `Constant`);
           proof/src/lib.rs (`ProofTerm::Constant` vs `ProofTerm::Variable` are distinct
           variants compared by derived `PartialEq`, which is what `assumption`
           (tactic.rs:363) and `exact` (tactic.rs:380) use);
           proof/src/formula.rs:437-445 (`ident_to_term` — the FOL surface's own convention:
           uppercase- or digit-leading ⟹ `Constant`, lowercase-leading ⟹ `Variable`. The script
           `exists` atom does NOT follow it)
@loss      WITNESS_TERM_SHAPE. Lean's witness is an arbitrary term: `use (n + 1)`,
           `use (f x)`, `use ⟨a, b⟩`, `use 0`. LOGOS's is one identifier. The translator must
           either (a) name the witness as a fresh global definition first — which the script
           surface cannot do, so it means emitting a kernel `add_definition` alongside the
           script — or (b) refuse. Acceptable only with (a) done explicitly and recorded.
@loss      TRAILING_DISCHARGE. Lean's `use` closes the residual goal with `rfl`/`trivial` when
           it can, so `use 0` often finishes the proof. LOGOS's `exists` always leaves the
           instantiated goal open (tactic.rs:450-458). Every translated `use` needs an explicit
           follow-up atom that the Lean source did not contain.
@loss      MULTIPLE_WITNESSES. `use a, b` supplies witnesses for nested `∃`s in one step. The
           script's `,` is a step SEPARATOR (tactic_script.rs:61-64), so `exists a , b` parses as
           `exists a` followed by the atom `b`, which is an unknown tactic. Nested existentials
           need one `exists` atom per binder.
@guard     THE VARIABLE-BECOMES-CONSTANT TRAP, and the subtlest failure in Tier 8. When the Lean
           witness is a variable already in scope — `intro n; use n` — the translator naturally
           emits `exists n`. That produces `ProofTerm::Constant("n")`
           (tactic_script.rs:280), while the `intro n` that bound it produced
           `ProofTerm::Variable("n")` (tactic.rs:333). The instantiated goal therefore mentions
           `Constant("n")` and the hypotheses mention `Variable("n")`; `assumption` and `exact`
           compare with derived `PartialEq` (tactic.rs:363, :380) and find no match. The script
           fails at a step AFTER the `exists`, with a message about the wrong hypothesis, and
           the two terms PRINT IDENTICALLY — `Display` renders both as `n` — so the difference
           is invisible in every diagnostic the user sees.
@guard     Catches the `∃`-elimination round trip. `cases h` on an existential introduces a
           fresh witness `ProofTerm::Constant("W{n}")` (tactic.rs:536-541), which IS a
           `Constant`. So `obtain ⟨w, hw⟩ := h; use w` translates correctly only if the
           translator emits the LOGOS-generated `W{n}` rather than Lean's `w` — and only
           because both sides are `Constant`s. The same emission pattern is right here and
           wrong in the previous guard, which is why the translator must track the PROVENANCE of
           every witness name, not just its spelling.
@guard     Catches emitting a numeral and assuming the `Nat` bridge. `exists 0` yields
           `ProofTerm::Constant("0")` — a constant whose NAME is the digit zero, not a
           `Literal::Nat` and not the kernel's `Zero` constructor. Whether that matches the
           goal depends on how the translator rendered numerals elsewhere in the same goal; if
           it rendered them as `Constant("Zero")` or as a literal, the witness will not match.
           One numeral convention must be chosen and applied everywhere.
@guard     Catches the `witness_type` field. `ExistentialIntro` records
           `witness_type: "Entity"` unconditionally (tactic.rs:455) — `Entity` being the LOGOS
           prelude's opaque zero-constructor entity type. The recorded type is therefore never
           the Lean witness's type, and a translator must not read it back as one.
@note      `use` is a MATHLIB tactic; bare Lean 4.14 has only the core `exists`, which accepts
           the same comma-separated witness list and also discharges the residual goal. Since
           this corpus forbids `import Mathlib`, the live code below uses `exists` throughout
           and each `use` spelling is named in the doc comment. The LOGOS alias table maps the
           WORD `use` onto the same `exists` atom (tactic_script.rs:314), so the two spellings
           are interchangeable on the target side regardless.
@oracle    Every emitted `exists` atom's argument is a single `[A-Za-z0-9_]+` word; for a
           witness that was a Lean local variable the translator has either emitted a global
           definition naming it or refused; a follow-up atom discharging the instantiated goal
           is always present; and the loss record names WITNESS_TERM_SHAPE and
           TRAILING_DISCHARGE. The harness asserts `run_script` leaves `open_goals() >= 1`
           immediately after every `exists`.
-/

namespace T08_017

/-- The shape that degrades cleanly: a closed, atomic witness. -/
theorem closedWitness : ∃ n : Nat, n = 0 := by
  exists 0

/-- Lean's `exists` (and Mathlib's `use`) close the residual goal themselves; LOGOS's
    `exists` does not. Mathlib: `by use 0`. -/
theorem useCloses : ∃ n : Nat, n + 1 = 1 := by
  exists 0

/-- AN APPLIED WITNESS. `use (n + 1)` cannot be tokenised by the script lexer. -/
theorem appliedWitness (n : Nat) : ∃ m : Nat, m = n + 1 := by
  exists n + 1

/-- THE VARIABLE TRAP: the witness is a local bound by `intro`. `exists n` makes it a
    `Constant` while `intro n` made it a `Variable`. -/
theorem localVariableWitness : ∀ n : Nat, ∃ m : Nat, m = n := by
  intro n
  exists n

/-- The `∃`-elimination round trip: the witness came FROM a `cases`, so it is already a
    constant on the LOGOS side — but its name is `W{k}`, not `w`. -/
theorem witnessRoundTrip (p : Nat → Prop) (h : ∃ n, p n) : ∃ m, p m := by
  obtain ⟨w, hw⟩ := h
  exists w

/-- NESTED existentials: one Lean `exists a, b` (Mathlib: `use a, b`), two LOGOS `exists`
    atoms, because `,` is a step SEPARATOR in the script grammar. -/
theorem nestedExists : ∃ a b : Nat, a + b = 2 := by
  exists 1, 1

/-- A witness that is a FUNCTION APPLICATION of a local. -/
theorem functionWitness (f : Nat → Nat) (a : Nat) : ∃ m, m = f a := by
  exists f a

/-- The term spelling — same elaborated `Expr`, same problem. -/
theorem asTerm : ∃ n : Nat, n = 5 := ⟨5, rfl⟩

end T08_017
