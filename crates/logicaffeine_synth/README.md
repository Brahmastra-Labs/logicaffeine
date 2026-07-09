# logicaffeine-synth

**EXODIA Phase 2 — the Forge's offline proof tooling.** Z3 specifications for
the JIT's integer micro-operations plus a three-way witness harness that grounds
each spec against the real machine code. Here "synth" means *stencil spec /
template synthesis*; it is unrelated to `logicaffeine_verify`'s hardware SVA
synthesis.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace. Excluded from
default-members (Z3-backed, offline tooling); depends on `logicaffeine_forge`
and the `z3` solver crate. Everything runs at development / CI time and is
**never** linked into the runtime path.

## Role in the workspace

`logicaffeine_forge` is the copy-and-patch JIT: it lowers a stream of `MicroOp`s
to machine code through an executable-memory stencil runtime. This crate proves
those micro-ops correct **ahead of time** — it pins each op to an SMT-bitvector
specification and grounds that spec against the real compiled stencil chain. It
consumes `MicroOp`, `compile_straightline`, `reference_eval`, and `ChainOutcome`
from `logicaffeine_forge::jit`; it is invoked only by the test crate's
`phase_exodia_forge`, behind that crate's `verification` feature. Library only —
no binaries, no feature flags of its own.

See [proof-and-verification](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/proof-and-verification.md) for the
verification stack and [execution-and-performance](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/execution-and-performance.md)
for the Forge JIT it validates.

### Why BV64

The specs are SMT BV64 arithmetic, matching the kernel's locked signed `Int`
semantics exactly. `add`/`sub`/`mul` are **exact**: their precondition asserts
the true result fits in signed 64-bit (the 128-bit sign-extended product equals
the sign-extension of the wrapped 64-bit result), so an overflowing op
side-exits (deopt → BigInt promotion) rather than wrapping. `div`/`mod` use
`bvsdiv` / `bvsrem`, which wrap `i64::MIN / -1` to `i64::MIN` and take the
dividend's sign just like `wrapping_div` / `wrapping_rem`. Shifts mask the
amount to the low six bits (`b & 63`) like `wrapping_shl(b as u32)`, and `shr`
is arithmetic (`bvashr`) because `Int` is signed.

## Public API

Re-exported at the crate root: `all_specs`, `OpSpec`, `SpecKind`,
`check_spec_with_witnesses`, `WitnessReport`.

### `spec`

```text
pub enum SpecKind { Binop, Checked }

pub struct OpSpec {
    pub name:   &'static str,
    pub kind:   SpecKind,
    pub build:  fn() -> MicroOp,
    pub pre:    for<'c> fn(&'c Context, &BV<'c>, &BV<'c>) -> Bool<'c>,
    pub result: for<'c> fn(&'c Context, &BV<'c>, &BV<'c>) -> BV<'c>,
}

pub fn all_specs() -> Vec<OpSpec>;
pub fn prove_all_satisfiable() -> Result<usize, String>;
pub fn prove_commutative(name: &'static str) -> Result<(), String>;
pub fn prove_min_div_wraps() -> Result<(), String>;
pub fn deliberately_wrong_spec_for_canary() -> OpSpec;
```

- `SpecKind::Binop` is a total 3-address op (`frame[2] = frame[0] op frame[1]`);
  `SpecKind::Checked` side-exits when the precondition fails.
- `all_specs()` returns the **13** ops: `add`, `sub`, `mul`, `div`, `mod`
  (`Checked`) and `and`, `or`, `xor`, `shl`, `shr`, `lt`, `lteq`, `eq`
  (`Binop`). `pre` and `result` are Z3 closures over `(ctx, a, b)`.
- `prove_all_satisfiable` checks every spec is inhabited (`pre ∧ post` is SAT)
  and returns the count proved.
- `prove_commutative` asserts `¬(f(a,b) = f(b,a))` is UNSAT, returning a Z3
  counterexample model on failure (so `sub`/`div`/… correctly `Err`).
- `prove_min_div_wraps` pins `i64::MIN / -1 == i64::MIN`.
- `deliberately_wrong_spec_for_canary` returns an "add claiming sub" spec the
  witness harness must reject — proof that the three-way comparison can fail.

### `witness`

```text
pub struct WitnessReport { pub spec: &'static str, pub inputs_checked: usize }

pub fn check_spec_with_witnesses(spec: &OpSpec, solver_models: usize)
    -> Result<WitnessReport, String>;
```

Three independent evaluators run every input and must agree: (1) the **real
stencil chain** — actual machine code through `compile_straightline` +
`run_with_frame`; (2) the forge's **`reference_eval`** — the deliberately-dumb
MicroOp interpreter; (3) the **Z3 spec** evaluated on the concrete input. Any
disagreement is a finding (wrong spec, miscompiled stencil, or reference bug).
Inputs are `solver_models` distinct Z3-chosen SAT models (each excluded to force
a fresh one) plus an adversarial 12-value corner battery — `i64::MIN`, `MIN+1`,
-2, -1, 0, 1, 2, 63, 64, 65, `MAX-1`, `MAX` — crossed for 144 pairs. For
`Checked` ops at a precondition-excluded input, all three must side-exit.

## Dependencies

- Internal: `logicaffeine_forge` (`jit::MicroOp`, `compile_straightline`,
  `reference_eval`, `ChainOutcome`).
- External: `z3` 0.12 (the SMT solver). On Linux, building the tests needs
  `Z3_SYS_Z3_HEADER=/usr/include/z3.h`; the gate runs via
  `cargo test -p logicaffeine-tests --features verification --test phase_exodia_forge`.

## License

Business Source License 1.1 — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/CHANGELOG.md)
