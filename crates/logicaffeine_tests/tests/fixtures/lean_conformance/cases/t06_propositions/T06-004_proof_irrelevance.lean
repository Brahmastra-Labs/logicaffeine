/-!
@case      T06-004
@tier      6 · Propositions, equality, classical reasoning
@title     Definitional proof irrelevance — total in Lean, a partial last-resort rule in LOGOS
@lean      definitional proof irrelevance in `Prop`, `Subsingleton.elim`-free `rfl` between
           two distinct proofs of the same proposition
@expect    degrade
@target    No emitted term: the two proofs are translated as-is and the translator must RELY
           on `type_checker::proof_irrelevant` to close the gap — plus a loss record, because
           that reliance is conditional. The relevant LOGOS rule is
           `def_eq_normalized(a, b)` falling through structural congruence into
           `proof_irrelevant(ctx, a, b)`.
@evidence  kernel/src/type_checker.rs:1536-1541 (proof irrelevance fires ONLY after structural
           congruence fails — "Fires only when structural comparison fails (so it never costs
           on the common path)"); kernel/src/type_checker.rs:1547-1567 (`proof_irrelevant`
           calls `infer_type(ctx, a)`, then `infer_type` on THAT type to check it is
           `Sort(Prop)` or `Sort(SProp)`, then `infer_type(ctx, b)` and `def_eq` on the two
           types — FOUR inference calls, and every one of them `return false` on `Err`);
           kernel/src/reduction.rs:52-69 (`normalize` has a hard fuel of 10000 and silently
           returns a partially-reduced term, which is what the `def_eq` at :1566 runs on)
@loss      PROOF_IRRELEVANCE_PARTIALITY. Lean's proof irrelevance is a TOTAL definitional rule
           applied by the kernel's `whnf`/`isDefEq` on any two terms whose type is a `Prop` —
           it never fails for well-typed input. LOGOS's is a *fallible predicate*: it returns
           `false` (meaning "not equal") whenever `infer_type` errors on either side, whenever
           `infer_type` of the type does not NORMALIZE to `Sort(Prop)` within the 10000-step
           fuel, or whenever the two propositions are not `def_eq`. So a Lean proof that is
           accepted by irrelevance can be REJECTED by LOGOS. The loss is one-directional —
           LOGOS never accepts more here than Lean does — so it is a degrade, not a soundness
           reject; but a translator that assumes it is total will produce certificates that
           fail to check for reasons the diagnostic will not explain.
@loss      COST. `proof_irrelevant` runs up to four `infer_type` calls per comparison, and
           `infer_type` itself calls `normalize`. Because the rule sits at the BOTTOM of
           `def_eq_normalized`, every FAILING definitional-equality check in the whole
           development pays this cost before reporting failure. A translator that emits many
           small propositional obligations turns an O(n) check into an O(n · |term|) one.
@guard     Catches the translator emitting an explicit `Subsingleton.elim`-style bridge term
           for every propositional equality "to be safe". There is no `Subsingleton` in the
           LOGOS prelude (see T06-015), so such a bridge is a fresh axiom — a silent
           trusted-base expansion (T06-009/010/011).
@guard     Catches the opposite error: assuming irrelevance is available for `SProp`. LOGOS
           mentions `Universe::SProp` in `proof_irrelevant` (type_checker.rs:1554-1557) and in
           `imax`, but NOTHING ever constructs or declares an `SProp` inductive — the variant is
           vestigial (see T01-013). An emitted `SProp` proposition has no constructors to prove
           it with.
@guard     Catches the specific partiality trap exercised by `irrelevanceUnderBinder` below: a
           proof whose type mentions a variable bound in an enclosing `Pi` is only typeable in
           the EXTENDED context, and `proof_irrelevant` calls `infer_type(ctx, …)` against the
           context it was handed. If the translator hoists a proof term out of its binder (a
           tempting CSE optimisation under the named representation), `infer_type` returns
           `Err(UnboundVariable)` and irrelevance silently reports "not equal" instead of
           reporting an error.
@oracle    Each `rfl` below that Lean accepts by irrelevance must be checked against LOGOS
           `def_eq`; every one that LOGOS's `proof_irrelevant` declines must appear in the loss
           record under PROOF_IRRELEVANCE_PARTIALITY. Silent success with no loss record is a
           failure of this case.
-/

namespace T06_004

/-- Two syntactically different proofs of the same proposition. Lean closes this by
    definitional proof irrelevance alone. -/
theorem twoProofs (p q : True) : p = q := rfl

/-- The canonical shape: a proposition with two genuinely different derivations. -/
theorem orProofs (h : True ∨ True) : h = Or.inl True.intro := by
  cases h <;> rfl

/-- Irrelevance inside a dependent pair-like structure: the proof field is erased. -/
structure Bounded where
  val : Nat
  isSmall : val = val

theorem boundedEta (a b : Bounded) (h : a.val = b.val) : a = b := by
  cases a; cases b; cases h; rfl

/-- Irrelevance UNDER a binder. The proof's type mentions `n`, so it is only typeable in the
    extended context — this is the shape that breaks a translator which hoists the proof. -/
theorem irrelevanceUnderBinder (f g : (n : Nat) → n = n → Nat)
    (hfg : ∀ n h, f n h = g n h) (n : Nat) (h₁ h₂ : n = n) : f n h₁ = g n h₂ := by
  have : h₁ = h₂ := rfl
  subst this
  exact hfg n h₁

/-- Irrelevance where the two propositions are equal only up to REDUCTION, not syntax —
    LOGOS must reach `def_eq` on the two inferred types, which runs `normalize` under fuel. -/
def Small (n : Nat) : Prop := n + 0 = n

theorem reducedProps (p : Small 3) (q : (3 : Nat) = 3) : p = q := rfl

end T06_004
