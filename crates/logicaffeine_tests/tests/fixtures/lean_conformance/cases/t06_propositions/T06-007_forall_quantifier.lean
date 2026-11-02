/-!
@case      T06-007
@tier      6 · Propositions, equality, classical reasoning
@title     Universal quantification — `∀` is `Term::Pi`, and its result sort comes from `imax`
@lean      ∀ binder over a Prop-valued body, Pi into Prop, impredicative quantification
@expect    translate
@target    `∀ (x : α), P x` ⟼ `Term::Pi { param: "x", param_type: <α>, body_type: <P x> }`.
           There is no separate `Forall` constant: universal quantification over a proposition
           IS the dependent function type, exactly as in Lean. The result sort is computed by
           `Universe::imax(sort α, sort (P x))`, which collapses to `Prop` whenever the body is
           a `Prop` — so quantification over ANY domain, including `Type 1`, still yields a
           `Prop`. That is impredicativity and LOGOS has it.
@evidence  kernel/src/term.rs:463 (`Term::Pi { param, param_type, body_type }`);
           kernel/src/type_checker.rs:108 (Pi formation computes the result sort with `imax`,
           matching Lean); kernel/src/term.rs:167-191 (`Universe::IMax` is a first-class
           universe former); kernel/src/prelude.rs:53-78 (`dne`'s own type is a `Pi` whose
           `param_type` is `Sort(Universe::Prop)` — the prelude itself relies on impredicative
           quantification over `Prop`)
@guard     Catches emitting `param: "_"` for a DEPENDENT `∀`. LOGOS's `Display` renders a `Pi`
           whose `param == "_"` as a non-dependent arrow (term.rs:585-590), so the mistake is
           invisible in printed output while the body still contains `Var("_")`, which the next
           `"_"`-named binder will capture. Lean's binder names are hints carried alongside de
           Bruijn indices, so the translator must decide from OCCURRENCE, not from the name.
@guard     Catches emitting a `Lambda` where a `Pi` is meant. Both variants have the same three
           fields with nearly the same names (`body` vs `body_type`, term.rs:463-470), and a
           `∀` whose body is a `Prop` and a `fun` whose body is a proof are easy to transpose in
           a generic lowering. A `Lambda` in type position infers a `Pi` type, so `infer_type`
           succeeds and the error only surfaces much later as an unexplainable mismatch.
@guard     Catches *hard-coding* the result sort instead of using `imax`. `∀ α : Type, P α` is
           a `Prop` in Lean and must be one in LOGOS. A translator that computes the result
           universe as `max` rather than `imax` puts `bigDomainProp` below in `Type 1`, at which
           point the proposition is no longer a `Prop` and proof irrelevance
           (type_checker.rs:1547-1567) stops applying to its proofs — silently, because LOGOS is
           cumulative (type_checker.rs:1330-1360) and a `Type 1` "proposition" still typechecks
           everywhere a `Prop` was expected downstream of a `Sort`-valued binder.
@guard     Catches the shadowing trap in `shadowed` below. Two nested `∀` binders may share a
           name in Lean (they are de Bruijn); in LOGOS's NAMED representation the inner one
           shadows the outer, and `substitute_avoiding` (type_checker.rs:1088) alpha-renames
           only when a replacement's free variables collide. Nested same-name binders must be
           freshened at emit time.
@oracle    `infer_type` on each emitted `Pi` returns a `Sort`, and for every declaration whose
           Lean type is a `Prop` the inferred `Sort` normalizes to `Sort(Universe::Prop)`; no
           emitted dependent `Pi` has `param == "_"`.
-/

namespace T06_007

-- `shadowed` below deliberately re-binds `n`, which the unused-variable linter flags.
set_option linter.unusedVariables false

/-- The baseline: a universally quantified proposition. -/
theorem forallRefl : ∀ n : Nat, n = n := fun n => rfl

/-- The binder is genuinely used, so `param` must be a real name. -/
theorem forallDependent (P : Nat → Prop) (h : ∀ n, P n) : P 7 := h 7

/-- A non-dependent `∀`, i.e. implication. `param` may be `"_"` here and only here. -/
theorem implication (p q : Prop) (h : ∀ _ : p, q) (hp : p) : q := h hp

/-- IMPREDICATIVITY: quantification over `Prop` itself still lands in `Prop`. This is what
    `imax` buys, and it is the shape of the prelude's own `dne` axiom. -/
theorem quantifyOverProp : ∀ p : Prop, p → p := fun _ hp => hp

/-- Quantification over `Type`. `imax (Type 1) Prop = Prop`, so this is a `Prop`; computing
    `max` instead would put it in `Type 1`. -/
theorem bigDomainProp : ∀ α : Type, α = α := fun _ => rfl

/-- Nested binders sharing a name. Legal in Lean; requires freshening in a named target. -/
theorem shadowed : ∀ n : Nat, ∀ n : Nat, n = n := fun _ n => rfl

/-- Alternating `∀`/`∃`, so the emitted `Pi` and the `Ex` predicate lambda interleave. -/
theorem alternating : ∀ n : Nat, ∃ m : Nat, ∀ k : Nat, n + 0 = n ∧ m = n ∧ k = k :=
  fun n => ⟨n, fun k => ⟨rfl, rfl, rfl⟩⟩

/-- A `∀` whose domain is itself a `∀` — the binder type is a `Pi`, so the lowering recurses
    into `param_type` as well as `body_type`. -/
theorem higherOrder (P : Nat → Prop) : (∀ n : Nat, P n) → P 0 := fun h => h 0

end T06_007
