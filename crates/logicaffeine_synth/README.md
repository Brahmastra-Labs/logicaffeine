# logicaffeine-synth

Z3 specifications for the integer ALU micro-ops of EXODIA, the copy-and-patch JIT, and a harness that checks
the JIT's machine code against them. A miscompiled stencil returns a wrong number without crashing, so the
harness runs Z3-chosen and corner-case inputs through the real stencil, the forge's reference interpreter and
the spec, and any disagreement is a finding. Coverage is the 13 integer ops `all_specs` returns (floats,
memory, calls and control flow have no spec), each checked on those witnesses rather than proved for every
input.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 1.
Outside `default-members` because it needs Z3, and unpublished (`publish = false`): it runs at development and
CI time from the test crate and is never linked into the runtime path. Despite the name, nothing here
synthesizes code, and it is unrelated to `logicaffeine_verify`'s hardware `synthesis`.

## Role in the workspace

`logicaffeine_forge` is the copy-and-patch JIT: it lowers a stream of `MicroOp`s
to machine code through an executable-memory stencil runtime. This crate pins
each integer ALU op to an SMT-bitvector specification ahead of time and grounds
that spec against the real compiled stencil chain. It consumes `MicroOp`,
`compile_straightline`, `reference_eval`, and `ChainOutcome` from
`logicaffeine_forge::jit`; it is invoked only by the test crate's
`phase_exodia_forge` (behind that crate's `verification` feature) and by its
own unit tests. Library only, with no binaries and no feature flags of its own.

See [proof-and-verification](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/proof-and-verification.md) for the
verification stack and [execution-and-performance](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/execution-and-performance.md)
for the JIT it validates.

### Why BV64

The specs are SMT BV64 arithmetic, matching the signed `Int` of the semantics
core (`logicaffeine_compile::semantics`) exactly. `add`/`sub`/`mul` are
**exact**: their precondition asserts the true result fits in signed 64-bit
(the 128-bit sign-extended product equals the sign-extension of the wrapped
64-bit result), so an overflowing op side-exits (deopt → BigInt promotion)
rather than wrapping. `div` is exact the same way: its precondition excludes a
zero divisor and `i64::MIN / -1`, so the overflowing quotient side-exits to the
promoting execution tiers, and on the defined domain `bvsdiv` truncates toward
zero like LOGOS `/`. `mod` excludes a zero divisor and uses `bvsrem`, which
takes the dividend's sign like `wrapping_rem`. Shifts mask the amount to the
low six bits (`b & 63`) like `wrapping_shl(b as u32)`, and `shr` is arithmetic
(`bvashr`) because `Int` is signed.

## Public API

Re-exported at the crate root: `all_specs`, `OpSpec`, `SpecKind`,
`check_spec_with_witnesses`, `WitnessReport`, `all_rev_specs`, `RevSpec`,
`prove_bijective`, `prove_swap_involution`.

### `spec`

```text
pub enum SpecKind { Binop, Checked }

pub struct OpSpec {
    pub name:   &'static str,
    pub kind:   SpecKind,
    pub build:  fn() -> MicroOp,
    pub pre:    fn(&BV, &BV) -> Bool,
    pub result: fn(&BV, &BV) -> BV,
}

pub fn all_specs() -> Vec<OpSpec>;
pub fn prove_spec_satisfiable(spec: &OpSpec) -> Result<(), String>;
pub fn prove_all_satisfiable() -> Result<usize, String>;
pub fn prove_commutative(name: &'static str) -> Result<(), String>;
pub fn prove_min_div_excluded() -> Result<(), String>;
pub fn deliberately_wrong_spec_for_canary() -> OpSpec;
```

- `SpecKind::Binop` is a total 3-address op (`frame[2] = frame[0] op frame[1]`);
  `SpecKind::Checked` side-exits when the precondition fails.
- `all_specs()` returns one spec per op: `add`, `sub`, `mul`, `div`, `mod`
  (`Checked`) and `and`, `or`, `xor`, `shl`, `shr`, `lt`, `lteq`, `eq`
  (`Binop`). `pre` and `result` are Z3 functions of the operands `(a, b)`. The
  rest of `MicroOp`, such as `Gt`, `GtEq`, `Neq`, `Ushr`, the bit counts, float
  arithmetic, memory, calls and control flow, has no spec here.
- `prove_spec_satisfiable` checks one spec is inhabited (`pre ∧ post` is SAT)
  in its own Z3 context; `prove_all_satisfiable` runs it over every spec and
  returns the count proved.
- `prove_commutative` asserts `¬(f(a,b) = f(b,a))` is UNSAT, returning a Z3
  counterexample model on failure (so `sub`/`div`/… correctly `Err`).
- `prove_min_div_excluded` proves `div`'s precondition is UNSAT at
  `(i64::MIN, -1)`: the spec never claims a value where the machine code
  side-exits.
- `deliberately_wrong_spec_for_canary` returns an "add claiming sub" spec the
  witness harness must reject, which proves the three-way comparison can fail.

### `witness`

```text
pub struct WitnessReport { pub spec: &'static str, pub inputs_checked: usize }

pub fn check_spec_with_witnesses(spec: &OpSpec, solver_models: usize)
    -> Result<WitnessReport, String>;
```

Three independent evaluators run every input and must agree: (1) the **real
stencil chain**, actual machine code through `compile_straightline` +
`run_with_frame`; (2) the forge's **`reference_eval`**, the deliberately simple
MicroOp interpreter; (3) the **Z3 spec** evaluated on the concrete input. Any
disagreement is a finding (wrong spec, miscompiled stencil, or reference bug).
Inputs are `solver_models` distinct Z3-chosen SAT models (each excluded to force
a fresh one) plus an adversarial 12-value corner battery (`i64::MIN`, `MIN+1`,
-2, -1, 0, 1, 2, 63, 64, 65, `MAX-1`, `MAX`) crossed for 144 pairs. For
`Checked` ops at a precondition-excluded input, all three must side-exit.

### `reversible`

```text
pub struct RevSpec {
    pub name:    &'static str,
    pub forward: fn(&BV, &BV) -> BV,
    pub inverse: fn(&BV, &BV) -> BV,
}

pub fn all_rev_specs() -> Vec<RevSpec>;
pub fn prove_bijective(spec: &RevSpec) -> Result<(), String>;
pub fn prove_swap_involution() -> Result<(), String>;
```

Offline Z3 bijectivity proofs for the VM's reversible opcodes, each of which
reverses with no history tape because it inverts algebraically. `all_rev_specs`
pairs `RevAdd`/`RevSub`, `RevXor` and `RevNeg` with the inverse the opcode
catalog declares (the pairing `reverse_op_lock` locks structurally), as maps of
the state word `x` and an independent delta `d`. `prove_bijective` discharges
both `∀x,d. G(F(x,d),d) = x` and `∀x,d. F(G(x,d),d) = x` over BV64 by
refutation; `prove_swap_involution` proves `RevSwap` applied twice is the
identity on its pair of words.

## Dependencies

- Internal: `logicaffeine_forge` (`jit::MicroOp`, `compile_straightline`,
  `reference_eval`, `ChainOutcome`).
- External: `z3` 0.20 (the SMT solver). On Linux, building the tests needs
  `Z3_SYS_Z3_HEADER=/usr/include/z3.h`; the gate runs via
  `cargo test -p logicaffeine-tests --features verification --test phase_exodia_forge`.

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_synth/CHANGELOG.md)
