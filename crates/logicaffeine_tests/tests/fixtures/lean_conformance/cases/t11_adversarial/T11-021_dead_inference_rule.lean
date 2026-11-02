/-!
@case      T11-021
@tier      11 · Adversarial and soundness traps
@title     Emitting a DEAD `InferenceRule` — seven of the 46 variants have no certifier arm, and
           the failure arrives as a certification error, not a translation error
@lean      a temporal/modal-flavoured proof: transitivity of a time order, a safety property by
           induction over a trace, fixpoint unfolding of an `Always` predicate, an existential
           witness for an `Eventually`
@expect    reject
@target    (none — the seven variants below must never appear in an emitted `DerivationTree`;
           a proof that seems to need one must be re-expressed with a live rule or rejected)
@evidence  logicaffeine_proof/src/lib.rs:488-728 (the `InferenceRule` enum, 46 variants);
           logicaffeine_proof/src/lib.rs:554-582 (the seven modal/temporal variants:
           `ModalAccess` at :558, `ModalGeneralization` at :562, `TemporalTransitivity` at
           :568, `TemporalInduction` at :571, `TemporalUnfolding` at :575,
           `EventualityProgress` at :579, `UntilInduction` at :582);
           logicaffeine_proof/src/certifier.rs:237 (`certify(tree, ctx)` — the dispatch, a
           `match &tree.rule` with one arm per implemented rule) and certifier.rs:1558-1562
           (the CATCH-ALL: `rule => Err(KernelError::CertificationError(format!(
           "Certification not implemented for {:?}", rule)))` — a grep of `certifier.rs` for
           `Modal`, `Temporal`, `Eventuality` or `Until` finds NO rule arm, so all seven land
           here); logicaffeine_proof/src/verify.rs:25-43 (`VerifiedProof`: "`verified == true`
           **iff** `proof_term.is_some()` and that term type-checks" — so a tree carrying a
           dead rule yields `verified == false` with a `verification_error`, never a proof);
           logicaffeine_proof/src/verify.rs:305-338 (`check_derivation` — the entry point that
           runs the certifier over an externally supplied tree);
           logicaffeine_tests/tests/phase_hw_temporal.rs:165,171 (the only other references to
           two of the variants in the whole repo are `let _rule = …` bindings in a test —
           nothing CONSTRUCTS them in any engine)
@diag      E_DEAD_INFERENCE_RULE — must name the variant, state that `certifier.rs` has no arm
           for it and that the catch-all at certifier.rs:1558-1562 will reject the whole tree,
           and emit nothing. The translator must not hand the tree to `check_derivation` and
           report the resulting `verification_error` as the user's problem.
@guard     **TIER 11 THESIS, ON THE PROOF-OBJECT SIDE.** The kernel accepting a term is not
           evidence of faithfulness — and here the analogous trap is that the DERIVATION LAYER
           accepting a tree is not evidence either. A `DerivationTree` is an ordinary Rust
           value: it can be constructed with any of the 46 variants, it passes every type check
           in the proof crate, and it fails only when `certify` reaches its catch-all. So a
           translator that picks a rule by NAME — `TemporalTransitivity` looks exactly right
           for a proof that `t1 < t2` and `t2 < t3` give `t1 < t3` — builds a tree that is
           well-formed, well-typed Rust and is not a proof of anything.
@guard     Catches the failure being misread. The error is
           `KernelError::CertificationError("Certification not implemented for
           TemporalTransitivity")`, surfaced through `VerifiedProof.verification_error` with
           `verified == false` (verify.rs:25-43). That reads like a KERNEL limitation — "the
           certifier does not support this yet" — and invites the translator to retry, to
           widen the search, or to report the input as unsupported. All three are wrong: the
           tree the translator built was never certifiable, and the correct response is to
           rebuild it out of live rules or to reject.
@guard     Catches `verified == false` being treated as a soft result. A `VerifiedProof` can
           carry a `derivation` and still have `verified == false` (verify.rs:29-33), and
           `compile_theorem` reports exactly that case as "derivation found but NOT
           kernel-certified (this is not a proof)". A translator that records "a derivation was
           found" as success has recorded the opposite of what happened.
@guard     Catches the rules being plausible. These seven are not obscure: transitivity of a
           temporal order, k-induction for a safety property, fixpoint unfolding of `Always`,
           and a witness for `Eventually` are the four most natural shapes in any
           verification-flavoured Lean file. The variants EXIST, with doc comments stating
           their logic (lib.rs:556-582), which is precisely what makes selecting one look
           correct. Existence in the enum is not evidence of implementation; the only evidence
           is an arm in `certify`.
