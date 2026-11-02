import Lean
/-!
Lane LF · W03 indexed projection — does Lean accept a projection out of an indexed family?

Lean's surface elaborator emits `Expr.proj` only for structures, which have no indices, so
these declarations are built as raw kernel terms and handed to Lean 4.14.0's kernel with
`addDecl`. The kernel ACCEPTS them: `infer_proj` requires one constructor and a fully applied
family (parameters AND indices), instantiates only the parameters, and walks the fields. The
index arguments are never substituted into a field type. Each accepted declaration below is
therefore a valid original kernel declaration, and the export records the kernel's own terms.
-/
open Lean Elab Command

namespace LF_W03P

/-- A genuinely indexed one-constructor family: the result index is fixed to `0`, so Lean
    cannot promote it to a parameter (`numParams = 0`, `numIndices = 1`). -/
inductive IBox : Nat → Type where
  | mk : Nat → Fin 3 → IBox 0

/-- A dependent second field under an index: `Fin (n + 1)` mentions the first field. -/
inductive DBox : Nat → Type where
  | mk : (n : Nat) → Fin (n + 1) → DBox 0

/-- A parameter AND an index: `α` is uniform, the `Bool` index is fixed per constructor. -/
inductive PBox (α : Type) : Bool → Type where
  | mk : α → Nat → PBox α true

end LF_W03P

open LF_W03P in
def addKernelDecl (name : Name) (type value : Expr) (theorem? : Bool := false) : CommandElabM Unit := do
  let decl := if theorem? then
      Declaration.thmDecl { name, levelParams := [], type, value }
    else
      Declaration.defnDecl { name, levelParams := [], type, value, hints := .abbrev, safety := .safe }
  match (← getEnv).addDecl (← getOptions) decl with
  | .ok env => setEnv env
  | .error e => throwError m!"{name}: kernel rejected: {e.toMessageData (← getOptions)}"

open LF_W03P in
run_cmd do
  let nat := mkConst ``Nat
  let ibox (i : Expr) := mkApp (mkConst ``IBox) i
  let dbox (i : Expr) := mkApp (mkConst ``DBox) i
  let pbox (a i : Expr) := mkApp2 (mkConst ``PBox) a i
  let natLit (n : Nat) := mkApp3 (mkConst ``OfNat.ofNat [0]) nat (mkRawNatLit n) (mkApp (mkConst ``instOfNatNat) (mkRawNatLit n))
  let zero := natLit 0
  -- b.1 at the fixed index
  addKernelDecl `LF_W03P.firstAtZero (.forallE `b (ibox zero) nat .default)
    (.lam `b (ibox zero) (.proj ``IBox 0 (.bvar 0)) .default)
  -- b.1 at a generic index variable
  addKernelDecl `LF_W03P.firstAtAnyIndex
    (.forallE `i nat (.forallE `b (ibox (.bvar 0)) nat .default) .default)
    (.lam `i nat (.lam `b (ibox (.bvar 0)) (.proj ``IBox 0 (.bvar 0)) .default) .default)
  -- the dependent second field: its type mentions the projection of the first field
  let fin (n : Expr) := mkApp (mkConst ``Fin) n
  let succ (n : Expr) := mkApp6 (mkConst ``HAdd.hAdd [0,0,0]) nat nat nat
    (mkApp2 (mkConst ``instHAdd [0]) nat (mkConst ``instAddNat)) n (natLit 1)
  addKernelDecl `LF_W03P.dependentSecond
    (.forallE `b (dbox zero) (fin (succ (.proj ``DBox 0 (.bvar 0)))) .default)
    (.lam `b (dbox zero) (.proj ``DBox 1 (.bvar 0)) .default)
  -- a parameter and an index together
  addKernelDecl `LF_W03P.paramFirst
    (.forallE `α (.sort 1) (.forallE `b (pbox (.bvar 0) (mkConst ``Bool.true)) (.bvar 1) .default) .default)
    (.lam `α (.sort 1) (.lam `b (pbox (.bvar 0) (mkConst ``Bool.true)) (.proj ``PBox 0 (.bvar 0)) .default) .default)

/-- Projection reduction on a constructor, decided by the elaborator and kernel. -/
theorem LF_W03P.firstAtZero_mk : LF_W03P.firstAtZero (LF_W03P.IBox.mk 7 2) = 7 := rfl
theorem LF_W03P.paramFirst_mk : LF_W03P.paramFirst Bool (LF_W03P.PBox.mk false 3) = false := rfl
