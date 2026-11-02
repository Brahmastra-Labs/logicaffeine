import Lean
/-!
Lane LF — a function whose type is not syntactically a `Π` needs only its head exposed.

Arena `perf/magma-list-pair-n21` applies `h : EquationRHS (Fin 21)` (an `abbrev` for a `∀`) and
`of_decide_eq_false rfl : ¬ …`. Lean types an application by `ensure_pi (whnf (infer f))`: only the
head is reduced. Every `Fin` numeral in such a type carries its instance's proof (`Nat.mod_lt …`),
and theorems are transparent, so normalizing the whole type unfolds proofs nothing needs. Each
theorem in namespace `LF_FT` applies a function whose type is an `abbrev`, a `def` or a `¬`; Lean
4.14.0's kernel accepts every one.

`LF_FT.idemWrong` claims the application at a false equation and `LF_FT.idemFalse` at `False`. Lean's
kernel REJECTS both (`addDecl`; the messages are logged when this file is compiled); they are then
added with `debug.skipKernelTC` only so that the exporter can hand them to LOGOS as corruption
controls.
-/
open Lean Elab Command

namespace LF_FT

/-- A proposition about every element of `Fin 5`, written as an `abbrev`. -/
abbrev AllFixed : Prop := ∀ x : Fin 5, x = x

theorem useAt (h : AllFixed) : (3 : Fin 5) = 3 := h 3

/-- An operation whose value carries a proof, as the Arena's magma operation does. -/
def op (x y : Fin 5) : Fin 5 := ⟨(x.val + y.val) % 5, Nat.mod_lt _ (by decide)⟩

/-- The same shape as a `def`, so the head is exposed by δ rather than by an `abbrev`. -/
def Idem : Prop := ∀ x : Fin 5, op x x = op x x

theorem idemAt (h : Idem) : op 2 2 = op 2 2 := h 2

theorem notAt (h : ¬ ((1 : Fin 5) = 2)) (e : (1 : Fin 5) = 2) : False := h e

end LF_FT

/-- Ask Lean's kernel, record its verdict, then add the declaration without kernel checking. -/
def refuteThenAdmit (name : Name) (stmt proof : Syntax.Term) : CommandElabM Unit := do
  let (type, value) ← liftTermElabM do
    let type ← instantiateMVars (← Term.elabTermAndSynthesize stmt none)
    let value ← instantiateMVars (← Term.elabTermAndSynthesize proof none)
    return (type, value)
  let decl := Declaration.thmDecl { name, levelParams := [], type, value }
  match (← getEnv).addDecl (← getOptions) decl with
  | .ok _ => logInfo m!"{name}: kernel ACCEPTED"
  | .error e => logInfo m!"{name}: kernel REJECTED: {e.toMessageData (← getOptions)}"
  withScope (fun sc => { sc with opts := debug.skipKernelTC.set sc.opts true }) do
    liftCoreM (addDecl decl)

/- `h 2 : op 2 2 = op 2 2`, claimed at `op 2 2 = op 2 3`. -/
run_cmd do
  refuteThenAdmit `LF_FT.idemWrong (← `(LF_FT.Idem → LF_FT.op 2 2 = LF_FT.op 2 3)) (← `(fun (h : LF_FT.Idem) => h 2))

/- `h 2 : op 2 2 = op 2 2`, claimed at `False`. -/
run_cmd do
  refuteThenAdmit `LF_FT.idemFalse (← `(LF_FT.Idem → False)) (← `(fun (h : LF_FT.Idem) => h 2))
