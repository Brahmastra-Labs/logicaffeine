/-!
@case      T11-003
@tier      11 · Adversarial and soundness traps
@title     Flat-name collision — the kernel TYPES a name from one map and REDUCES it from
           another, silently
@lean      hierarchical Name, namespace, `Foo.bar` vs a top-level `Foo_bar`, `_root_`,
           `export`, `open ... in`
@expect    reject
@target    (none — two distinct Lean declarations cannot share one LOGOS key)
@evidence  kernel/src/context.rs:449-464 (`get_global` — the lookup precedence, in order:
           `self.inductives` → `self.constructors` → `self.definitions` →
           `self.declarations`. FIRST MATCH WINS across four PARALLEL maps, all keyed by flat
           `String`); kernel/src/context.rs:442-444 (`add_definition` inserts into
           `self.definitions` with no check against the other maps — and, note, with no check
           that `body : ty` either); kernel/src/context.rs:253 (`add_inductive` inserts into
           `self.inductives`, likewise unchecked); kernel/src/context.rs:474
           (`get_definition_body` reads `self.definitions` DIRECTLY — it does not go through
           `get_global` and therefore does not respect its precedence);
           kernel/src/reduction.rs:305-314 (delta reduction: `Term::Global(name) => if let
           Some(body) = ctx.get_definition_body(name) { body.clone() } else { term.clone() }`
           — the reducer consults `definitions` ONLY); LOGOS_FACTS: LOGOS keys have no
           namespaces, no `Name` type, no visibility and no attributes
@diag      E_NAME_COLLISION_AFTER_FLATTENING — must name BOTH colliding Lean declarations,
           print the single LOGOS key they map to, say which of the four maps each would land
           in, and emit nothing. A mangling scheme that avoids the collision is acceptable ONLY
           if the translator proves it injective over the whole input.
@guard     **TIER 11 THESIS.** LOGOS's kernel accepting a term is NOT evidence that the
           translation is faithful. This case is the sharpest demonstration, because the kernel
           does not merely accept a wrong term — IT DISAGREES WITH ITSELF ABOUT ONE NAME.
           Concretely: a Lean inductive `Foo_bar` and a Lean definition `Foo.bar` both flatten
           to the key `"Foo_bar"`. `add_inductive` puts the first in `context.inductives`;
           `add_definition` puts the second in `context.definitions`. Nothing complains: the
           maps are parallel and neither insertion checks the other.
           Now the kernel splits in two.
             · TYPING goes through `get_global` (context.rs:449-464), which checks `inductives`
               FIRST — so `Foo_bar` has the INDUCTIVE's type.
             · REDUCTION goes through `get_definition_body` (context.rs:474), called directly
               from the delta rule (reduction.rs:305-314), which reads `definitions` ONLY — so
               `Foo_bar` unfolds to the DEFINITION's body.
           The same `Term::Global("Foo_bar")` therefore has one type and a completely unrelated
           value. `infer_type` succeeds. `normalize` succeeds. They are describing different
           objects. No error is raised at any point, and no amount of re-checking helps,
           because the inconsistency is in the environment, not in the term.
@guard     Catches the "namespaces are just sugar" assumption. In Lean, `Foo.bar` and `Foo_bar`
           are DIFFERENT names that can both exist in one file, and `_root_.Foo_bar` can exist
           alongside a `Foo.bar` in an open namespace. Flattening with `.` ↦ `_` is not
           injective. Neither is stripping the namespace: `List.map` and `Array.map` both
           become `map`.
@guard     Catches the collision that is invisible in the SOURCE. The two colliding declarations
           need not be in the same file, or written by the same person; the second may arrive
           from a dependency, or from the translator's own emitted support code (a `List_map`
           emitted for T10-012 colliding with a user's `List.map`). A translator must maintain a
           global injective key map, and must fail when it cannot extend it.
