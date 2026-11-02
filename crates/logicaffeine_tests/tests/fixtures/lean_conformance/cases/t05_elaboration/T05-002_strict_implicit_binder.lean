/-!
@case      T05-002
@tier      5 · Elaboration surface
@title     Strict implicit `⦃x : A⦄` — LOGOS has no strict-implicit `ParamKind`
@lean      ⦃x : A⦄ / {{x : A}} strict implicit binder, BinderInfo.strictImplicit
@expect    degrade
@target    `ParamKind::Implicit` for the strict-implicit position, plus a loss record. The
           TERM is unaffected: `Term::Pi` carries no binder info at all.
@evidence  kernel/src/elaborate.rs:1016-1027 (`pub enum ParamKind { Explicit, Implicit,
           Instance }` — exactly three variants; there is no `StrictImplicit`);
           kernel/src/term.rs:464-469 (`Term::Pi` has no binder-info field, so the distinction
           cannot live in the term either);
           kernel/src/elaborate.rs:1029-1045 (`elaborate_app` inserts a fresh metavariable for
           EVERY `Implicit` position unconditionally — there is no "only if a later explicit
           argument follows" condition, which is precisely what strict-implicit means);
           kernel/src/context.rs:199-206 (`set_binder_kinds` stores the `Vec<ParamKind>`; a
           strict implicit has nowhere to be recorded)
@loss      STRICT_IMPLICIT_DEMOTION. Lean distinguishes `{x : A}` (insert eagerly) from
           `⦃x : A⦄` (insert only when a later explicit argument is supplied). LOGOS has one
           implicit kind, which behaves like `{x : A}`. Demoting `⦃⦄` to `{}` changes the
           ELABORATION BEHAVIOUR of partially-applied occurrences: `f` alone now has its
           strict implicit solved by a metavariable that nothing constrains, instead of
           remaining a function awaiting the argument. This is acceptable for the *fully
           applied* occurrences that dominate a translated library — they are already fully
           explicit in the `Expr` — and unacceptable for any LOGOS-side term that mentions the
           global unapplied, which the loss record must flag.
@loss      ETA_SENSITIVITY. The classic use of `⦃⦄` is on a coercion-like or membership-like
           function so that `f` used as a value does not eagerly instantiate. After demotion,
           `Term::Var("?m")` appears where the source had a genuine binder, and
           `unify_in`'s eta rule (elaborate.rs:254-268) may then unify it with something the
           source would have kept abstract.
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax`.** Strict
           implicit is a pure *elaboration* annotation: after Lean has elaborated, the `Expr`
           contains the argument explicitly, and `Expr.forallE`'s `BinderInfo` is only a hint.
           A translator on `Syntax` must reimplement Lean's "insert only before an explicit
           argument" rule; a translator on `Expr` only has to decide what to write into
           `set_binder_kinds` and record the loss.
@guard     Catches promoting `⦃⦄` to `Explicit` instead of `Implicit` "to be safe". That
           changes the ARITY every LOGOS-side caller must supply: `elaborate_app`
           (elaborate.rs:1029-1045) consumes `explicit_args` positionally against the kinds
           vector, so the caller's first real argument is checked against the promoted
           parameter's type. For `memOf` below that means a `Nat` is checked against `Type`.
@guard     Catches silently succeeding. `Term::Pi` is identical either way
           (term.rs:464-469), so the emitted term typechecks and round-trips perfectly; the
           ONLY observable difference is in `binder_kinds`. A translator that compares emitted
           terms to decide whether information was lost will conclude nothing was lost. The
           loss record is the only evidence, which is exactly why `degrade` requires it.
@guard     Catches losing the distinction between the *two* Lean spellings `⦃x : A⦄` and
           `{{x : A}}`, which are the same binder — a translator that treats them as different
           kinds emits two different `ParamKind` vectors for the same declaration depending on
           how it was written.
@guard     Catches demoting in a dependent telescope and breaking solvability. In `memPair`
           the strict implicit `β` is mentioned by a LATER explicit binder's type; once
           promoted to eager `Implicit`, `elaborate_app` inserts `?β` before that binder has
           been seen, and `?β` is only solved if the expected-type pass
           (`elaborate_app_against`, elaborate.rs:1046-1080) runs — which it does only when an
           expected type is supplied. Unapplied occurrences leave `?β` unsolved.
@oracle    The emitted `Pi` chain is identical to what T05-001's eager-implicit translation
           would produce; `binder_kinds` records `Implicit` at the strict-implicit position;
           and the loss record contains STRICT_IMPLICIT_DEMOTION naming the declaration and
           the position.
-/

namespace T05_002

/-- The canonical strict implicit: `α` is inserted only when the explicit `a` is supplied. -/
def memOf ⦃α : Type⦄ (a : α) : α := a

/-- The `{{ }}` spelling of the same binder. -/
def memOf' {{α : Type}} (a : α) : α := a

/-- Strict implicit with NO following explicit argument. Lean will never insert it from an
    application, so the only way to pin it is `@` or an expected type. -/
def nilOf ⦃α : Type⦄ : List α := []

/-- A strict implicit mentioned by a later explicit binder's TYPE — demotion changes when the
    metavariable is created relative to when it can be solved. -/
def memPair ⦃α : Type⦄ ⦃β : Type⦄ (a : α) (_b : β) : α := a

/-- Mixed kinds in one telescope: `[Implicit, StrictImplicit, Explicit]` in Lean, all of which
    collapse to `[Implicit, Implicit, Explicit]` in LOGOS. -/
def mixed {α : Type} ⦃β : Type⦄ (f : α → β) (a : α) : β := f a

/-- The behavioural difference, made observable: `memOf` unapplied is still a function of an
    explicit argument, and Lean does not eagerly solve `α`. -/
example : (fun (a : Nat) => memOf a) 3 = 3 := rfl

example : memOf 3 = 3 := rfl
example : memOf' true = true := rfl
example : (@nilOf Nat) = [] := rfl
example : memPair 1 true = 1 := rfl
example : mixed (fun n : Nat => n + 1) 2 = 3 := rfl

end T05_002
