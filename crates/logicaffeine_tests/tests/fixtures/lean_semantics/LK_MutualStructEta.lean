import Lean
/-!
Lane LK — which members of a MUTUAL block are structure-like, decided by Lean 4.14.0's KERNEL.
Lean's `isStructureLike` requires one constructor, no indices and `isRec = false`, and `isRec` is a
property of the whole block. `NA`/`NB` form a block in which no constructor mentions a member, so
`NA` is structure-like and `naEta` (η by `rfl`) is accepted. `MA`/`MB` form a block in which `MA.mk`
takes an `MB`, so the block is recursive: `MB` has one constructor and no indices but is NOT
structure-like, and the `run_cmd` below records the kernel refusing η by `rfl` for it.
-/
open Lean Elab Command
namespace LK_MSE
mutual
inductive NA where
  | mk : Nat → NA
inductive NB where
  | mk : Bool → NB
end
def NA.get : NA → Nat
  | .mk n => n
theorem naEta (a : NA) : a = NA.mk a.get := rfl
mutual
inductive MA where
  | mk : MB → MA
inductive MB where
  | mk : Nat → MB
end
def MB.get : MB → Nat
  | .mk n => n
end LK_MSE
run_cmd do
  let mb := mkConst ``LK_MSE.MB
  let get (b : Expr) := mkApp (mkConst ``LK_MSE.MB.get) b
  let rebuilt (b : Expr) := mkApp (mkConst ``LK_MSE.MB.mk) (get b)
  let stmt := Expr.forallE `b mb (mkApp3 (.const ``Eq [1]) mb (.bvar 0) (rebuilt (.bvar 0))) .default
  let proof := Expr.lam `b mb (mkApp2 (.const ``Eq.refl [1]) mb (.bvar 0)) .default
  let decl := Declaration.thmDecl { name := `LK_MSE.mbEta, levelParams := [], type := stmt, value := proof }
  match (← getEnv).addDecl (← getOptions) decl with
  | .ok _ => logInfo "mbEta: kernel ACCEPTED"
  | .error e => logInfo m!"mbEta: kernel REJECTED: {e.toMessageData (← getOptions)}"
