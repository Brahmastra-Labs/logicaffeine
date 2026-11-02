/-!
@case      T11-020
@tier      11 · Adversarial and soundness traps
@title     The large-elimination gate measures TWO DIFFERENT TELESCOPES — it admits shapes Lean
           refuses, and combined with proof irrelevance the admitted direction is inconsistent
@lean      Prop-valued inductive, singleton elimination, Acc.rec, Eq.rec (J), False.elim,
           an indexed Prop whose constructor carries a data field
@expect    degrade
@target    For the shapes both systems admit (`False`, `Eq`, `Acc`, `And`): the derived
           recursor, via `derive_recursor` + `add_definition`. For every other shape: a
           rejection. The translator must decide by LEAN's criterion, never by calling
           `is_subsingleton_prop`.
@evidence  kernel/src/type_checker.rs:1688-1716 (`is_subsingleton_prop`, in full). The decisive
           three lines are :1694-1701: it takes `ctor_type` from `ctx.get_constructors(...)[0]`,
           takes `ind_type` from `ctx.get_global(inductive_name)`, and computes
           `let arity = pi_param_count(&ind_type);` — the number of leading `Π` binders of the
           INDUCTIVE's type, i.e. parameters PLUS INDICES. It then walks the CONSTRUCTOR's
           telescope at :1703-1710 and applies the propositionality test only when
           `i >= arity`. A well-formed constructor's telescope begins with the PARAMETERS and
           then continues with its FIELDS — the indices are not bound there, they are
           determined by the constructor's target. So the skip of `arity` binders discards
           `num_indices` genuine FIELDS. Two telescopes, one counter.
           kernel/src/type_checker.rs:1719-1724 (`pi_param_count` — a plain recursive count,
           with no reference to `inductive_num_params`);
           kernel/src/context.rs:374 and :404-411 (`add_indexed_inductive` /
           `inductive_num_indices` — the params/indices split IS recorded in the context, and
           `is_subsingleton_prop` does not consult it);
           kernel/src/recursor.rs:64-71 (`derive_recursor` chooses the motive target using this
           predicate); kernel/src/type_checker.rs:672-680 (the indexed-match path applies the
           same predicate); kernel/src/type_checker.rs:1547-1567 (`proof_irrelevant` — any two
           proofs of one `Prop` are definitionally equal);
           kernel/src/prelude.rs:1354-1421 (`add_indexed_inductive("Acc", acc_ty, 2)` — `Acc`
           has arity 3 and 2 parameters, and it is the full-arity skip that makes `Acc` come
           out a subsingleton and therefore makes well-founded recursion's eliminator exist at
           all: the behaviour is load-bearing, not merely a latent bug)
@loss      GATE_IS_NOT_LEAN'S. Lean admits large elimination from a `Prop` iff the inductive has
           zero constructors, or exactly one whose every NON-PARAMETER field is a proof. LOGOS
           admits it iff the inductive has zero constructors, or exactly one whose constructor
           telescope, AFTER DISCARDING `params + indices` LEADING BINDERS, contains only
           propositional entries. On a `Prop` with no indices the two coincide. On an INDEXED
           `Prop` they do not, and LOGOS's is strictly more permissive by exactly the number of
           indices. The loss record must state which criterion was used and must list, per
           declaration, the fields the LOGOS gate skipped.
@guard     **TIER 11 THESIS, AND THE PLACE IT BITES HARDEST.** LOGOS's kernel accepting a term
           is not evidence that the translation is faithful — and here the admitted direction
           is not merely unfaithful, it is INCONSISTENT WITH ANOTHER RULE THE SAME KERNEL HAS.
           Walk it on the example below.
             · `Tag : Bool → Prop` with the single constructor `mk : ∀ (n : Nat), Tag (n == 0)`.
               `ind_type` is `Π(_:Bool). Prop`, so `arity = 1` (type_checker.rs:1701).
             · The constructor's telescope is `Π(n : Nat). Tag (n == 0)` — ONE binder, and it
               is a FIELD, not a parameter or an index.
             · The loop at :1703-1710 skips it because `i = 0 < arity = 1`. Nothing is ever
               tested. `is_subsingleton_prop` returns `true`.
             · So `derive_recursor` (recursor.rs:64-71) gives `Tag` a `Sort(Type 0)` motive:
               large elimination is ALLOWED, and a proof of `Tag b` can be eliminated to
               produce the `Nat` it was built from. Lean forbids exactly this (its `Tag.rec`
               motive is `Prop`-valued; the error is reproduced below).
             · Now add `proof_irrelevant` (type_checker.rs:1547-1567). `Tag.mk 1` and
               `Tag.mk 2` are both proofs of `Tag false`, so they are DEFINITIONALLY EQUAL.
               Congruence then makes `extract (Tag.mk 1) ≡ extract (Tag.mk 2)`, i.e. `1 ≡ 2`.
           A proof yields data, and proof irrelevance says that data is equal to other data.
           The kernel typechecks every step.
