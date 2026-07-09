# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.10.0] - 2026-07-08


### Added
- `repl` module: `ReplSession`, the replay-based interactive session behind `largo repl` — accumulates definitions + Main statements, re-runs the composed program through `interpret_for_ui_with_args` (the exact `run --interpret` engine), surfaces output past a high-water mark, and rolls back failing inputs. `source()` is always a valid runnable program. Plus `Interpreter::global_bindings` and `ui_bridge::repl_global_bindings` for `:vars` inspection.
- Register bytecode VM (`vm/`): bytecode compiler, dispatch machine, shared semantics-kernel delegation, and `Int` fast paths bit-identical to the kernel by the wrapping-`i64` spec (pinned by an edge-grid differential).
- Tier-up seam to `logicaffeine-jit`: hot functions compile per call with argument guards and kind-inference; hot Main loops region-tier (OSR-lite) with incoming-dead analysis and per-entry guards. The integer/float subset gates tier-up; anything else fails closed to bytecode.
- **The rustc→LOGOS sourcemap is live.** `codegen_program_mapped` records, for every generated line, the top-level LOGOS statement span that produced it (post-hoc newline counting — no cursor threading through emission; function bodies map to their `## To` span; peephole-fused statements map to the merged span), and walks the AST recording variable origins with ownership roles (`GiveObject` beats `ZoneLocal` beats `ShowObject`… — the move is what a borrow error is about). Previously `SourceMapBuilder` was never called: every translated rustc error had `logos_span: None`.
- `rustc_check(source, cache_dir)` — the flycheck engine: an optimizer-OFF compile front (spans stay 1:1; a check build has no use for optimized output) with prelude-offset translation (stdlib prepend never shifts user spans), `cargo check --message-format=json` in a persistent cache dir, and `translate_diagnostics_all` through the REAL interner and populated map. Fail-loud: a cargo failure that translates to nothing is a `Build` error, never a silent all-clear. Plus `rustc_check_artifacts` (the cargo-free substrate for tests) and `write_cargo_project` (extracted from `compile_to_dir`, now shared).
- `translate_diagnostics_all` — every translatable rustc diagnostic, not just the first (`translate_diagnostics` now delegates).
- `OwnershipChecker::check_program_collect` (+ `OwnershipFinding`) and `EscapeChecker::check_program_collect` — every finding reports with its top-level statement index, and ownership findings carry the index of the statement that MOVED the variable (tracked at the four move sites via a central `mark_moved`); after each finding the variable resets to owned so errors don't cascade. Strict `check_program` on both checkers is unchanged.
- `analysis::check_program_collect` + `IndexedTypeError` — collect EVERY failing top-level statement (with its statement index, mapping 1:1 onto `Parser::stmt_spans()`), instead of bailing at the first; inference continues best-effort so one bad statement does not cascade. The strict fail-fast `check_program` contract is unchanged and compile paths keep using it — this is the IDE substrate.
- **The stdlib is literate.** Every definition across the 12 embedded stdlib modules (158 in all — the crypto suite included) carries a `## Note` doc line above its header; `loader::prelude_module_sources()` exposes the RAW sources so the LSP reads the prose, while `prelude()`/`apply_prelude` strip Note blocks byte-exactly — the runtime prelude is identical to the pre-documentation join, and documentation prose can never mint an auto-import trigger (`module_names` derives from the stripped code). En route, the root `assets/std` copies of `env`/`random`/`time` had silently drifted from the canonical compile-tree copies (`## To native args` vs `## To native args ()`); they're re-synced and a byte-parity lock in `logicaffeine_tests` (`stdlib_asset_sync.rs`) ends that drift class.

### Fixed
- `compile_to_dir` appended `## Requires` dependencies after the `[target.'cfg(linux)'.dependencies]` section of the generated Cargo.toml, silently scoping user crates to Linux; they now land inside `[dependencies]`.

### Changed
- Run-path optimizer: magic-reciprocal division/modulo, run-path recursion inlining, and loop-invariant pointer/length plus constant hoisting on the interpreted execution path.

See the root CHANGELOG for the cross-crate history.

## [0.8.18] - 2026-02-15

