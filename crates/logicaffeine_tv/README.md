# logicaffeine-tv

Translation validation for the LOGOS compiler: instead of proving the compiler correct once, check each
program it translates, the way you might grade every exam instead of certifying the student. The source half
exists today: a Z3-checked symbolic encoder of straight-line `Int`/`Bool` LOGOS plus a determinate
concurrency subset, proven to agree with the tree-walking interpreter program by program. The emitted-Rust
half has no executor yet, so no source ↔ Rust equivalence is proved, and no compile path calls this crate.

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
- `and`/`or`: logical on `Bool`, bitwise (`And`/`Or`) on `Int`; `not`: boolean on `Bool`, bitwise `~` (`XOR u64::MAX`) on `Int`.
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

Public submodules: `symexec` (big-step symbolic execution of the Verifiable Core into `VerifyExpr`), `parse` (arena-lifetime-safe parsing via a callback), `equiv` (Z3 validity), `verdict` (the `SoundnessReport` / `TvError` result types), and `tensor` (a rewrite of the one tensor IR, proven):

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

The crate declares no feature flags and no binary. The encoder-soundness suite lives in `crates/logicaffeine_tests/tests/phase_tv_encoder_sound.rs`, gated on that crate's `verification` feature:

```bash
cargo test -p logicaffeine-tests --features verification --test phase_tv_encoder_sound
```

The rule proofs are `tensor_rule_proof_lock`: its digests re-check without Z3 in the default suite, and
`--features verification` discharges every obligation again.

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_tv/CHANGELOG.md)
