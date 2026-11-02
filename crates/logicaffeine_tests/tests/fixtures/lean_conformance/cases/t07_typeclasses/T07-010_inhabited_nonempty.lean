/-!
@case      T07-010
@tier      7 · Typeclasses
@title     `Inhabited` / `Nonempty` — neither is in the LOGOS prelude; both must be emitted
@lean      Inhabited, default, Inhabited.mk, Nonempty, Nonempty.intro, Nonempty.elim,
           `opaque` declarations' inhabitedness obligation
@expect    translate
@target    `Inhabited α` ⟼ `add_structure("Inhabited", &[("a", Sort(Type 0))],
                              &[("default", Var("a"))])`, which yields the inductive
           `Inhabited : Type 0 → Type 0`, the constructor `Inhabited_mk : Π(a:Type 0). a →
           Inhabited a`, and the projection definition
           `Inhabited_default : Π(a:Type 0). Inhabited a → a`. Then one `add_definition` +
           `add_instance` per instance.
           `Nonempty α` ⟼ `add_inductive("Nonempty", Π(a:Type 0). Prop)` +
           `add_constructor("Nonempty_intro", "Nonempty", Π(a:Type 0). a → Nonempty a)` —
           it must be an INDUCTIVE, not a structure, because `add_structure` hard-codes
           `Type 0` and `Nonempty` is a `Prop`.
@evidence  a grep of `crates/logicaffeine_kernel/src` for `Inhabited` or `Nonempty` finds them
           ONLY in doc comments (context.rs:69,182; elaborate.rs:1025,1159,1162,1175,1193) as
           worked examples of how the instance database is meant to be used — NEITHER is
           registered by `StandardLibrary::register` (prelude.rs:33-44);
           kernel/src/context.rs:271-369 (`add_structure`, sort hard-coded `Type 0` at :300 —
           hence `Nonempty` cannot go through it); kernel/src/context.rs:253
           (`add_inductive(name, sort)` — the `Prop`-capable path, but UNCHECKED);
           kernel/src/context.rs:545 (`add_constructor_checked` — the positivity- and
           universe-checking path, which is what a translator should use);
           kernel/src/type_checker.rs:1688-1716 (`is_subsingleton_prop`: `Nonempty` has ONE
           constructor whose non-arity argument `a : Type 0` is NOT a `Prop`, so `Nonempty` is
           NOT a subsingleton) and kernel/src/recursor.rs:64-71 (its derived eliminator's motive
           is therefore pinned to `Sort(Prop)` — no large elimination, which is correct: large
           elimination from `Nonempty` IS choice, see T06-009)
@guard     Catches assuming either class is pre-registered because the LOGOS source keeps naming
           them. `context.rs:182` literally says "a `value` of type `ty` (e.g. `mk Nat Zero` of
           type `Inhabited Nat`)" and `elaborate.rs:1193` says "`instance {A} [Inhabited A] :
           Inhabited (List A)`" — these are DOC COMMENTS illustrating the API, not
           registrations. `Global("Inhabited")` resolves to nothing (context.rs:449). A
           translator that pattern-matches the LOGOS source for familiar names will get this
           wrong for `Inhabited`, `Nonempty`, `List`, `Subtype` and `Setoid` alike.
@guard     Catches emitting `Nonempty` via `add_structure`. The sort is hard-coded to
           `Term::Sort(Universe::Type(0))` (context.rs:300), so the emitted `Nonempty α` would
           be a `Type 0`, not a `Prop`. That is not a cosmetic difference: a `Type 0`-valued
           `Nonempty` in a proposition's statement changes that statement's sort through `imax`
           (type_checker.rs:108), silently promoting a `Prop` to a `Type 0`, and — much worse —
           makes `Nonempty` eligible for ordinary `Match` elimination into `Type`, which is
           exactly `Classical.choice` (T06-009) written by hand.
