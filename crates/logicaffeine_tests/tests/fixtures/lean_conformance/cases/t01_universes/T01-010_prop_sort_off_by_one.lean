/-!
@case      T01-010
@tier      1 · Universes
@title     `Prop : Type 0` (Lean) versus `Prop : Type 1` (LOGOS) — an off-by-one INSIDE LOGOS
@lean      Prop as a TYPE rather than as a sort: `(Prop : Type)`, `Prop → Nat`, a `Prop`-typed
           structure field, a container of propositions
@expect    reject
@target    (none — any term in which `Prop` occupies a TERM position, i.e. one whose own sort is
           computed, has no faithful LOGOS translation)
@diag      E_PROP_SORT_OFFBY1 — must name the declaration, the sort Lean assigns (`Type n`) and
           the sort LOGOS computes (`Type (n+1)`), and state that the discrepancy originates in
           `Universe::succ`'s `Prop` arm. Must NOT emit the declaration at a bumped level and
           must NOT emit `Prop` as `Type 0` to sidestep it.
@evidence  kernel/src/term.rs:194-201 — `Universe::succ`:
               `Universe::SProp | Universe::Prop => Universe::Type(1),`
               `Universe::Type(n) => Universe::Type(n + 1),`
           kernel/src/type_checker.rs:54 — `Term::Sort(u) => Ok(Term::Sort(u.succ()))`.
           Together: `infer_type(Sort(Prop)) = Sort(Type(1))`, i.e. LOGOS says `Prop : Type 1`.
           Lean says `Prop = Sort 0` and `Sort 0 : Sort 1 = Type 0`, i.e. `Prop : Type 0`.
           The discrepancy is not merely a disagreement with Lean — it is INTERNALLY
           INCONSISTENT in LOGOS. kernel/src/term.rs:266-276 (`to_lnat`), the ℕ-model that
           `level_leq` and therefore `equiv` and `is_subtype_of` are decided over, maps
           `Prop ↦ Const(0)` and `Type n ↦ Const(n+1)`. In that model the successor of `Prop`
           (0) is 1, which is `Type 0`. But `succ` returns `Type(1)`, which `to_lnat` reads as 2.
           So LOGOS's successor operation and LOGOS's level-ordering decision procedure disagree
           about `Prop` by exactly one, and only about `Prop`.
           kernel/src/term.rs:205-216 and :219-237 (`max` / `imax` — both take their operands'
           levels as given, so the shift propagates through every Π formation at
           type_checker.rs:104-110);
           kernel/src/type_checker.rs:452 and :1309-1336 (`check_type` → `is_subtype` →
           `level_leq`, which uses the CORRECT `to_lnat` model — so a term whose sort was
           computed one too high is compared against a bound computed correctly, and fails);
           kernel/src/context.rs:295-301 (`add_structure` pins the record's sort to
           `Term::Sort(Universe::Type(0))`) and context.rs:545
           (`add_constructor_checked` performs a universe check — which a `Prop`-typed field will
           now fail, while context.rs:419's UNCHECKED `add_constructor` will not)
@guard     Catches the belief that this is harmless because `Prop` is usually a CODOMAIN. It
           usually is, and there `imax(a, Prop) = Prop` (term.rs:224) absorbs the operand
           entirely, so the shift never appears: every theorem statement, every `∀` into a
           proposition, every `And`/`Or`/`Ex`, is unaffected. A conformance suite made of
           THEOREMS will pass at 100% while the defect is live. The shift appears only where
           `Prop`'s OWN sort enters a `max` as the strict maximum — that is, where `Prop` sits in
           a DOMAIN position, a structure field, or a container argument. Those are data-level
           constructs, which a proof-oriented test suite barely exercises.
@guard     The crisp discriminator, worked out: `Prop → Nat`.
             Lean  — domain `Prop : Sort 1` (level 1); codomain `Nat : Sort 1` (level 1);
                     `imax(1,1) = 1`; result `Sort 1` = `Type 0`.
             LOGOS — domain sort `succ(Prop) = Type(1)`; codomain sort `Type(0)`;
                     `imax(Type 1, Type 0) = max(...) = Type 1`; result `Type 1`.
           One level apart. And `Prop → Type` is NOT a discriminator: there the codomain's own
           level (`Type 0 : Type 1`) already equals the shifted `Prop` level, so `max` returns
           the same answer in both systems. A translator that tests only the second case
           concludes there is no bug.
@guard     Catches "just bump every level that mentions `Prop` down by one". The compensation is
           not expressible: `Universe::Type` takes a concrete `u32` (term.rs:177) and there is no
           level-subtraction form, so the correction must be applied by the translator to every
           computed sort — and it cannot be applied to a level VARIABLE, because `Var(u)` ranges
           over the ℕ-model including 0 = `Prop` (term.rs:270-272). A translator that shifts
           concrete levels and leaves variables alone makes `level_leq`'s case-split on the
           `Prop`-ness of a pivot variable (term.rs:322-360) answer against a model that no
           longer matches the terms.
@guard     Catches "map `Prop` to `Universe::Type(0)`, then `Prop : Type 1` comes out right".
           It does come out right — `succ(Type(0)) = Type(1)`, matching Lean's `Type 0 : Type 1`
           for the thing that used to be `Prop`. And it destroys impredicativity
           (`imax(a, Type 0) = max(a, Type 0)`, term.rs:227), definitional proof irrelevance
           (`proof_irrelevant` matches `Sort(Universe::Prop)` syntactically,
           type_checker.rs:1553-1560), and the large-elimination gate (same syntactic match,
           type_checker.rs:277-288). Trading a level shift for the loss of the
           `Prop`-elimination restriction trades a precision bug for an UNSOUNDNESS. See T01-001.
@guard     Catches the interaction with `add_structure`. A Lean `structure` with a `Prop`-typed
           field is `Type 0` in Lean. `add_structure` hard-codes the record's sort to `Type 0`
           (context.rs:300) — so the record's declared sort is right — but the constructor's
           field is `Sort(Prop)`, whose own sort LOGOS computes as `Type 1`. The UNCHECKED
           `add_constructor` (context.rs:419) accepts it; `add_constructor_checked`
           (context.rs:545), which performs the universe check, does not. The same declaration is
           therefore accepted or refused depending on which registration path the translator
           took — and the unchecked path is the one that "works". This is T11-013's trap reached
           through the universe layer.
@guard     Catches the direction of failure being mistaken for safety. Here LOGOS computes a
           level too HIGH, and `is_subtype` only forgives going UP (term.rs:242-246), so the
           declaration below that states `Type` is REJECTED — a loud failure. A translator that
           responds by relaxing the stated level to `Type 1` has made the failure quiet and has
           changed the type of the declaration. Every consumer that stated `Type 0` then fails in
           turn, and the shift propagates through the whole development, one declaration per
           round of "fixes".
@oracle    For every declaration, the translator recomputes the sort Lean assigned and the sort
           `type_checker::infer_type` assigns to the emitted term, and asserts `Universe::equiv`
           between them. Any declaration where they differ — and they differ exactly when
           `Sort(Prop)`'s own sort is the strict maximum of a level computation — exits non-zero
           with E_PROP_SORT_OFFBY1 and writes nothing. The harness additionally asserts, directly
           against the kernel, that `infer_type(Sort(Prop)) == Sort(Type(1))` and that
           `is_subtype(Sort(Type(1)), Sort(Type(0)))` is FALSE — pinning both halves of the
           defect so that a future LOGOS release which fixes `succ` flips this case, rather than
           silently changing its meaning.
-/

namespace T01_010

/-- `Prop` in a TERM position. Lean: `Prop : Type 0`. LOGOS: `Sort(Prop) : Sort(Type 1)`
    (term.rs:198 + type_checker.rs:54), and `Type 1 ≰ Type 0`, so LOGOS REJECTS this. -/
def propIsAType : Type := Prop

/-- THE DISCRIMINATOR. Lean computes `Type 0`; LOGOS computes `Type 1`. -/
def propToNatType : Type := Prop → Nat

def propToNat : Prop → Nat := fun _ => 0

/-- Same shift, both operands being `Prop`. Lean: `Type 0`. LOGOS: `Type 1`. -/
def propToPropType : Type := Prop → Prop

def predicateOnProps : Prop → Prop := fun p => p

/-- The NON-discriminator, and the reason a partial test suite misses the bug: the codomain's own
    level already equals the shifted `Prop` level, so `max` returns the same answer in both
    systems. Lean and LOGOS agree here. -/
def propToTypeType : Type 1 := Prop → Type

/-- UNAFFECTED, and this is most of a proof development: the codomain is a proposition, so
    `imax(a, Prop) = Prop` (term.rs:224) absorbs the domain's level entirely and the shift
    never enters the computation. -/
def forallOverProps : Prop := ∀ (p : Prop), p → p

theorem usesForallOverProps : forallOverProps := fun _ h => h

/-- A `Prop`-typed structure FIELD. Lean: `Holder : Type 0`. In LOGOS the record's sort is pinned
    to `Type 0` by `add_structure` (context.rs:300) while the field's sort computes to `Type 1` —
    accepted by the unchecked `add_constructor` (context.rs:419), refused by
    `add_constructor_checked` (context.rs:545). -/
structure Holder where
  p : Prop

def holderIsAType : Type := Holder
def aHolder : Holder := ⟨True⟩

/-- A CONTAINER of propositions. The parameter position requires `Prop : Type 0`, which LOGOS
    denies — so this Lean-valid term has no LOGOS image at all, at any level. -/
inductive Box (α : Type) where
  | mk : α → Box α

def boxOfProp : Box Prop := Box.mk True

/-- The same shape through a prelude-shaped container, so the failure is not an artefact of a
    locally declared inductive. -/
def listOfProps : List Prop := [True, False]

/-- A dependent family INDEXED by propositions. Lean: `(p : Prop) → P p : Type 0`. -/
def familyOverProps (P : Prop → Type) : Type := (p : Prop) → P p

/-- A `Prop`-typed field alongside a `Type 0` field, so the record's sort in Lean is the max of
    `Type 0` (for `Nat`) and `Type 0` (for `Prop`) — still `Type 0`. Under the shift, the
    `Prop` field alone forces the record up. -/
structure Mixed where
  n : Nat
  q : Prop

def mixedIsAType : Type := Mixed

/-- `Prop` as an ARGUMENT to a universe-polymorphic definition, which is how the shift enters a
    declaration whose own level is a variable: instantiating `u := 0` in Lean means `Sort 0`,
    and LOGOS's `Universe::Var` ℕ-model (term.rs:270-272) agrees that 0 is `Prop` — so the
    `Var` path and the `succ` path disagree with each other inside one declaration. -/
def polyId.{u} {α : Sort u} (a : α) : α := a

theorem polyIdAtProp (h : True) : True := polyId h
def polyIdOnProp : Prop := polyId True

end T01_010
