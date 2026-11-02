# logicaffeine-verify

Z3-backed verification and model checking: a small verification IR (`VerifyExpr` / `VerifyType`) encoded into
Z3 to decide validity, equivalence, consistency, and temporal safety and liveness, from refinement-type checks
up to a hardware model checker. A test shows a property holds on the inputs you tried; an UNSAT from Z3 shows
it holds on every input the encoding covers (every trace up to the bound, for bounded checks), and a SAT comes
back as a counterexample. Its verdicts are Z3's, not proofs the kernel re-checks.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 0.
Outside `default-members` because it needs a Z3 toolchain; it compiles only when a consumer turns on its
`verification` feature. **Tarski invariant**: the verification IR is decoupled from the main AST, so
verification is front-end-agnostic and this crate depends on no other workspace crate.

## Role in the workspace

This crate is the Z3 oracle behind the project's third trust layer (kernel →
proof engine → SMT/model-checking); see
[proof-and-verification.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/proof-and-verification.md). It is a
workspace member but **not in `default-members`**, so a plain
`cargo build`/`cargo test` skips it and needs no Z3. Its consumers:

- `logicaffeine-proof`: `verification = ["dep:logicaffeine-verify"]`
- `logicaffeine-compile`: `verification = ["dep:logicaffeine-verify", "logicaffeine-proof/verification"]`
- `logicaffeine-cli` (`largo`): `verification`, and `verification-static`, which adds this crate's `static-link-z3`
- `logicaffeine-tv` depends on it unconditionally.

Because the IR carries no reference to the language AST, the language crate never
depends on this one: `logicaffeine_compile` translates `LogicExpr` into
`VerifyExpr` (its `verification` module, behind its `verification` feature). The
model-checking stack backs the **English → SVA/PSL** hardware-verification path.
`check_equivalence` decides whether a generated SVA (for example one an LLM wrote)
expresses the *same* property as the parsed FOL (via `¬(FOL ↔ SVA)` over a
bounded unrolling), and `synthesis` emits SVA/Verilog from an LTL spec.

## Capabilities

- **Validity and refinement**: `VerificationSession` accumulates declarations and
  assumptions, then proves a goal valid (P valid iff ¬P is UNSAT). Three-valued
  throughout: `SolverUnknown` never silently reads as a verdict.
- **Equivalence** (`equivalence`): bounded FOL↔SVA equivalence with
  counterexample traces (`EquivalenceResult`, `Trace`, `CycleState`,
  `SignalValue`).
- **Consistency** (`consistency`): joint satisfiability, MUS-style vacuity,
  redundancy, and pairwise-conflict reporting over a labeled spec.
- **Bounded model checking**: `verify_temporal` unrolls a transition relation
  and checks a property at every state; `AtState`/`Transition` IR nodes.
- **Unbounded and compositional checking**: k-induction (`kinduction`), IC3/PDR (`ic3`), Craig
  interpolation (`interpolation`), liveness-to-safety (`liveness`),
  predicate abstraction + CEGAR (`abstraction`),
  assume-guarantee (`compositional`), multiclock scheduling
  (`multiclock`), parameterized verification (`parameterized`),
  non-interference / taint (`security`), LTL→Büchi (`automata`),
  reactive synthesis → SVA/Verilog (`synthesis`).
- **Tooling**: strategy auto-selection (`strategy`), SMT-LIB2 export
  (`smtlib`), self-certifying proof certificates (`certificate`), result caching
  (`incremental`), and type inference over the IR (`type_infer`).

## Public API

The IR (`ir`):

- `VerifyType`: `Int`, `Bool`, `Object`, `BitVector(u32)`,
  `Array(Box<VerifyType>, Box<VerifyType>)`, `Real`.
- `VerifyOp`: arithmetic `Add`, `Sub`, `Mul`, `Div` (Euclidean, Z3's `div`),
  `FloorDiv` (rounds toward negative infinity, encoded through `to_int` over the
  reals); comparisons `Eq`, `Neq`, `Gt`, `Lt`, `Gte`, `Lte`; logic `And`, `Or`,
  `Implies`.
- `BitVecOp`: `And`, `Or`, `Xor`, `Not`, `Shl`, `Shr` (logical), `AShr`, `Add`,
  `Sub`, `Mul`, `SDiv`, `SRem`, `ULt`, `SLt`, `ULe`, `SLe`, `Eq`.
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

- `Verifier`: low-level single-shot checks with a 10 s Z3 timeout
  (`check_bool`, `check_int_greater_than`, `check_int_less_than`,
  `check_int_equals`, `context() -> VerificationContext`).
- `rename_var_in_expr(expr, from, to)`: total textual substitution over every
  IR variant.

Top-level entry points:

```text
check_equivalence(fol: &VerifyExpr, sva: &VerifyExpr, signals: &[String], bound: usize) -> EquivalenceResult
check_consistency(props: &[VerifyExpr], signals: &[String], bound: usize) -> ConsistencyResult
check_spec_consistency(formulas: &[LabeledFormula], config: &ConsistencyConfig) -> ConsistencyReport
```

Errors (`error`): `VerificationError`, `VerificationErrorKind`
(`ContradictoryAssertion`, `BoundsViolation`, `RefinementViolation`,
`SolverUnknown`, `SolverError`,
`TerminationViolation`), `VerificationResult<T = ()>`, `CounterExample`.

## Build

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

Verification needs no key: LOGOS is open source under the MIT License, and `largo verify` runs Z3 for
anyone with the full build.

## Feature flags

| Feature | Default | Enables |
|---------|---------|---------|
| `static-link-z3` | off | Builds Z3 from source (`z3/vendored`, the `z3-src` crate) and links it statically: the substrate of the release `largo-full` binaries (needs cmake, a C++ toolchain and libclang). Off, the crate links dynamically against the system libz3. |

## Dependencies

- `z3` 0.20: the SMT solver bindings (the only heavyweight dependency, and the
  reason for the default-members exclusion).
- `serde` / `serde_json`: certificate and IR (de)serialization.

No internal workspace dependencies: the Tarski invariant keeps this crate
standalone. The version is lockstep with the workspace.

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_verify/CHANGELOG.md)
