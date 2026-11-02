/-!
@case      T10-014
@tier      10 · Literals and primitive types
@title     `OfNat` overloading — the numeral's meaning is an instance, and LOGOS has no classes
@lean      OfNat, OfNat.ofNat, instOfNatNat, Neg / OfScientific, custom OfNat instance,
           @[default_instance]
@expect    degrade
@target    The literal must be resolved AT ELABORATION TIME to the value the instance produces,
           and that value emitted directly — `Term::Lit(Literal::Nat)` for `Nat`,
           `Term::Lit(int_lit)` for `Int`, and a fully applied translated constructor term for
           a user type. The `OfNat` class itself is NOT emitted.
@evidence  kernel/src/context.rs:184-189 (`add_instance` stores a flat `Vec<(Term, Term)>` and
           resolution is FIRST MATCH WINS — there is no priority, no `@[default_instance]`, no
           coherence check, and no `outParam`); kernel/src/context.rs:201
           (`set_binder_kinds` with `ParamKind::{Explicit, Implicit, Instance}` —
           elaborate.rs:1018-1027 — is the whole of LOGOS's instance-argument machinery, and it
           is a property of a NAME, not a class); kernel/src/term.rs:13-44 (the `Literal` enum
           has exactly one numeric-natural variant `Nat` and one integer family
           `Int`/`BigInt` — a numeral cannot be left polymorphic);
           kernel/src/type_checker.rs:296-304 (a literal's type is determined by its VARIANT,
           not by the context: `Literal::Nat` is `Nat`, full stop — there is no
           "numeral at whatever type is expected");
           kernel/src/prelude.rs:18-51 (no `OfNat` in the trusted base)
@loss      OVERLOADING_IS_RESOLVED_AWAY. Lean's `(5 : α)` is `@OfNat.ofNat α 5 inst`, and its
           meaning depends on `inst`. LOGOS literals are monomorphic by construction
           (type_checker.rs:296-304), so the translator must run Lean's instance resolution and
           emit the RESULT. What is lost is the indirection: a downstream Lean lemma stated
           about `OfNat.ofNat` generically, or a proof that rewrites through an `OfNat`
           instance equation, has no target in the output.
@loss      NO_DEFAULT_INSTANCE. `@[default_instance]` is what makes a bare `5` with no expected
           type default to `Nat`. LOGOS's `add_instance` is an insertion-ordered flat list with
           first-match-wins (context.rs:184-189) and no priority concept, so the defaulting
           decision cannot be represented at all; it must be made in Lean and baked in.
@guard     Catches emitting `OfNat.ofNat` as a global application. `@OfNat.ofNat Nat 5
           (instOfNatNat 5)` lowered structurally becomes
           `App(App(App(Global("OfNat_ofNat"), Global("Nat")), Lit(Nat(5))), Global("instOfNatNat"))`
           — three unbound globals and a literal buried as an ARGUMENT rather than the term
           itself. `infer_type` fails with `UnboundVariable("OfNat_ofNat")`, which reads like a
           missing-prelude-entry bug; the translator then "fixes" it with `add_declaration`, and
           now the literal `5` is an opaque axiom application that does not reduce, so
           `isZero 5` is stuck and `normalize` reports the stuck term as normal
           (reduction.rs:52-69).
@guard     Catches resolving the instance at the WRONG TYPE. `(5 : Int)` and `(5 : Nat)` are the
           same surface numeral and different LOGOS `Literal` variants with different types and
           different reduction mechanisms (T10-004). A translator that reads the numeral rather
           than the elaborated `Expr`'s type gets a term that typechecks — `Literal::Nat(5)` is
           a perfectly good term — but at the wrong type, and the error surfaces only where the
           two arithmetics diverge.
@guard     Catches a USER `OfNat` instance being silently dropped. If the source defines
           `instance : OfNat Mod7 n := ⟨Mod7.mk (n % 7)⟩`, then `(9 : Mod7)` is `Mod7.mk 2`, NOT
           `Mod7.mk 9`. A translator that emits the raw numeral into the user type produces a
           well-typed term with the wrong value. Note that the reduction here uses `%`, which
           on `Nat` truncates toward zero trivially but on `Int` follows LOGOS's
           dividend-signed remainder (T10-018) — so a user instance over `Int` can go wrong
           twice.
@guard     Catches assuming LOGOS instance resolution can stand in. `add_instance` takes a flat
           `(Term, Term)` pair and returns the FIRST match (context.rs:184-189), so two `OfNat`
           instances for one type silently resolve to whichever was registered first, with no
           diamond detection (compare T07-013). Instance selection must not be deferred to
           LOGOS.
@oracle    Every numeral in the output is a `Term::Lit` of the correct variant, or a fully
           applied translated constructor term, with NO occurrence of a global named
           `OfNat_ofNat` or `instOfNatNat`; and the loss record contains
           OVERLOADING_IS_RESOLVED_AWAY. A harness asserts that for each numeral the emitted
           term is `def_eq` to the translation of Lean's own `whnf` of the elaborated literal.
-/

namespace T10_014

/-- The default: a bare numeral with no expected type resolves to `Nat` via
    `@[default_instance] instOfNatNat`. -/
def bare := 5

example : bare = (5 : Nat) := rfl

/-- The same numeral at three types, three different LOGOS emissions. -/
def asNat : Nat := 5
def asInt : Int := 5
def asFloat : Float := 5

/-- A user type with its own `OfNat`, where the numeral is NOT the value. -/
structure Mod7 where
  rep : Nat
  deriving DecidableEq

instance (n : Nat) : OfNat Mod7 n := ⟨Mod7.mk (n % 7)⟩

/-- `(9 : Mod7)` is `Mod7.mk 2`. Emitting the raw numeral gives `Mod7.mk 9`, which typechecks
    and is a different value. -/
example : (9 : Mod7) = Mod7.mk 2 := rfl
example : (9 : Mod7) ≠ Mod7.mk 9 := by decide

/-- The elaborated form, written out, so the translator's target is unambiguous. -/
example : (5 : Nat) = @OfNat.ofNat Nat 5 (instOfNatNat 5) := rfl

/-- A `Neg` numeral: two instances compose, and only the composed VALUE may be emitted. -/
def negInt : Int := -5

/-- A polymorphic-in-the-instance function. Lean can state this; LOGOS has no class to
    quantify over, so this declaration must be rejected rather than monomorphised silently. -/
def twiceLit {α : Type} [OfNat α 2] [Add α] : α := (2 : α) + (2 : α)

example : (twiceLit : Nat) = 4 := rfl

end T10_014
