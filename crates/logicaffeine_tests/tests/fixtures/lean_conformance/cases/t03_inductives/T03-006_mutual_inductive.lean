/-!
@case      T03-006
@tier      3 · Inductives and structures
@title     `mutual inductive` — one transactional block registration, never N separate ones
@lean      mutual … inductive … inductive … end, cross-referencing constructors
@target    Context::add_mutual_inductives(&[MutualInductive { name, sort, num_params,
           constructors }, …]) — a single call carrying every member
@expect    translate
@evidence  kernel/src/context.rs:562-614 (`add_mutual_inductives`: positivity of every
           constructor is checked **against the whole block before any mutation**, then
           universe consistency in a temp environment, then all headers, then all constructors,
           then block membership — "the registration is TRANSACTIONAL: if any constructor
           violates positivity, nothing is added");
           kernel/src/positivity.rs:36-47 (`check_positivity_mutual` treats the block as one
           inductive: a sibling to the right of every arrow is a recursive occurrence, a sibling
           in a domain is a cross-block paradox);
           kernel/src/context.rs:605-612 (membership is recorded only for a block of ≥ 2
           members — a single-member "block" records nothing);
           kernel/src/context.rs:616-621 (`mutual_block_of`);
           kernel/src/recursor.rs:179-212 (the mutual recursor takes one motive per member and
           routes a sibling recursive occurrence to the sibling's fixpoint)
@guard     Catches registering the members one at a time with `add_inductive` /
           `add_constructor`. Both are **unchecked** (context.rs:253, :419), so the registration
           succeeds — but three things are lost. (1) Positivity is never checked *across* the
           block, so `Tree`'s constructor storing a `Forest` and `Forest`'s storing a
           `Tree → False` is admitted: a cross-block paradox that
           `check_positivity_mutual` exists to catch. (2) `mutual_blocks` is never populated,
           so `mutual_block_of` returns `None` and `derive_recursor` builds a *standalone*
           recursor that recurses only on itself — a sibling occurrence becomes an opaque
           field and structural recursion through the pair is no longer derivable. (3) The
           registration is not transactional, so a failure halfway leaves a half-declared
           block in the context.
@guard     Catches ordering the members wrong. The block's `num_params` is taken from
           `block[0]` (recursor.rs:193-199) and every other member is checked against it; and
           each member's constructors are registered in block order, which fixes the positional
           order every `Match` on that member depends on (type_checker.rs:253-266). Lean's
           declaration order must be preserved exactly.
@guard     Catches declaring the members with different parameter counts. `derive_recursor`
           refuses a block whose members disagree (recursor.rs:200-207) — that is T03-007. Here
           the counts agree, which is the precondition for this case being `translate` at all.
@guard     Catches forgetting that a sibling occurrence is a *recursive* occurrence for
           termination too. `check_termination_mutual` requires a call to **any** member to
           descend (termination.rs:21-44); a translator that emits two independent `Fix`es over
           a mutual pair escapes that requirement entirely (see T02-014).
@oracle    One `add_mutual_inductives` call per Lean `mutual` block; `mutual_block_of(m)`
           returns the full ordered member list for every member; each member's
           `inductive_num_params` is equal; `derive_recursor` succeeds for each member; and
           structural recursion across the pair typechecks.
-/

namespace T03_006

mutual
  /-- `Tree` stores a `Forest`; `Forest` stores `Tree`s. Neither is declarable alone. -/
  inductive Tree where
    | node : Nat → Forest → Tree

  inductive Forest where
    | nil : Forest
    | cons : Tree → Forest → Forest
end

mutual
  def treeSum : Tree → Nat
    | Tree.node n f => n + forestSum f

  def forestSum : Forest → Nat
    | Forest.nil => 0
    | Forest.cons t f => treeSum t + forestSum f
end

def sample : Tree :=
  Tree.node 1 (Forest.cons (Tree.node 2 Forest.nil)
                (Forest.cons (Tree.node 3 Forest.nil) Forest.nil))

example : treeSum sample = 6 := rfl

mutual
  /-- A mutual block of *propositions*, with indices. Both members are `Nat → Prop`, so both
      have `num_params = 0` and `num_indices = 1` — the agreement `add_mutual_inductives`
      requires. -/
  inductive EvenP : Nat → Prop where
    | zero : EvenP 0
    | succ : {n : Nat} → OddP n → EvenP (n + 1)

  inductive OddP : Nat → Prop where
    | succ : {n : Nat} → EvenP n → OddP (n + 1)
end

example : EvenP 4 :=
  EvenP.succ (OddP.succ (EvenP.succ (OddP.succ EvenP.zero)))

mutual
  /-- A three-member block, so the "take `num_params` from `block[0]`" rule has more than one
      opportunity to disagree. All three are parameterless. -/
  inductive A3 where
    | a : B3 → A3
  inductive B3 where
    | b : C3 → B3
    | bstop : B3
  inductive C3 where
    | c : A3 → C3
    | cstop : C3
end

mutual
  def sizeA : A3 → Nat
    | A3.a b => Nat.succ (sizeB b)
  def sizeB : B3 → Nat
    | B3.b c => Nat.succ (sizeC c)
    | B3.bstop => 0
  def sizeC : C3 → Nat
    | C3.c a => Nat.succ (sizeA a)
    | C3.cstop => 0
end

example : sizeA (A3.a (B3.b C3.cstop)) = 2 := rfl

mutual
  /-- A block with a shared *parameter*. Every member must agree on the count — here 1. -/
  inductive PTree (α : Type) where
    | pnode : α → PForest α → PTree α
  inductive PForest (α : Type) where
    | pnil : PForest α
    | pcons : PTree α → PForest α → PForest α
end

mutual
  def pcount {α : Type} : PTree α → Nat
    | PTree.pnode _ f => Nat.succ (pfcount f)
  def pfcount {α : Type} : PForest α → Nat
    | PForest.pnil => 0
    | PForest.pcons t f => pcount t + pfcount f
end

example : pcount (PTree.pnode (1 : Nat) (PForest.pcons (PTree.pnode 2 PForest.pnil) PForest.pnil)) = 2 := rfl

end T03_006
