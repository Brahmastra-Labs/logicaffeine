/-!
@case      T01-007
@tier      1 · Universes
@title     Universe-polymorphic inductive type — unrepresentable in LOGOS
@lean      inductive with universe parameters, Sort u / Type u valued inductive families
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/term.rs:458 (`Term::Const { name, levels }` is the ONLY universe-
           instantiating form); kernel/src/type_checker.rs:70-85 and kernel/src/reduction.rs:82-91
           (`Const` resolves *only* against `Context::universe_polys`, which
           context.rs:65 documents as a map of universe-polymorphic **definitions**);
           kernel/src/context.rs:253,374 (`add_inductive` / `add_indexed_inductive` take a
           single fixed `sort: Term` — there is no universe-parameter list on an inductive or on
           a constructor); kernel/src/recursor.rs:64-71 (`derive_recursor` hard-codes the motive
           target to `Sort(Prop)` for non-subsingleton Prop inductives and `Sort(Type 0)` for
           everything else)
@diag      E_NO_UNIVERSE_POLYMORPHIC_INDUCTIVE — must name the inductive, list its universe
           parameters, and state that LOGOS inductives carry a fixed sort. Must NOT suggest a
           silent monomorphisation.
@guard     This is the single highest-value rejection in the corpus. The tempting wrong move is
           **silent monomorphisation at `Type 0`**: emit `List : Type 0 → Type 0` and carry on.
           That compiles, typechecks, and is wrong the moment the source instantiates the
           inductive at two different levels — the two instantiations collapse to one LOGOS
           global, so `List (List α)` and `List α` share a declaration whose recursor can only
           eliminate into `Type 0`. Every downstream proof is then checked against a type that
           is not the one the Lean source meant. Because LOGOS is *cumulative*
           (type_checker.rs:1330-1360) while Lean is not, the resulting terms very often still
           typecheck — so the kernel will not catch this for you.
@guard     Second tempting wrong move: emitting the inductive as a universe-polymorphic
           *definition* via `add_universe_poly` (context.rs:218) so that `Term::Const` resolves.
           That makes the type delta-transparent, so it is no longer an inductive: it has no
           constructors, cannot be matched on, and `add_constructor` will refuse to attach to it.
@guard     Third: monomorphising *per instantiation* (`List@0`, `List@1`) is sound-ish for the
           types but breaks as soon as a universe-polymorphic *proof* quantifies over `u`,
           because there is no level variable to bind. If the translator does this it must still
           reject any declaration whose universe parameters are not fully determined at every
           use site, and must emit a loss record — see T01-008.
@oracle    Translator exits non-zero with E_NO_UNIVERSE_POLYMORPHIC_INDUCTIVE and writes no
           declaration into the LOGOS context.
-/

namespace T01_007

/-- The canonical case: a universe-polymorphic container. -/
inductive Box (α : Type u) : Type u where
  | mk : α → Box α

/-- Instantiated at two different levels in the same file — this is what makes
    monomorphisation at `Type 0` unsound rather than merely lossy. -/
def atLevel0 : Box Nat := Box.mk 0
def atLevel1 : Box Type := Box.mk Nat

/-- Universe-polymorphic inductive with a `Sort u` (rather than `Type u`) codomain, so it can
    land in `Prop` when `u = 0`. LOGOS's `Universe::Type(u32)` cannot even express this arity:
    `Sort` takes a `Universe`, but `Type` takes a concrete `u32` (term.rs:167-191). -/
inductive Squash (α : Sort u) : Prop where
  | mk : α → Squash α

/-- Two universe parameters with a `max` — LOGOS's `Universe::Max` exists for *levels*, but
    there is nowhere to bind `u` and `v` on an inductive. -/
inductive Either (α : Type u) (β : Type v) : Type (max u v) where
  | left  : α → Either α β
  | right : β → Either α β

end T01_007
