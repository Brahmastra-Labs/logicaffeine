/-!
@case      T12-013
@tier      12 · Vernacular surfaces
@title     The trust door — a derivation is not a proof; `compile_theorem` returns `Ok` for both
@lean      any `theorem`; the case is about how a translator's harness decides that the
           LOGOS side succeeded
@expect    degrade
@target    Whatever surface the theorem is routed to, the translator's success predicate
           must be `VerifiedProof::verified == true`, never "a derivation came back" and
           never "the call returned `Ok`". Concretely, one of:
           · `compile_theorem(text)` ⟶ `Ok(s)` AND `s.contains("[kernel-verified]")`
           · `compile_theorem_for_ui(text)` ⟶ `r.verified == true`
           · `prove_certify_check(&premises, &goal)` ⟶ `.verified == true`
           · the raw kernel path ⟶ `Context::add_definition` succeeded after
             `infer_type` returned the goal's type
@evidence  proof/src/verify.rs:26-44 — the invariant, in LOGOS's own words:
           "`verified == true` **iff** `proof_term.is_some()` and that term type-checks in
           `kernel_ctx`. When verification fails, `derivation` may still be present (the
           chainer found *a* derivation) but it is never reported as verified".
@evidence  proof/src/verify.rs:1-15 — the module doc: "A derivation alone never counts as
           verified — `verified == true` is always backed by a `Term` that re-checks under
           `infer_type` in the returned `Context`."
@evidence  proof/src/verify.rs:501-580 — `finish_check` has THREE distinct exits that
           return `derivation: Some(derivation)` with `verified: false`:
             · :513-521  certification failed (`certify` returned `Err`)
             · :540-548  the certified term did not type-check (`infer_type` failed)
             · :556-563  the goal's own type could not be built
           and a fourth, further down, when the inferred type is not the goal's type.
@evidence  language/src/compile.rs:1154-1176 — the door itself. BOTH of the first two
           arms return `Ok`:

               Some(derivation) if outcome.verified => Ok(format!(
                   "Theorem '{}' Proved! [kernel-verified]\n{}", …)),
               Some(derivation) => Ok(format!(
                   "Theorem '{}' — derivation found but NOT kernel-certified (this is not a proof)\n{}", …)),
               None => Err(…)

           So `compile_theorem(...).is_ok()` is TRUE for an uncertified derivation, and
           the only thing distinguishing the two is a substring.
@evidence  language/src/compile.rs:1129-1135 — the comment at the call site says the same
           thing: "This door reports a proof when a derivation is found, annotating
           whether the kernel certified it; the strong (kernel-checked) guarantee is
           exposed by `verify_theorem` and the `verified` flag on
           `TheoremCompileResult`."
