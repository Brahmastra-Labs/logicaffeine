/-!
@case      T02-017
@tier      2 · Declarations
@title     `deriving Repr, BEq, …` — translate the handler's *output*, never the clause
@lean      deriving, deriving instance, `Lean.Elab.Deriving` handlers (Repr, BEq, Hashable,
           Inhabited, DecidableEq, Ord, ToString)
@target    for each generated declaration: Context::add_definition for the instance body
           plus Context::add_instance for the class registration; and a loss record for every
           class whose generated body is not translatable
@expect    degrade
@evidence  kernel/src/context.rs:20-100 (no `deriving` mechanism, no derive-handler registry,
           and no environment extension to hook one into); kernel/src/context.rs:184-186
           (`add_instance` — the only registration path for the generated instances);
           kernel/src/prelude.rs:34-35,923-1186 (the *only* decidable-equality artefacts in
           LOGOS are two hand-written definitions, `decEqBool` and `decEqNat`; there is no
           generic `DecidableEq` derivation — see T03-023);
           kernel/src/term.rs:13-44 (`Literal` has `Text(String)`, but LOGOS has **no string
           operations at all**, so a derived `Repr`/`ToString` body has no target — see
           T10-016)
@loss      DERIVE_CLAUSE. The `deriving` clause itself vanishes. Nothing in the LOGOS context
           records that an instance was generated rather than written, so a later re-derivation
           cannot be distinguished from a user instance.
@loss      UNTRANSLATABLE_HANDLERS. `Repr`, `ToString` and `Hashable` generate bodies over
           `Std.Format`/`String`/`UInt64`, none of which exist in LOGOS (`String` operations,
           `UInt*` and `Char` are all absent). Those derived instances must be dropped with a
           loss record, not approximated.
@guard     Catches translating the `deriving` *clause* — e.g. emitting some LOGOS marker that
           says "this type has BEq". There is no such marker. The only thing that exists after
           translation is whatever declarations Lean's handler actually produced, and those are
           ordinary definitions. The translator must read the post-elaboration environment.
@guard     Catches emitting the derived instance as a *definition* only, without the matching
           `add_instance`. The instance then exists but is unreachable by resolution, so every
           `[BEq α]` argument downstream fails to synthesise (T02-010).
@guard     Catches silently approximating `DecidableEq`. A derived `DecidableEq Foo` for a
           user inductive has a real, structural body that Lean generated; LOGOS has only
           `decEqBool` and `decEqNat` and no way to synthesise the rest. Mapping a derived
           `DecidableEq Foo` onto `decEqNat` because the fields happen to be `Nat` produces an
           instance that decides the wrong equality — it compares one field and ignores the
           constructor tag. See T03-023.
@guard     Catches translating the derived instance's *equations* as axioms when the body does
           not translate. That converts an unimplementable instance into an assumed one.
@oracle    Every declaration Lean's derive handlers added to the environment is either
           translated (definition + `add_instance`) or reported in the loss record by name and
           class; the `deriving` clause produces no LOGOS artefact of its own; and no derived
           instance is mapped onto a prelude definition for a different type.
-/

namespace T02_017

/-- Derivation on an enum. `BEq` and `DecidableEq` have structural bodies; `Repr` does not
    translate. -/
inductive Colour where
  | red
  | green
  | blue
  deriving Repr, BEq, DecidableEq, Inhabited

/-- The derived `BEq` is what makes this compute. -/
example : (Colour.red == Colour.red) = true := rfl
example : (Colour.red == Colour.blue) = false := rfl

/-- The derived `DecidableEq` — a *different* declaration from `BEq`, with a different type. -/
example : decide (Colour.green = Colour.green) = true := rfl

/-- The derived `Inhabited` picks the first constructor. -/
example : (default : Colour) = Colour.red := rfl

/-- Derivation on a structure with fields, so the generated body recurses into the field
    types' instances — the generated declaration depends on `instBEqNat`. -/
structure Pixel where
  x : Nat
  y : Nat
  hue : Colour
  deriving BEq, DecidableEq

example : (Pixel.mk 1 2 Colour.red == Pixel.mk 1 2 Colour.red) = true := rfl
example : (Pixel.mk 1 2 Colour.red == Pixel.mk 1 2 Colour.blue) = false := rfl

/-- The trap the `DecidableEq` guard names: an instance that compares *one* field and
    ignores the rest looks plausible and disagrees with the derived one only on inputs that
    differ outside that field. -/
example : (Pixel.mk 1 2 Colour.red == Pixel.mk 1 9 Colour.red) = false := rfl

/-- `deriving instance` as a standalone command — same handlers, different syntax, and a
    translator that only scans `deriving` clauses on declarations misses it. -/
inductive Shape where
  | dot
  | line

deriving instance BEq for Shape

example : (Shape.dot == Shape.line) = false := rfl

/-- A recursive inductive: the derived `BEq` body is itself a structural recursion, so the
    translated term must be a `Term::Fix` that passes the guard. -/
inductive Chain where
  | stop
  | link : Colour → Chain → Chain
  deriving BEq

example : (Chain.link Colour.red Chain.stop == Chain.link Colour.red Chain.stop) = true := rfl
example : (Chain.link Colour.red Chain.stop == Chain.stop) = false := rfl

end T02_017
