/-!
Lane LF · W03 nested inductives, dimension 2 (universe handling).
Original Lean-positive witness: a universe-polymorphic family nesting itself through the
universe-polymorphic `List`. Lean 4.14.0 generates `UTree.rec.{u_1, u}` and `UTree.rec_1.{u_1, u}`;
the family is used at `Type` and at `Type 1`, and Lean's kernel checks each `rfl`.
-/
namespace LF_W03U

universe u

inductive UTree (α : Type u) : Type u where
  | node : α → List (UTree α) → UTree α

noncomputable def size {α : Type u} (t : UTree α) : Nat :=
  UTree.rec (motive_1 := fun _ => Nat) (motive_2 := fun _ => Nat)
    (fun _ _ ih => ih + 1) 0 (fun _ _ ih₁ ih₂ => ih₁ + ih₂) t

def small : UTree Nat := .node 1 [.node 2 [], .node 3 []]

/-- The same family one universe up: its labels are types. -/
def large : UTree Type := .node Nat [.node Bool []]

theorem size_small : size small = 3 := rfl
theorem size_large : size large = 2 := rfl

end LF_W03U