@guard     Catches the translator delegating the decision. The tempting workflow is: emit the
           inductive, call `derive_recursor`, and if the kernel gives you a `Type 0` motive
           then large elimination must be legitimate. It is not — the kernel is answering with
           the predicate above. The criterion must be computed from LEAN's judgement, recorded
           at translation time, and compared.
@guard     Catches the REFUSED direction, which loses proofs rather than soundness. The same
           counter runs the other way whenever the inductive's recorded arity is SMALLER than
           the constructor's parameter prefix — which the translator controls, because
           `add_inductive` treats the whole arity as parameters while `add_indexed_inductive`
           takes an explicit split (context.rs:253 vs :374, and T03-005). Register a
           parameterised `Prop` with a sort that omits its parameters and the gate starts
           testing parameter binders for propositionality, `infer_sort` returns `Type 0` for a
           `Type`-valued parameter, and a perfectly legitimate singleton elimination — `Eq`'s
           J, `False`'s ex falso — is refused.
@guard     Catches "fixing" the gate by disabling it. The predicate is load-bearing for the
           prelude: `Acc` is registered with `add_indexed_inductive("Acc", acc_ty, 2)` at
           arity 3 (prelude.rs:1379), and it is precisely the full-arity skip that lets
           `is_subsingleton_prop` return `true` for it, which is what gives `Acc` a `Type 0`
           motive and makes well-founded recursion's eliminator exist. A translator that works
           around the gate by re-registering prelude types will break `Acc`, `Eq` and `False`
           together.
@guard     Catches the shapes where the two DO agree, which is most of them, and which is why
           this is easy to miss. `False` (zero constructors), `And` (one constructor, both
           fields `Prop`), `Eq` (one constructor, no fields beyond the arity), `Or` (two
           constructors — refused by both) all come out the same. The divergence needs an
           INDEXED `Prop` with a data-carrying constructor, which is rare in mathematical Lean
           and common in program-verification Lean.
@oracle    For each declaration below the translator emits a recursor ONLY when Lean's own
           criterion admits large elimination, and a rejection otherwise; and a loss record
           with category GATE_IS_NOT_LEAN'S names, per inductive, the parameter count, the
           index count, and the constructor fields LOGOS's gate would skip. The decisive
           harness assertion is differential: for every `Prop` inductive in the corpus,
           `is_subsingleton_prop`'s answer is computed AND Lean's answer is computed, and any
           disagreement must have been reported — a translator that never computes Lean's
           answer fails this case even if every emitted term typechecks.
