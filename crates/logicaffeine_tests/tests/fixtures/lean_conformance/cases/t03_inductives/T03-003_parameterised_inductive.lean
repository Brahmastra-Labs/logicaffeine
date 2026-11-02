/-!
@case      T03-003
@tier      3 · Inductives and structures
@title     Parameterised inductive — the one shape where `add_inductive`'s default is right
@lean      inductive with uniform type parameters, `InductiveVal.numParams`, `numIndices = 0`
@target    Context::add_inductive(name, Pi{"α", Sort(Type 0), Sort(Type 0)}) — or, equivalently
           and preferably, Context::add_indexed_inductive(name, arity, num_params = arity) —
           plus Context::add_constructor_checked per constructor, each constructor's type
           carrying the parameter telescope explicitly
@expect    translate
@evidence  kernel/src/context.rs:246-255 (`add_inductive`: "All of the arity is treated as
           uniform parameters (0 indices) unless `set_inductive_params` records a smaller
           parameter count"); kernel/src/context.rs:399-411
           (`inductive_num_params` defaults to the full arity; `inductive_num_indices` is
           `arity − params`, hence 0); kernel/src/context.rs:384-389 (`inductive_arity` counts
           the leading `Π`s of the stored sort);
           kernel/src/recursor.rs:52-72 (`derive_recursor` splits the arity telescope at
           `num_params` and abstracts the recursor over the parameters);
           kernel/src/type_checker.rs:685-700 (`compute_indexed_case_type` instantiates a
           constructor's leading `num_params` from the *discriminant's* parameter arguments —
           the reason the split must be right)
@guard     Catches omitting the parameter from the *constructor* types. LOGOS constructors carry
           their full type including the parameter telescope: `cons` is
           `Π(α:Type 0). α → MyList α → MyList α`, not `α → MyList α → MyList α`. A constructor
           registered without the leading `Π(α)` is accepted unchecked by `add_constructor`
           (context.rs:419-428), and then `compute_case_type` builds a case type whose binders
           do not line up with the discriminant's parameters — producing a `Match` that fails
           to typecheck with a message about the *branch*, not about the declaration.
@guard     Catches the universe. A `MyList (α : Type 0)` must be declared with sort
           `Π(α : Type 0). Type 0`. Lean's `List : Type u → Type u` is universe-polymorphic and
           has **no** LOGOS counterpart at all (T01-007); this case is deliberately restricted
           to the monomorphic `Type 0` instance, and a translator must not silently turn the
           polymorphic Lean declaration into this one.
@guard     Catches a *non-uniform* "parameter". Lean rejects a parameter that varies across
           constructors, demoting it to an index. LOGOS's `add_inductive` will happily register
           it as a parameter (it is unchecked), and `compute_indexed_case_type` will then
           instantiate it from the discriminant — meaning the constructor's own choice of that
           argument is discarded. That is the T03-005 rejection; this case pins the boundary
           from the safe side.
@guard     Catches forgetting `add_constructor_checked`'s universe test. A `Type 0` inductive
           may not store a `Type 0`-typed field (`check_constructor_universes`,
           type_checker.rs:545-580). `MyList` storing an `α : Type 0` *value* is fine; a
           constructor storing the *type* `α` itself is not — see T03-022.
@oracle    `inductive_arity(name)` equals Lean's `numParams`, `inductive_num_indices(name)`
           is 0, every constructor's registered type begins with the parameter telescope, and
           a `Match` on a value of the type typechecks with the parameters supplied by the
           discriminant.
-/

namespace T03_003

/-- A monomorphic list at `Type 0`. Lean's own `List` is universe-polymorphic; this local
    re-declaration is the translatable fragment. -/
inductive MyList (α : Type) where
  | nil : MyList α
  | cons : α → MyList α → MyList α

/-- The parameter is uniform: every constructor returns `MyList α` for the *same* `α`. -/
def lengthOf : MyList Nat → Nat
  | MyList.nil => 0
  | MyList.cons _ t => Nat.succ (lengthOf t)

example : lengthOf (MyList.cons 1 (MyList.cons 2 MyList.nil)) = 2 := rfl

/-- Parameter-polymorphic *function* over the parameterised type. The `α` here is a binder of
    the function, not of the inductive — the inductive's parameter is instantiated at the
    call. -/
def lengthGen (α : Type) : MyList α → Nat
  | MyList.nil => 0
  | MyList.cons _ t => Nat.succ (lengthGen α t)

example : lengthGen Bool (MyList.cons true MyList.nil) = 1 := rfl

/-- Two parameters. The arity is 2, so `num_params` must be 2 and `num_indices` 0. -/
inductive MyPair (α : Type) (β : Type) where
  | mk : α → β → MyPair α β

def firstOf : MyPair Nat Bool → Nat
  | MyPair.mk a _ => a

example : firstOf (MyPair.mk 5 true) = 5 := rfl

/-- A parameterised type whose parameter appears in a *nested* position inside a constructor
    field, but still uniformly. Positivity accepts it; the parameter split is unchanged. -/
inductive Boxed (α : Type) where
  | wrap : (Bool → α) → Boxed α

def unwrap (α : Type) : Boxed α → α
  | Boxed.wrap f => f true

example : unwrap Nat (Boxed.wrap (fun b => if b then 1 else 0)) = 1 := rfl

/-- A parameterised type instantiated at two different `Type 0` arguments in the same file.
    Both instantiations share one LOGOS declaration — which is correct *here*, because the
    parameter is a term-level argument of the inductive, not a universe. -/
def natList : MyList Nat := MyList.cons 1 MyList.nil
def boolList : MyList Bool := MyList.cons true MyList.nil

example : lengthGen Nat natList = 1 := rfl
example : lengthGen Bool boolList = 1 := rfl

/-- Parameterised *and* recursive through the parameter position: `MyList (MyList Nat)` is a
    legitimate instantiation and needs no nesting machinery, because `MyList` occurs as a
    plain argument of a `Type 0` parameter. Contrast T03-008, where the inductive being
    declared occurs inside the container. -/
def nestedList : MyList (MyList Nat) := MyList.cons natList MyList.nil

example : lengthGen (MyList Nat) nestedList = 1 := rfl

end T03_003
