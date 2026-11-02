/-!
Lane LF · W03 nested inductives — the remaining dimensions, as original Lean-positive witnesses.
Lean 4.14.0 accepts every family below. Each is exported on its own, with one definition that
uses it, so an importer that cannot reconstruct the family exactly must refuse at the family
and can never reach its user.
-/
namespace LF_W03N

universe u

/-- Dimension 2: a universe-polymorphic nested family. -/
inductive UTree (α : Type u) : Type u where
  | node : α → List (UTree α) → UTree α

def uLeaf : UTree Nat := .node 0 []

/-- Dimension 3: nesting through an INDEXED container. -/
inductive Vec (α : Type) : Nat → Type where
  | nil : Vec α 0
  | cons : α → Vec α n → Vec α (n + 1)

inductive VTree where
  | node : Vec VTree 2 → VTree
  | leaf : VTree

def vPair : VTree := .node (.cons .leaf (.cons .leaf .nil))

/-- Dimension 3': the nested family itself carries an index. -/
inductive ITree : Nat → Type where
  | leaf : ITree 0
  | node : List (ITree 0) → ITree 1

def iNode : ITree 1 := .node [.leaf]

/-- Dimension 4: a dependent field and a function field beside the nesting. -/
inductive DTree where
  | node : (n : Nat) → (Fin n → DTree) → List DTree → DTree

def dLeaf : DTree := .node 0 (fun i => i.elim0) []

-- Dimension 5: nesting inside an ordinary mutual block.
mutual
inductive MTree where
  | node : List MForest → MTree
inductive MForest where
  | grove : MTree → MForest
  | empty : MForest
end

def mEmpty : MTree := .node [.empty]

end LF_W03N
