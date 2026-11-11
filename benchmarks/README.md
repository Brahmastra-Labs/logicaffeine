# LOGICAFFEINE Benchmarks

Cross-language benchmark suite: 35 programs, each implemented in C, C++,
Rust, Zig, Go, Java, JavaScript, Nim, Python, Ruby and LOGOS, measured with
[hyperfine](https://github.com/sharkdp/hyperfine) and verified for output
correctness before any timing happens. The publishable run times every row of
`lib/languages.tsv`: the compiled and managed languages, C and C++ through
both gcc and clang, and LOGOS on every tier. Python and Ruby are verified, and
timed only by `run-quick.sh`.

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

Installs every missing toolchain: clang, Go/JDK/Ruby via apt, plus Zig
0.15.2, Nim 2.0.4, and hyperfine 1.18.0, pinned so every bench box measures
with the same tools. Rust is expected to already be present via rustup.
Idempotent — tools already on PATH are left alone. clang builds the C (clang)
and C++ (clang++) rows and the C-side assembly/LLVM dumps.

## The optimization loop: LOGOS vs C

```bash
bash benchmarks/run-logos-vs-c.sh
```

Answers "where does LOGOS win and where does C win" as fast as possible:

- **Both sides build as the publishable run builds them.** `suite_build`
  (`lib/suite.sh`) runs the command `lib/languages.tsv` gives each row and
  reuses a binary only when that same command and toolchain built it; every
  timing runs pinned to the performance cores.
- **C baselines are static.** They're measured once at each benchmark's
  development size (`dev_size` in `lib/benchmarks.tsv`: hundreds of
  milliseconds of C runtime, so process startup is noise) and cached in
  `results/c-baselines.json` beside the SHA-256 of the binary they timed. A
  baseline is re-measured whenever the C binary is not that one — `main.c`,
  its build command or gcc changed — or you pass `FORCE_BASELINE=1`. A typical
  run measures *only the LOGOS side*.
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
```

It closes with how many benchmarks LOGOS wins and the geometric mean of its
speed relative to C.

Everything is teed to `logs/optimization/logos-vs-c.log`.

With `DUMPS=1` a build also leaves a full diagnostic trail per benchmark
(slow — a second LOGOS build and two `cargo rustc --emit` passes per
benchmark, so combine with `ONLY=`):

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
| `ONLY` | all | Comma-separated benchmark subset, e.g. `ONLY=loop_sum,collect` |
| `RUNS` | 10 | Timed runs per side |
| `WARMUP` | 2 | Warmup runs per side |
| `SIZE_<bench>` | `dev_size` | Per-benchmark size override, e.g. `SIZE_fib=42`. Sizes without an `expected_<n>.txt` are verified by C/LOGOS output agreement |
| `VERIFY` | 1 | `VERIFY=0` skips the correctness phase (one full-size run per side) when iterating hard |
| `DUMPS` | — | `DUMPS=1` emits asm + LLVM IR for both sides into `asm/` |
| `FORCE_BASELINE` | — | `FORCE_BASELINE=1` re-measures cached C baselines |
| `OUT` | `results/local-logos-vs-c.json` | Output JSON path |
| `FORCE_BUILD` | — | `FORCE_BUILD=1` rebuilds every binary, even one `suite_build` would reuse |

## The coverage check: every language, every benchmark

```bash
bash benchmarks/run-quick.sh
```

Builds every implementation with the publishable run's commands
(`suite_build`), verifies every output, times each pair at a small size
pinned to the performance cores, and prints the full coverage matrix:

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

The first run builds everything and takes a while (a largo release build per
benchmark dominates); after that, build caching makes reruns mostly
measurement time.
Knobs: `SIZE=quick|ref|max`, `SIZE_<bench>=N`, `ONLY`, `RUNS` (default 10),
`WARMUP` (default 3), `SKIP_LANGS`, `OUT`, `FORCE_BUILD` — same semantics
as above. Sizes without an `expected_<n>.txt` are verified by
cross-language agreement (C's output is the reference all others must
match), so arbitrary sizes stay safe and double as a consistency check.

## Layout

```
benchmarks/
├── setup-local.sh        # One-time toolchain installer
├── run-logos-vs-c.sh     # LOGOS vs C head-to-head + asm/LLVM dumps
├── run-quick.sh          # Full language x benchmark coverage matrix
├── run.sh                # The publishable suite: multi-size scaling + compile times (bench box only)
├── run-interp-vs-js.sh   # The interpreter tiers vs Node, and cold start
├── run-solver-vs-z3.sh   # The certified SAT solver vs Z3, Kissat, CaDiCaL, CryptoMiniSat → results/solvers.json
├── verify.sh             # Correctness-only sweep
├── compare-runs.py       # Judges a run against earlier runs: regressions on one chip, ratios across chips
├── gen-readme-charts.py  # Draws results/charts/*.svg and records every figure in results/charts/headlines.json
├── lib/benchmarks.tsv    # The registry: every program, its sizes, its collapse note
├── lib/languages.tsv     # Every timed row: its build, run and version commands
├── lib/suite.sh          # Loads the registry; suite_build, the one way every script builds an implementation
├── lib/largo.sh          # largo_build: the one way every script builds largo
├── programs/<bench>/     # One dir per benchmark
│   ├── main.{c,cpp,rs,zig,go,js,py,rb,nim,lg} + Main.java
│   ├── interp.lg         # The program the interpreter tests run (untimed)
│   ├── main_word64.lg    # Integer benchmarks: the Word64 variant, locked byte-identical to main.lg
│   ├── sizes.txt         # The sizes run.sh sweeps (space-separated)
│   └── expected_<n>.txt  # Reference output per size
├── arena/                # SAT competition arena: instance fetch/generation and proof generation, the crypto arena
├── eda/                  # The EDA arenas (run-eda.sh): model checking, BMC, rewriting, counting → results/eda/*.json
├── sat/                  # Pinned external solver builds and the SAT benchmark driver
├── parity/               # The stdlib parity ledger and its benchmarks (PARITY_LEDGER.md)
├── profile/              # Profiles of the interpreter tiers (folded stacks, flamegraphs)
├── silicon/              # Hardware ledgers, RTL and signoff records for the silicon backends
├── generated/<bench>.rs  # Rust emitted by largo for each LOGOS program (not tracked)
├── asm/                  # Assembly + LLVM IR dumps for C and LOGOS (not tracked)
└── results/
    ├── latest.json       # The published run's data, embedded by the web frontend
    ├── latest-interp.json, latest-codec.json, solvers.json   # The interpreter, codec and solver runs
    ├── arena/            # The SAT arenas (sat.json, scale.json) and the crypto arena (crypto.json)
    ├── eda/              # mc.json, rewrite.json, count.json: the EDA section of the benchmarks page
    ├── charts/           # The README charts + headlines.json (readme_benchmark_numbers_lock)
    ├── history/v*.json   # One snapshot per release
    └── local.json        # run-quick.sh output (never clobbers latest.json)
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
and every row of `lib/languages.tsv` included; commit the refreshed
`results/` afterwards.
Takes hours — use the local scripts for iteration.

The `languages` array in the output JSON only lists languages that actually
produced results, so skipped languages never show up as data-less gaps on
the frontend benchmarks page.

## Comparing runs

```bash
python3 benchmarks/compare-runs.py <results-dir> [--against <results-dir>]...
```

Says whether a run regressed. Each directory is one `run.sh` writes
(`BENCH_RESULTS_DIR`, or `results/` itself); the baseline defaults to
`results/`, the published run, and every `--against` is its own comparison.
Each suite is measured against its yardstick (C, Node, postcard) and only the
LOGOS rows are judged:

- **On the same chip** a row regresses when it is slower beyond the noise
  band *and* slower relative to the yardstick, so a machine that slowed down
  across the board reports drift, not a regression. A row that lost its
  timing regresses.
- **On another chip** only the yardstick-normalized ratios are compared: a
  row out of line with the baseline is a finding for that chip, never a
  failure, and the report gives the chip's own speed against the baseline.
- **Codec sizes** are deterministic, so a changed size at the same commit
  regresses on any chip.

It prints a Markdown report (`--report` saves it, `--json` writes the
verdict for tools) and exits 1 on a regression. `--tolerance` and
`--cross-tolerance` set the bands; the script's header gives the exact rules.

## Fairness ground rules

**The same algorithm, doing the same work.** Every implementation runs the
algorithm `main.c` runs, step for step. Sorts and hash tables are hand-rolled
in every language: collect and two_sum each build the same open-addressing
table (flat slots, Fibonacci hashing, linear probing, a power-of-two capacity
at least twice n) rather than a standard-library map. Generated inputs come
from the same LCG with the same seed, integers are as wide as `main.c` makes
them, and every program prints the same bytes. What C allocates once up front,
every language allocates once up front, and no implementation formats,
decodes or allocates through a general-purpose path in a hot loop where C
does not. Output is verified byte for byte at every timed size before
anything is timed.

**The same build, in every script.** A build command lives in one place, the
`build` column of `lib/languages.tsv`, and every script — `run.sh`,
`run-quick.sh`, `run-logos-vs-c.sh`, `calibrate-c.sh`, `verify.sh`,
`run-logos-vs-interp.sh` and the stdlib parity gate — builds through
`suite_build` (`lib/suite.sh`), which reuses a binary only when that same
command and toolchain built it. Every compiled row builds at its fastest
release setting for the host CPU:

| Row | Build |
|-----|-------|
| C, C++ (gcc, clang) | `-O3 -march=native -flto -ffp-contract=off`, C++ with `-std=c++17` |
| Rust | `-C opt-level=3 -C lto=fat -C codegen-units=1 -C target-cpu=native -C panic=abort` |
| Zig | `-O ReleaseFast -mcpu native` |
| Go | `go build` at the host's `GOAMD64` level |
| Nim | `-d:danger`, its C at `-O3 -march=native -flto -ffp-contract=off` |
| Java, JavaScript | `javac` and the default JVM; `node --stack-size=4000` |
| LOGOS | `largo build --release` (the Rust settings above), `largo build --release --emit c` (the C flags above), the JIT, VM and tree-walker |

C, C++ and Nim compile with floating-point contraction off: Rust, Zig, Go,
Java, JavaScript and every LOGOS tier round each operation, and a C that fused
`a*b + c` would compute different values (and run faster for it).

**The same machine.** `run.sh` refuses a busy box (load, competing processes,
a non-performance governor) and times every command pinned to the performance
cores with warmup runs; the development scripts pin the same way.

**What LOGOS's compiler does with the same program.** Every `main.lg` is a
natural LOGOS program doing what `main.c` does; none is contorted to trigger
an optimization. Two kinds of compiler work make a LOGOS binary more than a
literal translation, and both are disclosed:

- *Collapses.* For some benchmarks the optimizer replaces the algorithm with
  an equivalent one that does asymptotically less work: memoizing a pure
  recursion, closing a loop into a formula, sieving a per-number search. Each
  carries a note in `lib/benchmarks.tsv` saying what the optimizer did
  (`benchmark_collapse_lock` holds every such optimization to one), and every
  headline figure is published both with them and over the same-algorithm
  benchmarks alone.
- *Representation.* With the algorithm unchanged, LOGOS chooses storage that
  the C program spells out: an `Int` sequence or map key whose values it
  proves fit in 32 bits is stored in 32 bits (`narrow`, `narrowmap`,
  `elemtype`), a local `Seq of Bool` is held one bit per item (`bitpack`), and
  the remainder of a value proven below twice its divisor is one conditional
  subtraction (`modwrap`). These count toward the same-algorithm figure: they
  are what a compiler that proves value ranges does, and what a C compiler may
  not do to a `long` array whose layout the program fixed. To measure without
  them, build with `LOGOS_OPT_OFF=narrow,narrowmap,elemtype,bitpack,modwrap`.

## Adding a benchmark

1. Create `programs/<name>/` with every implementation —
   `main.{c,cpp,rs,zig,go,js,py,rb,nim,lg}`, `Main.java` and `interp.lg` —
   each reading `n` from the first argument and printing the same output.
2. Add `sizes.txt` (the timed sizes, space-separated) and generate an
   `expected_<n>.txt` for each, and for every other size its registry row
   names, with `bash benchmarks/gen-expected.sh <name> <n>…` — written only
   when the C, C++ and Rust ports print the same bytes.
3. Add its row to `lib/benchmarks.tsv`: name, description, declared
   complexity, every size column, what its output proves, and the collapse
   note when the LOGOS optimizer does asymptotically less work. Every script
   reads the registry; there is no other list to update.
4. `ONLY=<name> bash benchmarks/run-quick.sh` until the matrix row is green.
