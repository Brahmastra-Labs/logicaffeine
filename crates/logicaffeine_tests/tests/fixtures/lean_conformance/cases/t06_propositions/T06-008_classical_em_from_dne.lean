/-!
@case      T06-008
@tier      6 · Propositions, equality, classical reasoning
@title     `Classical.em` — derivable from LOGOS's single classical axiom `dne`
@lean      Classical.em, Classical.byContradiction, Classical.not_not, double-negation
@expect    translate
@target    `dne : Π(P:Prop). ((P → False) → False) → P` — the ONE classical axiom in the LOGOS
           trusted base, already registered. `Classical.byContradiction h` ⟼
           `App(App(Global("dne"), <p>), <h>)` directly. `Classical.em p` ⟼ a NEW
           `add_definition("Classical_em", Π(P:Prop). Or P (Not P), …)` whose body is
           `dne (Or P (Not P)) (λk. k (right P (Not P) (λhp. k (left P (Not P) hp))))` —
           the standard intuitionistic derivation of excluded middle from DNE, using the
           prelude's `Or`/`left`/`right` (prelude.rs:1847-1909).
@evidence  kernel/src/prelude.rs:46-52 (the doc comment: "double-negation elimination …
           This makes the logic CLASSICAL — exactly as Lean/Coq's Mathlib are classical via
           `Classical.em`/`choice` … It is an EXPLICIT, type-checked axiom in the trusted base,
           recorded in the certificate's axiom-set version"); kernel/src/prelude.rs:53-78
           (`register_classical` builds the type and calls `ctx.add_declaration("dne", …)` —
           this function registers exactly one declaration and nothing else);
           kernel/src/prelude.rs:1847-1909 (`Or` with constructors `left`/`right`);
           kernel/src/context.rs:442 (`add_definition` — the emission path for the derived `em`)
@guard     Catches emitting `Classical.em` as a SECOND axiom "because Lean has it as one". It
           is derivable, so `add_declaration("em", …)` is a gratuitous trusted-base expansion:
           `certificate::recheck` rebuilds the context from `StandardLibrary::register` alone
           (certificate.rs:73-75) and will not have it, so certificates mentioning it fail to
           re-check — while `PRELUDE_VERSION` still reports `"logos-coc-1"` (certificate.rs:28),
           i.e. the certificate's own record of what was trusted is wrong. Every classical step
           in the file must route through `dne` and nothing else.
@guard     Catches the direction confusion. `dne` is `¬¬P → P`, NOT `P ∨ ¬P` and NOT
           `Decidable p`. A translator that maps `Classical.em` to `dne` by name similarity
           emits a term of the wrong type; because the emitted term is applied to a
           `Term::Hole` for the proposition in the common case, `types_equal`'s Hole-matches-
           anything rule (type_checker.rs:1594-1597) can let the mismatch through.
@guard     Catches routing `Classical.propDecidable` through `dne`. Turning `p ∨ ¬p` into
           `Decidable p` requires CHOICE, not DNE — `Decidable` is `Type`-valued
           (prelude.rs:781) and `Or` is a two-constructor `Prop` whose derived eliminator is
           pinned to a `Sort(Prop)` motive (recursor.rs:64-71, type_checker.rs:1688-1716), so
           `Or`-elimination into `Decidable` is not available. That is T06-009/T06-018, and it
           must be rejected there rather than smuggled in here.
@guard     Catches `Not`-unfolding asymmetry. `dne`'s type is spelled out as
           `Π(P:Prop). ((P → False) → False) → P` with literal `Pi` binders named `"_"`
           (prelude.rs:56-77) — it does NOT mention the `Not` global. LOGOS's `Not` is a
           delta-transparent definition (prelude.rs:755) so `def_eq` bridges the two, but only
           after `normalize` unfolds it; a translator that emits `¬¬p` as `Not (Not p)` and
           then compares SYNTACTICALLY against `dne`'s type will report a spurious mismatch.
@oracle    Every classical step in the emitted output is an application of `Global("dne")`;
           the translator emits ZERO new `add_declaration`s; `Classical_em` is emitted once via
           `add_definition` and typechecks against `Π(P:Prop). Or P (Not P)`.
-/

namespace T06_008

/-- The axiom itself, in Lean's spelling. This is the only classical primitive LOGOS has. -/
theorem dne (p : Prop) (h : ¬¬p) : p := Classical.byContradiction h

/-- Excluded middle, derived from `dne` by the standard argument. Written out explicitly so
    the translator has the derivation and does not need to invent an axiom. -/
theorem em (p : Prop) : p ∨ ¬p :=
  dne (p ∨ ¬p) (fun k => k (Or.inr (fun hp => k (Or.inl hp))))

/-- The `Not`-unfolding boundary: `¬¬p` spelled as a bare arrow chain, which is what `dne`'s
    LOGOS type literally says. -/
theorem dneUnfolded (p : Prop) (h : (p → False) → False) : p := dne p h

/-- Double-negation introduction is INTUITIONISTIC — it must NOT be routed through `dne`. -/
theorem dni (p : Prop) (hp : p) : ¬¬p := fun k => k hp

/-- `¬¬p ↔ p`: only the forward direction is classical. -/
theorem notNotIff (p : Prop) : ¬¬p ↔ p := Iff.intro (dne p) (dni p)

/-- Proof by contradiction in its usual shape. -/
theorem byContra (p q : Prop) (h : ¬p → q) (hnq : ¬q) : p :=
  dne p (fun hnp => hnq (h hnp))

/-- Case analysis on `em` — the derived `Classical_em` feeding the prelude's `Or` eliminator
    at a `Prop` motive, which is the only motive its derived recursor allows. -/
theorem cases (p q : Prop) (hp : p → q) (hnp : ¬p → q) : q :=
  (em p).elim hp hnp

/-- De Morgan's law whose forward direction genuinely needs excluded middle. -/
theorem deMorgan (p q : Prop) (h : ¬(p ∧ q)) : ¬p ∨ ¬q :=
  (em p).elim (fun hp => Or.inr (fun hq => h ⟨hp, hq⟩)) Or.inl

end T06_008
