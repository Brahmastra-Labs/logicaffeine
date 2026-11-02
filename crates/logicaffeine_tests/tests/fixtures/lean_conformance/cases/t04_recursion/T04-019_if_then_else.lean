/-!
@case      T04-019
@tier      4 · Recursion and pattern matching
@title     `if then else` — `Bool` branching is a `Match`; `Prop` branching goes through
           `Decidable`
@lean      if c then a else b, `cond`, `ite`, Decidable instance resolution
@expect    translate
@target    For a `Bool` condition: `Match { discriminant: <cond>, motive: <λ_:Bool. C>,
           cases: [<false case>, <true case>] }` in `Bool`'s registration order.
           For a `Prop` condition: a `Match` on the resolved `Decidable p` instance,
           `cases: [<isTrue case, λhp. a>, <isFalse case, λhnp. b>]` — LOGOS has **no `ite`
           global**, so the branch must be emitted as a match on the instance itself.
@evidence  kernel/src/prelude.rs:777-794 (`Decidable : Π(p:Prop). Type 0` is registered by
           `add_indexed_inductive("Decidable", .., 1)` at :781, with `isTrue : Π(p:Prop). p →
           Decidable p` at :783 and `isFalse : Π(p:Prop). Not p → Decidable p` at :786);
           kernel/src/prelude.rs:791-794 (`Decidable_rec` is the auto-DERIVED eliminator,
           `add_definition`ed — not a primitive);
           kernel/src/prelude.rs:796-815 (`decide : Π(p:Prop). Decidable p → Bool`, defined as
           a `Decidable_rec` application — `isTrue ↝ true`, `isFalse ↝ false`);
           kernel/src/prelude.rs:923-1021 and :1023-1160 (`decEqBool` and `decEqNat` are the
           ONLY `Decidable` instances in the prelude);
           kernel/src/context.rs:184 (`add_instance` is a flat `Vec<(Term, Term)>`; resolution
           is first-match-wins);
           kernel/src/type_checker.rs:255-259 (positional, two-case coverage for `Bool`);
           there is **no `ite` and no `cond` global anywhere in prelude.rs** — the branch has
           no combinator to call
@guard     Catches emitting the cases in the wrong order. `Bool` is registered `true` then
           `false` (LOGOS_FACTS, prelude inductive list), and `Decidable` is registered
           `isTrue` then `isFalse` (prelude.rs:783,786). Lean's surface `if c then a else b`
           puts the "then" branch first, which HAPPENS to agree with both registration
           orders — so a translator that copies source order looks correct. It stops being
           correct for `if ¬c then …`, for `cond`'s `Bool`-first argument convention, and for
           any inductive whose LOGOS registration order the translator chose differently. Both
           branches have the same type, so the kernel accepts the swap and the function is
           silently negated.
@guard     Catches emitting `Global("ite")`. There is no `ite` in the LOGOS prelude;
           `get_global` (context.rs:449) returns `None` and the failure appears at the use
           site. The `Decidable` instance must be scrutinised directly, or `decide p inst`
           must be used to get a `Bool` and then matched.
@guard     Catches conflating the two branchings. `if (b : Bool) then …` matches on `b`;
           `if (p : Prop) [inst : Decidable p] then …` matches on `inst`. Emitting a match on
           `decide p inst` instead of on `inst` loses the proofs `hp : p` and `hnp : ¬p` that
           `isTrue`/`isFalse` carry — which is exactly what T04-020's `dite` needs. It also
           inserts an extra delta step through `decide`'s `Decidable_rec` body
           (prelude.rs:796-815), which counts against `normalize`'s 10000 fuel
           (reduction.rs:52-69).
@guard     Catches assuming Lean's `Decidable` instance translates. The prelude has exactly
           TWO instances, `decEqBool` and `decEqNat` (prelude.rs:923-1021, :1023-1160). A
           Lean `if x < y then …` on `Nat` needs `Nat.decLt`, which has no LOGOS counterpart;
           the translator must either emit a `Decidable` instance of its own or reject. And
           because `add_instance` is a flat `Vec` resolved first-match-wins (context.rs:184),
           registering a second instance for an already-covered type silently shadows or is
           silently shadowed, with no coherence check — see T07-008.
@guard     Catches losing the `Decidable` *argument*. `Decidable p` is an indexed inductive
           with ONE parameter (prelude.rs:781), so a `Match` on an instance takes the indexed
           path (type_checker.rs:193-201) and each case's type is computed by
           `compute_indexed_case_type` (:685-740) against the constructor's own result index.
           A case emitted as a bare term instead of `λh. body` fails, and a case emitted with
           the wrong arity may typecheck when `p` is itself a function type.
@oracle    The emitted term typechecks; `normalize` agrees with Lean on both truth values of
           each condition; the emitted `Match` on a `Bool` has its cases in LOGOS's `Bool`
           registration order; and the output contains no `Global("ite")` or `Global("cond")`.
-/

namespace T04_019

/-- A `Bool` condition — a two-case `Match` on `Bool`, nothing else. -/
def pickB (b : Bool) (x y : Nat) : Nat := if b then x else y

/-- The negated form, where source order and registration order disagree. -/
def pickNotB (b : Bool) (x y : Nat) : Nat := if !b then x else y

/-- `cond`, Lean's `Bool`-eliminating combinator — the same `Match` with the arguments in a
    different surface order. -/
def pickCond (b : Bool) (x y : Nat) : Nat := cond b x y

/-- A `Prop` condition resolved by a `DecidableEq Nat` instance. -/
def isZero (n : Nat) : Nat := if n = 0 then 1 else 0

/-- A `Prop` condition needing `Nat.decLt`, which the LOGOS prelude does NOT provide. -/
def clamp (n : Nat) : Nat := if n < 10 then n else 10

/-- Branching under recursion — each branch is a case of the same `Match`, and the recursive
    call sits in only one of them. -/
def countOdds : List Nat → Nat
  | [] => 0
  | x :: t => if x % 2 = 1 then countOdds t + 1 else countOdds t

/-- Nested `if`, which becomes nested `Match`es. -/
def sign3 (n : Nat) : Nat := if n = 0 then 0 else if n < 5 then 1 else 2

/-- A `Decidable` instance supplied explicitly, to pin that the scrutinee of the emitted
    `Match` is the INSTANCE and not the `Bool` that `decide` would produce. -/
def pickExplicit (p : Prop) [inst : Decidable p] (x y : Nat) : Nat :=
  @ite Nat p inst x y

example : pickB true 1 2 = 1 := rfl
example : pickNotB true 1 2 = 2 := rfl
example : pickCond false 1 2 = 2 := rfl
example : isZero 0 = 1 := rfl
example : clamp 20 = 10 := rfl
example : countOdds [1, 2, 3] = 2 := rfl
example : sign3 7 = 2 := rfl

end T04_019
