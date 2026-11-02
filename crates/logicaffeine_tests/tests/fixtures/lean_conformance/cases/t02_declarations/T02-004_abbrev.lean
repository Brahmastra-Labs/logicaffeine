/-!
@case      T02-004
@tier      2 · Declarations
@title     `abbrev` — the `@[reducible]` hint has no carrier in LOGOS
@lean      abbrev, @[reducible], ReducibilityStatus, TransparencyMode
@target    Context::add_definition(name, ty, body)  + a loss record
@expect    degrade
@evidence  kernel/src/context.rs:442-444 (`add_definition` is the only definition form; the
           stored triple is `name -> (ty, body)` with **no** reducibility field);
           kernel/src/context.rs:20-100 (the whole `Context` — there is no transparency map,
           no `ReducibilityStatus` analogue, and no `TransparencyMode` parameter anywhere);
           kernel/src/reduction.rs:52-69 (`normalize` unfolds whatever `get_definition_body`
           returns, with a single global fuel of 10000 and no per-constant policy)
@loss      REDUCIBILITY_HINT. Lean distinguishes `@[reducible]` (unfolded by `whnf` at
           `.instances` transparency, so typeclass search sees through it), `regular`, and
           `@[irreducible]` (never unfolded without `unfold`). LOGOS has exactly one
           behaviour: always transparent. The hint is dropped.
@loss      ELABORATION_BEHAVIOUR. Because `abbrev` is reducible, Lean's instance search and
           unifier see through it; a `def` with the same body would block them. After
           translation both collapse to the same thing, so a *Lean* development that relied on
           a `def` being opaque to instance search has no LOGOS counterpart — see T00-013.
@guard     Catches a translator that "preserves" `abbrev` by inlining its body at every use
           site. That is semantically right and operationally catastrophic: `normalize` has a
           hard fuel of 10000 steps and **silently returns the partially-reduced term** when it
           runs out (reduction.rs:52-69), so inlining a chain of abbreviations turns a
           successful conversion check into a silent failure that reports as a *type mismatch*
           on an unrelated term. Substitution is also O(size)-per-binder
           (type_checker.rs:1088), so inlining is quadratic.
@guard     Catches a translator that maps `abbrev` to `add_declaration` to "keep it abstract".
           Backwards: `abbrev` is the *most* transparent form in Lean.
@guard     Catches silently succeeding. Degrade means the loss record is mandatory: a
           conformance run that emits the right term and no `REDUCIBILITY_HINT` record fails.
@oracle    The emitted declaration is a definition with the abbreviation's elaborated body, and
           the loss report contains REDUCIBILITY_HINT naming this declaration.
-/

namespace T02_004

/-- The plain abbreviation. In Lean this is `@[reducible] def`. -/
abbrev NatPair := Nat × Nat

/-- A `def` with the *same* body. Lean keeps these distinguishable; LOGOS cannot. -/
def NatPairOpaque := Nat × Nat

/-- Reducibility is observable in Lean's elaborator: an instance on `Nat × Nat` is found for
    the `abbrev` because instance search unfolds reducible constants. -/
example : NatPair := (1, 2)

/-- The same literal against the `def` still elaborates here only because the ascription
    forces unification at default transparency — the point is that the two constants have
    *different* elaboration behaviour in Lean and identical behaviour after translation. -/
example : NatPairOpaque := (1, 2)

/-- Explicit attribute form: `@[reducible] def` is exactly what `abbrev` expands to. -/
@[reducible] def Flag := Bool

/-- The opposite end of the axis — also unrepresentable, also dropped. Included so the loss
    record can be checked to name *both* directions of the hint, not just `reducible`. -/
@[irreducible] def sealedZero : Nat := 0

/-- A chain of abbreviations: this is the shape that breaks a translator which "preserves
    reducibility" by inlining, because the inlined term grows multiplicatively. -/
abbrev A0 := Nat
abbrev A1 := A0 → A0
abbrev A2 := A1 → A1
abbrev A3 := A2 → A2
def useChain (f : A3) : A3 := f

end T02_004
