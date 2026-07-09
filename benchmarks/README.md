# LOGICAFFEINE Benchmarks

Cross-language benchmark suite: 32 programs implemented in 11 languages
(C, C++, Rust, Zig, Go, Java, JavaScript, Python, Ruby, Nim, and LOGOS),
measured with [hyperfine](https://github.com/sharkdp/hyperfine) and verified
for output correctness before any timing happens.

The suite serves two jobs:

1. **The optimization loop** — `run-logos-vs-c.sh` shows exactly where LOGOS
   wins and loses against C, and dumps generated Rust, assembly, and LLVM IR
   for both sides so a bad ratio can be diagnosed immediately.
2. **The coverage check** — `run-quick.sh` proves every benchmark still
   builds, runs, and produces correct output in every language.

## One-time setup

```bash
bash benchmarks/setup-local.sh
```

Installs every missing toolchain: Go/JDK/Ruby via apt, plus Zig 0.15.2,
Nim 2.0.4, and hyperfine 1.18.0, pinned so every bench box measures with
the same tools. Rust is expected to already be present
via rustup. Idempotent — tools already on PATH are left alone. clang is
needed for the C-side assembly/LLVM dumps.

## The optimization loop: LOGOS vs C

```bash
bash benchmarks/run-logos-vs-c.sh
```

Answers "where does LOGOS win and where does C win" as fast as possible:

- **C baselines are static.** They're measured once at calibrated sizes
  (100–2000ms of C runtime, so process startup is noise) and cached in
  `results/c-baselines.json`. A baseline is only re-measured when `main.c`
  changes or you pass `FORCE_BASELINE=1`. A typical run measures *only the
  LOGOS side*.
- **LOGOS binaries rebuild exactly when largo changes.** Rebuilding the
  compiler invalidates the cached LOGOS binaries; nothing else rebuilds.
- Output ends with a worst-first ratio table and tells you where you get
  beat the worst:

```
benchmark                 n         C ms     LOGOS ms      ratio
----------------------------------------------------------------
loop_sum          500000000       1017.3       1210.1      1.19x
...

Here's where C beats LOGOS the worst:
  loop_sum       LOGOS is 1.19x slower (C 1017.3ms vs LOGOS 1210.1ms at n=500000000)
    -> ONLY=loop_sum DUMPS=1 bash benchmarks/run-logos-vs-c.sh  # then diff asm/...

LOGOS wins 18 / 32 benchmarks against C
Geometric mean: LOGOS runs at 0.97x the speed of C
```

Everything is teed to `logs/optimization/logos-vs-c.log`.

With `DUMPS=1` a build also leaves a full diagnostic trail per benchmark
(slow — 2 extra cargo builds per LOGOS benchmark, so combine with `ONLY=`):

| Artifact | Path | What it tells you |
|----------|------|-------------------|
| Generated Rust | `generated/<bench>.rs` | What largo emitted — the first place to look |
| LOGOS assembly | `asm/<bench>_logos.s` | What LLVM made of the generated Rust |
| LOGOS LLVM IR | `asm/<bench>_logos.ll` | Pre-codegen IR for the LOGOS side |
| C assembly | `asm/<bench>_c.s` | The target to beat |
| C LLVM IR | `asm/<bench>_c.ll` | Pre-codegen IR for the C side |

A typical iteration on one slow benchmark:

```bash
bash benchmarks/run-logos-vs-c.sh                       # the gist: full table, ~minutes
ONLY=loop_sum DUMPS=1 bash benchmarks/run-logos-vs-c.sh # deep-dive one bench
less benchmarks/generated/loop_sum.rs                   # inspect codegen
diff benchmarks/asm/loop_sum_c.s benchmarks/asm/loop_sum_logos.s
# ...change the compiler...
ONLY=loop_sum VERIFY=0 bash benchmarks/run-logos-vs-c.sh  # re-measure, seconds
```

### Knobs

| Variable | Default | Meaning |
|----------|---------|---------|
| `ONLY` | all 32 | Comma-separated benchmark subset, e.g. `ONLY=loop_sum,collect` |
| `RUNS` | 10 | Timed runs per side |
| `WARMUP` | 2 | Warmup runs per side |
| `SIZE_<bench>` | calibrated | Per-benchmark size override, e.g. `SIZE_fib=42`. Sizes without an `expected_<n>.txt` are verified by C/LOGOS output agreement |
| `VERIFY` | 1 | `VERIFY=0` skips the correctness phase (one full-size run per side) when iterating hard |
| `DUMPS` | — | `DUMPS=1` emits asm + LLVM IR for both sides into `asm/` |
| `FORCE_BASELINE` | — | `FORCE_BASELINE=1` re-measures cached C baselines |
| `OUT` | `results/local-logos-vs-c.json` | Output JSON path |
| `FORCE_BUILD` | — | `FORCE_BUILD=1` rebuilds every binary |

## The coverage check: every language, every benchmark

```bash
bash benchmarks/run-quick.sh
```

Builds all implementations, verifies every output, times each pair once at
a small size, and prints the full coverage matrix:

```
benchmark            n         C       C++      Rust       Zig   ...     LOGOS
-------------------------------------------------------------------------------
fib                 25       2.0       1.8       3.0       1.9   ...       2.5
sieve           100000       2.5       4.8       3.0       2.1   ...       3.8
```

Cells show mean milliseconds, or why a pair is missing: `T/O` (timed out),
`FAIL` (wrong output), `ERR` (crashed), `BUILD` (compile failed), `NOSRC`
(no source file), `-` (toolchain missing or skipped). The script exits
non-zero if any pair is broken. Results land in `results/local.json` in the
exact schema of `latest.json` (set `OUT=results/latest.json` to preview in
the web frontend — revert before committing).

The quick sizes are deliberately tiny: they answer "does everything run
everywhere", not "how fast is it" — process startup dominates at a few
milliseconds. For speed questions use run-logos-vs-c.sh, or `SIZE=ref` /
`SIZE=max` here.

The first run builds everything and takes a while (32 largo release builds
dominate); after that, build caching makes reruns mostly measurement time.
Knobs: `SIZE=quick|ref|max`, `SIZE_<bench>=N`, `ONLY`, `RUNS` (default 10),
`WARMUP` (default 3), `SKIP_LANGS`, `OUT`, `FORCE_BUILD` — same semantics
as above. Sizes without an `expected_<n>.txt` are verified by
cross-language agreement (C's output is the reference all others must
match), so arbitrary sizes stay safe and double as a consistency check.

## Layout

```
benchmarks/
├── setup-local.sh        # One-time toolchain installer (matches CI versions)
├── run-logos-vs-c.sh     # LOGOS vs C head-to-head + asm/LLVM dumps
├── run-quick.sh          # Full language x benchmark coverage matrix
├── run.sh                # Full CI suite: multi-size scaling + compile times
├── verify.sh             # Correctness-only sweep
├── programs/<bench>/     # One dir per benchmark
│   ├── main.{c,cpp,rs,zig,go,js,py,rb,nim,lg} + Main.java
│   ├── sizes.txt         # The sizes run.sh sweeps (space-separated)
│   └── expected_<n>.txt  # Reference output per size
├── generated/<bench>.rs  # Rust emitted by largo for each LOGOS program
├── asm/                  # Assembly + LLVM IR dumps for C and LOGOS
└── results/
    ├── latest.json       # CI-produced data embedded by the web frontend
    ├── history/v*.json   # One snapshot per release
    ├── local.json        # run-quick.sh output (never clobbers latest.json)
    └── local-logos-vs-c.json
```

## Full suite (the publishable run)

```bash
bash benchmarks/run.sh
```

The complete pipeline: builds everything (plus a LOGOS debug build), verifies
at reference sizes, sweeps every size in each benchmark's `sizes.txt` with
per-language timeout isolation, measures compile times, and writes
`results/latest.json`. This runs on the dedicated bench box — never CI, whose
shared runners are too noisy for publishable numbers — with the box silenced
and all 11 languages included; commit the refreshed `results/` afterwards.
Takes hours — use the local scripts for iteration.

The `languages` array in the output JSON only lists languages that actually
produced results, so skipped languages never show up as data-less gaps on
the frontend benchmarks page.

## Fairness ground rules

Every language implements the **same algorithm with the same approach**:
hand-rolled sorts and hash patterns (no stdlib `sort()` shortcuts), the same
LCG RNG with the same seed for generated inputs, the same numeric widths
(64-bit ints, doubles), and the same printed output. Compiled languages use
their standard release flags (`gcc -O2`, `g++ -O2`, `rustc -O`,
`zig -O ReleaseFast`, `nim -d:release`, `go build`, `largo build --release`).
What each compiler does with that is the benchmark. If you add or change an
implementation, keep it structurally identical to `main.c` and regenerate or
add `expected_<n>.txt` entries for the sizes in `sizes.txt`.

## Adding a benchmark

1. Create `programs/<name>/` with all 11 implementations reading `n` from
   the first CLI argument and printing the same checksum line.
2. Add `sizes.txt` (2–4 space-separated sizes) and an `expected_<n>.txt`
   per size (generate with the C implementation).
3. Register `<name>` in the `BENCHMARKS` array of `run.sh`, `run-quick.sh`,
   and `run-logos-vs-c.sh`, plus `bench_name`/`bench_desc` entries and a
   `quick_size`/`ref_size`/`bench_size` calibration in each.
4. `ONLY=<name> bash benchmarks/run-quick.sh` until the matrix row is green.
