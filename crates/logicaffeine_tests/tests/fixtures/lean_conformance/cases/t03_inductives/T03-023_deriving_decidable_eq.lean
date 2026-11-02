/-!
@case      T03-023
@tier      3 · Inductives and structures
@title     `deriving DecidableEq` — LOGOS has exactly two decidable equalities, both
           hand-written
@lean      deriving DecidableEq, `Lean.Elab.Deriving.DecEq`, `decide`, `DecidableEq α`,
           `instDecidableEqNat`
@target    the *generated* Lean declaration, translated as an ordinary recursive definition
           (`Term::Fix` + nested `Term::Match`) of type
           `Π(a b : T). Decidable (Eq T a b)`, registered with Context::add_definition and
           Context::add_instance; plus a loss record for every type where that construction is
           not available
@expect    degrade
@evidence  kernel/src/prelude.rs:34-35 (the prelude registers exactly two:
           `register_dec_eq_bool` and `register_dec_eq_nat`);
           kernel/src/prelude.rs:923-1020 (`decEqBool : Π(a b:Bool). Decidable (Eq Bool a b)`,
           written out by hand); kernel/src/prelude.rs:1023-1186 (`decEqNat`, a hand-written
           structural recursion on `a`);
           kernel/src/prelude.rs:781 (`Decidable` is an *indexed* inductive,
           `add_indexed_inductive("Decidable", Π(p:Prop). Type 0, 1)`, with constructors
           `isTrue`/`isFalse`); kernel/src/prelude.rs:798-812 (`decide : Π(p:Prop).
           Decidable p → Bool`, defined through `Decidable_rec`);
           kernel/src/context.rs:20-100 (no derive-handler registry — see T02-017);
           kernel/src/type_checker.rs:253-266 and :631-638 (the generated body is a nest of
           `Match`es, each needing one case per constructor with no wildcard, so the derived
           term's size is quadratic in the constructor count)
@loss      NO_DERIVATION_MECHANISM. LOGOS cannot generate a `DecidableEq`. Whatever the
           translator emits is a *transcription* of Lean's generated term, and it must be
           re-emitted whenever the inductive changes.
@loss      TERM_SIZE. Lean's generated decision procedure uses nested pattern matching with
           wildcards; LOGOS's `Match` has no patterns and no wildcard
           (type_checker.rs:253-266), so an n-constructor type needs n² explicit branches.
           Against the fixed 10000-step `normalize` fuel (reduction.rs:52-69) and
           O(size)-per-binder substitution (type_checker.rs:1088), `decide` on a
           moderately-sized enum can exhaust the fuel and **silently return a partially
           reduced term**.
@loss      DISJOINTNESS_DEPENDENCY. Every "different constructors ⇒ not equal" branch must
           produce a proof of `Eq T a b → False`. That is constructor disjointness, which LOGOS
           does not generate (T03-018); it has to be built by large elimination through a
           discriminating `Match`, which is available for `Type 0` inductives and blocked for
           non-subsingleton `Prop`s (type_checker.rs:277-290).
@guard     **Catches mapping a derived `DecidableEq Foo` onto `decEqNat` or `decEqBool`.** This
           is the concrete disaster: an enum whose payload happens to be `Nat`, or a structure
           with a single `Nat` field, "obviously" decides by `decEqNat`. The resulting instance
           compares one field and ignores the constructor tag — so `decide (Foo.a 1 = Foo.b 1)`
           returns `true`. Because `decide`'s result is a `Bool` and the instance's *type*
           still says `Decidable (Eq Foo a b)`, nothing type-errors; every proof by `decide`
           downstream is then a proof of a false statement. `decEqNat` decides equality of
           `Nat` and of nothing else.
@guard     Catches emitting the instance as an **axiom**. `add_declaration("decEqFoo", Π(a b).
           Decidable (Eq Foo a b))` typechecks everything and computes nothing: `decide` is
           defined through `Decidable_rec` (prelude.rs:798-812), so an opaque instance leaves
           every `decide` stuck, and every `by decide` proof in the source fails to reduce.
@guard     Catches forgetting the `add_instance` registration. The definition alone is
           unreachable by resolution (T02-010), so `[DecidableEq α]` obligations fail to
           synthesise.
@guard     Catches `Decidable`'s params/indices split. The prelude registers it with
           `num_params = 1` (prelude.rs:781) over the arity `Π(p : Prop). Type 0` — so there
           are zero indices and `p` is a parameter. A translator that re-registers `Decidable`
           (rather than mapping onto the prelude's) with the `add_inductive` default gets the
           same count by luck; one that decides `p` is an index gets `num_params = 0` and every
           `Match` on a `Decidable` takes the indexed path with a motive of the wrong arity.
@guard     Catches deriving `DecidableEq` for a type whose fields are not themselves decidable
           in LOGOS — a field of type `Int` (opaque, prelude.rs:92-340), `Text`, `Float`
           (completely inert), or a function type. Lean may still derive it (or refuse); LOGOS
           has no decision procedure to call, and the instance cannot be built at all.
@oracle    Each derived `DecidableEq` becomes a LOGOS *definition* of type
           `Π(a b : T). Decidable (Eq T a b)` plus a matching `add_instance`; `decide (a = b)`
           reduces to `true` exactly when `a` and `b` are the same constructor applied to
           pointwise-equal arguments; no emitted instance body mentions `decEqNat` or
           `decEqBool` for a type other than `Nat`/`Bool`; and the loss report contains
           NO_DERIVATION_MECHANISM.
-/

namespace T03_023

/-- The two types LOGOS can already decide. Everything else must be transcribed. -/
example : decide ((3 : Nat) = 3) = true := rfl
example : decide ((3 : Nat) = 4) = false := rfl
example : decide (true = true) = true := rfl

/-- An enum. Three constructors ⇒ nine branches in a wildcard-free encoding. -/
inductive Colour where
  | red
  | green
  | blue
  deriving DecidableEq

example : decide (Colour.red = Colour.red) = true := rfl
example : decide (Colour.red = Colour.green) = false := rfl
example : decide (Colour.blue = Colour.green) = false := rfl

/-- **The `decEqNat` trap.** Both constructors carry a `Nat`, so a translator that "notices the
    payload is `Nat`" and reuses `decEqNat` produces an instance for which
    `Tagged.left 1 = Tagged.right 1` decides `true`. The last example is the one that catches
    it. -/
inductive Tagged where
  | left : Nat → Tagged
  | right : Nat → Tagged
  deriving DecidableEq

example : decide (Tagged.left 1 = Tagged.left 1) = true := rfl
example : decide (Tagged.left 1 = Tagged.left 2) = false := rfl
example : decide (Tagged.left 1 = Tagged.right 1) = false := rfl

/-- The same trap for a **structure**: one `Nat` field plus one `Bool` field. Reusing
    `decEqNat` decides on `n` alone. -/
structure Cell where
  n : Nat
  flag : Bool
  deriving DecidableEq

example : decide (Cell.mk 1 true = Cell.mk 1 true) = true := rfl
example : decide (Cell.mk 1 true = Cell.mk 1 false) = false := rfl

/-- A **recursive** type: the derived decision procedure is itself a structural recursion, so
    the translated term is a `Term::Fix` that must pass the Giménez guard. -/
inductive Chain where
  | stop : Chain
  | link : Colour → Chain → Chain
  deriving DecidableEq

example : decide (Chain.stop = Chain.stop) = true := rfl
example : decide (Chain.link Colour.red Chain.stop = Chain.link Colour.red Chain.stop) = true := rfl
example : decide (Chain.link Colour.red Chain.stop = Chain.link Colour.blue Chain.stop) = false := rfl
example : decide (Chain.link Colour.red Chain.stop = Chain.stop) = false := rfl

/-- Decidability used in a **proof**, not just a computation — this is where an opaque
    instance stops working, because `decide`'s reduction goes through `Decidable_rec`. -/
example : Colour.red ≠ Colour.blue := by decide

/-- Decidability of an **indexed** type. The generated instance must handle the index, and
    `Decidable` itself is indexed with `num_params = 1`. -/
inductive Small : Bool → Type where
  | yes : Small true
  | no : Small false

instance : DecidableEq (Small true) := fun a b =>
  match a, b with
  | Small.yes, Small.yes => isTrue rfl

example : decide ((Small.yes : Small true) = Small.yes) = true := rfl

/-- A field type LOGOS **cannot** decide: `Int` is opaque in LOGOS with only axiomatised
    ring/order laws (prelude.rs:92-340), so no structural decision procedure exists for it even
    though Lean derives one here. This is the "no procedure to call" loss. -/
structure WithInt where
  v : Int
  deriving DecidableEq

example : decide (WithInt.mk 3 = WithInt.mk 3) = true := rfl
example : decide (WithInt.mk 3 = WithInt.mk (-3)) = false := rfl

end T03_023
