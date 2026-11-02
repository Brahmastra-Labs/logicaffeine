/-!
@case      T11-009
@tier      11 · Adversarial and soundness traps
@title     Proof irrelevance is a LAST-RESORT rule that calls `infer_type` on both sides — so it
           fails exactly where it is most needed
@lean      proof irrelevance, two proofs of one Prop, `Subtype` elements differing only in the
           proof, `Subsingleton.elim`, a proof term inside a data structure
@expect    degrade
@target    The faithful term, PLUS a loss record naming the conditions under which LOGOS's
           `proof_irrelevant` will not fire.
@evidence  kernel/src/type_checker.rs:1545-1567 (`proof_irrelevant`: it calls
           `infer_type(ctx, a)`, RETURNS FALSE on any error, then calls `infer_type(ctx, &ta)`
           and requires the result to normalise to `Sort(Prop)` or `Sort(SProp)`, then calls
           `infer_type(ctx, b)` and requires `def_eq(ctx, &ta, &tb)` — three `infer_type` calls
           and one recursive `def_eq`, every one of which can fail);
           kernel/src/type_checker.rs:1465-1545 (`def_eq_normalized`: structural congruence,
           structure eta and the Peano bridge are ALL tried first; `proof_irrelevant` is the
           fallback reached only after they fail);
           kernel/src/type_checker.rs:311-313 (`infer_type` on a bare `Term::Hole` is
           `Err(KernelError::CannotInferHole)` — so a `Hole` anywhere in a proof term's TYPE
           makes `proof_irrelevant` return `false` for that pair);
           kernel/src/reduction.rs:52-69 (the `def_eq(ta, tb)` inside `proof_irrelevant`
           normalises, so it is subject to the 10000-step fuel and can answer `false` from
           exhaustion); kernel/src/term.rs:170-176 and type_checker.rs:1556-1562 (`SProp` is
           accepted here but nothing in LOGOS ever constructs or declares an `SProp` inductive,
           so this arm is dead — see T01-013)
@loss      IRRELEVANCE_IS_CONDITIONAL. Lean's kernel treats proof irrelevance as a primitive
           conversion rule: if `a : P` and `b : P` with `P : Prop`, then `a ≡ b`, decided
           cheaply and unconditionally. LOGOS's is a derived last-resort rule whose
           preconditions are three successful `infer_type` calls and one successful `def_eq` on
           the two proofs' TYPES. Whenever any of those fails — an unresolved placeholder, an
           expensive normalisation, a proof whose type mentions a declaration the translator
           emitted but whose own typing is delicate — the rule silently does not fire and two
           proofs of the same proposition compare UNEQUAL. The loss record must name the
           preconditions, not merely say "irrelevance is present".
@loss      IRRELEVANCE_IS_EXPENSIVE. Because it is the fallback, every FAILED definitional
           equality check in the whole development pays for three `infer_type` calls on the way
           to answering `false`. On a proof-heavy translation this is the dominant cost, and it
           compounds with T11-007's quadratic substitution and T11-005's fuel budget.
@guard     **TIER 11 THESIS.** The kernel accepting a term is not evidence of faithfulness —
           and symmetrically, the kernel REFUSING a definitional equality is not evidence that
           the equality is false. Here that asymmetry is sharp: `proof_irrelevant` answering
           `false` can mean "these are not both proofs of the same proposition" (a real
           failure) or "one of my three `infer_type` calls errored" or "the `def_eq` on the
           types ran out of fuel". A translator that reacts to the `false` by CHANGING THE
           TRANSLATION — inserting a cast, weakening a type, replacing a proof with a different
           one — is repairing a symptom whose cause is in the target's conversion checker.
@guard     Catches the interaction with `Term::Hole` (T11-004), which is the most common way to
           disable irrelevance without noticing. A `Hole` makes `types_equal` return `true`
           (type_checker.rs:1594-1597) so the term TYPECHECKS more easily — and simultaneously
           makes `infer_type` return `CannotInferHole` (:311-313) so `proof_irrelevant` returns
           `false` and two proofs of one proposition become UNEQUAL. One `Hole` therefore makes
           checking more permissive and conversion more restrictive at the same time, which is
           a combination no single test will reveal.
@guard     Catches assuming irrelevance rescues a `Subtype` encoding. `Subtype` is absent from
           LOGOS and must be emitted (T10-013); `add_structure` builds it with a proof-typed
           FIELD (context.rs:271-369). Two elements with the same value and different proofs
           are equal in Lean by irrelevance plus structure eta. In LOGOS that requires
           `try_structure_eta` (type_checker.rs:1391-1417) AND `proof_irrelevant`
           (:1547-1567) both to fire, in `def_eq_normalized` and nowhere else (:1473-1478) —
           so the equality holds under `def_eq` and NOT under `normalize`. A proof routed
           through `normalize` fails; the same proof routed through `def_eq` succeeds.
