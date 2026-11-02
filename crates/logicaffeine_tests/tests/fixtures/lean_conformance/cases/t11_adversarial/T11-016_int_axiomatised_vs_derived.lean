/-!
@case      T11-016
@tier      11 · Adversarial and soundness traps
@title     `Int` is AXIOMATISED in LOGOS and DERIVED in Lean — the same theorem has a proof in
           one system and a trusted assumption in the other
@lean      Int, Int.add_comm, Int.add_assoc, Int.mul_comm, Int.le_total, Int.ofNat / negSucc,
           Int.induction_on
@expect    degrade
@target    `Global("Int")` plus the prelude's 16 axioms and 5 primitives; every Lean PROOF
           about `Int` becomes either an appeal to one of those axioms or a rejection.
@evidence  kernel/src/prelude.rs:354 (`ctx.add_inductive("Int", Sort(Type 0))` with NO
           `add_constructor` call anywhere for it — `Int` is an inductive of arity 0 with zero
           constructors, i.e. an opaque carrier that cannot be matched on or inducted over);
           kernel/src/prelude.rs:92-191 (`register_int_ring_axioms` — the EIGHT ring axioms,
           every one an `add_declaration`: `add_comm`, `add_assoc`, `add_zero`, `mul_comm`,
           `mul_assoc`, `mul_one`, `mul_zero`, `mul_distrib_add`);
           kernel/src/prelude.rs:202-340 (`register_int_order_axioms` — the EIGHT order axioms,
           with `a ≤ b` encoded as `Eq Bool (le a b) true`: `le_refl`, `le_trans`,
           `le_add_mono`, `le_mul_nonneg`, `le_sub`, `lt_succ_le`, `lt_add1_le`, `le_total`);
           kernel/src/prelude.rs:363-381 (`add`/`sub`/`mul`/`div`/`mod : Int → Int → Int` and
           `le`/`lt`/`ge`/`gt : Int → Int → Bool`, all `add_declaration`s — opaque);
           kernel/src/reduction.rs:444-506 (`try_primitive_reduce` — the ONLY computational
           content, and it fires only when BOTH operands are `Term::Lit`);
           in Lean 4, `Int` is `inductive Int | ofNat : Nat → Int | negSucc : Nat → Int` and
           every one of the sixteen facts above is a THEOREM with a proof term
@loss      AXIOMATISED_VERSUS_DERIVED. In Lean, `Int.add_comm` is proved from the constructor
           structure and `Nat` arithmetic; its proof term is checkable and its truth follows
           from the definition. In LOGOS it is an `add_declaration` — a trusted assumption with
           no body (prelude.rs:130-136). So a translated theorem that cites it has a different
           epistemic status from the Lean original: the Lean version is a proof, the LOGOS
           version is a derivation FROM SIXTEEN AXIOMS. The loss record must name every axiom
           the translated proof leans on; "translated to LOGOS `Int`" is not a loss record.
@loss      NO_INDUCTION_ON_INT. Lean's `Int` has constructors, so `Int.induction_on`,
           `Int.rec`, and case analysis on `ofNat`/`negSucc` are available. LOGOS's `Int` has
           ZERO constructors, so `Match` over it is inexpressible and `derive_recursor` has
           nothing to derive. Any Lean proof by induction or case analysis on an `Int` has NO
           translation — that part of the source is a `reject`, not a `degrade`, and must be
           reported as such.
@loss      SIXTEEN_AXIOMS_IS_NOT_A_RING. The axiom set is deliberately small and is not
           complete for `Int`: there is no additive inverse axiom, no cancellation, no
           `a * b = 0 → a = 0 ∨ b = 0`, no well-ordering, and nothing relating `div`/`mod` to
           `mul`/`add` at all (they appear in no axiom — see T10-018). Facts Lean proves easily
           are simply unprovable about LOGOS's `Int` on symbolic arguments, and provable only
           on literals via `try_primitive_reduce`.
@guard     **TIER 11 THESIS.** The kernel accepting a term is not evidence of faithfulness, and
           here the acceptance is doing genuinely less work than it appears to. A translated
           `Int` theorem that typechecks has been checked against SIXTEEN ASSUMPTIONS the Lean
           source did not make. If any of them is wrong — and they are hand-written `Term`
           values in a Rust file, not machine-checked (prelude.rs:92-340) — every `Int`
           theorem in the output inherits the error, and the kernel's verdict is unaffected.
           The translator must record the dependency; it cannot verify it.
