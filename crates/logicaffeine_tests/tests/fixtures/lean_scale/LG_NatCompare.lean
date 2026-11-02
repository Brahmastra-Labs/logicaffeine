/-!
Lane LG — minimal originals for Lean's `Nat.ble`, `Nat.blt` and `Nat.beq` on literals.
Lean's kernel computes `Nat.ble` and `Nat.beq` natively (GMP); `Nat.blt n m` unfolds to
`Nat.ble (n+1) m`. A kernel without a checked acceleration reduces them in unary, which cannot
finish for these operands. The bounds are the ones `isValidChar_UInt32` decides, and 2^32.
-/
namespace LG_NC
theorem bleBound : Nat.ble 1114112 4294967296 = true := rfl
theorem bleRefuses : Nat.ble 4294967296 1114112 = false := rfl
theorem bltBound : Nat.blt 1114112 4294967296 = true := rfl
theorem bltRefusesEqual : Nat.blt 4294967296 4294967296 = false := rfl
theorem beqSame : Nat.beq 4294967296 4294967296 = true := rfl
theorem beqDiffer : Nat.beq 4294967296 4294967295 = false := rfl
theorem decEqSame : (4294967296 : Nat) = 4294967296 := by decide
theorem decNe : (4294967296 : Nat) ≠ 4294967295 := by decide
theorem decLe : (1114112 : Nat) ≤ 4294967296 := by decide
theorem decNotLt : ¬ ((4294967296 : Nat) < 1114112) := by decide
end LG_NC
