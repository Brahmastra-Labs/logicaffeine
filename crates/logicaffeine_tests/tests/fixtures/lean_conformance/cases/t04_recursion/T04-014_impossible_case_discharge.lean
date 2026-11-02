/-!
@case      T04-014
@tier      4 · Recursion and pattern matching
@title     Impossible-case discharge — LOGOS has no index unification, so the case must be
           synthesised anyway
@lean      index-refined match, omitted constructor branches, `Nat.noConfusion`-style
           discharge, `absurd` in an unreachable branch
@expect    degrade
@target    A `Term::Match` with the FULL case vector: the Lean branches in constructor
           registration order, plus a SYNTHESISED, well-typed body for every constructor Lean
           ruled out by index unification.
@evidence  kernel/src/type_checker.rs:612-678 (`infer_indexed_match`; :630-638 "Coverage:
           exactly one case per constructor, in registration order" — and there is no index
           unifier anywhere in the function);
           kernel/src/type_checker.rs:685-740 (`compute_indexed_case_type`: a case's expected
           type is the motive applied to THAT constructor's *declared* result indices, not to
           the discriminant's — so the "impossible" case is given a perfectly well-formed type
           and simply has to be inhabited);
           kernel/src/type_checker.rs:653-666 (the result type must infer to a `Sort`; this is
           the only soundness backstop, and it does not notice unreachable branches);
           kernel/src/context.rs:374 (`add_indexed_inductive(name, sort, num_params)` — the
           params/indices split is a declaration-time number, not a unifier)
@loss      UNREACHABILITY. Lean's elaborated matcher records that a constructor is impossible
           at the scrutinee's indices; LOGOS's `Match` records only a case term. After
           translation the emitted body is indistinguishable from a reachable one, so (a) any
           later consistency audit cannot tell a synthesised filler from real code, and (b) if
           the indices are ever weakened the filler silently becomes live. The loss record
           must name the constructor, the index equation Lean discharged, and the filler's
           provenance.
@loss      NOCONFUSION_ABSENCE. Lean discharges such branches with `noConfusion`/`injection`,
           which LOGOS does not generate at all (LOGOS_FACTS: "no `casesOn`/`brecOn`/
           `noConfusion`/`injection`"). The discharge *proof* is therefore dropped, not
           translated; only the filler term survives.
@guard     Catches synthesising the filler as `Term::Hole`. `types_equal` matches `Hole`
           against **anything** (type_checker.rs:1593-1597) and the leniency leaks into
           `is_subtype` (:1314), so the padded `Match` typechecks and a hole lands in the
           library. Never emit a `Hole`.
@guard     Catches synthesising the filler by recursion — the lazy fix of copying the *other*
           branch's body. In `vzipWith` below the other branch contains the recursive call, so
           the filler's argument is not a match-bound smaller variable of the guarding match
           and `verify_structural_arg_smaller` (termination.rs:378-395) rejects the whole
           definition. A correct Lean function becomes a build error.
@guard     Catches the case where the filler is NOT constructible. For a `Vect α (n+1) → α`
           the synthesised `nil` body must inhabit `α`, which is an arbitrary type. There is no
           sound filler, so the translator must fall back to rejection (T04-013) rather than
           inventing an axiom via `add_declaration` (context.rs:434) — an axiom of type
           `Π(α:Type 0). α` is a global inconsistency.
@guard     Catches getting the filler's TYPE wrong. `compute_indexed_case_type`
           (type_checker.rs:685-740) instantiates the constructor's leading `num_params`
           binders from the DISCRIMINANT's parameters and applies the motive to the
           CONSTRUCTOR's own result indices. A translator that applies the motive to the
           discriminant's indices instead builds the filler at the wrong type; for `vnilFill`
           that is `motive 0 nil` versus `motive (n+1) …`, and the two coincide only when
           `n` happens to be a literal — so the bug hides on small tests.
@guard     Catches setting `num_params` wrong on the inductive. `infer_indexed_match` is only
           reached when `disc_args.len() > num_params` (type_checker.rs:193-201); with the
           default "all arity is parameters" (`add_inductive`, context.rs:253) the indexed path
           is never taken, the plain `Match` rule requires the motive's domain to equal the
           discriminant's type, and the index is silently treated as a fixed parameter. See
           T03-005.
@oracle    The emitted `Match` has `cases.len() == ctor count` for every node; the emitted term
           contains no `Term::Hole` and introduces no new axiom; it typechecks; `normalize`
           agrees with Lean on every reachable input; and the loss record contains
           UNREACHABILITY with the discharged constructor named.
-/

namespace T04_014

inductive Vect (α : Type) : Nat → Type where
  | nil  : Vect α 0
  | cons : α → {n : Nat} → Vect α n → Vect α (n + 1)

/-- Lean omits `cons` because the index `0` rules it out. The filler IS constructible here
    (`Nat` is inhabited), so this degrades rather than rejecting. -/
def vnilFill {α : Type} : Vect α 0 → Nat
  | .nil => 0

/-- Lean omits `nil` because the index `n + 1` rules it out. The filler must inhabit `Nat`. -/
def vlast : {n : Nat} → Vect Nat (n + 1) → Nat
  | _, .cons a .nil => a
  | _, .cons _ (.cons b t) => vlast (.cons b t)

/-- Two scrutinees at the SAME index: Lean rules out the two mixed combinations by unifying
    `0` with `n + 1`. LOGOS needs all four cases in the nested encoding. -/
def vzipWith {α : Type} (f : α → α → α) : {n : Nat} → Vect α n → Vect α n → Vect α n
  | _, .nil, .nil => .nil
  | _, .cons a s, .cons b t => .cons (f a b) (vzipWith f s t)

/-- A `Prop`-indexed family: the impossible branch is discharged by a *proof* in Lean, and the
    filler in LOGOS must inhabit a `Prop` — which is only sound when that `Prop` is actually
    provable, never by axiom. -/
inductive Even : Nat → Prop where
  | zero : Even 0
  | step : {n : Nat} → Even n → Even (n + 2)

def evenPred : {n : Nat} → Even (n + 2) → Even n
  | _, .step h => h

/-- Index refinement over an equality, the shape Lean discharges with `noConfusion`. -/
def vheadOfSucc {α : Type} {n : Nat} : Vect α (n + 1) → Nat
  | .cons _ _ => n

example : vnilFill (α := Nat) .nil = 0 := rfl
example : vlast (Vect.cons 1 (Vect.cons 2 Vect.nil)) = 2 := rfl
example : vzipWith (· + ·) (Vect.cons 1 Vect.nil) (Vect.cons 2 Vect.nil)
        = Vect.cons 3 Vect.nil := rfl

end T04_014
