/-!
@case      T03-011
@tier      3 · Inductives and structures
@title     Structure projection — Lean's primitive `Expr.proj` has no LOGOS counterpart
@lean      structure, Expr.proj, dot-notation field access, structure eta
@expect    degrade
@target    A `Match`-bodied global definition per field, produced by
           Context::add_structure, applied to the receiver:
           `p.fst`  ⟼  `App(App(App(Global("Point_x"), <params>), p))`
@evidence  kernel/src/context.rs:271-369 (add_structure desugars a record into an inductive,
           one `{name}_mk` constructor, and one ordinary *definition* per field whose body is a
           `Term::Match`); kernel/src/term.rs:443-545 (`Term` has NO `Proj` variant);
           kernel/src/type_checker.rs:1391-1417 (`try_structure_eta` exists but only inside
           `def_eq`, only for structures in the `structures` registry, never in `normalize`)
@loss      PROJECTION_PRIMITIVITY. Lean's `.proj` is a kernel primitive that reduces in one
           iota step and participates in eta. The LOGOS encoding is a delta-unfold to a `Match`,
           so (a) reduction is two steps not one, (b) the projection is only definitionally
           transparent while the structure stays registered, and (c) structure eta holds only
           in `def_eq`, not under `normalize` — so a proof that relies on `normalize` seeing
           `⟨p.1, p.2⟩ ≡ p` will fail even though `def_eq` would accept it.
@loss      SORT_PINNING. `add_structure` hard-codes the structure's sort to `Type 0`
           (context.rs:300). A Lean structure in `Type u` for `u ≥ 1`, or in `Prop`, cannot be
           declared this way at all — that case is `reject`, see T03-012.
@guard     Catches the translator emitting a projection as an *opaque axiom* (which would make
           `⟨a, b⟩.1 = a` unprovable) or as a `Match` inlined at each use site (which blows up
           term size and breaks the `structures` registry lookup that gates eta, so eta
           silently stops firing).
@guard     Catches the translator forgetting that `add_structure` field definitions take the
           structure's parameters *before* the receiver, so `p.x` is
           `Point_x α β p`, not `Point_x p`.
@oracle    Emitted term typechecks; and `def_eq(⟨a,b⟩.1, a)` must hold in LOGOS; and the loss
           record must contain PROJECTION_PRIMITIVITY.
-/

namespace T03_011

structure Point where
  x : Nat
  y : Nat

/-- Field access compiles to `Expr.proj` in Lean's kernel. -/
def getX (p : Point) : Nat := p.x

/-- Projection of a literal constructor — must reduce to `1`. -/
example : ({ x := 1, y := 2 } : Point).x = 1 := rfl

/-- Structure eta: Lean proves this by `rfl` using definitional eta for structures. -/
example (p : Point) : ({ x := p.x, y := p.y } : Point) = p := rfl

/-- A *parameterised* structure: the projection definition takes the parameters first. -/
structure Pair (α : Type) (β : Type) where
  fst : α
  snd : β

def getFst {α β : Type} (p : Pair α β) : α := p.fst

example : (Pair.mk 3 true).fst = 3 := rfl

/-- Eta again, under parameters. -/
example {α β : Type} (p : Pair α β) : Pair.mk p.fst p.snd = p := rfl

end T03_011
