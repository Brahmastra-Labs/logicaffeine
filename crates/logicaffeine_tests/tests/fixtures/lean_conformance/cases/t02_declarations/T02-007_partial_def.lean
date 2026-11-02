/-!
@case      T02-007
@tier      2 · Declarations
@title     `partial def` — no termination proof, and LOGOS has no opaque-recursion escape
@lean      partial def, the compiler-only implementation + `Inhabited`-backed opaque constant
@target    (none — no sound translation)
@expect    reject
@diag      E_PARTIAL_DEF_UNREPRESENTABLE — must name the declaration, state that its recursion
           is unjustified, and state that LOGOS's only recursion forms (`Term::Fix`,
           `Term::MutualFix`) are gated by the syntactic structural guard. Must NOT fall back
           to `add_declaration` (that turns the function into an axiom with no defining
           equations, silently) and must NOT fall back to `add_definition` with the recursive
           body (that is rejected by the guard, or worse, accepted by an unchecked path).
@evidence  kernel/src/type_checker.rs:321-343 (`Term::Fix` typing calls
           `crate::termination::check_termination` unconditionally — "THE GUARDIAN");
           kernel/src/type_checker.rs:349-377 (same for `MutualFix` via
           `check_termination_mutual`); kernel/src/termination.rs:1-14,21-44 (the guard is a
           purely syntactic Giménez/Coq structural check over a match-bound smaller variable —
           there is no measure, no well-founded relation, no `Acc`-driven escape);
           kernel/src/context.rs:434-436 (`add_declaration` gives a type with no body, so the
           defining equations are gone)
@guard     Catches the tempting fallback: emit `partial def loop : Nat → Nat` as
           `add_declaration("loop", Nat → Nat)`. It compiles, every call site typechecks, and
           the function is now an **uninterpreted constant** — `loop 0` is stuck, no equation
           holds, and any Lean theorem about `loop` that was proved by `unfold`/`simp` on its
           equations becomes either unprovable or, if the translator also emitted the equations
           as axioms, an assumed-consistent recursive equation. `partial`'s whole point in Lean
           is that its logical content is *exactly* "there is some inhabitant of this type";
           re-asserting the equations is unsound in general.
@guard     Catches the other fallback: emit the recursive body as a `Term::Fix`. The guard
           refuses it — which is the correct outcome — but a translator that then *retries*
           through an unchecked path has done real damage. Note that `add_definition` performs
           no typecheck at all (context.rs:442-444), so a `Fix` body can be parked in the
           context without ever passing `check_termination`; it only fails later, when someone
           happens to `infer_type` it. A translator must reject at emit time, not rely on the
           kernel to notice.
@guard     Catches translating `partial` as `Acc`-based well-founded recursion. LOGOS's
           prelude registers `Acc`/`Acc_intro` (prelude.rs:1379), but the termination guard is
           purely structural: recursion *through* an accessibility proof is not a structural
           descent and is refused. See T04-015.
@oracle    Translator exits non-zero with E_PARTIAL_DEF_UNREPRESENTABLE and writes **no**
           declaration — neither a definition nor a declaration — for the partial function.
-/

namespace T02_007

/-- The canonical non-terminating `partial def`. Lean accepts it because `Nat` is `Inhabited`
    and the logical content is discharged by that, not by a termination proof. -/
partial def spin (n : Nat) : Nat := spin (n + 1)

/-- A `partial def` that *does* terminate on every input a human would supply, but not for a
    reason any syntactic guard can see. This is the dangerous case: it looks translatable. -/
partial def collatzSteps (n : Nat) : Nat :=
  if n <= 1 then 0
  else if n % 2 == 0 then 1 + collatzSteps (n / 2)
  else 1 + collatzSteps (3 * n + 1)

-- Mutual `partial` recursion — the `MutualFix` path, same guard, same rejection.
mutual
  partial def isEvenP (n : Nat) : Bool := if n == 0 then true else isOddP (n - 1)
  partial def isOddP  (n : Nat) : Bool := if n == 0 then false else isEvenP (n - 1)
end

/-- For contrast: the *same function* written structurally. This one is translatable
    (see T04-001) — included so the case pins down that the rejection is about `partial`,
    not about the function's mathematical content. -/
def isEvenStruct : Nat → Bool
  | 0 => true
  | 1 => false
  | Nat.succ (Nat.succ k) => isEvenStruct k

end T02_007
