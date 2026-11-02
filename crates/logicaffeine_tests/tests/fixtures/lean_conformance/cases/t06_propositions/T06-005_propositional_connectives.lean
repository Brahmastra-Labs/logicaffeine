/-!
@case      T06-005
@tier      6 · Propositions, equality, classical reasoning
@title     `And` / `Or` / `Not` / `False` / `True` are in the prelude; `Iff` is not
@lean      And, Or, Iff, Not, False, True, their constructors and eliminators
@expect    translate
@target    `p ∧ q`   ⟼ `App(App(Global("And"), p), q)`,  `And.intro` ⟼ `Global("conj")`
           `p ∨ q`   ⟼ `App(App(Global("Or"),  p), q)`,  `Or.inl/inr` ⟼ `Global("left")`/`Global("right")`
           `¬p`      ⟼ `App(Global("Not"), p)` — a delta-TRANSPARENT definition `λP. P → False`
           `False`   ⟼ `Global("False")` (zero constructors)
           `True`    ⟼ `Global("True")`,  `True.intro` ⟼ `Global("I")`
           `p ↔ q`   ⟼ a NEW `add_definition("Iff", Π(P:Prop)(Q:Prop). Prop,
                          λP Q. And (P → Q) (Q → P))` plus derived `Iff_intro`/`Iff_mp`/`Iff_mpr`.
@evidence  kernel/src/prelude.rs:1804-1846 (`register_and`: `And : Prop → Prop → Prop` via
           `add_inductive`, constructor `conj : Π(P:Prop)(Q:Prop). P → Q → And P Q`);
           kernel/src/prelude.rs:1847-1909 (`register_or`, constructors named `left` and
           `right` — NOT `inl`/`inr`); kernel/src/prelude.rs:722-727 (`True` + constructor `I`);
           kernel/src/prelude.rs:729-733 (`False`, zero constructors);
           kernel/src/prelude.rs:735-755 (`Not` is an `add_definition`, body
           `λ(P:Prop). Π(_:P). False` — a DEFINITION, so delta-transparent, not an inductive);
           a grep of `crates/logicaffeine_kernel/src` for `Iff` returns NOTHING — there is no
           biconditional anywhere in the kernel or prelude
@guard     Catches emitting `Or.inl`/`Or.inr` verbatim. LOGOS's constructors are literally
           named `left` and `right` (prelude.rs:1847-1909) and lookup is by flat `String`
           (context.rs:449), so `Global("Or.inl")` resolves to nothing and `Global("inl")`
           resolves to nothing. Worse, `left` and `right` are ALSO the names of two tactic
           script primitives, so a translator that "helpfully" registers its own `inl` ends up
           with two constructor names for one inductive and `Match`'s positional case list
           (type_checker.rs:254-260) no longer lines up with registration order.
@guard     Catches treating `Not` as opaque. LOGOS's `Not` is an `add_definition`
           (prelude.rs:755), hence delta-transparent: `Not P` and `P → False` are definitionally
           equal and a proof of one is a proof of the other with NO coercion. A translator that
           emits `Not` as an `add_declaration` (axiom) makes `¬p` an opaque `Prop` with no
           inhabitants and no elimination rule — every `absurd`/`Not.elim` downstream becomes
           unprovable. Symmetrically, a translator that ALWAYS inlines `P → False` loses the
           head symbol that the instance-resolution `class_heads` scan and the `cases` tactic
           key on.
@guard     Catches assuming `Iff` exists. It does not. `p ↔ q` must be emitted as a new
           definition; emitting it as an `add_declaration` axiom would make the biconditional
           opaque (no `.mp`/`.mpr`), and emitting it as an `add_inductive` with an `Iff_intro`
           constructor would give a DIFFERENT proposition from `And (P→Q) (Q→P)` — one that
           `propext`-style reasoning cannot bridge, since `propext` is absent too (T06-010).
@guard     `And` is registered with `add_inductive` (prelude.rs:1816), which treats the FULL
           arity as parameters (context.rs:253). Both `P` and `Q` are therefore parameters and
           `derive_recursor` builds a zero-index eliminator. A translator re-registering `And`
           via `add_indexed_inductive` with a different `num_params` silently changes the
           recursor's telescope.
@guard     `Or` has TWO constructors, so `is_subsingleton_prop` is false (type_checker.rs:1716)
           and `derive_recursor` pins its motive to `Sort(Prop)` (recursor.rs:64-71): `Or`
           cannot be large-eliminated into `Type`. `And` and `False` CAN. Lean agrees on `Or`
           and `False`; a translator must not assume a uniform rule. See T06-015 and T01-014.
@oracle    Every emitted connective term typechecks; `Or` constructors are emitted as `left`/
           `right`; `Not` is referenced as the prelude DEFINITION; `Iff` is emitted exactly once
           via `add_definition` and never via `add_declaration` or `add_inductive`.
-/

namespace T06_005

/-- Conjunction: introduction and both eliminations. -/
theorem andBoth (p q : Prop) (hp : p) (hq : q) : p ∧ q := And.intro hp hq
theorem andLeft  (p q : Prop) (h : p ∧ q) : p := h.left
theorem andRight (p q : Prop) (h : p ∧ q) : q := h.right

/-- Disjunction: LOGOS names these constructors `left` and `right`. -/
theorem orIntroL (p q : Prop) (hp : p) : p ∨ q := Or.inl hp
theorem orIntroR (p q : Prop) (hq : q) : p ∨ q := Or.inr hq

/-- `Or` elimination — restricted to a `Prop` motive in LOGOS, because `Or` has two
    constructors and so is not a subsingleton. -/
theorem orElim (p q r : Prop) (h : p ∨ q) (hp : p → r) (hq : q → r) : r :=
  h.elim hp hq

/-- Negation. `Not p` is definitionally `p → False` in both systems. -/
theorem notIsArrow (p : Prop) (h : ¬p) (hp : p) : False := h hp
theorem arrowIsNot (p : Prop) (h : p → False) : ¬p := h

/-- `False` elimination — legal at ANY motive, including a large one, because `False` has zero
    constructors and is therefore a subsingleton in both systems. -/
theorem exFalso (p : Prop) (h : False) : p := False.elim h
def exFalsoLarge (h : False) : Nat := False.elim h

/-- `True` and its single constructor `I`. -/
theorem trueIntro : True := True.intro

/-- Biconditional — absent from LOGOS, must be emitted as `And (P→Q) (Q→P)`. -/
theorem iffIntro (p q : Prop) (hpq : p → q) (hqp : q → p) : p ↔ q := Iff.intro hpq hqp
theorem iffMp  (p q : Prop) (h : p ↔ q) (hp : p) : q := h.mp hp
theorem iffMpr (p q : Prop) (h : p ↔ q) (hq : q) : p := h.mpr hq

/-- The `Iff`/`And`-of-arrows correspondence the emitted definition must make definitional. -/
theorem iffAsAnd (p q : Prop) (h : p ↔ q) : (p → q) ∧ (q → p) := ⟨h.mp, h.mpr⟩

/-- A nested formula that exercises all five connectives at once. -/
theorem mixed (p q r : Prop) (h : (p ∧ q) ∨ (¬r)) (hr : r) : p ∨ True :=
  h.elim (fun hpq => Or.inl hpq.left) (fun hnr => absurd hr hnr)

end T06_005
