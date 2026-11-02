/-!
@case      T03-005
@tier      3 · Inductives and structures
@title     Params/indices misdeclaration — `add_inductive`'s default silently flattens a family
@lean      InductiveVal.numParams / numIndices, indexed inductive families
@target    (none — no sound translation via `add_inductive`; the *only* correct call is
           `add_indexed_inductive(name, sort, num_params)` with `num_params` equal to Lean's
           `numParams`, and a translator that cannot determine it must refuse)
@expect    reject
@diag      E_PARAM_INDEX_SPLIT_UNKNOWN — must name the inductive, report Lean's `numParams` and
           `numIndices`, and state that the declaration was **not** registered. Must not fall
           back to `add_inductive`.
@evidence  kernel/src/context.rs:246-255 (`add_inductive`'s doc: "All of the arity is treated
           as uniform parameters (0 indices) unless `set_inductive_params` records a smaller
           parameter count" — and the body is a bare `HashMap::insert`, **unchecked**);
           kernel/src/context.rs:38-44 (the `inductive_params` map; "Absence means *all of the
           arity is parameters* (0 indices) — so every non-indexed inductive behaves exactly as
           before this map existed");
           kernel/src/context.rs:399-411 (`inductive_num_params` falls back to
           `inductive_arity`; `inductive_num_indices` is then 0);
           kernel/src/type_checker.rs:200-300 vs :602-678 (the **non-indexed** `Match` path is
           taken whenever `num_indices == 0`; the indexed path, with its motive-over-indices
           discipline, is never entered);
           kernel/src/type_checker.rs:685-714 (`compute_indexed_case_type`: a constructor's
           leading `num_params` arguments are instantiated from the **discriminant's**
           parameter arguments — so anything counted as a parameter has its
           constructor-declared value *discarded*);
           kernel/src/recursor.rs:52-73 (`derive_recursor` splits the arity at `num_params`;
           a wrong split yields a recursor with the wrong motive arity);
           kernel/src/reduction.rs (ι-reduction skips exactly
           `inductive_declared_params` arguments of a constructor, context.rs:391-397 — a wrong
           count skips the wrong arguments)
@guard     **This is the central trap of the tier.** The natural translation of
           `inductive Vect (α : Type) : Nat → Type` is "register the arity
           `Π(α:Type 0). Nat → Type 0`", i.e. `add_inductive`. That call succeeds. It is also
           silently wrong: `inductive_num_params` now reports **2**, so the length index is
           treated as a uniform parameter. Every subsequent `Match` takes the non-indexed path
           (type_checker.rs:200-300), the motive never abstracts over the index, and the
           constructors' declared indices (`Vect α 0`, `Vect α (n+1)`) are replaced by whatever
           the discriminant carries. The family collapses: after translation,
           `Vect.nil : Vect α n` for *every* `n`, and `vhead : Vect α (n+1) → α` becomes a
           partial function that the kernel believes is total. Nothing errors — the declaration
           registers, the matches typecheck, and the proofs "go through".
@guard     Catches the same bug through `add_structure`. `add_structure` calls
           `add_indexed_inductive(name, ind_type, params.len())` (context.rs:301) — it always
           declares *all* of its telescope as parameters. So a Lean structure with an index
           cannot be routed through `add_structure` at all; doing so re-introduces this
           collapse under a different API.
@guard     Catches the off-by-one in the other direction: declaring *too few* parameters. With
           `num_params = 0` for `Vect`, `α` becomes an index, so `compute_indexed_case_type`
           instantiates no parameters and each constructor must supply its own `α` from the
           discriminant's index list — which changes the case types and, where the sorts line
           up, still typechecks. `derive_recursor` then builds a recursor with an empty
           parameter telescope (recursor.rs:72-73) whose motive has the wrong arity.
@guard     Catches deriving the split heuristically from the arity's shape. It cannot be read
           off the sort: `Π(α:Type 0). Nat → Type 0` is the arity of both a 2-parameter type
           and a 1-parameter/1-index family. The split is a *separate fact* that lives in
           `InductiveVal.numParams` and must be transported explicitly.
@guard     Catches "it round-tripped, so it is fine". Because LOGOS is cumulative
           (type_checker.rs:1330-1360) and `add_inductive`/`add_constructor` are unchecked, a
           collapsed family typechecks end to end. `infer_type` succeeding is not evidence.
@oracle    Translator exits non-zero with E_PARAM_INDEX_SPLIT_UNKNOWN for any inductive it
           cannot split correctly, and for every inductive it *does* register,
           `inductive_num_params(name) == numParams` and
           `inductive_num_indices(name) == numIndices`. Additionally: a match on `Vect α 0`
           must **not** accept a `Vect.cons` value, and `Vect.nil` must **not** typecheck at
           index `1`.
-/

namespace T03_005

/-- `numParams = 1`, `numIndices = 1`. Registering arity-as-parameters makes it 2 and 0. -/
inductive Vect (α : Type) : Nat → Type where
  | nil : Vect α 0
  | cons : {n : Nat} → α → Vect α n → Vect α (n + 1)

/-- The two constructors disagree on the index. That disagreement is the *entire* content of
    the declaration, and it is exactly what a parameter split of 2 discards. -/
def emptyVect : Vect Nat 0 := Vect.nil
def oneVect : Vect Nat 1 := Vect.cons 5 Vect.nil

/-- Lean rejects `(Vect.nil : Vect Nat 1)`. A collapsed translation accepts it, because
    `nil`'s declared index `0` was replaced by the discriminant's. -/
example : Vect Nat 0 := Vect.nil

/-- Total by index reasoning. Under a collapsed family this function is applied to `nil`
    and there is no branch — Lean's guarantee is gone. -/
def vhead {α : Type} {n : Nat} : Vect α (n + 1) → α
  | Vect.cons a _ => a

example : vhead oneVect = 5 := rfl

/-- Exhaustiveness at a *fixed* index: Lean knows `cons` is impossible here. -/
def isEmpty {α : Type} : Vect α 0 → Bool
  | Vect.nil => true

example : isEmpty emptyVect = true := rfl

/-- `Eq` itself is an indexed family with `numParams = 2`, `numIndices = 1` — and the LOGOS
    prelude registers it that way (prelude.rs:1439, `add_indexed_inductive("Eq", …, 2)`). A
    translator that flattened it would make `refl` inhabit `a = b` for arbitrary `b`. -/
example (a : Nat) : a = a := rfl

/-- A family with **two** indices and one parameter: `numParams = 1`, `numIndices = 2`. A
    single integer is being transported; an off-by-one here mixes a parameter with an index. -/
inductive Path (α : Type) : α → α → Type where
  | refl : (a : α) → Path α a a
  | step : {a b c : α} → Path α a b → Path α b c → Path α a c

def pathEnds {α : Type} {a b : α} (_ : Path α a b) : α × α := (a, b)

example : pathEnds (Path.refl (3 : Nat)) = (3, 3) := rfl

/-- Zero parameters, one index — `numParams = 0`. A translator that assumes "the leading
    `Type`-sorted binders are the parameters" gets 0 here by luck and 2 for `Vect` by the same
    rule; the rule is simply wrong. -/
inductive Even : Nat → Prop where
  | zero : Even 0
  | plusTwo : {n : Nat} → Even n → Even (n + 2)

example : Even 4 := Even.plusTwo (Even.plusTwo Even.zero)

/-- A *non-uniform* would-be parameter: `α` varies across the constructors' result types, so
    Lean demotes it to an index (`numParams = 0`, `numIndices = 1`). Declaring it a parameter
    is the same collapse, now on a type-valued position. -/
inductive Shape : Type → Type 1 where
  | ofNat : Nat → Shape Nat
  | ofBool : Bool → Shape Bool

def shapeIsNat {α : Type} : Shape α → Bool
  | Shape.ofNat _ => true
  | Shape.ofBool _ => false

example : shapeIsNat (Shape.ofNat 1) = true := rfl
example : shapeIsNat (Shape.ofBool true) = false := rfl

end T03_005
