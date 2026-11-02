/-!
Lane LG — minimal originals for Lean's `Nat.div` and `Nat.mod` on literals. Lean's kernel computes
both natively (GMP); in the export they are `Nat.div._unary` and `Nat.modCore._unary`, definitions by
well-founded recursion (`WellFounded.fix`) that no kernel can unroll by reduction on large operands.
Each theorem is `rfl` or `decide`, so it checks only if the division means exactly Lean's: by zero
(`n / 0 = 0`, `n % 0 = n`), at 2^64 and 2^128, through `Fin` and `UInt8` literals.
-/
namespace LG_DV
theorem modSmall : (4 : Nat) % 5 = 4 := rfl
theorem modWrap : (5 : Nat) % 5 = 0 := rfl
theorem divBasic : (17 : Nat) / 5 = 3 := rfl
theorem divByZero : (17 : Nat) / 0 = 0 := rfl
theorem modByZero : (17 : Nat) % 0 = 17 := rfl
theorem mod64 : (18446744073709551621 : Nat) % 7 = 0 := rfl
theorem div128 : (340282366920938463463374607431768211456 : Nat) / 18446744073709551616 = 18446744073709551616 := rfl
theorem mod128 : (340282366920938463463374607431768223801 : Nat) % 1000000007 = 279644622 := rfl
theorem finDiffer : (⟨(2 + 2) % 5, by decide⟩ : Fin 5) ≠ ⟨(2 + 3) % 5, by decide⟩ := by decide
theorem uint8Wrap : (300 : UInt8).toNat = 44 := rfl
end LG_DV
