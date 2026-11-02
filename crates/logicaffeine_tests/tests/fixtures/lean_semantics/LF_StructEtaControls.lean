import Lean
/-!
Lane LF — structure-eta iota controls, decided by Lean 4.14.0's KERNEL (`addDecl`).
`propPairRecVar` asks the kernel to reduce a recursor of a `Prop` structure on a variable: the
kernel refuses (Lean's `toCtorWhenStruct` requires a never-`Prop` sort). `typePairRecVar` asks
the same of a `Type` structure: the kernel accepts. The exported `PropPair`/`TPair` families
and the accepted declaration are the witness; the refusal message is recorded below.
-/
open Lean Elab Command
namespace LF_SEC
structure PropPair (p q : Prop) : Prop where
  left : p
  right : q
structure TPair (α β : Type) where
  fst : α
  snd : β
/-- A `Type` structure's recursor on a VARIABLE reduces by struct-eta iota. -/
theorem typePairRecVar (x : TPair Nat Nat) :
    TPair.rec (motive := fun _ => Nat) (fun a _ => a) x = x.fst := rfl
end LF_SEC
run_cmd do
  let nat := mkConst ``Nat
  let pp := mkApp2 (mkConst ``LF_SEC.PropPair) (mkConst ``True) (mkConst ``True)
  let motive := Expr.lam `t pp nat .default
  let minor := Expr.lam `a (mkConst ``True) (.lam `b (mkConst ``True) (mkNatLit 0) .default) .default
  let recApp (h : Expr) := mkAppN (.const ``LF_SEC.PropPair.rec [1]) #[mkConst ``True, mkConst ``True, motive, minor, h]
  let stmt := Expr.forallE `h pp (mkApp3 (.const ``Eq [1]) nat (recApp (.bvar 0)) (mkNatLit 0)) .default
  let proof := Expr.lam `h pp (mkApp2 (.const ``Eq.refl [1]) nat (mkNatLit 0)) .default
  let decl := Declaration.thmDecl { name := `LF_SEC.propPairRecVar, levelParams := [], type := stmt, value := proof }
  match (← getEnv).addDecl (← getOptions) decl with
  | .ok _ => logInfo "propPairRecVar: kernel ACCEPTED"
  | .error e => logInfo m!"propPairRecVar: kernel REJECTED: {e.toMessageData (← getOptions)}"
