/-!
@case      T02-010
@tier      2 · Declarations
@title     `instance` — a definition plus an entry in a flat, first-match-wins instance list
@lean      instance, Lean.Meta.InstanceExtension, instance priorities, discrimination trees
@target    Context::add_definition(name, ty, body)  followed by
           Context::add_instance(ty: Term, value: Term)
@expect    translate
@evidence  kernel/src/context.rs:184-186 (`add_instance` pushes onto a flat
           `Arc<Vec<(Term, Term)>>` — no discrimination tree, no priority, no keying);
           kernel/src/context.rs:67-71 (documented: "each `(type, value)` is an instance the
           elaborator may resolve … Searched by unifying `type` against the required class
           type"); kernel/src/context.rs:210-213 (`instances()` hands the raw slice back, in
           insertion order); kernel/src/elaborate.rs:1017-1027 (`ParamKind::Instance` — an
           instance-implicit argument is resolved only *after* the explicit arguments, so the
           class's type variables are pinned first)
@guard     Catches emitting only the `add_definition` and forgetting `add_instance`. The
           instance then exists as an ordinary global that nothing will ever find: LOGOS has no
           attribute system to scan (see T02-013), so the *only* way an instance becomes
           resolvable is an explicit `add_instance` call. Every `[Foo α]` argument downstream
           then fails to synthesise, and the error names the *use site*, not the missing
           registration.
@guard     Catches the argument order. `add_instance(ty, value)` takes the **class type first**
           and the witness second (context.rs:184). Both are `Term`s, both are accepted without
           a check, and a swapped pair produces an instance database in which nothing ever
           unifies — search silently returns no match rather than erroring.
@guard     Catches ignoring insertion order. LOGOS instance resolution is **first match wins**
           over a `Vec` in registration order; Lean's is priority-ordered with a
           most-recently-declared tiebreak and backtracking. A translator that emits instances
           in source order reproduces Lean's *default* behaviour only when no priorities are
           used and no backtracking is needed. Any `instance (priority := …)` must be
           re-ordered into the `Vec` by hand, and any development relying on backtracking must
           be reported — see T07-008.
@guard     Catches emitting the instance as an *axiom*. A class method is a structure
           projection applied to the instance value (context.rs:310-362); with no body the
           method never reduces and `Add.add 1 2` is stuck.
@oracle    The class type appears in `instances()` paired with a value whose `infer_type` is a
           subtype of it; an instance-implicit argument of that class resolves; and the
           emitted order of `instances()` matches Lean's resolution order for this module.
-/

namespace T02_010

/-- A local class, so the case needs no Mathlib. -/
class Pointed (α : Type) where
  point : α

/-- The instance itself: a definition (`Pointed.mk 0`) **plus** a registration. -/
instance instPointedNat : Pointed Nat where
  point := 0

instance instPointedBool : Pointed Bool where
  point := true

/-- Resolution through an instance-implicit binder. -/
def getPoint (α : Type) [Pointed α] : α := Pointed.point

example : getPoint Nat = 0 := rfl
example : getPoint Bool = true := rfl

/-- A *parameterised* instance: its own type mentions an instance-implicit argument, so the
    emitted `(ty, value)` pair has an instance-implicit telescope and resolution is recursive.
    LOGOS searches the flat `Vec` again for the inner obligation. -/
instance instPointedProd (α β : Type) [Pointed α] [Pointed β] : Pointed (α × β) where
  point := (Pointed.point, Pointed.point)

example : getPoint (Nat × Bool) = (0, true) := rfl

/-- Two instances for overlapping types, declared in a specific order. Lean picks the
    most recently declared; LOGOS picks the **first** entry in the `Vec`. A translator that
    pushes in source order gets the opposite answer here. -/
class Tag (α : Type) where
  name : Nat

instance tagAny (α : Type) : Tag α where
  name := 0

instance tagNat : Tag Nat where
  name := 1

/-- Lean resolves this to `tagNat` (declared later, higher effective precedence for the more
    specific match). Registering `tagAny` first in the LOGOS `Vec` would resolve it to `0`. -/
example : (Tag.name (α := Nat)) = 1 := rfl

/-- An anonymous instance — Lean generates a name like `instPointedUnit`; LOGOS needs a
    concrete `String`, with the same uniqueness obligation as T02-005. -/
instance : Pointed Unit where
  point := ()

end T02_010
