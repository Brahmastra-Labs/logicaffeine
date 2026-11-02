/-!
@case      T00-005
@tier      0 · Kernel term layer
@title     `Expr.lam` — nesting, the mandatory domain type, and the dropped `BinderInfo`
@lean      Expr.lam, BinderInfo.{default, implicit, strictImplicit, instImplicit}, nested `fun`
@expect    translate
@target    Term::Lambda { param: String, param_type: Box<Term>, body: Box<Term> }
           — kernel/src/term.rs:464. Nested `fun`s become nested `Lambda`s, outermost first.
           Binder kind is recorded OUT OF BAND, on the enclosing declaration, via
           `Context::set_binder_kinds(name, kinds)` with `ParamKind::{Explicit, Implicit,
           Instance}`.
@evidence  kernel/src/term.rs:464-468 (`Term::Lambda` has exactly three fields — there is no
           `binder_info`); kernel/src/elaborate.rs:1016-1027 (`ParamKind` has THREE variants —
           `Explicit`, `Implicit`, `Instance`; Lean's `strictImplicit` has no counterpart, see
           T05-002); kernel/src/context.rs:201 (`set_binder_kinds` is keyed on a declaration
           NAME, so an anonymous lambda has nowhere to carry its binder kinds);
           kernel/src/type_checker.rs:112-131 (`Lambda` inference requires `param_type` to be a
           well-formed sort — the domain is not optional and cannot be inferred from context);
           kernel/src/type_checker.rs:474-500 (`check_type`'s Lambda-against-Pi special case,
           which is what tempts a translator to leave the domain as a placeholder)
@guard     Catches leaving `param_type` as `Term::Hole` when Lean's binder type came from the
           expected type rather than from the source. `check_type` (type_checker.rs:468-471)
           accepts ANY term against a `Hole` expected type and `types_equal`
           (type_checker.rs:1594-1597) treats a `Hole` as matching anything, so the lambda
           typechecks — and the `Hole` then propagates into the inferred `Pi`'s `param_type`,
           where it makes every later application of that function typecheck regardless of the
           argument's type. See T00-016; this is the single most common way a `Hole` escapes.
@guard     Catches recording binder kinds on the LAMBDA. There is no field for them. They must
           be lifted to the enclosing declaration with `set_binder_kinds`, and a lambda that is
           not the body of a named declaration (an argument, a match case, a motive) simply
           cannot carry them — so `fun {α : Type} (a : α) => a` passed as an argument loses its
           implicitness, and every use site the translator generates must then supply `α`
           explicitly. A translator that emits the same call shape for both kinds produces
           either an arity mismatch or, worse, an argument silently bound to the wrong parameter.
@guard     Catches flattening nesting. `Term::Lambda` is unary; `fun a b c => e` is three nested
           `Lambda`s and the ORDER is load-bearing — `infer_type` builds the `Pi` outside-in
           (type_checker.rs:125-130), so a reversed telescope produces a well-typed function of
           the wrong type, which then still applies to arguments of the swapped types.
@guard     Catches collapsing `strictImplicit` (`⦃x⦄`) onto `Implicit`. `ParamKind` has no strict
           variant (elaborate.rs:1018-1027), and `Implicit` makes the elaborator insert a
           metavariable EAGERLY (elaborate.rs:1029-1035) where Lean would have waited for a
           later explicit argument — changing which unification problems are solvable.
@oracle    No emitted `Lambda` has `Hole` as its `param_type`; the emitted telescope order
           matches the Lean `Expr` binder order; `infer_type` returns a `Pi` whose spine is the
           translation of Lean's inferred type; and for every named declaration the recorded
           `binder_kinds` vector has the same length as the declaration's leading `Pi` telescope.
-/

namespace T00_005

/-- Single explicit binder. -/
def idNat : Nat → Nat := fun a => a

/-- Three nested binders. Three nested `Term::Lambda`s, outermost = first. The body picks the
    middle one, so a reversed telescope is not caught by arity. -/
def pickMiddle : Nat → Nat → Nat → Nat := fun _a b _c => b

/-- Binders of *different* types, so a reversed telescope is a type error rather than a silent
    mis-binding — the control for `pickMiddle`. -/
def mixedTelescope : Nat → Bool → Nat := fun n b => if b then n else 0

/-- Implicit binder. The `{α}` must become `ParamKind::Implicit` on the *declaration*
    `implicitId`, not on the lambda. -/
def implicitId {α : Type} (a : α) : α := a

/-- Strict-implicit binder — `ParamKind` has no such kind. -/
def strictImplicitId ⦃α : Type⦄ (a : α) : α := a

/-- Instance-implicit binder. -/
def instanceDefault {α : Type} [Inhabited α] : α := default

/-- Mixed kinds in one telescope, in an order that matters: the instance argument must be
    resolved AFTER the explicit one (elaborate.rs:1023-1026), because `α` is only determined by
    `a`. A translator that records the kinds in source order but resolves them left-to-right
    will try to synthesise `Inhabited ?α` against an unsolved `?α`. -/
def mixedKinds {α : Type} [Inhabited α] (a : α) (b : Bool) : α := if b then a else default

/-- An ANONYMOUS lambda with an implicit binder, passed as an argument. There is no declaration
    to hang `set_binder_kinds` on, so the implicitness is unrepresentable. -/
def applyToPolyFn (f : {α : Type} → α → α) : Nat := f 3
def anonymousImplicit : Nat := applyToPolyFn (fun {_α} a => a)

/-- A lambda whose domain the Lean *source* never writes: it is recovered from the expected
    type. This is exactly the position where a translator reaches for `Hole`. -/
def domainFromExpectedType : Nat → Bool → Nat := fun n _ => n

end T00_005