### Fixed
- Constant propagation no longer substitutes `Literal::Text` (String) values — `is_literal` → `is_propagatable_literal` excludes non-Copy types to preserve use-after-move detection (E0382)
- Zone-scoped `Let` bindings no longer leak into propagation environment — new `propagate_zone_block` processes zone bodies without registering their bindings, preserving escape analysis (E0597)

## [0.8.17] - 2026-02-15

### Added
- C codegen backend (`codegen_c.rs`) — `compile_to_c()` produces self-contained C files with embedded runtime (Seq, Map, Set, string helpers, IO)
- C runtime: `Seq_i64`, `Seq_bool`, `Seq_str`, `Map_i64_i64` (open-addressing), string helpers, IO
- C keyword escaping: `is_c_reserved()` + `escape_c_ident()` → `logos_` prefix for reserved words
- Constant propagation optimizer pass (`optimize/propagate.rs`) — forward substitution of immutable constants
- Pipeline order: fold → propagate → dce
- `is_simple_expr()` guard for for-range pattern — prevents codegen of complex limit expressions

### Fixed
- For-range guard for complex expressions — `While i is at most length of items` no longer produces `_` in generated Rust
- For-range post-loop value for empty loops — empty loops keep counter at start value using `max(start, limit)`
- Vec-fill pattern relaxed mutability — `Let items be a new Seq of Bool` now matches without explicit `mutable`
- C codegen missing `SetI64`/`SetStr` in `c_type_str()` match
- Interpreter `apply_comparison` now handles Float-Float, Int-Float, Float-Int comparisons

## [0.8.16] - 2026-02-15

### Fixed
- For-range loop emission: switched from `RangeInclusive` (`..=`) to exclusive `Range` (`..`) with `limit + 1`. `RangeInclusive` has per-iteration overhead that caused 41.4% regression in bubble sort's O(n^2) inner loop. Literal limits compute `limit + 1` at compile time.

## [0.8.15] - 2026-02-15

### Added
- For-range loop emission: `Let counter + While counter <= limit + Set counter to counter + 1` → `for counter in start..(limit + 1)` (and `start..limit` for exclusive `<` bounds)
- Post-loop counter value restoration for correct semantics after for-range transformation
- `body_modifies_var` helper for detecting counter modification inside loop bodies (prevents invalid for-range optimization)
- Iterator-based loops: `.iter().copied()` for Copy-type `Vec` iteration instead of `.clone()` when body doesn't mutate the collection
- `body_mutates_collection` helper for recursive mutation detection across nested If/While/Repeat/Zone blocks
- List literal element type inference: `[10, 20, 30]` registers as `Vec<i64>` for direct indexing and Copy-type elision
- Vec fill pattern: `BinaryOpKind::Lt` (exclusive bound) support alongside existing `LtEq`
- Swap pattern: `BinaryOpKind::Eq` and `BinaryOpKind::NotEq` support alongside existing comparison operators

## [0.8.14] - 2026-02-15

### Added
- Deep expression recursion in constant folder — all 26 Expr variants now get sub-expressions folded
- Unreachable-after-return DCE — statements after `Return` truncated from blocks
- Algebraic simplification — identity/annihilator rules for int and float (`x + 0`, `x * 1`, `x * 0`, etc.)
- Maybe type support in codegen — `Maybe` handled as alias for `Option` in all 7 codegen paths

## [0.8.13] - 2026-02-14

### Added
- Accumulator introduction optimization for single non-tail recursive calls with `+` or `*`
- Automatic memoization for pure multi-branch recursive functions with hashable parameters
- Mutual tail call optimization merging paired mutually-recursive functions into a single loop
- Purity analysis pass (`collect_pure_functions`) using two-pass fixed-point propagation
- Helper functions: `count_self_calls`, `is_hashable_type`, `detect_mutual_tce_pairs`, `find_tail_call_targets`

## [0.8.12] - 2026-02-14

Synced to workspace version 0.8.12. See root CHANGELOG for full history.

## [0.6.0] - 2026-01-17

Initial crates.io release.

### Added

- LOGOS compilation pipeline
- Code generation from AST to target output
- Tree-walking interpreter (optional `interpreter-only` feature)
- Refinement type syntax with `where` clauses
- DRS (Discourse Representation Structures) for donkey anaphora
- Event adjective analysis
- Escape analysis for memory safety
- Diagnostic system with source mapping
- Optional `codegen` feature for full compilation
- Optional `verification` feature for Z3 integration
