# logicaffeine-tv

`tv` = **translation validation**: per compile, prove the Rust the logicaffeine compiler emits is *observationally equivalent* to its LOGOS source by symbolically executing into the shared `logicaffeine-verify` semantic domain (bitvectors / booleans) and discharging the equivalence with Z3.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace. Excluded from default-members (Z3-backed); depends on `logicaffeine_compile` and `logicaffeine_verify`.

## Role in the workspace

This is **rung 3–4 (translation validation), not rung 5 (machine-checked proof)** — the trust boundary is the encoders + Z3 + rustc, not a mechanized meta-theorem. See [proof-and-verification.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/proof-and-verification.md) for how the verification rungs fit together.

Today only the **LOGOS source side** has a symbolic executor; there is no Rust-emitter-side executor yet, so the full source↔emitted-Rust equivalence is not closed in code. What runs is the source encoder plus its **meta-soundness oracle**: `check_encoder_sound` runs a program through the tree-walking interpreter (`interpret_program`, the independent ground truth) *and* the symbolic encoder, then proves with Z3 that they agree on the full observable behavior — the ordered `Show` outputs (each slot equality validated) and whether an error was raised. A buggy encoder is caught here, not masked by a downstream equivalence that "proves" two wrong things equal. Out-of-fragment programs are *soundly excluded* (`Unsupported`), never reported as agreement.

### Verifiable Core

Straight-line `Int`/`Bool` programs; anything outside yields `Unsupported(reason)`. `Int` is a 64-bit bitvector (`INT_WIDTH = 64`), so wrapping/overflow matches the interpreter's native `i64`.

- `+ - *` → wrapping `Add`/`Sub`/`Mul`.
- `/` `%` → `SDiv`/`SRem`; divisor `== 0` is OR'd into a sticky `errored` condition (div-by-zero is an *observable error*; outputs are compared only when neither side errored).
- Comparisons: `<` → `SLt`, `>` → swapped `SLt`; `<=`/`>=` → `SLe` (swapped for `>=`).
- `== !=` — Int via bitvector `Eq`, Bool via `iff`.
- `and`/`or` — logical on `Bool`, bitwise (`And`/`Or`) on `Int`; `not` — boolean on `Bool`, bitwise `~` (`XOR u64::MAX`) on `Int`.
- Statements: `Let`, `Set` (target must be in scope), `Show <expr> to show`.
- Punted: `If`/`While`/`Repeat`/`Return`/functions/calls, indexing, lists, interpolated strings, non-Int/Bool literals.

## Public API

```text
// crate root
pub fn summarize_logos(source: &str) -> Result<SymSummary, TvError>;
pub fn check_encoder_sound(source: &str) -> SoundnessReport;
pub use symexec::{SymSummary, SymValue};
pub use verdict::{SoundnessReport, TvError};
```

- `summarize_logos` — parse and symbolically execute the source side into a summary (no optimizer).
- `check_encoder_sound` — the encoder/interpreter cross-check described above.
- `SymSummary { outputs: Vec<SymValue>, errored: VerifyExpr }`; `SymValue` is `Int(VerifyExpr)` (width-64 bitvector) or `Bool(VerifyExpr)`.
- `SoundnessReport` = `Agrees | Disagrees { detail } | Unsupported { reason } | ParseFailed { detail }`; `TvError` = `Parse(ParseError) | Unsupported(String)`.

Public submodules — `symexec` (big-step symbolic execution of the Verifiable Core into `VerifyExpr`), `parse` (arena-lifetime-safe parsing via a callback), `equiv` (Z3 validity), and `verdict` (the `SoundnessReport` / `TvError` result types):

```text
// symexec — big-step symbolic execution of the Verifiable Core into VerifyExpr
pub const INT_WIDTH: u32 = 64;
pub fn execute(stmts: &[Stmt], interner: &Interner) -> Result<SymSummary, Unsupported>;

// parse — arena lifetimes can't escape, so the AST is handed back via callback
pub fn with_program<R>(source: &str, optimize: bool, f: impl FnOnce(&[Stmt], &Interner) -> R)
    -> Result<R, ParseError>;

// equiv — validity via check_equivalence(pred, true): Unsat ⇒ valid, Sat ⇒ counterexample
pub fn prove_valid(pred: &VerifyExpr) -> EquivalenceResult;
pub fn is_valid(pred: &VerifyExpr) -> bool;
```

`with_program(.., optimize = true, ..)` runs the production `optimize_program` pipeline — the hook for validating optimizer output against the source.

### Example

```rust,no_run
use logicaffeine_tv::{check_encoder_sound, summarize_logos, SoundnessReport};

let src = "## Main\nLet x be 2 + 3 * 4.\nShow x.";
assert_eq!(check_encoder_sound(src), SoundnessReport::Agrees);

let summary = summarize_logos(src).unwrap();
assert_eq!(summary.outputs.len(), 1);
```

## Dependencies

- **`logicaffeine_compile`** — parser, interner, arena AST, tree-walking interpreter (`interpret_program`), optimizer (`optimize_program`).
- **`logicaffeine_verify`** — `VerifyExpr`, `BitVecOp`, `EquivalenceResult`, `check_equivalence`; pulls in **Z3** (`z3 = "0.12"`) transitively, which is why this crate is excluded from `default-members` (plain `cargo build`/`cargo test` need no Z3).

The crate declares no feature flags and no binary. The encoder-soundness suite lives in `crates/logicaffeine_tests/tests/phase_tv_encoder_sound.rs`, gated on that crate's `verification` feature:

```bash
cargo test -p logicaffeine-tests --features verification phase_tv_encoder_sound
```

## License

Business Source License 1.1 — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/CHANGELOG.md)
