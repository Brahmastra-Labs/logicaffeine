/-!
@case      T06-006
@tier      6 · Propositions, equality, classical reasoning
@title     Existential quantification — `Ex` is in the prelude but pinned to `Type 0`
@lean      Exists, Exists.intro, Exists.elim, ∃ binder notation, nested and dependent ∃
@expect    translate
@target    `∃ x : α, P x` ⟼ `App(App(Global("Ex"), <α>), Lambda{param:"x", param_type:<α>,
           body:<P x>})` — note the predicate is a LAMBDA argument, not a second binder.
           `⟨w, h⟩` ⟼ `App(App(App(App(Global("witness"), <α>), <P>), <w>), <h>)`.
@evidence  kernel/src/prelude.rs:1910-1926 (`register_ex`:
           `Ex : Π(A : Type 0). (A → Prop) → Prop`, registered via `add_inductive` so the FULL
           arity is parameters — context.rs:253); kernel/src/prelude.rs:1928-1983
           (`witness : Π(A : Type 0). Π(P : A → Prop). Π(x : A). P x → Ex A P`, all four
           arguments EXPLICIT); kernel/src/type_checker.rs:1688-1716 (`is_subsingleton_prop`:
           `Ex` has one constructor, but its non-arity argument `x : A` lives in `Type 0`, not
           `Prop`, so `Ex` is NOT a subsingleton) and kernel/src/recursor.rs:64-71 (so
           `derive_recursor` pins `Ex`'s motive to `Sort(Prop)` — no large elimination)
@guard     Catches emitting the constructor under Lean's name. LOGOS calls it `witness`, not
           `Exists.intro` and not `intro`; lookup is by flat `String` (context.rs:449).
@guard     Catches emitting `∃ x, P x` as a two-binder application `Ex α x (P x)`. LOGOS's `Ex`
           takes the predicate as a single `A → Prop` FUNCTION argument, so the binder must be
           reified into a `Term::Lambda`. A translator that forgets this produces a term whose
           `Var("x")` is dangling; because substitution is capture-avoiding by alpha-renaming
           over a NAMED representation (type_checker.rs:1088), the dangling variable will
           sometimes be captured by an unrelated enclosing binder of the same name rather than
           erroring — a silent change of meaning.
@guard     Catches translating an existential over a non-`Type 0` domain. `Ex`'s `A` is
           `Sort(Type 0)` (prelude.rs:1912) while Lean's is `{α : Sort u}`. `∃ p : Prop, …`
           and `∃ α : Type, …` — both ordinary Lean — have no translation: `Prop : Type 1` in
           LOGOS (term.rs:198) and `Type 1 ≤ Type 0` is false even under cumulativity
           (type_checker.rs:1330-1360). `bigDomain` and `propDomain` below must be escalated.
@guard     Catches assuming `Exists.elim` can produce data. LOGOS's derived `Ex` eliminator has
           a `Sort(Prop)` motive (recursor.rs:64-71 via type_checker.rs:1688-1716), so
           `Exists.elim` into `Type` is unavailable — and there is no `Classical.choose` to
           recover it, because `Classical.choice` is absent (T06-009). Lean is the same here for
           the non-classical fragment, so this is not a divergence — but a translator that
           silently routes `Exists.choose` through a fresh axiom IS the T06-009 bug.
@oracle    Emitted terms typecheck; the constructor global is `witness`; the predicate argument
           is a `Term::Lambda`; and `propDomain`/`bigDomain` are escalated rather than emitted.
-/

namespace T06_006

/-- The baseline: an existential over a `Type 0` domain. -/
theorem existsNat : ∃ n : Nat, n = 3 := Exists.intro 3 rfl

/-- Anonymous-constructor spelling — the same `witness` application after elaboration. -/
theorem existsAnon : ∃ n : Nat, n + 1 = 4 := ⟨3, rfl⟩

/-- Elimination into a `Prop` motive, which is all LOGOS's derived eliminator allows. -/
theorem existsElim (P : Nat → Prop) (h : ∃ n, P n) (Q : Prop) (k : ∀ n, P n → Q) : Q :=
  Exists.elim h k

/-- Nested existentials: the inner `Ex` is inside the outer one's predicate lambda. -/
theorem existsNested : ∃ m : Nat, ∃ n : Nat, m + n = 5 := ⟨2, 3, rfl⟩

/-- An existential whose predicate itself mentions an equation — the predicate lambda's body
    is a full `Eq` application, so the reification must be structural, not syntactic. -/
theorem existsWithEq (f : Nat → Nat) (h : f 0 = 7) : ∃ n : Nat, f n = 7 := ⟨0, h⟩

/-- An existential under a universal — exercises binder interleaving. -/
theorem forallExists : ∀ n : Nat, ∃ m : Nat, m = n + 1 := fun n => ⟨n + 1, rfl⟩

/-- BOUNDARY. The domain is `Prop`, which in LOGOS has type `Type 1`. No translation. -/
theorem propDomain : ∃ p : Prop, p := ⟨True, True.intro⟩

/-- BOUNDARY. The domain is `Type`, i.e. `Type 0`, whose own type is `Type 1`. No translation. -/
theorem bigDomain : ∃ α : Type, α = Nat := ⟨Nat, rfl⟩

end T06_006
