/-!
Lane LF · W03 nested inductives — the remaining F01 shapes, as original Lean-positive witnesses:
a container with a value parameter, a container with dependent fields, a container with a
`Sort`-valued field, nesting under a binder, and a proposition nested through `And`.
Lean 4.14.0 accepts every family; each is exported on its own with one user.
-/
namespace LF_F01

/-- A container with a VALUE parameter before its type parameter. -/
inductive VBox (n : Nat) (α : Type) where
  | mk : Fin n → α → VBox n α

inductive VBoxTree where
  | node : VBox 3 VBoxTree → VBoxTree
  | leaf : VBoxTree

def vbLeaf : VBoxTree := .leaf

/-- A container whose constructor has DEPENDENT value fields. -/
inductive DPair (α : Type) where
  | mk : (n : Nat) → (Fin n → α) → DPair α

inductive DPairTree where
  | node : DPair DPairTree → DPairTree
  | leaf : DPairTree

def dpLeaf : DPairTree := .node ⟨0, fun i => i.elim0⟩

/-- A container with a SORT-valued field. -/
inductive SBox (α : Type 1) : Type 1 where
  | mk : Type → α → SBox α

inductive SBoxTree : Type 1 where
  | node : SBox SBoxTree → SBoxTree
  | leaf : SBoxTree

def sbLeaf : SBoxTree := .node (.mk Nat .leaf)

/-- Nesting under a binder: the nested occurrence is a function codomain. -/
inductive FunTree where
  | node : (Nat → List FunTree) → FunTree
  | leaf : FunTree

def ftLeaf : FunTree := .node (fun _ => [])

/-- A proposition nested through `And`. -/
inductive Good : Prop where
  | base : Good
  | both : And Good Good → Good

theorem good : Good := .both ⟨.base, .base⟩

end LF_F01
