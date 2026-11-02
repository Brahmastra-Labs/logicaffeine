/-!
@case      T05-001
@tier      5 · Elaboration surface
@title     Implicit binders `{x : A}` — a `ParamKind`, not a term-level distinction
@lean      {x : A} implicit binder, implicit argument insertion, `binderInfo`
@expect    translate
@target    An ordinary `Term::Pi { param: "α", .. }` — implicitness is NOT in the term — plus
           `Context::set_binder_kinds("idf", vec![ParamKind::Implicit, ParamKind::Explicit])`.
@evidence  kernel/src/term.rs:464-469 (`Term::Pi { param, param_type, body_type }` — there is
           no binder-info field; implicit and explicit Pis are the same term);
           kernel/src/elaborate.rs:1016-1027 (`ParamKind::{Explicit, Implicit, Instance}` —
           "how a function parameter is supplied during elaboration");
           kernel/src/context.rs:199-206 (`set_binder_kinds(name, kinds)` /
           `binder_kinds(name)` — a per-GLOBAL, positional `Vec<ParamKind>`);
           kernel/src/elaborate.rs:1029-1045 (`elaborate_app`: a fresh metavariable is inserted
           for each `Implicit` position; `Instance` positions are DEFERRED until the explicit
           arguments have been unified);
           kernel/src/elaborate.rs:137-139 (metavariables are `Term::Var` names beginning with
           `?`, and must be fully instantiated before the kernel sees the term)
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax`.** After Lean
           elaborates, every implicit argument is already present in the `Expr` as an ordinary
           `app`. A translator working on `Syntax` has to re-derive which positions were
           implicit and re-run insertion; a translator working on `Expr` gets them for free and
           only needs `set_binder_kinds` so that *LOGOS-side* elaboration of hand-written terms
           behaves. This case is the simplest instance of that rule and the rest of Tier 5 is
           variations on it.
@guard     Catches omitting `set_binder_kinds` entirely. Nothing breaks immediately, because
           the emitted `Expr`-derived applications are fully explicit. It breaks later: any
           LOGOS-side term that applies the global with only its explicit arguments — a proof
           script, a hand-written lemma, another translated declaration that went through
           `elaborate` — gets `binder_kinds(name) == None` (context.rs:206), no metavariables
           are inserted, and the first explicit argument is checked against the IMPLICIT
           parameter's type. For `idf` that means `α` receives a value instead of a type.
@guard     Catches recording the kinds with the wrong ARITY or in the wrong ORDER.
           `binder_kinds` is a positional `Vec` (context.rs:201) with no names attached; Lean's
           `{α : Type} (a : α)` and `(a : α) {α : Type}` differ only in that vector. An
           off-by-one silently makes an explicit parameter implicit, and then
           `elaborate_app` inserts a metavariable where the caller's argument should have gone,
           shifting every later argument by one.
@guard     Catches leaving a metavariable in the output. If the translator lets LOGOS's own
           elaborator solve an implicit and does not call `instantiate` (elaborate.rs:143-186)
           before handing the term to the kernel, a `Term::Var("?m0")` reaches `infer_type` and
           is treated as an ordinary free variable — it will either fail lookup or, if some
           binder is literally named `?m0`, resolve to it.
@guard     Catches conflating implicit-ness with erasure. Lean's implicits are *inferred*, not
           *erased*: `@idf Nat 3` and `idf 3` elaborate to the same `Expr`. A translator that
           drops implicit arguments from the emitted application produces a `Term::App` spine
           of the wrong length, and `infer_type`'s `Pi` rule (type_checker.rs:156-167) then
           substitutes the wrong argument into the body type.
@oracle    The emitted `Pi` chain matches the translation of Lean's type exactly (implicitness
           is invisible); `ctx.binder_kinds(name)` equals the Lean declaration's binder-info
           vector, position for position; the emitted term contains no `Term::Var` whose name
           starts with `?`; and a LOGOS-side `elaborate_app` of the global with only its
           explicit arguments reproduces the fully-applied term.
-/

namespace T05_001

/-- The minimal implicit: `α` is inferred from the value argument. -/
def idf {α : Type} (a : α) : α := a

/-- Implicit in a NON-leading position — the binder-kind vector is `[Explicit, Implicit,
    Explicit]`, which is where off-by-one errors show up. -/
def constMid (_a : Nat) {β : Type} (b : β) : β := b

/-- An implicit that no value argument pins: it is solved from the EXPECTED type, which in
    LOGOS is `elaborate_app_against` (elaborate.rs:1046-1080), not `elaborate_app`. -/
def emptyOf {α : Type} : List α := []

/-- Two implicits, one of which is determined only through the other. -/
def pairUp {α : Type} {β : Type} (a : α) (b : β) : α × β := (a, b)

/-- An implicit whose type MENTIONS an earlier implicit — the Pi telescope is dependent, so
    the metavariables must be solved in order. -/
def applyTo {α : Type} {β : α → Type} (f : (x : α) → β x) (a : α) : β a := f a

/-- Implicit arguments supplied explicitly with `@` — the same `Expr`, so the same LOGOS
    term. See T05-004. -/
example : @idf Nat 3 = idf 3 := rfl

/-- An implicit in a THEOREM, where the argument is a proof. -/
theorem idfEq {α : Type} (a : α) : idf a = a := rfl

example : constMid 1 true = true := rfl
example : (emptyOf : List Nat) = [] := rfl
example : pairUp 1 true = (1, true) := rfl

end T05_001
