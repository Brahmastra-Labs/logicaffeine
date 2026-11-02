/-!
@case      T05-015
@tier      5 · Elaboration surface
@title     Elaboration postponement order — LOGOS's elaborator has fixed phases, not a queue
@lean      postponed elaboration of lambdas/literals against unknown expected types, `default_instance`
           resolution after postponement, `(· + 1)` sections, numeric-literal defaulting to `Nat`
@expect    degrade
@target    The term Lean's elaborator ultimately produced, with every postponed decision
           already made — including the DEFAULT instance Lean picked when nothing constrained a
           numeric literal's type. Plus a loss record naming each decision that LOGOS's own
           elaborator would make differently.
@evidence  kernel/src/elaborate.rs:1029-1045 (`elaborate_app`: implicit positions get a fresh
           metavariable immediately; INSTANCE positions are DEFERRED — "only AFTER the explicit
           arguments have been processed" — this two-phase order is the whole of LOGOS's
           postponement machinery);
           kernel/src/elaborate.rs:1046-1080 (`elaborate_app_against`: the expected-type pass
           runs BEFORE instance resolution, a third fixed phase — there is no queue, no
           re-scheduling, and no "try again when more is known" loop);
           kernel/src/elaborate.rs:216-292 (`unify_in` is a single pass: a constraint that
           cannot be solved now is a FAILURE, not a postponement — there is no constraint
           store);
           kernel/src/context.rs:184-188 (`add_instance` is a flat `Vec`, first-match-wins:
           there is no `default_instance` notion at all);
           kernel/src/term.rs:538 (`Term::Lit` — a literal's type is decided by
           `infer_type` (type_checker.rs:294-311) from the literal's KIND, not from context:
           `Literal::Nat` is always `Nat`, `Literal::Int` always `Int`)
@loss      POSTPONEMENT_ORDER. Lean's elaborator maintains a queue of postponed problems and
           re-runs them as metavariables get solved, then applies `default_instance`s to
           whatever is still unresolved. LOGOS has three fixed phases and no queue. Any
           decision Lean made by postponement — most importantly, which `OfNat` instance a bare
           numeric literal got — has to be baked into the emitted term. The loss record must
           name each such decision, because a later re-elaboration on the LOGOS side will not
           reproduce it.
@loss      DEFAULT_INSTANCE_ABSENCE. Lean's `@[default_instance]` is what makes `fun x => x + 1`
           elaborate at `Nat` rather than staying polymorphic. LOGOS has no default-instance
           mechanism (context.rs:184-188 is the whole instance system), so the defaulted type
           is not recoverable from the LOGOS side and must be recorded. See T07-007.
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax`.** This case is
           the reason the rule is stated as *post*-elaboration and not merely "after parsing".
           `fun x => x + 1` has no type in `Syntax`; it acquires one only after Lean's queue
           drains and defaulting fires. A translator on `Syntax` must reimplement the queue,
           the re-scheduling, and the defaulting table — three mechanisms LOGOS does not have
           and Lean does not document as stable.
@guard     Catches translating a numeric literal without its resolved type. `Term::Lit`'s type
           is read off the literal's KIND (type_checker.rs:294-311): `Literal::Nat` is `Nat`
           and `Literal::Int` is `Int`, and LOGOS's `Nat` (inductive) and `Int` (opaque, with
           axiomatised ring laws) are completely different things (LOGOS_FACTS). A translator
           that emits `Literal::Nat` for a Lean literal Lean elaborated at `Int` has changed
           the type AND lost every computation rule — see T10-004.
@guard     Catches the instance-phase order. LOGOS resolves instances only after the explicit
           arguments and the expected type (elaborate.rs:1029-1080). A translator that
           re-resolves instances itself, earlier, may pick a different one from the flat
           first-match list — and since both have the class type, the emitted term typechecks
           and computes differently.
@guard     Catches assuming a failed constraint will be retried. `unify_in`
           (elaborate.rs:216-292) has no constraint store: a constraint that cannot be solved
           at the moment it is raised is a hard failure. A translator that emits terms in a
           different ORDER than Lean elaborated them (say, all types before all values) can
           therefore raise a constraint before its solution is known and fail on a term Lean
           accepted.
@guard     Catches `(· + 1)` sections and other syntax whose elaboration is postponed twice —
           once for the section's own lambda and once for the `HAdd` instance. The emitted term
           must be the fully-resolved `fun x : Nat => Nat.add x 1`-shaped term, not a
           polymorphic skeleton.
@guard     Catches a postponed decision that Lean made from a LATER line. In `laterPin` below
           nothing in the definition constrains the literal; the ascription on the following
           `example` is what pins it in the source's reading order but not in the elaborator's.
@oracle    Every numeric literal in the emitted term carries the type Lean gave it (`Nat` vs
           `Int` vs a user `OfNat`), checkable by `infer_type` on the emitted `Lit`; every
           instance argument is the one in Lean's `Expr`; the emitted term contains no `Hole`
           and no `?`-prefixed `Var`; and the loss record contains POSTPONEMENT_ORDER listing
           each defaulted decision.
-/

namespace T05_015

/-- The canonical postponed elaboration: nothing constrains `x`, so Lean's
    `default_instance` for `OfNat`/`HAdd` fires and the whole thing lands at `Nat`. -/
def addOne := fun x => x + 1

example : addOne 3 = 4 := rfl

/-- A section, which postpones the lambda AND the instance. -/
def addOne' := (· + 1)

example : addOne' 3 = 4 := rfl

/-- The same expression pinned at `Int` — the literal's LOGOS kind differs, and `Int` is
    axiomatised rather than inductive. -/
def addOneInt : Int → Int := fun x => x + 1

example : addOneInt 3 = 4 := rfl

/-- A literal whose type is decided only by the EXPECTED type of the enclosing definition. -/
def laterPin : Int := 5

/-- A lambda elaborated against an unknown expected type, then constrained by its use. -/
def applyIt (f : Nat → Nat) : Nat := f 0

example : applyIt (fun x => x + 1) = 1 := rfl

/-- Instance resolution that can only succeed AFTER the explicit argument pins the class's
    type variable — the two-phase order `elaborate_app` implements. -/
class Sizeable (α : Type) where
  size : α → Nat

instance : Sizeable Nat := ⟨fun n => n⟩
instance : Sizeable Bool := ⟨fun b => if b then 1 else 0⟩

def sizeOf' {α : Type} [Sizeable α] (a : α) : Nat := Sizeable.size a

example : sizeOf' 5 = 5 := rfl
example : sizeOf' true = 1 := rfl

/-- Instance resolution that needs the EXPECTED type, because no explicit argument mentions
    `α` — the third phase. -/
class Defaultable (α : Type) where
  dflt : α

instance : Defaultable Nat := ⟨0⟩
instance : Defaultable Bool := ⟨false⟩

def defaultOf {α : Type} [Defaultable α] : α := Defaultable.dflt

example : (defaultOf : Bool) = false := rfl

/-- A user `OfNat` instance: the literal `3` elaborates to a value of a custom type, and which
    instance fires is a postponed decision. -/
structure MyNum where
  v : Nat

instance : OfNat MyNum 3 := ⟨⟨3⟩⟩

def three : MyNum := 3

example : three.v = 3 := rfl

end T05_015
