# Test Suite Performance: ProxApi-Native Distributed Test Runner

## Executive Summary

The LogicAffeine test suite compiles **235 integration test files** as separate binaries, resulting in 235 independent linker invocations. Combined with ~694 unit tests across 12 crates, the workspace contains **~3,700 test functions**.

**Current bottlenecks:**
- **235 test binaries** in `logicaffeine_tests` — each links the full dependency graph
- **No linker optimization** — no mold, no `.cargo/config.toml`, using platform default ld
- **No build profile tuning** — debug profile uses defaults (full debuginfo, no split, no thin LTO)
- **No nextest** — using `cargo test` which runs tests serially within each binary
- **60 e2e-harness tests** spawn `cargo run` subprocesses, each compiling a fresh Rust project
- **`logicaffeine_web` pulled into all 235 test binaries** — Dioxus/WASM tooling linked into every binary even though only 3 files use it
- **CI runs on a single machine** with no parallelization

**Target:** N-way distributed test execution via ProxApi. Build once, create N LXC workers on the Proxmox cluster, partition and run tests in parallel, aggregate results. No custom coordinator or worker daemon — ProxApi already provides runner lifecycle, shell sessions, task tracking, and cluster-aware scheduling.

### Current Numbers

| Metric | Value |
|--------|-------|
| Integration test files (`logicaffeine_tests`) | 235 |
| Test functions (integration) | ~3,011 |
| Test functions (unit, across 12 crates) | ~694 |
| **Total test functions** | **~3,705** |
| Files using e2e subprocess harness (`mod common`) | 60 |
| Files importing `logicaffeine_web` | 3 |
| Files using `extraction_common` | 1 |
| CI runner | `ubuntu-latest-m` (single) |
| Linker | Platform default (Apple ld on macOS, GNU ld on Linux) |
| `.cargo/config.toml` | Does not exist |
| `rust-toolchain.toml` | Does not exist |
| nextest | Not installed |

---

## Architecture Overview

```
Dev Machine (Mac Mini)
  │
  │  1. cargo nextest archive → tests.tar.zst
  │  2. cargo test-remote --workers 3 --archive tests.tar.zst
  │
  ▼
ProxApi Server (:3000)                    ← already running
  │
  ├─ POST /v1/runners × N                ← create test workers (LXC, least-loaded scheduling)
  ├─ POST /v1/runners/{name}/shell       ← persistent tmux session per worker
  ├─ POST /v1/shell/{id}/exec            ← upload archive, run nextest partition
  ├─ POST /v1/tasks                      ← track partitions as DAG tasks
  ├─ PUT  /v1/tasks/{id}/complete        ← report results
  │
  ├─ pve-01 ── worker-1 (LXC, 4 cores)
  ├─ pve-02 ── worker-2 (LXC, 4 cores)
  ├─ pve-03 ── worker-3 (LXC, 4 cores)
  └─ ...
```

ProxApi replaces everything that would otherwise require building from scratch:

| Without ProxApi | ProxApi Equivalent |
|-----------------|-------------------|
| Custom coordinator HTTP server | ProxApi server (:3000) |
| Custom worker daemon + systemd | `POST /v1/runners` (LXC containers) |
| Worker registration + heartbeat | Runner status tracking + reaper |
| SSH exec boilerplate | `POST /v1/runners/{name}/exec` |
| Persistent shell sessions | `POST /v1/runners/{name}/shell` + tmux |
| Task assignment + tracking | Task DAG (`POST /v1/tasks`, claim/complete/fail) |
| Cluster node scheduling | Least-loaded scheduler |
| Template sync + provisioning | `POST /v1/sync/trigger` + app templates |
| State persistence | JSON auto-persist every 60s |
| Ansible + cloud-init + LXC templates | ProxApi handles all of this |

This architecture works for local dev (run from Mac Mini), Proxmox homelab (distributed across 5 nodes), and CI (GitHub Actions self-hosted runner invoking the same flow).

---

## Phase 1: Foundation

Prerequisites for everything else. No ProxApi involvement — these are local build and tooling improvements.

### 1.1 Binary Consolidation (235 → 8)

The most impactful single change. Every `.rs` file in `tests/` compiles as its own binary. Each binary links the full dependency graph including `logicaffeine_web` (Dioxus). Consolidating into grouped binaries eliminates ~227 redundant link invocations.

