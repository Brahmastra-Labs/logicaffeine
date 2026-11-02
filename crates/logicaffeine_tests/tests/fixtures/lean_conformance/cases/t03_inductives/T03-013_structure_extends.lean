/-!
@case      T03-013
@tier      3 · Inductives and structures
@title     `structure … extends` — flattening vs. parent-as-field, and the lost subtyping
@lean      structure … extends, `Lean.StructureInfo.parentInfo`, `toParent` coercions,
           inherited field access, diamond inheritance
@target    Context::add_structure(child, params, parent_fields ++ own_fields) — a **flattened**
           record — plus an explicitly emitted `Child_toParent` definition and, if the source
           relies on it, Context::add_coercion; and a loss record
@expect    degrade
@evidence  kernel/src/context.rs:257-276 (`add_structure` takes a flat `fields: &[(&str, Term)]`
           slice; there is no parent list, no `StructureParentInfo` analogue, and no
           inheritance concept anywhere in `Context`);
           kernel/src/context.rs:117-127 (`StructInfo { mk, num_params, projections }` — three
           fields, none of which records a parent);
           kernel/src/context.rs:308-363 (projections are generated for exactly the fields
           passed in, named `{child}_{field}`, so an inherited field's projection is
           `Child_x`, a **different global** from `Parent_x`);
           kernel/src/elaborate.rs:1267-1299 (`elaborate_dot` builds `format!("{hname}_{field}")`
           from the *receiver's own type head* — so `c.x` on a child resolves to `Child_x`
           and never to `Parent_x`);
           kernel/src/context.rs:188-197 (`add_coercion` — a flat `Vec<(Term, Term, Term)>`,
           the only place a `Child → Parent` conversion can live);
           kernel/src/type_checker.rs:1330-1360 (`is_subtype_normalized` gives cumulativity for
           *sorts* and variance for `Pi` only — there is **no** structural or nominal subtyping
           between two inductives)
@loss      INHERITANCE_STRUCTURE. After flattening there is no parent. `Parent` and `Child` are
           unrelated LOGOS inductives; nothing records that one extends the other, and no
           later declaration can be checked against the relationship.
@loss      PARENT_COERCION_IS_NOT_SUBTYPING. Lean lets a `Child` be used where a `Parent` is
           expected by inserting `Child.toParent`. LOGOS has no subtyping between inductives,
           so every such use site must have the conversion **inserted explicitly** at
           translation time; `add_coercion` only helps the LOGOS *elaborator*, not the kernel,
           and it is searched first-match-wins over a flat list.
@loss      SHARED_FIELD_IDENTITY. A flattened `Child` has its own `Child_x` projection. A Lean
           lemma stated about `Parent.x` does not apply to `c.x` for `c : Child` after
           translation — the two are different globals over different types. Every inherited
           lemma must be re-stated or transported through `toParent`.
@loss      DIAMOND_COLLAPSE. When two parents share a field, Lean identifies the field once and
           records both parent projections. Flattening produces a single field and *two*
           distinct `toParent` conversions that must be proved to agree; nothing in LOGOS checks
           that they do (see T07-005, T07-013).
@guard     Catches the "parent-as-field" encoding: registering `Child` with a field
           `toParent : Parent` plus the child's own fields. It is closer to Lean's
           `parentInfo`, and it breaks inherited field access: `c.x` elaborates via
           `elaborate_dot` to `Child_x`, which in that encoding does not exist — so every
           inherited projection in the translated source fails to resolve. The translator must
           either flatten *and* synthesise `Child_x` definitions, or keep the parent field
           *and* synthesise `Child_x := Parent_x (Child_toParent c)` for every inherited field.
           Doing neither is the common bug.
@guard     Catches the field **order** after flattening. Lean places the parent's fields first,
           in parent order, then the child's own. `add_structure` builds the constructor's
           telescope and the `projections` list in the order it is given (context.rs:305,
           :308-312), and `Match` is positional, so a reordered flattening silently permutes
           the anonymous-constructor arguments `⟨…⟩` at every use site.
@guard     Catches forgetting that a `Prop`-valued or `Type 1`-valued parent drags the child
           out of `add_structure`'s reach entirely, because the sort is hard-coded
           (context.rs:299-301, T03-012).
@guard     Catches emitting `Child_toParent` as an axiom. It must be a definition — `Parent_mk`
           applied to the child's inherited projections — or the round-trip law
           `(Child_toParent c).x = c.x` is unprovable.
@oracle    `Child` is a registered structure whose `projections` list begins with the parent's
           field names in parent order; `Child_toParent` is a *definition* satisfying
           `Parent_x (Child_toParent c) ≡ Child_x c` for every inherited field; and the loss
           report contains INHERITANCE_STRUCTURE.
-/

namespace T03_013

structure Base where
  x : Nat
  y : Nat

/-- Single inheritance. Lean's flattened `Extended` has fields `x`, `y`, `z` in that order,
    plus a generated `Extended.toBase`. -/
structure Extended extends Base where
  z : Nat

def e : Extended := { x := 1, y := 2, z := 3 }

/-- Inherited field access. In LOGOS this resolves to `Extended_x`, not `Base_x`. -/
example : e.x = 1 := rfl
example : e.z = 3 := rfl

/-- The parent projection, via the generated conversion. -/
example : e.toBase.x = 1 := rfl

/-- The round-trip law that pins `toBase` as a *definition* rather than an axiom. -/
example (c : Extended) : c.toBase.x = c.x := rfl
example (c : Extended) : c.toBase.y = c.y := rfl

/-- A function expecting the parent, applied to a child. Lean inserts `Extended.toBase`; a
    LOGOS translation must insert it explicitly, since there is no subtyping between the two
    inductives. -/
def baseSum (b : Base) : Nat := b.x + b.y

example : baseSum e.toBase = 3 := rfl

/-- Field order after flattening, made observable: all three fields are `Nat`, so a permuted
    flattening is type-invisible at the constructor. -/
example : (Extended.mk (Base.mk 7 8) 9).x = 7 := rfl
example : (Extended.mk (Base.mk 7 8) 9).z = 9 := rfl

/-- **Diamond.** Both parents extend `Base`, so the shared fields are identified once and
    there are two conversions into `Base` that must agree. -/
structure LeftArm extends Base where
  l : Nat

structure RightArm extends Base where
  r : Nat

structure Diamond extends LeftArm, RightArm

def d : Diamond := { x := 1, y := 2, l := 3, r := 4 }

example : d.x = 1 := rfl
example : d.l = 3 := rfl
example : d.r = 4 := rfl

/-- The two routes to the shared parent must produce the same thing. Nothing in LOGOS checks
    this after flattening. -/
example : d.toLeftArm.toBase.x = d.toRightArm.toBase.x := rfl

/-- Inheritance with a **parameter**: the parameter must be threaded through the flattening
    and through the conversion. -/
structure PBase (α : Type) where
  val : α

structure PExt (α : Type) extends PBase α where
  count : Nat

def pe : PExt Nat := { val := 5, count := 2 }

example : pe.val = 5 := rfl
example : pe.toPBase.val = 5 := rfl

/-- A parent field that is a `Prop`. Flattening keeps it; the *child* remains `Type 0`, so
    this stays inside `add_structure`'s reach — unlike a `Prop`-sorted parent (T03-012). -/
structure Checked extends Base where
  ok : x ≤ y

def c1 : Checked := { x := 1, y := 2, ok := by decide }

example : c1.x = 1 := rfl

end T03_013
