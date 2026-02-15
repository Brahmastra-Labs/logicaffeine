# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.8.15] - 2026-02-15

### Added
- For-range loop emission: `Let counter + While counter <= limit + Set counter to counter + 1` → `for counter in start..=limit` (and `start..limit` for exclusive `<` bounds)
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
