/-!
@case      T07-003
@tier      7 · Typeclasses
@title     Simple instance resolution — a linear scan of a flat vector, run AFTER the explicit
           arguments
@lean      `[C α]` binder resolved by unification on a ground goal, method call at a known type
@expect    translate
@target    A `ParamKind::Instance` binder whose metavariable is left as a deferred OBLIGATION
           during `elaborate_app_against`, then solved by `resolve_instance` once the explicit
           arguments (and the expected type) have pinned the class's type variables. The
           emitted term must be fully instantiated — no `?`-prefixed `Term::Var` may survive
           into the kernel.
@evidence  kernel/src/elaborate.rs:1016-1027 (`ParamKind::Instance`: "resolved from the
           `Context`'s instance database — but only AFTER the explicit arguments have been
           processed, so any metavariable in the class type (e.g. the `A` in `Inhabited A`) is
           solved first"); kernel/src/elaborate.rs:1044-1049 (`elaborate_app_against` — "The
           expected-type pass runs BEFORE instance resolution, so a class's type variable can be
           fixed by the context too"); kernel/src/elaborate.rs:1136-1153 (the obligation loop;
           failure is `KernelError::CertificationError("no typeclass instance found for …")`);
           kernel/src/elaborate.rs:1321-1341 (`resolve_instance_at`: `for (inst_ty, inst_val) in
           ctx.instances()` — a LINEAR SCAN in insertion order, each attempt on a CLONE of the
           metavariable context so failed trials never pollute it, first success wins);
           kernel/src/elaborate.rs:1346-1400 (`try_instance` freshens the instance's parameter
           telescope to metavariables and unifies its CONCLUSION with the goal);
           kernel/src/elaborate.rs:138 (elaborator metavariables are `Term::Var` names beginning
           with `?` — never a `Term` variant, and they must be fully instantiated before the
           kernel sees the term)
@guard     Catches emitting the instance argument as an unsolved metavariable. LOGOS's
           metavariables are `Term::Var("?0")` (elaborate.rs:138), which is a perfectly ordinary
           `Var` to the kernel — so `infer_type` reports `UnboundVariable("?0")` at best, and at
           worst the name collides with a binder and is CAPTURED by `substitute_avoiding`
           (type_checker.rs:1088), silently changing the term. Run `instantiate` and assert no
           `?`-prefixed variable remains.
@guard     Catches resolving the instance BEFORE the explicit arguments. The class's type
           variable is usually pinned only by an explicit argument (`Sized.size a` pins `α` from
           `a`'s type), so an eager resolver sees the goal `Sized ?0` and either fails or —
           much worse — matches the FIRST entry in the vector by unifying `?0` against that
           entry's carrier. `earlyResolution` below is the discriminating case: the first
           registered instance is at `Bool` and the call is at `Nat`.
@guard     Catches resolving from the EXPECTED TYPE only. `elaborate_app_against` runs the
           expected-type unification before instance resolution (elaborate.rs:1044-1049), which
           is how `Pointed.point` at an ascribed type resolves with no explicit argument at all.
           A translator that only ever pins from arguments fails `fromExpectedType` below.
@guard     Catches assuming resolution is CACHED or CANONICAL. It is neither: the scan runs
           afresh at every use site (elaborate.rs:1331) and is O(|instances|) each time, with a
           full `unify` per candidate. A development with hundreds of instances pays that on
           every method call, and because `try_instance` clones the whole `MetaCtx` per
           candidate (elaborate.rs:1332) the cost is in allocations, not just comparisons.
@guard     Catches treating a resolution FAILURE as recoverable. `elaborate_app_against` returns
           `Err(CertificationError("no typeclass instance found for …"))`
           (elaborate.rs:1147-1152); there is no backtracking into an alternative elaboration of
           the explicit arguments. A translator must therefore commit to one elaboration and
           report, not retry.
@oracle    Every emitted term is `?`-free and typechecks; for each method call the resolved
           instance is the one Lean chose (checked against `#print`/`whnf` of the Lean term);
           and `earlyResolution` resolves to `sizedNat`, not to the first vector entry.
-/

namespace T07_003

class Sized (α : Type) where
  size : α → Nat

/-- Registered FIRST, so it is the first entry in LOGOS's flat instance vector. -/
instance sizedBool : Sized Bool := ⟨fun b => if b then 1 else 0⟩

/-- Registered SECOND. -/
instance sizedNat : Sized Nat := ⟨fun n => n⟩

/-- THE DISCRIMINATING CASE: the class's type variable is pinned only by the explicit argument
    `a`. An eager resolver would take `sizedBool` (the first vector entry) by unifying the
    goal's metavariable against `Bool`. -/
def earlyResolution (n : Nat) : Nat := Sized.size n

example : earlyResolution 7 = 7 := rfl

/-- The instance is pinned by an ENCLOSING instance-implicit binder, not by a concrete type.
    The binder's class type must itself be a resolution premise. -/
def generic (α : Type) [Sized α] (a : α) : Nat := Sized.size a

example : generic Bool true = 1 := rfl

class Pointed (α : Type) where
  point : α

instance : Pointed Nat := ⟨0⟩
instance : Pointed Bool := ⟨true⟩

/-- Resolution driven ONLY by the expected type — there is no explicit argument to pin `α`.
    This is the `elaborate_app_against` path. -/
def fromExpectedType : Bool := Pointed.point

example : fromExpectedType = true := rfl

/-- Two instance premises in one signature, resolved independently. -/
def twoPremises (α β : Type) [Sized α] [Pointed β] (a : α) : Nat × β :=
  (Sized.size a, Pointed.point)

example : twoPremises Nat Bool 3 = (3, true) := rfl

/-- A method call nested inside another method's argument, so the obligation loop runs at two
    depths in one elaboration. -/
def nested (α : Type) [Sized α] [Pointed α] : Nat := Sized.size (Pointed.point : α)

example : nested Nat = 0 := rfl

end T07_003
