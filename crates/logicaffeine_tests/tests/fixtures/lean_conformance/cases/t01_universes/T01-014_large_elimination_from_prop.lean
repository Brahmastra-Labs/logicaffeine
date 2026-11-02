/-!
@case      T01-014
@tier      1 · Universes
@title     Large elimination from `Prop` — LOGOS's subsingleton gate differs from Lean's, in BOTH directions
@lean      False.elim, Eq.rec / ▸, And.rec into Type, Acc.rec, Or (refused), motive restrictions
@expect    degrade
@target    A `Term::Match` (kernel/src/term.rs:469) whose motive targets the translated Lean
           motive, subject to LOGOS's gate at kernel/src/type_checker.rs:277-288 — plus, where
           the Lean elimination is universe-polymorphic, a loss record naming
           LARGE_ELIM_LEVEL_CEILING. Where LOGOS's gate is STRICTER than Lean's the loss record
           is LARGE_ELIM_REFUSED; where it is LOOSER the translator must REJECT (see `@note`).
@evidence  kernel/src/type_checker.rs:277-288 (the gate: if the discriminant's type is a `Prop`
           and the return type is not, the inductive must satisfy `is_subsingleton_prop`);
           kernel/src/type_checker.rs:1688-1716 (`is_subsingleton_prop`: 0 constructors ⇒ true;
           exactly 1 constructor ⇒ every constructor argument BEYOND THE INDUCTIVE'S ARITY must
           have sort `Prop`; ≥ 2 constructors ⇒ false) and :1719-1724 (`pi_param_count`, the
           "arity" it skips — a COUNT of leading `Π`s in the inductive's own type, applied
           positionally to the CONSTRUCTOR's telescope);
           kernel/src/type_checker.rs:655-678 (`infer_indexed_match` repeats the same gate for
           indexed families);
           kernel/src/recursor.rs:64-71 (`derive_recursor`'s motive target is HARD-CODED:
           `Sort(Prop)` for a non-subsingleton `Prop` inductive, `Sort(Type 0)` for everything
           else — never `Type 1`, never a level variable);
           Lean's rule, for contrast: an inductive proposition eliminates into a larger sort iff
           it has at most one constructor and every constructor argument is either a proof or
           APPEARS IN THE RESULT TYPE'S INDICES — the second disjunct is the one LOGOS does not
           implement
@loss      LARGE_ELIM_LEVEL_CEILING. Lean's `False.elim`, `Eq.rec`, `And.rec` and `Acc.rec` are
           universe-POLYMORPHIC: they eliminate into `Sort u` for any `u`. LOGOS's derived
           recursors are pinned to `Sort(Type 0)` (recursor.rs:64-71) and are not
           universe-polymorphic (T01-007), so an elimination into `Type 1` or into a level
           variable has no image. `False.elim h : Type 1` below is exactly this. Acceptable only
           in the sense that the `Type 0` instance survives; every higher instance must be
           monomorphised (T01-008) or rejected.
@loss      LARGE_ELIM_REFUSED. LOGOS's gate is stricter than Lean's for any single-constructor
           proposition whose non-proof constructor arguments are justified by APPEARING IN THE
           INDICES rather than by being propositional. `Acc` survives because its data argument
           sits inside the arity prefix that `is_subsingleton_prop` skips; a differently-ordered
           constructor of the same shape does not. When LOGOS refuses an elimination Lean
           allowed, the proof must be restructured, and that restructuring must be recorded.
@guard     Catches translating a `Prop` elimination into `Type` without checking WHICH gate
           applies. The two systems agree on the easy cases — `False` (0 constructors), `Eq` and
           `And` (1 constructor, propositional arguments), `Or` (2 constructors, refused by
           both) — so a suite built from those tells you nothing.
@guard     Catches the LOOSER direction, which is a soundness hole. `is_subsingleton_prop`
           (type_checker.rs:1700-1711) computes `arity = pi_param_count(<the inductive's type>)`
           and then SKIPS the first `arity` arguments of the CONSTRUCTOR's telescope, checking
           only the rest. Those are two different telescopes. For `Sneaky : Nat → Prop` with
           `mk : (junk : Nat) → Sneaky 0`, the arity is 1 and the constructor has exactly one
           argument — `junk`, which is DATA, not a proof, and which does NOT appear in the result
           type's index (`Sneaky 0`). LOGOS skips it as though it were the index and returns
           `true`. Lean refuses this elimination (its `Sneaky.rec` motive is restricted to
           `Prop`). So LOGOS will large-eliminate a proposition that carries a genuine `Nat`,
           while `proof_irrelevant` (type_checker.rs:1547-1567) simultaneously makes
           `Sneaky.mk 0` and `Sneaky.mk 1` definitionally equal — two definitionally equal proofs
           yielding two distinct `Nat`s. A translator must detect this shape and refuse it,
           because LOGOS will not.
@guard     Catches keying the gate on the SOURCE's motive rather than on the emitted one.
           `type_checker.rs:278-279` classifies the RETURN TYPE by
           `normalize(infer_type(return_type))`, where `return_type` is
           `beta_reduce(App(motive, discriminant))` (:271). A motive the translator emitted as
           `Term::Hole` (T00-016) makes `infer_type` fail or return a non-`Sort`, and the
           `large` test at :279 then takes the wrong branch — the restriction is skipped
           entirely.
@guard     Catches relying on `derive_recursor` to reproduce Lean's `.rec`. It does not: the
           motive target is hard-coded (recursor.rs:64-71), so the derived `Eq_rec` eliminates
           into `Type 0` and nothing higher, and the derived recursor for a multi-constructor
           `Prop` targets `Sort(Prop)` regardless of what the source asked for. `Match` is the
           primitive in LOGOS and `.rec` is derived — the inverse of Lean (T03-021) — so a
           Lean proof written through `.rec` must be re-expressed as a `Match`, and its level
           re-checked against the ceiling.
@guard     Catches assuming the gate runs at all. `add_inductive` and `add_constructor`
           (context.rs:253, :419) are UNCHECKED; `is_subsingleton_prop` reads
           `ctx.get_constructors` (type_checker.rs:1689), so an inductive registered with the
           wrong constructor list — or with none — answers the gate's question from data the
           translator supplied. Zero constructors returns `true` unconditionally
           (type_checker.rs:1691), so a `Prop` inductive whose constructors the translator failed
           to register large-eliminates freely.
@note      DISAGREEMENT WITH INDEX.md. INDEX.md classifies T01-014 as `degrade`, and that is
           right for the case as a whole: the dominant direction is Lean-accepted eliminations
           that LOGOS refuses or can only perform at `Type 0`, which is a loss. But the `Sneaky`
           shape above is not a loss — it is a `reject`. LOGOS's gate ADMITS a large elimination
           that Lean denies, and combined with definitional proof irrelevance that admission is
           inconsistent. This case is therefore written as `degrade` to match the index, with the
           `Sneaky` sub-class carrying a mandatory rejection obligation in the guards and the
           oracle. I believe the index should split this into T01-014 (`degrade`,
           ceiling/refusal) and a new `reject` case for the arity-skip hole, and that T11-020's
           blanket `degrade` for "large-elimination gate differs from Lean's" under-classifies
           the same defect.
@oracle    For each `Prop` elimination: if Lean's motive targets a sort above `Type 0`, a
           LARGE_ELIM_LEVEL_CEILING loss record exists; if Lean permits the elimination and
           `is_subsingleton_prop` returns false, a LARGE_ELIM_REFUSED loss record exists; and if
           `is_subsingleton_prop` returns TRUE while Lean's rule refuses — the arity-skip hole —
           the translator exits non-zero and writes nothing. The harness independently asserts,
           against the kernel, that `is_subsingleton_prop(ctx, "Sneaky")` is `true` while
           `Sneaky.rec`'s Lean motive is `Prop`-restricted, pinning the divergence so a future
           LOGOS fix flips this case rather than silently changing its meaning.
-/

namespace T01_014

/-- ZERO constructors: both systems permit elimination into anything. LOGOS:
    `is_subsingleton_prop` returns `true` at type_checker.rs:1691. -/
def exFalsoAtTypeZero (h : False) : Nat := False.elim h

/-- The SAME elimination one level up. Lean's `False.elim` is universe-polymorphic into
    `Sort u`; LOGOS's derived recursor is pinned to `Sort(Type 0)` (recursor.rs:64-71), so this
    instance has no image. LARGE_ELIM_LEVEL_CEILING. -/
def exFalsoAtTypeOne (h : False) : Type 1 := False.elim h

/-- ONE constructor, PROPOSITIONAL arguments: both systems permit large elimination.
    `is_subsingleton_prop` walks `And`'s constructor and finds both arguments in `Prop`. -/
def andToNat (p q : Prop) (h : p ∧ q) : Nat :=
  And.rec (motive := fun _ => Nat) (fun _ _ => 0) h

/-- `Eq` — the other canonical subsingleton. Elimination into `Type 0`. -/
def eqRecToType {α : Type} (a b : α) (h : a = b) (F : α → Type) (x : F a) : F b := h ▸ x

/-- `Acc`. Its constructor's DATA argument sits inside the inductive's arity prefix, which
    `is_subsingleton_prop` skips (type_checker.rs:1704-1707), so LOGOS permits this — and so
    does Lean, for a different reason (the argument appears in the result type's indices). The
    two rules agree here BY COINCIDENCE, which is what makes the next case dangerous. -/
noncomputable def accToNat {α : Type} {r : α → α → Prop} {x : α} (h : Acc r x) : Nat :=
  Acc.rec (motive := fun _ _ => Nat) (fun _ _ _ => 0) h

/-- TWO constructors: both systems refuse large elimination. `is_subsingleton_prop` returns
    false at type_checker.rs:1714; Lean restricts `Or.rec`'s motive to `Prop`. The agreement
    here is why a suite of `Or`/`Exists` tests finds nothing. -/
theorem orSmallElim (p q : Prop) (h : p ∨ q) : q ∨ p :=
  match h with
  | Or.inl hp => Or.inr hp
  | Or.inr hq => Or.inl hq

/-- THE DIVERGENCE. One constructor; one argument, `junk : Nat`, which is DATA and does NOT
    appear in the result type's index (`Sneaky 0`, a literal). Lean refuses large elimination —
    `Sneaky.rec`'s motive is `Prop`-restricted. LOGOS's `is_subsingleton_prop` computes
    `arity = pi_param_count(Π(_:Nat). Prop) = 1`, then skips the first constructor argument as
    though it were that index, finds nothing left to check, and returns `true`. -/
inductive Sneaky : Nat → Prop where
  | mk : (junk : Nat) → Sneaky 0

def sneak0 : Sneaky 0 := Sneaky.mk 0
def sneak1 : Sneaky 0 := Sneaky.mk 1

/-- …and the two proofs are definitionally equal, by proof irrelevance, in BOTH systems. Large
    elimination out of `Sneaky` would therefore extract two different `Nat`s from two
    definitionally equal proofs. Lean's gate is what prevents it; LOGOS's does not. -/
theorem sneaksEqual : sneak0 = sneak1 := rfl

/-- The same hole with the data argument in a different position, so that LOGOS's positional
    skip does NOT cover it — `is_subsingleton_prop` reaches `junk` and returns false. The pair
    pins that the divergence is about ARGUMENT POSITION, not about the shape of the type. -/
inductive NotSneaky : Nat → Prop where
  | mk : (n : Nat) → (junk : Nat) → NotSneaky n

/-- A single-constructor proposition with NO indices, carrying data: both systems refuse. -/
inductive Carries : Prop where
  | mk : Nat → Carries

theorem carriesIrrelevant (a b : Carries) : a = b := rfl

end T01_014
