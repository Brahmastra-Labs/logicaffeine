/-!
@case      T01-009
@tier      1 · Universes
@title     Cumulativity mismatch — LOGOS admits `Prop ≤ Type 0 ≤ Type 1`; Lean 4 admits NOTHING
@lean      ULift, PLift, explicit universe lifting, `Type 0` vs `Type 1` argument positions,
           Lean's non-cumulative `Sort` hierarchy
@expect    reject
@target    (none — no sound translation of a term whose Lean typing DEPENDS on the absence of
           cumulativity, and no licence to use LOGOS's cumulativity to resolve any level
           mismatch the translator could not otherwise resolve)
@diag      E_CUMULATIVITY_RELIANCE — must name the declaration, the two levels, and the exact
           position at which the translator's emitted term would be accepted only by
           `is_subtype_normalized`'s universe arm. Must state that Lean rejects the corresponding
           coercion. The translator must write NO declaration.
@evidence  kernel/src/type_checker.rs:1330-1336 (`is_subtype_normalized`:
           `(Term::Sort(u1), Term::Sort(u2)) => u1.is_subtype_of(u2)` — the cumulativity arm);
           kernel/src/term.rs:242-246 (`Universe::is_subtype_of` = `level_leq`, with the doc
           "Cumulative subtyping `self ≤ other`, decided over the level normal form. Sound for
           variables: `u ≤ u`, `Type 0 ≤ u`, `u ≤ succ u` hold");
           kernel/src/term.rs:266-276 (`to_lnat`: `Prop ↦ 0`, `Type n ↦ n+1` — so
           `level_leq(Prop, Type 0)` is `0 ≤ 1`, TRUE: a proposition is admitted wherever a
           `Type 0` is expected);
           kernel/src/type_checker.rs:1338-1355 (the Π arm: CONTRAVARIANT in the domain,
           COVARIANT in the codomain — so `Type 1 → Nat ≤ Type 0 → Nat`, a rule Lean has in no
           form whatsoever);
           kernel/src/type_checker.rs:1356-1358 (every OTHER position is INVARIANT and delegates
           to `def_eq_normalized` — cumulativity is SHALLOW);
           kernel/src/type_checker.rs:1309-1316 (`is_subtype` is what `check_type` calls at
           :452, so cumulativity is on the main path of every application, every declaration
           body check, and every `Match` case check);
           kernel/src/type_checker.rs:1473-1478 (structure eta and, at :1567,
           `proof_irrelevant` sit inside `def_eq_normalized`, i.e. below the cumulative layer);
           Lean 4 side: `Type 0` is not a subtype of `Type 1` in any judgement — `ULift` /
           `PLift` (`Init.Core`) exist precisely because the coercion does not
@guard     THIS IS THE CORPUS'S CENTRAL SOUNDNESS ASYMMETRY, stated as a rule: a translator may
           never use LOGOS's acceptance of a term as evidence that the term is a faithful
           translation. LOGOS's typing relation is STRICTLY LARGER than Lean's on exactly this
           axis. Every universe bug in Tier 1 — the `Type n` off-by-one (T01-002), the `Prop`
           sort off-by-one (T01-010), monomorphisation at the wrong level (T01-007, T01-008),
           defaulting an unresolved level (T01-012) — produces a term whose level is TOO HIGH,
           and `is_subtype_normalized`'s universe arm forgives every one of them upward. The
           kernel is therefore blind to the entire class. Conformance must be checked against
           Lean's judgement, re-derived, not against LOGOS's acceptance.
@guard     Catches ERASING `ULift` / `PLift`. `ULift.up` and `ULift.down` look like noise — they
           are the identity on the underlying value — and a translator that drops them produces
           `big Nat` where Lean wrote `big (ULift Nat)`. In Lean that is a hard error
           ("`Nat` has type `Type` but is expected to have type `Type 1`"). In LOGOS it
           typechecks, because `is_subtype_normalized` at :1333 answers
           `Type(0).is_subtype_of(Type(1))` = true. The emitted development then no longer needs
           the lift, which means it no longer needs `ULift` at all, which means every lemma about
           `ULift.up`/`ULift.down` in the source has no counterpart — and the two Lean types
           `ULift.{1} Nat` and `Nat`, which are genuinely distinct, have been identified.
@guard     Catches the Π-VARIANCE arm, which is the half a translator is least likely to expect.
           `is_subtype_normalized` at :1338-1352 makes `Type 1 → Nat ≤ Type 0 → Nat`
           (contravariance) and `Nat → Type 0 ≤ Nat → Type 1` (covariance). Lean has NEITHER.
           So a translator that hits a function-type mismatch — because it monomorphised a level,
           or because it got an off-by-one — will find that LOGOS silently accepts the function
           at a domain Lean never permitted, and the wrongly-typed function then propagates
           through every application.
@guard     Catches `Prop ≤ Type 0`. `level_leq(Prop, Type 0)` is `0 ≤ 1`, true (term.rs:268-271).
           So a PROOF is admitted wherever a `Type 0` value is expected. Lean refuses this
           outright — `PLift` exists for it. The consequence is worse than a level slip: once a
           `Prop` inhabitant sits in a `Type 0` position, `proof_irrelevant`
           (type_checker.rs:1547-1567) may still fire on it (it tests the inferred type's sort,
           not the position), so two DIFFERENT values in that position are definitionally equal.
           Computation over them is then not well-defined.
@guard     Catches the belief that "LOGOS is cumulative, so levels do not matter". Cumulativity
           is SHALLOW (type_checker.rs:1356-1358): it fires between two `Sort`s and covariantly
           under a `Pi` codomain, and NOWHERE else. Inside a type constructor's argument the
           comparison is INVARIANT and goes to `def_eq_normalized`, where two `Sort`s are
           compared by `equiv` (type_checker.rs:1496) — mutual `level_leq`, i.e. genuine
           equality. So `Box (Type 0)` and `Box (Type 1)` are NOT interchangeable even though
           `Type 0` and `Type 1` are. A translator that relies on cumulativity therefore
           produces a development that works at the top level and fails one constructor deep —
           the worst possible failure distribution, because the simple tests pass.
@guard     Catches "reject only when the levels differ". The mismatch is not always visible as a
           level: `is_subtype`'s FAST PATH is `types_equal` (type_checker.rs:1314), which returns
           true when either side is a `Term::Hole` (:1595-1597). A `Hole` left in a type (T00-016)
           therefore hides a cumulativity reliance behind a comparison that never examined the
           levels at all.
@oracle    The translator re-derives Lean's judgement for every subterm it emits and asserts that
           each emitted `check_type`/`is_subtype` obligation would ALSO be discharged by
           definitional equality alone — i.e. by `def_eq_normalized` with the cumulative arms of
           `is_subtype_normalized` disabled. Any obligation that needs `u1.is_subtype_of(u2)`
           with `u1 != u2`, or that needs the `Pi` variance arm, is reported as
           E_CUMULATIVITY_RELIANCE and the declaration is not written. Re-checking the whole
           emitted corpus with a non-cumulative `is_subtype` must produce zero new failures.
-/

namespace T01_009

/-- A function whose domain is `Type 1`. In Lean, `big Nat` is a HARD ERROR: `Nat : Type 0`, and
    `Type 0` is not a subtype of `Type 1`. -/
def big (α : Type 1) : Type 1 := α

/-- The lift Lean requires. `ULift.{1, 0} Nat : Type 1` is a genuinely different type from
    `Nat : Type 0` — it is a one-field structure, not a coercion. -/
def liftedNat : Type 1 := ULift.{1, 0} Nat
def appliedViaLift : Type 1 := big liftedNat

/-- The lift's introduction and elimination, which carry real computational content and which a
    translator that trusts cumulativity will delete. -/
def upNat : ULift.{1, 0} Nat := ULift.up 3
def downNat : Nat := upNat.down

example : downNat = 3 := rfl

/-- `Type 0` INHABITS `Type 1` — a statement Lean does accept, and which must not be confused
    with `Type 0` being a SUBTYPE of `Type 1`, which it does not. -/
def typeZeroInhabitsTypeOne : Type 1 := Type 0

/-- Π VARIANCE. LOGOS's `is_subtype_normalized` (type_checker.rs:1338-1352) makes
    `Type 1 → Nat ≤ Type 0 → Nat`. Lean has no such rule, so the adaptation must be written by
    hand, and it must go through the lift. -/
def consumesTypeOne (f : Type 1 → Nat) : Nat := f (ULift.{1, 0} Nat)
def someTypeOneFn : Type 1 → Nat := fun _ => 0
def variantApplication : Nat := consumesTypeOne someTypeOneFn

/-- The covariant direction: a `Nat → Type 0` where a `Nat → Type 1` is wanted. LOGOS accepts it
    by covariance under the Π codomain; Lean requires the lift at every result. -/
def wantsTypeOneFamily (F : Nat → Type 1) : Type 1 := F 0
def liftedFamily : Nat → Type 1 := fun _ => ULift.{1, 0} Nat
def covariantApplication : Type 1 := wantsTypeOneFamily liftedFamily

/-- `Prop ≤ Type 0` in LOGOS (`level_leq(Prop, Type 0)` = `0 ≤ 1`). Lean refuses it and provides
    `PLift` instead. -/
def wantsAType (α : Type) : Type := α
def liftedProp (p : Prop) : Type := PLift p
def propViaLift (p : Prop) : Type := wantsAType (liftedProp p)
def propUp (p : Prop) (h : p) : PLift p := PLift.up h

/-- The SHALLOWNESS of LOGOS's cumulativity, made concrete: inside a type constructor's argument
    the comparison is INVARIANT (type_checker.rs:1356-1358), so these two are not
    interchangeable in either system — the one place the two systems agree, and the place a
    cumulativity-reliant translator breaks. -/
structure Cell (α : Type 1) : Type 1 where
  val : α

def cellOfLifted : Cell liftedNat := ⟨ULift.up 3⟩

/-- A declaration whose Lean typing depends on the ABSENCE of cumulativity: `idAtZero` has type
    `Type → Type` and cannot be given where `Type 1 → Type 1` is expected, so the two
    applications below are genuinely different functions and must stay so. -/
def idAtZero (α : Type) : Type := α
def idAtOne (α : Type 1) : Type 1 := α

def usesZero : Type := idAtZero Nat
def usesOne : Type 1 := idAtOne (ULift.{1, 0} Nat)

end T01_009
