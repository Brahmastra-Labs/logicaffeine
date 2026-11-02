/-!
@case      T05-004
@tier      5 · Elaboration surface
@title     `@f` — explicit-all application is a surface notation that vanishes in the `Expr`
@lean      @f, @id, explicit-all application, `Expr.app` spines
@expect    translate
@target    The same left-nested `Term::App` spine that the non-`@` form produces:
           `App(App(Global("idf"), Global("Nat")), Lit(Nat 3))`. There is nothing to emit for
           the `@` itself.
@evidence  kernel/src/term.rs:478 (`App(Box<Term>, Box<Term>)` — binary and left-nested; an
           application carries no record of how its arguments were supplied);
           kernel/src/type_checker.rs:144-170 (the `App` rule: the function's type is whnf'd to
           a `Pi`, the argument is checked against `param_type`, and the result is
           `substitute(body_type, param, arg)` — the same rule for every argument regardless of
           binder kind);
           kernel/src/elaborate.rs:1029-1045 (`elaborate_app` is what CONSUMES `binder_kinds`;
           an already-explicit spine never goes through it);
           kernel/src/context.rs:199-206 (`binder_kinds` is per-global and is not consulted by
           the kernel at all)
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax`.** `@f a b` and
           `f b` elaborate to the *same* `Expr`; the `@` exists only to suppress insertion. A
           translator on `Expr` has literally nothing to do for this case — which is the point:
           it is the cheapest demonstration that the `Syntax`-level surface of Tier 5 is noise.
           A translator on `Syntax` must implement `@` as a mode flag threaded through its own
           reimplementation of argument insertion, and every divergence from Lean's rules is a
           silent mistranslation.
@guard     Catches emitting `set_binder_kinds` as though `@` changed the DECLARATION. `@` is a
           property of a use site, not of the definition; a translator that records
           `[Explicit, Explicit]` for `idf` because it saw `@idf Nat 3` makes every *other*
           call site in the library wrong, since LOGOS-side elaboration will then refuse to
           insert the type argument.
@guard     Catches spine-direction errors. `Term::App` is binary and LEFT-nested
           (term.rs:478), so `@f a b c` is `App(App(App(f,a),b),c)`. A translator that folds
           the argument list right-nested produces `App(f, App(a, App(b,c)))`, which fails
           `infer_type` with `NotAFunction` — but only when `a` is not itself a function. For
           `@compose` below, where several arguments ARE functions, a right-nested fold can
           typecheck and mean something entirely different.
@guard     Catches `@` on a UNIVERSE-polymorphic constant. Lean's `@f.{u}` supplies levels too.
           In LOGOS only `Term::Const { name, levels }` (term.rs:458) instantiates levels, and
           it resolves ONLY against `Context::universe_polys` — i.e. only for definitions
           registered with `add_universe_poly` (context.rs:218). `@List.nil.{0}` has no LOGOS
           counterpart for an *inductive* constructor at all (T01-007). A translator must not
           quietly drop the level arguments and emit a plain `Global`.
@guard     Catches `@` on a constant with an INSTANCE parameter. `@sizeOf' Nat inst 5` supplies
           the instance positionally; the emitted spine is the same as the synthesised one only
           if the translator takes the instance term from the `Expr` rather than re-resolving
           it against LOGOS's flat, first-match instance list (context.rs:184-188), which may
           pick a different instance.
@guard     Catches partial application. `@idf Nat` is a well-typed value of type `Nat → Nat`;
           the emitted `App(Global("idf"), Global("Nat"))` must typecheck on its own, so the
           translator must not eta-expand or complete the spine.
@oracle    The emitted term for `@f …` is byte-identical to the emitted term for the
           corresponding non-`@` application; it typechecks; `binder_kinds` for the callee is
           unchanged by the presence of `@`; and the emitted `App` spine is left-nested.
-/

namespace T05_004

def idf {α : Type} (a : α) : α := a

/-- `@` and no-`@` elaborate to the same `Expr`, hence to the same LOGOS term. -/
example : @idf Nat 3 = idf 3 := rfl

/-- Partial `@` application: a well-typed value in its own right. -/
def natId : Nat → Nat := @idf Nat

/-- A longer spine with function-typed arguments, where a right-nested fold can typecheck. -/
def compose {α β γ : Type} (g : β → γ) (f : α → β) (a : α) : γ := g (f a)

example : @compose Nat Nat Nat (· + 1) (· * 2) 3 = 7 := rfl

/-- `@` on a constant with an instance parameter. -/
class Sizeable (α : Type) where
  size : α → Nat

instance instSizeableNat : Sizeable Nat := ⟨fun n => n⟩

def sizeOf' {α : Type} [Sizeable α] (a : α) : Nat := Sizeable.size a

example : @sizeOf' Nat instSizeableNat 5 = sizeOf' 5 := rfl

/-- `@` on a universe-polymorphic constant, with the level supplied explicitly. LOGOS can
    express this only via `Term::Const { levels }`, and only for `add_universe_poly`
    definitions — never for an inductive's constructor. -/
def polyId.{v} {α : Sort v} (a : α) : α := a

example : @polyId.{1} Nat 3 = 3 := rfl
example : @List.nil.{0} Nat = ([] : List Nat) := rfl

/-- `@` on a structure projection: the projection's own implicit parameters become explicit. -/
structure Pair (α β : Type) where
  fst : α
  snd : β

example : @Pair.fst Nat Bool ⟨1, true⟩ = 1 := rfl

example : natId 7 = 7 := rfl

end T05_004
