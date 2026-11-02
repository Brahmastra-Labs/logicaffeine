/-!
@case      T04-008
@tier      4 · Recursion and pattern matching
@title     Nested constructor patterns — LOGOS `Match` has **no patterns at all**
@lean      nested patterns (`(a, b) :: t`, `n + 2`, `.cons (.node f) t`), the equation compiler
@expect    degrade
@target    A TREE of unary, positional `Term::Match`es — one match per constructor layer the
           Lean pattern inspected, each with exactly one case per constructor of that layer's
           inductive, in registration order, and each case a λ-telescope binding that
           constructor's fields.
@evidence  kernel/src/term.rs:487-492 (`Term::Match { discriminant, motive, cases }` —
           `cases: Vec<Term>` carries NO pattern, NO constructor name, NO binder list);
           kernel/src/type_checker.rs:255-259 (`cases.len()` must equal the constructor count);
           kernel/src/type_checker.rs:262-265 (cases are zipped against
           `ctx.get_constructors(..)` positionally, so the i-th case IS the i-th constructor);
           kernel/src/type_checker.rs:268-272 (the beta-reduction of `motive disc` exists
           specifically so NESTED matches typecheck);
           kernel/src/termination.rs:403-440 (each case's constructor fields are its leading
           λ-binders, counted by `count_pi_params` on the constructor's type)
@loss      PATTERN_STRUCTURE. The source's pattern language — which columns were inspected, in
           what order, with what binder names, and which equations were written by the user
           versus synthesised by the compiler — is not recoverable from the emitted match
           tree. Concretely: `| 0 => .. | 1 => .. | n+2 => ..` and the two-level nested match
           that encodes it are the same LOGOS term, so error messages, `simp` lemma shapes and
           equation lemmas (T04-017) cannot be reconstructed. This is acceptable because the
           *denotation* is preserved: every input reduces to the same value.
@loss      ARITY_FLATTENING. A Lean pattern binds a variable at an arbitrary depth
           (`(a, b) :: t` binds `a` two layers down). In LOGOS the variable is bound by an
           inner case's λ, so its scope is narrower and its name may have to be freshened
           (`Term::Var` is NAMED — term.rs:450). The source binder names survive only as hints.
@guard     Catches emitting a single flat `Match` with one case per *equation*. For `classify`
           below Lean has three equations but `Nat` has two constructors, so
           `WrongNumberOfCases { expected: 2, found: 3 }` fires — visibly. The dangerous
           version is `sumPairs`, where the Lean equation count (2) coincidentally equals the
           `List` constructor count (2): the flat match then TYPECHECKS with the `(a,b) :: t`
           equation body installed as the `cons` case, and the pair is never destructured —
           except that the body refers to `a` and `b`, which are now free `Term::Var`s. If any
           enclosing binder happens to be named `a` or `b`, the named representation CAPTURES
           them and the term both typechecks and computes the wrong answer.
@guard     Catches dropping the inner layer's exhaustiveness. `| 0 => .. | 1 => .. | n+2 => ..`
           becomes `Match n [c0, λk. Match k [c1, λj. c2 j]]` — the INNER match needs two cases
           too. A translator that emits the inner match with only the `Succ` case fails
           `cases.len()`; one that emits the `Zero` case as a `Term::Hole` produces a term
           where `types_equal` matches the hole against ANYTHING (type_checker.rs:1594-1597),
           so the hole survives into the library.
@guard     Catches binder-count mismatch in a case. The `TCons`-style constructor's case must
           bind every Π binder of the constructor's type *including parameters*
           (termination.rs:410-415 peels `count_pi_params` λs). Binding only the value fields
           makes the guard peel into the body and mark a non-field variable as structurally
           smaller — a termination-soundness hole, not a type error.
@guard     Catches reusing one binder name across sibling cases. Lean's de Bruijn bodies make
           `| .cons a t` and `| .cons a t'` interchangeable; LOGOS's named `Var` means a case
           body that mentions `a` must be under a λ that actually binds `a` at that position.
@oracle    The emitted term typechecks; every `Match` node in it satisfies
           `cases.len() == ctor_count` for its own inductive; the emitted term contains no
           `Term::Hole`; and `normalize` agrees with Lean on every example below. The loss
           record must contain PATTERN_STRUCTURE.
-/

namespace T04_008

/-- Nesting into a *product* inside a list cell: two constructor layers in one pattern. -/
def sumPairs : List (Nat × Nat) → Nat
  | [] => 0
  | (a, b) :: t => a + b + sumPairs t

/-- Nesting two `Succ`s deep. Lean's three equations become a two-level match tree over an
    inductive with two constructors. -/
def classify : Nat → Nat
  | 0 => 10
  | 1 => 20
  | n + 2 => n

/-- Nesting across two *different* inductives. -/
inductive Shape where
  | circle : Nat → Shape
  | rect   : Nat → Nat → Shape

def describe : Option Shape → Nat
  | none => 0
  | some (.circle r) => r
  | some (.rect w h) => w * h

/-- Nesting under recursion: the decreasing argument `t` is bound at the OUTER layer, so the
    structural guard still sees it even though the pattern also destructures the head. -/
def firstComponents : List (Nat × Nat) → List Nat
  | [] => []
  | (a, _) :: t => a :: firstComponents t

/-- A binder introduced at depth 2 whose name collides with an enclosing binder — the named
    representation must freshen it. -/
def shadowed (a : Nat) : List (Nat × Nat) → Nat
  | [] => a
  | (a, b) :: _ => a + b

example : sumPairs [(1, 2), (3, 4)] = 10 := rfl
example : classify 5 = 3 := rfl
example : describe (some (.rect 3 4)) = 12 := rfl
example : firstComponents [(1, 2), (3, 4)] = [1, 3] := rfl
example : shadowed 100 [(1, 2)] = 3 := rfl

end T04_008
