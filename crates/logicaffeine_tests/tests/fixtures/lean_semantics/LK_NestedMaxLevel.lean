import Lean
/-!
Lane LK — a nested inductive whose container instance lives at a universe that is EQUAL to the
family's but written differently, decided by Lean 4.14.0's KERNEL. `Node.{u,v} : Type (max u v)`
nests through `Array (Entry α β (Node α β))`, and `Entry.{u,v,max u v} α β (Node α β)` lives in
`Type (max u v (max u v))`, which is `Type (max u v)`. This is the shape of Lean's own
`Lean.PersistentHashMap.Node`, which `nodeEntriesEmpty` uses directly. The `run_cmd` below asks the
kernel to accept a nested family whose container instance lives in a DIFFERENT universe
(`LBox.{0} BadT : Type 1` inside `BadT : Type 0`): the kernel refuses, and the refusal is recorded.
-/
open Lean Elab Command
namespace LK_NML
universe u v w
inductive Entry (α : Type u) (β : Type v) (σ : Type w) where
  | entry (key : α) (val : β) : Entry α β σ
  | ref (node : σ) : Entry α β σ
  | null : Entry α β σ
inductive Node (α : Type u) (β : Type v) : Type (max u v) where
  | entries (es : Array (Entry α β (Node α β))) : Node α β
  | collision (ks : Array α) (vs : Array β) : Node α β
def Node.width {α : Type u} {β : Type v} : Node α β → Nat
  | .entries es => es.size
  | .collision ks _ => ks.size
theorem widthEmpty (α : Type u) (β : Type v) : (Node.entries (α := α) (β := β) #[]).width = 0 := rfl
theorem nodeEntriesEmpty :
    (Lean.PersistentHashMap.Node.entries (α := Nat) (β := Nat) #[]) =
      Lean.PersistentHashMap.Node.entries #[] := rfl
inductive LBox (α : Type u) : Type (max u 1) where
  | mk : α → LBox α
end LK_NML
run_cmd liftTermElabM do
  let badT := mkConst `LK_NML.BadT
  let field := mkApp (.const ``LK_NML.LBox [.zero]) badT
  let decl := Declaration.inductDecl [] 0 [
    { name := `LK_NML.BadT, type := .sort 1,
      ctors := [{ name := `LK_NML.BadT.mk, type := .forallE `b field badT .default }] }] false
  match (← getEnv).addDecl (← getOptions) decl with
  | .ok _ => logInfo "BadT: kernel ACCEPTED"
  | .error e => logInfo m!"BadT: kernel REJECTED: {e.toMessageData (← getOptions)}"
