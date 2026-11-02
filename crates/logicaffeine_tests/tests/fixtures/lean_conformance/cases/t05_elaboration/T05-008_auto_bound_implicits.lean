/-!
@case      T05-008
@tier      5 · Elaboration surface
@title     Auto-bound implicits — Lean invents binders the source never wrote
@lean      autoBound implicits (`relaxedAutoImplicit`), `Type*`-style auto-bound universes,
           free lowercase Greek/single-letter identifiers in a signature
@expect    degrade
@target    The FULL `Term::Pi` telescope, including every binder Lean auto-bound, plus a
           `set_binder_kinds` vector whose auto-bound positions are `ParamKind::Implicit`, plus
           a loss record naming each auto-bound binder.
@evidence  kernel/src/term.rs:464-469 (`Term::Pi` has no notion of a binder's origin — a
           binder the user wrote and one Lean invented are the same term);
           kernel/src/elaborate.rs:1016-1027 (`ParamKind` records only how a parameter is
           supplied, not where it came from);
           kernel/src/context.rs:199-206 (`set_binder_kinds` is the only per-parameter metadata
           in `Context`);
           kernel/src/term.rs:458 and kernel/src/context.rs:218 (an auto-bound UNIVERSE has no
           counterpart at all: `Term::Const { name, levels }` resolves only against
           `universe_polys`, i.e. only for `add_universe_poly` DEFINITIONS — an auto-bound
           universe on an inductive is T01-007, a rejection);
           kernel/src/term.rs:167-191 (`Universe::Type(u32)` takes a concrete `u32`, so a level
           VARIABLE can only be `Universe::Var` and only where a level parameter list exists)
@loss      BINDER_PROVENANCE. After translation there is no way to tell that `α` in `idf`'s
           type was auto-bound rather than written. Consequences: (a) a round-trip back to Lean
           cannot reproduce the source, (b) a later edit to Lean's `autoImplicit` setting
           changes the declaration's arity while the translated one is frozen, and (c) an
           auto-bound binder's NAME is chosen by Lean from the identifier the user happened to
           type, so it participates in LOGOS's NAMED `Var` representation (term.rs:450) and can
           collide with a user binder of the same name elsewhere in the telescope.
@loss      AUTO_BOUND_ORDERING. Lean inserts auto-bound implicits at the FRONT of the
           telescope, in first-occurrence order, and re-sorts them by dependency. That order is
           not derivable from the surface text. The emitted `Pi` telescope must be Lean's, and
           the loss record must pin it, because `binder_kinds` is positional
           (context.rs:201) and any later re-derivation that orders them differently makes
           every call site's argument land in the wrong slot.
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax`.** Auto-bound
           implicits exist *only* in the elaborated signature: the `Syntax` for `def idf (a : α)
           : α := a` contains no binder for `α` at all. A `Syntax`-level translator must
           reimplement Lean's auto-binding rules — which identifiers qualify, whether
           `relaxedAutoImplicit` is on, where the binders are inserted, and how they are sorted
           — and any divergence changes the declaration's arity. This is the case where the
           `Syntax` surface is not merely inconvenient but actively missing information.
@guard     Catches emitting the telescope WITHOUT the auto-bound binders, on the theory that
           "the source did not write them". The emitted type is then `α → α` with `α` a free
           `Term::Var`, which in LOGOS's named representation is not a detectable error — it
           resolves against whatever `α` is in scope at the use site, or fails lookup far from
           the declaration.
@guard     Catches getting the auto-bound binder's SORT wrong. With `autoImplicit` on, Lean
           binds `α` at `Sort u` for a fresh auto-bound universe `u`, not at `Type 0`. Pinning
           it to `Type 0` is a silent monomorphisation with exactly the failure mode of
           T01-007: because LOGOS is CUMULATIVE (type_checker.rs:1330-1360) while Lean is not,
           the monomorphised declaration still typechecks at most use sites and is wrong
           wherever the source instantiated it above `Type 0`.
@guard     Catches auto-bound UNIVERSES specifically. `sortId` below is auto-bound at
           `Sort u`; if the declaration is a `def`, `add_universe_poly` (context.rs:218) can
           carry it and `Term::Const { levels }` can instantiate it. If the same auto-binding
           happens on an `inductive` or a `structure`, there is no carrier at all and the case
           becomes a rejection (T01-007, T03-012) — a translator must not silently degrade
           those.
@guard     Catches name collision between an auto-bound binder and a user binder. `collide`
           below has a user binder named `α` in an inner scope; the auto-bound outer `α`
           shadows or is shadowed depending on emit order, and `substitute_avoiding`
           (type_checker.rs:1088) only alpha-renames when the REPLACEMENT's free variables
           collide — not when two binders merely share a name. Freshening is the translator's
           job.
@oracle    The emitted `Pi` telescope has exactly the arity of Lean's elaborated signature,
           binder for binder and in Lean's order; `binder_kinds` marks each auto-bound position
           `Implicit`; the emitted term has no free `Term::Var`; and the loss record contains
           BINDER_PROVENANCE listing each auto-bound binder by name and position.
-/

set_option autoImplicit true
set_option relaxedAutoImplicit true

namespace T05_008

/-- `α` is never written as a binder; Lean auto-binds it at the front of the telescope. -/
def idf (a : α) : α := a

/-- Two auto-bound type variables, in first-occurrence order. -/
def constf (a : α) (_b : β) : α := a

/-- An auto-bound variable whose binder must be inserted BEFORE another auto-bound one it
    depends on — the dependency re-sort is not derivable from the source text. -/
def applyF (f : α → β) (a : α) : β := f a

/-- Auto-bound at `Sort u`, with an auto-bound UNIVERSE. For a `def` this can be carried by
    `add_universe_poly`; for an inductive it cannot be carried at all. -/
def sortId (a : α) : α := a

/-- An auto-bound variable used in a THEOREM statement. -/
theorem idfEq (a : α) : idf a = a := rfl

/-- The auto-bound binder's name collides with a user binder in an inner scope. -/
def collide (a : α) : α :=
  let g := fun {α : Type} (x : α) => x
  g a

/-- Auto-bound inside an inductive's signature — this is where the degrade stops being
    available, because LOGOS inductives carry a fixed sort (context.rs:253,374). -/
inductive BoxOf (a : α) : Type where
  | mk : BoxOf a

/-- A `Nat`-valued auto-bound implicit: `relaxedAutoImplicit` binds even non-type variables
    that look like they could be implicit. -/
def replicateN (n : Nat) (x : α) : List α := List.replicate n x

example : idf 3 = 3 := rfl
example : constf 1 true = 1 := rfl
example : applyF (fun n : Nat => n + 1) 2 = 3 := rfl
example : sortId true = true := rfl
example : collide 5 = 5 := rfl
example : replicateN 2 (7 : Nat) = [7, 7] := rfl

end T05_008
