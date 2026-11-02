/-!
@case      T06-003
@tier      6 · Propositions, equality, classical reasoning
@title     `Eq.symm`, `Eq.trans`, `congrArg` — two are in the prelude, one must be derived
@lean      Eq.symm, Eq.trans, congrArg, congrFun, congr
@expect    translate
@target    `Eq.symm`  ⟼ `Global("Eq_sym")`   — prelude DEFINITION, derived from J
           `Eq.trans` ⟼ `Global("Eq_trans")` — prelude DEFINITION, derived from J
           `congrArg` ⟼ a NEW `Context::add_definition("congrArg", …)` whose body is an
           `Eq_rec_dep` instance at motive `λ(y:A). λ(_ : Eq A x y). Eq B (f x) (f y)`,
           with base `refl B (f x)`. There is no `congrArg` in the LOGOS prelude.
@evidence  kernel/src/prelude.rs:1597 (`register_eq_sym` — `Eq_sym` is an `add_definition`
           "DERIVED from J", not an axiom); kernel/src/prelude.rs:1684 (`register_eq_trans`,
           likewise a definition); kernel/src/prelude.rs:1422-1478 (`register_eq` registers
           exactly `Eq`, `refl`, `Eq_rec_dep`, `Eq_rec`, `Eq_sym`, `Eq_trans` — no congruence
           lemma of any kind); kernel/src/cc.rs:247-261 (the only "congruence" in the kernel is
           a congruence-CLOSURE decision procedure used by the `cc` tactic — it is not a term
           and cannot be applied to an equation proof); kernel/src/context.rs:442
           (`add_definition` — the correct emission path for the derived lemma)
@guard     Catches the translator axiomatising `congrArg` because "it is a primitive in Lean
           too". It is not: Lean proves `congrArg` from `Eq.rec`, and LOGOS can too. Emitting
           `add_declaration("congrArg", …)` adds a proposition to the trusted base that
           `certificate::recheck` will not have (it rebuilds from `StandardLibrary::register`,
           certificate.rs:73-75) while `PRELUDE_VERSION` still claims `"logos-coc-1"`
           (certificate.rs:28).
@guard     Catches mapping `Eq.symm`/`Eq.trans` to freshly derived definitions instead of the
           prelude's `Eq_sym`/`Eq_trans`. Name collision is the failure mode: LOGOS keys are
           flat `String`s with no namespaces (context.rs:449), so an emitted `Eq_sym` would
           REPLACE the prelude's in the `definitions` map, and every prelude lemma already
           elaborated against the original keeps pointing at a body that no longer matches
           the name. Reuse, do not re-derive.
@guard     Catches dropping the explicit carrier arguments. `Eq_sym`'s telescope is
           `Π(A)(x)(y). Eq A x y → Eq A y x` with `A`, `x`, `y` all EXPLICIT `Pi` binders
           (prelude.rs:1597-1683), whereas Lean's `Eq.symm` is `{α} {a b}` — three implicits.
           A translator that copies Lean's argument count emits `Eq_sym h`, which infers as a
           `Pi`, not an equation. Filling the gap with `Term::Hole` is worse: `types_equal`
           matches `Hole` against anything (type_checker.rs:1594-1597), so `Eq_sym Hole Hole
           Hole h` would be accepted against an unrelated equation.
@guard     `congrFun` is NOT derivable this way. `congrFun : f = g → ∀ a, f a = g a` is an
           `Eq_rec_dep` instance and is fine; it is `funext`, its converse, that is absent —
           see T06-011. A translator must not conflate the two directions.
@oracle    Emitted terms typecheck; the translator emits exactly one new global for `congrArg`
           via `add_definition` and zero via `add_declaration`; and `Eq_sym`/`Eq_trans` are
           referenced, not redefined.
-/

namespace T06_003

/-- Symmetry. Maps onto the prelude's derived `Eq_sym`. -/
theorem sym {α : Type} {a b : α} (h : a = b) : b = a := Eq.symm h

/-- Transitivity. Maps onto the prelude's derived `Eq_trans`. -/
theorem trans {α : Type} {a b c : α} (h₁ : a = b) (h₂ : b = c) : a = c := Eq.trans h₁ h₂

/-- Congruence for a unary function — must be emitted as a NEW derived definition. -/
theorem congrArgNat (f : Nat → Nat) {a b : Nat} (h : a = b) : f a = f b := congrArg f h

/-- Congruence across two different carriers, so the derived definition really does need
    both `A` and `B` bound. -/
theorem congrArgCross {α β : Type} (f : α → β) {a b : α} (h : a = b) : f a = f b :=
  congrArg f h

/-- `congrFun` — the sound direction. Derivable from J; do not confuse with `funext`. -/
theorem congrFunNat {f g : Nat → Nat} (h : f = g) (a : Nat) : f a = g a := congrFun h a

/-- Full `congr`, which is `congrArg` composed with `congrFun`. -/
theorem congrBoth {α β : Type} {f g : α → β} {a b : α} (hf : f = g) (ha : a = b) :
    f a = g b := congr hf ha

/-- Chained symm/trans — exercises that the prelude lemmas compose, i.e. that the emitted
    terms really are the prelude globals and not four private copies. -/
theorem chained {α : Type} {a b c : α} (h₁ : a = b) (h₂ : c = b) : a = c :=
  Eq.trans h₁ (Eq.symm h₂)

end T06_003
