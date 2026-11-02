/-!
@case      T03-017
@tier      3 · Inductives and structures
@title     The large-elimination gate — `Acc`, `Eq`, `False`, `And` may cross the `Prop`/`Type`
           line; `Or` may not
@lean      `Prop` inductives eliminated into `Type`, Lean's `InductiveVal.isRec`/`recursor`
           large-elimination criterion, `Acc.rec`, `Eq.rec`, `False.elim`, subsingleton
           elimination
@target    Context::add_inductive(name, Sort(Prop)) + Context::add_constructor_checked, with
           the *subsingleton* property preserved exactly, so that
           `type_checker::is_subsingleton_prop` returns the same verdict LOGOS needs
@expect    translate
@evidence  kernel/src/type_checker.rs:1683-1714 (`is_subsingleton_prop`: zero constructors ⇒
           true; exactly one constructor ⇒ true iff every argument **beyond the inductive's
           arity** has sort `Prop`; two or more ⇒ false);
           kernel/src/type_checker.rs:271-290 (the non-indexed gate: if the discriminant's type
           is a `Prop` and the return type is not, refuse unless the inductive is a
           subsingleton — "large-eliminating `Or` would let a *proof* pick a `Type`-level
           value"); kernel/src/type_checker.rs:667-677 (the identical gate on the indexed
           path); kernel/src/prelude.rs:1379 (`Acc` at `Prop`, `num_params = 2`),
           :1400ff (`Acc_rec` is the **auto-derived** dependent eliminator, not an axiom);
           kernel/src/prelude.rs:1439-1460 (`Eq` at `Prop`, `num_params = 2`, one index;
           `Eq_rec_dep` is a kernel-checked derived definition — LOGOS *derives* J rather than
           axiomatising it); kernel/src/recursor.rs:59-71 (`derive_recursor`'s motive target is
           `Sort(Prop)` for a non-subsingleton `Prop` inductive and `Sort(Type 0)` otherwise —
           so the derived recursor is itself gated)
@guard     Catches getting `is_subsingleton_prop`'s *arity* offset wrong. The criterion skips
           the inductive's arity (parameters **and** indices, `pi_param_count` of the
           inductive's own type, type_checker.rs:1699-1702) and inspects only the constructor's
           remaining arguments. A translator that registers an indexed `Prop` family with the
           wrong `num_params`/arity — the T03-005 bug — shifts that offset, so a constructor
           argument that should have been skipped is tested for `Prop`-ness, or vice versa.
           The result is a subsingleton verdict that disagrees with Lean's, in either
           direction: either `Eq` stops large-eliminating (and `Eq.rec` becomes underivable,
           taking most of the development with it), or `Or` starts (and a proof can produce
           data).
@guard     Catches assuming the verdict is about the *number of constructors* alone. `And` has
           one constructor and is a subsingleton; a one-constructor `Prop` inductive that
           stores a `Nat` is **not** — Lean refuses to generate its projections at all
           (T03-012) and LOGOS's criterion rejects it at the `infer_sort(param_type) != Prop`
           test. Both conditions matter.
@guard     Catches relying on cumulativity to paper over a mis-sorted inductive. If a `Prop`
           inductive is registered at `Type 0`, the gate never fires (the discriminant's type
           does not infer to `Sort(Prop)`, type_checker.rs:277) and **every** elimination into
           `Type` is permitted, including `Or`'s. Because LOGOS is cumulative
           (type_checker.rs:1330-1360), the resulting terms typecheck. That is the single
           easiest way to turn a translated development into one that can extract data from
           proofs.
@guard     Catches translating `Acc.rec`/`Eq.rec` as axioms. LOGOS **derives** both
           (prelude.rs:1400ff, :1462-1470) and the derived terms are kernel-checked. Emitting
           them as `add_declaration` enlarges the trusted base for no reason and loses their
           computation rules, so `Eq.rec … (refl …)` no longer reduces.
@guard     Catches forgetting that the derived recursor's motive target is *fixed*
           (recursor.rs:59-71). Even for a subsingleton, `derive_recursor` gives a motive into
           `Sort(Type 0)` — not `Type 1`, not a universe variable. A Lean proof that eliminates
           `Eq` into `Type 1` has no derived recursor available (T01-007, T01-014).
@guard     Catches assuming the two systems' subsingleton criteria coincide. They do not.
           `is_subsingleton_prop` skips the first `arity` constructor binders **positionally**
           (type_checker.rs:1699-1712); Lean permits a non-`Prop` field wherever it sits, so
           long as the indices determine it. The file below contains one declaration on which
           LOGOS is stricter (a large elimination Lean grants and LOGOS refuses — a lost
           theorem) and one on which LOGOS is **more permissive** (a large elimination Lean
           refuses and LOGOS grants — a proof yielding the data it was built from, while proof
           irrelevance says the two proofs are equal). A translator must compute Lean's verdict
           and refuse, or report a loss, wherever the two disagree. See T11-020 and T01-014.
@oracle    For every `Prop` inductive emitted, `is_subsingleton_prop` agrees with Lean's
           large-elimination verdict; `Acc`/`Eq` map onto the prelude's entries rather than
           being re-declared; every elimination of a `Prop` into `Type` present in the source
           typechecks after translation; and no elimination Lean refused is accepted.
-/

namespace T03_017

/-- **Zero constructors ⇒ subsingleton.** `False.elim` is large elimination, and it is
    load-bearing everywhere. -/
