/-!
@case      T03-020
@tier      3 · Inductives and structures
@title     Zero-constructor inductive (`False`, `Empty`) — a `Match` with an empty `cases`
           vector
@lean      inductive with no constructors, `False`, `Empty`, `False.elim`, `Empty.elim`,
           `nomatch`
@target    Context::add_inductive(name, sort) with **no** `add_constructor` call at all; every
           elimination is `Term::Match { discriminant, motive, cases: vec![] }`
@expect    translate
@evidence  kernel/src/prelude.rs:730 (`ctx.add_inductive("False", Term::Sort(Universe::Prop))`,
           followed by no constructor registration — the prelude's `False` is exactly this
           shape); kernel/src/context.rs:498-515 (`get_constructors` returns an empty vector
           for an inductive with no `constructor_order` entry);
           kernel/src/type_checker.rs:253-266 (the coverage rule is `cases.len() ==
           constructors.len()`, so zero cases is the *correct* and only legal shape — not a
           degenerate one); kernel/src/type_checker.rs:1688-1691 (`is_subsingleton_prop`
           returns `true` for zero constructors, which is what licenses `False.elim` into any
           sort); kernel/src/type_checker.rs:277-290 (the large-elimination gate, which
           therefore never blocks an empty `Prop`)
@guard     Catches emitting a dummy constructor "so the inductive is well-formed". Adding even
           one constructor makes the type **inhabited**: `False` gains a proof, and every
           downstream theorem becomes vacuously derivable. Nothing in LOGOS objects —
           `add_constructor` is unchecked (context.rs:419-428) and an inhabited `Prop` is a
           perfectly ordinary registration. This is the most direct route from a translator bug
           to an inconsistent context.
@guard     Catches emitting `False.elim` as an **axiom**. It is not an axiom in LOGOS: it is a
           `Match` with zero cases, which typechecks because the coverage count is zero and the
           subsingleton rule permits the large elimination. Axiomatising it adds a trusted-base
           entry for something the kernel proves, and the added entry has the form
           `Π(α). False → α`, which is indistinguishable from a genuinely dangerous axiom in
           the `declarations` audit (context.rs:517-522).
@guard     Catches emitting a `Match` with one "absurd" case. The count rule is exact
           (type_checker.rs:256-260): one case against zero constructors is
           `WrongNumberOfCases`. A translator that habitually synthesises a filler branch
           (T03-019) must special-case the empty inductive.
@guard     Catches conflating Lean's `False` (a `Prop`) with Lean's `Empty` (a `Type 0`). They
           differ in sort, in the gate that applies, and in what proof irrelevance says about
           them. LOGOS's prelude registers `False` at `Prop` (prelude.rs:730) and has **no**
           `Empty` — so `Empty` must be emitted, at `Type 0`, under a name that does not
           collide (T02-020).
@guard     Catches registering an empty inductive that Lean gave *indices*. `inductive Bad :
           Nat → Prop` with no constructors still has arity 1, so `inductive_arity` is 1 and
           `add_inductive`'s default makes `num_params` 1 and `num_indices` 0 — the T03-005
           collapse, on a type where it is easy to overlook because there are no constructors
           to disagree.
@oracle    `get_constructors(name)` is empty; `is_subsingleton_prop(name)` is `true`; a `Match`
           on the type with `cases: vec![]` typechecks at an arbitrary motive; and no
           `add_constructor` call was made for the name.
-/

namespace T03_020

/-- `False` is the prelude's zero-constructor `Prop`. Elimination into any sort is licensed by
    the subsingleton rule. -/
def elimToProp (h : False) : True := False.elim h
def elimToType (h : False) : Nat := False.elim h
def elimToSort (h : False) : Type := False.elim h

/-- Lean's `nomatch` is the surface for "zero branches". Its elaborated term is a `casesOn`
    with no minor premises, which is exactly `Match { cases: [] }`. -/
def viaNomatch (h : False) : Nat := nomatch h

/-- A **user-defined** zero-constructor `Prop`. It has no prelude counterpart, so it must be
    emitted — with no constructor call. -/
inductive Void : Prop where

noncomputable def voidElim (α : Type) (v : Void) : α := nomatch v

/-- A **user-defined** zero-constructor `Type 0` — Lean's `Empty` shape, which LOGOS's prelude
    does not provide. Different sort, different gate. -/
inductive Nothing : Type where

def nothingElim (α : Type) (n : Nothing) : α := nomatch n

/-- The two are genuinely different: this function cannot be given the `Void` type, because
    `Void : Prop` and `Nothing : Type`. -/
def nothingToNat (n : Nothing) : Nat := nomatch n

/-- An empty inductive with an **index**. Arity 1, zero constructors: the params/indices split
    is `num_params = 0`, `num_indices = 1`, which `add_inductive`'s default gets wrong. -/
inductive NeverAt : Nat → Prop where

noncomputable def neverElim {n : Nat} (h : NeverAt n) : Nat := nomatch h

/-- An empty inductive with a **parameter**, for the opposite split: `num_params = 1`,
    `num_indices = 0`. -/
inductive VoidOf (α : Type) : Prop where

noncomputable def voidOfElim {α : Type} (h : VoidOf α) : α := nomatch h

/-- Negation is an arrow into the empty type, so every `¬P` proof is a function whose body is
    ultimately a zero-case `Match`. -/
example : ¬ (0 = 1) := by decide

def notBoth {P : Prop} (h : P) (hn : ¬ P) : Nat := False.elim (hn h)

/-- Emptiness is *not* observable by the kernel from the declaration alone: `Nothing` and a
    one-constructor inductive differ only by whether `add_constructor` was ever called. This
    is why the dummy-constructor guard matters. -/
inductive NotNothing : Type where
  | oops : NotNothing

example : NotNothing := NotNothing.oops

end T03_020
