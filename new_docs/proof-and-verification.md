# Proof & verification

LOGOS can do more than translate logic — it can *check* it. There are three layers, increasing in
power and in trust assumptions:

1. **The kernel** — a small, pure type-theory core that *checks* proofs.
2. **The proof engine** — searches for proofs and gives Socratic hints; everything it finds is
   checked by the kernel.
3. **Z3 verification** — an optional SMT/model-checking layer for static verification and hardware
   assertions.

## The kernel

[`logicaffeine_kernel`](../crates/logicaffeine_kernel/README.md) is a Calculus of Constructions type
checker — the trusted base. It is pure: by the **Milner invariant** it has no path to the lexicon
(see [Architecture](architecture.md)). Core surface
([`kernel/src/lib.rs`](../crates/logicaffeine_kernel/src/lib.rs)):

- `Term`, `Universe`, `Literal`, `Context` — the term language and typing context.
- `infer_type`, `is_subtype` — type checking and cumulativity.
- `normalize` — β/δ-reduction to normal form.

It ships decision procedures as modules, each a checkable engine:
[`ring`](../crates/logicaffeine_kernel/src/ring.rs) (polynomial/ring equalities),
[`lia`](../crates/logicaffeine_kernel/src/lia.rs) (linear integer arithmetic),
[`omega`](../crates/logicaffeine_kernel/src/omega.rs) (Presburger/integer arithmetic),
[`cc`](../crates/logicaffeine_kernel/src/cc.rs) (congruence closure),
[`bitvector`](../crates/logicaffeine_kernel/src/bitvector.rs),
[`simp`](../crates/logicaffeine_kernel/src/simp.rs) (simplification), and
[`termination`](../crates/logicaffeine_kernel/src/termination.rs). Proof **certificates** can be
serialized and re-checked independently. Tests: `phase69_kernel_coc`, `phase73_certifier`,
`phase86_kernel_primitives`, and the incompleteness arc (`phase93_diagonal_lemma` …
`phase95_incompleteness`).

## The proof engine

[`logicaffeine_proof`](../crates/logicaffeine_proof/README.md) is a backward-chaining prover over an
arena-independent IR (`ProofExpr` / `ProofTerm`). By the **Liskov invariant** it has no dependency on
the language crate, so it is reusable across front-ends. It produces a proof tree (each `leaf` tagged
with the `InferenceRule` used) and **Socratic hints** (`SocraticHint`, `SuggestedTactic`) that guide
a learner toward the next step rather than just reporting failure. Tests: `phase60_proof_engine`,
`phase61_induction`, `phase96_tactics`, `phase97_deep_induction`.

Beyond backward chaining, the crate ships its own **Z3-free SAT/BMC core**
([`cdcl.rs`](../crates/logicaffeine_proof/src/cdcl.rs)): the `sat` module does CDCL model finding,
equivalence (`prove_equivalence`), and unsat proofs; the `bmc` module does bounded model checking,
k-induction (`prove_invariant`), counterexample search, and vacuity (`check_vacuity`). Because it
needs no external solver, it runs in the browser — it is the engine behind Studio's
[Hardware mode](studio-and-learn.md) in-browser proving (Z3 stays a native-only backend).

A theorem is written in English as a named block of `Given:` premises and a `Prove:` goal:

```logos
## Theorem: Socrates_Mortality_Verified
Given: Socrates is a man.
Given: Every man is mortal.
Prove: Socrates is mortal.
```

Each line is parsed as a sentence (see [Logic mode](logic-mode.md)); the engine then proves the goal
from the premises. The English → `ProofExpr` lowering lives in the language crate
([`proof_convert.rs`](../crates/logicaffeine_language/src/proof_convert.rs)), preserving the Liskov boundary. See
`phase78_e2e_verification` and `phase84_extraction` (extracting proven properties to Rust
assertions).

## Z3 verification (optional, Pro+)

[`logicaffeine_verify`](../crates/logicaffeine_verify/README.md) is an SMT-backed verification and
model-checking engine. By the **Tarski invariant** its IR (`VerifyExpr` / `VerifyType`,
[`verify/src/ir.rs`](../crates/logicaffeine_verify/src/ir.rs)) is decoupled from the main AST, so it
is front-end-agnostic. The crate is **excluded from `default-members`**: a plain build needs no Z3.
It is reached via the `verification` feature and is **license-gated** (Pro+, validated against the
licensing service — [`license.rs`](../crates/logicaffeine_verify/src/license.rs)).

It is far more than an assertion checker. Public modules
([`verify/src/lib.rs`](../crates/logicaffeine_verify/src/lib.rs)) include:

- `equivalence` (`check_equivalence`), `consistency` — tautology/contradiction/equivalence checking.
- `ic3` (IC3/PDR), `kinduction` (k-induction), `interpolation` (Craig interpolation), `liveness`,
  `multiclock`, `parameterized` — a full hardware model-checking stack.
- `solver` (`Verifier`, `VerificationSession`), `smtlib`, `incremental`, `abstraction`, `automata`,
  `strategy`, `synthesis`, `security`.

This backs the **English → SVA/PSL** path (the SVA code generator,
[`compile/src/codegen_sva/`](../crates/logicaffeine_compile/src/codegen_sva/)) for formal hardware
verification. Verification errors are written as Socratic explanations with counter-examples. Run it
with `largo verify` / `largo build --verify` (see [the CLI guide](cli.md)).

Tests: `phase42` (Z3), `phase99_solver`, `phase104_induction_vs_z3`, `phase102_unified_verification`,
and the many `phase_hw_*` hardware-verification suites.

## Translation validation

[`logicaffeine_tv`](../crates/logicaffeine_tv/README.md) closes the loop on the compiler itself: it
uses SMT to prove the emitted Rust is observationally equivalent to the LOGOS source
(`check_encoder_sound`, `summarize_logos`). It depends on both `compile` and `verify` and, like them,
is opt-in.

## Synthesis (EXODIA)

[`logicaffeine_synth`](../crates/logicaffeine_synth/README.md) is offline tooling that uses Z3 to
verify JIT stencil specifications against witness inputs (`all_specs`, `check_spec_with_witnesses`) —
it never runs inside the language at runtime; it validates the JIT back-end ahead of time.

## The trust story

The kernel is the small trusted base; the proof engine, Z3, and translation validation all *produce*
artifacts the kernel (or an independent re-check) can verify. The point: search can be heuristic, but
acceptance is checked.

## See also

- [Logic mode](logic-mode.md) — where the propositions come from
- [Architecture](architecture.md) — the Milner / Liskov / Tarski invariants

---
[Docs index](README.md) · [Root README](../NEW_README.md) · [Changelog](../CHANGELOG.md)
