# logicaffeine-verify

A Z3-backed SMT verification and model-checking engine: it encodes a small,
AST-independent verification IR into Z3 and decides validity, equivalence,
consistency, and temporal/safety/liveness properties — from refinement-type
checks up to a full hardware model checker.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace. Excluded from
default-members (requires a Z3 toolchain); reached via the `verification`
feature in dependent crates. **Tarski invariant**: the verification IR
(`VerifyExpr` / `VerifyType`) is decoupled from the main AST, so verification is
front-end-agnostic. License-gated (Pro+).

## Role in the workspace

This crate is the Z3 oracle behind the project's third trust layer (kernel →
proof engine → SMT/model-checking); see
[proof-and-verification.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/proof-and-verification.md). It is a
workspace member but **not in `default-members`**, so a plain
`cargo build`/`cargo test` skips it and needs no Z3. It compiles only when a
consumer turns on its `verification` feature:

- `logicaffeine-proof`: `verification = ["dep:logicaffeine-verify"]`
- `logicaffeine-compile`: `verification = ["dep:logicaffeine-verify", "logicaffeine-proof/verification"]`
- `logicaffeine-tv` depends on it unconditionally.

Because the IR carries no reference to the language AST, the language crate can
depend on the verify crate without a cycle; translation from `LogicExpr` happens
in the compile layer. The model-checking stack backs the **English → SVA/PSL**
hardware-verification path: `check_equivalence` decides whether an LLM-generated
SVA expresses the *same* property as the parsed FOL (via `¬(FOL ↔ SVA)` over a
bounded unrolling), and `synthesis` emits SVA/Verilog from an LTL spec.

## Capabilities

- **Validity & refinement** — `VerificationSession` accumulates declarations and
  assumptions, then proves a goal valid (P valid iff ¬P is UNSAT). Three-valued
  throughout: `SolverUnknown` never silently reads as a verdict.
- **Equivalence** (`equivalence`) — bounded FOL↔SVA equivalence with
  counterexample traces (`EquivalenceResult`, `Trace`, `CycleState`,
  `SignalValue`).
- **Consistency** (`consistency`) — joint satisfiability, MUS-style vacuity,
  redundancy, and pairwise-conflict reporting over a labeled spec.
- **Bounded model checking** — `verify_temporal` unrolls a transition relation
  and checks a property at every state; `AtState`/`Transition` IR nodes.
- **k-induction** (`kinduction`), **IC3/PDR** (`ic3`), **Craig interpolation**
  (`interpolation`), **liveness-to-safety** (`liveness`),
  **predicate abstraction + CEGAR** (`abstraction`),
  **assume-guarantee** (`compositional`), **multiclock** scheduling
  (`multiclock`), **parameterized** verification (`parameterized`),
  **non-interference / taint** (`security`), **LTL→Büchi** (`automata`),
  **reactive synthesis → SVA/Verilog** (`synthesis`).
- **Tooling** — strategy auto-selection (`strategy`), SMT-LIB2 export
  (`smtlib`), self-certifying proof certificates (`certificate`), result caching
  (`incremental`), and type inference over the IR (`type_infer`).

## Public API

The IR (`ir`):

- `VerifyType`: `Int`, `Bool`, `Object`, `BitVector(u32)`,
  `Array(Box<VerifyType>, Box<VerifyType>)`, `Real`.
- `VerifyOp` (13 arithmetic/comparison/logic ops) and `BitVecOp` (17 bitwise /
  shift / arithmetic / comparison ops).
- `VerifyExpr`: `Int`, `Bool`, `Var`, `Binary`, `Not`, `ForAll`/`Exists`,
  `Apply` (predicate, `Int^n → Bool`), `ApplyInt` (term function, `Int^n → Int`),
  `BitVecConst`/`BitVecBinary`/`BitVecExtract`/`BitVecConcat`, `AtState`/
  `Transition` (BMC), `Select`/`Store` (arrays), `Iff`. Builders: `var`, `int`,
  `bool`, `binary`, `not`, `apply`, `apply_int`, `forall`, `exists`,
  `eq`/`neq`/`gt`/`lt`/`gte`/`lte`, `and`/`or`/`implies`, `bv_const`,
  `bv_binary`, `iff`.

