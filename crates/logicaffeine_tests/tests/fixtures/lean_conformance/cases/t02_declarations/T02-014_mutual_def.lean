/-!
@case      T02-014
@tier      2 · Declarations
@title     `mutual def` block — one `Term::MutualFix`, projected once per member
@lean      mutual … end over `def`s, Lean's `WellFounded`/structural mutual compilation
@target    Term::MutualFix { defs: Vec<(String, Term)>, index: usize }, registered with one
           Context::add_definition per member, each at its own `index`
@expect    translate
@evidence  kernel/src/term.rs:443-545 (`MutualFix { defs, index }` is a *term*, not a
           declaration form — the whole block is one term and each member is a projection of
           it); kernel/src/type_checker.rs:349-377 (typing a `MutualFix` infers every member's
           type structurally, runs `termination::check_termination_mutual`, then re-checks each
           body with *all* siblings in scope, and returns `types[index]`);
           kernel/src/termination.rs:21-44 (`GuardContext.fix_positions` maps every member of
           the block to its structural argument position — "a call to ANY member must
           decrease")
@guard     Catches emitting each member as its own single `Term::Fix`. The sibling call is then
           a free `Var`/`Global` that either does not resolve or resolves to a *separately*
           declared fixpoint; in the latter case `check_termination` (which is single-block,
           type_checker.rs:321-343) sees the sibling call as a call to an unrelated function
           and imposes no descent requirement on it. Two mutually recursive functions each
           "structurally recursive" on nothing at all then pass the guard, and the pair
           diverges — the classic way to admit a non-terminating definition into a kernel.
@guard     Catches getting `index` wrong. `MutualFix { defs, index }` returns `types[index]`
           (type_checker.rs:376). All members are typed and all bodies are checked regardless,
           so an off-by-one in `index` produces a term that **typechecks fine** and simply
           denotes the wrong function. Registering `isEven` with the body of `isOdd` is a
           silent, total semantic inversion that no kernel check will catch.
@guard     Catches re-building the block per member. Emitting N separate `MutualFix` terms that
           differ only in `index` is correct but quadratic: `infer_type` on a `MutualFix` types
           *and* re-checks every body (type_checker.rs:356-374), so an N-member block costs
           O(N²) body checks, on top of `substitute`'s O(size)-per-binder cost
           (type_checker.rs:1088). The block should be built once and shared.
@guard     Catches assuming Lean's mutual block shape survives. Lean may compile a `mutual`
           block by structural recursion, by well-founded recursion, or by packing the
           arguments into a sum type. Only the *structural* compilation has a LOGOS
           counterpart; the others are T04-004/T04-015 rejections. The translator must read how
           Lean actually compiled the block, not the surface `mutual` keyword.
@oracle    One `MutualFix` term with `defs.len()` equal to the block size; each member is
           registered with a distinct, correct `index`; `infer_type` succeeds (so
           `check_termination_mutual` passed); and each member's inferred type matches the
           translation of Lean's type for that member.
-/

namespace T02_014

mutual
  /-- Structural mutual recursion on `Nat`: each call descends by one constructor. -/
  def isEven : Nat → Bool
    | 0 => true
    | Nat.succ n => isOdd n

  def isOdd : Nat → Bool
    | 0 => false
    | Nat.succ n => isEven n
end

example : isEven 4 = true := rfl
example : isOdd 4 = false := rfl
example : isEven 7 = false := rfl

/-- The `index` guard made observable: these two disagree on every odd input, so an
    off-by-one in `index` flips them and nothing else changes. -/
example : isEven 3 ≠ isOdd 3 := by decide

mutual
  inductive Tree where
    | node : Forest → Tree
  inductive Forest where
    | nil : Forest
    | cons : Tree → Forest → Forest
end

mutual
  /-- Mutual recursion over a *mutual inductive* — the shape the guard's
      `fix_positions` map exists for: the descent is on different types in different
      members. -/
  def treeSize : Tree → Nat
    | Tree.node f => Nat.succ (forestSize f)

  def forestSize : Forest → Nat
    | Forest.nil => 0
    | Forest.cons t f => treeSize t + forestSize f
end

example : treeSize (Tree.node (Forest.cons (Tree.node Forest.nil) Forest.nil)) = 2 := rfl

-- A three-member block, so `index` has more than two values to be wrong about and the
-- quadratic-recheck guard has something to bite on.
mutual
  def modThree0 : Nat → Bool
    | 0 => true
    | Nat.succ n => modThree2 n

  def modThree1 : Nat → Bool
    | 0 => false
    | Nat.succ n => modThree0 n

  def modThree2 : Nat → Bool
    | 0 => false
    | Nat.succ n => modThree1 n
end

example : modThree0 6 = true := rfl
example : modThree1 4 = true := rfl
example : modThree2 5 = true := rfl

end T02_014
