/-!
@case      T04-013
@tier      4 · Recursion and pattern matching
@title     Incomplete `match` — LOGOS requires `cases.len() == constructor count`, always
@lean      non-exhaustive match, `nomatch`, `Empty.elim`, matches Lean accepts with fewer
           branches than the type has constructors
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/type_checker.rs:253-259 (`let constructors = ctx.get_constructors(..)`;
           `if cases.len() != constructors.len() { WrongNumberOfCases { expected, found } }`);
           kernel/src/type_checker.rs:630-638 (`infer_indexed_match` repeats the same check and
           documents it: "Coverage: exactly one case per constructor, in registration order");
           kernel/src/term.rs:487-492 (`cases: Vec<Term>` has no optionality and no default);
           kernel/src/type_checker.rs:313 (`Term::Hole` reaching `infer_type` is
           `CannotInferHole`) and :1593-1597 (`types_equal` treats `Hole` as matching ANYTHING)
@diag      E_INCOMPLETE_MATCH — must name the inductive, list the constructors with no case,
           and state the required `cases.len()`. Must NOT emit a `Match` with a padded or
           placeholder case.
@note      Lean itself refuses a genuinely non-exhaustive `match`, so the offending form cannot
           appear as live code in a file that must compile. It is given below as a comment, and
           the file provides the closest COMPILING neighbours: (a) `nomatch` / `Empty.elim`,
           where Lean supplies *zero* branches because the type has no constructors — which
           LOGOS accepts, since `cases.len() == 0 == ctor count`; and (b) `Vect`'s
           index-refined match, where Lean supplies *fewer* branches than `Vect` has
           constructors because index unification rules one out — which LOGOS does **not**
           accept, because `infer_indexed_match` has no index unifier (type_checker.rs:612-678
           and LOGOS_FACTS). (b) is the live, compiling witness for this rejection. The
           overlap with T04-014 is deliberate and the two differ in obligation: T04-014 is
           `degrade` because the translator can SYNTHESISE the missing case (the indices make
           it unreachable, so any well-typed body will do); this case is `reject` for the
           situation where no such body can be built — see `headBad` below, whose missing
           `nil` case would have to inhabit `α` out of nothing.
@guard     The tempting wrong move is to **pad the case vector with `Term::Hole`** so the
           length check passes. That is the single worst outcome in this tier:
           `types_equal` matches `Hole` against anything (type_checker.rs:1593-1597) and the
           leniency leaks into `is_subtype`'s fast path (:1314), so the padded term can
           typecheck against a type it does not have, and the hole is now inside a declaration
           in the library.
@guard     Catches padding with a *fabricated* inhabitant instead — e.g. `Lit(Nat 0)` for a
           missing `Nat`-valued case. It typechecks, it reduces, and it is a different
           function: `headBad`'s `nil` branch would return `0` where the Lean source had no
           branch at all, so a theorem proved about the Lean function is false about the
           translation. Nothing in the kernel can notice.
@guard     Catches padding with an `add_declaration` axiom (context.rs:434) of the branch's
           type. `add_declaration` is unchecked and produces an opaque constant, so the branch
           type gets an inhabitant by fiat — for a `Prop`-valued branch that is an outright
           inconsistency.
@guard     Catches the inverse over-rejection: `nomatch`/`Empty.elim` (zero constructors) and
           `False.elim` must **translate**, because `cases.len() == 0` genuinely equals the
           constructor count. A translator that rejects every "fewer branches than expected"
           match loses `ex falso`, which every proof library needs.
@guard     Catches mis-counting constructors after flattening. `get_constructors` is keyed on
           the FLAT `String` name (context.rs:419, LOGOS_FACTS "no namespaces"), so two Lean
           inductives whose names collide after flattening share one constructor list and the
           arity check compares against the wrong count — passing an incomplete match or
           failing a complete one. See T02-020.
@oracle    Translator exits non-zero with E_INCOMPLETE_MATCH on `vheadRefined`/`headBad`;
           emits no declaration for them; and does NOT reject `nomatchEmpty`, `absurdElim` or
           `vnilOnly`, all of which have `cases.len() == ctor count`.
-/

namespace T04_013

/-
  THE OFFENDING FORM — Lean rejects it, so it cannot be live code:

    def headBad {α : Type} : List α → α
      | x :: _ => x
      -- error: missing cases: []

  There is no body the translator could synthesise for the `nil` case: `α` is an arbitrary
  type with no inhabitant in scope. Padding with a `Hole`, a literal, or an axiom are the
  three wrong answers this case exists to catch.
-/

/-- Zero constructors: Lean supplies zero branches and so does LOGOS. This must TRANSLATE. -/
def nomatchEmpty (e : Empty) : Nat := nomatch e

/-- `False.elim` — the `Prop` version. `False` is registered in the LOGOS prelude with zero
    constructors, so `cases.len() == 0` is correct. -/
def absurdElim {α : Type} (h : False) : α := False.elim h

inductive Vect (α : Type) : Nat → Type where
  | nil  : Vect α 0
  | cons : α → {n : Nat} → Vect α n → Vect α (n + 1)

/-- Lean supplies ONE branch for a two-constructor inductive, because the index `n + 1` rules
    `nil` out. `infer_indexed_match` has no index unifier, so LOGOS demands a `nil` case —
    which would have to inhabit `α`. This is the live witness for the rejection. -/
def vheadRefined {α : Type} {n : Nat} : Vect α (n + 1) → α
  | .cons a _ => a

/-- The mirror image, where the missing branch is `cons`: still one branch for two
    constructors. The synthesised `cons` case here WOULD be constructible (`Nat` is
    inhabited), which is what makes it a `degrade` under T04-014 rather than a reject. -/
def vnilOnly {α : Type} : Vect α 0 → Nat
  | .nil => 0

/-- A nested version: the index refinement happens two layers down. -/
def vsecond {α : Type} {n : Nat} : Vect α (n + 2) → α
  | .cons _ (.cons a _) => a

example : nomatchEmpty = nomatchEmpty := rfl
example : vheadRefined (Vect.cons 7 Vect.nil) = 7 := rfl
example : vnilOnly (α := Nat) .nil = 0 := rfl

end T04_013