@guard     Catches colliding with the PRELUDE. `StandardLibrary::register` occupies bare keys
           including `Nat`, `Int`, `Bool`, `true`, `false`, `Eq`, `refl`, `And`, `Or`, `Ex`,
           `add`, `sub`, `mul`, `div`, `mod`, `le`, `lt` (prelude.rs:352-381, :529-552,
           :593-604, and throughout). A Lean file with a top-level `def add` flattens onto the
           prelude's `Int` addition axiom — which lands in `declarations`, the LAST map
           `get_global` checks (context.rs:463), so the user's definition WINS for typing while
           the primitive reduction hook in `try_primitive_reduce` (reduction.rs:444-506) still
           fires on the name `"add"` for literal operands. Typing and reduction disagree again,
           now against the trusted base.
@guard     Catches the mangling that merely moves the problem. Appending a hash or a counter is
           fine for soundness and terrible for certificates: `PRELUDE_VERSION` pins the trusted
           base (certificate.rs:28) but nothing pins the translator's mangling, so two runs of
           the translator can produce two different key sets for one Lean input and the
           certificates will not be comparable. See T11-015.
@oracle    Translator exits non-zero with E_NAME_COLLISION_AFTER_FLATTENING for the pairs below
           and emits nothing. A harness additionally asserts, over the whole corpus output,
           that (a) the map from Lean `Name` to LOGOS key is injective, and (b) no emitted key
           appears in more than one of `inductives` / `constructors` / `definitions` /
           `declarations`, and (c) no emitted key collides with a name registered by
           `StandardLibrary::register`.
-/

namespace T11_003

/-! ### The canonical collision: a namespaced declaration and a top-level one that flatten to
    the same key. -/

namespace Foo
/-- Flattens to `"Foo_bar"`. A DEFINITION — lands in `context.definitions`. -/
def bar : Nat := 7
end Foo

/-- Also flattens to `"Foo_bar"`. An INDUCTIVE — lands in `context.inductives`, which
    `get_global` checks FIRST (context.rs:452-454). -/
inductive Foo_bar : Type where
  | mk : Foo_bar

/-- Both exist, and Lean distinguishes them perfectly. -/
example : Foo.bar = 7 := rfl
example : Foo_bar := Foo_bar.mk

/-- The witness that they are different: one is a `Nat`, the other is a `Type`. After
    flattening, LOGOS types the key as the inductive (`Type 0`) and reduces it to `7`. -/
example : Foo.bar ≠ 8 := by decide

/-! ### The constructor variant: a constructor and a definition colliding. -/

namespace Baz
inductive Colour where
  | red
  | green
  deriving DecidableEq
end Baz

/-- Flattens to `"Baz_Colour_red"`; the constructor `Baz.Colour.red` flattens to the same
    key. `get_global` finds the CONSTRUCTOR first (context.rs:455-458) because
    `constructors` precedes `definitions`. -/
def Baz_Colour_red : Nat := 1

example : Baz_Colour_red = 1 := rfl
example : Baz.Colour.red ≠ Baz.Colour.green := by decide

/-! ### Collision with the LOGOS prelude's own bare keys. -/

/-- Flattens to `"add"`, which the prelude registers as the opaque `Int → Int → Int`
    arithmetic axiom (prelude.rs:377) in `declarations` — the LAST map `get_global` checks.
    A user `definitions` entry therefore wins for TYPING, while `try_primitive_reduce`
    (reduction.rs:444-506) still matches the literal name `"add"` during reduction. -/
def add (a b : Nat) : Nat := a * b + 1

example : add 2 3 = 7 := rfl

/-- Flattens to `"le"`, which the prelude registers as `Int → Int → Bool` (prelude.rs:379) and
    which the ENTIRE `Int` order theory is stated in terms of (`a ≤ b ≔ Eq Bool (le a b) true`,
    prelude.rs:202-340). -/
def le (a b : Nat) : Bool := decide (b < a)

example : le 5 2 = true := rfl

/-! ### Two different namespaces, one leaf name — the collision that crosses files. -/

namespace Alpha
def size : Nat := 1
end Alpha

namespace Beta
def size : Nat := 2
end Beta

/-- A translator that strips namespaces rather than flattening them collides these two. -/
example : Alpha.size ≠ Beta.size := by decide

/-! ### `_root_` escape, which makes the two spellings coexist inside one namespace. -/

namespace Gamma
def dup : Nat := 10
end Gamma

def Gamma_dup : Nat := 20

example : Gamma.dup ≠ Gamma_dup := by decide

end T11_003
