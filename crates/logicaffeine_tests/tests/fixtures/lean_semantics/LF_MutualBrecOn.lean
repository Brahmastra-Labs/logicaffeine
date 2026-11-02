/-!
Lane LF — Lean's generated `brecOn` for an ordinary mutual block, and a structural recursion
through it. Lean 4.14.0 generates `Tree.below`, `Tree.brecOn` (and `_1` variants for `Forest`).
-/
namespace LF_MB
mutual
inductive Tree where
  | node : Forest → Tree
inductive Forest where
  | nil : Forest
  | cons : Tree → Forest → Forest
end
mutual
def Tree.size : Tree → Nat
  | .node f => f.size + 1
def Forest.size : Forest → Nat
  | .nil => 0
  | .cons t f => t.size + f.size
end
theorem size_sample : (Tree.node (.cons (.node .nil) .nil)).size = 2 := rfl
end LF_MB
