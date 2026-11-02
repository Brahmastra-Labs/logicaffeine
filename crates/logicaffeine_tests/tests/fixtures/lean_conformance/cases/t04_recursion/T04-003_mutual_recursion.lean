/-!
@case      T04-003
@tier      4 · Recursion and pattern matching
@title     `mutual` recursive definitions — one `MutualFix` block, one shared Giménez guard
@lean      mutual def ... end, mutually recursive functions and mutual structural recursion
@expect    translate
@target    Term::MutualFix { defs: [("isEven", <λn. Match ..>), ("isOdd", <λn. Match ..>)],
                             index: 0 }   -- and `index: 1` for the sibling's reference
@evidence  kernel/src/term.rs:511-525 (`Term::MutualFix { defs, index }` — a block plus the
           index of the member this occurrence denotes);
           kernel/src/type_checker.rs:341-372 (`MutualFix` inference: every member's type is
           inferred structurally, then `check_termination_mutual`, then every body is
           re-checked with ALL siblings bound);
           kernel/src/termination.rs:79-110 (`check_termination_mutual` computes each member's
           structural ARGUMENT POSITION first, then checks every body so that a call to ANY
           member passes, at that member's position, something smaller than the CURRENT body's
           structural parameter);
           kernel/src/termination.rs:218-232 (`fix_positions.get(name)` — the position is
           per-callee, not per-caller)
@guard     Catches emitting two *separate* `Term::Fix`es, one per Lean definition. Each would
           then be checked by `check_termination` (termination.rs:49) against a `fix_positions`
           map containing only its own name, so the sibling call's head is not recognised as a
           recursive name at all — it falls through to the generic `App` arm
           (termination.rs:205-242) and is never checked for decrease. Two mutually recursive
           functions that loop forever would both pass. This is the exact soundness hole
           `check_termination_mutual` exists to close, and a naive translator walks straight
           into it because Lean's `Expr` presents the two definitions as two separate constants.
@guard     Catches emitting the block but referring to a sibling by `Term::Global` instead of
           `Term::Var`. The guard only recognises a recursive call when the application head is
           `Term::Var(name)` with `name ∈ fix_positions` (termination.rs:216-220); a `Global`
           head is a plain constant, so the call is unguarded — and it will also fail to
           resolve, because the sibling is not in the context yet.
@guard     Catches getting `index` wrong. `MutualFix { defs, index }` returns `types[index]`
           (type_checker.rs:371); with `defs` in a different order than the translator assumed,
           `isEven` gets `isOdd`'s type. Both here are `Nat → Bool`, so the kernel accepts it
           and every use site is silently the other function.
@guard     Catches the differing-structural-parameter trap: `check_termination_mutual`
           recomputes the structural parameter per member (termination.rs:88-94) but checks
           each body against the CURRENT member's parameter, so a block where one member
           recurses on argument 0 and another on argument 1 is fine — but a translator that
           normalises all members to "position 0" (a tempting simplification) makes the check
           inspect the wrong argument of the sibling call.
@oracle    The emitted `MutualFix` typechecks (so `check_termination_mutual` passed), its
           `defs` has exactly the Lean block's members in a fixed, recorded order, and
           `normalize` of member `index` applied to `Lit(Nat 4)` matches Lean's `isEven 4`.
-/

namespace T04_003

-- The canonical mutual block: neither member is structurally recursive on its own.
mutual
  def isEven : Nat → Bool
    | 0 => true
    | n + 1 => isOdd n

  def isOdd : Nat → Bool
    | 0 => false
    | n + 1 => isEven n
end

-- A mutual block over mutually recursive *inductives*, where the two members recurse on
-- different types — so the block's members genuinely have different structural types and the
-- guard must track one `struct_type` per member.
mutual
  inductive Tree where
    | node : Forest → Tree

  inductive Forest where
    | nil  : Forest
    | cons : Tree → Forest → Forest
end

mutual
  def treeSize : Tree → Nat
    | .node f => forestSize f + 1

  def forestSize : Forest → Nat
    | .nil => 0
    | .cons t f => treeSize t + forestSize f
end

-- A block whose members decrease on their second argument, behind a first accumulator
-- binder — the argument POSITION the guard records per member is 1, not 0.
mutual
  def evenFrom (acc : Nat) : Nat → Nat
    | 0 => acc
    | n + 1 => oddFrom (acc + 1) n

  def oddFrom (acc : Nat) : Nat → Nat
    | 0 => acc
    | n + 1 => evenFrom (acc + 2) n
end

example : isEven 4 = true := rfl
example : isOdd 4 = false := rfl
example : treeSize (.node (.cons (.node .nil) .nil)) = 2 := by
  simp [treeSize, forestSize]
example : evenFrom 0 3 = 4 := rfl

end T04_003
