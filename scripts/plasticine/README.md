# Plasticine lane — runbook

The LOGOS → Plasticine (open RDU, Prabhakar et al. ISCA 2017) lane has two provenances, and every artifact
says which one it is:

| provenance | what produced it | where it lives |
|---|---|---|
| `modeled` | `codegen_sva::{plasticine, metapipeline, pmu, plasticine_place, plasticine_rrg, plasticine_report}` — the paper-parameterized descriptors (ISCA'17 16×8 / DDR3, SARA 20×20 / HBM2), Spatial's II definition, the PMU banking model, row-balanced placement, and static routing by the private place-and-route engine | `benchmarks/silicon/plasticine_report.json` (generated ONLY by that engine's report emitter, `emit_plasticine_report`; `plasticine_report_lock` pins it byte-equal) |
| `simulated_tungsten` | the public Plasticine toolchain (Spatial → PIR → plastiroute → Tungsten) — today on Spatial's OWN regression apps (the credibility gate); our emitted Spatial next | `benchmarks/silicon/plasticine_toolchain.json` (generated ONLY by `scripts/accel/plasticine-gate-ledger.py`, excerpts under `benchmarks/silicon/plasticine_logs/`) + `benchmarks/silicon/plasticine_calibration.json` (the model joined against it; `plasticine_sim_lock` pins both) |

Nothing in this lane is measured on silicon; no Plasticine chip exists outside Stanford.

## What runs today (modeled)

```bash
# the report over the canonical workloads × the two published targets: the private place-and-route engine's
# report emitter (emit_plasticine_report) writes it, and the lock checks the committed copy
cargo nextest run -p logicaffeine-tests -E 'binary(plasticine_report_lock)'

# one program → Spatial source + placement + report row
cargo run -p logicaffeine-cli -- emit plasticine path/to/loop.lg            # prints Logos_<fn>.scala
cargo run -p logicaffeine-cli -- emit plasticine path/to/loop.lg -o out/    # + placement.json, report.json
```

The region unit is the first function, which must be a counted range loop (`Repeat for i from lo to hi`);
anything else is refused typed (`plasticine backend: …`).

## The gate PASSED (2026-09-02) — the first calibration point

`gate.sh GEMM_0` ran Spatial's own `GEMM_0` (128^3, 64-tiles, 16-lane reduce, unmodified) through the whole
chain: genpir → PIR (every pass; `resource.csv`: PCU 7/200, PMU 5/200, MC 3/40, DAG 7/40 on the 20x20
mesh) → Tungsten simulator generated and built → ideal-network simulation → plastiroute (best score 1.01,
0 VCs) → hybrid-network simulation: **PASS on both, 132,068 cycles**, DRAM read 1.98 GB/s. Our MODELED row
for the same shape/tile/unroll on the 20x20 preset: 131,339 cycles — **0.55 % apart**, same 16-lane
structure (PIR's 7 PCUs vs our 1 compute PCU is its partitioner splitting control/address units; both
printed). Recorded by `scripts/accel/plasticine-gate-ledger.py add …` and joined by `emit_plasticine_report`.

```bash
python3 scripts/accel/plasticine-gate-ledger.py add --app GEMM_0 \
    --gen-dir target/plasticine-toolchain/spatial/gen/Tst_pirTest/GEMM_0 \
    --shape 128x128x128 --params "dim=128 ts=64 its=64 ip=16 loop_ii=1 loop_jj=1 loop_kk=1 loop_i=1 loop_j=1" \
    --toolchain "<branch snapshot string>"
python3 scripts/accel/plasticine-gate-ledger.py verify
# the private place-and-route engine's report emitter (emit_plasticine_report) regenerates the calibration file
cargo nextest run -p logicaffeine-tests -E 'binary(plasticine_sim_lock)'
```

## OUR loops through THEIR chain — the calibration that fixed the PCU model (2026-09-02)

`emit_plasticine_spatial_tests <regression dir>` writes every canonical counted loop as a Spatial regression
app (`class Logos_<name> extends SpatialTest`, 64-bit `Long`, the VM's answer as the gold, the anchor's
inputs baked into the loop bounds); `gate.sh Logos_<name>` runs it through the chain. Five ran end to end
and PASSED (the simulator computed the VM's answer); Tungsten's cycle counts are what fixed the model:

| loop | iterations | Tungsten cycles | cycles / iteration |
|---|---|---|---|
| `s = s + i` | 1000 | 7,003 | 7.0 |
| `s = s*3 + i` | 30 | 213 | 7.1 |
| `s = (s*3 + i)*2` | 20 | 143 | 7.2 |
| `s = ((s*2)*3)*5 + i` | 12 | 87 | 7.3 |
| `s = i*7 + 1` (no carried dependency) | 1000 | 1,009 | 1.0 |

A loop-carried register costs the PCU's pipeline depth plus one per iteration — **7 on the six-stage PCU,
however many operations sit on the carried path** — and a loop with no carried dependency issues every
cycle. `UnitModel::plasticine_pcu(stages)` encodes exactly that (result latency = pipeline depth, register
turnaround 1); with it the modeled cycles land within 0.06 % to 4.6 % of Tungsten on the five loops, and the
report's counted-loop rows now carry the calibrated II (7 with a carried register, 1 without). The ideal
model (`UnitModel::ideal_pipelined`, II = carried-path length) remains the bound `metapipeline_lock` proves.

Spatial's `DotProduct_0` (65,536 elements, 16,384-element chunks, 16-lane reduce) also passed: 6,056 cycles,
memory-bound — Tungsten reports `Average DRAM Read Bandwidth: 86.57 GB/s`. The chain's memory is NOT the
SARA preset's HBM2: every generated app carries `DDR4-config.cfg` (ramulator, `DDR4_2133`, 4 channels,
`DDR4_8Gb_x16`), so the 20x20 rows calibrate against `PlasticineTarget::tungsten_20x20_ddr4()` — PIR's
default mesh in front of that memory, at the bandwidth Tungsten itself reports. Compute-bound rows do not
move; the dot product's roofline lands on it.

Spatial's `DotProduct` variants (and their LOGOS twins, `scripts/plasticine/twins.py`) calibrated the operand
stream. A K-chunked reduction loads each chunk at the reduce's own lane width (`par ip` on its loads), so its first
chunk loads before any compute starts and a chunk whose load outlasts its compute stalls the MAC: a one-lane dot
product of four 16,384-element chunks runs (4 + 1) × 16,384 cycles, and side-by-side chunk pipelines (`op`) take whole
rounds of chunks. `gemm_row_unrolled` models both (`operand_stall_cycles`, `GemmRow::chunks_in_sequence`), at any lane
count — Spatial's own apps run five.

Two loops with data-dependent selects (`loop-select`, `moe-argmax-round-128`) do not go through PIR: its
RewriteTransformer asserts a single producer for an accumulation ("Not exactly one element … for accum") —
a limitation of the chain on general loop-carried computations, recorded, not worked around.

## The Tungsten lane — scripts (`scripts/plasticine/`)

```bash
scripts/plasticine/fetch.sh              # GitLab archive tarballs → target/plasticine-toolchain (no git)
scripts/plasticine/run-in-container.sh   # builds the deps image (ubuntu:18.04, JDK 8, sbt, python3.6, gcc-10) and runs build.sh in it
scripts/plasticine/run-in-container.sh bash /gate.sh GEMM_0   # the credibility gate (mount gate.sh, see below)
```

`emit_plasticine_spatial_gemm <regression dir> M N K [ts its ip]` writes a LOGOS matmul (the triple loop the
tensor recognizer reads) as Spatial's own GEMM structure with our shape and tiles (`Logos_gemm_MxNxK`), so
`gate.sh Logos_gemm_MxNxK` runs OUR GEMM through the chain.

`build.sh` sets aside (renames, never deletes) the regression apps this Spatial master cannot compile
(the Capstan-era sparse apps, `LUTTest`, `MLP`, `DRAMCompression`); the dense apps — `GEMM_0..8` — are the
gate's subjects. State reached so far: plastiroute built, Tungsten's simulator library objects built
(`lib/plasticine.o.{DBG,PRF,OPT}`), PIR published to the local ivy cache, the Spatial `pirTest` regression
set compiled, Spatial's polyhedral helper (`emptiness`, needs libisl + libgmp) compiles; the current
frontier is the gate run itself (`gate.sh GEMM_0`). `plastisim` does not build (fallback oracle only).
Python: the pins are installed one by one; cvxpy 1.0.x has no Python 3.6 distribution any more, so the venv
carries cvxpy 0.4.10 + scipy 1.2.3 — PIR's cvxpy partitioner may be unusable (its Gurobi and heuristic
partitioners are the alternatives).

First vendor-toolchain output (2026-09-02): with Spatial@`pir` + PIR@`master`, `bin/test -a GEMM_0 -b Tst`
ran Spatial's genpir pass and ALL of PIR's passes through 63-RuntimeAnalyzer and the plastiroute link/node
generation; `gen/Tst_pirTest/GEMM_0/pir/out/resource.csv` reports the mapping of Spatial's own `GEMM_0`
(128^2, 64-tiles) onto PIR's default 20x20 mesh — PCU 7/200 (3.5 %), PMU 5/200 (2.5 %), MC 3/40, DAG
7/40. Provenance of that number: PIR's mapper (the vendor toolchain), no simulation yet; it is NOT one of
our rows. Tungsten codegen (pass 70) then needs `tungsten/plasticine/resources`, absent on the `mlir`
branch — PIR master's pinned Tungsten commit lives on Tungsten `master`, which fetch.sh now takes.

Branch pairing (the mirrors' defaults are skewed): Spatial at branch `pir` (2023-01-31, emits
`StridedCounter`/`ForeverCounter`) with PIR `master` (2021-05-04, defines them); Spatial `master` (2020-11)
emits the older `Counter(par=…)` and cannot compile its own generated apps against this PIR.

Bridges (no toolchain source edited): the Spatial snapshot's `bin/test` calls a `main()` that this PIR
snapshot's `spatial_test.py` lacks, so `gate.sh` drives `SpatialTest(args=[...]).run()` directly; Tungsten's
Makefile links `-lfmt` without an include path (`CPATH`); `make tungsten` links a test stub `main.cc`, so the
library objects are built (`make all`) and each app's generated main links against them.

## The Tungsten lane (P5 — to do, in this order)

The public chain is mirrored at `https://gitlab.com/dataplane-ai/{spatial,pir,plastiroute,plastisim,tungsten}`
(2023-01-31 snapshot). Build it in a container so the old toolchain never touches the host:

1. `FROM ubuntu:20.04`; install OpenJDK 8, sbt (with the repositories patch for the retired Bintray
   resolvers), Python 3.6 venv, g++ ≥ 10, GLib, OpenMP, an old TBB.
2. Clone the five mirrors side by side; rewrite `pir/.gitmodules` URLs from GitHub to the GitLab mirrors
   (the GitHub submodules are private).
3. Build Spatial (`sbt compile`), PIR (`sbt`), plastiroute (`make`), Tungsten (`make`, C++20 + Ramulator).
4. Credibility gate: `bin/test -a GEMM -b Tst -p pirTest` on the UNMODIFIED chain must reproduce Spatial
   PLDI'18 Table 4's Plasticine GEMM row (PCU 96.8 % / PMU 64.1 % / AG 11.7 %, 15.98 ms) from
   `resource.csv` + `Simulation complete at cycle N`. Until this reproduces, no row of ours goes through it.
5. Run our Spatial sources (`largo emit plasticine … -o`) through the same chain; record Tungsten cycles and
   `resource.csv` beside the modeled rows as `provenance: simulated_tungsten`.
6. Add the gated `plasticine_sim_lock` (`#[ignore]`, skip-loud without the container) pinning simulated vs
   modeled within the tolerance the reviewer sets (default ±15 % cycles, ±5 points utilization).

Tungsten and plastiroute carry no LICENSE file in the mirrors — ask before shipping anything built from them.
