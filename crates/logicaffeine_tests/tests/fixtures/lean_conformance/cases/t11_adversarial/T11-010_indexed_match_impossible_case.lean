/-!
@case      T11-010
@tier      11 · Adversarial and soundness traps
@title     Indexed match — LOGOS has NO index unifier, so it demands a case for a constructor
           whose indices cannot match, and the case a translator invents is a lie
@lean      indexed inductive family (Vect, Fin, Eq), dependent match, impossible-case
           elimination, `nomatch`, `Nat.noConfusion`
@expect    reject
@target    (none — a `Match` whose impossible case carries an invented inhabitant is not a
           translation of a Lean match that had no such case)
@evidence  kernel/src/type_checker.rs:612-678 (`infer_indexed_match`, in full: it checks
           coverage with `if cases.len() != constructors.len() { return
           Err(WrongNumberOfCases) }` at :632-638 — EVERY constructor must be given a case,
           unconditionally — then checks each case against
           `compute_indexed_case_type(motive, ctor_name, ctor_type, num_params, disc_params)`
           at :644-647, which instantiates the motive at the CONSTRUCTOR's OWN indices. The
           discriminant's indices are used only to build the RESULT type at :650-655. There is
           no unification step between the two, and no arm that discharges a case because its
           indices are incompatible); kernel/src/type_checker.rs:254-260 (the non-indexed path
           has the same positional coverage requirement);
           kernel/src/context.rs:374 (`add_indexed_inductive(name, sort, num_params)` — the
           ONLY way to tell LOGOS that part of an arity is indices; `add_inductive`
           (context.rs:253) treats the WHOLE arity as parameters, which is the default and is
           silently wrong for a family, see T03-005);
           kernel/src/prelude.rs:3477 (the prelude itself registers the indexed `BVec` with
           `add_inductive`, so even the trusted base carries this misdeclaration — T10-010);
           LOGOS_FACTS: there is no `noConfusion`, no `injection`, no `casesOn`, no `brecOn`
@diag      E_NO_INDEX_UNIFICATION — must name the inductive, the constructor whose indices
           cannot match the discriminant's, and the Lean match arm that therefore has no
           counterpart, and must emit nothing. Must NOT offer to fill the case with
           `False.elim`, a default value, or a `Term::Hole`.
@guard     **TIER 11 THESIS.** The kernel's acceptance is not evidence of faithfulness. Here is
           the mechanism in full. Lean writes `match v with | Vect.cons x xs => …` on a
           `v : Vect α (n+1)` and needs no `nil` arm, because unification proves `0 ≠ n+1`.
           LOGOS demands `cases.len() == constructors.len()` (type_checker.rs:632-638), so the
           translator must supply a `nil` arm. There is no honest term to put there — the arm
           is unreachable, so its "value" is arbitrary. The three things a translator will
           reach for are each wrong in a different way:
             · A `Term::Hole`. Accepted by `types_equal` against anything
               (type_checker.rs:1594-1597), so the whole match typechecks and the emitted term
               contains a wildcard that will match any later type. See T11-004.
             · An arbitrary inhabitant of the result type. Well-typed, silent, and now the
               function has a DEFINED VALUE on an input Lean said could not occur — so a later
               proof that case-splits differently, or a `decide` that evaluates the branch, can
               observe it.
             · `False.elim` applied to a fabricated proof. That requires proving the case
               impossible, which is exactly what LOGOS gives no tool for: no `noConfusion`, no
               `injection`, no index unifier.
           In every variant LOGOS accepts the term. In none of them does it mean what the Lean
           source meant.
@guard     Catches the params/indices misdeclaration that makes this worse. If the translator
           uses `add_inductive` rather than `add_indexed_inductive` (context.rs:253 vs :374),
           LOGOS believes the family has zero indices, the match never reaches
           `infer_indexed_match` at all, and the index is not related to the scrutinee in ANY
           way — so cases that Lean rules impossible are not merely undischargeable, they are
           indistinguishable from possible ones. The prelude's own `BVec` is registered this
           way (prelude.rs:3477).
