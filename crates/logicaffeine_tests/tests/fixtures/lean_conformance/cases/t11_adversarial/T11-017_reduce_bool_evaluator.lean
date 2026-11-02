/-!
@case      T11-017
@tier      11 · Adversarial and soundness traps
@title     `reduceBool` wires a whole EVALUATOR into kernel reduction — the TCB expansion is
           real and is not the same one Lean's `native_decide` makes
@lean      native_decide, Lean.ofReduceBool, Lean.reduceBool, Lean.trustCompiler, decide
@expect    degrade
@target    `Global("reduceBool")` applied to the translated decision procedure, discharged by
           `Global("ofReduceBool")` — PLUS a loss record naming the evaluator as a trusted
           component.
@evidence  kernel/src/prelude.rs:1195-1219 (`register_native_decide` — registers exactly
           `reduceBool : Bool → Bool` and `ofReduceBool`, the LOGOS analogue of Lean's pair);
           kernel/src/reduction.rs:105-116 (the reduction rule, with the repo's own comment:
           "Native reduction: `reduceBool t` evaluates `t` with the fast CBV evaluator (the
           `native_decide` engine). This is the ONLY place the trusted evaluator enters kernel
           reduction — gated behind the `reduceBool` primitive" — the body is
           `if name == "reduceBool" { if let Some(b) = crate::eval::eval_bool(ctx, arg) {
           return Term::Global(if b { "true" } else { "false" }) } }`);
           kernel/src/eval.rs (`eval_bool` — the entire CBV evaluator, a second implementation
           of LOGOS's operational semantics, now inside the kernel's conversion checker);
           kernel/src/reduction.rs:52-69 (`normalize`'s 10000-step fuel does NOT bound
           `eval_bool`, which runs to completion or fails);
           kernel/src/prelude.rs:593-604 (the `true`/`false` globals the rule returns are the
           prelude's, so this rule also depends on `Bool` not being shadowed — T10-008);
           kernel/src/certificate.rs:28 and :63-75 (`PRELUDE_VERSION` covers the prelude's
           DECLARATIONS; it says nothing about the evaluator's implementation, which is a
           different file and can change without the string changing — T11-015)
@loss      EVALUATOR_IN_THE_TCB. Ordinary kernel reduction is a few hundred lines of
           `reduce_step` (reduction.rs). `reduceBool` admits `eval.rs` — a separate, much
           larger CBV evaluator — into the conversion relation. Any bug in `eval_bool` is a
           soundness bug in the kernel, reachable from any proof that uses `native_decide`. The
           loss record must say so and must name every declaration that uses it, because the
           set of theorems whose validity depends on the evaluator is exactly that set.
@loss      DIFFERENT_TCB_FROM_LEAN. Lean's `native_decide` trusts the Lean COMPILER (`#print
           axioms` reports `Lean.ofReduceBool`, and via it `Lean.trustCompiler`): the decision
           procedure is compiled to machine code and run. LOGOS's `reduceBool` trusts
           `eval.rs`, an interpreter inside the kernel crate. These are different trusted
           components with different failure modes, so a `native_decide` proof does not
           translate to "the same amount of trust"; it translates to a DIFFERENT amount, and
           the record must say which.
@guard     **TIER 11 THESIS.** The kernel accepting a term is not evidence of faithfulness —
           and `reduceBool` is the case where the kernel's acceptance is partly delegated to
           code that is not the kernel. A `reduceBool`-discharged proof typechecks; what it
           establishes is "the CBV evaluator returned `true`", which is a claim about a
           program, not a derivation.
@guard     Catches emitting `reduceBool` for something that is not a decision procedure. The
           rule fires on ANY `Term::Global("reduceBool")` application whose argument
           `eval_bool` can evaluate (reduction.rs:109-115); there is no check that the argument
           is the `decide` of the proposition being proved. So a translator that reaches for
           `reduceBool` to make a stuck computation go through — a large `Nat` recursion past
           the fuel boundary (T11-005), say — has replaced a definitional-equality obligation
           with an evaluator call that answers a different question, and `ofReduceBool` will
           happily convert the answer into a proof of the ORIGINAL proposition.
@guard     Catches the fuel asymmetry. `normalize`'s 10000-step budget (reduction.rs:52-69)
           does not apply inside `eval_bool`. So a translator can make a proof succeed by
           routing it through `reduceBool` — which is exactly the wrong reason to choose an
           encoding, because it silently converts a budget problem into a trust problem. The
           choice must be driven by the SOURCE using `native_decide`, never by the target
           failing to reduce.
@guard     Catches the `Bool` shadowing interaction. The rule returns `Term::Global("true")` /
           `Global("false")` literally (reduction.rs:112-114), which are the prelude's
           constructors (prelude.rs:599-603). A translator that emitted its own `Bool`
           (T10-008) gets a `reduceBool` result of a type unrelated to the proposition it was
           meant to decide — and, because the branches usually have the same type, the term
           still checks.
@guard     Catches leaving `reduceBool` out of the trusted-base record. It is registered by
           `StandardLibrary::register` (prelude.rs:1195-1219), so it is inside
           `PRELUDE_VERSION`'s nominal scope — but the EVALUATOR it invokes lives in `eval.rs`
           and is not described by that string at all (T11-015). A record that names only the
           axioms understates the TCB.
@oracle    A `reduceBool`/`ofReduceBool` pair is emitted ONLY for Lean declarations whose
           `#print axioms` lists `Lean.ofReduceBool`, never as a fallback for a reduction that
           did not go through; and each such declaration carries a loss record with categories
           EVALUATOR_IN_THE_TCB and DIFFERENT_TCB_FROM_LEAN. A harness asserts that the set of
           corpus declarations emitting `reduceBool` is exactly the set whose Lean proofs
           depend on `Lean.ofReduceBool`.
-/

namespace T11_017

/-- The canonical use: a decision that is too big for kernel reduction, discharged by the
    compiled evaluator. -/
theorem bigArithmetic : (123456 : Nat) * 654321 = 80779853376 := by native_decide

#print axioms bigArithmetic

/-- The same proposition by kernel reduction, for contrast. Lean's `Nat` is GMP-accelerated so
    this is cheap in Lean; in LOGOS the emitted `Nat` multiplication is a `Fix`/`Match`
    recursion far past the 10000-step fuel (T10-017, T11-005) — which is exactly the pressure
    that tempts a translator to reach for `reduceBool` when the source did not. -/
theorem smallArithmetic : (12 : Nat) * 12 = 144 := by decide

#print axioms smallArithmetic

/-- A decidable predicate over a bounded range — the realistic `native_decide` shape. -/
def allBelow (n : Nat) : Bool :=
  (List.range n).all (fun k => k < n)

theorem allBelowHolds : allBelow 2000 = true := by native_decide

#print axioms allBelowHolds

/-- The same by `decide`, to mark the boundary between the two discharge mechanisms. -/
theorem allBelowSmall : allBelow 20 = true := by decide

#print axioms allBelowSmall

/-- A `Prop` decided natively. This is the shape `ofReduceBool` exists to convert: a `Bool`
    result becomes a proof of the `Prop`. -/
theorem propNatively : (2 : Nat) ^ 16 = 65536 := by native_decide

#print axioms propNatively

/-- A proof with NO evaluator dependency at all — the baseline the loss record must
    distinguish from the above. -/
theorem pure : (2 : Nat) + 2 = 4 := rfl

#print axioms pure

end T11_017