@guard     Catches the `Prop`-versus-`Type` precondition being read too loosely.
           `proof_irrelevant` requires `infer_type(ta)` to normalise to `Sort(Prop)`
           specifically (type_checker.rs:1556-1562). Because LOGOS is cumulative, a proposition
           can legitimately be CHECKED at `Type 0` (T11-001) — and a term the translator
           ascribed at `Type 0` for that reason will then have `infer_type(ta) = Sort(Type 0)`,
           irrelevance will not fire, and two proofs will compare unequal. The cumulativity
           leniency and the irrelevance rule pull in opposite directions on the same term.
@guard     Catches counting on `SProp`. The rule accepts `Sort(SProp)` as well
           (type_checker.rs:1559), but nothing in LOGOS constructs or declares an `SProp`
           inductive, so that arm is unreachable and cannot be used to make irrelevance
           unconditional. See T01-013.
@oracle    The emitted term is the faithful one, AND a loss record with categories
           IRRELEVANCE_IS_CONDITIONAL and IRRELEVANCE_IS_EXPENSIVE is produced for every
           declaration whose correctness depends on two distinct proof terms being
           definitionally equal. A harness asserts, for each such declaration, that LOGOS
           `def_eq` on the two emitted proofs returns `true` — and that the translator
           PREDICTED that outcome. A `def_eq` failure the translator did not predict is a
           corpus failure.
-/

namespace T11_009

/-- Two visibly different proofs of one proposition. Lean identifies them by irrelevance. -/
theorem proofA : 0 ≤ 1 := Nat.le_of_lt Nat.zero_lt_one
theorem proofB : 0 ≤ 1 := Nat.zero_le 1

example : proofA = proofB := rfl

/-- The general statement, which is what a translator must preserve. -/
example (p : Prop) (h1 h2 : p) : h1 = h2 := rfl

/-- Irrelevance inside a DATA structure: two `Subtype` elements with the same value and
    different proofs. In LOGOS this needs `try_structure_eta` AND `proof_irrelevant`, both of
    which live only in `def_eq_normalized` (type_checker.rs:1473-1478). -/
def pos1 : {n : Nat // 0 < n} := ⟨1, Nat.zero_lt_one⟩
def pos2 : {n : Nat // 0 < n} := ⟨1, Nat.one_pos⟩

example : pos1 = pos2 := rfl

/-- A proof carried inside an ordinary structure field, which is the realistic shape. -/
structure Bounded where
  val : Nat
  small : val < 10

def b1 : Bounded := ⟨3, by decide⟩
def b2 : Bounded := ⟨3, Nat.lt_of_lt_of_le (show 3 < 5 by decide) (show 5 ≤ 10 by decide)⟩

example : b1 = b2 := rfl

/-- A proof whose TYPE requires reduction to see as a `Prop` — so `proof_irrelevant`'s second
    `infer_type` must normalise, and that normalisation is on the 10000-step budget
    (reduction.rs:52-69). -/
def bigProp : Prop := (fun (_ : Nat) => 0 = 0) 5

theorem bp1 : bigProp := rfl
theorem bp2 : bigProp := Eq.refl 0

example : bp1 = bp2 := rfl

/-- A proof of an EQUALITY, where the type itself is an applied inductive whose arguments must
    be compared by `def_eq` before irrelevance can be reached. -/
theorem eq1 : (2 : Nat) + 2 = 4 := rfl
theorem eq2 : (2 : Nat) + 2 = 4 := by decide

example : eq1 = eq2 := rfl

/-- `Subsingleton` elimination, the packaged form of the same fact. -/
example (p : Prop) (h1 h2 : p) : h1 = h2 := Subsingleton.elim h1 h2

/-- The DATA counterpart, which is NOT irrelevant and must not be made so. A translator that
    "fixes" a failing irrelevance check by widening the rule breaks this. -/
example : (1 : Nat) ≠ 2 := by decide

/-- Irrelevance under a binder: the two proofs are functions, so `def_eq_normalized` must
    descend through the `Lambda` congruence (type_checker.rs:1495-1520) before reaching the
    fallback. -/
def f1 : ∀ n : Nat, 0 ≤ n := fun n => Nat.zero_le n
def f2 : ∀ n : Nat, 0 ≤ n := fun _ => Nat.zero_le _

example : f1 = f2 := rfl

end T11_009
