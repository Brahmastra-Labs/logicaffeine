/-!
@case      T03-021
@tier      3 · Inductives and structures
@title     Recursor primitivity inversion — Lean's `.rec` is primitive, LOGOS's `Match` is
@lean      `Foo.rec`, `Foo.casesOn`, `Foo.brecOn`, `Nat.rec` used directly in a term,
           `@[reducible] def Foo.casesOn := Foo.rec …`
@target    for a `.rec` occurrence: `recursor::derive_recursor(ctx, ind)` to synthesise
           `(type, term)`, then Context::add_definition to register it under a chosen flat
           name; every *use* then refers to that definition; plus a loss record
@expect    degrade
@evidence  kernel/src/recursor.rs:36 (`derive_recursor(ctx, ind) -> KernelResult<(Term, Term)>`
           — it *returns* a pair; nothing registers it, so a recursor exists only if the caller
           `add_definition`s it); kernel/src/term.rs:443-545 (`Term::Match` is a primitive
           constructor of the term language; there is no `Rec` variant);
           kernel/src/recursor.rs:150-167 (the derived recursor's body is
           `fix rec. λindices. λx. match x return … with { cases }` — a `Fix` wrapping a
           `Match`, i.e. built *out of* the primitives);
           kernel/src/recursor.rs:59-71 (**the motive target is hard-coded**: `Sort(Prop)` for a
           non-subsingleton `Prop` inductive, `Sort(Type 0)` for everything else);
           kernel/src/type_checker.rs:321-343 (the derived `Fix` is termination-checked like any
           other, so `derive_recursor`'s output must pass the Giménez guard);
           kernel/src/prelude.rs:1400ff, :1462-1470 (`Acc_rec`, `Eq_rec_dep`, `Eq_rec`,
           `Decidable_rec` — the prelude derives and registers these by hand, one at a time)
@loss      RECURSOR_IS_DERIVED. In Lean `Foo.rec` is a kernel primitive with its own iota rule;
           in LOGOS it is an ordinary delta-transparent definition wrapping a `Fix` and a
           `Match`. Reduction therefore takes more steps (delta, then fix-unfold, then iota)
           against the fixed 10000-step `normalize` fuel (reduction.rs:52-69), and the recursor
           can be unfolded in places Lean would keep it opaque.
@loss      MOTIVE_UNIVERSE_PINNED. The derived recursor's motive targets `Type 0` (or `Prop`).
           It is **not** universe-polymorphic — `Universe::Type` takes a concrete `u32`
           (term.rs:169-191) and an inductive cannot carry level parameters at all (T01-007).
           Every Lean use of `Foo.rec` at `Type 1` or at a level variable is therefore outside
           the derived recursor's type, and cumulativity (`Prop ≤ Type 0 ≤ Type 1`,
           type_checker.rs:1330-1360) only helps in the *downward* direction.
@loss      NO_casesOn_NO_brecOn. LOGOS derives one eliminator per inductive. Lean's
           `casesOn` (non-recursive), `brecOn`/`below` (course-of-values), `binductionOn` and
           the `.rec` auxiliaries are separate constants, none of which LOGOS generates. A term
           mentioning `brecOn` is a T04-016 rejection, not a degrade.
@guard     Catches assuming the recursor exists. `derive_recursor` is a *function call*, not a
           side effect of declaration (context.rs:253-255 registers a header and nothing else).
           A translated term containing `Global("Nat_rec")` where nobody called
           `add_definition` resolves to `None` in `get_global` (context.rs:449-464), and the
           failure is reported as an unbound name at whatever point the term is first checked.
@guard     Catches translating `Foo.rec` **as a `Match`** at each use site. That is the right
           *shape* for `casesOn` and wrong for `rec`: the recursor supplies induction
           hypotheses (the results of the recursive calls) to each minor premise, and a bare
           `Match` does not. Substituting one for the other changes the arity of every branch,
           and for a non-recursive inductive — where the two coincide — it will typecheck, so
           the bug only surfaces on the first recursive type.
@guard     Catches the inverse: translating every `match` in the source *through* the derived
           recursor. LOGOS's `Match` is the primitive; routing matches through a derived
           recursor means every pattern match carries a `Fix` and is subject to the termination
           guard, so a perfectly non-recursive match can be rejected for failing to descend.
