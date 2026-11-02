/-!
@case      T12-010
@tier      12 · Vernacular surfaces
@title     Surface C `induction` is hardcoded to `Nat` — the named variable and the type are both ignored
@lean      `induction xs with | nil => … | cons h t ih => …` (and the `Nat` case, for
           contrast)
@expect    degrade
@target    A literate proof using the `induction <v>` tactic with bullet cases:

           ```logos
           ## Theorem: Test
               Statement: True.
               Proof:
                   induction n.
                   - auto.
                   - auto.
           ```

           which `parse_proof_tactic` routes to `build_induction_derivation`
           (literate_parser.rs:865-902), producing
           `try_induction (SName "Nat") <motive> (DCase c1 (DCase c2 DCaseEnd))`.
@evidence  kernel/src/interface/literate_parser.rs:865-875 — the whole of the type
           selection:

               fn build_induction_derivation(
                   &self,
                   _var_name: &str,        // NOTE: leading underscore — unused
                   statement: &Term,
                   cases: Vec<Term>,
               ) -> Result<Term, ParseError> {
                   // Build the inductive type (for now assume Nat if not specified)
                   let ind_type = Term::App(
                       Box::new(Term::Global("SName".to_string())),
                       Box::new(Term::Lit(Literal::Text("Nat".to_string()))),
                   );

           The inductive type is a LITERAL `"Nat"`. The variable name is bound to
           `_var_name` and never read, so `induction n.` and `induction xs.` build the
           identical term.
@evidence  kernel/src/interface/literate_parser.rs:876-902 — the case list is a flat
           `DCase`/`DCaseEnd` chain in bullet order, with no constructor names and no
           per-case binder list. Lean's `| cons h t ih =>` naming has nowhere to go.
@evidence  tests/phase_literate_induction.rs:14-28 and :31-42 — the bullet syntax quoted
           above, verbatim; note both tests end in `let _ = result;` ("Document the
           behavior") rather than an assertion.
@evidence  tests/phase_literate_induction.rs:48-61 — `Proof: induction.` with no variable
           is asserted `is_err()`, so the variable token is REQUIRED to parse even though
           its value is discarded.
@evidence  tests/phase_literate_induction.rs:67-79 — a bullet count that does not match
           `Nat`'s two constructors is an error, which is how the `Nat` hard-coding
           becomes observable: a three-bullet proof fails, whatever the intended type.
@evidence  proof/src/tactic.rs:561-600 — the SIBLING surface has the same restriction for
           the same reason: `ProofState::induction` substitutes
           `ProofTerm::Constant("Zero")` and `ProofTerm::Function("Succ", [K<n>])`
           (tactic.rs:583-586). `induction_over` (tactic.rs:623-636) is the generic
           version and is Rust-API only — `tactic_script.rs:308-327` has no word that
           canonicalises to it.
@loss      INDUCTION_TYPE_PINNED. The proof term records induction over `Nat` regardless
           of the Lean source's inductive type, and records neither the variable name nor
           the constructor names. For a Lean `induction n : Nat` proof this loses only
           the variable name — acceptable, because `build_induction_derivation` takes the
           motive from the statement and `Nat` really is the type. For any other type it
           is not a loss but a falsehood; see @guard.
@loss      CASE_BINDER_ERASURE. `| succ k ih =>` names the predecessor and the induction
           hypothesis; the bullet list names neither. On the Surface A sibling the
           hypothesis is named `ih` and the step variable is a fresh `K<n>` constant
           (tactic.rs:577, :587), so a script that refers to a Lean-chosen name will not
           find it.
@guard     Catches a translator that emits `induction xs.` for a Lean list induction and
           takes the resulting `Ok` as success. The parse succeeds (the variable token is
           consumed, the bullets are counted), the emitted term is
           `try_induction (SName "Nat") …`, and the definition typechecks because
           `try_induction` is an opaque declaration over `Syntax`. Nothing anywhere
           reports that the type is wrong. This is a `degrade` verdict only because the
           loss record makes it detectable; a translator that omits the loss record has
           produced a certificate for induction over the wrong type.
@guard     Catches relying on the bullet count as a type check. It is a check against
           `Nat`'s two constructors, not against the source type's. A Lean induction over
           a two-constructor type that is not `Nat` (`List`, `Bool`, `Option`) passes the
           count check and is still mis-recorded; a three-constructor type fails with an
           error about bullet count that says nothing about types.
@guard     Catches the variable name. `_var_name` is discarded, so if the Lean proof
           inducts on a variable that is not the statement's first, the literate rendering
           inducts on whatever the motive's first binder is. Since Surface C has no
           binders at all (T12-009), "the motive's first binder" is itself ill-defined —
           which is why induction on this surface is only usable for statements that are
           already effectively closed over a single `Nat`.
@oracle    `Repl::execute(<emitted block>)` returns `Ok`, AND the loss record contains
           INDUCTION_TYPE_PINNED, AND — for any Lean source whose induction is not over
           `Nat` — the translator either refuses or records the loss with the source
           type named. A harness must additionally check `concludes` against the goal
           syntax (see T12-008), because `Ok` alone does not mean the induction closed.
-/

namespace T12_010

/-- Induction over `Nat`: the only case for which the hard-coded type is correct. Two
    constructors, so the two-bullet form also passes the count check. -/
theorem natSumFormula (P : Nat → Prop) (base : P 0) (step : ∀ k, P k → P (k + 1)) :
    ∀ n, P n := by
  intro n
  induction n with
  | zero => exact base
  | succ k ih => exact step k ih

/-- Induction over a list. Two constructors, so the bullet count matches and nothing
    complains — but the emitted term says `SName "Nat"`. -/
theorem listInduction (α : Type) (P : List α → Prop)
    (base : P [])
    (step : ∀ (h : α) (t : List α), P t → P (h :: t)) :
    ∀ xs, P xs := by
  intro xs
  induction xs with
  | nil => exact base
  | cons h t ih => exact step h t ih

/-- A concrete list induction, so the case is not purely schematic. -/
theorem appendNilRight (α : Type) (xs : List α) : xs ++ [] = xs := by
  induction xs with
  | nil => rfl
  | cons h t ih => simp

/-- A three-constructor inductive: here the bullet-count check fires, producing an error
    about bullets rather than about types. -/
inductive Colour where
  | red
  | green
  | blue

theorem colourInduction (P : Colour → Prop)
    (hr : P Colour.red) (hg : P Colour.green) (hb : P Colour.blue) :
    ∀ c, P c := by
  intro c
  cases c with
  | red => exact hr
  | green => exact hg
  | blue => exact hb

/-- Induction on a variable that is not the statement's first binder. `_var_name` is
    discarded, so the choice is unrecoverable from the emitted term. -/
theorem inductOnSecond (m n : Nat) : m + n = n + m := by
  induction n with
  | zero => simp
  | succ k _ => omega

end T12_010
