/-!
@case      T02-018
@tier      2 · Declarations
@title     `section` + `variable` — binders are inserted by Lean before the translator sees
           anything
@lean      section … end, variable, variable [inst], autoBound section variables
@target    Context::add_definition with the *closed* type and body Lean produced, plus
           Context::set_binder_kinds recording where the inserted binders are implicit or
           instance-implicit
@expect    translate
@evidence  kernel/src/context.rs:20-100 (the `Context` has no section stack and no ambient
           binder list — every registered type is closed); kernel/src/context.rs:199-208
           (`set_binder_kinds` / `binder_kinds` — the only carrier for
           implicit/explicit/instance positions); kernel/src/elaborate.rs:1017-1027
           (`ParamKind::{Explicit, Implicit, Instance}`; instance positions are resolved
           *after* the explicit arguments, so their position matters);
           kernel/src/context.rs:82-86 (`implicit_args` — the legacy "N leading implicits"
           model that `binder_kinds` supersedes; using it for an interleaved telescope is
           wrong)
@guard     Catches translating the *surface* declaration instead of the elaborated one.
           `def f : Nat := n` inside `variable (n : Nat)` elaborates to `f : Nat → Nat`. A
           translator reading syntax emits a body with a free `Var("n")`; `add_definition`
           accepts it without any check (context.rs:442-444), so the dangling variable is
           discovered only when something normalizes through `f`, and the error names `n`.
@guard     Catches getting the *order* of inserted binders wrong. Lean inserts section
           variables in declaration order, but only those actually used (and their
           dependencies). A translator that inserts all section variables, or inserts them in
           source order regardless of dependency, produces a type with the right binders in the
           wrong positions. Since LOGOS is cumulative and `is_subtype` is structural on `Pi`
           telescopes (type_checker.rs:1330-1360), a permuted telescope of same-sorted binders
           can still typecheck at every call site while meaning something different.
@guard     Catches dropping `set_binder_kinds`. A `variable {α : Type}` becomes an *implicit*
           leading binder; if the translator records nothing, LOGOS falls back to the
           `implicit_args` count model (context.rs:82-86) which assumes implicits are all
           *leading*. An interleaved telescope (`{α} (n) [Inst α]`) is then mis-modelled and
           every application inserts the wrong number of metavariables.
@guard     Catches `variable [Inst α]`. The inserted binder is `ParamKind::Instance`, and
           LOGOS resolves instance positions only after the explicit arguments
           (elaborate.rs:1017-1027). Recording it as `Implicit` makes resolution fire before
           `α` is pinned, and the flat first-match instance search (context.rs:184-186) then
           picks whatever entry unifies with an unsolved metavariable — i.e. the first one.
@oracle    Each declaration's emitted type is closed (no free `Var`), equals the translation of
           Lean's `ConstantInfo.type`, and `binder_kinds(name)` matches Lean's `BinderInfo`
           sequence position for position.
-/

namespace T02_018

section Basics

variable (n : Nat)

/-- Elaborates to `useN : Nat → Nat`. The surface syntax shows no binder at all. -/
def useN : Nat := n + 1

/-- Uses `n` twice; still exactly one inserted binder. -/
def useNTwice : Nat := n + n

/-- Does **not** use `n`, so Lean inserts nothing: `noN : Nat`. A translator that inserts
    every section variable gets this one wrong. -/
def noN : Nat := 0

end Basics

example : useN 1 = 2 := rfl
example : useNTwice 3 = 6 := rfl
example : noN = 0 := rfl

section Implicits

variable {α : Type} (xs : List α) [Inhabited α]

/-- The inserted telescope is `{α : Type} (xs : List α) [Inhabited α]` — implicit, explicit,
    instance, *interleaved*. This is the shape the `implicit_args` fallback cannot express. -/
def headOrDefault : α :=
  match xs with
  | [] => default
  | y :: _ => y

end Implicits

example : headOrDefault ([] : List Nat) = 0 := rfl
example : headOrDefault [7, 8] = 7 := rfl

section Ordering

variable (a : Nat) (b : Nat)

/-- Uses `b` but not `a`. Lean inserts only `b`, so this is `Nat → Nat`, not `Nat → Nat → Nat`.
    Inserting both — or inserting `a` — changes the arity. -/
def onlyB : Nat := b * 2

/-- Uses both, in the section's declaration order. -/
def bothAB : Nat := a + b

end Ordering

example : onlyB 5 = 10 := rfl
example : bothAB 2 3 = 5 := rfl

section Dependency

variable {β : Type} (f : β → β) (z : β)

/-- `β` is used only in the *types* of the other inserted binders, never in the body. It must
    still be inserted, and it must come first. -/
def applyTwice : β := f (f z)

end Dependency

example : applyTwice (fun n => n + 1) 0 = 2 := rfl

-- `variable` also works at the top level of a namespace, outside any `section`.
variable (k : Nat)

def topLevelK : Nat := k

example : topLevelK 4 = 4 := rfl

end T02_018
