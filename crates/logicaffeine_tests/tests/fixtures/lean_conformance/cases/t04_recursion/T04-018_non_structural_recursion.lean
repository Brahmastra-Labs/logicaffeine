/-!
@case      T04-018
@tier      4 · Recursion and pattern matching
@title     Recursion Lean accepts structurally that the Giménez guard rejects
@lean      structural recursion through nested patterns (`| n+2 => f n`), through a `let`
           alias of a smaller variable, and with the recursive function used as a value
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/termination.rs:262-280 (`check_guarded`'s `Match` arm: constructor
           binders are marked structurally smaller **only** when the discriminant is the
           structural parameter and `struct_param_live` holds; a match on any other scrutinee
           falls through to ":274 Not matching on structural param - just recurse normally",
           which does NOT extend `smaller_than`);
           kernel/src/termination.rs:378-395 (`verify_structural_arg_smaller`: the recursive
           call's structural argument must be headed by a variable in `smaller_than` — nothing
           else is accepted);
           kernel/src/termination.rs:316-327 (the `Let` arm: "The value is NOT registered as
           smaller (the conservative v1 guard — a `fix` whose recursive call passes a let-alias
           of a smaller variable is rejected)");
           kernel/src/termination.rs:329-347 (a bare occurrence of the recursive name is a
           `TerminationViolation`: "recursive name '{}' occurs as a first-class value, not
           applied to a structurally-smaller argument");
           kernel/src/termination.rs:182-196 (`enter_binder`: an inner binder that shadows the
           structural parameter sets `struct_param_live = false`, so a later match on that name
           stops guarding);
           kernel/src/type_checker.rs:336 (there is no admission path around the guard)
@diag      E_GIMENEZ_GUARD_STRICTER — must name the function, name the argument the recursion
           runs on, and state which of the guard's conditions fails (not marked smaller /
           let-aliased / first-class occurrence / shadowed structural parameter). Must NOT
           emit a `Fix`.
@guard     This is the case that makes "Lean accepted it, so it is structural" false.
           `halve` below is *textbook* structural recursion in Lean — `brecOn` gives it every
           subterm of `n` at once. Translated literally it becomes
           `Match n [c0, λm. Match m [c1, λk. … rec k …]]`, and the inner match is on `m`, not
           on the structural parameter `n`. Per termination.rs:265-272 the inner match does not
           extend `smaller_than`, so `k` is unmarked and the whole definition is rejected with
           a `TerminationViolation`. Every `| n+2 => …` function in a Lean library hits this.
@guard     Catches the "helpfully hoist it into a `let`" move — binding the smaller variable to
           a name for readability. The `Let` arm (termination.rs:316-327) deliberately does not
           propagate smallness, so `letAlias` below is rejected where the identical inline form
           would pass. A translator that introduces `let`s during lowering (for sharing, say —
           see T00-022) can turn a *passing* definition into a failing one.
@guard     Catches passing the recursive function as a value. Lean's nested recursion over a
           container — `| .node ts => 1 + (ts.map roseSize).foldl (·+·) 0`, shown commented
           below — hands the recursive name to `List.map`. The `Var` leaf at
           termination.rs:334-347 rejects a bare occurrence of a block member outright, so the
           literal translation is refused; and the tempting workaround, eta-expanding to
           `fun x => roseSize x`, does not help, because the eta-expanded occurrence is still a
           bare `Var` under a λ whose argument is not a smaller variable. The only LOGOS-legal
           shape is the mutual `Tree`/`Forest` pair of T04-003, which is a *re-expression*, not
           a translation, and must be reported as such.
@guard     Catches the *shadowing* failure, which is the silent one and is entirely
           self-inflicted. `enter_binder` (termination.rs:182-196) clears `struct_param_live`
           when an inner binder reuses the structural parameter's name, and
           termination.rs:265-272 then refuses to treat the guarding match as guarding.
           Because LOGOS `Var`s are NAMED (term.rs:450) while Lean's are de Bruijn, a
           translator that reuses Lean's binder-name *hints* can emit two nested binders called
           `n` from a Lean term that had no collision at all. The recursion is then rejected
           and the cause — a name collision the translator introduced — appears nowhere in the
           diagnostic. Binders must be freshened at emit time (see T00-004).
@guard     Catches "just wrap it with `termination_by`". There is no measure-based path
           (T04-004); the guard is the only admission rule.
@guard     Catches over-rejection: a translator that responds by rejecting ALL nested-pattern
           recursion loses T04-001's `double` and T04-002's `len`, which are single-level and
           must translate. The diagnostic must name the specific unmarked variable.
@oracle    Translator exits non-zero with E_GIMENEZ_GUARD_STRICTER for every definition below,
           naming the failing variable, and writes no declaration. It must NOT reject the
           single-level structural definitions of T04-001/T04-002.
-/

namespace T04_018

/-- Two constructors deep. Lean: structural via `brecOn`. LOGOS: the inner `match m` does not
    mark `k` smaller, so `halve k` is an unguarded call. -/
def halve : Nat → Nat
  | 0 => 0
  | 1 => 0
  | n + 2 => halve n + 1

/-- The same shape with two recursive calls at different depths — `fib n` is two constructors
    down and `fib (n+1)` is one. Even the *shallower* call is unreachable for the guard,
    because `n` is bound by the inner match. -/
def fib : Nat → Nat
  | 0 => 0
  | 1 => 1
  | n + 2 => fib n + fib (n + 1)

/-- The same failure over a list rather than `Nat`: `t` is two `cons` layers down. -/
def pairSum : List Nat → Nat
  | [] => 0
  | [x] => x
  | x :: y :: t => x + y + pairSum t

/-- A `let` alias of a genuinely smaller variable. Identical in Lean to writing `letAlias t`
    inline; rejected by the conservative `Let` arm of the guard. -/
def letAlias : List Nat → Nat
  | [] => 0
  | _ :: t =>
    let smaller := t
    letAlias smaller + 1

/-- The `Nat` version of the same let-alias trap, to show it is about the `Let` arm and not
    about lists. -/
def letAliasNat : Nat → Nat
  | 0 => 0
  | n + 1 =>
    let m := n
    letAliasNat m + 1

/-
  TWO MORE SHAPES, given as comments because LEAN ITSELF refuses them without a
  `termination_by` (which would make them T04-004 cases instead). They are the two remaining
  ways the Giménez guard is stricter, and a translator must recognise both.

  (a) NESTED RECURSION THROUGH A CONTAINER — the recursive name occurs as a first-class
      value, which `termination.rs:334-347` rejects outright:

        inductive Rose where
          | node : List Rose → Rose

        def roseSize : Rose → Nat
          | .node ts => 1 + (ts.map roseSize).foldl (· + ·) 0
          -- Lean 4.14: "failed to infer structural recursion: unexpected occurrence of
          --             recursive application roseSize"

      The only LOGOS-legal form is the mutual `Tree`/`Forest` re-expression of T04-003.

  (b) A BINDER THAT SHADOWS THE STRUCTURAL PARAMETER — `enter_binder`
      (termination.rs:182-196) clears `struct_param_live`, so the guarding match stops
      guarding:

        def shadowing : Nat → Nat → Nat
          | 0, _ => 0
          | n + 1, m =>
            let n := m
            shadowing n 0
          -- Lean 4.14: "Could not find a decreasing measure."

      The danger is not this source text — it is a *translator* that emits such a collision
      from a Lean term that had none, because Lean's binders are de Bruijn and LOGOS's are
      named.
-/

example : halve 5 = 2 := rfl
example : fib 6 = 8 := rfl
example : pairSum [1, 2, 3, 4, 5] = 15 := rfl
example : letAlias [1, 2, 3] = 3 := rfl
example : letAliasNat 4 = 4 := rfl

end T04_018
