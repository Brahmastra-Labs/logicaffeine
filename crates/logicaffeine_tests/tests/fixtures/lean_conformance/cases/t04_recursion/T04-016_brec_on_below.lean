/-!
@case      T04-016
@tier      4 · Recursion and pattern matching
@title     `brecOn` / `below` — Lean's course-of-values machinery has no LOGOS generator
@lean      Nat.brecOn, Nat.below, List.brecOn, binductionOn, the auto-generated
           course-of-values recursors Lean uses to compile structural recursion
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/recursor.rs:36-48 (`derive_recursor(ctx, ind)` is the ONLY generator in
           the kernel and it produces exactly one thing: `(recursor_type, recursor_term)` for
           `I_rec`. There is no `derive_brec_on`, no `derive_below`, no `derive_cases_on`);
           kernel/src/recursor.rs:60-72 (even that single recursor's motive target is
           hard-coded, so it is not a drop-in for Lean's universe-polymorphic `brecOn`);
           kernel/src/context.rs:253,374,419,545 (the only inductive-registration entry points
           — nothing generates auxiliary eliminators);
           kernel/src/term.rs:487-509 (`Match` is the primitive and `Fix` the only recursion
           form: course-of-values recursion has to be re-expressed, not translated);
           kernel/src/termination.rs:403-440 (`check_match_cases_guarded` marks only the
           DIRECT constructor fields of the guarding match as smaller — there is no notion of
           "every subterm reachable below", which is exactly what `below` packages)
@diag      E_NO_COURSE_OF_VALUES_RECURSOR — must name the Lean constant (`I.brecOn`,
           `I.below`, `I.binductionOn`) and state that LOGOS generates only `I_rec` via
           `derive_recursor`. Must NOT emit a `Global` referring to a `brecOn`/`below` name.
@guard     This case fires on almost every real Lean file, because `brecOn` is what the
           equation compiler EMITS: a translator reading `Expr`s sees `Nat.brecOn` wherever the
           source wrote `| 0 => .. | n+1 => ..`. The correct response is not to translate
           `brecOn` but to recognise the pattern and emit `Fix` + `Match` (T04-001). A
           translator that instead emits `Global("Nat_brecOn")` produces a term referring to a
           name `get_global` (context.rs:449) cannot resolve — and the failure surfaces at the
           first USE, not at the declaration, so the diagnostic blames the wrong file.
@guard     Catches "just declare `Nat_brecOn` as an axiom" to make the name resolve.
           `add_declaration` (context.rs:434) is unchecked and opaque, so the constant never
           reduces: every `rfl` in the source that relied on the recursion computing silently
           stops holding, and the translator has axiomatised the recursion scheme itself. For
           `below`, whose type is a *Sort*-valued family, the axiom additionally asserts a type
           former with no constructors, so nothing can ever inhabit it.
@guard     Catches "synthesise `below` as a nested product". `below` for `Nat` is
           `fun n => PProd (motive n) (Nat.below n)`-shaped — a recursively defined type family
           built from `PProd`/`PUnit`, and **`Prod`, `PProd`, `Sigma`, `PSigma` and `Unit`'s
           Lean counterparts are all absent from the LOGOS prelude** (LOGOS_FACTS, "Absent
           entirely"). Declaring them opens T03-022/T11-013 (unchecked `add_inductive`) and the
           universe questions of T01-007, none of which this case is licensed to answer.
@guard     Catches `casesOn` being lumped in. `I.casesOn` is *also* not generated, but it is a
           strictly easier problem: it is a `Match` with a constant motive and no recursion, so
           a translator may handle it — as a separately recorded path — while still rejecting
           `brecOn`. Rejecting both with the same diagnostic makes the conformance report
           uninformative.
@guard     Catches the "strong induction" form. `Nat.strongRecOn` / `Nat.strongInductionOn` are
           `brecOn`-derived in Lean and are the usual way a Mathlib-style proof recurses; they
           carry the same rejection, and a translator that special-cases `brecOn` but not these
           lets them through as unresolvable globals.
@oracle    Translator exits non-zero with E_NO_COURSE_OF_VALUES_RECURSOR for every definition
           below, emits no declaration for them, and emits no `Global` whose name contains
           `brecOn`, `below`, or `binductionOn` anywhere in its output for the whole corpus.
-/

namespace T04_016

/-- `brecOn` applied directly: the course-of-values recursor, with the "everything below n"
    bundle as the second argument's input. -/
noncomputable def sumBrec (n : Nat) : Nat :=
  Nat.brecOn (motive := fun _ => Nat) n
    (fun k below =>
      match k, below with
      | 0, _ => 0
      | m + 1, b => (m + 1) + b.1)

/-- `below` mentioned as a TYPE. Lean's `Nat.below` is a `Sort`-valued family built from
    `PProd`; there is no LOGOS generator and no `PProd`. -/
def belowType (n : Nat) : Type := @Nat.below (fun _ => Nat) n

/-- `List.brecOn` — the same machinery over a parameterised inductive, where `below` also
    carries the parameter. -/
noncomputable def lenBrec (xs : List Nat) : Nat :=
  List.brecOn (motive := fun _ => Nat) xs
    (fun ys below =>
      match ys, below with
      | [], _ => 0
      | _ :: _, b => b.1 + 1)

/-- Strong induction, the `brecOn`-derived form a Mathlib-style proof would use. -/
theorem strongLe (n : Nat) : n ≤ n :=
  Nat.strongRecOn (motive := fun k => k ≤ k) n (fun _ _ => Nat.le_refl _)

/-- The shape a translator actually meets: an ordinary structural definition whose COMPILED
    `Expr` is a `brecOn` application. The rejection must key off the compiled value, and the
    translator must instead emit `Fix` + `Match` (T04-001) rather than the `brecOn`. -/
def plainStructural : Nat → Nat
  | 0 => 0
  | n + 1 => plainStructural n + 1

end T04_016
