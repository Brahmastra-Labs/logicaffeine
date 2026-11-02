/-!
Lane LF — iota on a structure-typed variable (Lean's kernel `toCtorWhenStructure`).
`beqByMatch` is defined by pattern matching, as `instBEqProd` is; reducing `a == b` for
VARIABLES `a b` needs `P2.casesOn a f ≡ f a.1 a.2`. Lean 4.14.0 accepts every declaration.
-/
namespace LF_SE
universe u v
structure P2 (α : Type u) (β : Type v) where
  fst : α
  snd : β
def beqByMatch {α : Type u} {β : Type v} [BEq α] [BEq β] : P2 α β → P2 α β → Bool
  | ⟨a₁, b₁⟩, ⟨a₂, b₂⟩ => a₁ == a₂ && b₁ == b₂
/-- Needs struct-eta iota: the match on variables reduces to its projections. -/
theorem unfoldsOnVariables {α : Type u} {β : Type v} [BEq α] [BEq β] (a b : P2 α β) :
    beqByMatch a b = (a.1 == b.1 && a.2 == b.2) := rfl
/-- The same for Lean's own `Prod` and `instBEqProd`. -/
theorem prodUnfoldsOnVariables {α : Type u} {β : Type v} [BEq α] [BEq β] (a b : α × β) :
    (a == b) = (a.1 == b.1 && a.2 == b.2) := rfl
end LF_SE
