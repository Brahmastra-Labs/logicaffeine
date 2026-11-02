/-!
@case      T01-013
@tier      1 · Universes
@title     `SProp` — vestigial in LOGOS; emitting it is INVENTING a universe Lean does not have
@lean      (no Lean 4 counterpart) — the Lean constructs a translator is tempted to map onto it:
           definitionally proof-irrelevant `Prop`s, `Subsingleton`, squash types, `PUnit`-like
           propositions, `Eq` proofs
@expect    reject
@target    (none — `Universe::SProp` must never appear in translator output)
@diag      E_NO_SPROP — must name the Lean declaration and the LOGOS universe the translator was
           about to emit, and state that Lean 4 has no `SProp` so no Lean construct maps to it.
           No declaration may be written at `Sort(SProp)`.
@evidence  kernel/src/term.rs:170-174 (`Universe::SProp` exists, documented as "the
           DEFINITIONALLY-proof-irrelevant sort (S) … the bottom of the hierarchy
           (`SProp ≤ Prop ≤ Type n`) … impredicative");
           and then, in the whole workspace, `SProp` is READ in exactly four places and
           CONSTRUCTED in none of them:
             kernel/src/term.rs:208, :222 (absorbed by `max`; `imax(a, SProp) = SProp`),
             kernel/src/term.rs:196-198 (`succ(SProp) = Type(1)` — the same off-by-one as `Prop`,
               T01-010),
             kernel/src/type_checker.rs:1553-1560 (`proof_irrelevant` accepts
               `Sort(Universe::Prop) | Sort(Universe::SProp)`),
             kernel/src/recheck.rs:757 (the de Bruijn re-checker's copy of the same test),
             kernel/src/reduction.rs:4539 (`Universe::SProp => Term::Global("USProp")` — a
               reflection token);
           kernel/src/context.rs:253, :271, :374, :419, :545 — NO registration path takes an
           `SProp` sort in any distinguished way, and nothing in kernel/src/prelude.rs declares an
           `SProp` inductive, so the sort is uninhabitable by any declared type;
           kernel/src/recursor.rs:64-71 — `derive_recursor`'s motive target is
           `Sort(Prop)` for a non-subsingleton `Prop` inductive and `Sort(Type 0)` for EVERYTHING
           ELSE; there is no `SProp` arm, so an `SProp`-sorted inductive would get a `Type 0`
           eliminator;
           kernel/src/type_checker.rs:277-288 — the large-elimination gate tests
           `Term::Sort(Universe::Prop)` ONLY; an `SProp`-sorted discriminant does not match and
           the restriction is NOT APPLIED;
           kernel/tests/sprop.rs:44-56 — the only exercise of the sort is a unit test asserting
           the level algebra (`SProp : Type 1`, `SProp ≤ Prop ≤ Type 0`, and not the converses)
@guard     THE SOUNDNESS HOLE, stated plainly. The large-elimination gate at
           type_checker.rs:277-288 fires only when the discriminant's type infers to
           `Term::Sort(Universe::Prop)`. An inductive registered at `Sort(Universe::SProp)` —
           which `add_inductive` (context.rs:253) will accept, being UNCHECKED — does not match
           that pattern, so a multi-constructor proposition placed in `SProp` may be eliminated
           into `Type` with no restriction at all. Meanwhile `proof_irrelevant`
           (type_checker.rs:1553-1560) DOES match `SProp`, so any two of its inhabitants are
           definitionally equal. Together: two definitionally equal proofs can be eliminated into
           two distinct `Type`-level values. That is an inconsistency, reachable by a translator
           that thought it was being precise.
@guard     Catches the premise itself. Lean 4 has no `SProp`. There is therefore no Lean
           construct that SHOULD map to `Universe::SProp`, and any emission of it is the
           translator inventing a universe. The temptation is real because several Lean idioms
           LOOK like `SProp`: `Prop` already has definitional proof irrelevance in Lean, so a
           translator reading LOGOS's `SProp` documentation ("definitionally proof-irrelevant")
           may reasonably conclude that Lean's `Prop` maps there. It does not: Lean's `Prop`
           maps to `Universe::Prop` (T01-001), which LOGOS also gives definitional proof
           irrelevance (type_checker.rs:1547-1567) — the feature is already available at the
           correct sort.
@guard     Catches mapping `Subsingleton`, a squash type, or a single-constructor `Prop` to
           `SProp` on the grounds that they are "more irrelevant" than an ordinary proposition.
           They are ordinary `Prop`s in Lean and must stay `Universe::Prop`. LOGOS's notion of a
           subsingleton is a PROPERTY computed from the constructor list
           (`is_subsingleton_prop`, type_checker.rs:1688-1716), not a sort — see T01-014.
@guard     Catches emitting `SProp` "because it is the bottom of the hierarchy and therefore
           safe". It is the bottom for SUBTYPING (`level_leq`; kernel/tests/sprop.rs:51-56), and
           that is exactly what makes it unsafe here: `SProp ≤ Prop ≤ Type 0` means an
           `SProp`-sorted thing is admitted in every position, so the mistranslation is absorbed
           silently everywhere by cumulativity (T01-009) instead of failing at the declaration.
@guard     Catches `imax(a, SProp) = SProp` (term.rs:222-224). Once ONE thing is at `SProp`,
           every Π into it collapses to `SProp` too, so the sort propagates through the
           development along exactly the paths that `Prop` would have — but into the branch of
           the kernel that has no elimination gate and no recursor arm.
@guard     Catches relying on `succ(SProp)`. term.rs:196-198 gives `Type(1)`, inheriting T01-010's
           off-by-one wholesale — so even the "harmless" act of mentioning `Sort SProp` as a type
           lands at the wrong level.
@oracle    A structural walk of every emitted `Term` (declaration types, bodies, constructor
           types, structure field types, recursor types) finds zero `Universe::SProp` nodes, and
           no `Context::add_inductive`/`add_indexed_inductive`/`add_structure` call in the
           emitted context carries an `SProp` sort. Any Lean declaration the translator
           classified as `SProp`-eligible exits non-zero with E_NO_SPROP.
-/

namespace T01_013

/-- Lean's `Prop` ALREADY has definitional proof irrelevance. This is the fact that makes a
    translator reach for LOGOS's "definitionally-proof-irrelevant sort" — and the reason it must
    not: `Universe::Prop` in LOGOS has the same property
    (`proof_irrelevant`, type_checker.rs:1547-1567). -/
example (p : Prop) (h1 h2 : p) : h1 = h2 := rfl

/-- A single-constructor proposition whose only argument is a proof — a genuine subsingleton.
    Still a `Prop` in Lean; still `Universe::Prop` in LOGOS. `is_subsingleton_prop`
    (type_checker.rs:1688-1716) computes the subsingleton property from the CONSTRUCTORS, so no
    sort change is needed or available. -/
inductive Wrapped (p : Prop) : Prop where
  | mk : p → Wrapped p

theorem wrappedIrrelevant (p : Prop) (a b : Wrapped p) : a = b := rfl

/-- A SQUASH: a `Prop` that erases the computational content of a `Type`. This is the classic
    `SProp` motivation, and in Lean it is an ordinary `Prop` inductive with a NON-propositional
    constructor argument — so it is NOT a subsingleton by LOGOS's rule
    (type_checker.rs:1704-1707 rejects the `α`-typed argument) and must not be large-eliminated.
    A translator that "fixes" that by moving it to `SProp` bypasses the gate entirely. -/
inductive MySquash (α : Type) : Prop where
  | mk : α → MySquash α

def squashNat : MySquash Nat := MySquash.mk 3
def squashNatAgain : MySquash Nat := MySquash.mk 4

/-- The two squashes above are definitionally equal by proof irrelevance, while the values inside
    them are not. This is precisely why elimination out of `MySquash` into `Type` must be
    refused — and precisely what the `SProp` sort would silently permit. -/
theorem squashesEqual : squashNat = squashNatAgain := rfl

/-- `Subsingleton`, the Lean typeclass. A `Prop`-valued class, not a sort. -/
class MySubsingleton (α : Sort u) : Prop where
  allEq : ∀ (a b : α), a = b

instance : MySubsingleton True := ⟨fun a b => by cases a; cases b; rfl⟩

/-- A `Prop`-valued type with exactly one inhabitant — `PUnit`-like, but propositional. Lean
    already gives it definitional irrelevance; there is nothing `SProp` would add. -/
inductive Triv : Prop where
  | mk

theorem trivIrrelevant (a b : Triv) : a = b := rfl

/-- `Eq` proofs: the canonical "definitionally irrelevant" objects. `Eq` is a `Prop` inductive in
    both systems (LOGOS registers it in its prelude), and it is a subsingleton by
    `is_subsingleton_prop`, so it already large-eliminates. Nothing here wants a new sort. -/
theorem eqProofsIrrelevant (n : Nat) (h1 h2 : n = n) : h1 = h2 := rfl

/-- A MULTI-constructor proposition — the shape whose large elimination the gate at
    type_checker.rs:277-288 must refuse. Moving it to `SProp` would make the gate not fire. -/
inductive TwoWays (p q : Prop) : Prop where
  | first  : p → TwoWays p q
  | second : q → TwoWays p q

theorem twoWaysSmallElim (p q : Prop) (h : TwoWays p q) : TwoWays q p :=
  match h with
  | TwoWays.first hp => TwoWays.second hp
  | TwoWays.second hq => TwoWays.first hq

end T01_013
