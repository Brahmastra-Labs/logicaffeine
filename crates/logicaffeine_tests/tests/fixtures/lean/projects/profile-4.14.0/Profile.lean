import Lean

/-! A small module every registered Lean release compiles unchanged: a proposition-level proof, a
definition over `Nat`, a string literal, a tactic `have` (a nondependent let in releases that
elaborate `have` as one), and the statement shapes a statement import distinguishes — a `Prop`
definition, an unproved theorem, an `answer(sorry)` hole and a statement resting on an unfinished
proof. -/

open Lean Elab Term in
/-- `answer(e)`, as formal-conjectures writes an unknown answer: `e` marked `answer := true`. -/
elab "answer(" e:term ")" : term <= expected => do
  let e ← elabTerm e expected
  return .mdata (KVMap.empty.insert `answer (.ofBool true)) e

namespace Profile

theorem andSwap (a b : Prop) (h : a ∧ b) : b ∧ a := ⟨h.2, h.1⟩

def double (n : Nat) : Nat := n + n

theorem doubleTwo : double 2 = 4 := rfl

def greeting : String := "hi"

theorem greetingIsHi : greeting = "hi" := rfl

theorem addZeroViaHave (n : Nat) : n + 0 = n := by
  have h : n + 0 = n := Nat.add_zero n
  exact h

def IsEven (n : Nat) : Prop := ∃ k, n = k + k

/-- A proposition stated by a definition: its value is the statement. -/
def EveryDoubleIsEven : Prop := ∀ n : Nat, IsEven (double n)

/-- An open problem: a clean statement whose proof is a placeholder. -/
theorem unbounded : ∀ n : Nat, ∃ m, n < m := sorry

/-- An unknown answer: the statement is a template over it. -/
theorem answered : answer(sorry) ↔ ∀ n : Nat, n + 0 = n := sorry

/-- An unfinished proof the next statement rests on. -/
theorem unfinished : double 1 = 2 := sorry

/-- A statement whose own value depends on an unfinished proof. -/
def RestsOnUnfinished : Prop := unfinished = unfinished

end Profile
