/-!
@case      T03-004
@tier      3 · Inductives and structures
@title     Indexed family (`Vect`) — `num_params` must be declared, and what is still lost
@lean      inductive family with indices, `InductiveVal.numIndices`, dependent elimination
@target    Context::add_indexed_inductive(name, full_arity_sort, num_params)  — **not**
           `add_inductive` — plus Context::add_constructor_checked per constructor, each
           constructor's declared return type carrying its own index expressions
@expect    degrade
@evidence  kernel/src/context.rs:371-382 (`add_indexed_inductive(name, sort, num_params)`:
           "the first `num_params` leading arguments are uniform parameters and the rest are
           indices that vary per constructor"); kernel/src/context.rs:38-44 (the
           `inductive_params` map, "Absence means *all of the arity is parameters*");
           kernel/src/context.rs:399-411 (`inductive_num_params` defaults to the full arity);
           kernel/src/type_checker.rs:602-678 (`infer_indexed_match`: the motive is
           `Π(indices…). Π(z : I params indices). Sort`, the result is the motive applied to
           the *discriminant's* index arguments, and coverage requires one case per
           constructor); kernel/src/type_checker.rs:631-638 (coverage is a bare count check —
           **no index unifier, no impossible-case elimination**);
           kernel/src/recursor.rs:59-71 (`derive_recursor`'s motive target is hard-coded to
           `Sort(Prop)` for a subsingleton `Prop` inductive and `Sort(Type 0)` otherwise);
           kernel/src/prelude.rs:1439 (`Eq` itself is registered this way, with
           `num_params = 2`)
@loss      INDEX_UNIFICATION. LOGOS's indexed `Match` does not unify the scrutinee's indices
           with each constructor's result indices. Every constructor needs a branch even when
           its indices provably cannot match — `Vect.cons` must be given a case in a match on
           a `Vect α 0`. Lean discharges those branches automatically; a LOGOS translation has
           to synthesise an inhabitant of an unreachable branch's type. See T03-019 and
           T04-014.
@loss      RECURSOR_MOTIVE_UNIVERSE. The derived recursor's motive targets `Type 0` (or `Prop`
           for subsingletons) and nothing else (recursor.rs:59-71). A Lean proof that eliminates
           an indexed family into `Type 1` — e.g. building a *type* by recursion on the index —
           has no derived recursor to use. Cumulativity (`Prop ≤ Type 0`,
           type_checker.rs:1330-1360) rescues the downward direction only.
@loss      INDEX_ARITHMETIC. Lean's `Vect α (n+1)` index is a `Nat` expression that the
           elaborator normalises during unification. LOGOS's indexed case type
           (`compute_indexed_case_type`, type_checker.rs:685-700) instantiates parameters from
           the discriminant and otherwise compares index terms up to `normalize`, whose fuel is
           10000 and which silently returns partial results (reduction.rs:52-69). Index
           expressions that Lean reduces cheaply can therefore fail to line up.
@guard     Catches calling `add_inductive` instead of `add_indexed_inductive`. That is the
           `reject` case T03-005 and it is *silent*: `inductive_num_params` then defaults to
           the full arity (context.rs:399-406), the length index is treated as a uniform
           parameter, and `compute_indexed_case_type` instantiates it from the discriminant —
           so `Vect.nil`'s declared index `0` and `Vect.cons`'s declared index `n+1` are both
           discarded and replaced by the scrutinee's. The family collapses into a
           *parameterised* type in which `nil : Vect α n` for every `n`.
@guard     Catches an off-by-one in `num_params`. For `Vect (α : Type) : Nat → Type` the answer
           is 1, not 0 and not 2. Passing 0 makes `α` an index, so every constructor's `α` is
           taken from the discriminant rather than being uniform, and `derive_recursor`
           (recursor.rs:72-73) builds a recursor with an empty parameter telescope.
