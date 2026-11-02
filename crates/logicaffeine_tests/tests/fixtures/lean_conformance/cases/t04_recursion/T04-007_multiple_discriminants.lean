/-!
@case      T04-007
@tier      4 · Recursion and pattern matching
@title     `match` on several discriminants — LOGOS `Match` is unary, so it must be nested
@lean      match e₁, e₂ with | p₁, p₂ => ..., multi-column equation compiler
@expect    translate
@target    Nested unary matches:
           Match { discriminant: a, motive: <λx:Bool. Bool → Bool>,
                   cases: [ Match { discriminant: b, motive: .., cases: [.., ..] },   -- a = false
                            Match { discriminant: b, motive: .., cases: [.., ..] } ] } -- a = true
@evidence  kernel/src/term.rs:487-492 (`Term::Match` has ONE `discriminant` — there is no
           multi-scrutinee form);
           kernel/src/type_checker.rs:172-200 (the `Match` rule infers a single discriminant's
           type and extracts a single inductive name);
           kernel/src/type_checker.rs:268-272 (the result type is `motive` applied to the
           discriminant and then BETA-REDUCED — "without beta reduction … causes type
           mismatches in NESTED matches", the kernel's own comment about exactly this case);
           kernel/src/termination.rs:262-280 (only a match whose discriminant IS the
           structural parameter marks variables smaller; an inner match on a different
           scrutinee does not)
@guard     Catches the outer motive being wrong. In a nested encoding the OUTER match's motive
           must have type `Bool → (Bool → C)`, i.e. it must still abstract the remaining
           scrutinees. A translator that computes the motive as though the match were the last
           thing in the term emits `λ_:Bool. C`, and then the outer case bodies — which are
           themselves `Bool → C` functions — fail to check. Where `C` is itself an arrow the
           two are confusable and the kernel may accept the wrong one.
@guard     Catches scrutinee-order inversion. Lean's `match a, b with` columns are ordered, and
           the equation compiler may internally reorder them; the LOGOS nesting order fixes
           which variable is bound where. For a function like `sub2` below, swapping the
           nesting silently computes `sub2 b a`. Both have type `Nat → Nat → Nat`, so nothing
           complains.
@guard     Catches the termination interaction. In `zipLen` the recursion decreases on the
           FIRST list, so the OUTER match must be on that list: `termination.rs:264-272` marks
           constructor binders smaller only when the discriminant is the structural parameter.
           Nesting the other way round leaves the recursive call's argument unmarked and the
           whole definition is rejected with a `TerminationViolation` — a correct Lean function
           turned into a build error purely by a choice of nesting order.
@guard     Catches "flatten the product": encoding `match a, b with` as a match on a pair.
           `Prod` is **absent** from the LOGOS prelude (LOGOS_FACTS, "Absent entirely"), so
           this requires declaring a product inductive, and then the structural guard sees the
           recursion running on the pair rather than on the list — which breaks termination for
           `zipLen`.
@oracle    The emitted term typechecks; `normalize` agrees with Lean on all four `Bool × Bool`
           inputs of `eqBool` and on `zipLen` for at least one asymmetric pair of lists; and
           the emitted outer `discriminant` is the translation of Lean's FIRST column.
-/

namespace T04_007

/-- Four equations over a 2×2 grid — the minimal genuinely two-dimensional match. -/
def eqBool : Bool → Bool → Bool
  | true, true => true
  | true, false => false
  | false, true => false
  | false, false => true

/-- Asymmetric in its two arguments, so a swapped nesting is observable. -/
def sub2 : Nat → Nat → Nat
  | 0, _ => 0
  | m + 1, 0 => m + 1
  | m + 1, n + 1 => sub2 m n

/-- Multi-discriminant match under recursion: the decreasing argument is the FIRST list, which
    pins the required nesting order. -/
def zipLen : List Nat → List Nat → Nat
  | [], _ => 0
  | _ :: _, [] => 0
  | _ :: xs, _ :: ys => zipLen xs ys + 1

/-- Three discriminants, and the third is only inspected in one branch — a nested encoding
    must still supply a full case vector for it in every branch that reaches it. -/
def firstTrue : Bool → Bool → Bool → Nat
  | true, _, _ => 0
  | false, true, _ => 1
  | false, false, true => 2
  | false, false, false => 3

example : eqBool true false = false := rfl
example : sub2 5 3 = 2 := rfl
example : sub2 3 5 = 0 := rfl
example : zipLen [1, 2, 3] [4, 5] = 2 := rfl
example : firstTrue false false true = 2 := rfl

end T04_007
