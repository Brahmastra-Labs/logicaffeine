# logicaffeine-tv

Translation validation for the LOGOS compiler: instead of proving the compiler correct once, check each
program it translates, the way you might grade every exam instead of certifying the student. `bytecode` proves,
for every input, that each function the register VM runs computes what its source computes, loops included, and
refutes a miscompile with an input that shows it; `rust` proves the same of the Rust the AOT backend emits, up to a
loud integer overflow where a value needs more than 64 bits. The source encoder is a Z3-checked symbolic encoder of
straight-line `Int`/`Bool` LOGOS plus a determinate concurrency subset, proven to agree with the tree-walking
interpreter program by program. No compile path calls this crate.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 5.
Outside `default-members` because it needs Z3 (through `logicaffeine_verify`); its only dependent is
`logicaffeine_tests`.

## Role in the workspace

Its guarantee is per program and rests on the encoder and Z3, not on a mechanized proof about the compiler:
translation validation, not a machine-checked compiler. See [proof-and-verification.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/proof-and-verification.md) for how the verification layers fit together.

What runs is the source encoder plus its **meta-soundness oracle**. `check_encoder_sound` runs a program through the tree-walking interpreter (`interpret_program`, the independent ground truth) *and* the symbolic encoder, then proves with Z3 that they agree on the full observable behavior: the ordered `Show` outputs (each slot equality validated) and whether an error was raised. A buggy encoder is caught here, not masked by a downstream equivalence that "proves" two wrong things equal. Out-of-fragment programs are *soundly excluded* (`Unsupported`), never reported as agreement.

`check_encoder_sound` first classifies the program's determinacy (`logicaffeine_compile::classify_source`). A determinate program is encoded under one schedule, which is canonical because its output is schedule-independent. A nondeterminate program (a `Select`) is validated by seeded replay: for each seed of a fixed sweep, the seeded encoder (`symexec::execute_seeded`) is proved equal to the seeded tree-walker (`run_treewalker_concurrent_seeded`), both drawing the `Select` winner from the same SplitMix64 sequence.

### Verifiable Core

Straight-line `Int`/`Bool` programs plus a determinate concurrency fragment; anything outside yields `Unsupported(reason)`. `Int` is a 64-bit bitvector (`INT_WIDTH = 64`) with wrapping arithmetic. The interpreter promotes an overflowing `Int` to a big integer instead, so on a program whose arithmetic overflows `i64` the two do not agree: the check reports `Disagrees`, or `Unsupported` when a shown value falls outside `i64`. It never reports agreement.

