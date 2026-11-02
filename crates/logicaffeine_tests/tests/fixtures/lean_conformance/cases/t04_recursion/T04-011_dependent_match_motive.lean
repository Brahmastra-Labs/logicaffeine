/-!
@case      T04-011
@tier      4 · Recursion and pattern matching
@title     Dependent `match` with an explicit motive — LOGOS motive inference is Miller-only
@lean      match ... with (return clause / `(motive := ..)`), dependent elimination
@expect    degrade
@target    Term::Match { discriminant, motive: <an EXPLICIT `Term::Lambda` motive, never
           `Term::Hole`>, cases } — the translator must compute the motive itself from Lean's
           `Expr.app (matcherApp motive) ..` and emit it, not rely on LOGOS to infer it.
@evidence  kernel/src/term.rs:487-492 (`motive` is a mandatory `Box<Term>` field of every
           `Match` — there is no "no return clause" form);
           kernel/src/type_checker.rs:204-250 (the motive must be either a `Π` whose domain is
           the discriminant's type, or a bare `Sort`, which is silently wrapped as the CONSTANT
           motive `λ_:I. T`);
           kernel/src/elaborate.rs:578-636 (`fill_match_motives` fills a `Term::Hole` motive
           only, and only by calling `infer_match_motive`);
           kernel/src/elaborate.rs:638-700 (`infer_match_motive` solves `?P disc =?= T` by
           ABSTRACTING the discriminant out of the EXPECTED type — the Miller-pattern
           solution — and errors with "cannot infer the motive of this `match`" when there is
           no expected type and the first case is a λ);
           kernel/src/elaborate.rs:294-340 (`try_pattern`: a Miller pattern requires the
           metavariable's arguments to be DISTINCT bound variables; a repeated or non-variable
           argument makes it `None`)
@loss      MOTIVE_PROVENANCE. Lean's elaborated matcher carries the user's `return` clause
           (or the motive Lean synthesised) as a first-class argument, together with the
           information that it was user-written. In LOGOS the motive is just a term in the
           `Match` node; whether it came from the source or was reconstructed is not
           recorded, so a later re-elaboration may silently substitute a *different* motive
           that happens to typecheck (a constant one, say) and quietly weaken the
           definition's type.
@loss      CONSTANT_MOTIVE_COLLAPSE. `type_checker.rs:236-245` accepts a bare `Sort`-typed
           motive by wrapping it as `λ_:I. T`. A dependent Lean match whose motive *happens*
           to be constant on the emitted cases is therefore indistinguishable from a
           non-dependent one after translation. Acceptable (the terms are definitionally the
           same) but must be recorded, because the round-trip does not preserve dependency.
@guard     Catches emitting `motive: Term::Hole` and hoping the LOGOS elaborator fills it.
           Two failure modes. (a) `fill_match_motives` is part of `elaborate`, NOT of the
           kernel: a `Hole` that reaches `infer_type` is `CannotInferHole`
           (type_checker.rs:313). (b) Worse, a `Hole` that survives into an emitted TYPE is
           matched against **anything** by `types_equal` (type_checker.rs:1594-1597), and that
           leniency leaks into `is_subtype`'s fast path (:1314) — so a translated theorem can
           typecheck against a type it does not have.
@guard     Catches relying on inference where the Miller condition fails. `infer_match_motive`
           abstracts the discriminant out of the expected type; when the discriminant is NOT a
           bare variable (e.g. `match f n with`), `try_pattern` (elaborate.rs:294-340) rejects
           the non-variable argument and the solution falls back to replacing occurrences
           syntactically. A discriminant that occurs in the expected type only up to
           definitional unfolding is then missed, and the inferred motive is constant where the
           source's was dependent — silently generalising the statement.
@guard     Catches computing the motive at the wrong sort. `derive_recursor` pins motive
           targets (recursor.rs:64-71) but `Match` does not — yet the large-elimination gate
           (type_checker.rs:275-288) *does*: a `Prop` discriminant may only eliminate into a
           larger sort if the inductive is a subsingleton per `is_subsingleton_prop`
           (type_checker.rs:1688-1716). A dependent match on a `Prop` whose motive lands in
           `Type` is accepted by Lean for `Acc`/`Eq`/`False` and must be checked against
           LOGOS's own, differently-implemented, subsingleton rule.
@guard     Catches emitting an indexed family's motive with the wrong arity. For an indexed
           inductive the motive must be `Π(indices…). Π(z : I params indices). Sort`
           (type_checker.rs:602-611); a motive that abstracts only the scrutinee gets the
           indices applied to it anyway (`:648-652`) and fails — or, if the index happens to
           be a `Sort`, typechecks meaninglessly.
@oracle    The emitted `Match` has a non-`Hole` motive; the whole term contains no
           `Term::Hole`; it typechecks; and for each `Match` the inferred result type is the
           translation of Lean's. The loss record contains MOTIVE_PROVENANCE.
-/

namespace T04_011

/-- A type-valued family built by matching — the motive is genuinely dependent. -/
def Carrier : Bool → Type
  | true => Nat
  | false => Bool

/-- A dependent match: the result type varies per branch, so the motive is `λb. Carrier b`. -/
def mkCarrier : (b : Bool) → Carrier b
  | true => (0 : Nat)
  | false => false

/-- The motive written by hand as a `return` clause. -/
def mkCarrier' (b : Bool) : Carrier b :=
  match b with
  | true => (0 : Nat)
  | false => false

/-- An indexed family: the motive must abstract the INDEX as well as the scrutinee. -/
inductive Vect (α : Type) : Nat → Type where
  | nil  : Vect α 0
  | cons : α → {n : Nat} → Vect α n → Vect α (n + 1)

/-- The motive here is `λn. λ_ : Vect α n. Vect α n` — dependent in the index. -/
def vreverseStep {α : Type} : {n : Nat} → Vect α n → Vect α n
  | _, .nil => .nil
  | _, .cons a t => .cons a (vreverseStep t)

/-- Dependent match on `Nat` where the motive mentions the scrutinee in a *proposition*. -/
def zeroOrPos : (n : Nat) → (n = 0) ∨ (0 < n)
  | 0 => Or.inl rfl
  | _ + 1 => Or.inr (Nat.succ_pos _)

/-- Discriminant that is NOT a bare variable — the Miller-pattern condition fails, so LOGOS's
    motive *inference* cannot be relied on and the translator must emit the motive. -/
def notVar (f : Nat → Bool) (n : Nat) : Carrier (f n) :=
  match f n with
  | true => (0 : Nat)
  | false => false

/-- Large elimination from a `Prop`: allowed in Lean for the subsingleton `Eq`, and gated in
    LOGOS by `is_subsingleton_prop`. -/
def transportNat {a b : Nat} (h : a = b) (x : Nat) : Nat :=
  match b, h with
  | _, rfl => x

example : mkCarrier true = (0 : Nat) := rfl
example : mkCarrier' false = false := rfl

end T04_011
