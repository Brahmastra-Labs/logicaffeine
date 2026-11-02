/-!
@case      T00-002
@tier      0 · Kernel term layer
@title     `Expr.sort` at concrete levels — the `Sort n` / `Type n` index shift
@lean      Expr.sort, Prop, Sort 0, Sort 1, Type, Type 0, Type 1, Type 2
@expect    translate
@target    Term::Sort(Universe) — kernel/src/term.rs:445.
           `Prop` ≡ `Sort 0`  ⟼ `Sort(Universe::Prop)`
           `Type 0` ≡ `Sort 1` ⟼ `Sort(Universe::Type(0))`
           `Type n` ≡ `Sort (n+1)` ⟼ `Sort(Universe::Type(n))`
@evidence  kernel/src/term.rs:167-191 (`Universe::{SProp, Prop, Type(u32), Var, Succ, Max, IMax}`
           — `Prop` is a SEPARATE constructor from `Type(0)`, and `Type` carries a concrete
           `u32`, not a level expression); kernel/src/term.rs:266-276 (`to_lnat`: LOGOS's internal
           decision encoding is `Prop = 0`, `Type n = n+1` — the SAME shift Lean uses between
           `Sort` and `Type`, which is exactly why copying it into the surface representation
           produces an off-by-one); kernel/src/type_checker.rs:54 (`Sort(u) : Sort(u.succ())`)
@guard     Catches the off-by-one that comes from reading LOGOS's internal `to_lnat` encoding
           (term.rs:266-276, where `Type n ↦ n+1`) and using it as the surface mapping. Lean's
           `Expr.sort (.succ .zero)` is `Type 0`, so `Sort (n+1)` must become `Type(n)`, NOT
           `Type(n+1)`. Every level in the file is then one too high. Because LOGOS is
           cumulative (type_checker.rs:1330-1336), a uniformly-too-high level still accepts every
           term that the correct one did — the error only surfaces when a *declared* type says
           `Type 0` and the computed one says `Type 1`, typically many declarations later.
@guard     Catches `Prop ↦ Universe::Type(0)`. They are distinct enum constructors and the
           kernel dispatches on them SYNTACTICALLY in three soundness-critical places:
           the large-elimination gate (type_checker.rs:277-288 matches
           `Term::Sort(Universe::Prop)`), definitional proof irrelevance
           (type_checker.rs:1553-1560, same match), and `imax` (term.rs:222-228, where
           `imax(a, Prop) = Prop` but `imax(a, Type 0) = max(a, Type 0)`). Mapping `Prop` to
           `Type(0)` therefore (a) silently disables proof irrelevance, (b) silently disables the
           large-elimination restriction, and (c) makes every `∀` into a proposition predicative.
           None of the three produces an error. See T01-001 and T01-010 for the full treatment.
@guard     Catches `Sort u` for a *variable* `u` being forced into `Type(u32)`. `Universe::Type`
           takes a concrete `u32`; there is no `Type(<expr>)` form. A `Sort u` must become
           `Sort(Universe::Var("u"))`, which is only bindable on a definition — see T01-003.
@oracle    For each `Expr.sort` in the source, the emitted `Universe` matches the table in
           `@target` exactly (structural equality on the `Universe` enum, not `Universe::equiv`,
           which would hide a `Prop`/`Type 0` confusion under cumulativity); and
           `infer_type(Sort(u))` returns `Sort(u.succ())` with the level the Lean kernel reports
           for the same `Expr`, modulo the documented `Prop` discrepancy of T01-010.
-/

namespace T00_002

/-- `Prop` is `Sort 0`. In Lean these are the same `Expr`. -/
def propAlias : Type := Prop
def sortZeroAlias : Type := Sort 0

/-- `Type` with no index is `Type 0`, which is `Sort 1`. -/
def typeAlias : Type 1 := Type
def typeZeroAlias : Type 1 := Type 0
def sortOneAlias : Type 1 := Sort 1

/-- Two levels up. `Type 1 = Sort 2`. -/
def typeOneAlias : Type 2 := Type 1
def sortTwoAlias : Type 2 := Sort 2

/-- A sort in a *binder* position rather than a body position — the same `Expr.sort`, but the
    translator reaches it through `Pi.param_type` rather than through a definition body. -/
def quantifiesOverTypes : Type 1 := ∀ (_α : Type), Nat
def quantifiesOverProps : Type := ∀ (_p : Prop), Nat

/-- A sort applied as an argument. `Expr.sort` in an application spine. Note that the two
    arguments need functions at two DIFFERENT levels, because Lean has no cumulativity:
    `Type 0 : Type 1` but `Prop : Type 0`, and neither may be passed to the other's slot. -/
def idTypeAtOne (α : Type 1) : Type 1 := α
def idTypeAtZero (α : Type) : Type := α
def appliedToType : Type 1 := idTypeAtOne Type
def appliedToProp : Type := idTypeAtZero Prop

/-- Deliberately adjacent levels, so an off-by-one is a type error rather than a silent
    widening: `typeZeroAlias : Type 1` and `typeOneAlias : Type 2` cannot be swapped. -/
def levelLadder : Type 3 := Type 2

end T00_002