**Proposed grouping:**

| Binary | Files | Description |
|--------|-------|-------------|
| `e2e_subprocess.rs` | 34 | `e2e_*` files — spawn `cargo run` subprocesses |
| `e2e_harness.rs` | 26 | Other `mod common` users — e2e harness but not `e2e_*` prefix |
| `linguistics.rs` | 42 | Phase 1-20, linguistic phenomena (FOL, tense, aspect, plurality) |
| `compiler.rs` | 51 | Phase 21-45, compiler pipeline (codegen, types, structs, iteration) |
| `proof_kernel.rs` | 47 | Phase 60-100+, proof engine, kernel, verification |
| `data_crdt.rs` | 15 | CRDT data-layer + temporal tests (no harness) |
| `literate_proofs.rs` | 16 | Literate proofs, tactic automation, verification |
| `misc.rs` | 3+1 | Web-dependent tests + `phase84_extraction_e2e` |

**How to consolidate:** Convert each file into a module. For example, `e2e_collections.rs` becomes a `mod e2e_collections;` inside `e2e_subprocess.rs`.

```
tests/
├── e2e_subprocess.rs            # mod e2e_collections; mod e2e_maps; ...
├── e2e_subprocess/
│   ├── e2e_collections.rs       # (renamed from tests/e2e_collections.rs)
│   ├── e2e_maps.rs
│   └── ...
├── linguistics.rs               # mod phase1_garden_path; mod phase2_polarity; ...
├── linguistics/
│   ├── phase1_garden_path.rs
│   └── ...
└── ...
```

Each top-level `.rs` file becomes the binary. Each subdirectory module is included via `mod`. All 235 files stay as-is content-wise — only the directory structure changes.

**Impact:** 235 link invocations → 8. With mold + profile optimizations, this alone could cut full-suite compile time by 60-80%.

**Risk:** Tests that previously ran in complete isolation now share a process. Any test using global mutable state (unlikely given the codebase) could interfere. Run the full suite before and after to verify.

**See Appendix A** for the complete file-to-binary classification.

### 1.2 Build Profile Optimizations

Add to workspace `Cargo.toml`:

```toml
[profile.dev]
debug = "line-tables-only"    # ~30-40% smaller binaries, much faster linking
split-debuginfo = "unpacked"  # macOS: skip dSYM bundling (huge win on Apple silicon)

[profile.test]
debug = "line-tables-only"
split-debuginfo = "unpacked"
```

Full debuginfo (`debug = 2`, the default) generates enormous `.debug_*` sections that the linker must process. `line-tables-only` preserves file:line info for backtraces but skips variable/type metadata. On macOS, `split-debuginfo = "unpacked"` avoids the expensive `dsymutil` pass.

**Impact:** 30-50% faster incremental link times. Zero risk — backtraces still work.

### 1.3 Install mold Linker

The linker is the single biggest bottleneck when compiling 235 separate test binaries. mold is 5-12x faster than GNU ld and 2-5x faster than Apple ld.

