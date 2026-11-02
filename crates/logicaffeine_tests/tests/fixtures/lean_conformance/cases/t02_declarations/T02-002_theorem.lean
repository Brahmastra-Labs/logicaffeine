/-!
@case      T02-002
@tier      2 · Declarations
@title     `theorem` — there is no theorem form in LOGOS; it is a `Prop`-typed definition
@lean      theorem, Lean.TheoremVal, proof irrelevance at the declaration level
@target    Context::add_definition(name, ty, proof_term)  — the same API as `def`
@expect    translate
@evidence  kernel/src/context.rs:442-444 (`add_definition` is the only body-carrying
           registration; there is no `add_theorem`, and no `Declaration` enum with a
           `theorem` arm — the context is parallel maps only, context.rs:20-100);
           kernel/src/context.rs:434-436 (`add_declaration` = axiom, type only);
           kernel/src/type_checker.rs:1547-1567 (`proof_irrelevant` is a *conversion* rule,
           not a declaration-level erasure)
@guard     Catches the translator treating `theorem` as `add_declaration` "because proofs are
           irrelevant anyway". That is exactly how a proof library becomes a pile of axioms:
           every theorem silently turns into an assumption, `sorry`-equivalent, and the
           certificate still verifies because the axiom is in the context. The distinction
           `is_definition(name)` (context.rs:467) is the only way to tell afterwards, and by
           then the provenance is lost.
@guard     Catches the inverse asymmetry with Lean: Lean's `theorem` bodies are *irreducible*
           for delta (the kernel will not unfold a theorem during defeq except in special
           cases), while LOGOS's `add_definition` is unconditionally transparent
           (context.rs:54-57). So a LOGOS proof term *can* be unfolded where Lean's cannot.
           That direction is harmless for soundness but changes reduction behaviour and the
           `normalize` fuel budget (reduction.rs:52-69); a translator that also relies on
           theorem-opacity for performance will not get it.
@guard     Catches emitting the *statement* rather than the *proof term* as the body. Since
           `add_definition` never typechecks, `add_definition("foo", P, P)` is accepted.
@guard     Catches losing the proof entirely for a theorem proved `by tac`. The translator must
           read the elaborated `TheoremVal.value` (the finished proof term), not the tactic
           block — LOGOS has no way to replay a Lean tactic.
@oracle    The emitted name is a *definition* whose stored type normalizes to a `Sort(Prop)`,
           whose body `infer_type`s to that type, and `is_definition(name) == true`.
-/

namespace T02_002

/-- The minimal theorem: a `Prop`-typed definition whose body is a proof term. -/
theorem zero_add_eq (n : Nat) : 0 + n = n := Nat.zero_add n

/-- Proved by a tactic block. The `TheoremVal.value` Lean stores is a *term*; the tactic
    script is not part of the declaration and cannot be translated. -/
theorem add_zero_eq (n : Nat) : n + 0 = n := by
  simp

/-- A theorem used by a later theorem. If the first was emitted as an axiom, this still
    compiles in LOGOS — which is precisely why the guard matters. -/
theorem add_zero_zero : 0 + 0 = (0 : Nat) := zero_add_eq 0

/-- A `def` whose type happens to be a `Prop`. In Lean this is a *definition*, not a
    theorem, and it is delta-transparent; in LOGOS the two are literally the same call,
    so the distinction is unrecoverable after translation. -/
def trivialProof : True := True.intro

/-- A theorem whose statement quantifies — the emitted type is a `Term::Pi` into `Prop`,
    whose sort is computed by `imax` and is therefore `Prop`. -/
theorem all_refl : ∀ n : Nat, n = n := fun _ => rfl

/-- A theorem whose proof term mentions a `def`. The `def` must be a transparent
    definition or this proof does not check after translation. -/
def isZero (n : Nat) : Bool := Nat.beq n 0
theorem isZero_zero : isZero 0 = true := rfl

end T02_002
