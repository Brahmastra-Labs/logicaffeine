/-!
Lane LG — minimal originals for projections whose discriminant's type only unfolds to a structure.
Lean's kernel types `Expr.proj` by `whnf (inferType e)`: it unfolds the type's head until the
structure appears and never normalizes the rest of the type. `Nat.log2_terminates` projects out of
`Nat.below` at `n + 4`, which reaches `PProd` only through `Nat.rec`.
-/
namespace LG_PJ
def PairAlias : Type := Nat × Nat
def firstOfAlias (p : PairAlias) : Nat := p.1
def Deep : Nat → Type
  | 0 => Nat × Nat
  | n + 1 => Deep n
def firstOfDeep (p : Deep 300) : Nat := p.1
theorem usesLog2Terminates : ∀ n, n ≥ 2 → n / 2 < n := Nat.log2_terminates
end LG_PJ