@guard     Catches the motive. `compute_indexed_case_type` instantiates the motive at the
           CONSTRUCTOR's indices (type_checker.rs:644-647) while the result type is the motive
           at the DISCRIMINANT's indices (:650-655). A translator that builds the motive by
           abstracting the discriminant's indices — the natural thing to do — produces a motive
           that is correct for the reachable arms and, for the unreachable arm, is being asked
           to accept a term at a type it was never meant to describe.
@guard     Catches the `Eq`-based workarounds. Lean's `Eq.rec`/`Eq.mpr` route around index
           unification by carrying propositional equalities. LOGOS has `Eq` and a derived
           `Eq_rec` (prelude.rs:1481-1596), so a translator CAN emit an equality-carrying
           version — but that is a different term with different computational behaviour, and
           it must be reported as such, not as a translation of the dependent match.
@oracle    Translator exits non-zero with E_NO_INDEX_UNIFICATION for every declaration below
           whose Lean match omits a constructor arm, and emits nothing. The decisive harness
           assertion is structural: for every `Term::Match` in the whole corpus output,
           `cases.len()` equals the constructor count AND every case is the translation of a
           Lean arm that actually existed — no case may be a `Hole`, a fabricated inhabitant,
           or a `False.elim` of a proof the Lean source did not contain.
-/

namespace T11_010

/-- A length-indexed vector: the canonical family. -/
inductive Vect (α : Type) : Nat → Type where
  | nil : Vect α 0
  | cons : {n : Nat} → α → Vect α n → Vect α (n + 1)

/-- Lean needs NO `nil` arm: unification proves `0 ≠ n + 1`. LOGOS demands one and there is
    no honest term to supply. -/
def head {α : Type} {n : Nat} (v : Vect α (n + 1)) : α :=
  match v with
  | Vect.cons x _ => x

/-- Same shape, returning the tail — so the invented `nil` arm would have to inhabit
    `Vect α n` for an arbitrary `n`, which is not even always possible. -/
def tail {α : Type} {n : Nat} (v : Vect α (n + 1)) : Vect α n :=
  match v with
  | Vect.cons _ xs => xs

example : head (Vect.cons 3 Vect.nil) = 3 := rfl

/-- The other direction: a match on `Vect α 0` that needs no `cons` arm. -/
def isEmpty {α : Type} (v : Vect α 0) : Bool :=
  match v with
  | Vect.nil => true

example : isEmpty (Vect.nil : Vect Nat 0) = true := rfl

/-- `Fin 0` is uninhabited, so a match on it needs NO arms at all. LOGOS requires
    `cases.len() == constructors.len()`, and `Fin`'s encoding has one constructor. -/
def fromEmptyFin {α : Type} (i : Fin 0) : α := i.elim0

/-- `nomatch` — Lean's explicit spelling of "this is impossible". There is no LOGOS
    counterpart. -/
def absurdVect {α : Type} (v : Vect α 0) : α → α :=
  match v with
  | Vect.nil => id

/-- An indexed family over `Bool`, so the impossible case is decided by a constructor rather
    than by arithmetic — the unifier's job is the same and LOGOS still has none. -/
inductive Tagged : Bool → Type where
  | yes : Tagged true
  | no : Tagged false

def onlyYes (t : Tagged true) : Nat :=
  match t with
  | Tagged.yes => 1

example : onlyYes Tagged.yes = 1 := rfl

/-- The shape where the invented case is OBSERVABLE. If a translator fills the impossible
    `Tagged.no` arm with `0`, then any later reasoning that unfolds `onlyYes` at a
    non-canonical `Tagged true` sees a defined value where Lean guarantees there is none. -/
def onlyNo (t : Tagged false) : Nat :=
  match t with
  | Tagged.no => 2

example : onlyNo Tagged.no = 2 := rfl

/-- `Eq` itself is an indexed family, and its single-arm match is the one LOGOS's prelude
    already handles via the derived `Eq_rec` (prelude.rs:1481-1596). It is included to mark
    the boundary: this shape translates, the ones above do not. -/
def transport {α : Type} {a b : α} (h : a = b) (P : α → Type) (x : P a) : P b :=
  match h with
  | rfl => x

example : transport (rfl : (0 : Nat) = 0) (fun _ => Nat) 5 = 5 := rfl

end T11_010
