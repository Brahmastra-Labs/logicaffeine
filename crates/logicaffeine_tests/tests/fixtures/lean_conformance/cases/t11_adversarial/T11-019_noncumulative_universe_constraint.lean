/-!
@case      T11-019
@tier      11 · Adversarial and soundness traps
@title     A universe constraint only Lean's NON-cumulative check catches — LOGOS's constructor
           universe check decides `≤` with the CUMULATIVE order
@lean      inductive storing a field one universe too high, Girard/Hurkens shape,
           `Type*`-style autobound levels, an inductive whose field is a `Sort u` for
           unconstrained `u`
@expect    reject
@target    (none — the Lean source is ill-typed, and LOGOS's own check does not reproduce
           Lean's judgement)
@evidence  kernel/src/type_checker.rs:540-583 (`check_constructor_universes` — the CIC universe
           constraint, and the decisive line is :570: `if !s.is_subtype_of(&target)`, where `s`
           is the field's inferred sort and `target` the inductive's result sort);
           kernel/src/term.rs:243-249 (`Universe::is_subtype_of` is `level_leq`, i.e. the
           CUMULATIVE order, not equality); kernel/src/term.rs:405-416 and :282-294
           (`level_leq` decides over the ℕ-model where `Prop = 0` and `Type n = n+1`, so
           `Prop ≤ Type 0 ≤ Type 1` all hold); kernel/src/type_checker.rs:555-557 (the
           impredicative-`Prop` exemption: an inductive whose result sort is `Prop` has NO
           constraint on its arguments' universes at all — `return Ok(())`);
           kernel/src/context.rs:539-561 (`add_constructor_checked` — the ONLY caller of the
           check; `add_constructor` at :419 performs none of it, see T11-013);
           kernel/src/term.rs:196-202 (`Universe::succ` sends `Prop ↦ Type(1)`, which is the
           off-by-one of T11-002 and which feeds the SORT side of this comparison through
           `infer_sort` at type_checker.rs:569); kernel/src/recursor.rs:64-71
           (`derive_recursor` pins a non-`Prop` inductive's motive to `Sort(Type 0)`, so even a
           correctly universe-checked inductive at `Type 1` gets an eliminator that cannot
           reach its own level)
@diag      E_UNIVERSE_CONSTRAINT_NOT_REPRODUCED — must name the inductive, the field, the sort
           Lean computed for it, and the sort Lean required, and state that LOGOS's own
           constructor check uses a cumulative `≤` and therefore does not reproduce the
           judgement. Must emit nothing.
@guard     **TIER 11 THESIS, AT THE DECLARATION LEVEL.** The kernel accepting a DECLARATION is
           not evidence that the declaration is the Lean one. Read `check_constructor_universes`
           closely: it is a real check, it exists, and it catches the Girard/Hurkens shape its
           doc comment names. But it decides `s ≤ target` with `is_subtype_of`
           (type_checker.rs:570), which is `level_leq` (term.rs:243-249) — the CUMULATIVE
           order. Lean's constraint on an inductive's universe is a constraint in a system with
           NO cumulativity: `Type 0` is not a subtype of `Type 1`, so Lean's acceptance set and
           LOGOS's are different sets, and the difference is not uniformly in one direction.
           Running `add_constructor_checked` is necessary and is NOT sufficient. The translator
           must additionally record Lean's own judgement and compare.
@guard     Catches the `Prop` exemption swallowing a real error. When the inductive's result
           sort is `Prop`, `check_constructor_universes` returns `Ok(())` immediately with no
           inspection of any field (type_checker.rs:555-557). Lean has the same impredicative
           exemption — but Lean ALSO restricts elimination out of such a `Prop`, and LOGOS's
           elimination gate is a different rule (`is_subsingleton_prop`, T11-020). So the two
           systems place the restriction in different places, and a translator that checks only
           the declaration has checked only half of it.
@guard     Catches the interaction with T11-002. The field's sort `s` comes from
           `infer_sort(&ext, param_type)` (type_checker.rs:569), which for a field whose type
           is `Prop` goes through `infer_type`'s `Sort` arm at :54 — `u.succ()` — and
           `Prop.succ()` is `Type(1)` (term.rs:198). Meanwhile the comparison `s ≤ target` is
           decided in the ℕ-model where `Prop` is `0`. So a `Prop`-typed field is measured with
           one procedure and compared with another that disagrees by one about `Prop`. The
           check is sound in the direction that matters for paradoxes and is not a faithful
           model of Lean's judgement.
@guard     Catches skipping the check entirely. `add_constructor` (context.rs:413-431) performs
           no universe check at all, so the universe-inconsistent inductive Lean refused will
           register silently and the Girard/Hurkens derivation of `False` becomes available.
           That is T11-013's rejection; this case is the one that remains even AFTER the
           translator does the right thing and uses `add_constructor_checked`.
@guard     Catches assuming a correctly-declared inductive is usable. `derive_recursor`
           hard-codes the motive target to `Sort(Type 0)` for anything that is not a
           non-subsingleton `Prop` inductive (recursor.rs:64-71). So an inductive that passes
           the universe check AT `Type 1` gets an eliminator that can only eliminate into
           `Type 0` — the declaration is accepted and the type is not usable at its own level.
@oracle    Translator exits non-zero with E_UNIVERSE_CONSTRAINT_NOT_REPRODUCED for each
           commented form below, and emits nothing. The decisive harness assertion is
           differential: for every inductive in the corpus, the translator must record Lean's
           computed field sorts and the inductive's declared sort, and must assert Lean's
           NON-cumulative constraint itself — never delegating to
           `check_constructor_universes`, whose `≤` is the wrong relation for reproducing
           Lean's judgement.
-/

namespace T11_019

/-
  inductive TooBig : Type where
    | mk : Type → TooBig

  error: invalid universe level in constructor 'T11_019.TooBig.mk', parameter has type
    Type
  at universe level
    2
  it must be smaller than or equal to the inductive datatype universe level
    1

  This one BOTH systems refuse: `check_constructor_universes` computes `s = Type 1`,
  `target = Type 0`, and `level_leq(Type 1, Type 0)` is false (term.rs:405-416). The check
  works. The cases below are where the two systems part company.
-/

/-- The compiling neighbour: declare the inductive one level up. -/
inductive BigEnough : Type 1 where
  | mk : Type → BigEnough

/-- A `Type 0` inductive storing a `Prop`. LOGOS: `s = Prop`, `target = Type 0`, and
    `level_leq(Prop, Type 0)` is TRUE in the ℕ-model (`0 ≤ 1`, term.rs:282-294) — accepted.
    Lean also accepts, because its own inductive constraint is on LEVELS (`Sort 0 ≤ Sort 1`),
    not on the cumulative subtyping it lacks. So this is the CONTROL: the two agree here, and
    a translator that concludes from it that the checks are equivalent has not looked at the
    next case. -/
inductive HoldsAProp : Type where
  | mk : Prop → HoldsAProp

/-- The impredicative-`Prop` exemption. `check_constructor_universes` returns `Ok(())` with no
    inspection at all (type_checker.rs:555-557), and Lean accepts too — but Lean then
    RESTRICTS elimination out of this inductive, and LOGOS's restriction is a different rule
    (`is_subsingleton_prop`, T11-020). The declaration check agreeing proves nothing about
    the elimination check agreeing. -/
inductive PropHoldingAType : Prop where
  | mk : Type → PropHoldingAType

/-- The eliminating direction, which is where the difference becomes observable. Lean refuses
    to eliminate `PropHoldingAType` into `Type`; that refusal is the real constraint, and it
    lives in a different place from the declaration check in both systems. -/
theorem tryEliminate (p : PropHoldingAType) : True :=
  PropHoldingAType.rec (motive := fun _ => True) (fun _ => trivial) p

/-
  noncomputable def tryEliminateIntoType (p : PropHoldingAType) : Type :=
    PropHoldingAType.rec (motive := fun _ => Type) (fun t => t) p

  error: type mismatch
    Type
  has type
    Type 1 : Type 2
  but is expected to have type
    Prop : Type

  — i.e. Lean will not even let the motive's CODOMAIN be `Type`: `PropHoldingAType.rec`'s
  motive is typed `PropHoldingAType → Prop`, because large elimination out of this
  proposition is forbidden.

  LOGOS's counterpart gate is `is_subsingleton_prop` (type_checker.rs:1688-1716), which
  computes the constructor's "data" arguments by skipping the INDUCTIVE's arity off the
  CONSTRUCTOR's telescope — two different telescopes. See T11-020.
-/

/-- Universe-polymorphic inductive: the constraint Lean checks is a constraint on the LEVEL
    VARIABLES, and LOGOS has nowhere to bind them on an inductive at all (T01-007). So the
    universe check cannot even be posed here. -/
inductive Wrapper (α : Type u) : Type u where
  | mk : α → Wrapper α

/-- Two instantiations at different levels, which is what makes monomorphising the inductive
    at `Type 0` unsound rather than merely lossy. -/
def wrapNat : Wrapper Nat := Wrapper.mk 0
def wrapType : Wrapper Type := Wrapper.mk Nat

/-- A `max`-valued inductive: the constraint is `max u v`, and `Universe::Type` takes a
    concrete `u32` (term.rs:170-176), so the level is not even expressible. -/
inductive Sum' (α : Type u) (β : Type v) : Type (max u v) where
  | inl : α → Sum' α β
  | inr : β → Sum' α β

/-- An inductive at `Type 1` that passes the universe check and whose derived recursor cannot
    reach its own level, because `derive_recursor` pins the motive to `Sort(Type 0)`
    (recursor.rs:64-71). Declaration accepted; type unusable. -/
inductive AtLevelOne : Type 1 where
  | mk : Type → AtLevelOne

def eliminateIntoTypeOne (x : AtLevelOne) : Type :=
  AtLevelOne.rec (motive := fun _ => Type) (fun t => t) x

end T11_019
