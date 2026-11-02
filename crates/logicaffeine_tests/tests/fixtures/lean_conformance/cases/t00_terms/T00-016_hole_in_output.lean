/-!
@case      T00-016
@tier      0 · Kernel term layer
@title     `Term::Hole` surviving into emitted output — `types_equal` matches it against ANYTHING
@lean      implicit arguments the translator cannot reconstruct, `_` in types, match motives,
           erased instance arguments
@expect    reject
@target    (none — a `Term::Hole` must never appear in translator output, in ANY position)
@diag      E_HOLE_IN_OUTPUT — must name the declaration, the path to the `Hole` inside the term,
           and the Lean subterm that could not be reconstructed. The translator must write NO
           declaration for that Lean declaration.
@evidence  kernel/src/type_checker.rs:1594-1597 (`types_equal`: `if matches!(a, Term::Hole) ||
           matches!(b, Term::Hole) { return true; }` — a `Hole` is equal to EVERY term);
           kernel/src/type_checker.rs:1309-1316 (`is_subtype` calls `types_equal` as its FAST
           PATH before normalising, so the leniency is reached on the cheapest, most-travelled
           route) and kernel/src/type_checker.rs:1366-1368 (`def_eq` does the same);
           kernel/src/type_checker.rs:455-471 (`check_type` accepts a `Hole` TERM against any
           `Sort`, and accepts ANY well-typed term against a `Hole` EXPECTED type);
           kernel/src/type_checker.rs:313 (`infer_type(Hole)` is `CannotInferHole` — the ONLY
           place a `Hole` is refused, and it is reached only when the `Hole` is in a position
           that is inferred rather than merely compared);
           kernel/src/type_checker.rs:1104 (`substitute_avoiding` propagates a `Hole` unchanged,
           so it survives every reduction and every instantiation);
           kernel/src/elaborate.rs:142-150 (`instantiate` likewise leaves `Hole` alone — the
           elaborator's metavariable machinery does not solve it)
@guard     This is the corpus's canonical "it typechecked, therefore it is fine" refutation. A
           single `Hole` in a TYPE makes every comparison against that type succeed. If the
           translator emits `theorem thm : <statement with a Hole> := <proof>`, then
           `check_type(proof, statement)` short-circuits through `types_equal`
           (type_checker.rs:1596) and returns `Ok(())` for a proof of something else entirely.
           The theorem is recorded, `verified == true` is reported (verify.rs:31-43), and the
           library now contains a certified statement that nothing proves. Nothing anywhere
           emits a warning.
@guard     Catches the belief that `CannotInferHole` (type_checker.rs:313) is a backstop. It
           fires only for a `Hole` that `infer_type` visits. `infer_type` visits a `Pi`'s
           `param_type` (via `infer_sort` at :96) but NOT the argument position of an
           application whose function type is already a `Pi` — there the argument is only
           `check_type`d (:171), and `check_type` against a non-`Hole` expected type calls
           `types_equal` first. So a `Hole` as a *type argument* of a polymorphic function
           (`@Eq _ a b`, `@List.cons _ x xs`) is never inferred and never reported.
@guard     Catches using `Hole` as a placeholder "to be filled in later" in a multi-pass
           translator. There is no later: `substitute_avoiding` (type_checker.rs:1104) and
           `instantiate` (elaborate.rs:142-150) both preserve `Hole` verbatim, so a `Hole`
           written in pass 1 is still there in the certificate. If a pass-structured translator
           needs a placeholder it must use its own sentinel OUTSIDE the `Term` type and assert
           the sentinel is gone before handing anything to the kernel.
@guard     Catches emitting `Hole` for an ERASED instance argument. Instance arguments are
           reconstructible — `Context::add_instance` (context.rs:184) holds a flat
           `Vec<(Term, Term)>` matched first-wins — so the translator must resolve them, accept
           that the resolution may differ from Lean's (T07-008), and RECORD that divergence.
           Substituting a `Hole` converts a recorded divergence into an unrecorded unsoundness.
@guard     Catches emitting `Hole` for a `Match` motive. `infer_match` computes the result type
           as `beta_reduce(App(motive, discriminant))` (type_checker.rs:271); with a `Hole`
           motive that is `App(Hole, disc)`, which is not a `Sort`, so the large-elimination
           gate at :277-288 — `matches!(normalize(infer_type(return_type)), Sort(Prop))` — takes
           the wrong branch and the `Prop`-elimination restriction is silently skipped.
@oracle    A structural walk of every emitted `Term` (declaration types, bodies, structure field
           types, constructor types, recursor types) finds zero `Term::Hole` nodes. This check
           must be run on the output, NOT inferred from the kernel accepting the output — the
           kernel accepting it is precisely the symptom.
-/

namespace T00_016

/-- Implicit type arguments Lean fills by unification. In the `Expr` they are ordinary explicit
    arguments; a translator that cannot reconstruct one reaches for a `Hole` here. -/
def constFn {α : Type} {β : Type} (a : α) (_b : β) : α := a
def implicitsPresent : Nat := constFn 3 true

/-- The `@Eq _ a b` shape: the type argument of `Eq` is an implicit the source never writes.
    A `Hole` there is never `infer_type`d, only compared — so it is never reported. -/
theorem eqWithImplicitType : @Eq Nat 1 1 := rfl

/-- An instance argument. Reconstructible from LOGOS's instance table, but only if the
    translator actually resolves it. -/
def usesInstance {α : Type} [Inhabited α] : α := default
def resolvedInstance : Nat := usesInstance

/-- A dependent match whose MOTIVE Lean infers. The motive is a real term in the `Expr`, and the
    position where a `Hole` disables the large-elimination gate. -/
def motiveInferred (n : Nat) : Fin (n + 1) :=
  match n with
  | 0 => ⟨0, by omega⟩
  | k + 1 => ⟨k + 1, by omega⟩

/-- A motive that genuinely DEPENDS on the discriminant, so it cannot be guessed as a constant
    function — the case where a lazy translator is most tempted. -/
def dependentMotive (b : Bool) : (if b then Nat else Bool) :=
  match b with
  | true => (3 : Nat)
  | false => false

/-- A `Prop`-valued inductive eliminated into `Prop`. If the motive is a `Hole`, the gate at
    type_checker.rs:277-288 cannot classify the return type and the restriction is skipped —
    which is how a `Hole` becomes a soundness bug rather than a precision bug. -/
theorem orElim (p q : Prop) (h : p ∨ q) : q ∨ p :=
  match h with
  | Or.inl hp => Or.inr hp
  | Or.inr hq => Or.inl hq

/-- A universe level the translator must supply rather than hole out. There is no `Hole` at the
    `Universe` level at all (term.rs:167-191), so the temptation is to hole the whole `Sort`. -/
def polyAtLevelZero : List Nat := []

/-- A type ascription against a type containing an implicit. Both sides are compared, never
    inferred — the maximally quiet position for a surviving `Hole`. -/
def ascribedWithImplicit : Nat := (constFn 3 true : Nat)

end T00_016
