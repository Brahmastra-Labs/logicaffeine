# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

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
