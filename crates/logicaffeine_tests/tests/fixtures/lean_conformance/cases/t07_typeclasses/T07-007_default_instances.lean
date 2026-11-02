/-!
@case      T07-007
@tier      7 · Typeclasses
@title     `@[default_instance]` — no fallback tier; LOGOS resolution either matches or fails
@lean      @[default_instance], instOfNatNat, instHAdd, numeric literal defaulting,
           `default_instance` priority tiers
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/context.rs:71,184-186 (`instances: Arc<Vec<(Term, Term)>>` — a single flat
           list with no tier, no priority field, and no notion of "try this only if everything
           else failed"); kernel/src/elaborate.rs:1321-1341 (`resolve_instance_at` — ONE linear
           pass over `ctx.instances()`; it returns at the first success and returns `None` if the
           pass completes with no match. There is no second pass, no fallback list, and no
           re-entry); kernel/src/elaborate.rs:1136-1153 (an unresolved obligation is a hard
           `KernelError::CertificationError("no typeclass instance found for …")` — there is no
           "postpone and default later" path); kernel/src/elaborate.rs:1044-1049
           (`elaborate_app_against`: the expected-type unification runs BEFORE instance
           resolution, so by the time resolution runs the metavariables are either solved or
           permanently stuck); kernel/src/term.rs:13-44 and kernel/src/reduction.rs:330-345
           (`Literal::Nat(BigInt)` is a KERNEL literal, definitionally `Succ^n Zero` via the
           `extract_constructor` bridge — LOGOS numerals do not go through a class at all, so
           there is no `OfNat` for a default instance to be the default OF)
@diag      E_NO_DEFAULT_INSTANCE — must name the class and the defaulted metavariable, and state
           that LOGOS resolution has a single tier. Must NOT pick a default itself.
@guard     THE TRAP: translating `@[default_instance]` as an ordinary instance and relying on
           position. It is not equivalent in EITHER direction, and both directions break
           silently:
             • Emitted FIRST, the default instance captures every goal of its class, including
               goals a specific instance should have handled, because `try_instance` unifies its
               conclusion's metavariables against the goal (elaborate.rs:1373) and a default
               instance's conclusion is maximally general by design. Lean would have used the
               specific instance; LOGOS uses the default. The program still elaborates.
             • Emitted LAST, the default instance is only reached when nothing else matches —
               which looks right, but Lean's defaulting fires on a metavariable that is still
               UNSOLVED at the end of elaboration, whereas LOGOS's resolution runs at the point
               of the application with whatever is known then (elaborate.rs:1136-1140). A goal
               that Lean would default, LOGOS resolves eagerly against an earlier instance the
               moment enough of the type is known — or fails outright if it is not.
@guard     Catches the numeral-literal special case, which is where this bites hardest.
           `(2 : α)` in Lean is `@OfNat.ofNat α 2 inst`, and `instOfNatNat` is a
           `@[default_instance]` that makes a bare `2` a `Nat`. LOGOS has no `OfNat`: numerals
           are `Term::Lit(Literal::Nat(BigInt))` (term.rs:13-44), definitionally `Succ^n Zero`
           (reduction.rs:330-345, type_checker.rs:1480-1493). So every Lean numeral must be
           translated to a LOGOS `Nat` literal — and a numeral the Lean source intended as an
           `Int`, a `Float`, or a user type is then silently a `Nat`. `Int` in LOGOS is a
           completely different kind of thing: an OPAQUE type with axiomatised ring laws
           (prelude.rs:92-191), not an inductive, so `Nat` and `Int` literals are not
           interchangeable. See T10-004.
@guard     Catches `instHAdd`. Lean's `a + b` is `HAdd.hAdd` resolved through the
           `@[default_instance] instHAdd : HAdd α α α` fallback, which ALSO depends on
           `outParam` (T07-006). A translator that handles neither will see `+` as an
           unresolvable class method in essentially every Lean file; one that special-cases `+`
           to `Nat.add` has hard-coded a defaulting decision that the source did not make and
           that is wrong for every non-`Nat` carrier.
@guard     Catches emitting the default instance and ALSO the specific ones and calling the
           order a "policy". It is not a policy, it is a semantic change, and it must be
           reported. If a translator chooses to proceed anyway it has switched this case's
           verdict to `degrade` and owes a loss record naming the defaulted position and the
           instance it picked — but INDEX.md marks this `reject`, and this file follows that.
@oracle    Translator exits non-zero with E_NO_DEFAULT_INSTANCE for every declaration below
           whose elaboration involves a `@[default_instance]`, and emits no `add_instance` for
           the defaulted class. A harness additionally checks that no emitted instance vector
           contains an entry whose conclusion is a bare `App(Global(C), Var("?…"))`.
-/

namespace T07_007

/-- A class with a specific instance and a general fallback, mirroring `OfNat`'s shape. -/
class Zeroish (α : Type) where
  zero : α

instance zeroishBool : Zeroish Bool := ⟨false⟩

/-- The DEFAULT. In Lean this fires only when nothing else determines `α`. -/
@[default_instance]
instance zeroishNat : Zeroish Nat := ⟨0⟩

/-- The defaulting call: `α` is not determined by any argument or by the expected type, so Lean
    falls back to the default instance and `α := Nat`. LOGOS has no such step. -/
def defaulted := (Zeroish.zero, 1)

example : defaulted = ((0 : Nat), 1) := rfl

/-- The NON-defaulting call at the same class: the expected type pins `α := Bool`, so the
    specific instance must win. A translator that emitted the default first gets `Nat` here. -/
def notDefaulted : Bool := Zeroish.zero

example : notDefaulted = false := rfl

/-- `OfNat` — Lean's own defaulting, re-declared locally. `lit` is a bare numeral whose type is
    fixed only by `instOfNatNat` being a `@[default_instance]`. -/
def lit := 42

example : lit = (42 : Nat) := rfl

/-- The same numeral at a DIFFERENT carrier. In LOGOS `Nat` is inductive and `Int` is an opaque
    axiomatised type (prelude.rs:92-191), so these two literals are not interchangeable. -/
def intLit : Int := 42

/-- A user type with its own `OfNat`, so the numeral does not default to `Nat` at all. -/
structure Money where
  cents : Nat

instance : OfNat Money n := ⟨⟨n⟩⟩

def price : Money := 500

example : price.cents = 500 := rfl

/-- `+` through the `instHAdd` default instance, which also needs `outParam` (T07-006). -/
def plus (a b : Nat) : Nat := a + b

end T07_007