@guard     Catches the generalisation. The lesson is not "avoid seven names" — it is that the
           translator must verify, per rule, that `certifier.rs` has an arm, and must keep that
           list pinned the same way it pins `PRELUDE_VERSION` (T11-015). The enum and the
           certifier are in different crates and can drift.
@oracle    Translator exits non-zero with E_DEAD_INFERENCE_RULE for any proof it would have
           expressed with one of the seven variants, and emits no `DerivationTree`. The
           decisive harness assertion is structural: the multiset of `InferenceRule` variants
           appearing in every emitted `DerivationTree` across the corpus must contain none of
           `ModalAccess`, `ModalGeneralization`, `TemporalTransitivity`, `TemporalInduction`,
           `TemporalUnfolding`, `EventualityProgress`, `UntilInduction` — checked on the tree
           before it is handed to `check_derivation`, not inferred from the result.
-/

namespace T11_021

/-! ### The four shapes that invite a dead rule. Each is ordinary Lean; none needs anything
     modal or temporal, and each has a perfectly good translation using LIVE rules. -/

/-- Transitivity of a time order. The tempting variant is `TemporalTransitivity`
    (lib.rs:568); the live translation is ordinary transitivity of `<` on `Nat`. -/
theorem timeTrans (t1 t2 t3 : Nat) (h12 : t1 < t2) (h23 : t2 < t3) : t1 < t3 :=
  Nat.lt_trans h12 h23

/-- A safety property by induction over a trace. The tempting variant is
    `TemporalInduction` (lib.rs:571); the live translation is `Nat` induction. -/
def state : Nat → Nat
  | 0 => 0
  | n + 1 => state n + 2

def safe (n : Nat) : Prop := state n % 2 = 0

theorem safeAlways : ∀ n, safe n := by
  intro n
  induction n with
  | zero => rfl
  | succ k ih => simpa [safe, state, Nat.add_mod] using ih

/-- Fixpoint unfolding of an `Always`-like predicate: `G P ⊢ P ∧ X (G P)`. The tempting
    variant is `TemporalUnfolding` (lib.rs:575); the live translation is a conjunction
    introduction plus an instantiation of the universal. -/
def always (P : Nat → Prop) : Prop := ∀ n, P n

theorem unfoldAlways (P : Nat → Prop) (h : always P) : P 0 ∧ always (fun n => P (n + 1)) :=
  ⟨h 0, fun n => h (n + 1)⟩

/-- `Eventually` by exhibiting a witness. The tempting variant is `EventualityProgress`
    (lib.rs:579); the live translation is existential introduction. -/
def eventually (P : Nat → Prop) : Prop := ∃ n, P n

theorem progress : eventually (fun n => state n = 4) := ⟨2, rfl⟩

/-- An `Until`-shaped statement, proved by induction on the trace length. The tempting variant
    is `UntilInduction` (lib.rs:582). -/
def until' (P Q : Nat → Prop) : Prop := ∃ k, Q k ∧ ∀ j, j < k → P j

theorem untilHolds : until' (fun n => state n < 4) (fun n => state n = 4) :=
  ⟨2, rfl, by
    intro j hj
    match j, hj with
    | 0, _ => decide
    | 1, _ => decide⟩

/-! ### The modal pair, in the only form Lean core can express them: an explicit accessibility
     relation and quantification over worlds. -/

/-- `□P` in `w0` with `Accessible w0 w1` gives `P` in `w1`. The tempting variant is
    `ModalAccess` (lib.rs:558); the live translation is universal instantiation followed by
    modus ponens. -/
def box (acc : Nat → Nat → Prop) (P : Nat → Prop) (w : Nat) : Prop :=
  ∀ v, acc w v → P v

theorem modalElim (acc : Nat → Nat → Prop) (P : Nat → Prop) (w0 w1 : Nat)
    (hbox : box acc P w0) (hacc : acc w0 w1) : P w1 :=
  hbox w1 hacc

/-- Necessity introduction. The tempting variant is `ModalGeneralization` (lib.rs:562); the
    live translation is universal introduction. -/
theorem modalIntro (acc : Nat → Nat → Prop) (P : Nat → Prop) (w0 : Nat)
    (h : ∀ v, acc w0 v → P v) : box acc P w0 :=
  h

/-! ### The control: a proof whose natural rule is live, so the rejection above is specific. -/

theorem plainModusPonens (P Q : Prop) (hpq : P → Q) (hp : P) : Q := hpq hp

theorem plainAndIntro (P Q : Prop) (hp : P) (hq : Q) : P ∧ Q := ⟨hp, hq⟩

end T11_021
