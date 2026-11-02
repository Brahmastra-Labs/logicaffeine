/-!
@case      T02-015
@tier      2 · Declarations
@title     `where` clauses — auxiliary declarations that must be lifted, not inlined
@lean      def … where, Lean's `mkAuxDefinition` lifting of `where` bodies
@target    one Context::add_definition per lifted auxiliary (its own flat name, closed over the
           parent's binders as explicit leading parameters), plus the parent definition
           referring to it by `Term::Global`
@expect    translate
@evidence  kernel/src/context.rs:442-444 (`add_definition` is flat and unnamespaced: a lifted
           auxiliary needs its own top-level `String` key); kernel/src/term.rs:443-545
           (`Term::Let { name, ty, value, body }` exists, so a *non-recursive* `where` body may
           alternatively stay local — but `Let` has no `Fix`, so a recursive one cannot);
           kernel/src/type_checker.rs:321-343 (a recursive auxiliary must become `Term::Fix`,
           which is termination-checked); kernel/src/context.rs:449-464 (`get_global` is a flat
           lookup — a lifted `Foo.go` and a user's `Foo_go` are the same key, see T02-020)
@guard     Catches inlining the `where` body at each call site instead of lifting it. For a
           *recursive* `where` that is not merely wasteful, it is impossible: the body refers to
           itself, so inlining does not terminate. For a non-recursive one used k times it
           multiplies term size by k, and LOGOS's `normalize` has a hard fuel of 10000 steps
           that **silently returns a partially-reduced term** when exhausted
           (reduction.rs:52-69) — so the inlined version can fail a conversion check the lifted
           version passes, reported as an unrelated type mismatch.
@guard     Catches forgetting that Lean's `where` auxiliaries capture the parent's binders.
           Lean lifts `go` in `def f (n : Nat) … where go := … n …` to a top-level
           `f.go : Nat → …` with `n` as a *leading parameter*. A translator that emits `go`
           without that parameter leaves a dangling `Term::Var("n")` in the body. Because
           `add_definition` performs no typecheck (context.rs:442-444) the broken definition is
           accepted, and the dangling `Var` only fails when something eventually normalizes
           through it — at which point the error names `n`, not the `where`.
@guard     Catches the name of the lifted auxiliary. Lean names it `f.go`; flattened that is
           `f_go`, which collides with any user declaration `f_go`. The lifting scheme must go
           through the same injective mangling as every other name (T02-011).
@guard     Catches choosing `Term::Let` for a *recursive* `where`. `Let` is zeta-transparent and
           has no fixpoint; recursion must be `Fix`, and `Fix` must pass the structural guard.
@oracle    Each `where` auxiliary appears as its own LOGOS definition whose type is the
           parent's captured telescope followed by the auxiliary's own; the parent's body
           mentions it as a `Global` and contains no residual free `Var` from the parent's
           binders; and no auxiliary body is duplicated.
-/

namespace T02_015

/-- A non-recursive `where`, used twice. Inlining doubles the term. -/
def sumOfSquares (a b : Nat) : Nat := sq a + sq b
  where sq (n : Nat) : Nat := n * n

example : sumOfSquares 3 4 = 25 := rfl

/-- A `where` auxiliary that **captures** the parent's binder `step`. Lean lifts this to
    `countUp.go : Nat → Nat → Nat → Nat` with `step` as a leading parameter. -/
def countUp (step : Nat) (n : Nat) : Nat := go n 0
  where
    go : Nat → Nat → Nat
      | 0,          acc => acc
      | Nat.succ k, acc => go k (acc + step)

example : countUp 2 3 = 6 := rfl
example : countUp 5 0 = 0 := rfl

/-- Two `where` auxiliaries in one declaration, the second referring to the first. Lifting
    must preserve the dependency order. -/
def describe (n : Nat) : Nat := outer n
  where
    inner (k : Nat) : Nat := k + 1
    outer (k : Nat) : Nat := inner (inner k)

example : describe 0 = 2 := rfl

/-- A user declaration whose flat name is exactly what a naive lifting scheme produces for
    `sumOfSquares.sq`. Both key to `"sumOfSquares_sq"`. -/
def sumOfSquares_sq (n : Nat) : Nat := n + 100

example : sumOfSquares_sq 1 = 101 := rfl

/-- A recursive `where` whose auxiliary is *also* mutually referenced by a sibling
    auxiliary — the lifted pair is a `MutualFix`, not two `Fix`es. -/
def parity (n : Nat) : Bool := ev n
  where
    ev : Nat → Bool
      | 0 => true
      | Nat.succ k => od k
    od : Nat → Bool
      | 0 => false
      | Nat.succ k => ev k

example : parity 4 = true := rfl
example : parity 5 = false := rfl

end T02_015