- `+ - *` → wrapping `Add`/`Sub`/`Mul`.
- `/` `%` → `SDiv`/`SRem`; divisor `== 0` is OR'd into a sticky `errored` condition (div-by-zero is an *observable error*; outputs are compared only when neither side errored).
- Comparisons: `<` → `SLt`, `>` → swapped `SLt`; `<=`/`>=` → `SLe` (swapped for `>=`).
- `== !=`: Int via bitvector `Eq`, Bool via `iff`.
- `and`/`or`: the truth of each operand (an `Int` is true when it is not zero), a `Bool` result, and the right operand's errors counted only when the left does not decide; `not`: boolean on `Bool`, bitwise `~` (`XOR u64::MAX`) on `Int`.
- Statements: `Let`, `Set` (target must be in scope), `Show <expr> to show`.
- Concurrency: `Let ch be a Pipe of T`, `Send … into ch`, `Receive x from ch` (a receive on an empty channel is `Unsupported`), `Launch a task to f with args` (the task's body runs to completion at its spawn point), `Attempt all of the following:` / `Simultaneously:` blocks run in order, and `Await the first of:` in seeded mode only (the winner is drawn from the ready receive arms; with none ready the timeout arm fires).
- Not modeled: `If`/`While`/`Repeat`/`Return`/`Inspect`/calls, indexing, lists, interpolated strings, non-Int/Bool literals.

## Public API

```text
// crate root
pub fn summarize_logos(source: &str) -> Result<SymSummary, TvError>;
pub fn check_encoder_sound(source: &str) -> SoundnessReport;
pub use symexec::{SymSummary, SymValue};
pub use verdict::{SoundnessReport, TvError};
```

- `summarize_logos`: parse and symbolically execute the source side into a summary (no optimizer).
- `check_encoder_sound`: the encoder/interpreter cross-check described above.
- `SymSummary { outputs: Vec<SymValue>, errored: VerifyExpr }`; `SymValue` is `Int(VerifyExpr)` (width-64 bitvector), `Bool(VerifyExpr)`, or `Chan(usize)` (a channel handle, not an SMT value).
- `SoundnessReport` = `Agrees | Disagrees { detail } | Unsupported { reason } | ParseFailed { detail } | SeedReplayAgrees | SeedReplayDisagrees { detail }`; `TvError` = `Parse(ParseError) | Unsupported(String)`.

Public submodules: `symexec` (big-step symbolic execution of the Verifiable Core into `VerifyExpr`), `parse` (arena-lifetime-safe parsing via a callback), `equiv` (Z3 validity), `verdict` (the `SoundnessReport` / `TvError` result types), `tensor` (a rewrite of the one tensor IR, proven), `bytecode` (the VM's bytecode validated against its source, below),
and `rust` (the emitted Rust validated the same way):

```text
// symexec: big-step symbolic execution of the Verifiable Core into VerifyExpr
pub const INT_WIDTH: u32 = 64;
pub struct Unsupported(pub String);
pub fn execute(stmts: &[Stmt], interner: &Interner) -> Result<SymSummary, Unsupported>;
pub fn execute_seeded(stmts: &[Stmt], interner: &Interner, seed: u64) -> Result<SymSummary, Unsupported>;

// parse: arena lifetimes can't escape, so the AST is handed back via callback
pub fn with_program<R>(source: &str, optimize: bool, f: impl FnOnce(&[Stmt], &Interner) -> R)
    -> Result<R, ParseError>;

// equiv: validity via check_equivalence(pred, true): Unsat ⇒ valid, Sat ⇒ counterexample
pub fn prove_valid(pred: &VerifyExpr) -> EquivalenceResult;
pub fn is_valid(pred: &VerifyExpr) -> bool;

// tensor: discharge an obligation logicaffeine_compile::tensor_graph::smt prints for two graphs
pub enum Verdict { Proven, Refuted(String), Vacuous, Unknown(String) }
pub fn discharge(o: &Obligation, timeout_ms: u32) -> Verdict;
```

`tensor` proves the shared fusion optimizer's rewrites. `tensor_graph::smt` (in `logicaffeine_compile`, no Z3) states
what a graph of the one IR computes as terms over arrays: the ops that move values as the index maps they are, every
other op as a function left open, an in-place write as a store into a range of rows or units. An obligation says the
rewritten graph computes every tensor both graphs keep as the reference does, for every input and every meaning of the
open ops; `smt::identity` computes products, elementwise sums and products and scales exactly, for algebraic identities
at a shape. Terms both graphs compute alike need no solver; what is left is a few lemmas, each a pair of terms that
differ in more than the operands of an open function, defined down to the subterms both share. `discharge` asks Z3,
for each lemma, that its premises be satisfiable (`Vacuous` otherwise) and that premises and goal together not be
(`Proven`), or returns the counterexample (`Refuted`).

`with_program(.., optimize = true, ..)` runs the production `optimize_program` pipeline: the hook for validating optimizer output against the source.

### `rust`: the emitted Rust, validated

```text
pub fn validate(source: &str) -> Result<RustReport, String>;
pub fn validate_edited(source: &str, edit: impl FnOnce(&mut String)) -> Result<RustReport, String>;
pub fn check_executor(source: &str) -> ExecutorCheck;
pub fn lower_rust(text: &str) -> Result<RProgram, String>;
```

`validate` holds every function `compile_to_rust` emits to its source with `bytecode`'s proof, the Rust as the
target. The emitted program is parsed with `syn`, so its layout never matters. Its operations are Rust's in a release
build: `i64` `+ - *` wrap, `/` and `%` raise on a zero divisor and stop on `i64::MIN / -1`. The runtime helpers are
taken as `logicaffeine_data::ops` defines them: `logos_add_i64` and its kin stop with "Integer overflow" when the
exact result leaves `i64`, the `*_exact` helpers are exact, and `expect_i64` stops when its value leaves `i64`. A stop
is a raise when the source raises too and an overflow when only a machine word's width caused it, kept in evaluation
order. A function whose Rust can stop on an overflow is `ProvenUpToOverflow`: on every input it shows what the source
shows and ends as it ends, or stops loudly where the source holds a value past 64 bits, and it never computes a wrong
answer. A silent wrap is refuted with the input that shows it. The emitter's guarded fast paths, its loops versioned
on a guard (one source loop paired with each Rust copy), and the loop a tail call becomes are proved as written, and so
is a loop the emitter replaces with its closed form (below). A recursion it memoizes is `Unsupported`. An `i64`
parameter is assumed to hold an `i64` value: a caller holding a larger one stops before the call, and the caller's
proof covers that stop. `check_executor` holds the Rust executor to the tree-walker on `## Main`.

### `bytecode`: the VM's bytecode, validated

```text
pub enum Target { Vm, Run }
pub enum Verdict {
    Proven { invariants: Vec<Vec<String>>, assumed: Vec<String> },
    Refuted { input: String, source: String, bytecode: String, at: String },
    Unproven { at: String, state: String },
    Unsupported { reason: String },
    Unknown { reason: String },
}
pub fn compile(source: &str, target: Target) -> Result<(CompiledProgram, Vec<String>), String>;
pub fn validate(source: &str, target: Target) -> Result<BytecodeReport, String>;
pub fn validate_edited(source: &str, target: Target, edit: impl FnOnce(&mut CompiledProgram, &[String]))
    -> Result<BytecodeReport, String>;
pub fn check_executors(source: &str, target: Target) -> ExecutorCheck;
```

`validate` checks `## Main` and every function against the `CompiledProgram` a target runs: `Target::Vm` is the
program as written, compiled with the oracle's facts (what the engine differentials run); `Target::Run` is the
program after the run-path optimizer (what `largo run` runs). The source side (`bytecode::source`) lowers each
function to a control-flow graph whose meaning is the tree-walker's. The bytecode side (`bytecode::machine`) takes
each op as the VM's handler computes it, for every value: `MagicDivU` as the reciprocal sequence computes it on the
`u64` reading of its dividend, `BitAnd` as two's complement on `i64`s, with the runtime's mask rule for a big
integer. A wrong oracle fact therefore shows up as a disagreement. Both sides run symbolically over exact integers,
since the runtime promotes an overflowing `i64` instead of wrapping.

The proof (`bytecode::product`) pairs the two loop nests at their heads. Houdini infers which registers equal which
variables (or hold a constant) at each head, keeping only what every pair of paths re-establishes. Beside those
candidates it offers what concrete runs of the source suggest (`product::sample`): the source runs on sampled inputs,
and at each head the polynomial relations among its integers and the inputs (an exact null space over the rationals,
searched by increasing degree, a relation's multiples not offered again) and the bounds the runs kept on each integer
and on each difference of two are candidates too, and so is every comparison over the inputs alone that a path tests
(a guard the compiled code checked before a loop is still known inside it); a candidate holds on the runs it came
from, and Houdini keeps only the inductive ones. The invariant is reported without the conjuncts the others imply. Z3
then shows
that every pair of paths that can run together ends alike, at the same head, returning the same value, raising,
or halting, having shown the same values. A loop is proved for every trip count, not unrolled. A loop-free callee
runs in place on each side; a looping or recursive one is an uninterpreted function of its arguments, named in
`assumed`. A failed obligation becomes `Refuted` only when some input, replayed concretely through both executors,
shows the two apart. Otherwise it is `Unproven`: the state may be unreachable, or depend on an uninterpreted
callee. `validate_edited` validates the bytecode a seeded edit leaves, which the lock uses to show a proof can
fail. `check_executors` holds both executors to the engines they model: run concretely on `## Main`, the source
executor must show what the tree-walker shows and the bytecode executor what the VM shows.

A target with no loop where the source has some computes them in closed form (`product::closed`): `n·(n+1)/2` behind a
guard, Faulhaber's sums of squares and cubes, a `## Main` folded to the constant it prints. Its run is a handful of
paths, each a condition on the inputs and the outcome under it, and each is proved against the source on its own: the
source is cut at its loop heads, the invariant at a head relates its state to the inputs, and every obligation assumes
the path's condition, which holds for the whole run. The sampled runs are filed by the path their inputs take, and a
path no sampled input takes is probed at the widths compiled code guards on (`2³¹ + 1`, `2⁶²`, the rims of `i64`), so
a bound one path keeps and another breaks is kept only where it is inductive. For the Gauss sum the invariant is
`n = n₀`, `i² = 2·total + i`, `i ≥ 1`, `total − i ≥ -1` and `i − n₀ ≤ 1` where the target takes `n₀ ≥ 1`, and the
relation with `total = 0` and `i = 1` where it takes `n₀ < 1`; the loop's exit turns the first into the closed form. The target always ends, so the source
must too: `i − n₀ ≤ 1` bounds a quantity every iteration raises, so the loop is left on every run (loops that reach each
other would need an argument across both, and are refused). A division a polynomial always meets exactly
(`n·(n+1) div 2`) is given to Z3 as a lemma when a goal needs it, decided by evaluating the polynomial at every residue
of its variables (`product::divides`).

Z3's work on each query is bounded in its resource units, not in time, so a verdict does not depend on how loaded the
machine is. A query the incremental engine gives up on is decided again by a fresh solver given the same assertions at
once: simplification, the elimination of solved equations, then the SMT core. Z3's default tactic for nonlinear
integers is not used, since it ignores the resource limit. A wall-clock limit of a minute is kept only against work
Z3 does without counting it (simplifying a remainder by 2⁶⁴ of a nonlinear product). Each validation runs its proofs in
a Z3 context of its own, so a verdict does not depend on what the process asked Z3 before.

### Example

```rust,no_run
use logicaffeine_tv::{check_encoder_sound, summarize_logos, SoundnessReport};

let src = "## Main\nLet x be 2 + 3 * 4.\nShow x.";
assert_eq!(check_encoder_sound(src), SoundnessReport::Agrees);

let summary = summarize_logos(src).unwrap();
assert_eq!(summary.outputs.len(), 1);
```

## Dependencies

- **`logicaffeine_compile`**: parser, interner, arena AST, tree-walking interpreter (`interpret_program`), optimizer (`optimize_program`).
- **`logicaffeine_verify`**: `VerifyExpr`, `BitVecOp`, `EquivalenceResult`, `check_equivalence`; pulls in **Z3** (`z3 = "0.20"`) transitively, which is why this crate is excluded from `default-members` (plain `cargo build`/`cargo test` need no Z3).
- **`z3`** directly, for `tensor`: it loads an obligation's SMT-LIB text into a solver.

The crate declares no feature flags and no binary. The encoder-soundness suite lives in `crates/logicaffeine_tests/tests/phase_tv_encoder_sound.rs` and the bytecode validator's in `bytecode_tv_lock.rs`, the Rust validator's in `rust_tv_lock.rs`, all gated on that crate's `verification` feature:

```bash
cargo test -p logicaffeine-tests --features verification --test phase_tv_encoder_sound --test bytecode_tv_lock \
    --test rust_tv_lock
```

The rule proofs are `tensor_rule_proof_lock`: its digests re-check without Z3 in the default suite, and
`--features verification` discharges every obligation again.

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_tv/CHANGELOG.md)