@note      INDEX lists T11-020 as `degrade`, and this file is kept at `degrade` so the
           conformance IDs stay stable. I disagree with the verdict for half the case and
           think it should be SPLIT. The two directions are not the same kind of failure:
             · The REFUSED direction (LOGOS's gate rejects a shape Lean admits) is a genuine
               `degrade`: a proof is lost, nothing unsound is emitted, and a loss record is the
               right response.
             · The ADMITTED direction (LOGOS's gate accepts a shape Lean refuses) is a
               `reject`. As the first `@guard` shows, it is not merely unfaithful — combined
               with `proof_irrelevant` it makes two distinct `Nat`s definitionally equal, which
               is an inconsistency in the emitted theory, not a lost proof. It belongs with
               T11-013 and T11-014 as a hard rejection with its own diagnostic
               (E_LARGE_ELIMINATION_NOT_LEAN'S), because "emit the term plus a loss record" is
               not an acceptable response to it.
           A harness should therefore score the admitted-direction declarations below
           (`Tag`, `Sized`) as `reject` cases even while the case ID's nominal verdict stays
           `degrade`.
-/

namespace T11_020

/-! ### Shapes where BOTH systems admit large elimination — the control. -/

/-- Zero constructors: ex falso. -/
example (h : False) : Nat := h.elim

/-- One constructor, no fields beyond the arity: J. -/
example {α : Type} {a b : α} (h : a = b) (P : α → Type) (x : P a) : P b := h ▸ x

/-- One constructor, all fields propositional. -/
example (p q : Prop) (h : p ∧ q) : p := h.1

/-- `Acc`, whose eliminator into `Type` is what well-founded recursion is built on. Marked
    `noncomputable` only because Lean's code generator declines bare recursors; the KERNEL
    accepts the `Type`-valued motive, which is the point. -/
noncomputable def accToNat {α : Type} {r : α → α → Prop} {a : α} (h : Acc r a) : Nat :=
  Acc.rec (motive := fun _ _ => Nat) (fun _ _ _ => 0) h

/-! ### Shapes where BOTH refuse — also the control. -/

/-
  noncomputable def orToNat (p q : Prop) (h : p ∨ q) : Nat :=
    Or.rec (motive := fun _ => Nat) (fun _ => 0) (fun _ => 1) h

  error: type mismatch
    Nat
  has type
    Type : Type 1
  but is expected to have type
    Prop : Type

  Two constructors, so `is_subsingleton_prop` returns `false` at its `_ => false` arm
  (type_checker.rs:1715) and Lean's criterion refuses too. Agreement.
-/

/-! ### THE DIVERGENCE. An INDEXED `Prop` whose single constructor carries a DATA field.
     LOGOS's gate skips `arity = 1` leading binders of the constructor telescope
     (type_checker.rs:1701-1710) — and that one binder IS the data field, because the index is
     not bound in the telescope. Nothing is tested; the predicate returns `true`. -/

inductive Tag : Bool → Prop where
  | mk : ∀ (n : Nat), Tag (n == 0)

/-
  noncomputable def extract {b : Bool} (t : Tag b) : Nat :=
    Tag.rec (motive := fun _ _ => Nat) (fun n => n) t

  error: type mismatch
    Nat
  has type
    Type : Type 1
  but is expected to have type
    Prop : Type

  Lean's `Tag.rec` has a `Prop`-valued motive:
    @Tag.rec : ∀ {motive : (a : Bool) → Tag a → Prop},
      (∀ (n : Nat), motive (n == 0) ⋯) → ∀ {a : Bool} (t : Tag a), motive a t
  LOGOS's `derive_recursor` would give it a `Sort(Type 0)` motive (recursor.rs:64-71),
  because `is_subsingleton_prop` said `true`.
-/

/-- The compiling neighbour: eliminate into `Prop`, which both systems allow. -/
theorem tagTrivial {b : Bool} (t : Tag b) : True :=
  Tag.rec (motive := fun _ _ => True) (fun _ => trivial) t

/-- The INCONSISTENCY, assembled. Two proofs of one proposition, built from different `Nat`s.
    Proof irrelevance identifies them — in Lean and in LOGOS
    (`proof_irrelevant`, type_checker.rs:1547-1567). -/
example : (Tag.mk 1 : Tag false) = (Tag.mk 2 : Tag false) := rfl

theorem tagIrrelevant (p q : Tag false) : p = q := rfl

/-- And the two `Nat`s they were built from are not equal. If LOGOS's `extract` existed — and
    under `is_subsingleton_prop` it does — congruence on the line above would make these two
    definitionally equal. -/
example : (1 : Nat) ≠ 2 := by decide

/-! ### A second instance of the same shape, to show it is not an artefact of `Bool`. -/

inductive Sized : Nat → Prop where
  | mk : ∀ (payload : Nat) (bound : Nat), Sized (payload + bound)

/-- Lean's motive is again `Prop`-valued; LOGOS's gate skips `arity = 1` binder (`payload`)
    and tests only `bound : Nat`, whose sort is `Type 0` — so THIS one LOGOS refuses, for the
    wrong reason. The predicate's answer happens to agree with Lean here while its
    justification does not, which is why a harness must compare criteria and not just
    outcomes. -/
theorem sizedTrivial {n : Nat} (s : Sized n) : True :=
  Sized.rec (motive := fun _ _ => True) (fun _ _ => trivial) s

/-! ### A parameterised `Prop` with NO indices, where the two criteria coincide exactly. -/

inductive Wrap (α : Type) : Prop where
  | mk : α → Wrap α

/-
  noncomputable def unwrap {α : Type} (w : Wrap α) : α :=
    Wrap.rec (motive := fun _ => α) (fun a => a) w

  error: type mismatch
    α
  has type
    Type : Type 1
  but is expected to have type
    Prop : Type

  LOGOS: `ind_type` is `Π(α:Type). Prop`, arity 1; the constructor telescope is
  `Π(α:Type). Π(_:α). Wrap α`, so `i = 1 >= arity` tests the field `α`, whose sort is
  `Type 0` — refused. The two agree, because there are no indices to mis-skip.
-/

theorem wrapTrivial {α : Type} (w : Wrap α) : True :=
  Wrap.rec (motive := fun _ => True) (fun _ => trivial) w

end T11_020
