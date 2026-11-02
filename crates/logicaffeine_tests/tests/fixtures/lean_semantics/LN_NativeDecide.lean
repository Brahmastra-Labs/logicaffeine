/-!
Lane LN witnesses: `native_decide`, re-proved by the LOGOS kernel's own evaluation.

Every root is exported alone with the pinned Lean 4.14.0 and lean4export `20c4a64a`
(`scripts/lean-export-roots.py`). Lean accepts every declaration here.
-/
namespace LN_ND

/-- One leaf: the shape of `T08_014.viaNative`. -/
theorem product : (100 : Nat) * 100 = 10000 := by native_decide

/-- Two leaves in one proof, each with its own compiled decision. -/
theorem twoLeaves : (100 : Nat) * 100 = 10000 ∧ (2 : Nat) ^ 10 = 1024 :=
  ⟨by native_decide, by native_decide⟩

/-- The leaf reached only through a lemma: `#print axioms` charges it transitively. -/
theorem throughLemma : (100 : Nat) * 100 = 10000 ∧ True := ⟨product, trivial⟩

/-- A closed leaf inside an open proof term. -/
theorem underBinders : ∀ n : Nat, n = n → (3 : Nat) * 7 = 21 := fun _ _ => by native_decide

/-- A leaf beside an admitted step: re-deriving the leaf must not hide the `sorry`. -/
theorem besideSorry : (100 : Nat) * 100 = 10000 ∧ (2 : Nat) + 2 = 5 := ⟨by native_decide, sorry⟩

/-- A decision Lean's compiled code finishes and no kernel evaluation budget covers. -/
def countdown : Nat → Nat
  | 0 => 0
  | n + 1 => countdown n

theorem beyondBudget : countdown 100000000 = 0 := by native_decide

/-- Lean's appeal written by hand with `rfl` as the evaluation step: not the recognized shape. -/
def handDecision : Bool := decide ((100 : Nat) * 100 = 10000)

theorem handAppeal : (100 : Nat) * 100 = 10000 :=
  of_decide_eq_true (Lean.ofReduceBool handDecision true rfl)

end LN_ND
