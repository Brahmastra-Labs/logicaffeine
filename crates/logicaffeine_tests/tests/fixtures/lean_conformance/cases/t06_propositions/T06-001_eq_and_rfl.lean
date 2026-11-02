/-!
@case      T06-001
@tier      6 · Propositions, equality, classical reasoning
@title     Propositional equality `Eq` and its constructor `rfl`
@lean      Eq, Eq.refl, rfl, definitional unfolding under rfl
@expect    translate
@target    `Eq` is already in the LOGOS prelude as an INDEXED inductive:
           `Context::add_indexed_inductive("Eq", Π(A:Type 0). A → A → Prop, 2)` with the single
           constructor `refl : Π(A:Type 0). Π(x:A). Eq A x x`. So `@Eq α a b` ⟼
           `App(App(App(Global("Eq"), <α>), <a>), <b>)` and `@rfl α a` ⟼
           `App(App(Global("refl"), <α>), <a>)`. No new declaration is emitted.
@evidence  kernel/src/prelude.rs:1422-1439 (`Eq : Π(A : Type 0). A → A → Prop`, registered via
           `add_indexed_inductive` with `num_params = 2` — `A` and `x` are parameters, the
           trailing slot is the INDEX); kernel/src/prelude.rs:1441-1460 (`refl` registered by
           `add_constructor("refl", "Eq", Π(A:Type 0). Π(x:A). Eq A x x)`);
           kernel/src/context.rs:374 (`add_indexed_inductive` is the params/indices-splitting
           entry point, distinct from `add_inductive` at :253 which treats the FULL arity as
           parameters)
@guard     Catches the translator re-declaring `Eq` instead of reusing the prelude's. A fresh
           `add_inductive("Eq", …)` would (a) shadow nothing — `get_global` resolves inductives
           first (context.rs:449) so the SECOND registration wins for constructors but the
           already-derived `Eq_rec_dep`, `Eq_rec`, `Eq_sym` and `Eq_trans` definitions
           (prelude.rs:1481-1783) still refer to the first — and (b) set `num_params` to 3
           instead of 2, which makes `derive_recursor` build a J whose motive telescope has no
           index binder at all (recursor.rs:56-71). Every equality proof downstream then checks
           against the wrong eliminator.
@guard     Catches dropping Lean's IMPLICIT `{α : Sort u}` on `Eq.refl`. LOGOS's `refl` binds
           `A` and `x` as ordinary explicit `Pi` params and the prelude never calls
           `set_binder_kinds` for it, so an under-applied `refl` infers as a `Pi`, not an `Eq`.
           The dangerous repair is emitting `Term::Hole` for `A`: `types_equal` treats `Hole` as
           matching ANYTHING (type_checker.rs:1594-1597), so `refl Hole a : Eq ? a a` would be
           accepted against a completely unrelated equation.
@guard     Catches translating an equation at a type that is NOT in `Type 0`. LOGOS's `Eq`
           pins its carrier to `Sort(Type 0)` (prelude.rs:1425) while Lean's is
           `{α : Sort u}`. `Prop : Type 1` in LOGOS (term.rs:198), so `Eq Prop p q` — the
           translation of `p = q` for `p q : Prop`, which is ordinary Lean — does not typecheck:
           `Type 1 ≤ Type 0` is false even under LOGOS cumulativity
           (type_checker.rs:1330-1360). The translator must detect this and escalate, not emit
           an `Eq` application it never typechecks.
@oracle    For each equation in this file whose carrier is a `Type 0` inhabitant, the emitted
           term typechecks under `infer_type` and the inferred type is the translation of the
           Lean type; the translator adds ZERO new declarations named `Eq` or `refl`; and the
           `propEq` / `largeEq` declarations below are escalated rather than emitted.
-/

namespace T06_001

/-- The baseline: `rfl` at a concrete `Type 0` carrier. -/
theorem natRefl : (3 : Nat) = 3 := rfl

/-- `rfl` closing a gap that is definitional but not syntactic. LOGOS must reach this by
    `def_eq_normalized`, which for `Nat` goes through the `Literal::Nat` ↔ `Succ^n Zero`
    bridge (type_checker.rs:1480-1493). -/
theorem natCompute : (2 : Nat) + 2 = 4 := rfl

/-- Explicit constructor form — this is what the translator actually sees in `Expr`. -/
theorem explicitRefl : @Eq Nat 3 3 := @Eq.refl Nat 3

/-- Equality at a function type. Still `Type 0`-inhabited, so still translatable. -/
theorem funRefl : (fun n : Nat => n) = (fun n : Nat => n) := rfl

/-- BOUNDARY. Lean's `Eq` is `{α : Sort u}`, so an equation between two `Prop`s is ordinary.
    LOGOS's `Eq` carrier is `Type 0`, and `Prop : Type 1` there — no translation exists. -/
theorem propEq : (True = True) := rfl

/-- BOUNDARY. Same reason, one level up: the carrier lives in `Type 1`. -/
theorem largeEq : (Nat : Type) = (Nat : Type) := rfl

/-- A universe-polymorphic equality lemma. Not only is the carrier not `Type 0`, there is
    nowhere on a LOGOS inductive to bind `u` at all (see T01-007). -/
theorem polyRefl {α : Sort u} (a : α) : a = a := rfl

end T06_001