@guard     Catches deriving the recursor **before** all of the inductive's constructors are
           registered. `derive_recursor` reads `get_constructors` (recursor.rs:126) at call
           time; deriving early yields a recursor with too few minor premises, and it is an
           ordinary definition afterwards — nothing re-derives it when the missing constructor
           arrives.
@guard     Catches naming collisions in the derived recursor. The translator chooses the flat
           name; the prelude already owns `Acc_rec`, `Eq_rec`, `Eq_rec_dep`, `Decidable_rec`
           (prelude.rs:1400ff, :1462-1470). Overwriting any of them silently replaces a
           kernel-checked derived term (T02-020).
@oracle    Every `Foo.rec` in the source has a corresponding LOGOS *definition* whose body is a
           `Fix` wrapping a `Match`, produced by `derive_recursor` after all of `Foo`'s
           constructors were registered; its inferred type matches `derive_recursor`'s reported
           type; no use of it requires a motive above `Type 0`; and the loss report contains
           RECURSOR_IS_DERIVED.
-/

namespace T03_021

inductive Colour where
  | red
  | green
  | blue

/-- `casesOn` on a non-recursive inductive. Here `rec` and a bare `Match` really do coincide,
    which is exactly why a translator that conflates them passes this and fails the next. -/
def code : Colour → Nat :=
  fun c => Colour.casesOn c 0 1 2

example : code Colour.green = 1 := rfl

/-- The same function written as a `match`. Both must translate to the same LOGOS `Match`. -/
def code' : Colour → Nat
  | Colour.red => 0
  | Colour.green => 1
  | Colour.blue => 2

example : code Colour.blue = code' Colour.blue := rfl

/-- **`Nat.rec` used directly.** The minor premise for `succ` takes both the predecessor *and*
    the recursive result — the induction hypothesis a plain `Match` does not supply. -/
def double : Nat → Nat :=
  fun n => Nat.rec 0 (fun _ ih => Nat.succ (Nat.succ ih)) n

example : double 3 = 6 := rfl

/-- The `Match`-only version, to make the arity difference concrete: this branch receives
    `k` but **not** `double k`, so the recursive call has to be written by hand — which is what
    the `Fix` inside the derived recursor exists to provide. -/
def doubleMatch : Nat → Nat
  | 0 => 0
  | Nat.succ k => Nat.succ (Nat.succ (doubleMatch k))

example : double 4 = doubleMatch 4 := rfl

/-- A **user** recursive inductive, so the recursor must be derived rather than taken from the
    prelude. -/
inductive Tree where
  | leaf : Nat → Tree
  | node : Tree → Tree → Tree

noncomputable def totalRec : Tree → Nat :=
  fun t => Tree.rec (motive := fun _ => Nat) (fun n => n) (fun _ _ l r => l + r) t

example : totalRec (Tree.node (Tree.leaf 2) (Tree.leaf 5)) = 7 := rfl

/-- `casesOn` on the *same* recursive type — one branch fewer argument each, and no induction
    hypothesis. Distinguishing `rec` from `casesOn` is the whole point. -/
def isLeaf : Tree → Bool :=
  fun t => Tree.casesOn t (fun _ => true) (fun _ _ => false)

example : isLeaf (Tree.leaf 1) = true := rfl
example : isLeaf (Tree.node (Tree.leaf 1) (Tree.leaf 2)) = false := rfl

/-- A **dependent** motive: the recursor's motive is a family, not a constant. The derived
    LOGOS recursor supports this, but only into `Type 0`. -/
def replicateTo : (n : Nat) → Fin (n + 1) :=
  fun n => Nat.rec (motive := fun m => Fin (m + 1)) ⟨0, by omega⟩ (fun _ ih => ⟨ih.val, by omega⟩) n

example : (replicateTo 3).val = 0 := rfl

/-- Elimination into `Prop` — the motive target the derived recursor uses for a
    non-subsingleton `Prop` inductive, and the direction cumulativity makes free. -/
theorem doubleEven : ∀ n : Nat, double n = 2 * n :=
  fun n => Nat.rec (motive := fun m => double m = 2 * m) rfl
    (fun k ih => by simp [double, Nat.mul_succ] at *; omega) n

/-- **Above `Type 0`.** A motive landing in `Type 1` has no derived recursor: the target is
    hard-coded (recursor.rs:59-71) and cumulativity does not go up. -/
def bigMotive : Nat → Type 1 :=
  fun n => Nat.rec (motive := fun _ => Type 1) (ULift Nat) (fun _ ih => ih) n

end T03_021