@evidence  proof/src/certifier.rs:1546-1557 — a concrete producer of the unverified state:
           an `InferenceRule::OracleVerification` leaf is deliberately non-certifiable
           ("oracle (Z3) results are not kernel-certifiable — they attest satisfiability
           but produce no checkable proof term"), so a goal the engine discharges only
           through the oracle yields derivation-without-proof.
@evidence  tests/phase_oracle_trust_boundary.rs:14-35 — that behaviour asserted directly:
           a hand-built `OracleVerification` leaf must fail `certify`, and the rejection
           must name the oracle.
@evidence  proof/src/lib.rs:488-728 — `InferenceRule` has 46 variants; seven
           (`ModalAccess`, `ModalGeneralization`, `TemporalTransitivity`,
           `TemporalInduction`, `TemporalUnfolding`, `EventualityProgress`,
           `UntilInduction`) have no certifier arm and fall to the catch-all at
           certifier.rs:1558-1560, "Certification not implemented for {:?}" — a second
           family of derivation-without-proof. (T11-021.)
@evidence  compile/src/ui_bridge.rs:644 and tests/phase_studio_grid_proof.rs:32-36 — the
           UI path exposes the flag directly as `r.verified`, which is the predicate the
           suite's own tests assert on. A translator harness should prefer this path for
           exactly that reason.
@loss      TRUST_LEVEL_UNRECORDED is the loss this case is about: a translator that
           records "LOGOS accepted it" without recording WHICH door it came through has
           produced a conformance report in which certified and uncertified results are
           indistinguishable. The verdict is `degrade` rather than `translate` because
           every surface can return this state and no surface prevents it; the obligation
           is to record the trust level per theorem, not to avoid it.
@guard     THE guard of this tier, and the one that corrupts a whole library rather than
           one theorem. A harness written as

               if compile_theorem(&text).is_ok() { report.pass(case_id) }

           reports as PROVED every theorem for which LOGOS found a derivation it could
           not certify. That is not a translator bug at all — the translation may be
           perfect — it is a *measurement* bug, and it silently inflates the conformance
           score in exactly the cases where the corpus is most valuable. Worse, the
           failure is invisible in the emitted text: the same `## Theorem` block returns
           `Ok` either way.
@guard     Catches matching on `"Proved"`. The certified string is
           `"Theorem '…' Proved! [kernel-verified]"` and the uncertified one is
           `"Theorem '…' — derivation found but NOT kernel-certified (this is not a
           proof)"`. Several tests in the suite (e.g. tests/phase63_theorem_parser.rs:47-51)
           assert `output.contains("Proved")`, which is adequate for those tests because
           the uncertified string does not contain `"Proved"` — but it is fragile, and
           `"[kernel-verified]"` is the only substring that means what a conformance
           harness needs it to mean.
@guard     Catches treating a returned `DerivationTree` as evidence. `VerifiedProof`
           exposes `derivation`, `proof_term` and `verified` separately precisely because
           they come apart. The checkable artefact is `proof_term`, and even that is
           insufficient on its own: `finish_check` additionally requires that the term's
           inferred type BE the goal's type (verify.rs:532-536, "The term must not merely
           be well-typed — its type must be the goal"). A harness that re-checks a
           returned `proof_term` with `infer_type` and stops there accepts a well-typed
           proof of the WRONG proposition.
@guard     Catches importing the mistake into the literate surface, where it is worse.
           On Surface C there is no `verified` flag at all: `Repl::execute` returns
           `Ok` whenever the definition typechecks, and `try_lia ⌜false thing⌝` always
           typechecks (T12-008). The literate analogue of this guard is to evaluate
           `concludes <name>` and compare it to the goal's `Syntax`.
@oracle    The harness's pass predicate is `verified == true` (or the
           `"[kernel-verified]"` substring), and the conformance report records, per
           case, which of {kernel-verified, derivation-only, no-derivation, parse-error}
           occurred. A meta-oracle: feeding the harness a theorem whose only route is the
           oracle must produce a FAIL, not a PASS.
-/

namespace T12_013

/-- A theorem whose LOGOS route is structural and certifiable — the control case. -/
theorem certifiable (P Q : Prop) (hpq : P → Q) (hp : P) : Q := hpq hp

/-- A theorem whose statement is arithmetic over a bounded range. In LOGOS this is the
    shape that reaches the arithmetic/oracle tier, where a derivation can exist without a
    certificate. Proved here by `omega`, which is Lean core. -/
theorem boundedRange (x : Nat) (h1 : 2 < x) (h2 : x < 4) : x = 3 := by omega

/-- Another oracle-shaped obligation: a linear consequence with a coefficient. -/
theorem coefficientBound (x : Nat) (h : 3 * x ≤ 10) : x ≤ 3 := by omega

/-- A modal/temporal proposition, for the dead-`InferenceRule` family: `□` is encoded
    here as an abstract operator so the Lean file stays core-only. A LOGOS derivation
    carrying `ModalAccess` or `TemporalInduction` has no certifier arm and comes back
    with `verified == false`. -/
axiom Box : Prop → Prop
axiom boxMP : ∀ (P Q : Prop), Box (P → Q) → Box P → Box Q

theorem modalDistribution (P Q : Prop) (h1 : Box (P → Q)) (h2 : Box P) : Box Q :=
  boxMP P Q h1 h2

end T12_013
