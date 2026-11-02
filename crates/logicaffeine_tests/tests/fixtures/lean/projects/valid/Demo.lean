import Demo.Defs

namespace Demo

theorem addZero (n : Nat) : n + 0 = n := rfl

theorem andSwap (p q : Prop) (h : p ∧ q) : q ∧ p := ⟨h.2, h.1⟩

theorem doubleTwo : double 2 = 4 := rfl

end Demo
