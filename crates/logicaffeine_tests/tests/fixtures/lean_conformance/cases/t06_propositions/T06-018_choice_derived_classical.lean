/-!
@case      T06-018
@tier      6 · Propositions, equality, classical reasoning
@title     Classical content that goes BEYOND `dne` — `Classical.byCases` is fine, its
           `Decidable`-producing relatives are not
@lean      Classical.byCases, Classical.not_forall, Classical.skolem, Classical.axiomOfChoice,
           Classical.decEq, Classical.dec, `open Classical in` instance activation
@expect    reject
@target    (none — for the declarations below that need choice. The PROP-only fragment
           (`Classical.byCases`, `not_forall` at a decidable-free statement) is derivable from
           `dne` and belongs to T06-008; everything in this file is deliberately outside it.)
@evidence  kernel/src/prelude.rs:46-78 (`register_classical` — the complete classical fragment
           of the LOGOS trusted base is the single `add_declaration("dne", …)` at :78);
           kernel/src/prelude.rs:757-818 (`Decidable` is `Type 0`-valued, with exactly two
           constructors, and its ONLY prelude instances are `decEqBool` at :926 and `decEqNat`
           at :1027); kernel/src/type_checker.rs:1688-1716 (`is_subsingleton_prop`: `Or` has two
           constructors, so it is NOT a subsingleton) and kernel/src/recursor.rs:64-71
           (`derive_recursor` therefore pins `Or`'s motive to `Sort(Prop)`) — so the derived
           `Classical_em : Π(P:Prop). Or P (Not P)` of T06-008 CANNOT be eliminated into
           `Decidable p`, which lives in `Type 0`; kernel/src/prelude.rs:1910-1983 (`Ex`'s
           derived eliminator is likewise `Prop`-motived, so no `Type`-level witness escapes);
           kernel/src/context.rs:184 (`add_instance` — a flat `Vec<(Term,Term)>`, so an
           `open Classical` blanket instance would be a single entry that FIRST-MATCHES every
           `Decidable` goal in the whole development, elaborate.rs:1331-1340)
@diag      E_CLASSICAL_BEYOND_DNE — must name the specific Lean constant, state that LOGOS's
           classical base is exactly `dne` (prelude.rs:78), and distinguish the two failure
           modes: (a) the statement needs CHOICE, or (b) the statement needs a `Prop` to be
           eliminated into `Type`. Must not fall back to E_AXIOM_NOT_IN_TRUSTED_BASE alone,
           because the fix is different — (a) is unfixable, (b) sometimes has a `Decidable`
           instance the user can supply.
@guard     Catches the "one more derivation" slide out of T06-008. `Classical.byCases` IS
           derivable from `dne` — it is `Or.elim` on the derived `Classical_em` at a `Prop`
           motive — and a translator that gets that right is one step from believing the same
           of `Classical.propDecidable`. It is not. The step from `p ∨ ¬p` to `Decidable p`
           crosses the `Prop`/`Type` boundary, and LOGOS blocks it structurally: `Or` has two
           constructors, so `is_subsingleton_prop` is false
           (type_checker.rs:1688-1716) and the derived eliminator's motive is pinned to
           `Sort(Prop)` (recursor.rs:64-71). The translator must key on the MOTIVE's sort, not
           on whether the Lean constant's name starts with `Classical.`.
@guard     Catches the blanket-instance disaster. `open Classical in` activates
           `Classical.propDecidable` as a low-priority instance in Lean, and it then silently
           discharges every `Decidable` goal in scope. Translated naively that becomes ONE
           `add_instance` entry in LOGOS's flat `Vec<(Term,Term)>` (context.rs:184). LOGOS has
           no priorities: `resolve_instance_at` scans the vector in INSERTION ORDER and the
           first that unifies wins (elaborate.rs:1331-1340). So the blanket instance either
           shadows `decEqNat`/`decEqBool` entirely (if inserted first) or is dead (if inserted
           last), and which one happens depends on translation order. See T07-007 and T07-008.
@guard     Catches `Classical.skolem` / `axiomOfChoice`. These are choice with a function
           produced, not merely a witness; translating them requires `Classical.choice`
           (T06-009) AND `funext` (T06-011) to state the extensionality of the chosen function.
           Rejecting one of the three and emitting the others is the worst outcome: the emitted
           fragment typechecks, so the rejection looks like a localised gap rather than a
           structural one.
@guard     Catches `Classical.not_forall`. Its statement `(¬∀ x, p x) ↔ ∃ x, ¬p x` is pure
           `Prop` and IS derivable from `dne` — but its Lean proof goes through
           `Classical.byContradiction` plus `Classical.em` at the INSTANTIATED predicate, and
           the elaborated term in Lean 4 core mentions `Classical.choice` through
           `Classical.propDecidable`. A translator that re-derives the statement from `dne` is
           correct and must say so in its output; a translator that translates Lean's PROOF
           TERM verbatim will drag choice in. This is the one declaration in the file where the
           right answer is "re-prove, do not transcribe".
@oracle    Translator exits non-zero with E_CLASSICAL_BEYOND_DNE for `decEqAnything`,
           `skolem`, `choiceFunction` and `blanketInstance`; performs zero `add_declaration`
           calls; and emits at most one `add_instance` for the whole file. `byCases` and
           `notForall` may be translated, but only via a `dne`-rooted derivation, and the
           harness checks that the emitted proof terms mention `Global("dne")` and no other
           axiom.
-/

namespace T06_018

/-- DERIVABLE from `dne` (this is T06-008 territory) — included as the control. -/
theorem byCases (p q : Prop) (hp : p → q) (hnp : ¬p → q) : q :=
  Classical.byCases hp hnp

/-- Statement derivable from `dne`; Lean's PROOF TERM is not. The translator must re-prove. -/
theorem notForall (p : Nat → Prop) : (¬∀ n, p n) ↔ ∃ n, ¬p n :=
  Iff.intro
    (fun h => Classical.byContradiction (fun hne =>
      h (fun n => Classical.byContradiction (fun hpn => hne ⟨n, hpn⟩))))
    (fun ⟨n, hpn⟩ h => hpn (h n))

/-- BEYOND `dne`: a `Prop` eliminated into `Type 0`. Needs choice. -/
noncomputable def decEqAnything (α : Type) : DecidableEq α :=
  fun a b => Classical.propDecidable (a = b)

/-- BEYOND `dne`: Skolemisation produces a FUNCTION, so it needs choice and, to be stated
    extensionally, `funext` as well. -/
noncomputable def skolem (p : Nat → Nat → Prop) (h : ∀ n, ∃ m, p n m) :
    { f : Nat → Nat // ∀ n, p n (f n) } :=
  ⟨fun n => Classical.choose (h n), fun n => Classical.choose_spec (h n)⟩

/-- BEYOND `dne`: the axiom of choice in its Lean-core spelling. -/
noncomputable def choiceFunction {α : Type} {β : α → Type} (h : ∀ a, Nonempty (β a)) :
    (a : α) → β a := fun a => Classical.choice (h a)

/-- BEYOND `dne`, and uniquely dangerous: a BLANKET `Decidable` instance. In LOGOS this is one
    entry in a flat, priority-free, first-match-wins instance vector. -/
noncomputable def blanketInstance : Nat := by
  classical
  exact if (∃ n : Nat, n > 5) then 1 else 0

end T06_018
