/-!
@case      T00-003
@tier      0 · Kernel term layer
@title     `Expr.const` with an empty level list lowers to `Term::Global`, never `Term::Const`
@lean      Expr.const (levels := []), references to monomorphic declarations
@expect    translate
@target    Term::Global(String) — kernel/src/term.rs:448. `Term::Const { name, levels }`
           (term.rs:458) is reserved for universe-POLYMORPHIC references only (see T01-006).
@evidence  kernel/src/term.rs:443-545 (`Global(String)` and `Const { name, levels }` are two
           different variants); kernel/src/type_checker.rs:63-67 (`Global` is resolved by
           `ctx.get_global`); kernel/src/context.rs:449 (`get_global` searches, in order,
           inductives → constructors → definitions → declarations — FIRST match wins);
           kernel/src/type_checker.rs:69-85 (`Const` is resolved ONLY by
           `ctx.get_universe_poly`, a disjoint map; a `Const` naming a plain definition is
           `UnboundVariable` no matter how many levels it carries);
           kernel/src/reduction.rs:82-91 (delta for `Const` looks in `universe_polys` and, on a
           miss OR an arity mismatch, silently returns the term UNREDUCED — no error)
@guard     Catches `Expr.const n []` ⟼ `Term::Const { name: n, levels: vec![] }`, which is the
           obvious structural match and is wrong. `infer_type` sends `Const` to
           `get_universe_poly` (type_checker.rs:70-72), which does not contain plain definitions,
           so the reference fails as an unbound variable. That failure is at least loud — the
           dangerous half is `reduce_step` (reduction.rs:82-91), which on the same miss returns
           the term unchanged with no diagnostic. So a `Const` that *is* registered but with the
           wrong level arity becomes silently OPAQUE: every `rfl`-style proof that needed it to
           unfold now fails, and the translator sees "the theorem did not prove", not "the
           reference was malformed".
@guard     Catches ignoring `get_global`'s lookup PRECEDENCE (context.rs:449). LOGOS keys are
           flat `String`s with one namespace shared by inductives, constructors, definitions and
           axioms. If the translator emits a definition named `Nat` (say, from Lean's
           `MyLib.Nat` flattened to `Nat`), every later `Global("Nat")` resolves to the prelude
           INDUCTIVE instead, because inductives are searched first. The definition is simply
           never reachable, and nothing reports it. (Name flattening in general is T02-011 /
           T02-020; this case guards only the resolution-order half of it.)
@guard     Catches emitting a constructor reference as `Global` of the *inductive*'s name or vice
           versa. `Nat.succ` and `Nat` are separate keys in separate maps; a translator that
           mangles both to the same flat string makes `get_global` return the inductive's sort
           where a constructor's type was meant.
@oracle    Every emitted `Term::Global(n)` satisfies `ctx.get_global(n).is_some()`, and
           `ctx.get_universe_poly(n).is_none()`; every emitted `Term::Const { name, levels }`
           satisfies `ctx.get_universe_poly(name)` with `params.len() == levels.len()`.
           Additionally, no emitted name resolves through `get_global` to a declaration of a
           different kind than the Lean source referenced.
-/

namespace T00_003

/-- A monomorphic definition and a reference to it. Both `Expr.const`s carry `levels := []`. -/
def base : Nat := 7
def refersToBase : Nat := base

/-- A reference to a prelude inductive (`Nat` is registered by LOGOS's prelude) and to one of
    its constructors. These live in two different maps behind one `get_global`. -/
def refersToInductive : Type := Nat
def refersToConstructor : Nat → Nat := Nat.succ

/-- An axiom-shaped reference: `Expr.const` to an opaque declaration. In LOGOS this is
    `add_declaration`, which `get_global` finds LAST. -/
axiom opaqueNat : Nat
noncomputable def refersToAxiom : Nat := opaqueNat

/-- A local inductive plus a constructor, so the constructors/inductives precedence is
    exercised against names the translator itself chose. -/
inductive Flag where
  | on
  | off

def refersToFlagType : Type := Flag
def refersToFlagCtor : Flag := Flag.on

/-- The collision shape: a *definition* whose flattened name would clash with an inductive.
    Here they are kept distinct on purpose; the guard is that the translator must check
    `get_global` before registering, because `add_definition` will not refuse the clash. -/
def FlagCount : Nat := 2

/-- A reference that is monomorphic in Lean *despite* the referenced declaration being
    universe-polymorphic: `List` is polymorphic, but `List Nat`'s head `Expr.const` carries the
    level `[0]`, not `[]`. This is the boundary between this case and T01-006 — the translator
    must branch on `levels.isEmpty`, not on the declaration's own polymorphism. -/
def monoInstanceOfPolyConst : Type := List Nat

end T00_003