@guard     Catches emitting `Inhabited` via `add_inductive` + hand-rolled projection. Only
           `add_structure` registers the `StructInfo` that `try_structure_eta` consults
           (context.rs:153-167, type_checker.rs:1391-1417 gated at :1473-1478). Without it,
           `Inhabited_mk α (Inhabited_default α i) ≡ i` stops holding, and any argument that
           reconstructs a dictionary — which is what T07-005's forget-instances do — fails.
@guard     Catches using `add_inductive`/`add_constructor` (both UNCHECKED, context.rs:253,419)
           rather than `add_constructor_checked` (context.rs:545) for `Nonempty`. The unchecked
           path performs no positivity and no universe check, so a malformed emission registers
           silently. See T11-013.
@guard     Catches the `default` name. `add_structure` names projections
           `format!("{name}_{field}")` (context.rs:310-362), so Lean's `Inhabited.default` is
           `Inhabited_default` and takes the PARAMETER first: `Inhabited_default α i`, not
           `Inhabited_default i`. `elaborate_dot` (elaborate.rs:1267-1299) looks up exactly that
           spelling.
@guard     Catches the `opaque` obligation. Lean's `opaque c : α` requires `Inhabited α` (or a
           `Nonempty` proof) to guarantee the type is inhabited before it will accept the
           declaration. LOGOS's `add_declaration` (context.rs:434) requires NOTHING — it takes a
           type and nothing else. So a translator that drops the obligation can declare an
           opaque constant of an EMPTY type, which is a proof of `False` if that type is `False`.
           `opaqueThing` below carries the obligation; the translator must not lose it. See
           T02-006.
@oracle    Both classes are emitted before any use; `Inhabited` via `add_structure` and
           `Nonempty` via `add_inductive` + `add_constructor_checked`; every instance is one
           `add_definition` plus one `add_instance`; projections are applied parameters-first;
           and the `opaque` declaration's inhabitedness witness is emitted alongside it.
-/

namespace T07_010

/-- Re-declared locally so the emission shape is explicit and the file needs no imports beyond
    core. -/
class MyInhabited (α : Type) where
  deflt : α

instance : MyInhabited Nat := ⟨0⟩
instance : MyInhabited Bool := ⟨false⟩

/-- The projection, which LOGOS spells `MyInhabited_deflt α i`. -/
def getDefault (α : Type) [MyInhabited α] : α := MyInhabited.deflt

example : getDefault Nat = 0 := rfl
example : getDefault Bool = false := rfl

/-- A RECURSIVE instance — the shape `elaborate.rs:1193`'s doc comment advertises. -/
inductive Box (α : Type) where
  | mk : α → Box α

instance {α : Type} [MyInhabited α] : MyInhabited (Box α) := ⟨Box.mk MyInhabited.deflt⟩

def boxedDefault : Box Nat := getDefault (Box Nat)

/-- `Nonempty` — a `Prop`, so it must be an `add_inductive`, not an `add_structure`. -/
inductive MyNonempty (α : Type) : Prop where
  | intro : α → MyNonempty α

/-- Introduction. -/
theorem natNonempty : MyNonempty Nat := MyNonempty.intro 0

/-- Elimination into a `Prop` motive — the ONLY motive LOGOS's derived eliminator allows,
    because `MyNonempty`'s constructor argument lives in `Type 0`, not `Prop`. -/
theorem nonemptyElim {α : Type} (h : MyNonempty α) (p : Prop) (k : α → p) : p := by
  cases h with
  | intro a => exact k a

/-- Inhabited ⇒ Nonempty, the sound direction. -/
theorem inhabitedNonempty (α : Type) [MyInhabited α] : MyNonempty α :=
  MyNonempty.intro MyInhabited.deflt

/-- Lean's own `Inhabited`/`Nonempty`, for the record — same shape, same absence in LOGOS. -/
example : Inhabited Nat := ⟨0⟩
example : Nonempty Nat := ⟨0⟩

/-- The `opaque` obligation: Lean demands inhabitedness; `add_declaration` does not. -/
opaque opaqueThing : Nat

end T07_010
