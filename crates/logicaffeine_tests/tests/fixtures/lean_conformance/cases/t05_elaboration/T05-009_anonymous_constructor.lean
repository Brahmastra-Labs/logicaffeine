/-!
@case      T05-009
@tier      5 · Elaboration surface
@title     Anonymous constructor `⟨…⟩` — resolved to a named constructor before translation
@lean      ⟨a, b⟩ anonymous constructor notation, nested and right-associated ⟨…⟩
@expect    translate
@target    A fully-applied `Term::App` spine headed by the resolved constructor's
           `Term::Global` — `⟨1, 2⟩ : Point` becomes `App(App(Global("Point_mk"), Lit(Nat 1)),
           Lit(Nat 2))`, with the structure's PARAMETERS supplied first.
@evidence  kernel/src/context.rs:271-369 (`add_structure(name, params, fields)` generates the
           inductive, ONE constructor named `{name}_mk`, and one projection definition per
           field — the anonymous-constructor notation resolves to that `{name}_mk`);
           kernel/src/context.rs:419 (`add_constructor(name, inductive, ty)` — constructors are
           ordinary flat `String` globals);
           kernel/src/term.rs:453 and :478 (`Global(String)` plus left-nested binary `App` is
           the whole of application — there is no anonymous-constructor term);
           kernel/src/elaborate.rs:1029-1080 (`elaborate_app`/`elaborate_app_against` — the
           expected-type pass is what lets a constructor's implicit parameters be solved from
           the surrounding type, which is exactly what `⟨…⟩` relies on);
           kernel/src/type_checker.rs:144-170 (the `App` rule; a spine of the wrong length
           fails only if the types disagree)
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax`.** `⟨…⟩` has no
           meaning until the expected type is known: the same `⟨1, 2⟩` is a `Point`, a
           `Prod Nat Nat`, an `And`, or a `Subtype` depending on context. Lean has resolved it
           by the time the `Expr` exists. A translator on `Syntax` must re-run
           expected-type-driven resolution, and since LOGOS's `Prod`, `Sigma`, `Subtype` and
           `Option` are **absent from the prelude** (LOGOS_FACTS, "Absent entirely") it would
           have to invent them too.
@guard     Catches the RIGHT-ASSOCIATION rule. Lean flattens `⟨a, b, c⟩` for a 3-field
           structure but nests it as `⟨a, ⟨b, c⟩⟩` for a 2-field one whose second field is
           itself a structure. For `Triple` and `NestedPair` below the source text is
           identical and the elaborated terms have different shapes. A translator that always
           flattens emits `Nested_mk a b c` against a 2-argument constructor; a translator that
           always nests emits `Triple_mk a (?_ b c)`. Both are arity errors — except when the
           field types line up, in which case one of them typechecks and is wrong.
@guard     Catches omitting the structure's PARAMETERS. `add_structure`'s field projections
           take the parameters before the receiver (context.rs:310-362, LOGOS_FACTS), and the
           constructor likewise takes them first. `⟨1, true⟩ : Pair Nat Bool` must become
           `Pair_mk Nat Bool 1 true`, not `Pair_mk 1 true`. Since `Nat` is a `Type 0` and the
           first field's type is the parameter, a two-argument spine can typecheck against a
           four-argument constructor's prefix and yield a partially-applied value.
@guard     Catches `⟨…⟩` for a `Prop`. `⟨h1, h2⟩ : A ∧ B` resolves to the LOGOS prelude's
           `conj`, and `⟨w, hw⟩ : ∃ x, P x` to `witness` — both present, but under LOGOS's own
           names (LOGOS_FACTS prelude list: `And`/`conj`, `Ex`/`witness`). Emitting
           `Global("And.intro")` or `Global("Exists.intro")` is an unresolvable name.
@guard     Catches `⟨…⟩` for a type with no LOGOS counterpart. `⟨1, true⟩ : Nat × Bool` and
           `⟨n, h⟩ : { x // P x }` resolve to `Prod.mk` and `Subtype.mk`, and neither `Prod`
           nor `Subtype` is in the LOGOS prelude — the translator must declare them (T10-013)
           or reject, never emit a dangling `Global`.
@guard     Catches `⟨…⟩` with a trailing `..` (T05-016), which supplies `_` — not defaults —
           for the remaining fields; the emitted spine must still be full-arity and must
           contain no `Term::Hole`.
@oracle    The emitted term is a fully-applied `App` spine headed by the resolved constructor's
           `Global`, with the structure's parameters first; it typechecks; it is term-for-term
           identical to the translation of the same value written with the explicit constructor
           name; and `normalize` of each projection agrees with Lean.
-/

namespace T05_009

structure Point where
  x : Nat
  y : Nat

/-- `⟨…⟩` and the named constructor elaborate to the same `Expr`. -/
example : (⟨1, 2⟩ : Point) = Point.mk 1 2 := rfl

structure Pair (α β : Type) where
  fst : α
  snd : β

/-- Parameterised: the emitted constructor spine takes `α` and `β` first. -/
example : (⟨1, true⟩ : Pair Nat Bool) = Pair.mk 1 true := rfl

structure Triple where
  a : Nat
  b : Nat
  c : Nat

/-- Three fields, flattened. -/
example : (⟨1, 2, 3⟩ : Triple) = Triple.mk 1 2 3 := rfl

structure NestedPair where
  head : Nat
  rest : Point

/-- Three components, but RIGHT-ASSOCIATED into a two-field constructor: the same surface
    text as `Triple`, a different elaborated shape. -/
example : (⟨1, 2, 3⟩ : NestedPair) = NestedPair.mk 1 (Point.mk 2 3) := rfl

/-- `⟨…⟩` for `And` — the LOGOS prelude's `conj`. -/
example (h1 : True) (h2 : True) : True ∧ True := ⟨h1, h2⟩

/-- `⟨…⟩` for `Exists` — the LOGOS prelude's `witness`. -/
example : ∃ n : Nat, n = 0 := ⟨0, rfl⟩

/-- `⟨…⟩` for `Prod` and `Subtype`, neither of which is in the LOGOS prelude. -/
example : (⟨1, true⟩ : Nat × Bool) = (1, true) := rfl
example : (⟨2, by decide⟩ : { n : Nat // n > 1 }).val = 2 := rfl

/-- `⟨…⟩` whose implicit parameters are solved only from the EXPECTED type — nothing in the
    components pins `β`. -/
def emptyPair : Pair Nat (List Bool) := ⟨0, []⟩

/-- Nested anonymous constructors written explicitly. -/
example : (⟨⟨1, 2⟩, ⟨3, 4⟩⟩ : Pair Point Point) = Pair.mk (Point.mk 1 2) (Point.mk 3 4) := rfl

example : emptyPair.snd = [] := rfl

end T05_009
