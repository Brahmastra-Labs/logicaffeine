/-!
@case      T03-019
@tier      3 · Inductives and structures
@title     Impossible case in an indexed match — LOGOS has **no index unification**
@lean      dependent pattern matching on an indexed family, automatic discharge of
           index-contradictory constructors, `Vect α 0` / `Fin 0` / `Even 1`
@target    Term::Match whose `cases` vector has **one entry per constructor**, including an
           explicitly synthesised inhabitant for every constructor Lean ruled out; plus a loss
           record
@expect    degrade
@evidence  kernel/src/type_checker.rs:602-611 (the doc for `infer_indexed_match`: the return
           type is the motive applied to the discriminant's own index arguments, "each
           constructor's case is checked against `motive` applied to THAT constructor's result
           indices");
           kernel/src/type_checker.rs:631-638 (**the whole coverage rule**: `if cases.len() !=
           constructors.len() { WrongNumberOfCases }` — a bare count. There is no unifier, no
           `K`, no `noConfusion`-driven discharge, and no way to mark a branch unreachable);
           kernel/src/type_checker.rs:685-714 (`compute_indexed_case_type` builds each branch's
           expected type from the constructor's *declared* result indices — so an impossible
           branch's expected type is a perfectly well-formed type that simply has no natural
           inhabitant);
           kernel/src/type_checker.rs:253-266 (the non-indexed path has the identical count
           rule — no wildcards anywhere);
           kernel/src/term.rs:443-545 (`Match { discriminant, motive, cases }` — `cases` is a
           positional `Vec<Term>`; there is no pattern language and no absurd-branch marker)
@loss      IMPOSSIBLE_CASE_MUST_BE_INHABITED. Lean discharges an index-contradictory branch by
           unification and emits nothing for it. LOGOS demands a term. The translator must
           synthesise one, and whatever it synthesises is *not* what the Lean source said.
@loss      DISCHARGE_JUSTIFICATION. The reason the branch is unreachable — that `Succ n` cannot
           equal `Zero` — is exactly constructor disjointness, which LOGOS does not generate
           (T03-018). So the honest filler, "derive `False` from the index mismatch and use
           `False.elim`", is generally unavailable; the translator falls back to an *arbitrary*
           inhabitant of the branch type.
@loss      TOTALITY_ILLUSION. Once an arbitrary inhabitant is supplied, the translated function
           is total in a way the Lean source was not: `vhead` applied to a `Vect α 0` reduces
           to the filler instead of being ill-typed. Any downstream reasoning that relied on
           "this case cannot occur" no longer has a kernel-enforced basis.
@guard     Catches omitting the impossible branch. `cases.len() != constructors.len()` is a hard
           `WrongNumberOfCases` error (type_checker.rs:631-638), so this fails loudly — which
           is the *good* outcome. The bad outcome is the translator noticing the error and
           patching it by the next three guards.
@guard     Catches filling the branch with `Term::Hole`. `types_equal` treats `Hole` as matching
           **anything** (type_checker.rs:1594-1597) and that leniency leaks into `is_subtype`'s
           fast path (`:1314`), so a `Hole` branch typechecks against every expected case type.
           The `Match` then carries an unchecked subterm into the context, and every later
           conversion involving it succeeds vacuously. This is the single most dangerous filler.
@guard     Catches filling the branch with an axiom (`add_declaration("absurd_case", …)`) or
           with `sorryAx`. Both make the branch typecheck and both extend the trusted base with
           a statement the translator did not prove; LOGOS has no axiom audit (T02-003).
@guard     Catches filling the branch with a *plausible but wrong* value of the right type —
           `Zero` for a `Nat`-valued branch. It typechecks, it reduces, and if the
           "impossible" case turns out to be reachable after a *different* mistranslation
           (notably the T03-005 params/indices collapse, which makes `Vect.cons` inhabit
           `Vect α 0`), the function silently returns the filler instead of failing. The two
           bugs compose into a function that answers wrongly with no diagnostic anywhere.
@guard     Catches assuming the motive can dodge the issue. The motive is applied to the
           **constructor's** declared result indices for each branch
           (type_checker.rs:685-714), so a motive that returns `True` on the impossible index
           changes the branch type — but it also changes the *result* type, which is the motive
           applied to the discriminant's indices. There is no motive that makes one branch
           trivial and leaves the others alone.
@oracle    The emitted `Match` has exactly `constructors.len()` cases in registration order;
           no case is `Term::Hole`; no case mentions a declaration introduced by
           `add_declaration` for this purpose; the loss report names every synthesised branch
           with IMPOSSIBLE_CASE_MUST_BE_INHABITED and the filler term used; and the `Match`
           typechecks.
-/

namespace T03_019

inductive Vect (α : Type) : Nat → Type where
  | nil : Vect α 0
  | cons : {n : Nat} → α → Vect α n → Vect α (n + 1)

/-- **One impossible branch.** The scrutinee's index is `n + 1`, so `Vect.nil` (index `0`)
    cannot occur. Lean emits nothing for it; LOGOS demands a term of the `nil` branch's type,
    which here is `α` — and there is no `α` to hand. -/
def vhead {α : Type} {n : Nat} : Vect α (n + 1) → α
  | Vect.cons a _ => a

def v3 : Vect Nat 3 := Vect.cons 1 (Vect.cons 2 (Vect.cons 3 Vect.nil))

example : vhead v3 = 1 := rfl

/-- **The other impossible branch.** Index `0` rules out `cons`. The filler needed here is a
    `Bool`, which *is* inhabited — so a careless translator will supply `true` or `false` and
    the mistake is invisible until the family is also collapsed (T03-005). -/
def isEmpty {α : Type} : Vect α 0 → Bool
  | Vect.nil => true

example : isEmpty (Vect.nil : Vect Nat 0) = true := rfl

/-- **A branch whose type is genuinely uninhabited.** The impossible `nil` case would have to
    produce a `Vect α n` for an arbitrary `α` with no value in scope — there is no filler at
    all short of `False.elim`, and the `False` needs constructor disjointness (T03-018). -/
def vtail {α : Type} {n : Nat} : Vect α (n + 1) → Vect α n
  | Vect.cons _ t => t

example : vhead (vtail v3) = 2 := rfl

/-- **Both branches reachable**, for contrast: when the index is a variable, nothing is ruled
    out and the translation is a plain two-case `Match`. This is the boundary. -/
def vcount {α : Type} : {n : Nat} → Vect α n → Nat
  | _, Vect.nil => 0
  | _, Vect.cons _ t => Nat.succ (vcount t)

example : vcount v3 = 3 := rfl

/-- A family over `Bool` rather than `Nat`, so the translator cannot special-case numeral
    index reasoning: index `true` rules out `plain`. -/
inductive Tagged (α : Type) : Bool → Type where
  | plain : α → Tagged α false
  | marked : α → Nat → Tagged α true

def markOf {α : Type} : Tagged α true → Nat
  | Tagged.marked _ k => k

example : markOf (Tagged.marked (1 : Nat) 42) = 42 := rfl

/-- **An indexed `Prop` family** where *every* constructor is ruled out. Lean discharges the
    whole match and the function is a proof of `False` from an impossible hypothesis; LOGOS
    would need two branches for a `Match` whose motive is `False`. -/
inductive Even : Nat → Prop where
  | zero : Even 0
  | plusTwo : {n : Nat} → Even n → Even (n + 2)

example : ¬ Even 1 := by
  intro h
  cases h

/-- Three constructors, two impossible — the count rule means all three branches must be
    supplied, and the two fillers differ in type. -/
inductive Three : Nat → Type where
  | atZero : Three 0
  | atOne : Nat → Three 1
  | atTwo : Bool → Three 2

def atOneValue : Three 1 → Nat
  | Three.atOne k => k

example : atOneValue (Three.atOne 9) = 9 := rfl

/-- Nested impossibility: the *inner* match's index is determined by the outer constructor, so
    the impossible branch count multiplies. LOGOS's `Match` is unary (T04-007), so this is two
    nested `Match`es, each owing a full set of branches. -/
def firstTwo {α : Type} {n : Nat} : Vect α (n + 2) → α × α
  | Vect.cons a (Vect.cons b _) => (a, b)

example : firstTwo v3 = (1, 2) := rfl

end T03_019
