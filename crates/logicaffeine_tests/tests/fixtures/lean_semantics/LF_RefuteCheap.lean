import Lean
/-!
Lane LF — a conversion refuted by one cheap component is a REJECTION, whatever the other
component would cost to evaluate.

Adapted from lean-kernel-arena `tests/perf/refute-cheap-first.lean` and `refute-cheap-last.lean`
(Courant and Leroy, POPL 2026, §10). There `n = 200`; here `n = 2000`, so `count #n` evaluates in
Θ(n²) ≈ 4 000 000 reductions — more than a normalization budget of 2²⁰ steps covers — while
`false` against `true` refutes in one comparison. A checker that asks whether its refutation was
"really" decided by normalizing both whole sides reports these as undecided; one that asks the
conversion it actually ran rejects them.

Lean's evidence, all from its kernel:
* `cheapFirst` is REJECTED by `addDecl` (the message is logged when this file is compiled).
* `cheapLast` is not submitted to `addDecl`: Lean's kernel compares `count #2000` against
  `count #2001` first and did not finish within 900 s. Its falsity is proved instead.
* `cheapFirst_false` and `cheapLast_false` — the negations, by `congrArg` on the boolean
  component — are ACCEPTED by Lean's kernel in the ordinary way.
Both claims are then added with `debug.skipKernelTC` only so that the exporter can hand them to
LOGOS as corruption controls.
-/
open Lean Elab Command

namespace LF_RC

inductive N where
  | O : N
  | S : N → N

/-- The recursor applied directly, as in the Arena source: no `brecOn` table. -/
noncomputable def N.add (a b : N) : N :=
  N.rec (motive := fun _ => N) b (fun _ ih => N.S ih) a

noncomputable def count (m : N) : N :=
  N.rec (motive := fun _ => N) N.O (fun _ ih => N.add ih (N.S N.O)) m

theorem fst_ne {β : Type} (b d : β) : ((false, b) : Bool × β) ≠ (true, d) :=
  fun h => Bool.noConfusion (congrArg Prod.fst h)

theorem snd_ne {α : Type} (a c : α) : ((a, false) : α × Bool) ≠ (c, true) :=
  fun h => Bool.noConfusion (congrArg Prod.snd h)

def numeral (k : Nat) : Expr := Id.run do
  let mut e := mkConst ``N.O
  for _ in [:k] do
    e := mkApp (mkConst ``N.S) e
  return e

def countOf (k : Nat) : Expr := mkApp (mkConst ``count) (numeral k)

def pair (α β x y : Expr) : Expr := mkApp4 (mkConst ``Prod.mk [0, 0]) α β x y

/-- `lhs = rhs` at `Prod α β`, proved by `Eq.refl lhs`. -/
def claim (name : Name) (α β lhs rhs : Expr) : Declaration :=
  let prodTy := mkApp2 (mkConst ``Prod [0, 0]) α β
  .thmDecl { name, levelParams := []
             type := mkApp3 (mkConst ``Eq [1]) prodTy lhs rhs
             value := mkApp2 (mkConst ``Eq.refl [1]) prodTy lhs }

/-- `lhs ≠ rhs` at `Prod α β`, proved by `proof`, checked by Lean's kernel as usual. -/
def refutation (name : Name) (α β lhs rhs proof : Expr) : Declaration :=
  let prodTy := mkApp2 (mkConst ``Prod [0, 0]) α β
  .thmDecl { name, levelParams := [], type := mkApp3 (mkConst ``Ne [1]) prodTy lhs rhs, value := proof }

def admitUnchecked (decl : Declaration) : CommandElabM Unit :=
  withScope (fun sc => { sc with opts := debug.skipKernelTC.set sc.opts true }) do
    liftCoreM (addDecl decl)

end LF_RC

set_option maxRecDepth 1000000

run_cmd do
  let n := 2000
  let nTy := mkConst ``LF_RC.N
  let bool := mkConst ``Bool
  let ff := mkConst ``Bool.false
  let tt := mkConst ``Bool.true
  let firstL := LF_RC.pair bool nTy ff (LF_RC.countOf n)
  let firstR := LF_RC.pair bool nTy tt (LF_RC.countOf (n + 1))
  let lastL := LF_RC.pair nTy bool (LF_RC.countOf n) ff
  let lastR := LF_RC.pair nTy bool (LF_RC.countOf (n + 1)) tt
  -- Lean's kernel on the claim whose cheap component comes first.
  let first := LF_RC.claim `LF_RC.cheapFirst bool nTy firstL firstR
  match (← getEnv).addDecl (← getOptions) first with
  | .ok _ => logInfo "LF_RC.cheapFirst: kernel ACCEPTED"
  | .error e => logInfo m!"LF_RC.cheapFirst: kernel REJECTED: {e.toMessageData (← getOptions)}"
  LF_RC.admitUnchecked first
  LF_RC.admitUnchecked (LF_RC.claim `LF_RC.cheapLast nTy bool lastL lastR)
  -- Lean's kernel proves both claims false.
  liftCoreM <| addDecl <| LF_RC.refutation `LF_RC.cheapFirst_false bool nTy firstL firstR
    (mkApp3 (mkConst ``LF_RC.fst_ne) nTy (LF_RC.countOf n) (LF_RC.countOf (n + 1)))
  liftCoreM <| addDecl <| LF_RC.refutation `LF_RC.cheapLast_false nTy bool lastL lastR
    (mkApp3 (mkConst ``LF_RC.snd_ne) nTy (LF_RC.countOf n) (LF_RC.countOf (n + 1)))
  logInfo "LF_RC.cheapFirst_false, LF_RC.cheapLast_false: kernel ACCEPTED"
