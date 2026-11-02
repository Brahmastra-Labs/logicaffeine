/-!
Lane LF · W03 nested inductives, dimension 1 (parameter preservation).
Original Lean-positive witnesses: each family nests itself inside a container while keeping
its uniform parameters. Lean 4.14.0 accepts every block below and generates nested
recursors with `numParams` equal to the family's parameter count. The `rfl` theorems are
checked by Lean's kernel, so they fix the generated computation rules as evidence.
-/
namespace LF_W03

/-- A type parameter, nested through `List`. -/
inductive PTree (α : Type) where
  | node : α → List (PTree α) → PTree α

/-- A value parameter whose later field depends on it. -/
inductive NTree (n : Nat) where
  | node : Fin n → List (NTree n) → NTree n

/-- A parameter inside an inert container argument, nested two containers deep. -/
inductive PairTree (α : Type) where
  | node : List (Prod α (PairTree α)) → PairTree α

/-- Node count through the generated recursor: both motives, all three minors. -/
noncomputable def count {α : Type} (t : PTree α) : Nat :=
  PTree.rec (motive_1 := fun _ => Nat) (motive_2 := fun _ => Nat)
    (fun _ _ ih => ih + 1) 0 (fun _ _ ih₁ ih₂ => ih₁ + ih₂) t

/-- Sum of labels: the parameter's values flow through the constructor field. -/
noncomputable def labels (t : PTree Nat) : Nat :=
  PTree.rec (motive_1 := fun _ => Nat) (motive_2 := fun _ => Nat)
    (fun a _ ih => a + ih) 0 (fun _ _ ih₁ ih₂ => ih₁ + ih₂) t

def sample : PTree Nat := .node 1 [.node 2 [], .node 3 [.node 4 []]]

theorem count_sample : count sample = 4 := rfl
theorem labels_sample : labels sample = 10 := rfl

/-- Sum of the `Fin` values carried under the value parameter. -/
noncomputable def finSum {n : Nat} (t : NTree n) : Nat :=
  NTree.rec (motive_1 := fun _ => Nat) (motive_2 := fun _ => Nat)
    (fun i _ ih => i.val + ih) 0 (fun _ _ ih₁ ih₂ => ih₁ + ih₂) t

def finSample : NTree 5 := .node 4 [.node 1 [], .node 2 []]

theorem finSum_sample : finSum finSample = 7 := rfl

/-- Leaf count through the three-motive recursor of the two-deep nesting. -/
noncomputable def pairCount {α : Type} (t : PairTree α) : Nat :=
  PairTree.rec (motive_1 := fun _ => Nat) (motive_2 := fun _ => Nat) (motive_3 := fun _ => Nat)
    (fun _ ih => ih + 1) 0 (fun _ _ ih₁ ih₂ => ih₁ + ih₂) (fun _ _ ih => ih) t

def pairSample : PairTree Nat := .node [(1, .node []), (2, .node [(3, .node [])])]

theorem pairCount_sample : pairCount pairSample = 4 := rfl

/-- No parameters, nested two containers deep through a container ARGUMENT. Lean discovers
    `List (List DeepTree)` before the `List DeepTree` inside it; the recursor, motive and
    minor order follow that discovery order. -/
inductive DeepTree where
  | node : List (List DeepTree) → DeepTree

noncomputable def deepCount (t : DeepTree) : Nat :=
  DeepTree.rec (motive_1 := fun _ => Nat) (motive_2 := fun _ => Nat) (motive_3 := fun _ => Nat)
    (fun _ ih => ih + 1) 0 (fun _ _ ih₁ ih₂ => ih₁ + ih₂) 0 (fun _ _ ih₁ ih₂ => ih₁ + ih₂) t

def deepSample : DeepTree := .node [[.node []], [.node [[.node []]], .node []]]

theorem deepCount_sample : deepCount deepSample = 5 := rfl

end LF_W03
