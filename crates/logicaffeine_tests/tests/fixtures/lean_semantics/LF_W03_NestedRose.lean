/-!
Lane LF · W03 — nesting through a container that is itself a nested family.
Lean 4.14.0 eliminates `Rose RoseOfTrees` to an auxiliary copy of `Rose`, whose own `List`
occurrence becomes a second auxiliary: three motives, discovered outermost first.
-/
namespace LF_W03R
/-- Nesting through a container that is itself a nested family. -/
inductive Rose (α : Type) where
  | node : α → List (Rose α) → Rose α

inductive RoseOfTrees where
  | node : Rose RoseOfTrees → RoseOfTrees
  | leaf : RoseOfTrees

def rLeaf : RoseOfTrees := .node (.node .leaf [])
end LF_W03R
