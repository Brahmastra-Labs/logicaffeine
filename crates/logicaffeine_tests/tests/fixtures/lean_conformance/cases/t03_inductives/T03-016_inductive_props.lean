/-!
@case      T03-016
@tier      3 · Inductives and structures
@title     Inductive propositions (`And`, `Or`, `Exists`) — map onto the prelude, do not
           re-declare
@lean      And, Or, Exists, Iff, True, False as inductive `Prop`s
@target    the **existing** prelude globals `And`/`conj`, `Or`/`left`/`right`, `Ex`/`witness`,
           `True`/`I`, `False` — no new registration; user-defined `Prop` inductives go through
           Context::add_inductive(name, Sort(Prop)) + Context::add_constructor_checked
@expect    translate
@evidence  kernel/src/prelude.rs:1815 (`add_inductive("And", and_type)` at `Prop`),
           :1858 (`Or`), :1925 (`Ex`), :723-724 (`True`/`I`), :730 (`False`);
           kernel/src/term.rs:169-176 (`Universe::Prop` is its own constructor — `Prop` is
           **not** `Type 0` in LOGOS's representation);
           kernel/src/type_checker.rs:1688-1714 (`is_subsingleton_prop`: zero constructors, or
           exactly one whose non-arity arguments all live in `Prop`);
           kernel/src/type_checker.rs:277-290 (the large-elimination gate — a `Prop` inductive
           may be eliminated into a larger sort only if it is a subsingleton);
           kernel/src/context.rs:545-560 (`add_constructor_checked` exempts `Prop` inductives
           from the universe constraint — "Impredicative Prop: no constraint on argument
           universes", type_checker.rs:556-561)
@guard     Catches re-declaring `And`/`Or`/`Ex` instead of mapping onto the prelude's. The names
           are already taken, `add_inductive` is a silent `HashMap::insert` (context.rs:253-255),
           and the prelude's *own* definitions reference them by name — `decide`,
           `Decidable_rec`, the `Int` order axioms (prelude.rs:329) and the `auto` tactic's
           certifier all mention `Or`, `And` and `False` as `Term::Global`s. Overwriting any of
           them re-points that machinery at the new declaration, and if the constructor order
           differs (`Or.inr` before `Or.inl`) every positional `Match` in the prelude silently
           takes the wrong branch. See T02-020.
@guard     Catches emitting a user `Prop` inductive at `Type 0` "because it has no
           computational content". LOGOS is cumulative (`Prop ≤ Type 0`,
           type_checker.rs:1330-1360) so almost everything still typechecks — but the
           large-elimination gate stops firing, proof irrelevance
           (`proof_irrelevant`, type_checker.rs:1547-1567) no longer applies, and a `Match` can
           now extract data from what the source called a proof.
@guard     Catches the constructor **order** of `Or`. LOGOS's prelude registers `left` then
           `right` (prelude.rs:1858ff); Lean's is `Or.inl` then `Or.inr`. Both orders are
           "natural", the two branches of a disjunction elimination have the same type whenever
           the motive is constant, and `Match` is positional
           (type_checker.rs:253-266) — so a mismatched mapping swaps every `Or` elimination in
           the development with no type error at all.
@guard     Catches translating `Exists` as a `Sigma`/`Subtype`. LOGOS has neither (T10-013),
           and the two are not interchangeable: `Ex` is a `Prop` and is **not** a subsingleton
           when its witness is data, so it cannot be large-eliminated. A translator that
           reaches for a `Type`-valued pair to model `∃` has changed the proposition into data.
@guard     Catches `Iff`. It is a *structure* in Lean (two fields), not a primitive, and LOGOS's
           prelude does not register it — so it must be emitted, and at `Prop`, which
           `add_structure` cannot reach (T03-012). The translator must build it as a plain
           `Prop` inductive with hand-written projections and accept the eta loss.
@oracle    Every occurrence of Lean's `And`, `Or`, `Exists`, `True`, `False` resolves to the
           corresponding prelude global with the prelude's constructor order; no prelude name is
           re-registered; every user `Prop` inductive is registered with `Sort(Prop)`; and
           `is_subsingleton_prop` agrees with Lean's `recursor.isK`/large-elimination verdict
           for each.
-/

namespace T03_016

/-- Conjunction: one constructor, both arguments in `Prop` — a subsingleton, so large
    elimination is permitted. -/
example (p q : Prop) (hp : p) (hq : q) : p ∧ q := And.intro hp hq
example (p q : Prop) (h : p ∧ q) : p := h.left
example (p q : Prop) (h : p ∧ q) : q ∧ p := And.intro h.right h.left

/-- Disjunction: two constructors, so **not** a subsingleton and **not** large-eliminable.
    The two branches are what a swapped constructor order silently exchanges. -/
example (p q : Prop) (hp : p) : p ∨ q := Or.inl hp
example (p q : Prop) (hq : q) : p ∨ q := Or.inr hq

/-- Elimination of `Or` **into `Prop`** — the only direction the gate permits. -/
def orSwap {p q : Prop} (h : p ∨ q) : q ∨ p :=
  match h with
  | Or.inl hp => Or.inr hp
  | Or.inr hq => Or.inl hq

/-- The **invisible** case: both branches have the same type, so the constructor order is pure
    convention and a mismatched `Or.inl`/`left` mapping is undetectable by any typing rule. -/
def orIdem {p : Prop} (h : p ∨ p) : p :=
  match h with
  | Or.inl hp => hp
  | Or.inr hp => hp

/-
The forbidden direction, for the record. Lean 4.14.0 rejects

    def orTag {p q : Prop} (h : p ∨ q) : Nat :=
      match h with
      | Or.inl _ => 0
      | Or.inr _ => 1

with

    error: tactic 'induction' failed, recursor 'Or.casesOn' can only eliminate into Prop

LOGOS applies the identical rule at `type_checker.rs:277-290`, gated on
`is_subsingleton_prop` (`:1688-1714`) — but only if `Or` was registered at `Sort(Prop)`.
Registering it at `Type 0` disables the gate and makes the above term *acceptable*, which is
the second guard of this case.
-/

/-- Existentials: `Ex` at `Prop` with a witness that may be data. Not a subsingleton when the
    witness is `Type`-valued, so it cannot be large-eliminated. -/
example : ∃ n : Nat, n = 3 := ⟨3, rfl⟩
example (h : ∃ n : Nat, n = 3) : True := h.elim (fun _ _ => True.intro)

/-- `True` (one constructor, no arguments) and `False` (zero constructors) are both
    subsingletons, so both large-eliminate. `False` large-eliminating is `absurd`/`False.elim`
    and is load-bearing. -/
example : True := True.intro
def exFalso (α : Type) (h : False) : α := False.elim h

/-- `Iff` is a Lean **structure**, not a primitive, and has no LOGOS prelude entry. -/
example (p : Prop) : p ↔ p := Iff.intro (fun h => h) (fun h => h)

/-- A **user-defined** `Prop` inductive with one propositional argument — subsingleton, so it
    may be large-eliminated. -/
inductive Wrapped (P : Prop) : Prop where
  | wrap : P → Wrapped P

def unwrapToType (P : Prop) (h : Wrapped P) : P :=
  match h with
  | Wrapped.wrap p => p

/-- A user-defined `Prop` inductive with **two** constructors — not a subsingleton, so
    `is_subsingleton_prop` returns `false` for it and eliminating into `Type` is refused by
    both systems. Only the `Prop`-valued elimination below is legal. -/
inductive Choice (P : Prop) (Q : Prop) : Prop where
  | takeLeft : P → Choice P Q
  | takeRight : Q → Choice P Q

def choiceSwap {P Q : Prop} (h : Choice P Q) : Choice Q P :=
  match h with
  | Choice.takeLeft p => Choice.takeRight p
  | Choice.takeRight q => Choice.takeLeft q

/-- A user-defined `Prop` inductive with **indices** — the shape LOGOS's prelude uses for
    `Eq` and `Acc` (prelude.rs:1379, :1439). `num_params = 0`, `num_indices = 1`. -/
inductive Positive : Nat → Prop where
  | one : Positive 1
  | more : {n : Nat} → Positive n → Positive (n + 1)

example : Positive 3 := Positive.more (Positive.more Positive.one)

/-- Impredicativity: a `Prop` inductive may quantify over all propositions, including itself.
    `check_constructor_universes` exempts `Prop` inductives entirely
    (type_checker.rs:556-561). -/
inductive AllProps : Prop where
  | mk : (∀ P : Prop, P → P) → AllProps

example : AllProps := AllProps.mk (fun _ h => h)

end T03_016
