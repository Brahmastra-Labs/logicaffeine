/-!
@case      T01-001
@tier      1 · Universes
@title     `Prop` ≡ `Sort 0` — a DISTINCT `Universe` constructor, never `Type(0)`
@lean      Prop, Sort 0, impredicative ∀ into Prop, proof irrelevance, Prop-valued inductives
@expect    translate
@target    Term::Sort(Universe::Prop) — kernel/src/term.rs:445 + term.rs:175. NOT
           `Term::Sort(Universe::Type(0))`, which is a different enum constructor.
@evidence  kernel/src/term.rs:167-191 (`Universe::{SProp, Prop, Type(u32), Var, Succ, Max,
           IMax}` — `Prop` and `Type(0)` are separate variants);
           kernel/src/term.rs:222-228 (`imax`: `imax(a, Prop) = Prop` — the IMPREDICATIVITY rule
           — versus `imax(a, Type n) = max(a, Type n)`, the predicative one);
           kernel/src/type_checker.rs:104-110 (Pi formation uses `a_sort.imax(&b_sort)`, so the
           choice of constructor decides whether a `∀` is a proposition);
           kernel/src/type_checker.rs:277-288 (the large-elimination gate matches
           `Term::Sort(Universe::Prop)` SYNTACTICALLY, after `normalize`);
           kernel/src/type_checker.rs:1547-1567 (`proof_irrelevant` matches
           `Term::Sort(Universe::Prop) | Term::Sort(Universe::SProp)` — same syntactic test);
           kernel/src/recursor.rs:64-71 (`derive_recursor` branches on
           `Term::Sort(Universe::Prop)` to decide the motive target);
           kernel/src/term.rs:266-276 (`to_lnat`: `Prop ↦ Const(0)`, `Type n ↦ Const(n+1)` — the
           level-DECISION encoding, where `Prop` really is level 0, which is what makes the
           confusion plausible)
@guard     Catches `Prop ⟼ Universe::Type(0)` on the reasoning that "`Prop` is `Sort 0` and
           `Sort 0` is level 0". LOGOS's own `to_lnat` (term.rs:268-271) agrees that `Prop` is
           level 0 — and that is a DECISION-PROCEDURE encoding, not the surface representation.
           Four things break, none of them loudly:
           (1) IMPREDICATIVITY. `imax(a, Prop) = Prop` (term.rs:224) versus
               `imax(a, Type 0) = max(a, Type 0)` (term.rs:227). `∀ (α : Type), α → α` is a
               `Prop` in Lean and becomes `Type 1` under the mistranslation — a strictly larger
               sort, which LOGOS's cumulativity (type_checker.rs:1333) then accepts everywhere
               the smaller one was accepted.
           (2) PROOF IRRELEVANCE silently switches OFF. `proof_irrelevant`
               (type_checker.rs:1553-1560) fires only when both sides' types infer to
               `Sort(Universe::Prop)`. Under the mistranslation every proof becomes a `Type 0`
               inhabitant, so two proofs of the same statement are no longer definitionally
               equal, and every `rfl` that relied on irrelevance fails — reported as a
               conversion failure, never as a universe bug.
           (3) The LARGE-ELIMINATION GATE silently switches OFF. `type_checker.rs:277-288`
               guards elimination of a `Prop` inductive into a larger sort. With the discriminant
               typed `Sort(Type 0)` the guard never runs, so `Or`, `Exists` and any other
               multi-constructor proposition may be eliminated into `Type` — extracting
               computational content from a proof. This is an UNSOUNDNESS, not a precision loss.
           (4) `derive_recursor` (recursor.rs:64-71) picks the wrong motive target, so the
               generated eliminator has the wrong type.
@guard     Catches the converse, `Type 0 ⟼ Universe::Prop`, which is what a translator does if
           it decides to "normalise" `Sort n` uniformly. Because `Prop ≤ Type 0` holds by
           cumulativity (`level_leq`, term.rs:244-246) but not the reverse, this direction FAILS
           loudly in most positions — which makes it the safer bug and therefore the one a test
           suite finds, while direction (1) above goes undetected.
