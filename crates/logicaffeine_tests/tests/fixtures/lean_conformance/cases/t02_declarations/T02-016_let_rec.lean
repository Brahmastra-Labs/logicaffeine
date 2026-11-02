/-!
@case      T02-016
@tier      2 · Declarations
@title     `let rec` — a local fixpoint; `Term::Let` cannot carry the recursion
@lean      let rec (term and tactic position), Lean's `letRec` lifting to an auxiliary
@target    Term::Fix { name, body }, bound by Term::Let or lifted to its own
           Context::add_definition
@expect    translate
@evidence  kernel/src/term.rs:443-545 (`Let { name, ty, value, body }` has **no** recursive
           scope — `value` is checked in the ambient context, so `name` is not in scope inside
           it; the only recursive forms are `Fix` and `MutualFix`);
           kernel/src/type_checker.rs:321-343 (`Fix` typing runs
           `termination::check_termination` — "THE GUARDIAN" — then re-checks the body with the
           fixpoint name bound to its structurally-inferred type);
           kernel/src/type_checker.rs:349-377 (`MutualFix` — the *only* other recursive form,
           and the one a fused or split translation would wrongly reach for);
           kernel/src/termination.rs:48-62 (`check_termination` extracts the structural
           parameter from the body and requires every recursive call to descend)
@guard     Catches emitting `let rec f := body in e` as `Term::Let { name: "f", value: body, … }`.
           LOGOS's `Let` is **not** recursive: the occurrence of `f` inside `body` is a free
           `Var("f")` that does not resolve to the binding. Worse, if the surrounding term has
           *another* binder named `f`, the named representation lets the free occurrence be
           captured by it (`substitute_avoiding`, type_checker.rs:1088, only alpha-renames when
           the *replacement's* free variables collide). The result typechecks and computes
           something else entirely.
@guard     Catches assuming `let rec` is always liftable to a top-level definition. It is, but
           only after closing over the enclosing binders — exactly the capture obligation of
           T02-015. A lifted body that still mentions the parent's `Var`s is accepted by
           `add_definition` without any check (context.rs:442-444).
@guard     Catches flattening *nested* `let rec`s into one block. Two independent local
           fixpoints are two `Fix` terms, one inside the other's body; fusing them into a
           `MutualFix` changes what the termination guard requires, because
           `check_termination_mutual` demands that a call to **any** member descend
           (termination.rs:21-44) — a fusion can therefore be rejected where the honest nesting
           is accepted, and, in the other direction, a translator that splits a genuinely
           mutual pair into two `Fix`es escapes the descent requirement entirely (see T02-014).
@guard     Catches the zeta-transparency difference. `Term::Let` is transparent (it zeta-reduces
           during `normalize`), so a `let rec` lifted into a `Let`-bound `Fix` will be unfolded
           at every use; Lean's `let rec` is lifted to an auxiliary *definition* and stays
           folded. Both are sound; only one keeps term size linear under the 10000-step fuel
           (reduction.rs:52-69).
@note      Lean 4's `let rec` has no mutual (`and`) form — two sibling `let rec`s cannot see
           each other — so the `MutualFix` obligation for *local* recursion is exercised
           through `where` in T02-015 and through `mutual def` in T02-014. This file therefore
           tests the single-`Fix` path plus nesting.
@oracle    The emitted term contains one `Fix` per `let rec`, each of whose `infer_type`
           succeeds — meaning the structural guard passed — and whose body has no
           free variable escaping into the enclosing term; the result type matches the
           translation of Lean's inferred type.
-/

namespace T02_016

/-- The canonical `let rec`: a local structurally-recursive function. -/
def sumTo (n : Nat) : Nat :=
  let rec go : Nat → Nat → Nat
    | 0,          acc => acc
    | Nat.succ k, acc => go k (acc + Nat.succ k)
  go n 0

example : sumTo 4 = 10 := rfl
example : sumTo 0 = 0 := rfl

/-- A `let rec` that **captures** an enclosing binder (`bump`). Lifting it must add `bump` as
    a leading parameter; leaving it free produces a dangling `Var`. -/
def scaledSum (bump : Nat) (n : Nat) : Nat :=
  let rec go : Nat → Nat → Nat
    | 0,          acc => acc
    | Nat.succ k, acc => go k (acc + bump)
  go n 0

example : scaledSum 3 4 = 12 := rfl

/-- The capture hazard made concrete: an enclosing binder named `go`. If `let rec go` is
    emitted as a non-recursive `Let`, the free `go` inside the body is captured by *this*
    `go` and the function silently computes with the wrong callee. -/
def shadowedRec (go : Nat → Nat → Nat) (n : Nat) : Nat :=
  let rec loop : Nat → Nat → Nat
    | 0,          acc => acc
    | Nat.succ k, acc => loop k (acc + 1)
  loop n (go 0 0)

example : shadowedRec (fun _ _ => 7) 3 = 10 := rfl

/-- **Nested** `let rec`s: two independent local fixpoints, the inner one inside the outer
    one's body. These are two separate `Term::Fix`es, not one `MutualFix` — fusing them
    changes what the termination guard demands of each. -/
def nestedRec (n : Nat) : Nat :=
  let rec outer : Nat → Nat
    | 0 => 0
    | Nat.succ k =>
      let rec inner : Nat → Nat
        | 0 => 1
        | Nat.succ j => inner j + 1
      inner k + outer k
  outer n

example : nestedRec 0 = 0 := rfl
example : nestedRec 2 = 3 := rfl

/-- A *non*-recursive `let` for contrast: this one really is `Term::Let`, and it is
    zeta-transparent in LOGOS. -/
def plainLet (n : Nat) : Nat :=
  let twice := n + n
  twice + twice

example : plainLet 2 = 8 := rfl

end T02_016
