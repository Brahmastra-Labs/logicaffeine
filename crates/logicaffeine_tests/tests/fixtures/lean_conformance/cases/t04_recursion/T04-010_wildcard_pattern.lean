/-!
@case      T04-010
@tier      4 · Recursion and pattern matching
@title     Wildcard `_` pattern — must be expanded to every remaining constructor
@lean      `_` catch-all pattern, `| _ => e`, wildcard in one column of a multi-column match
@expect    degrade
@target    A `Term::Match` whose `cases` vector has exactly one entry per constructor, with the
           wildcard's body REPLICATED (as a λ-telescope of the right arity) into every
           constructor slot the wildcard covered.
@evidence  kernel/src/term.rs:487-492 (`Term::Match` has no wildcard, no default, no
           fall-through case);
           kernel/src/type_checker.rs:253-259 (`let constructors = ctx.get_constructors(..)` ;
           `cases.len() != constructors.len()` is `WrongNumberOfCases`);
           kernel/src/type_checker.rs:262-265 (positional zip against the constructor list);
           kernel/src/type_checker.rs:778-830 (`compute_case_type` — each case's expected type
           is the constructor's remaining Π-telescope ending in `motive (ctor args…)`, so each
           replicated body must be re-wrapped with THAT constructor's field binders)
@loss      DEFAULT_BRANCH_SHARING. Lean's `Expr` for a compiled wildcard shares one body
           (Lean's matcher lifts it into an auxiliary `match_i` constant or a `let`). The LOGOS
           `Term` is a TREE (LOGOS_FACTS: "LOGOS `Term` is a tree; sharing is lost"), so the
           body is duplicated k times. For a k-constructor inductive with a large default
           branch this multiplies term size by k, and since substitution is O(size) per binder
           comparison (type_checker.rs:1088) the cost is superlinear. Acceptable because the
           denotation is unchanged; must be recorded because it is how a translated file
           becomes unbuildable.
@loss      CATCH_ALL_STABILITY. The expansion is computed against the constructor list AT
           TRANSLATION TIME. If the Lean inductive later gains a constructor, Lean's `_`
           silently keeps covering it while the emitted LOGOS term does not — it simply has
           the wrong arity and stops typechecking. The loss record must pin the constructor
           list (names and order) the expansion was computed from.
@guard     Catches emitting one case for the enumerated patterns plus ONE case for the
           wildcard. For `isRed` below, `Color` has five constructors and the Lean source has
           two equations, so this gives `cases.len() == 2` against `expected: 5` — caught. But
           for `isNil`, `List` has exactly two constructors and the source has exactly two
           equations, one of which is a wildcard: the naive vector has the right LENGTH, and
           if the wildcard equation was written FIRST the two cases land in swapped
           constructor slots. The term typechecks (both branches are `Bool`) and `isNil [] `
           returns `false`. Silent.
@guard     Catches forgetting to re-abstract the wildcard body over each constructor's fields.
           `compute_case_type` (type_checker.rs:778-830) expects the `cons` case to be
           `λhd. λtl. body`, and the `nil` case to be a bare `body`. Copying the same bare term
           into both slots fails for `cons` — or, if the body happens to already be a function
           of the right type, typechecks and means something else.
@guard     Catches expanding against the *Lean* constructor order rather than the LOGOS
           registration order. `get_constructors` returns them in the order
           `add_constructor` was called (context.rs:419), which is the translator's own
           choice; the expansion and the registration must use one single recorded order.
@guard     Catches wildcards in a non-leading column of a multi-column match (`| _, true => ..`)
           being expanded only at the outer layer. Each nesting level needs its own full
           expansion.
@oracle    The emitted term typechecks; for each `Match` node `cases.len()` equals the
           constructor count of its own inductive; `normalize` agrees with Lean on EVERY
           constructor of each scrutinised inductive (not merely on the enumerated ones); and
           the loss record contains DEFAULT_BRANCH_SHARING with the pinned constructor list.
-/

namespace T04_010

inductive Color where
  | red | green | blue | cyan | magenta

/-- Two equations, five constructors: the wildcard must fan out to four cases. -/
def isRed : Color → Bool
  | .red => true
  | _ => false

/-- The dangerous shape: the wildcard comes FIRST and the inductive has exactly as many
    constructors as there are equations. -/
def notRed : Color → Bool
  | .red => false
  | _ => true

/-- Wildcard on `List`, where the equation count coincides with the constructor count. -/
def isNil : List Nat → Bool
  | [] => true
  | _ => false

/-- The wildcard body is large and mentions the scrutinee's *fields* nowhere — replicating it
    is what costs term size. -/
def rank : Color → Nat
  | .magenta => 0
  | _ => 1 + 2 + 3 + 4 + 5 + 6 + 7 + 8 + 9

/-- Wildcard in a non-leading column: each nesting level needs its own expansion. -/
def pickCol : Color → Bool → Nat
  | .red, _ => 0
  | _, true => 1
  | _, false => 2

/-- Wildcard under recursion: every replicated copy contains the recursive call, so the
    structural guard must see a smaller argument in each one. -/
def countReds : List Color → Nat
  | [] => 0
  | .red :: t => countReds t + 1
  | _ :: t => countReds t

example : isRed .green = false := rfl
example : notRed .red = false := rfl
example : isNil ([] : List Nat) = true := rfl
example : rank .blue = 45 := rfl
example : pickCol .green true = 1 := rfl
example : countReds [.red, .blue, .red] = 2 := rfl

end T04_010
