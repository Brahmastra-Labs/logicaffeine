/-!
@case      T11-018
@tier      11 · Adversarial and soundness traps
@title     Recursion Lean accepts structurally that LOGOS's Giménez guard rejects — and the
           workarounds that make it accept the wrong thing
@lean      structural recursion through a helper, recursion on a projection, nested recursion,
           recursion under a `let`, course-of-values recursion, `Nat.rec` with an accumulator
@expect    reject
@target    (none for a recursion the guard refuses — LOGOS has no escape hatch)
@evidence  kernel/src/termination.rs:378-401 (`verify_structural_arg_smaller`: it peels the
           recursive call's structural argument to its application HEAD, and accepts ONLY
           `Term::Var(arg_name)` where `guard_ctx.smaller_than.contains(arg_name)`. Anything
           else — a literal, a global, a `Match`, an application whose head is not a tracked
           variable — is `TerminationViolation` with "Recursive call whose structural argument
           is not headed by a structurally-smaller variable");
           kernel/src/termination.rs:402-440 (the `smaller_than` set is populated by
           MATCH-BOUND binders of the scrutinee, and the extension noted in LOGOS_FACTS accepts
           `rec (h a…)` for such a variable — i.e. the smaller variable may be APPLIED, but it
           must still be the head); LOGOS_FACTS: there is NO well-founded recursion, no
           `termination_by`, no `decreasing_by`, and no equation compiler;
           kernel/src/prelude.rs:1354-1421 (`register_acc` — `Acc`/`Acc_intro` exist, so
           `Acc.rec` can be DERIVED, but recursion THROUGH it is not what the guard tracks:
           see T04-015); kernel/src/term.rs:443-545 (`Term::Fix { name, body }` and
           `MutualFix { defs, index }` are the only recursion forms)
@diag      E_TERMINATION_GUARD_STRICTER — must name the declaration, quote the guard's own
           reason string, and emit nothing. Must NOT offer to re-express the recursion in a way
           that passes the guard unless the re-expression is proved equivalent.
@guard     **TIER 11 THESIS.** The kernel accepting a term is not evidence of faithfulness —
           and this case is where a REJECTION tempts the translator into an unfaithful
           acceptance. The guard is a syntactic Giménez check
           (termination.rs:378-401); Lean's structural-recursion elaborator is strictly more
           capable, so there are Lean functions that are perfectly structural and that LOGOS
           refuses. The translator's options are then: reject (correct), or rewrite the
           recursion until the guard is satisfied (dangerous). The dangerous rewrites all share
           a shape — they change the function and the kernel cannot tell, because the REWRITTEN
           function typechecks fine and has the same type.
@guard     Catches the accumulator rewrite. Turning `f n = g (f (n-1))` into a
           tail-recursive accumulator version passes the guard and is a DIFFERENT function
           unless the fold is proved associative. Lean's own `List.foldl`/`foldr` distinction
           is exactly this. A translator that silently picks the one that passes has changed
           the semantics of every proof about it.
@guard     Catches the "just add a fuel parameter" rewrite. `f fuel n` with a `Nat` fuel
           argument always passes the guard (it recurses on `fuel`) and is TOTAL for a
           different reason than the original: it returns a default when fuel runs out. Every
           theorem about the original is now a theorem about a function that may return the
           default, and nothing in LOGOS records the difference.
@guard     Catches recursion on a PROJECTION. LOGOS has no `Term::Proj`; a projection is a
           `Match`-bodied global definition applied to the receiver (context.rs:271-369,
           T03-011). So `f s.tail` has an application head of `Global("S_tail")`, not a
           `Term::Var` — and `verify_structural_arg_smaller` (termination.rs:385-394) rejects
           it outright even though `s.tail` is structurally smaller. A translator that
           "fixes" this by inlining the projection's `Match` at the call site changes the
           argument's shape to a `Match`, whose head is also not a `Var`, and the guard rejects
           that too.
@guard     Catches recursion through a HELPER. `f (g x)` where `g` is smaller-preserving is
           structural in Lean's sense once `g` is unfolded; the guard looks at the SYNTACTIC
           head (termination.rs:381-384), sees `Global("g")`, and refuses. Delta-unfolding `g`
           at the call site to satisfy the guard makes the term larger, which pushes against
           the 10000-step fuel (T11-005) and the quadratic substitution cost (T11-007).
@guard     Catches reaching for `Acc`. `Acc`/`Acc_intro` are in the prelude
           (prelude.rs:1354-1421) and `derive_recursor` will produce an `Acc` eliminator, so it
           LOOKS as though well-founded recursion is available. It is not: the guard tracks
           `Fix`, and a recursive call inside an `Acc.rec` motive is still a recursive call the
           guard must accept. See T04-015.
@oracle    Translator exits non-zero with E_TERMINATION_GUARD_STRICTER for every declaration
           below and emits nothing. The decisive harness assertion is about what was NOT
           emitted: for every corpus declaration whose Lean recursion is structural but whose
           translation was rejected, the output must contain no `Term::Fix` at all — in
           particular no accumulator variant, no fuel parameter, and no inlined-projection
           variant of the same function.
-/

namespace T11_018

/-! ### Recursion through a PROJECTION. Structurally smaller in Lean; in LOGOS the argument's
     head is `Global("Stream_tl")`, not a `Term::Var`, so termination.rs:385-394 refuses. -/

structure NatList where
  hd : Nat
  tl : List Nat

def sumTail : List Nat → Nat
  | [] => 0
  | x :: xs => x + sumTail xs

/-- The projection form: `s.tl` is smaller, but only after unfolding the projection. -/
def sumViaProjection (s : NatList) : Nat := s.hd + sumTail s.tl

example : sumViaProjection ⟨1, [2, 3]⟩ = 6 := by simp [sumViaProjection, sumTail]

/-! ### Recursion through a HELPER whose result is smaller. Note that Lean's STRUCTURAL checker
     also declines here and falls back to well-founded recursion with an explicit
     `decreasing_by` — which is precisely the escape hatch LOGOS does not have (T04-005). The
     LOGOS guard sees a `Global` head in the recursive call's argument and refuses
     (termination.rs:381-384), and there is nowhere to supply a measure. -/

def shrink : List Nat → List Nat
  | [] => []
  | _ :: xs => xs

def viaHelper : List Nat → Nat
  | [] => 0
  | x :: xs => x + viaHelper (shrink (x :: xs))
  decreasing_by simp [shrink]; omega

example : viaHelper [1, 2] = 3 := by simp [viaHelper, shrink]

/-! ### Recursion under a `let`. The smaller variable is bound by `Term::Let`, not by a
     `Match`, so it is not in the guard's `smaller_than` set (termination.rs:402-440). -/

def underLet : List Nat → Nat
  | [] => 0
  | x :: xs =>
    let rest := xs
    x + underLet rest

example : underLet [1, 2, 3] = 6 := by simp [underLet]

/-! ### Course-of-values recursion: the recursive call is on a value two constructors down,
     reached through a nested match. Lean's elaborator handles it; the guard tracks the
     immediately match-bound variables. -/

def everyOther : List Nat → Nat
  | [] => 0
  | [x] => x
  | x :: _ :: rest => x + everyOther rest

example : everyOther [1, 2, 3, 4] = 4 := by simp [everyOther]

/-! ### The two DANGEROUS rewrites, written out so a harness can assert they are absent from
     the output. -/

/-- Accumulator version of `sumTail`. Passes any structural guard, and is a different function
    unless the fold is proved associative and commutative in the right order. -/
def sumAcc : Nat → List Nat → Nat
  | acc, [] => acc
  | acc, x :: xs => sumAcc (acc + x) xs

example : sumAcc 0 [1, 2, 3] = sumTail [1, 2, 3] := by simp [sumAcc, sumTail]

/-- Fuel version. Total for a DIFFERENT reason: it returns the default when fuel runs out, so
    every theorem about it carries an implicit "provided the fuel was enough". -/
def sumFuel : Nat → List Nat → Nat
  | 0, _ => 0
  | _ + 1, [] => 0
  | f + 1, x :: xs => x + sumFuel f xs

example : sumFuel 10 [1, 2, 3] = 6 := by simp [sumFuel]

/-- The observable difference: with insufficient fuel the answer is wrong, and the type is the
    same. This is what makes the rewrite silent. -/
example : sumFuel 1 [1, 2, 3] ≠ sumTail [1, 2, 3] := by simp [sumFuel, sumTail]

/-! ### The CONTROL: a recursion the guard does accept, so the rejection above is specific
     rather than a blanket refusal of recursion. -/

def countDown : Nat → Nat
  | 0 => 0
  | n + 1 => countDown n

example : countDown 5 = 0 := rfl

end T11_018