Solvers (`solver`):

```text
let mut session = VerificationSession::new();
session.declare("x", VerifyType::Int);                                   // (&mut self, &str, VerifyType)
session.assume(&VerifyExpr::eq(VerifyExpr::var("x"), VerifyExpr::int(10)));
session.verify(&VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(5)))?; // -> VerificationResult

session.check_sat()?;                          // -> Result<bool, VerificationError>  (three-valued)
session.verify_with_binding(name, ty, value, predicate)?;  // refinement check, scoped binding
session.verify_temporal(initial, transition, property, bound)?;  // BMC
```

- `Verifier` — low-level single-shot checks with a 10 s Z3 timeout
  (`check_bool`, `check_int_greater_than`, `check_int_less_than`,
  `check_int_equals`, `context() -> VerificationContext`).
- `rename_var_in_expr(expr, from, to)` — total textual substitution over every
  IR variant.

Top-level entry points:

```text
check_equivalence(fol: &VerifyExpr, sva: &VerifyExpr, signals: &[String], bound: usize) -> EquivalenceResult
check_consistency(props: &[VerifyExpr], signals: &[String], bound: usize) -> ConsistencyResult
check_spec_consistency(formulas: &[LabeledFormula], config: &ConsistencyConfig) -> ConsistencyReport
```

Errors (`error`): `VerificationError`, `VerificationErrorKind`
(`ContradictoryAssertion`, `BoundsViolation`, `RefinementViolation`,
`License{Required,Invalid,InsufficientPlan}`, `SolverUnknown`, `SolverError`,
`TerminationViolation`), `VerificationResult<T = ()>`, `CounterExample`.

Licensing (`license`):

```text
let validator = LicenseValidator::new();
let plan: LicensePlan = validator.validate("sub_…")?;   // -> VerificationResult<LicensePlan>
if plan.can_verify() { /* Pro, Premium, Lifetime, Enterprise */ }
```

## Build & licensing

The `z3` bindings need a Z3 toolchain installed at build time:

```bash
# Linux
sudo apt install z3 libz3-dev
export Z3_SYS_Z3_HEADER=/usr/include/z3.h

# macOS
brew install z3
export Z3_SYS_Z3_HEADER=/opt/homebrew/include/z3.h
export BINDGEN_EXTRA_CLANG_ARGS="-I/opt/homebrew/include"
export LIBRARY_PATH="/opt/homebrew/lib"

cargo build -p logicaffeine-verify
```

Verification is license-gated. Keys are Stripe subscription IDs (`sub_` prefix).
`LicenseValidator::validate` (1) rejects malformed keys, (2) returns a fresh
cached result (< 24 h) when available, (3) otherwise POSTs
`{ "licenseKey": "<key>" }` to `https://api.logicaffeine.com/validate`, and
(4) falls back to stale cache when the network is down. Results cache at
`{cache_dir}/logos/verification_license.json`. Only `Pro`, `Premium`,
`Lifetime`, and `Enterprise` return `can_verify() == true`.

## Feature flags

| Feature | Effect |
|---------|--------|
| `static-link-z3` | Build Z3 from the source vendored in z3-sys and link it statically — the substrate of the release `largo-full` binaries (needs cmake + a C++ toolchain + libclang). Default stays dynamic against the system libz3. |

## Dependencies

- `z3` 0.12 — the SMT solver bindings (the only heavyweight dependency, and the
  reason for the default-members exclusion).
- `serde` / `serde_json` — license-cache (de)serialization.
- `ureq` — HTTP client for license validation.
- `dirs` — locating the system cache directory.

No internal workspace dependencies — the Tarski invariant keeps this crate
standalone. The version is lockstep with the workspace.

## License

Business Source License 1.1 — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/CHANGELOG.md)
