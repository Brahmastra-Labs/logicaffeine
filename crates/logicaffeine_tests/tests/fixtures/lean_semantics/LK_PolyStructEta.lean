import Lean
/-!
Lane LK — structure η at a SYMBOLIC universe level, decided by Lean 4.14.0's KERNEL.
`Pt.{u}` is a universe-polymorphic structure. `ptEta` states that every point is its own
reconstruction from its projections at an arbitrary level `u`; its proof is `rfl`, which the
kernel accepts only through structure η (`tryEtaStruct`). `ptEtaSymm` is the other orientation,
`ptEtaNat` the monomorphic instance. The `run_cmd` below asks the kernel (`addDecl`) to accept
the same `rfl` against the reconstruction with its fields SWAPPED: the kernel refuses, and the
refusal is recorded in the compile log.
-/
open Lean Elab Command
namespace LK_PSE
universe u
structure Pt (A : Type u) where
  x : A
  y : A
theorem ptEta (A : Type u) (p : Pt A) : p = Pt.mk p.x p.y := rfl
theorem ptEtaSymm (A : Type u) (p : Pt A) : Pt.mk p.x p.y = p := rfl
theorem ptEtaNat (p : Pt Nat) : p = Pt.mk p.x p.y := rfl
end LK_PSE
run_cmd do
  let u := Level.param `u
  let sortU := Expr.sort (.succ u)
  let pt (a : Expr) := mkApp (.const ``LK_PSE.Pt [u]) a
  let proj (n : Name) (a p : Expr) := mkApp2 (.const n [u]) a p
  let body (a p : Expr) :=
    let swapped := mkApp3 (.const ``LK_PSE.Pt.mk [u]) a (proj ``LK_PSE.Pt.y a p) (proj ``LK_PSE.Pt.x a p)
    mkApp3 (.const ``Eq [.succ u]) (pt a) p swapped
  let stmt := Expr.forallE `A sortU (.forallE `p (pt (.bvar 0)) (body (.bvar 1) (.bvar 0)) .default) .default
  let proof := Expr.lam `A sortU (.lam `p (pt (.bvar 0)) (mkApp2 (.const ``Eq.refl [.succ u]) (pt (.bvar 1)) (.bvar 0)) .default) .default
  let decl := Declaration.thmDecl { name := `LK_PSE.ptEtaSwapped, levelParams := [`u], type := stmt, value := proof }
  match (← getEnv).addDecl (← getOptions) decl with
  | .ok _ => logInfo "ptEtaSwapped: kernel ACCEPTED"
  | .error e => logInfo m!"ptEtaSwapped: kernel REJECTED: {e.toMessageData (← getOptions)}"
