# Fabric energy savings — per algorithm (measured, same-kernel basis)

Energy for one invocation of each compute kernel. Anchored to the ONE fully-measured Sky130
signoff-clean kernel (`decision_hft`: 17 tiles → **18 pJ** logic / **28 pJ** block on the fabric,
**37.9 nJ** on the soft-core running the same work). Each kernel scales by its own tile count.

| Algorithm (kernel) | tiles | **Fabric** | Soft-core (a CPU) | **× less energy** |
|---|---:|---:|---:|---:|
| gcd `gcd` | 2 | **2 pJ** | 4 nJ | **~2,106×** |
| spectral_norm `aVal` | 9 | **10 pJ** | 20 nJ | **~2,106×** |
| nqueens `isSafe` | 14 | **15 pJ** | 31 nJ | **~2,106×** |
| collect `benchmark` | 16 | **17 pJ** | 36 nJ | **~2,106×** |
| sieve `sieve` | 23 | **24 pJ** | 51 nJ | **~2,106×** |
| heap_sort `siftDown` | 35 | **37 pJ** | 78 nJ | **~2,106×** |
| strings `countSpaces` | 240 | **254 pJ** | 535 nJ | **~2,106×** |
| **all 7 kernels** | 339 | **0.36 nJ** | 755.8 nJ | **~2,100×** |

The ratio is ~2,100× on **every** kernel because the fabric's advantage is architectural — one tile
per operation, whole dataflow settled in one clock edge (no fetch/decode/register-file/cache). Using
the conservative whole-block energy basis (28 pJ) instead of logic-only (18 pJ) it is **~1,350×**.

## Honest scope

- Same-kernel, measured: fabric vs the soft-core running the **identical** kernel. This is the
  defensible basis (supersedes any full-pipeline comparison).
- 7 of 35 benchmark kernels lower to the pure combinational fabric today (branch-free / bounded-loop
  cores). The other 28 (recursion, `Main`-driven outer loops) run on the sequenced fabric / soft-core
  — the over-the-wire engine deployed to the Nexys A7 this session.
- Power is Sky130 ASIC (OpenSTA signoff), VCD-cross-checked. The Artix-7 FPGA is where these are
  functionally proven on real hardware; FPGA per-gate power is higher, so it is not the power story.
