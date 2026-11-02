/-!
@case      T04-009
@tier      4 · Recursion and pattern matching
@title     Overlapping patterns with first-match semantics — unrecoverable positionally
@lean      overlapping match equations, `match`'s top-to-bottom first-match rule
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/term.rs:487-492 (`Term::Match { discriminant, motive, cases }` — `cases`
           is indexed by CONSTRUCTOR, not by source equation: there is no pattern, no guard,
           and no notion of order among alternatives for one constructor);
           kernel/src/type_checker.rs:255-259 (`cases.len()` must equal the constructor count —
           so N overlapping equations cannot even be represented as N cases);
           kernel/src/type_checker.rs:262-265 (the i-th case is checked against the i-th
           constructor's case type, positionally);
           kernel/src/type_checker.rs:612-678 (`infer_indexed_match` — still exactly one case
           per constructor; there is no index unifier and no residual-pattern machinery)
@diag      E_OVERLAPPING_PATTERNS — must name the function, list the pair of equations that
           overlap and at least one value matched by both, and state that LOGOS matching is
           positional-by-constructor so first-match order has no carrier. Must NOT emit a
           `Match`.
@guard     This is the highest-value Tier-4 rejection. The tempting wrong move is **take the
           first equation for each constructor and drop the rest**, because that always
           produces a well-typed `Match` of exactly the right arity. For `pick` below that
           yields `pick 0 0 = 1`, which happens to be right, and `pick 0 5 = 1`, also right —
           but `pick 5 0` becomes `3` where Lean says `2`. The term typechecks, reduces, and
           is silently a different function. Nothing in the LOGOS kernel can detect this,
           because the kernel never saw the discarded equations.
@guard     Catches the mirror-image move: **take the LAST equation for each constructor**
           (natural if the translator folds the equation list left-to-right into a case
           vector). That is exactly last-match semantics, the opposite of Lean's, and again
           produces a well-typed term.
@guard     Catches "compile it myself": expanding the overlap into disjoint cases by hand.
           That is a correct *strategy* in principle — it is what Lean's equation compiler
           does — but doing it in the translator means reimplementing the equation compiler,
           including its splitting heuristics and its treatment of `Nat` literal patterns, and
           any divergence is silent. The corpus therefore requires rejection unless the
           translator works from the ALREADY-COMPILED `Expr` (Lean's `matcher` application),
           in which case the source has no overlapping patterns left and this case does not
           arise. A translator that reads surface `Syntax` is exactly the one that gets this
           wrong: **translate the post-elaboration `Expr`, never the surface `Syntax`.**
@guard     Catches ignoring overlap that is only visible through *literal* patterns. In
           `bucket` the equations `| 0 => ..`, `| 1 => ..`, `| _ => ..` overlap the wildcard;
           the `Nat` literal/Peano bridge (reduction.rs:329-346) means the LOGOS scrutinee may
           be a `Literal::Nat`, so a translator that "specialises on literals" still has to
           decide an order and has nowhere to record it.
@guard     Catches treating a *non-overlapping* multi-column match as overlapping (and thus
           over-rejecting). `T04-007`'s `eqBool` and `sub2` have disjoint equations and MUST
           translate. The diagnostic must be driven by an actual overlap witness, not by the
           mere presence of more equations than constructors.
@oracle    Translator exits non-zero with E_OVERLAPPING_PATTERNS and writes no declaration.
           The diagnostic must include a concrete witness value matched by two equations —
           e.g. `pick 0 0`, matched by both `| 0, _` and `| _, 0`.
-/

namespace T04_009

/-- The minimal overlap: `pick 0 0` matches equations 1 and 2; Lean takes the first. -/
def pick : Nat → Nat → Nat
  | 0, _ => 1
  | _, 0 => 2
  | _, _ => 3

/-- Overlap with a wildcard catch-all, plus literal patterns — three equations over an
    inductive with two constructors. -/
def bucket : Nat → Nat
  | 0 => 100
  | 1 => 200
  | _ => 300

/-- Overlap where the two overlapping equations return *different* values on the shared
    witness, and the function is recursive — so the wrong choice also changes the recursion. -/
def collapse : List Nat → List Nat → Nat
  | [], _ => 0
  | _, [] => 1
  | _ :: xs, _ :: ys => collapse xs ys + 1

/-- Overlap inside a nested pattern: `both (some 0)` matches equations 2 and 3. -/
def both : Option Nat → Nat
  | none => 0
  | some 0 => 7
  | some _ => 9

example : pick 0 0 = 1 := rfl
example : pick 5 0 = 2 := rfl
example : bucket 1 = 200 := rfl
example : collapse [] [] = 0 := rfl
example : collapse [1] [] = 1 := rfl
example : both (some 0) = 7 := rfl

end T04_009