**macOS (via sold, mold's macOS port):**
```bash
brew install sold
```

**Linux (CI and Proxmox workers):**
```bash
sudo apt-get install -y mold
```

**Create `.cargo/config.toml`:**

```toml
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=mold"]

[target.aarch64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=mold"]

[target.aarch64-apple-darwin]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=/opt/homebrew/bin/ld64.mold"]

[target.x86_64-apple-darwin]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=/usr/local/bin/ld64.mold"]
```

**Impact:** With 235 binaries, each saved ~1-3 seconds of link time means 4-12 minutes saved on a full test build.

### 1.4 cargo-nextest as Test Runner

nextest runs test binaries in parallel at the test level, not just the binary level. It provides better output, retries, and — critically — archive support and hash-based partitioning for distributed execution.

```bash
cargo install cargo-nextest
```

Replace `cargo test` with:
```bash
cargo nextest run
```

For the current `--skip e2e` workflow:
```bash
cargo nextest run -E 'not test(e2e)'
```

**Key capabilities:**
- Runs each test in its own process (better parallelism, no shared state issues)
- Cancels remaining tests on first failure (fail-fast)
- Retries flaky tests automatically
- Machine-readable output (JUnit XML for CI)
- **Archive support:** build once, ship binary, run elsewhere (Phase 2 depends on this)
- **Hash partitioning:** `--partition hash:1/3` for deterministic N-way splits

**Impact:** 2-4x faster test execution on multi-core machines.

### 1.5 Web Dependency Isolation

`logicaffeine_web` is a dev-dependency that pulls Dioxus/WASM tooling into the link graph of all 235 test binaries. Only 3 files actually use it: `learn_state_tests.rs`, `unlock_logic_tests.rs`, `struggle_tests.rs`.

**Option A — Feature-gate:**
```toml
[features]
web-tests = ["logicaffeine-web"]

[dev-dependencies]
logicaffeine-web = { path = "../../apps/logicaffeine_web", optional = true }
```

Then `#[cfg(feature = "web-tests")]` on those 3 files. Default test runs skip them.

**Option B — Separate crate:**
Create `crates/logicaffeine_web_tests/` with its own `Cargo.toml`. The main test crate no longer links web at all.

**Impact:** Removing Dioxus from the link graph saves 5-15 seconds per binary × 235 binaries. After consolidation (1.1), this matters less but still eliminates unnecessary weight from 7 of 8 binaries.

---

## Phase 2: ProxApi Test Runner Integration

With Phase 1 complete (consolidated binaries, nextest installed, mold linker), the test suite can be built into an archive and shipped to remote workers.

### 2.1 App Template: `test-worker`

A ProxApi app template that creates Debian LXC containers pre-configured for test execution. Workers don't need Rust installed — nextest runs from the archive.

```toml
[app_templates.test-worker]
description = "Nextest test execution worker"
os = "debian-12"
kind = "container"
cores = 4
memory_mb = 2048
setup_commands = [
  "apt-get update && apt-get install -y curl ca-certificates mold clang",
  "curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C /usr/local/bin",
  "mkdir -p /tmp/testrun"
]
```

Register by adding to ProxApi's `config.toml` under `[app_templates]`, then sync to all nodes:

```bash
curl -X POST http://proxapi:3000/v1/sync/trigger \
  -H "Authorization: Bearer sk-proxapi-..."
```

Verify:
```bash
curl http://proxapi:3000/v1/templates \
  -H "Authorization: Bearer sk-proxapi-..."
```

### 2.2 CLI Tool: `cargo test-remote`

A thin client that orchestrates test execution through ProxApi's API. Could start as a shell script and graduate to a Rust binary later.

**Usage:**
```bash
# Build archive locally, then distribute to 3 workers
cargo test-remote --workers 3

# Use a pre-built archive
cargo test-remote --workers 3 --archive tests.tar.zst

# Skip e2e tests on remote workers
cargo test-remote --workers 3 --filter 'not test(e2e)'

# Custom TTL for workers (default: 600s)
cargo test-remote --workers 5 --ttl 900
```

### 2.3 Test Run Flow (ProxApi API Calls)

Complete sequence of API calls that `cargo test-remote` performs:

**Step 1 — Build archive locally:**
```bash
cargo nextest archive --archive-file /tmp/tests.tar.zst --workspace
```

**Step 2 — Create N runners (ProxApi schedules on least-loaded nodes):**
```bash
# ProxApi auto-selects which Proxmox node to place each container on
POST /v1/runners
{
  "name": "test-1",
  "template": "test-worker",
  "cores": 4,
  "ttl": 600
}

POST /v1/runners  { "name": "test-2", "template": "test-worker", ... }
POST /v1/runners  { "name": "test-3", "template": "test-worker", ... }
```

**Step 3 — Wait for runners to reach Ready status:**
```bash
GET /v1/runners/test-1  → poll until status == "Ready"
GET /v1/runners/test-2  → poll until status == "Ready"
GET /v1/runners/test-3  → poll until status == "Ready"
```

**Step 4 — Upload archive to each runner:**
```bash
# Option A: exec via SSH (ProxApi exposes runner IP)
scp -i ~/.ssh/proxapi /tmp/tests.tar.zst root@{runner_ip}:/tmp/testrun/

# Option B: pipe through shell session
POST /v1/runners/{name}/shell  → get session_id
POST /v1/shell/{session_id}/pipe  (stdin: archive bytes)
```

**Step 5 — Create DAG tasks for tracking:**
```bash
POST /v1/tasks
{ "title": "partition-1/3", "input": { "partition": "hash:1/3", "runner": "test-1" } }

POST /v1/tasks
{ "title": "partition-2/3", "input": { "partition": "hash:2/3", "runner": "test-2" } }

POST /v1/tasks
{ "title": "partition-3/3", "input": { "partition": "hash:3/3", "runner": "test-3" } }
```

**Step 6 — Execute partitions via shell:**
```bash
POST /v1/runners/test-1/shell  → get session_id_1
POST /v1/shell/{session_id_1}/exec
{
  "command": "cd /tmp/testrun && cargo nextest run --archive-file tests.tar.zst --partition hash:1/3 --message-format libtest-json 2>&1"
}

# Same for test-2 (hash:2/3) and test-3 (hash:3/3) — all in parallel
```

**Step 7 — Stream output:**
```bash
# SSE stream for real-time progress
POST /v1/shell/{session_id_1}/stream
POST /v1/shell/{session_id_2}/stream
POST /v1/shell/{session_id_3}/stream

# Or WebSocket for bidirectional:
GET /v1/shell/{session_id}/ws
```

**Step 8 — Mark DAG tasks complete/failed based on exit codes:**
```bash
PUT /v1/tasks/{task_id}/complete
{ "output": { "passed": 1234, "failed": 0, "skipped": 12 } }

# Or on failure:
PUT /v1/tasks/{task_id}/fail
{ "error": "3 tests failed", "output": { "passed": 1200, "failed": 3 } }
```

**Step 9 — Aggregate results:**
- Collect JUnit XML from each runner (`/tmp/testrun/target/nextest/default/junit.xml`)
- Merge into a single report
- Print summary table to terminal
- Exit with pass/fail code

**Step 10 — Cleanup:**
Runners auto-destroyed by TTL (the `runner_reaper` background task runs every 30s). Explicit cleanup also available:
```bash
DELETE /v1/runners/test-1
DELETE /v1/runners/test-2
DELETE /v1/runners/test-3
```

### 2.4 Result Aggregation

Each nextest partition produces a JUnit XML report. `cargo test-remote` merges them:

```
┌──────────────────────────────────────────────────┐
│ test-remote: 3 workers, 3705 tests               │
├──────────┬──────────┬──────────┬─────────────────┤
│ Worker   │ Passed   │ Failed   │ Duration        │
├──────────┼──────────┼──────────┼─────────────────┤
│ test-1   │ 1235     │ 0        │ 42s             │
│ test-2   │ 1234     │ 0        │ 45s             │
│ test-3   │ 1236     │ 0        │ 41s             │
├──────────┼──────────┼──────────┼─────────────────┤
│ Total    │ 3705     │ 0        │ 45s (wall)      │
└──────────┴──────────┴──────────┴─────────────────┘
```

### 2.5 Fault Tolerance

ProxApi provides several fault-tolerance mechanisms:

- **Runner reaper:** If a worker crashes, the runner is reaped after TTL. The DAG task remains uncompleted, and `cargo test-remote` can detect the stall and re-dispatch to a new worker.
- **Task failure cascade:** `PUT /v1/tasks/{id}/fail` marks the task as failed. Parent tasks (if any) can cascade.
- **Shell session reaper:** Idle shell sessions are cleaned up automatically.
- **Retry logic:** `cargo test-remote` can re-create a runner and re-run the failed partition on a fresh worker.

---

## Phase 3: Optimizations

After Phase 2 delivers basic distributed execution, these optimizations improve partition balance, reduce unnecessary test runs, and speed up the e2e pipeline.

### 3.1 Timing-Aware Partitioning

nextest's default `hash` partitioning distributes tests by name hash — fast but potentially unbalanced. After running with `--message-format libtest-json`, parse test durations and store them:

```bash
# Store timing data after each run
cargo test-remote --workers 3 --save-timing timing.json

# Use timing data for balanced partitioning
cargo test-remote --workers 3 --timing timing.json
```

The CLI partitions tests by cumulative wall-clock time instead of count, so each worker finishes at roughly the same time.

### 3.2 Smart Test Selection

Avoid running the full suite when only a few files changed:

```bash
# Only run tests affected by changes since main
cargo test-remote --workers 3 --changed-since main

# Only run tests in the compiler binary
cargo test-remote --workers 3 --filter 'binary(compiler)'
```

Implement `--changed-since` by:
1. `git diff --name-only main` to find changed files
2. Map changed crate files → dependent test binaries
3. Pass `--filter` to nextest on each worker

### 3.3 E2E Pipeline Improvements

The 60 e2e harness tests are the slowest tests — each spawns a `cargo` subprocess.

**Template pre-build:** The e2e harness creates a fresh Cargo project per test. The dependencies are identical every time. Pre-build a template project during the test build phase:

```rust
fn ensure_template_built() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let template_dir = get_shared_target_dir().join("_template");
        // Write a minimal main.rs that imports all common deps
        // Run `cargo build` once to populate the shared target cache
    });
}
```

**Interpreter conversion:** Many e2e tests only check output correctness. These can use the interpreter instead of compiling to Rust:

```rust
// Current (slow): spawns cargo run
common::assert_exact_output("Set x to 5.\nShow x.", "5");

// Alternative (fast): uses interpreter, no subprocess
let result = common::run_interpreter("Set x to 5.\nShow x.");
assert_eq!(result.output.trim(), "5");
```

Each converted test goes from ~5-30 seconds (subprocess compilation) to <100ms (in-process interpretation).

**Batch compilation:** Instead of one `cargo build` per test, batch multiple programs into a single binary:

```rust
fn main() {
    let result_1 = { /* generated code for test 1 */ };
    println!("TEST_1_OUTPUT: {:?}", result_1);
    let result_2 = { /* generated code for test 2 */ };
    println!("TEST_2_OUTPUT: {:?}", result_2);
}
```

N subprocess compilations → 1. A single panic would abort all batched programs, so this works best for tests expected to succeed.

### 3.4 Runner Pooling

Instead of creating and destroying runners per test run, keep warm workers between runs:

```bash
# Create a pool of long-lived workers
cargo test-remote pool create --workers 3 --ttl 3600

# Run tests using the existing pool
cargo test-remote --pool

# Destroy the pool when done
cargo test-remote pool destroy
```

This eliminates the ~10-15 second container creation overhead per run, useful during active development when running tests frequently.

---

## Phase 4: CI Integration

### 4.1 GitHub Actions Self-Hosted Runner

Run a self-hosted GitHub Actions runner on the Mac Mini. CI workflows invoke the same `cargo test-remote` flow as local dev:

```yaml
jobs:
  test:
    runs-on: self-hosted
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: taiki-e/install-action@nextest

      - name: Build and archive tests
        run: cargo nextest archive --archive-file tests.tar.zst --workspace

      - name: Run distributed tests
        run: cargo test-remote --workers 5 --archive tests.tar.zst

      - name: Upload test results
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: test-results
          path: test-results/*.xml
```

Benefits over cloud CI runners:
- Full access to the Proxmox cluster (5 nodes, distributed execution)
- No egress costs for shipping archives
- Persistent cargo registry/build cache on local disk
- Same tooling and flow as local development

### 4.2 Fallback: Cloud CI with Nextest Partitioning

For PRs from forks or when the self-hosted runner is unavailable, fall back to standard cloud CI with nextest partitioning:

```yaml
jobs:
  build:
    runs-on: ubuntu-latest-m
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: taiki-e/install-action@nextest

      - name: Cache cargo
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/bin/
            ~/.cargo/registry/index/
            ~/.cargo/registry/cache/
            ~/.cargo/git/db/
            target/
          key: ${{ runner.os }}-cargo-${{ hashFiles('Cargo.lock') }}

      - name: Build and archive tests
        run: cargo nextest archive --archive-file tests.tar.zst --workspace

      - name: Upload archive
        uses: actions/upload-artifact@v4
        with:
          name: nextest-archive
          path: tests.tar.zst

  test:
    needs: build
    runs-on: ubuntu-latest-m
    strategy:
      matrix:
        partition: [1, 2, 3]
    steps:
      - uses: taiki-e/install-action@nextest
      - uses: actions/download-artifact@v4
        with:
          name: nextest-archive
      - name: Run partition ${{ matrix.partition }}/3
        run: >
          cargo nextest run
          --archive-file tests.tar.zst
          --partition hash:${{ matrix.partition }}/3
```

### 4.3 Improved Cache Keys

```yaml
- name: Cache cargo registry
  uses: actions/cache@v4
  with:
    path: |
      ~/.cargo/bin/
      ~/.cargo/registry/index/
      ~/.cargo/registry/cache/
      ~/.cargo/git/db/
    key: ${{ runner.os }}-cargo-registry-${{ hashFiles('Cargo.lock') }}

- name: Cache build artifacts
  uses: actions/cache@v4
  with:
    path: target/
    key: ${{ runner.os }}-target-${{ hashFiles('Cargo.lock') }}-${{ hashFiles('rust-toolchain.toml') }}
    restore-keys: |
      ${{ runner.os }}-target-${{ hashFiles('Cargo.lock') }}-
      ${{ runner.os }}-target-
```

---

## Appendix A: File Classification (235 Files → 8 Binaries)

### Binary 1: `e2e_subprocess` (34 files)

All `e2e_*` prefixed files. Use `mod common` harness, spawn `cargo run` subprocesses.

```
e2e_async_cross_cutting.rs    e2e_maps.rs
e2e_causal_consistency.rs     e2e_mesh.rs
e2e_collections.rs            e2e_multi_node.rs
e2e_comparisons.rs            e2e_network_partition.rs
e2e_concurrency.rs            e2e_policy.rs
e2e_control_flow.rs           e2e_primitives.rs
e2e_crdt.rs                   e2e_refinement.rs
e2e_edge_cases.rs             e2e_sets.rs
e2e_enums.rs                  e2e_structs.rs
e2e_expressions.rs            e2e_studio_examples.rs
e2e_feature_matrix.rs         e2e_temporal.rs
e2e_functions.rs              e2e_temporal_show.rs
e2e_gossip.rs                 e2e_tuples.rs
e2e_gossip_edge_cases.rs      e2e_types.rs
e2e_integration.rs            e2e_variables.rs
e2e_iteration.rs              e2e_zones.rs
e2e_language_gaps.rs
e2e_logical.rs
```

### Binary 2: `e2e_harness` (26 files)

Non-`e2e_*` files that use `mod common` harness (also spawn subprocesses).

```
aktionsart_tests.rs               phase52_sync.rs
diagnostic_bridge.rs              phase53_persistence.rs
integration_tests.rs              phase54_concurrency.rs
phase10_io.rs                     phase85_zones.rs
phase46_agents.rs                 phase9_structured_concurrency.rs
phase48_network.rs                phase_crdt_language.rs
phase49_crdt.rs                   phase_crdt_variants.rs
phase50_security.rs               phase_escape_hatch.rs
phase51_mesh.rs                   phase_ffi_requires.rs
phase102_bridge.rs                phase_interpreter_crdt.rs
phase103_generics.rs              phase_interpreter_features.rs
test_concurrency.rs               phase_interpreter_policy.rs
user_concurrency.rs               phase_interpreter_string_concat.rs
```

### Binary 3: `linguistics` (42 files)

Phase 1-20 + linguistic phenomena tests. No `mod common`, test against workspace crates directly.

```
complex_combinations.rs           phase10_ellipsis.rs
debug_any.rs                      phase10b_sluicing.rs
debug_aspect.rs                   phase11_metaphor.rs
debug_card.rs                     phase11_sorts.rs
debug_garden.rs                   phase12_ambiguity.rs
debug_modal_relclause.rs          phase13_mwe.rs
debug_reciprocal.rs               phase14_ontology.rs
debug_tense.rs                    phase15_negation.rs
gq_test.rs                        phase16_aspect.rs
intensionality_tests.rs           phase17_degrees.rs
modal_scope_tests.rs              phase18_plurality.rs
parser_fixes_test.rs              phase19_group_plurals.rs
phase1_garden_path.rs             phase20_axioms.rs
phase2_polarity.rs                phase3_aspect.rs
phase3_time.rs                    phase4_movement.rs
phase4_reciprocals.rs             phase5_wh_movement.rs
phase6_complex_tense.rs           phase7_semantics.rs
phase8_degrees.rs                 phase9_conversion.rs
phase_hints.rs                    phase_inversion.rs
phase_kripke.rs                   phase_lexer_refactor.rs
phase_privation_modal.rs          torture_tests.rs
```

### Binary 4: `compiler` (51 files)

Phase 21-45 + compiler pipeline tests. Codegen, types, structs, iteration, etc.

```
grand_challenge_mergesort.rs      phase30_iteration.rs
interpreter_tests.rs              phase31_structs.rs
phase21_block_headers.rs          phase32_functions.rs
phase21_imperative_verbs.rs       phase33_enums.rs
phase21_ownership.rs              phase34_generics.rs
phase22_equals.rs                 phase35_proofs.rs
phase22_index.rs                  phase35_respectively.rs
phase22_is_rejection.rs           phase36_modules.rs
phase22_resolution.rs             phase37_cli.rs
phase22_scope.rs                  phase38_stdlib.rs
phase23_blocks.rs                 phase41_event_adjectives.rs
phase23_parsing.rs                phase42_drs.rs
phase23_stmt.rs                   phase43_collections.rs
phase23_tokens.rs                 phase43_discourse_scope.rs
phase23_types.rs                  phase43_refinement.rs
phase24_codegen.rs                phase43_type_check.rs
phase24_wired_types.rs            phase44_distributive.rs
phase25_assertions.rs             phase44_modal_subordination.rs
phase25_smoke_tests.rs            phase45_intension.rs
phase25_type_expr.rs              phase45_session.rs
phase26_e2e.rs                    phase46_ellipsis.rs
phase27_guards.rs                 phase57_maps.rs
phase28_precedence.rs             phase_operator.rs
phase29_runtime.rs                phase_ownership.rs
phase_primitives_extended.rs      phase_sets.rs
symbol_dict_tests.rs
```

### Binary 5: `proof_kernel` (47 files)

Phase 60-100+, proof engine, kernel, verification, tactics.

```
phase60_proof_engine.rs           phase86_kernel_primitives.rs
phase61_induction.rs              phase87_reflection.rs
phase62_oracle.rs                 phase88_substitution.rs
phase63_theorem_parser.rs         phase89_computation.rs
phase65_event_semantics.rs        phase90_bounded_eval.rs
phase66_higher_order.rs           phase91_quote.rs
phase67_pattern_unification.rs    phase92_inference.rs
phase68_auto_induction.rs         phase93_diagonal_lemma.rs
phase69_kernel_coc.rs             phase94_godel_sentence.rs
phase70_inductive_types.rs        phase95_incompleteness.rs
phase70b_elimination.rs           phase96_tactics.rs
phase70c_computation.rs           phase97_deep_induction.rs
phase71_cumulativity.rs           phase98_strategist.rs
phase72_kernel_prelude.rs         phase99_solver.rs
phase73_certifier.rs              phase100_the_summit.rs
phase74_certify_quantifiers.rs    phase101a_poly_inductive.rs
phase75_certify_intro.rs          phase101b_generic_elim.rs
phase76_certify_induction.rs      phase101c_list_ops.rs
phase77_certify_exists.rs         phase101d_theorems.rs
phase78_e2e_verification.rs       phase_induction.rs
phase79_termination.rs            phase_totality.rs
phase80_equality_rewriting.rs
phase81_computation.rs
phase82_delta_reduction.rs
phase83_vernacular.rs
phase84_extraction.rs
```

### Binary 6: `data_crdt` (15 files)

CRDT data-layer tests, temporal, literate proofs. No `mod common`.

```
phase_crdt_causal.rs              phase_crdt_sequence.rs
phase_crdt_concurrent.rs          phase_crdt_serialization.rs
phase_crdt_delta.rs               phase_crdt_stress.rs
phase_crdt_edge_cases.rs          phase_temporal_lexer.rs
phase_crdt_mvregister.rs          phase_temporal_operations.rs
phase_crdt_ormap.rs               phase_temporal_primitives.rs
phase_crdt_orset.rs               phase_temporal_spans.rs
phase_crdt_pncounter.rs
```

### Binary 7: `literate_proofs` (16 files)

Literate proof, tactic automation, and verification tests. No `mod common`.

```
phase_auto.rs                     phase_literate_lia.rs
phase_barber_updated.rs           phase_literate_omega.rs
phase_cc.rs                       phase_literate_ring.rs
phase_lia.rs                      phase_literate_simp.rs
phase_literate_auto.rs            phase_omega.rs
phase_literate_cc.rs              phase_ring.rs
phase_literate_induction.rs       phase_simp.rs
phase_verification.rs             phase_verification_refinement.rs
```

### Binary 8: `misc` (3 files)

Web-dependent tests. Only binary that needs `logicaffeine-web`.

```
learn_state_tests.rs
struggle_tests.rs
unlock_logic_tests.rs
```

### Remaining: `extraction_common` (1 file)

Uses its own shared module (`extraction_common/mod.rs`). Can go into `proof_kernel` or stay standalone.

```
phase84_extraction_e2e.rs
```

---

## Appendix B: Unit Test Distribution

Tests embedded in crate `src/` files (via `#[cfg(test)]`):

| Crate | Unit Tests |
|-------|-----------|
| `logicaffeine_language` | 173 |
| `logicaffeine_lsp` | 107 |
| `logicaffeine_kernel` | 98 |
| `logicaffeine_web` | 89 |
| `logicaffeine_system` | 66 |
| `logicaffeine_data` | 62 |
| `logicaffeine_base` | 26 |
| `logicaffeine_compile` | 22 |
| `logicaffeine_verify` | 20 |
| `logicaffeine_proof` | 16 |
| `logicaffeine_cli` | 10 |
| `logicaffeine_lexicon` | 5 |
| **Total** | **694** |

These compile as part of their respective crate's test binary — no consolidation needed. They benefit from Phase 1 optimizations (linker, profile) and nextest parallelism.

---

## Appendix C: CI Workflow Reference

Current workflow: `.github/workflows/test.yml`

```yaml
# Current (slow, single machine)
- cargo build --workspace --verbose
- cargo test --workspace 2>&1 | tee test_output.txt

# Phase 1 (nextest, same machine)
- cargo nextest archive --archive-file tests.tar.zst --workspace
- cargo nextest run --archive-file tests.tar.zst

# Phase 2 (distributed via ProxApi, self-hosted runner)
- cargo nextest archive --archive-file tests.tar.zst --workspace
- cargo test-remote --workers 5 --archive tests.tar.zst

# Fallback (cloud CI with matrix partitioning)
- cargo nextest run --archive-file tests.tar.zst --partition hash:${{ matrix.partition }}/3
```

---

## Appendix D: ProxApi Test Worker Template

Full app template configuration for `config.toml`:

```toml
[app_templates.test-worker]
description = "Nextest test execution worker"
os = "debian-12"
kind = "container"
cores = 4
memory_mb = 2048
disk_gb = 10
setup_commands = [
  "apt-get update && apt-get install -y curl ca-certificates mold clang",
  "curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C /usr/local/bin",
  "mkdir -p /tmp/testrun",
]
```

The template is synced to all Proxmox nodes via:
```
POST /v1/sync/trigger
```

Workers created from this template have nextest pre-installed and a working directory at `/tmp/testrun`. They don't need Rust, cargo, or the source code — only the nextest binary and the test archive.

---

## Appendix E: `test-remote` CLI Reference

```
cargo test-remote [OPTIONS]

OPTIONS:
    --workers <N>           Number of test workers to create (default: 3)
    --archive <PATH>        Path to nextest archive (default: build one)
    --filter <EXPR>         nextest filter expression (e.g., 'not test(e2e)')
    --ttl <SECONDS>         Worker TTL in seconds (default: 600)
    --timing <PATH>         Timing data file for balanced partitioning
    --save-timing <PATH>    Save timing data after run
    --changed-since <REF>   Only run tests affected by changes since git ref
    --pool                  Use existing warm worker pool
    --proxapi <URL>         ProxApi server URL (default: http://localhost:3000)
    --verbose               Stream worker output to terminal

SUBCOMMANDS:
    pool create             Create a warm worker pool
    pool destroy            Destroy the warm worker pool
    pool status             Show pool status

EXAMPLES:
    # Quick: build and distribute to 3 workers
    cargo test-remote --workers 3

    # Skip e2e, use 5 workers
    cargo test-remote --workers 5 --filter 'not test(e2e)'

    # Use pre-built archive
    cargo nextest archive --archive-file tests.tar.zst
    cargo test-remote --workers 3 --archive tests.tar.zst

    # Balanced partitioning from previous timing data
    cargo test-remote --workers 3 --timing .test-timing.json --save-timing .test-timing.json

    # Only tests affected by recent changes
    cargo test-remote --workers 3 --changed-since main
```
