/-!
@case      T11-012
@tier      11 · Adversarial and soundness traps
@title     Nested inductive outside the unary-container fragment — the compiler refuses, and the
           workaround silently changes the type
@lean      nested inductive through List / Array / Prod / a user container, `Tree` with
           `List (Tree α)` children
@expect    reject
@target    (none — `compile_nested_inductive` refuses, and every hand-rolled substitute is a
           different inductive from the Lean one)
@evidence  kernel/src/inductive_compile.rs:220-232 (the specialisability check:
           `analyze_container_ctor` classifies each constructor argument, and
           `if kinds.iter().any(|k| matches!(k, ArgKind::Other(_)))` the compile fails with
           "container '{c}' constructor '{orig_ctor}' has an argument that is neither the
           element nor a recursive occurrence — specializing it is not supported (only pure
           containers like `List`/`TList`)" — so a container with ANY extra field, including
           a length index or a second element type, is outside the fragment);
           kernel/src/inductive_compile.rs:261-271 (the universe gate: `if decl.sort !=
           Term::Sort(Universe::Type(0))` the compile fails with "'{}' must be a `Type 0`
           inductive (specialized siblings are `Type 0`); higher-universe nesting is not
           supported"); kernel/src/inductive_compile.rs:318-327 (the emitted siblings are built
           with `sort: Term::Sort(Universe::Type(0))` and `num_params: 0` — so the
           specialisation is MONOMORPHIC and PARAMETERLESS, whatever the source container was);
           kernel/src/recursor.rs:193-207 (the emitted block goes through the mutual recursor
           derivation, which requires all members to agree on parameter count — satisfied here
           only because every sibling is forced to `num_params: 0`);
           kernel/src/context.rs:630 (`add_nested_inductive(&NestedDecl)` — the entry point);
           kernel/src/prelude.rs:609-652 (`TList`, the one prelude container inside the
           fragment)
@diag      E_NESTED_INDUCTIVE_OUTSIDE_FRAGMENT — must name the nesting container, say which of
           the two gates it fails (non-unary/non-pure container, or non-`Type 0` sort), and
           emit nothing. Must NOT silently fall back to a hand-written mutual block.
@guard     **TIER 11 THESIS.** The kernel accepting a term is not evidence of faithfulness, and
           here the trap is the WORKAROUND rather than the refusal. `compile_nested_inductive`
           refuses loudly, which is good — so the translator's next move is to write the mutual
           block by hand with `add_inductive`/`add_constructor`, which are UNCHECKED
           (context.rs:253, :419) and will take anything. The hand-rolled block typechecks.
           It is a DIFFERENT TYPE from the Lean one: the specialised sibling is a fresh
           inductive with no relationship to the container it specialises, so
           `Tree_List` is not `List Tree`, and every Lean lemma about `List` applied to a
           tree's children has no target. The kernel cannot notice, because there is nothing in
           the environment recording that the two were meant to be the same.
@guard     Catches losing the ISOS. `compile_nested_inductive` emits "the conversion isos, one
           pair per node … inner-first" (inductive_compile.rs:329-331) precisely so that the
           specialised sibling can be moved to and from the container. A hand-rolled mutual
           block that omits them produces a type that cannot be converted at all, so every
           function the source wrote over `List (Tree α)` must be rewritten over the sibling,
           and the rewriting is unchecked.
@guard     Catches the `num_params: 0` collapse. Every emitted sibling is `num_params: 0`
           (inductive_compile.rs:322-327), so a nested `List (Tree α)` for a POLYMORPHIC `Tree`
           specialises to a parameterless sibling — i.e. the element type is monomorphised. If
           the source instantiates the tree at two different element types, the two
           instantiations collapse onto one LOGOS global. That is T01-007's silent
           monomorphisation trap arriving through a different door, and because LOGOS is
           cumulative (type_checker.rs:1330-1360) the resulting terms very often still
           typecheck.
@guard     Catches the `Type 0` gate being read as a formality. It is not: the siblings are
           EMITTED at `Type 0` (inductive_compile.rs:322-327), so the gate at :261-271 is
           enforcing consistency with what the compiler will build. A translator that "works
           around" it by declaring the nested type at `Type 0` when the source had it at
           `Type u` has silently monomorphised the universe as well as the element type.
