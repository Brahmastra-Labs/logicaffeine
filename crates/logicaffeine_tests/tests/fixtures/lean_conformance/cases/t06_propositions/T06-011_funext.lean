/-!
@case      T06-011
@tier      6 · Propositions, equality, classical reasoning
@title     `funext` — ABSENT from the LOGOS trusted base; pointwise equality does not imply
           equality of functions
@lean      funext, Function.funext, extensionality of dependent functions, `@[ext]` lemmas
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/prelude.rs:33-44 (`StandardLibrary::register` — the complete trusted
           base); kernel/src/prelude.rs:46-78 (`register_classical` adds exactly one
           declaration, `dne`); a grep of `crates/logicaffeine_kernel/src` for `funext` or
           `ext` returns NOTHING; kernel/src/type_checker.rs:1391-1417 (`try_structure_eta` —
           the ONLY extensionality-flavoured rule in the kernel, and it is for STRUCTURES, not
           functions, and lives only inside `def_eq_normalized` at :1473-1478, never in
           `normalize`); kernel/src/type_checker.rs:1499-1541 (`def_eq_normalized`'s structural
           congruence for `Lambda` compares bodies after renaming ONE binder to the other's
           name — this is alpha-equivalence and eta for *syntactically* lambda-headed terms, NOT
           extensionality: two lambdas with definitionally-equal-but-not-reducibly-equal bodies
           are compared pointwise on a VARIABLE, which is exactly the case funext is needed for
           and exactly the case that fails); kernel/src/certificate.rs:28,63-75
@diag      E_AXIOM_NOT_IN_TRUSTED_BASE — must name `funext`, state that the LOGOS trusted base
           contains exactly one classical axiom (`dne`, prelude.rs:78), and list the
           declarations that depend on it. Must NOT suggest emitting it.
@guard     The trusted-base expansion trap, as in T06-009/T06-010: `add_declaration("funext",
           …)` is one line, compiles, typechecks locally, and silently makes the LOGOS trusted
           base something other than what `PRELUDE_VERSION = "logos-coc-1"`
           (certificate.rs:28) names. Because `certificate::recheck` rebuilds the context from
           `StandardLibrary::register` alone (certificate.rs:73-75), the resulting certificate
           fails re-check with a bare `UnboundVariable("funext")` — a message that reads like a
           name-resolution bug and will be "fixed" by patching the prelude, at which point
           the version guard at certificate.rs:65 passes while describing the wrong axiom set.
@guard     Catches the MOST plausible wrong move in this whole tier: believing LOGOS already has
           funext because `def_eq` compares lambdas pointwise. It does not. The `Lambda`/`Lambda`
           arm of `def_eq_normalized` (type_checker.rs:1530-1535) renames one binder and
           recurses on the BODIES — so it proves `(λx. e₁) ≡ (λx. e₂)` exactly when `e₁ ≡ e₂`
           as open terms under `normalize` (fuel 10000, reduction.rs:52-69). That is
           intensional. `funext` is needed precisely when the bodies are equal only because of a
           HYPOTHESIS (`h : ∀ x, f x = g x`), which `normalize` cannot use. `pointwiseNotDefEq`
           below is the discriminating instance: `Nat.add n 0` and `n` are propositionally but
           not definitionally equal in Lean, so the two functions are NOT `def_eq` in LOGOS and
           `funext` is genuinely required.
@guard     Catches substituting `Quot.sound` for `funext`. `Quot_sound` IS in the prelude
           (prelude.rs:1288-1320) and it does identify terms, so a translator may be tempted to
           quotient the function type by pointwise equality and push everything through
           `Quot_lift`. That changes the TYPE of every function it touches — `Quot A r` is
           declared OPAQUE (`add_declaration`, prelude.rs:1248-1252), not an inductive, so the
           quotiented function can no longer be applied at all.
@guard     Catches emitting `funext` only for non-dependent functions "because the dependent
           case is rarer". Lean's `funext` is `{β : α → Sort v}`-dependent; a monomorphic
           `(Nat → Nat)` axiom is a DIFFERENT, weaker axiom, so a translator that emits it has
           both expanded the trusted base AND failed to cover `dependentFunext` below.
@guard     Catches the scale error. `funext` is the third of Lean 4's core axioms. Every `ext`
           lemma, every `Set`/`Finset` equality, every `Matrix` and `Polynomial` argument in
           Mathlib goes through it.
@oracle    Translator exits non-zero with E_AXIOM_NOT_IN_TRUSTED_BASE for every declaration
           below and performs zero `add_declaration` calls. A harness additionally checks that
           `pointwiseNotDefEq` is NOT closed by an appeal to LOGOS `def_eq`.
-/

namespace T06_011

/-- The axiom itself, in its dependent form. -/
theorem ext {α : Sort u} {β : α → Sort v} {f g : (x : α) → β x}
    (h : ∀ x, f x = g x) : f = g := funext h

/-- The non-dependent specialisation. A translator that axiomatises only this has emitted a
    strictly weaker axiom AND still expanded the trusted base. -/
theorem extSimple {f g : Nat → Nat} (h : ∀ n, f n = g n) : f = g := funext h

/-- THE DISCRIMINATING INSTANCE. `fun n => n + 0` and `fun n => n` are pointwise equal, but the
    proof of `n + 0 = n` is by induction — it is not definitional, so LOGOS's `def_eq` (which
    only normalizes, reduction.rs:52-69) cannot see it. Anything that closes this goal is
    funext, however it is spelled. -/
theorem pointwiseNotDefEq : (fun n : Nat => n + 0) = (fun n : Nat => n) :=
  funext (fun n => Nat.add_zero n)

/-- The dependent case, where `β` genuinely varies. -/
theorem dependentFunext (β : Nat → Type) (f g : (n : Nat) → β n)
    (h : ∀ n, f n = g n) : f = g := funext h

/-- Function extensionality at a HIGHER-ORDER argument — the shape `Set`/`Filter` reasoning
    in Mathlib actually uses. -/
theorem higherOrderExt (F G : (Nat → Nat) → Nat) (h : ∀ f, F f = G f) : F = G := funext h

/-- An innocuous-looking lemma whose elaborated proof term mentions `funext`. The Lean source
    never types the word. -/
theorem innocuous : (fun n : Nat => n * 1) = (fun n : Nat => n) := by
  funext n
  exact Nat.mul_one n

/-- Predicate extensionality needs `funext` AND `propext` (T06-010) together. -/
theorem predExt (p q : Nat → Prop) (h : ∀ n, p n ↔ q n) : p = q :=
  funext (fun n => propext (h n))

end T06_011
