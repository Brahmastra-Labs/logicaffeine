/-!
@case      T05-003
@tier      5 · Elaboration surface
@title     Instance implicit `[Foo A]` — `ParamKind::Instance` and a flat, first-match instance list
@lean      [Foo A] instance-implicit binder, typeclass instance synthesis, `inferInstance`
@expect    translate
@target    An ordinary `Term::Pi` for the instance parameter (no binder info in the term), plus
           `set_binder_kinds(name, [.., ParamKind::Instance, ..])` and one
           `Context::add_instance(<class type>, <instance value>)` per Lean `instance`.
@evidence  kernel/src/elaborate.rs:1016-1027 (`ParamKind::Instance` — "resolved from the
           Context's instance database, but only AFTER the explicit arguments have been
           processed, so any metavariable in the class type is solved first");
           kernel/src/context.rs:184-188 (`add_instance(ty, value)` pushes onto a flat
           `Arc<Vec<(Term, Term)>>`) and :211-214 (`instances()` returns the slice — resolution
           is a linear scan, FIRST MATCH WINS, with no priority and no coherence check);
           kernel/src/elaborate.rs:1046-1080 (`elaborate_app_against`: the expected-type pass
           runs BEFORE instance resolution, so the class's type variable can be fixed by the
           surrounding context);
           kernel/src/term.rs:464-469 (the term itself is just a `Pi`);
           kernel/src/context.rs:271-369 (`add_structure` — a Lean `class` is a structure plus
           an instance registration; see T07-001)
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax`.** After
           elaboration the instance argument is an ordinary `Expr.app` carrying the *resolved*
           instance term. A translator on `Syntax` must re-run instance synthesis against
           LOGOS's flat list — which has different resolution semantics (first-match, no
           priorities, no coherence) — and will pick a different instance whenever more than
           one matches. Taking the resolved instance from the `Expr` sidesteps the whole
           question for the translated declarations; `add_instance` is then only needed so
           LOGOS-side terms elaborate.
@guard     Catches recording the position as `Implicit` instead of `Instance`. Both insert a
           metavariable, so simple cases work — but `Implicit` positions are solved by
           unification ONLY, so an instance that no argument's type pins (`defaultOf` below,
           where the class's type variable comes from the expected type) is left as an
           unsolved `Term::Var("?inst")`. `Instance` positions are deliberately deferred
           (elaborate.rs:1016-1027) so that resolution runs after the explicit arguments and
           the expected type have constrained the class type.
@guard     Catches instance-order sensitivity. `add_instance` appends to a flat `Vec`
           (context.rs:184-188) and resolution is first-match-wins with no priority mechanism.
           Lean resolves by priority and by most-recently-declared-first within a priority. A
           translator that registers instances in *source* order therefore reverses Lean's
           preference wherever two instances overlap. The emitted term typechecks either way,
           because both instances have the class type — the difference only shows up in what
           the function COMPUTES. `Sizeable Nat` versus an overlapping `Sizeable Nat` below is
           exactly that.
@guard     Catches forgetting that a Lean `class` needs `add_structure` (context.rs:271-369)
           before `add_instance` can mention its type. `add_structure` hard-codes the
           structure's sort to `Type 0` (context.rs:300-303), so a class in `Type u` for `u ≥ 1`
           or in `Prop` cannot be registered at all — see T03-012 and T07-011. A translator
           that registers the instance without the class silently has an instance whose type
           mentions an unresolvable `Global`.
@guard     Catches dropping the instance argument from the emitted application because "it is
           implicit". The `Expr` has it; the `Pi` expects it; omitting it makes the spine one
           argument short and `infer_type`'s `Pi` rule (type_checker.rs:156-167) substitutes
           the next argument into the wrong binder.
@guard     Catches an instance parameter whose class type mentions a LATER explicit binder.
           Because instance resolution is deferred to after the explicit pass, that works in
           LOGOS — but only if the kinds vector positions it correctly; an off-by-one makes the
           deferral apply to the wrong slot.
@oracle    The emitted `Pi` chain matches the translation of Lean's type; `binder_kinds` has
           `Instance` at exactly the instance-implicit positions; each Lean `instance` produced
           one `add_instance` call, in a recorded order; the emitted term contains no `?`-named
           `Term::Var`; and `normalize` of each example agrees with Lean.
-/

namespace T05_003

/-- A local class standing in for any Lean typeclass (no Mathlib). -/
class Sizeable (α : Type) where
  size : α → Nat

instance instSizeableNat : Sizeable Nat := ⟨fun n => n⟩
instance instSizeableBool : Sizeable Bool := ⟨fun b => if b then 1 else 0⟩

/-- The canonical instance-implicit binder. -/
def sizeOf' {α : Type} [Sizeable α] (a : α) : Nat := Sizeable.size a

/-- A RECURSIVE instance: resolving `Sizeable (List α)` requires first resolving
    `Sizeable α`, so the flat list is scanned more than once. -/
instance instSizeableList {α : Type} [Sizeable α] : Sizeable (List α) :=
  ⟨fun xs => xs.foldl (fun acc a => acc + Sizeable.size a) 0⟩

/-- An instance-implicit that NO explicit argument pins — the class type comes from the
    expected type, so resolution must be deferred past the expected-type pass. -/
class Defaultable (α : Type) where
  dflt : α

instance : Defaultable Nat := ⟨0⟩
instance : Defaultable Bool := ⟨false⟩

def defaultOf {α : Type} [Defaultable α] : α := Defaultable.dflt

/-- The instance parameter sits BETWEEN two explicit ones — the kinds vector is
    `[Implicit, Explicit, Instance, Explicit]`. -/
def sizeBoth {α : Type} (a : α) [Sizeable α] (b : α) : Nat :=
  Sizeable.size a + Sizeable.size b

/-- A class type mentioning a later binder's value is not expressible, but a class over a
    type built from an earlier implicit is — and it is what makes deferral observable. -/
def sizeList {α : Type} [Sizeable α] (xs : List α) : Nat := Sizeable.size xs

/-- The instance supplied explicitly, which is the same `Expr` as the synthesised one. -/
example : @sizeOf' Nat instSizeableNat 5 = 5 := rfl

example : sizeOf' 5 = 5 := rfl
example : sizeOf' true = 1 := rfl
example : sizeList [1, 2, 3] = 6 := rfl
example : (defaultOf : Nat) = 0 := rfl
example : sizeBoth 2 3 = 5 := rfl

end T05_003