@guard     Catches the translator re-declaring `Int` with constructors to recover induction.
           `add_inductive`/`add_constructor` are UNCHECKED (context.rs:253, :419), so
           `Int_ofNat`/`Int_negSucc` will register — and the result is a DIFFERENT type from
           the prelude's `Int`. The sixteen axioms are stated about the prelude's `Int`
           (prelude.rs:93, :203 both open with `let int = || Term::Global("Int")`), so none of
           them applies to the shadow type; and `try_primitive_reduce` fires only on
           `Term::Lit` operands (reduction.rs:455-500), so `add (Int_ofNat 2) (Int_ofNat 3)`
           never computes. The translation typechecks and proves nothing. See T10-003.
@guard     Catches the OPPOSITE move: keeping the prelude's `Int` and silently dropping the
           Lean proofs of the sixteen facts, on the grounds that "LOGOS already has them". It
           does have them — as axioms. The translated library's trusted base is then larger
           than the Lean source's by sixteen statements, and nothing in LOGOS records that
           (T11-015).
@guard     Catches the order encoding. LOGOS states `a ≤ b` as `Eq Bool (le a b) true`
           (prelude.rs:202-340), a `Bool`-reflected relation, whereas Lean's `Int.le` is a
           `Prop` defined via `Int.NonNeg`. These are different propositions with different
           proof terms, and translating Lean's `≤` to LOGOS's encoding silently changes what a
           proof of `a ≤ b` IS. It also makes the translation depend on the `Bool` constructor
           names `true`/`false` being the PRELUDE's (T10-008).
@guard     Catches `Int` and `Nat` being conflated to dodge all of this. They are different
           kinds of object in LOGOS — one opaque and axiomatised, one inductive and
           computational — and conflating them is T10-004's rejection.
@oracle    Every emitted `Int` term uses the prelude's `Global("Int")` and the prelude's
           operation names, with no re-declared constructors; every translated `Int` theorem
           carries a loss record listing exactly which of the sixteen axioms its proof cites;
           and every Lean proof by induction or case analysis on an `Int` is REJECTED, not
           degraded. A harness asserts that no `Term::Match` in the corpus output has an
           `Int`-typed discriminant.
-/

namespace T11_016

/-! ### Facts that are THEOREMS in Lean and AXIOMS in LOGOS. -/

example (a b : Int) : a + b = b + a := Int.add_comm a b
example (a b c : Int) : a + b + c = a + (b + c) := Int.add_assoc a b c
example (a : Int) : a + 0 = a := Int.add_zero a
example (a b : Int) : a * b = b * a := Int.mul_comm a b
example (a b c : Int) : a * b * c = a * (b * c) := Int.mul_assoc a b c
example (a : Int) : a * 1 = a := Int.mul_one a
example (a : Int) : a * 0 = 0 := Int.mul_zero a
example (a b c : Int) : a * (b + c) = a * b + a * c := Int.mul_add a b c

/-! ### Order facts, likewise. Note that Lean's `≤` on `Int` is a `Prop`, while LOGOS encodes
     it as `Eq Bool (le a b) true` (prelude.rs:202-340) — a different proposition. -/

example (a : Int) : a ≤ a := Int.le_refl a
example (a b c : Int) (h1 : a ≤ b) (h2 : b ≤ c) : a ≤ c := Int.le_trans h1 h2
example (a b : Int) : a ≤ b ∨ b ≤ a := Int.le_total a b

/-! ### Facts Lean proves and the sixteen LOGOS axioms do NOT give. -/

/-- Additive inverse. There is no `add_neg` axiom in `register_int_ring_axioms`
    (prelude.rs:92-191). -/
example (a : Int) : a + (-a) = 0 := Int.add_right_neg a

/-- Cancellation. Not an axiom either. -/
example (a b c : Int) (h : a + b = a + c) : b = c := Int.add_left_cancel h

/-- Anything relating `div`/`mod` back to `mul`/`add`. `div` and `mod` appear in NO axiom
    (prelude.rs:92-340); they are opaque declarations with a literal-only reduction hook. -/
example (a b : Int) : b * (a / b) + a % b = a := Int.ediv_add_emod a b

/-! ### Case analysis and induction on `Int` — inexpressible in LOGOS, where `Int` has zero
     constructors. These declarations are `reject`, not `degrade`. -/

def sign : Int → Int
  | Int.ofNat 0 => 0
  | Int.ofNat (_ + 1) => 1
  | Int.negSucc _ => -1

example : sign (-4) = -1 := rfl

/-- Induction over `Int`, using Lean's constructor structure. -/
theorem nonNegOfNat : ∀ n : Nat, (0 : Int) ≤ Int.ofNat n := by
  intro n
  exact Int.ofNat_nonneg n

/-! ### The literal fragment, which DOES compute in LOGOS via `try_primitive_reduce`
     (reduction.rs:444-506) — and which is the only `Int` reasoning that needs no axiom. -/

example : (3 : Int) + 4 = 7 := by decide
example : (3 : Int) * (-4) = -12 := by decide
example : ((3 : Int) ≤ 4) := by decide

end T11_016
