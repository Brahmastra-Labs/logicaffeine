/-!
@case      T05-019
@tier      5 · Elaboration surface
@title     `do` notation — the elaboration produces `Expr`s referencing globals LOGOS does not
           have
@lean      do, `let x ← e`, `let mut`, `for … in … do`, `return`, `if … then … else` in a do
           block, `try/catch`, `unless`
@expect    reject
@target    (none — no sound translation)
@evidence  There is **no `Monad`, no `Bind`, no `Pure`, no `Functor`, no `Applicative`, no
           `ForIn`, no `IO`, and no `Id`** anywhere in `crates/logicaffeine_kernel/src` — the
           prelude's complete inductive list is `Entity`, `Int`, `Float`, `Text`, `Duration`,
           `Date`, `Moment`, `Nat`/`Zero`/`Succ`, `EList`/`ENil`/`ECons`, `Bool`/`true`/`false`,
           `TList`, `True`/`I`, `False`, `Decidable`/`isTrue`/`isFalse`, `Eq`/`refl`,
           `Acc`/`Acc_intro`, `And`/`conj`, `Or`/`left`/`right`, `Ex`/`witness`, and the
           hardware types `Bit`, `Unit`, `BVec`, `Circuit` (LOGOS_FACTS, prelude.rs);
           kernel/src/context.rs:449 (`get_global` resolves inductives → constructors →
           definitions → declarations; a `Global("Bind_bind")` matches none of them);
           kernel/src/term.rs:443-545 (there is no sequencing, no effect, and no state form —
           `Let` is pure zeta-transparent substitution);
           kernel/src/context.rs:271-369 and :300-303 (`add_structure` hard-codes `Type 0`, so a
           `Monad` class — which is `(Type u → Type v)`-valued — cannot even be declared; see
           T03-012 and T07-011);
           kernel/src/termination.rs:1-14 and :378-400 (a `for` loop's `ForIn.forIn` is a
           higher-order fold over the recursive body — the recursive name occurs as a
           first-class value, which the Giménez guard rejects outright, :329-347)
@diag      E_NO_DO_NOTATION — must name the declaration, name the monad Lean elaborated the
           block at, and list the absent globals the elaborated `Expr` refers to
           (`bind`, `pure`, `ForIn.forIn`, …). Must NOT emit a `Global` for any of them and
           must NOT declare them as axioms.
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax` — and then
           reject anyway.** This is the case that shows the rule is necessary but not
           sufficient. A `Syntax`-level translator sees `do`/`←`/`for` and has to reimplement
           Lean's do-elaborator (which is a nontrivial CPS transformation with mutable-variable
           lifting and early-`return` handling). An `Expr`-level translator sees the RESULT of
           that transformation — and the result is a tower of applications of
           `Bind.bind`, `Pure.pure`, `ForIn.forIn` and the monad's instance projections, none
           of which exist in LOGOS. So reading `Expr` turns an impossible re-implementation
           into a clean, precise rejection, which is the whole value.
@guard     Catches "just declare the monad operations as axioms". `add_declaration`
           (context.rs:434) is unchecked and opaque. Axiomatising `bind` and `pure` gives
           constants that never reduce, so every `rfl` the source proved by running the do
           block stops holding; and axiomatising a `Monad` *dictionary* asserts the existence
           of a structure `add_structure` could not have built. For `Option`, whose `bind`
           genuinely is definable, the axiom is worse than useless: it shadows the definable
           version with an opaque one.
@guard     Catches "inline the `Option` monad, it's just a match". True for `optInc` below, and
           it stops being true the moment the block uses `let mut`, `for`, or early `return`:
           those elaborate to `ForIn.forIn` folds and `ExceptT`-style wrapping whose inlining
           is Lean's do-elaborator, not a local rewrite. A translator that inlines the easy
           cases and rejects the rest must record which path it took, because the two produce
           observably different terms.
@guard     Catches `for` loops specifically. `ForIn.forIn` applies the loop body — which, in a
           recursive definition, mentions the recursive name — as a first-class function
           argument. `termination.rs:329-347` rejects a bare occurrence of the recursive name
           outright, so even a hand-inlined `for` inside a `Fix` fails the Giménez guard. See
           T04-018.
@guard     Catches `let mut`. Lean lifts the mutable variable into the monad's state; LOGOS's
           `Term::Let` (term.rs:527-537) is zeta-transparent pure substitution with no state at
           all. Translating `let mut s := 0` as a `Let` silently makes every later assignment a
           fresh shadowing binding, so the loop computes with the initial value.
@guard     Catches early `return`. Inside a `do` block `return` is not the block's value — it
           is an early exit, elaborated through the monad. Treating it as "the last
           expression" drops every statement after it.
@guard     Catches over-rejection of ordinary `let`. A `do` block whose only statement is a
           pure `let x := e` still elaborates through `pure`; the rejection must be keyed on
           the elaborated `Expr` referring to absent globals, not on the presence of the `do`
           keyword — otherwise a translator cannot tell this case from T00-007.
@oracle    Translator exits non-zero with E_NO_DO_NOTATION for every definition below; writes
           no declaration for them; emits no `Global` named `bind`/`pure`/`forIn`/`Monad`/`Id`
           or any flattening of those Lean names; and adds no axiom.
-/

namespace T05_019

/-- The minimal `do`: one bind and one `pure`. Elaborates to
    `Bind.bind x (fun a => Pure.pure (a + 1))` at the `Option` monad. -/
def optInc (x : Option Nat) : Option Nat := do
  let a ← x
  return a + 1

example : optInc (some 1) = some 2 := rfl
example : optInc none = none := rfl

/-- Two binds and a pure computation in between. -/
def optAdd (x y : Option Nat) : Option Nat := do
  let a ← x
  let b ← y
  let s := a + b
  return s

example : optAdd (some 1) (some 2) = some 3 := rfl

/-- `let mut` plus `for` in the `Id` monad — `ForIn.forIn` over a `Std.Range`, with the
    mutable variable lifted into the monad's state. -/
def sumTo (n : Nat) : Nat := Id.run do
  let mut s := 0
  for i in [0:n] do
    s := s + i
  return s

example : sumTo 4 = 6 := rfl

/-- Early `return` inside a loop — not the block's last expression, an exit through the
    monad. -/
def firstBig (xs : List Nat) : Option Nat := Id.run do
  for x in xs do
    if x > 10 then
      return some x
  return none

example : firstBig [1, 20, 3] = some 20 := rfl

/-- A pure `do` block with no bind at all — it STILL goes through `pure`, which is why the
    rejection must be keyed on the elaborated `Expr` and not on the `do` keyword. -/
def pureDo : Option Nat := do
  let a := 1
  return a

example : pureDo = some 1 := rfl

/-- `unless` / `if` inside a do block, which desugar through the monad too. -/
def guarded (x : Option Nat) : Option Nat := do
  let a ← x
  if a = 0 then
    return 0
  return a

example : guarded (some 5) = some 5 := rfl

end T05_019
