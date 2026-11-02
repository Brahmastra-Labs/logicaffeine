# Coverage Baseline — Tier-A Correctness Core

First real measurement, produced by `scripts/test/run-coverage.sh` (cargo-llvm-cov). This is the
**isolation** view — each crate measured by its OWN tests only — which is what surfaces the
"under-tested in isolation" gap the plan targets. The full-suite view (integration tests
included) is a separate, larger run.

Measured: 2026-07-12. Toolchain: cargo-llvm-cov (region / function / line coverage).

## Tier-A core (kernel + base[crypto] + data[wire]), by their own tests

| Surface | Region % | Line % | Note |
|---|---:|---:|---|
| **TOTAL (kernel+base+data)** | **77.82%** | **74.52%** | isolation |
| `data/src/wire.rs` (wire codec) | 95.52% | 98.42% | strong — Tier-A |
| `kernel/src/type_checker.rs` | 86.28% | 84.52% | trusted core |
| `kernel/src/recheck.rs` (2nd checker) | 80.50% | 79.20% | trusted core |
| `kernel/src/eval.rs` | 94.50% | 93.87% | |
| `kernel/src/prelude.rs` | 99.90% | 99.90% | |
| `kernel/src/field_algebra.rs` / `word_ring.rs` | 100% | 100% | |
| `kernel/src/cc.rs` | 93.94% | 88.61% | |
| `kernel/src/termination.rs` | 90.63% | 92.16% | |

## Findings (drive Wave 3 floors + Wave 4 mutation scope)

- **`kernel/src/reduction.rs` = 12.4% region in ISOLATION** (4,802 / 5,481 regions uncovered
  by the kernel's own unit tests). It is heavily exercised by the *integration* suite and the
  Lean-parity tests, NOT the kernel's unit tests. **Implication for Wave 4:** mutation of
  `reduction.rs` must run with the broader test set in scope, or its mutants will mostly
  SURVIVE for lack of covering tests. This is the single most important scoping note for the
  mutation campaign.
- `kernel/src/error.rs` (0%), `interface/error.rs` (6%) — error-formatting paths untested in
  isolation; low priority (presentation, not correctness).
- `kernel/src/positivity.rs` (59%), `term.rs` (62%), `reify.rs` (60%) — moderate isolation
  gaps in the trusted core worth unit-test backfill before mutation.

## Next

1. Full-suite coverage run (integration tests included) → real per-file numbers for the
   integration-covered files (esp. `reduction.rs`), the input for `.config/coverage/thresholds.toml`.
2. Commit per-crate floors at measured levels (monotone) + `coverage_lock.rs` (Wave 3).