def exFalsoType (h : False) : Type := False.elim h
def exFalsoNat (h : False) : Nat := False.elim h

/-- **One constructor, all arguments in `Prop` ⇒ subsingleton.** `And` large-eliminates. -/
def andToNat {p q : Prop} (h : p ∧ q) : Nat :=
  match h with
  | And.intro _ _ => 0

/-- `Eq` is the other canonical subsingleton: one constructor (`refl`) whose only arguments
    are the inductive's own parameters. Large elimination is `Eq.rec` / `▸`. -/
def transportType {α : Type} {a b : α} (h : a = b) (P : α → Type) (x : P a) : P b := h ▸ x

example : transportType (rfl : (1 : Nat) = 1) (fun _ => Nat) 5 = 5 := rfl

/-- `Acc` — one constructor whose single non-arity argument is a `Prop`-valued function, so it
    is a subsingleton and `Acc.rec` may eliminate into `Type`. This is what makes
    well-founded *recursion* typecheck in Lean. -/
noncomputable def accDepth {α : Type} {r : α → α → Prop} {x : α} (h : Acc r x) : Nat :=
  Acc.rec (motive := fun _ _ => Nat) (fun _ _ _ => 0) h

/-- **Two constructors ⇒ not a subsingleton.** Only `Prop`-valued elimination is legal; see
    the comment block in T03-016 for Lean's exact refusal of the `Nat`-valued version. -/
def orIdem {p : Prop} (h : p ∨ p) : p :=
  match h with
  | Or.inl hp => hp
  | Or.inr hp => hp

/-! ### The arity-offset divergence

`is_subsingleton_prop` skips the **first `arity` binders of the constructor type,
positionally** (type_checker.rs:1699-1712) and requires every later one to have sort `Prop`.
Lean's criterion is *semantic*: a non-`Prop` field is permitted iff it is determined by the
indices, wherever it sits in the telescope. The two rules agree on `And`, `Eq`, `Acc` and
`False` — and disagree on the two declarations below, in opposite directions. -/

/-- **LOGOS is stricter here.** The `Nat` field sits at telescope position 1 while the arity
    is 1, so LOGOS's positional skip consumes the `True` and then tests the `Nat` — verdict
    *not* a subsingleton. Lean determines `n` from the index and grants large elimination:
    `@Weird.rec` has motive into `Sort u_1`. The Lean-legal function below therefore has no
    LOGOS counterpart, and a translator must report the loss rather than assume parity. -/
inductive Weird : Nat → Prop where
  | mk : True → (n : Nat) → Weird n

noncomputable def weirdToNat : {a : Nat} → Weird a → Nat :=
  fun {_} h => Weird.rec (motive := fun _ _ => Nat) (fun _ n => n) h

example : weirdToNat (Weird.mk True.intro 3) = 3 := rfl

/-- **LOGOS is more permissive here — and that is the dangerous direction.** The `Nat` field
    sits at telescope position 0, so LOGOS's positional skip consumes *it* and then tests the
    `True`, which is a `Prop`: verdict **subsingleton**, large elimination permitted. Lean
    refuses — `@Weird2.rec`'s motive is restricted to `Prop`, because `m` is data that the
    index `0` does not determine. After a naive translation, a *proof* of `Weird2 0` yields the
    `Nat` it was built from. See T11-020.
    The Lean-illegal function is commented because Lean will not elaborate it:
    `def weird2ToNat {a : Nat} (h : Weird2 a) : Nat :=`
    `  Weird2.rec (motive := fun _ _ => Nat) (fun m _ => m) h`
    ⇒ `recursor 'Weird2.rec' can only eliminate into Prop` -/
inductive Weird2 : Nat → Prop where
  | mk : (m : Nat) → True → Weird2 0

def w2a : Weird2 0 := Weird2.mk 7 True.intro
def w2b : Weird2 0 := Weird2.mk 9 True.intro

/-- In Lean these two proofs are equal by proof irrelevance, so no function can separate them.
    Under LOGOS's positional rule a large elimination exists that would return `7` for one and
    `9` for the other. -/
example : w2a = w2b := rfl

/-- A user-defined **non**-subsingleton with one constructor: it stores a `Nat`, which is not
    a `Prop`, so `is_subsingleton_prop`'s second test fails. Lean refuses to large-eliminate
    it — the commented line below is its exact diagnostic shape.
    `def sizedToNat (h : Sized) : Nat := match h with | Sized.mk n _ => n`
    ⇒ `recursor 'Sized.rec' can only eliminate into Prop` -/
inductive Sized : Prop where
  | mk : Nat → True → Sized

def sizedToProp (h : Sized) : True :=
  match h with
  | Sized.mk _ t => t

/-- A `Prop` inductive eliminated into `Prop` — always legal, never gated, for any number of
    constructors. -/
inductive Tri : Prop where
  | a : Tri
  | b : Tri
  | c : Tri

def triToTrue (h : Tri) : True :=
  match h with
  | Tri.a => True.intro
  | Tri.b => True.intro
  | Tri.c => True.intro

/-- The *downward* direction, which cumulativity makes free in LOGOS and which Lean handles by
    the ordinary `Prop`-valued recursor: eliminating a `Type 0` inductive into `Prop`. No gate
    applies. -/
inductive Colour where
  | red
  | green

def colourProp : Colour → Prop
  | Colour.red => True
  | Colour.green => False

example : colourProp Colour.red := True.intro

end T03_017
