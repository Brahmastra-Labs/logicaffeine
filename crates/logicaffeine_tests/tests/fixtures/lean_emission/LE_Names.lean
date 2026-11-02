/-!
Lean → LOGOS source-emission fixture (lane LE, W04): names. Each declaration exercises one naming
obligation of the emitter — a Lean root spelled like a prelude identity (Lean's class `Zero` is not the
prelude's `Zero`, which is Lean's `Nat.zero`), a reference the declaration's own namespace would
capture (the root `pick.{u}` called from `LE_Names.pick`; the root `twice` called beside the
universe-polymorphic `LE_Names.twice`), and binders named like prelude constants. Compiled and exported
with the pinned toolchain (Lean 4.14.0, lean4export 20c4a64a); see PROVENANCE.json.
-/

/-- A root universe-polymorphic constant. -/
def pick {α : Sort u} (a : α) : α := a

/-- A root monomorphic constant. -/
def twice (n : Nat) : Nat := n + n

namespace LE_Names

/-- Calls the root `pick` from inside its own namespace, where `pick` means itself. -/
def pick (n : Nat) : Nat := _root_.pick n

/-- A universe-polymorphic neighbour of the root `twice`. -/
def twice {α : Sort u} (a : α) : α := a

/-- Calls the root `twice` beside `LE_Names.twice`. -/
def useTwice (n : Nat) : Nat := _root_.twice (LE_Names.twice n)

/-- Lean's `Zero` class. -/
def viaZero : Nat := Zero.zero

/-- Binders named like prelude constants: `le`, `add`, and `Nat` itself. -/
def keepsNothing (le : Nat) (add : Nat) (Nat : Bool) : Bool := Nat

end LE_Names
