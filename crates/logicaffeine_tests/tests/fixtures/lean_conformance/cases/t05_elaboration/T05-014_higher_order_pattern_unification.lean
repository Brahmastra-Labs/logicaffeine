/-!
@case      T05-014
@tier      5 · Elaboration surface
@title     Higher-order pattern unification — Miller patterns only, on both sides
@lean      motive inference for `Eq.mpr`/`▸`/`Eq.subst`, `congrArg`, `?m x y =?= t`, dependent
           `match` motive synthesis
@expect    translate
@target    The SOLVED term with every motive written out as an explicit `Term::Lambda` —
           `h ▸ e` becomes an application of the translated `Eq` eliminator to a motive the
           translator took from Lean's `Expr`, never a `Hole` for LOGOS to re-solve.
@evidence  kernel/src/elaborate.rs:216-292 (`unify_in(ctx, mctx, lctx, a, b)`: metavariable
           assignment, then `try_pattern` in BOTH directions, then the η rule, then structural
           congruence — first-order unification is `unify` with an empty local context);
           kernel/src/elaborate.rs:294-340 (`try_pattern`: the head must be an UNSOLVED
           metavariable; EVERY argument must be a DISTINCT bound variable of `lctx`; a repeated
           argument returns `None`, a non-variable argument returns `None`, an occurs-check
           failure returns `Some(false)`, and an out-of-scope free variable on the right
           returns `Some(false)`);
           kernel/src/elaborate.rs:638-700 (`infer_match_motive` is the Miller solution of
           `?P disc =?= T`, by abstracting the discriminant out of the expected type);
           kernel/src/elaborate.rs:137-139,143-186 (metavariables are `?`-prefixed `Term::Var`s
           and must be `instantiate`d away before the kernel sees the term)
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax`.** Lean's
           unifier has already solved these motives; the `Expr` contains them explicitly. A
           translator on `Syntax` must re-solve them with LOGOS's unifier — which is a strictly
           weaker algorithm (pattern fragment only, no postponement, no unfolding heuristics) —
           and will fail on the cases below where Lean succeeded by a non-pattern
           approximation.
@guard     Catches relying on LOGOS to solve a NON-pattern constraint. `try_pattern`
           (elaborate.rs:294-340) returns `None` for a repeated argument (`?M x x =?= t`) and
           for a non-variable argument (`?M (f y) =?= t`), so unification falls through to
           structural congruence and simply FAILS. Lean solves both cases — by picking a
           canonical solution among several, which is why they are outside the pattern
           fragment. A translator that emits a `Hole` and expects LOGOS to fill it gets a hard
           error here, and a `CannotInferHole` (type_checker.rs:313) if the hole survives.
@guard     Catches the difference in what "distinct bound variables" means. `try_pattern`
           checks membership in the LOCAL CONTEXT `lctx` (elaborate.rs:313-325), and the
           top-level `unify` runs with an EMPTY `lctx` (elaborate.rs:216-219) — so at top level
           NO higher-order pattern is solvable at all, only first-order unification. A
           translator that calls `unify` rather than `unify_in` with a populated context gets
           first-order behaviour and silently different answers.
@guard     Catches the η rule's reach. `unify_in` (elaborate.rs:254-268) η-expands a λ against
           a non-λ, so `?M =?= fun x => t` and `?M x =?= t` can both succeed and assign
           DIFFERENT solutions depending on which side was tried first. Lean's answer is one
           specific one; the emitted term must be Lean's.
@guard     Catches an occurs-check difference. `try_pattern` returns `Some(false)` when the
           metavariable occurs in the right-hand side (elaborate.rs:327-329), aborting the
           unification rather than falling through — so a constraint Lean solved by unfolding a
           definition first will fail in LOGOS with no fallback.
@guard     Catches leaving the motive as a `Hole` in a `Match`. `fill_match_motives`
           (elaborate.rs:578-636) fills a `Hole` motive only from an EXPECTED type; with none
           available and a λ-shaped first case it errors with "cannot infer the motive of this
           `match`" (elaborate.rs:660-665). Emitting motives explicitly sidesteps this
           entirely — see T04-011.
@oracle    The emitted term contains no `Term::Hole` and no `?`-prefixed `Term::Var`; every
           motive appears as an explicit `Term::Lambda`; `infer_type` succeeds; and the emitted
           motive is term-for-term the translation of the motive in Lean's `Expr`, not one
           re-derived by LOGOS's unifier.
-/

namespace T05_014

/-- The classic Miller pattern: `?P a =?= (f a = f a)` when elaborating `▸`. -/
example {α : Type} (f : α → α) (a b : α) (h : a = b) : f a = f b := h ▸ rfl

/-- `Eq.subst` with the motive left to the unifier. -/
example (a b : Nat) (h : a = b) (p : a = a) : a = b := h ▸ p

/-- `congrArg` — the motive is `fun x => f x`, a pattern with one distinct bound variable. -/
example {α β : Type} (f : α → β) (a b : α) (h : a = b) : f a = f b := congrArg f h

/-- TWO distinct bound variables: `?P x y =?= …`, still inside the pattern fragment. -/
example {α : Type} (g : α → α → α) (a b : α) (h : a = b) : g a a = g b b := by
  rw [h]

/-- A REPEATED argument — `?M x x =?= t` is NOT a Miller pattern (`try_pattern` returns `None`
    on the repeat). Lean still solves it; LOGOS's unifier would not. -/
example {α : Type} (g : α → α → α) (a : α) : g a a = g a a := rfl

/-- A NON-VARIABLE argument — `?M (f a) =?= t` is outside the fragment for the same reason. -/
example {α : Type} (f : α → α) (a b : α) (h : f a = b) : b = f a := h.symm

/-- Motive inference for a dependent `match`, which is `infer_match_motive`'s Miller
    solution of `?P disc =?= T`. -/
def Carrier : Bool → Type
  | true => Nat
  | false => Bool

def mk (b : Bool) : Carrier b :=
  match b with
  | true => (0 : Nat)
  | false => false

/-- An η-sensitive constraint: `?M =?= fun x => t` versus `?M x =?= t`. -/
example {α β : Type} (f : α → β) : (fun x => f x) = f := rfl

/-- A constraint whose solution requires UNFOLDING a definition first — outside the pattern
    fragment and outside LOGOS's occurs-check fallback. -/
def Alias : Type := Nat

example (a : Alias) : a = a := rfl

/-- Higher-order unification under a binder, so the local context is non-empty — this is the
    only place `try_pattern` can succeed, since top-level `unify` runs with an empty `lctx`. -/
example {α : Type} (P : α → Prop) (h : ∀ x, P x) (a : α) : P a := h a

end T05_014