@guard     Catches comparing emitted universes with `Universe::equiv` (term.rs:249-252) in the
           conformance harness. `equiv` is defined as mutual `level_leq`, i.e. over `to_lnat`,
           where `Prop` and `Type 0` are levels 0 and 1 — so `equiv` DOES distinguish them, but
           an harness that instead compares `is_subtype_of` in one direction will find
           `Prop ≤ Type 0` true and conclude the mistranslation is harmless.
@guard     Catches assuming `Prop`'s own TYPE is the same in both systems. It is not:
           Lean says `Prop : Type 0`, LOGOS's `succ` (term.rs:196-201) says `Prop : Type 1`.
           That off-by-one is a separate defect with its own case — T01-010 — and must not be
           conflated with the `Prop`/`Type 0` constructor choice guarded here.
@oracle    Every Lean `Prop` / `Sort 0` emits `Universe::Prop` under STRUCTURAL equality of the
           `Universe` enum (not `equiv`, not `is_subtype_of`); every `Type 0` / `Sort 1` emits
           `Universe::Type(0)`; and for each `∀` into a proposition below, `infer_type` on the
           emitted `Pi` returns `Sort(Universe::Prop)` — the test that impredicativity survived.
-/

namespace T01_001

/-- `Prop` and `Sort 0` are the same Lean `Expr`. -/
def propSort : Type := Prop
def sortZero : Type := Sort 0

example : propSort = sortZero := rfl

/-- A proposition. Its sort is `Prop`, not `Type 0`. -/
def simpleProp : Prop := True

/-- IMPREDICATIVITY, the load-bearing case: a `∀` quantifying over ALL types whose codomain is a
    proposition is itself a proposition. Under `Prop ⟼ Type 0` this lands in `Type 1`. -/
def impredicativeForall : Prop := ∀ (α : Type), ∀ (_a : α), True

/-- The same shape one level up, to pin that the collapse is `imax`'s doing and not an
    accident of `Type 0`. -/
def impredicativeForallAtTypeOne : Prop := ∀ (α : Type 1), ∀ (_a : α), True

/-- The PREDICATIVE control: the codomain is `Type`, so `imax` does not collapse and the `∀`
    lives in a `Type` universe. If the translator maps `Prop` to `Type 0`, this and
    `impredicativeForall` become indistinguishable. -/
def predicativeForall : Type 1 := ∀ (_α : Type), Nat

/-- PROOF IRRELEVANCE: definitional in Lean for any `Prop`. LOGOS has it
    (type_checker.rs:1547-1567) but gated on the sort being SYNTACTICALLY `Universe::Prop`. -/
example (p : Prop) (h1 h2 : p) : h1 = h2 := rfl

/-- A `Prop`-valued inductive with TWO constructors. The large-elimination gate
    (type_checker.rs:277-288) must refuse to eliminate it into `Type`; that gate only runs if the
    discriminant's type infers to `Sort(Universe::Prop)`. -/
inductive Choice (p q : Prop) : Prop where
  | left  : p → Choice p q
  | right : q → Choice p q

/-- Small elimination — into `Prop` — which is always permitted. -/
theorem choiceComm (p q : Prop) (h : Choice p q) : Choice q p :=
  match h with
  | Choice.left hp => Choice.right hp
  | Choice.right hq => Choice.left hq

/-- A SUBSINGLETON `Prop` (one constructor, propositional arguments), which both Lean and LOGOS
    do permit to eliminate into `Type` — the positive control for the gate. -/
inductive Both (p q : Prop) : Prop where
  | mk : p → q → Both p q

/-- A `Prop`-indexed family, so the sort appears in an index position rather than a codomain. -/
def propFamily (p : Prop) : Prop := p → p

/-- `Prop` as a DOMAIN. This is the position where T01-010's off-by-one bites; here it is only
    a reminder that the sort of `Prop` itself is a separate question. -/
def quantifiesOverProps : Type := Prop → Nat

end T01_001
