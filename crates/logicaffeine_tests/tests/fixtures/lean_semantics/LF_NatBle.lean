/-!
Lane LF — minimal originals for the `isValidChar_UInt32` wrong rejection (42 roots).
Each `decide` reduces `Nat.decLt` to Lean's `Nat.ble` on literals, which Lean's kernel computes
natively (GMP). The bounds are the ones `isValidChar_UInt32` decides.
-/
namespace LF_NB
theorem small : (55296 : Nat) < 4294967296 := by decide
theorem charBound : (1114112 : Nat) < 4294967296 := by decide
end LF_NB
