/-!
Lane LG — controls for the certified division: two definitions shaped like Lean's `Nat.modCore`
(well-founded recursion on `x`, the same guard `0 < y ∧ y ≤ x`) that must never be computed as `%`.
`badMod` answers `x + 1` where `Nat.modCore` answers `x`; `axMod` recurses through an axiom where
`Nat.modCore` subtracts.
-/
namespace LG_DI
def badMod (x y : Nat) : Nat := if h : 0 < y ∧ y ≤ x then badMod (x - y) y else x + 1
termination_by x
decreasing_by exact Nat.sub_lt (Nat.lt_of_lt_of_le h.1 h.2) h.1
axiom opaqueSub : Nat → Nat → Nat
axiom opaqueSub_lt : ∀ x y, 0 < y → y ≤ x → opaqueSub x y < x
noncomputable def axMod (x y : Nat) : Nat := if h : 0 < y ∧ y ≤ x then axMod (opaqueSub x y) y else x
termination_by x
decreasing_by exact opaqueSub_lt x y h.1 h.2
end LG_DI