@guard     Catches nesting through `Prod`, `Sum`, `Array` or a user container with more than one
           field. `analyze_container_ctor` rejects any constructor argument that is neither the
           bare element nor a recursive occurrence (inductive_compile.rs:220-230), so `Prod` —
           whose constructor takes TWO different element types — is outside the fragment, and
           so is any indexed container. `Array` is absent entirely (T10-011).
@oracle    Translator exits non-zero with E_NESTED_INDUCTIVE_OUTSIDE_FRAGMENT for each
           declaration below and emits nothing. A harness asserts that every nested inductive
           anywhere in the corpus output was produced by `add_nested_inductive`
           (context.rs:630) — NOT by a sequence of raw `add_inductive`/`add_constructor` calls
           — and that the conversion isos it emits are present and registered.
-/

namespace T11_012

/-! ### Inside the fragment (the control): nesting through a unary, pure container. -/

inductive RoseList (α : Type) : Type where
  | node : α → List (RoseList α) → RoseList α

-- Written as an explicit mutual pair, which is exactly what `compile_nested_inductive`
-- produces: the nesting is compiled away into a mutual block plus conversion isos
-- (inductive_compile.rs:329-331). Lean's own structural-recursion checker requires the same
-- shape here, which is a useful hint that the fragment restriction is not arbitrary.
mutual
  def roseSize {α : Type} : RoseList α → Nat
    | RoseList.node _ cs => 1 + roseListSize cs

  def roseListSize {α : Type} : List (RoseList α) → Nat
    | [] => 0
    | c :: cs => roseSize c + roseListSize cs
end

example : roseSize (RoseList.node (1 : Nat) []) = 1 := by simp [roseSize, roseListSize]

/-! ### Outside the fragment: nesting through `Prod`, whose constructor takes two DIFFERENT
     element types, so `analyze_container_ctor` classifies one of them as `ArgKind::Other`
     (inductive_compile.rs:220-230). -/

inductive PairTree (α : Type) : Type where
  | leaf : α → PairTree α
  | fork : PairTree α × PairTree α → PairTree α

def pairSize {α : Type} : PairTree α → Nat
  | PairTree.leaf _ => 1
  | PairTree.fork (l, r) => 1 + pairSize l + pairSize r

example : pairSize (PairTree.leaf (1 : Nat)) = 1 := by simp [pairSize]

/-! ### Outside the fragment: nesting through an INDEXED container. The length index is an
     argument that is neither the element nor a recursive occurrence. -/

inductive Vect (α : Type) : Nat → Type where
  | nil : Vect α 0
  | cons : {n : Nat} → α → Vect α n → Vect α (n + 1)

inductive VTree (α : Type) : Type where
  | node : α → Vect (VTree α) 2 → VTree α

/-! ### Outside the fragment: nesting through a container with an EXTRA field alongside the
     element — a labelled list. -/

inductive Labelled (α : Type) : Type where
  | nil : Labelled α
  | cons : String → α → Labelled α → Labelled α

inductive LTree (α : Type) : Type where
  | node : α → Labelled (LTree α) → LTree α

/-! ### Outside the fragment by UNIVERSE: the nested inductive is not `Type 0`, so
     inductive_compile.rs:261-271 refuses even though the container is `List`. -/

inductive BigRose (α : Type 1) : Type 1 where
  | node : α → List (BigRose α) → BigRose α

/-! ### Two instantiations at different element types — the condition under which the
     `num_params: 0` sibling collapse (inductive_compile.rs:322-327) becomes unsound rather
     than merely lossy. -/

def natRose : RoseList Nat := RoseList.node 1 []
def boolRose : RoseList Bool := RoseList.node true []

/-! ### Nesting through `Array`, which is absent from LOGOS entirely (T10-011), so the
     fragment question does not even arise. -/

inductive ATree (α : Type) : Type where
  | node : α → Array (ATree α) → ATree α

end T11_012
