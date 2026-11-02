/-!
@case      T03-010
@tier      3 · Inductives and structures
@title     `structure` — one call that registers an inductive, a constructor, N definitions
           and the eta metadata
@lean      structure … where, `Lean.StructureInfo`, the anonymous constructor `⟨…⟩`
@target    Context::add_structure(name, params, fields) — a *single* call; anything assembled
           by hand is missing something
@expect    translate
@evidence  kernel/src/context.rs:257-369 (`add_structure` registers, in this order: the
           inductive via `add_indexed_inductive(name, ind_type, params.len())` (:301), the
           constructor `{name}_mk` (:304-306), one **definition** `{name}_{field}` per field
           whose body is a `Term::Match` (:308-363), and the `StructInfo` that gates eta
           (:365-368));
           kernel/src/context.rs:117-127 (`StructInfo { mk, num_params, projections }`);
           kernel/src/context.rs:88-92 (the `structures` registry: "Only these one-constructor
           inductives get DEFINITIONAL ETA … never inferred for arbitrary one-constructor
           inductives");
           kernel/src/type_checker.rs:1391-1417 (`try_structure_eta`, gated on
           `struct_info`); kernel/src/type_checker.rs:1446-1478 (eta fires only inside
           `def_eq_normalized`, never in `normalize`);
           kernel/src/context.rs:314-328 (a later field's type may reference an earlier field,
           and `add_structure` rewrites those references to projections applied to the
           discriminant — dependent fields are supported);
           kernel/src/context.rs:299-301 (**the sort is hard-coded to `Type 0`** — see T03-012)
@guard     Catches hand-assembling the structure from `add_inductive` + `add_constructor` +
           projections. That produces something that typechecks and computes but is **not** in
           the `structures` registry, so `try_structure_eta` never fires for it
           (type_checker.rs:1391-1400 returns `None` when `struct_of_constructor` is `None`).
           Lean proves `⟨p.x, p.y⟩ = p` by `rfl` using definitional eta; after a hand-assembled
           translation that goal is simply false definitionally, and the failure appears as an
           unrelated conversion error in whatever proof depended on it.
@guard     Catches the projection's argument order. `add_structure` builds each projection as
           `Π(params). Π(s : Name params). Tᵢ` (context.rs:330-339), so `p.x` is
           `Point_x <params…> p`, parameters **first**. `elaborate_dot` reproduces exactly that
           order (elaborate.rs:1285-1299). Emitting `Point_x p` for a parameterised structure
           is an arity error at best and a silently-wrong application at worst.
@guard     Catches the generated names. The constructor is `{name}_mk` and the projections are
           `{name}_{field}` — literal string concatenation (context.rs:304, :311). Those keys
           are claimed the moment `add_structure` runs, and a Lean declaration that flattens
           onto one of them overwrites it (T02-020). The translator must reserve them.
@guard     Catches emitting the *fields* in the wrong order. `StructInfo.projections` is in
           field order (context.rs:367), the constructor's telescope is in field order
           (:305), and the `Match` case is `λ(fields). fieldᵢ` (:348) — a permutation makes
           `p.x` return `p.y` with no type error whenever the fields share a type.
@guard     Catches dropping the *dependent* field rewriting. If field `b`'s type mentions field
           `a`, the projection type must be `Tᵦ[a := Name_a params s]` (context.rs:314-331). A
           translator that emits `Tᵦ` with a free `Var("a")` gets a projection whose type has a
           dangling variable, accepted unchecked by `add_definition`.
@oracle    `struct_info(name)` is `Some`, with `mk == "{name}_mk"`, `num_params` equal to the
           structure's parameter count, and `projections` in Lean's field order; `def_eq` proves
           `{name}_mk (Name_f₁ p) … (Name_fₙ p) ≡ p`; and each projection applied to a literal
           constructor reduces to the corresponding argument.
-/

namespace T03_010

/-- The plain structure: no parameters, two fields of the same type — so a field permutation
    is type-invisible. -/
structure Point where
  x : Nat
  y : Nat

example : (Point.mk 1 2).x = 1 := rfl
example : (Point.mk 1 2).y = 2 := rfl

/-- Definitional eta, the property that only the `structures` registry grants. -/
example (p : Point) : Point.mk p.x p.y = p := rfl

/-- Anonymous-constructor notation elaborates to `Point.mk`. -/
example : ({ x := 3, y := 4 } : Point).x = 3 := rfl
example : (⟨3, 4⟩ : Point).y = 4 := rfl

/-- A **parameterised** structure: the projections take the parameters before the receiver. -/
structure Pair (α : Type) (β : Type) where
  fst : α
  snd : β

example : (Pair.mk (3 : Nat) true).fst = 3 := rfl
example : (Pair.mk (3 : Nat) true).snd = true := rfl
example {α β : Type} (p : Pair α β) : Pair.mk p.fst p.snd = p := rfl

/-- A structure with a **dependent** field: `wit`'s type mentions the earlier field `bound`.
    `add_structure` must rewrite that reference into `Sized_bound s`. -/
structure Sized where
  bound : Nat
  wit : Fin (bound + 1)

def someSized : Sized := Sized.mk 3 ⟨0, by omega⟩

example : someSized.bound = 3 := rfl

/-- Three fields, so `projections` order has more than one way to be wrong, and two of the
    three share a type. -/
structure Triple where
  a : Nat
  b : Bool
  c : Nat

example : (Triple.mk 1 true 2).a = 1 := rfl
example : (Triple.mk 1 true 2).c = 2 := rfl
example : (Triple.mk 1 true 2).a ≠ (Triple.mk 1 true 2).c := by decide

/-- A one-constructor **inductive** that is *not* a structure. Lean does not give it eta
    either, so a translator that routes every one-constructor inductive through
    `add_structure` would make `Wrapper.mk (unwrapped w) = w` hold definitionally in LOGOS
    where it does not in Lean. -/
inductive Wrapper where
  | mk : Nat → Wrapper

def unwrapped : Wrapper → Nat
  | Wrapper.mk n => n

example : unwrapped (Wrapper.mk 5) = 5 := rfl

/-- A zero-field structure. `{name}_mk` takes only the parameters; eta says every inhabitant
    equals `mk`. -/
structure UnitLike where

example (u : UnitLike) : u = UnitLike.mk := rfl

/-- A structure whose field type is itself a structure — the projections compose, and the
    inner structure must already be registered when the outer one is. -/
structure Segment where
  start : Point
  stop : Point

example : (Segment.mk (Point.mk 0 0) (Point.mk 1 1)).stop.x = 1 := rfl
example (s : Segment) : Segment.mk s.start s.stop = s := rfl

end T03_010
