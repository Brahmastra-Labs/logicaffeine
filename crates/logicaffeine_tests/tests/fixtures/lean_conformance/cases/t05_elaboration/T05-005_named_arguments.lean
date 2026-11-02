/-!
@case      T05-005
@tier      5 · Elaboration surface
@title     Named arguments `f (x := 1)` — resolved into positional order before translation
@lean      named arguments, `f (y := 2) (x := 1)`, named arguments that skip explicit
           parameters and thereby eta-expand
@expect    translate
@target    A positional, left-nested `Term::App` spine in the DECLARATION's parameter order —
           never in the source's writing order. There is no named-argument form in LOGOS.
@evidence  kernel/src/term.rs:478 (`App(Box<Term>, Box<Term>)` — positional, binary,
           left-nested; arguments carry no names);
           kernel/src/type_checker.rs:144-170 (the `App` rule substitutes the argument for the
           `Pi`'s `param` — the binder's name comes from the FUNCTION's type, never from the
           call site);
           kernel/src/context.rs:199-206 (`binder_kinds` is a positional `Vec<ParamKind>` with
           no names; there is no named-parameter table anywhere in `Context`);
           kernel/src/elaborate.rs:1029-1045 (`elaborate_app` takes `explicit_args: &[Term]` —
           a positional slice)
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax`.** This is the
           cleanest case for that rule. Lean resolves named arguments during elaboration, so
           the `Expr` is already positional; a translator on `Expr` does nothing. A translator
           on `Syntax` must reimplement Lean's resolution — including the rule that a named
           argument which SKIPS a preceding explicit parameter causes Lean to eta-expand the
           call into a lambda over the skipped parameter (`mkLambda` around the remaining
           application). That is not a reordering; it is a different term. See `skipping`
           below, whose `Expr` is `fun a => sub3 a 2 9`, not a three-argument application.
@guard     Catches translating the arguments in SOURCE order. `f (y := 2) (x := 1)` written
           with `y` first must still emit `App(App(f, 1), 2)`. Because `sub3`'s parameters all
           have the same type (`Nat`), the source-ordered spine typechecks perfectly and
           computes a different number. Nothing in the kernel can notice: `Term::App` has no
           names to compare against.
@guard     Catches resolving names against the WRONG telescope. Lean's parameter names come
           from the declaration's signature, including binders introduced by `variable` and by
           auto-bound implicits (T05-008). A translator that reads names off the *body's*
           lambda binders, or off a `Syntax` signature that has not yet had `variable`s
           inserted, resolves `(x := ..)` to a different position.
@guard     Catches named arguments naming an IMPLICIT parameter. `idf (α := Nat) 3` supplies
           an implicit by name; the resulting `Expr` is the same as `@idf Nat 3`, so the
           emitted spine must include the type argument. A translator that treats
           "named argument" as "explicit argument" and appends it at the end of the explicit
           spine puts `Nat` where the value should go.
@guard     Catches named arguments combined with DEFAULTS. `withDefaults (c := 9)` supplies
           the third parameter by name and leaves the first two to their defaults; the `Expr`
           contains the default *values* inlined (T05-006), so the emitted spine has three
           arguments even though the source wrote one.
@guard     Catches the dependent case. In `depNamed` the named argument's value determines a
           later parameter's TYPE, so the reordering must respect the Pi telescope's dependency
           order; emitting the arguments in any other order makes
           `substitute(body_type, param, arg)` (type_checker.rs:166) substitute into a type
           that does not yet mention the binder.
@oracle    The emitted spine equals, term for term, the spine emitted for the same call written
           positionally; it typechecks; `normalize` agrees with Lean on every example; and for
           `skipping` the emitted term is a `Term::Lambda`, not an `App` spine.
-/

namespace T05_005

/-- All parameters share a type, so a mis-ordered spine typechecks. -/
def sub3 (x y z : Nat) : Nat := x * 100 + y * 10 + z

example : sub3 (y := 2) (x := 1) (z := 3) = 123 := rfl
example : sub3 (z := 3) (y := 2) (x := 1) = 123 := rfl
example : sub3 (y := 2) (x := 1) (z := 3) = sub3 1 2 3 := rfl

/-- A named argument that SKIPS an earlier explicit parameter. Lean eta-expands: the elaborated
    `Expr` is `fun x => sub3 x 2 9`, a LAMBDA, not a partial application. -/
def skipping : Nat → Nat := sub3 (y := 2) (z := 9)

example : skipping 1 = 129 := rfl

/-- A named argument naming an IMPLICIT parameter. -/
def idf {α : Type} (a : α) : α := a

example : idf (α := Nat) 3 = @idf Nat 3 := rfl

/-- Named arguments mixed with defaults — the `Expr` carries the inlined default values. -/
def withDefaults (a : Nat := 1) (b : Nat := 2) (c : Nat := 3) : Nat := a * 100 + b * 10 + c

example : withDefaults (c := 9) = 129 := rfl
example : withDefaults (b := 5) = 153 := rfl

/-- Dependent: the named argument's value appears in a later parameter's TYPE, so the
    positional order is forced by the Pi telescope. -/
def depNamed (n : Nat) (v : Fin (n + 1)) : Nat := v.val

example : depNamed (v := (0 : Fin 3)) (n := 2) = 0 := rfl

/-- Named argument on a constructor. -/
structure Point where
  x : Nat
  y : Nat

example : Point.mk (y := 2) (x := 1) = { x := 1, y := 2 } := rfl

end T05_005
