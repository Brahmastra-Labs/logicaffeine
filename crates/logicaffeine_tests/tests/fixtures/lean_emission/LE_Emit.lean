/-!
Lean → LOGOS source-emission fixture (lane LE, W04). Every declaration exists to exercise one
obligation of the emitter: literate forms, universe preservation, binder capture, name collision,
opacity, assumptions and dependency order. Compiled and exported with the pinned toolchain
(Lean 4.14.0, lean4export 20c4a64a); see PROVENANCE.json.
-/

namespace LE_Emit

/-- An enumeration. -/
inductive Color where
  | red
  | green
  | blue

/-- Constructors with non-dependent fields. -/
inductive Shape where
  | dot
  | segment (length : Nat)
  | box (width height : Nat)

/-- A parameterized recursive type. -/
inductive Chain (α : Type) where
  | done
  | link (head : α) (tail : Chain α)

def favorite : Color := Color.green

def two : Nat := 2

def double (n : Nat) : Nat := Nat.add n n

def area : Shape → Nat
  | Shape.dot => 0
  | Shape.segment _ => 0
  | Shape.box w h => Nat.mul w h

theorem double_two : double 2 = 4 := rfl

universe u v

def pick {α : Sort u} {β : Sort v} (a : α) (_b : β) : α := a

def Arrow (α : Sort u) (β : Sort v) : Sort (imax u v) := α → β

def Lift (α : Type u) : Type (max u v) := ULift.{v} α

theorem pick_self {α : Type u} (a : α) : pick a a = a := rfl

def capture (x : Nat) (f : (x : Nat) → Nat) : Nat := f x

def shadowGlobal (Nat : Type) (two : Nat) : Nat := two

def dependent (α : Type) (β : α → Type) (f : (a : α) → β a) (x : α) : β x := f x

axiom Oracle : Nat → Prop

axiom oracle_zero : Oracle 0

theorem uses_oracle : Oracle 0 := oracle_zero

theorem chained : Oracle 0 := uses_oracle

end LE_Emit

def add (a b : Nat) : Nat := Nat.add a b

def «a b» : Nat := 1

def a_b : Nat := 2

namespace x
def y : Nat := 3
end x

def «x.y» : Nat := 4

def collisions : Nat := Nat.add (Nat.add (add «a b» a_b) x.y) «x.y»
