import Lean
/-!
Lane LF — the quotient computation rule behind a DEFINED head.

Arena `mathlib`: the primary kernel accepted Lean core's `Quotient.lift₂._proof_2` and the
independent re-checker called it ill-typed. The proof needs `Quotient.lift f h (Quotient.mk s a)`
to reduce to `f a`, and that happens only after δ: `Quotient.lift` unfolds to the PARTIAL
primitive `Quot.lift f`, so the respect proof and the major premise reach `Quot.lift` from the
enclosing application, and `Quotient.mk s a` unfolds to `Quot.mk Setoid.r a`.

Every theorem in namespace `LF_QL` is accepted by Lean 4.14.0's kernel. The corrupted
statements at the end are REJECTED by Lean's kernel (`addDecl`; the messages are logged when this
file is compiled); they are then added with `debug.skipKernelTC` only so that the exporter can hand
them to LOGOS as corruption controls.
-/
open Lean Elab Command Meta
namespace LF_QL
universe u v w

/-- The lift of a representative, through `Quotient.lift`'s definition. -/
theorem liftMk {α : Sort u} {β : Sort v} [s : Setoid α] (f : α → β)
    (h : ∀ a b, a ≈ b → f a = f b) (a : α) :
    Quotient.lift f h (Quotient.mk s a) = f a := rfl

/-- Two δ-steps deep: `Quotient.liftOn` unfolds to `Quotient.lift`, which unfolds to `Quot.lift`. -/
theorem liftOnMk {α : Sort u} {β : Sort v} [s : Setoid α] (f : α → β)
    (h : ∀ a b, a ≈ b → f a = f b) (a : α) :
    Quotient.liftOn (Quotient.mk s a) f h = f a := rfl

/-- The binary lift on two representatives: the outer lift reduces to the inner one, which reduces
in turn. Its export carries Lean core's `Quotient.lift₂` and its auxiliary proofs. -/
theorem lift₂Mk {α : Sort u} {β : Sort v} {γ : Sort w} [s₁ : Setoid α] [s₂ : Setoid β]
    (f : α → β → γ)
    (c : ∀ a₁ b₁ a₂ b₂, a₁ ≈ a₂ → b₁ ≈ b₂ → f a₁ b₁ = f a₂ b₂) (a : α) (b : β) :
    Quotient.lift₂ f c (Quotient.mk s₁ a) (Quotient.mk s₂ b) = f a b := rfl

/-- The dependent eliminator: `Quot.rec` is a definition over `Quot.lift` and a projection. -/
theorem recMk {α : Sort u} {r : α → α → Prop} {motive : Quot r → Sort v}
    (f : (a : α) → motive (Quot.mk r a))
    (h : ∀ (a b : α) (p : r a b), (Quot.sound p ▸ f a : motive (Quot.mk r b)) = f b) (a : α) :
    Quot.rec f h (Quot.mk r a) = f a := rfl

/-- The primitive spine itself, with no δ in front of it. -/
theorem primLiftMk {α : Sort u} {β : Sort v} (r : α → α → Prop) (f : α → β)
    (h : ∀ a b, r a b → f a = f b) (a : α) :
    Quot.lift f h (Quot.mk r a) = f a := rfl

end LF_QL

/-- Ask Lean's kernel, record its verdict, then add the declaration without kernel checking. -/
def refuteThenAdmit (name : Name) (stmt proof : TSyntax `term) : CommandElabM Unit := do
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

/- The rule applied to the right representative, claimed at the wrong one. -/
run_cmd do
  let stmt ← `(∀ {α β : Type} [s : Setoid α] (f : α → β) (h : ∀ a b, a ≈ b → f a = f b) (a b : α),
    Quotient.lift f h (Quotient.mk s a) = f b)
  let proof ← `(fun {α β : Type} [s : Setoid α] (f : α → β) (h : ∀ a b, a ≈ b → f a = f b) (a b : α) =>
    @rfl β (f b))
  refuteThenAdmit `LF_QL.liftMkWrongRep stmt proof

/- The binary lift with its two representatives exchanged. -/
run_cmd do
  let stmt ← `(∀ {α γ : Type} [s : Setoid α] (f : α → α → γ)
    (c : ∀ a₁ b₁ a₂ b₂, a₁ ≈ a₂ → b₁ ≈ b₂ → f a₁ b₁ = f a₂ b₂) (a b : α),
    Quotient.lift₂ f c (Quotient.mk s a) (Quotient.mk s b) = f b a)
  let proof ← `(fun {α γ : Type} [s : Setoid α] (f : α → α → γ)
    (c : ∀ a₁ b₁ a₂ b₂, a₁ ≈ a₂ → b₁ ≈ b₂ → f a₁ b₁ = f a₂ b₂) (a b : α) =>
    @rfl γ (f b a))
  refuteThenAdmit `LF_QL.lift₂MkSwapped stmt proof

