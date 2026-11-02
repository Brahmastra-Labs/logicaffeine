/-!
@case      T11-001
@tier      11 · Adversarial and soundness traps
@title     LOGOS is CUMULATIVE and Lean 4 is not — the target kernel accepts terms the source
           language rejects
@lean      Type 0 / Type 1 ascription, Prop used where Type 0 is expected, Pi variance,
           universe-level mismatch
@expect    reject
@target    (none — the Lean source these terms come from is itself ill-typed, so there is
           nothing faithful to emit)
@evidence  kernel/src/type_checker.rs:1330-1360 (`is_subtype_normalized` — its own doc comment:
           "Cumulativity lives ONLY here: universes follow `Prop ≤ Type i ≤ Type j`, and a `Π`
           is contravariant in its domain and covariant in its codomain");
           kernel/src/type_checker.rs:1299-1323 (`is_subtype`, whose doc comment lists the
           admitted judgements verbatim: `Type 0 <= Type 1`, `Nat -> Type 0 <= Nat -> Type 1`,
           `Type 1 -> Nat <= Type 0 -> Nat`); kernel/src/type_checker.rs:505-513 (`check_type`
           ends in `is_subtype(ctx, &inferred, expected)` — so EVERY checking position,
           including a function ARGUMENT via `infer_type`'s `Term::App` arm at :163, is
           cumulative); kernel/src/term.rs:405-416 and :282-294 (`level_leq` decides the order
           over the ℕ-model where `Prop = 0` and `Type n = n+1`, so `Prop ≤ Type 0` holds);
           kernel/src/type_checker.rs:1333
           (`(Sort(u1), Sort(u2)) => u1.is_subtype_of(u2)` — the single line that admits all of
           it); Lean 4 by contrast has NO cumulativity: `Type 0` and `Type 1` are unrelated and
           `Prop` is not `Type 0` (the error texts are reproduced below)
@diag      E_TARGET_MORE_PERMISSIVE(cumulativity) — must state that the LOGOS kernel accepting
           the emitted term proves nothing about the Lean source, name the specific universe
           judgement that Lean refused, and produce no output. The diagnostic must be raised by
           the TRANSLATOR's own check, because the target kernel will not raise it.
@guard     ### TIER 11 THESIS — THIS CASE IS ITS CLEANEST INSTANCE.
           **Because LOGOS is cumulative where Lean is not, because `add_inductive` /
           `add_constructor` are unchecked, and because `types_equal` treats `Term::Hole` as
           matching anything, LOGOS's kernel accepting a term is NOT evidence that the
           translation is faithful.** A translator must carry its OWN faithfulness oracle; it
           cannot outsource that to the target kernel. Every other case in this tier is a
           different route to the same conclusion.
           Here the mechanism is one line: `is_subtype_normalized`'s `Sort`/`Sort` arm
           (type_checker.rs:1333). The tempting workflow — "translate, then run LOGOS
           `infer_type`, and if it passes we are done" — has a strictly larger acceptance set
           than Lean's elaborator. So a translator bug that emits a term one universe off, or
           that confuses `Prop` with `Type 0`, is INVISIBLE to that workflow. It is not merely
           that the check is weak: the check is being asked to certify a property (this term
           is the Lean source's meaning) that it was never designed to decide.
@guard     Catches the specific silent repair: "the universes did not line up, so I inserted a
           lift / dropped the level annotation, and it typechecked". In LOGOS a dropped level
           annotation frequently still typechecks by cumulativity. In Lean it would not. The
           translator must compare against LEAN's judgement, recorded at translation time, not
           against LOGOS's acceptance.
@guard     Catches over-generalising the leniency. Cumulativity applies at the top level of a
           checking position, and INVARIANTLY inside a type application: `is_subtype_normalized`
           delegates every non-`Sort`, non-`Pi` shape to `def_eq_normalized`
           (type_checker.rs:1358-1360), so `F Prop` is NOT a subtype of `F (Type 0)`. A
           translator that assumes cumulativity is uniform will emit terms that are rejected
           for the opposite reason, and will then "fix" them with a `Term::Hole` — which
           `types_equal` accepts against anything (:1594-1597). Two bugs compose into a
           passing build. See T11-004.
@guard     Catches the `Prop ≤ Type 0` direction in particular, which is also the off-by-one of
           T11-002 seen from the other side: LOGOS says `Prop : Type 1` (term.rs:196-202) AND
           `Prop ≤ Type 0` (term.rs:282-294). Lean says `Prop : Type 0` and admits no
           subtyping at all. There is no consistent reading under which a `Prop`/`Type 0`
           confusion is harmless.
@oracle    Translator exits non-zero with E_TARGET_MORE_PERMISSIVE for each commented form
           below, emits nothing, and — decisively — a harness re-checks that for every case in
           the WHOLE corpus, the translator's accept/reject decision was made by its own
           universe check and not by calling LOGOS `infer_type`. A translator whose only
           universe oracle is the target kernel fails this tier by construction.
-/

namespace T11_001

/-! ### The forms Lean REFUSES. Each is given with Lean 4.14's actual error text, followed by
    the closest compiling neighbour. LOGOS's `is_subtype` admits every one of them. -/

/-
  def liftNat : Type 1 := Nat

  error: type mismatch
    Nat
  has type
    Type : Type 1
  but is expected to have type
    Type 1 : Type 2

  In LOGOS: `infer_type(Global("Nat"))` is `Sort(Type 0)`; `check_type` ends in
  `is_subtype(Sort(Type 0), Sort(Type 1))` (type_checker.rs:505-513), which is
  `Type 0 ≤ Type 1` — ACCEPTED (type_checker.rs:1333).
-/

/-- The compiling neighbour: the level must match exactly in Lean. -/
def natAtItsOwnLevel : Type 0 := Nat

/-
  def propAsType : Type 0 := True

  error: type mismatch
    True
  has type
    Prop : Type
  but is expected to have type
    Type : Type 1

  In LOGOS: `infer_type(Global("True"))` is `Sort(Prop)`; `is_subtype(Sort(Prop),
  Sort(Type 0))` is `Prop ≤ Type 0` under the ℕ-model (`Prop = 0`, `Type 0 = 1`,
  term.rs:282-294) — ACCEPTED.
-/

/-- The compiling neighbour. -/
def propAtProp : Prop := True

/-
  def idT (α : Type 0) : Type 0 := α
  def applyToProp := idT True

  error: application type mismatch
    idT True
  argument
    True
  has type
    Prop : Type
  but is expected to have type
    Type : Type 1

  In LOGOS the ARGUMENT position is cumulative too: `infer_type`'s `Term::App` arm calls
  `check_type(ctx, arg, &param_type)` (type_checker.rs:163), and `check_type` ends in
  `is_subtype` (:505-513) — ACCEPTED.
-/

def idT (α : Type 0) : Type 0 := α

/-- The compiling neighbour: apply it to something that really is in `Type 0`. -/
def applyToType : Type 0 := idT Nat

/-
  structure Holder where
    fld : Type 0
  def holdProp : Holder := ⟨True⟩

  error: application type mismatch
    { fld := True }
  argument
    True
  has type
    Prop : Type
  but is expected to have type
    Type : Type 1

  Same mechanism, now inside a constructor application — the shape a translator is most
  likely to emit while carrying a `Prop` where a `Type 0` field was declared.
-/

structure Holder where
  fld : Type 0

/-- The compiling neighbour. -/
def holdType : Holder := ⟨Nat⟩

/-! ### Pi variance. Lean refuses both directions; LOGOS's `is_subtype_normalized` admits both
    (type_checker.rs:1337-1356, and the doc comment at :1305-1308 lists them by name). -/

/-
  def covariant (f : Nat → Type 0) : Nat → Type 1 := f

  error: type mismatch
    f
  has type
    Nat → Type : Type 1
  but is expected to have type
    Nat → Type 1 : Type 2

  In LOGOS: covariant in the codomain, `Type 0 ≤ Type 1` — ACCEPTED.
-/

/-
  def contravariant (g : Type 1 → Nat) : Type 0 → Nat := g

  error: type mismatch
    g
  has type
    Type 1 → Nat : Type 2
  but is expected to have type
    Type → Nat : Type 1

  In LOGOS: contravariant in the domain, `Type 0 ≤ Type 1` — ACCEPTED.
-/

/-- The compiling neighbour for both: the levels must agree on the nose in Lean. -/
def sameLevel (f : Nat → Type 0) : Nat → Type 0 := f

/-! ### The place LOGOS is STRICTER, which is what makes a naive "just try it" translator
    produce a `Hole` and compose two bugs. -/

/-- Lean accepts this: `List` is universe-polymorphic, so `List Prop` is `List.{0} Prop`.
    In LOGOS a `Type 0`-pinned `List` compared against `List Prop` goes through
    `is_subtype_normalized`'s catch-all `App` case, which delegates to `def_eq_normalized`
    (type_checker.rs:1358-1360) — INVARIANT, so `Prop` and `Type 0` do not match and the term
    is rejected. Two opposite leniencies in one kernel. -/
def listOfProps : List Prop := [True, False]

end T11_001
