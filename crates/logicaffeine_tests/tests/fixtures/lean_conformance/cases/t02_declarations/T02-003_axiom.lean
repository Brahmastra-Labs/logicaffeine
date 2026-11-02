/-!
@case      T02-003
@tier      2 · Declarations
@title     `axiom` — an opaque, body-less global
@lean      axiom, Lean.AxiomVal, Environment.addDecl on an axiom
@target    Context::add_declaration(name: &str, ty: Term)
@expect    translate
@evidence  kernel/src/context.rs:434-436 (`add_declaration` stores `name -> ty` in
           `declarations`, the axiom/hypothesis map); kernel/src/context.rs:50-52 (documented
           as "axioms/hypotheses … no body"); kernel/src/context.rs:474-476
           (`get_definition_body` is `None` for a declaration, so nothing delta-unfolds);
           kernel/src/prelude.rs:78 (`dne` is registered this way — the entire classical
           content of LOGOS is one `add_declaration`)
@guard     Catches emitting an axiom as a *definition* with some placeholder body. A
           `Term::Hole` body is the worst version: `types_equal` treats `Hole` as matching
           **anything** (type_checker.rs:1594-1597), so the fake definition typechecks against
           every use and the axiom becomes an unsound universal.
@guard     Catches losing the axiom from the trust report. LOGOS has no `#print axioms`: the
           `declarations` map is the *entire* record of what is assumed, and
           `iter_declarations` (context.rs:520) is the only audit surface. A translator must
           emit a machine-readable list of every `add_declaration` it performs, because a
           downstream certificate that verifies against an enlarged axiom set is not evidence
           of anything. The certificate's `PRELUDE_VERSION` (certificate.rs:28, "logos-coc-1")
           pins only the *prelude's* axioms, not translator-added ones.
@guard     Catches translating a Lean axiom whose statement is refutable in LOGOS's prelude.
           LOGOS already assumes `dne` plus 16 `Int` ring/order axioms (prelude.rs:92-340); an
           incoming Lean axiom is added to *that* set, not to an empty one, so the consistency
           obligation is joint. A translator must at minimum record the union.
@guard     Catches the `get_global` precedence trap (context.rs:449-464): declarations are
           looked up **last**. If a name is registered both as a definition and as a
           declaration, the declaration is invisible — the type reported is the definition's.
@oracle    The emitted name resolves via `get_global` but `is_definition(name) == false` and
           `get_definition_body(name) == None`; and the translator's loss/trust report lists
           the axiom by name and type.
-/

namespace T02_003

/-- The canonical axiom: a `Prop` with no proof. -/
axiom choiceLike (P : Prop) : ¬¬P → P

/-- A *data*-valued axiom. Lean allows this; LOGOS allows it too (`add_declaration` does not
    care about the sort). It is far more dangerous than a `Prop` axiom because it introduces
    an inhabitant that has no computational content but is nonetheless matched on. -/
axiom mysteryNat : Nat

/-- An axiom that is *used*, so the dependence is real and must show up in the trust report
    of anything proved from it. -/
theorem usesAxiom : mysteryNat = mysteryNat := rfl

/-- An axiom with a dependent telescope: the emitted type is a `Term::Pi` chain, and
    `add_declaration` stores it verbatim with no well-formedness check. -/
axiom transportLike : ∀ (α : Type) (P : α → Prop) (a b : α), a = b → P a → P b

/-- An axiom shadowing the *name* of a later definition is the collision hazard: LOGOS's
    lookup order puts declarations last, so this axiom would be masked. Kept distinct here
    on purpose; see T02-020 for the collision case proper. -/
axiom separateName : Nat → Nat

end T02_003
