# Are the benchmarks "90% malloc and printf"? No — they're ~97% compute.

Someone claimed the LOGOS benchmark suite is measuring `malloc` and `printf`, not
compute. This is a CPU profile of all 35 benchmarks — **LOGOS and the C baseline** —
that settles it. Reproduce with `bash scripts/bench/profile-benchmarks.sh`.

## Verdict

Across the 31 compute benchmarks (excluding the 4 the LOGOS optimizer collapses — see
below):

| | LOGOS | C baseline |
|---|---|---|
| mean time in **compute** (the actual algorithm) | **96.8%** | 96.1% |
| benchmarks with **>1% malloc** | **2 / 31** (`binary_trees`, `mergesort`) | 2 / 31 (`binary_trees`, `mergesort`) |
| benchmarks with **>1% printf/formatting** | **0 / 31** | 1 / 31 (`strings`) |
| real **stdout output** (`write` syscalls per run) | **1** | 1 |

LOGOS and C spend essentially the **same** ~97% in compute, and the only allocation
cost either pays is the one its algorithm asks for. Even if the accusation were true it
would apply identically to the C baseline and cancel out of the speed ratio. It is not
true for either.

The visual proof is `benchmarks/profile/flamegraphs.pdf` (page 1 lines up all 35,
LOGOS vs C): **33 of 35 benchmarks are a solid green tower of the actual algorithm**
on LOGOS. The two that show anything else are the two allocation-bound algorithms,
and C shows the same two.

## What the four colors mean (userspace CPU sampling, `perf`)

- **compute** (green) — the algorithm itself: `bench::qs`, the nbody force loop, the
  sieve, matrix multiply, etc.
- **malloc** (red) — allocator (`malloc`/`free`/`realloc`/Rust alloc).
- **printf / formatting** (purple) — *all* number→text work: Rust `core::fmt`, C
  `sprintf`/`printf`/`vfprintf`. At the symbol level `printf` and `sprintf` share
  `_IO_vfprintf`, so they can't be split by leaf. **Real stdout output is a `write`
  syscall** — `strace` shows **1 per whole run** (a single final line), i.e. ~0 CPU.
  So this bucket is in-memory formatting, not I/O.
- **startup** (gray) — one-time ELF loader + thread spawn/teardown. These frames sit
  at the base of every stack; they're trimmed from the flamegraph so it shows the
  algorithm, and reported separately. Largest on the shortest-running benchmarks.

## Per-benchmark (trimmed, LOGOS vs C)

LOGOS compute% / malloc% / printf-fmt%, then C compute%. Everything not shown is
one-time startup. `*` = the optimizer collapses it (see below); `—` = LOGOS finishes in
under a millisecond, so there is nothing to sample.

| benchmark | L compute% | L malloc% | L printf% | C compute% |
|---|---|---|---|---|
| fib * | — | 0 | 0 | 100 |
| ackermann * | — | 0 | 0 | 100 |
| nqueens * | 100 | 0 | 0 | 100 |
| bubble_sort | 100 | 0 | 0 | 100 |
| mergesort | 83 | **17** | 0 | 83 |
| quicksort | 100 | 0 | 0 | 100 |
| counting_sort | 100 | 0 | 0 | 100 |
| heap_sort | 100 | 0 | 0 | 100 |
| nbody | 100 | 0 | 0 | 100 |
| mandelbrot | 100 | 0 | 0 | 100 |
| spectral_norm | 100 | 0 | 0 | 100 |
| pi_leibniz | 100 | 0 | 0 | 100 |
| gcd | 100 | 0 | 0 | 100 |
| collatz | 100 | 0 | 0 | 100 |
| primes | 100 | 0 | 0 | 100 |
| sieve | 100 | 0 | 0 | 100 |
| matrix_mult | 100 | 0 | 0 | 100 |
| prefix_sum | 100 | 0 | 0 | 100 |
| array_reverse | 100 | 0 | 0 | 100 |
| array_fill | 100 | 0 | 0 | 100 |
| collect | 100 | 0 | 0 | 100 |
| two_sum | 100 | 0 | 0 | 100 |
| histogram | 100 | 0 | 0 | 100 |
| knapsack | 100 | 0 | 0 | 100 |
| coins | 100 | 0 | 0 | 100 |
| fannkuch | 100 | 0 | 0 | 100 |
| strings | 100 | 0 | 0 | 60 |
| binary_trees | 16 | **84** | 0 | 36 |
| loop_sum * | 100 | 0 | 0 | 100 |
| fib_iterative | 100 | 0 | 0 | 100 |
| graph_bfs | 100 | 0 | 0 | 100 |
| string_search | 100 | 0 | 0 | 100 |
| analytics | 100 | 0 | 0 | 100 |
| vm | 100 | 0 | 0 | 100 |
| sha256 | 100 | 0 | 0 | 100 |

## The only two benchmarks that aren't ~100% compute — and C matches both

- **`binary_trees` — 84% malloc (LOGOS), 64% (C).** The Benchmarks Game program: it
  builds and frees complete binary trees, so the allocator *is* the workload, and **the
  C baseline pays it too**. 0% printf.
- **`mergesort` — 17% malloc (LOGOS), 17% (C).** Top-down merge sort allocates temporary
  merge buffers, intrinsic to the algorithm, and C pays the same share. Still 83%
  compute, and 0% printf.

`strings` is the one benchmark where the two differ on formatting: its *job* is turning
integers into text and concatenating, and C spends 40% of its time in `snprintf` while
LOGOS's integer-to-text runs in its own code, which the profile counts as compute. Both
make a single `write`.

Every other benchmark: allocate the working array **once** before the loop (the same
single `malloc` C does), run millions of iterations touching neither the heap nor
stdout, print **one line** at the end.

## The 4 collapsed benchmarks (`*`) — honest disclosure

`fib`, `ackermann`, `nqueens` and `loop_sum` run faster on LOGOS than on C because the
optimizer does less work: it memoizes `fib` (304× C at the reference size), evaluates
`ackermann` in closed form (537×), searches half of the mirror-symmetric `nqueens` board
(1.9×), and replaces `loop_sum`'s loop with the closed form of its running sum (12.9×).
`fib` and `ackermann` finish in under a millisecond, so there is nothing to profile on
the LOGOS side. Those speedups come from doing less work, not faster work — **a real and
separate thing to know about these four benchmarks, but not "malloc and printf": they
are 0% both.** The raw-compute story is the other 31, where LOGOS and C run the
identical algorithm at ~97% compute.

## Reproduce

```bash
# needs perf, strace, jq, python3, libc6-dbg (better symbols); perf is auto-enabled
# via sudo sysctl (restored on exit)
bash scripts/bench/profile-benchmarks.sh                                 # all 35, every profiled tier
BENCH_ONLY=nbody,mergesort,strings bash scripts/bench/profile-benchmarks.sh   # a subset
```

Outputs: `benchmarks/profile/flamegraphs.pdf` (36 pages: a summary + 35 LOGOS-vs-C
flamegraphs), `benchmarks/results/latest-profile.json` (what the page reads: the
flamegraphs, counters and census of every profiled tier — C, LOGOS (Rust), LOGOS (C),
LOGOS (C, bytecode) and LOGOS (JIT)), `benchmarks/results/profile.json` (raw numbers),
and `benchmarks/profile/folded/*.folded` (the raw stacks). Measured on Linux (kernel
6.8), `perf record -e cycles:u -F 2000 --call-graph=dwarf`, at one size per benchmark
chosen so the C binary runs about 4 s, the same n on every tier. Public benchmark page:
<https://logicaffeine.com/benchmarks>.
