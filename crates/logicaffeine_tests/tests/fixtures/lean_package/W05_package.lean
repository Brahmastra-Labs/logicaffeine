/-!
W05 proof-package fixtures (LOGOS lane LD, 2026-09-22). Every declaration below is accepted by the
pinned Lean 4.14.0; the file is compiled and exported with the pinned lean4export (structural
expression keys) so each package test starts from a genuine Lean artifact.

* `clean`       — a theorem proved through an opaque lemma and an alias definition.
* `tainted`     — the same shape, with `sorry` two declarations away (`lemmaAdmitted` → `lemmaAlias`).
* `byGoalAxiom` — a theorem "proved" by an axiom that states its goal.
* `usesPropext`, `usesFunext`, `usesChoice` — each rests on exactly one of Lean's standard axioms.
* `restatesPropext` — a theorem whose statement IS `propext`'s, proved by `propext`: a standard
  axiom restated is not a new axiom assumed.
-/

namespace W05

theorem symm' {α : Sort u} {a b : α} (h : a = b) : b = a := h ▸ rfl

def symmAlias {α : Sort u} {a b : α} (h : a = b) : b = a := symm' h

theorem clean (α : Type) (a b : α) (h : a = b) : b = a := symmAlias h

theorem lemmaAdmitted (α : Type) (a b : α) : a = b := sorry

def lemmaAlias (α : Type) (a b : α) : a = b := lemmaAdmitted α a b

theorem tainted (α : Type) (a b : α) : a = b := lemmaAlias α a b

axiom goalAx (α : Type) (a b : α) : a = b

theorem byGoalAxiom (α : Type) (a b : α) : a = b := goalAx α a b

theorem usesPropext (p q : Prop) (h : p ↔ q) : p = q := propext h

theorem usesFunext (α β : Type) (f g : α → β) (h : ∀ x, f x = g x) : f = g := funext h

theorem usesChoice (α : Type) (h : Nonempty α) : ∃ x : α, x = Classical.choice h := ⟨_, rfl⟩

theorem restatesPropext : ∀ {a b : Prop}, (a ↔ b) → a = b := propext

end W05