@guard     Catches registering constructor types without their index expressions.
           `Vect.nil`'s registered type must literally end in `Vect α Zero` and `Vect.cons`'s
           in `Vect α (Succ n)`; `compute_indexed_case_type` reads those result indices off the
           declared type (type_checker.rs:685-700). A constructor declared to return
           `Vect α n` for a fresh `n` typechecks at registration (`add_constructor` is
           unchecked) and destroys the family.
@oracle    `inductive_num_params(name)` equals Lean's `numParams` and
           `inductive_num_indices(name)` equals Lean's `numIndices`; each constructor's
           registered result type carries its own index expression; an indexed `Match`
           typechecks; and the loss report contains INDEX_UNIFICATION.
-/

namespace T03_004

/-- Length-indexed vectors at `Type 0`. `α` is a parameter (uniform); the `Nat` is an index
    (it differs between `nil` and `cons`). -/
inductive Vect (α : Type) : Nat → Type where
  | nil : Vect α 0
  | cons : {n : Nat} → α → Vect α n → Vect α (n + 1)

/-- The index is genuinely non-uniform: these two have different types. -/
def v0 : Vect Nat 0 := Vect.nil
def v2 : Vect Nat 2 := Vect.cons 1 (Vect.cons 2 Vect.nil)

/-- Dependent elimination: the return *type* mentions the index. -/
def vlength {α : Type} {n : Nat} (_ : Vect α n) : Nat := n

example : vlength v2 = 2 := rfl

/-- A total head that is only well-typed because the index rules out `nil`. Lean discharges
    the `nil` branch by index unification; LOGOS must be *given* a `nil` branch whose type is
    uninhabited-in-context. -/
def vhead {α : Type} {n : Nat} : Vect α (n + 1) → α
  | Vect.cons a _ => a

example : vhead v2 = 1 := rfl

/-- Recursion over the family: the motive abstracts over the index as well as the
    scrutinee. -/
def vtoList {α : Type} : {n : Nat} → Vect α n → List α
  | _, Vect.nil => []
  | _, Vect.cons a t => a :: vtoList t

example : vtoList v2 = [1, 2] := rfl

/-- Index *arithmetic* in the result type — the shape that stresses index comparison under
    `normalize`. Every branch's index must line up definitionally with the declared one. -/
def vsnoc {α : Type} : {n : Nat} → Vect α n → α → Vect α (n + 1)
  | _, Vect.nil, a => Vect.cons a Vect.nil
  | _, Vect.cons x xs, a => Vect.cons x (vsnoc xs a)

example : vlength (vsnoc v2 9) = 3 := rfl

/-- Recursion *on the index itself*, producing a member of the family. The motive here is a
    family into `Type 0`, which is the only target `derive_recursor` offers. -/
def vreplicate {α : Type} (a : α) : (n : Nat) → Vect α n
  | 0 => Vect.nil
  | Nat.succ k => Vect.cons a (vreplicate a k)

example : vtoList (vreplicate (7 : Nat) 3) = [7, 7, 7] := rfl

/-- A family whose index is *not* `Nat`, so the translator cannot special-case the prelude's
    numeral bridge. `num_params` here is 1 and `num_indices` is 1. -/
inductive Tagged (α : Type) : Bool → Type where
  | plain : α → Tagged α false
  | marked : α → Tagged α true

def untag {α : Type} {b : Bool} : Tagged α b → α
  | Tagged.plain a => a
  | Tagged.marked a => a

example : untag (Tagged.marked 3) = 3 := rfl

/-- Zero parameters, one index: `num_params` must be 0 here, which is the opposite off-by-one
    from `Vect`. -/
inductive Parity : Nat → Type where
  | even : (k : Nat) → Parity (2 * k)
  | odd : (k : Nat) → Parity (2 * k + 1)

def parityWitness : {n : Nat} → Parity n → Nat
  | _, Parity.even k => k
  | _, Parity.odd k => k

example : parityWitness (Parity.odd 3) = 3 := rfl

end T03_004
