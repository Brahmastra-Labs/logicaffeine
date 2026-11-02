/-!
@case      T03-002
@tier      3 · Inductives and structures
@title     Recursive inductive (`Nat`-like) — and the prelude's hard-coded literal bridge
@lean      inductive with a recursive constructor argument, `Nat.zero`/`Nat.succ`, numeric
           literals via `OfNat`
@target    Context::add_inductive(name, Sort(Type 0)) plus
           Context::add_constructor_checked for each constructor, the recursive one typed
           `Pi { param_type: Global(name), body_type: Global(name) }`
@expect    translate
@evidence  kernel/src/context.rs:545-560 (`add_constructor_checked` runs
           `positivity::check_positivity`, which permits an occurrence of the inductive to the
           right of every arrow — positivity.rs:60-80); kernel/src/prelude.rs:533-541 (the
           prelude registers `Nat` with constructors `Zero` and `Succ`, **by those exact
           strings**); kernel/src/reduction.rs:330-345 (`extract_constructor` bridges
           `Literal::Nat(n)` to `Succ`/`Zero` by name, so a numeral is *definitionally*
           `Succ^n Zero`); kernel/src/type_checker.rs:1480-1493 (the same bridge inside
           `def_eq_normalized`); kernel/src/termination.rs:1-14 (the structural guard's notion
           of "smaller" is a match-bound argument of the scrutinee — the reason recursion on
           this shape is accepted)
@guard     Catches translating a Lean `Nat` — or any user inductive that flattens to the key
           `"Nat"`, `"Zero"` or `"Succ"` — as a fresh registration. Those three strings are
           load-bearing in the LOGOS kernel: the numeral/Peano bridge is keyed on them
           literally (reduction.rs:330-345). Re-registering `Nat` with different constructors,
           or in a different order, silently changes what every `Literal::Nat` in the context
           means, including inside the prelude's own `decEqNat` (prelude.rs:1027-1186). The
           translator must map Lean's `Nat` onto the *existing* prelude inductive and must
           refuse to overwrite it (see T02-020).
@guard     Catches the opposite error: mapping a user's own Peano-shaped inductive onto the
           prelude's `Nat` because it "looks the same". The user type has no literal bridge,
           no `decEqNat`, and is a genuinely different type in the Lean source; identifying
           them makes `MyNat`-typed and `Nat`-typed terms interchangeable, which they are not.
@guard     Catches getting the recursive constructor's type shape wrong. `Succ`'s LOGOS type
           must be `Pi { param: "_", param_type: Global("Nat"), body_type: Global("Nat") }`.
           Writing the domain as anything else — say a `Var` left over from a parameter
           telescope — is accepted by `add_constructor` (context.rs:419-428, unchecked) and by
           `add_constructor_checked` only if positivity happens to pass, and then breaks the
           termination guard, which identifies smaller variables by the *inductive type name*
           of the structural parameter (termination.rs:33-37).
@guard     Catches constructor order once more, but with consequences: `Match` is positional
           (type_checker.rs:253-266), and here the two branches have *different* shapes (one
           takes a predecessor argument, one does not), so a swapped order produces a case type
           mismatch — a loud failure rather than a silent one. That asymmetry is worth pinning:
           enums fail silently (T03-001), recursive types usually fail loudly.
@oracle    `get_constructors` matches Lean's order; the recursive constructor's registered type
           has the inductive itself as its domain; structural recursion over the type
           typechecks (so `check_termination` accepted it); and no prelude name was overwritten.
-/

namespace T03_002

/-- A Peano-shaped inductive that is **not** Lean's `Nat`. It has no literal bridge: `2` is
    not notation for `MyNat.succ (MyNat.succ MyNat.zero)` here. -/
inductive MyNat where
  | zero
  | succ : MyNat → MyNat

/-- Structural recursion: the guard's "smaller" is `k`, bound by matching the scrutinee. -/
def toNat : MyNat → Nat
  | MyNat.zero => 0
  | MyNat.succ k => Nat.succ (toNat k)

example : toNat (MyNat.succ (MyNat.succ MyNat.zero)) = 2 := rfl

/-- Recursion with an accumulator — the descent is still on the first argument. -/
def addMyNat : MyNat → MyNat → MyNat
  | MyNat.zero, m => m
  | MyNat.succ k, m => MyNat.succ (addMyNat k m)

example : toNat (addMyNat (MyNat.succ MyNat.zero) (MyNat.succ MyNat.zero)) = 2 := rfl

/-- Lean's own `Nat` in the same file, so the two are visibly distinct types. Identifying
    `MyNat` with the prelude's `Nat` would make this ill-typed pair well-typed. -/
def fromNat : Nat → MyNat
  | 0 => MyNat.zero
  | Nat.succ k => MyNat.succ (fromNat k)

example : toNat (fromNat 3) = 3 := rfl

/-- The literal bridge, exercised on Lean's `Nat`: this holds *definitionally* in LOGOS only
    because `extract_constructor` knows the strings `Nat`/`Zero`/`Succ`. -/
example : (2 : Nat) = Nat.succ (Nat.succ Nat.zero) := rfl

/-- A recursive inductive with two recursive positions and a non-recursive one, so the
    constructor telescope is not uniform. -/
inductive Tree where
  | leaf : Nat → Tree
  | node : Tree → Tree → Tree

def total : Tree → Nat
  | Tree.leaf n => n
  | Tree.node l r => total l + total r

example : total (Tree.node (Tree.leaf 2) (Tree.node (Tree.leaf 3) (Tree.leaf 4))) = 9 := rfl

/-- A recursive occurrence to the right of an arrow *inside* a field — still strictly
    positive, still accepted by `check_positivity` (positivity.rs:60-80). Contrast T03-015. -/
inductive Branching where
  | stop
  | fork : (Bool → Branching) → Branching

def depth : Branching → Nat
  | Branching.stop => 0
  | Branching.fork f => Nat.succ (depth (f true))

example : depth Branching.stop = 0 := rfl
example : depth (Branching.fork (fun _ => Branching.stop)) = 1 := rfl

end T03_002
