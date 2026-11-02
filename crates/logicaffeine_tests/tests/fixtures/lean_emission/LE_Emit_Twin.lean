/-!
The corruption twin of `LE_Emit.lean` (lane LE, W04). Every declaration has the SAME Lean name and the
same dependency closure as in `LE_Emit.lean`; the marked ones differ in exactly one respect, so a
document emitted from this export, presented as the emission of `LE_Emit`, must be refused naming that
declaration and that respect.
-/

namespace LE_Emit

/-- TWIN: constructor order differs (green before red). -/
inductive Color where
  | green
  | red
  | blue

inductive Shape where
  | dot
  | segment (length : Nat)
  | box (width height : Nat)

inductive Chain (α : Type) where
  | done
  | link (head : α) (tail : Chain α)

def favorite : Color := Color.green

/-- TWIN: body differs. -/
def two : Nat := 3

def double (n : Nat) : Nat := Nat.add n n

def area : Shape → Nat
  | Shape.dot => 0
  | Shape.segment _ => 0
  | Shape.box w h => Nat.mul w h

/-- TWIN: statement differs. -/
theorem double_two : double 1 = 2 := rfl

universe u v

/-- TWIN: universe parameter order differs. -/
def pick.{v', u'} {α : Sort u'} {β : Sort v'} (a : α) (_b : β) : α := a

def Arrow (α : Sort u) (β : Sort v) : Sort (imax u v) := α → β

def Lift (α : Type u) : Type (max u v) := ULift.{v} α

theorem pick_self {α : Type u} (a : α) : pick a a = a := rfl

def capture (x : Nat) (f : (x : Nat) → Nat) : Nat := f x

def shadowGlobal (Nat : Type) (two : Nat) : Nat := two

def dependent (α : Type) (β : α → Type) (f : (a : α) → β a) (x : α) : β x := f x

/-- TWIN: axiom type differs. -/
axiom Oracle : Nat → Nat → Prop

axiom oracle_zero : Oracle 0 0

/-- TWIN: a transparent definition where the original is a theorem. -/
def uses_oracle : Oracle 0 0 := oracle_zero

theorem chained : Oracle 0 0 := uses_oracle

end LE_Emit

def add (a b : Nat) : Nat := Nat.add a b

def «a b» : Nat := 1

def a_b : Nat := 2

namespace x
def y : Nat := 3
end x

def «x.y» : Nat := 4

def collisions : Nat := Nat.add (Nat.add (add «a b» a_b) x.y) «x.y»
