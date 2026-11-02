#!/usr/bin/env bash
# Per-capability performance gate for the stdlib-parity campaign.
#
#   scripts/bench/bench-parity.sh <bench-dir> [n]
#
# Builds the pure-LOGOS implementation of a stdlib capability (which auto-imports the stdlib
# module — it does NOT define the function inline) and its C/Python/Rust/Go counterparts, checks
# they all produce the same output, then hyperfine-benchmarks them, pinned to the performance
# cores. Every implementation builds exactly as the benchmark suite builds it: suite_build, with
# the commands benchmarks/lib/languages.tsv publishes, and a counterpart whose source is present
# but whose build fails stops the gate rather than dropping out of the comparison. LOGOS must be
# STRICTLY FASTER than every counterpart. Writes benchmarks/parity/bench/results/<capability>.json — the committed
# evidence parity_bench_lock.rs verifies against the catalog's perf.speedup.
#
# Requires: cargo (largo is built from this tree by benchmarks/lib/largo.sh), gcc, rustc, go, python3,
# hyperfine. Run on a QUIET box for meaningful numbers (a busy box inflates variance).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DIRNAME="${1:?usage: bench-parity.sh <bench-dir> [n]}"
BENCH="$ROOT/benchmarks/parity/bench/$DIRNAME"
[ -d "$BENCH" ] || { echo "no bench dir: $BENCH" >&2; exit 1; }

CAP="$(python3 -c "import json;print(json.load(open('$BENCH/meta.json'))['capability'])")"
EXPECTED="$(python3 -c "import json;print(json.load(open('$BENCH/meta.json'))['expected'])")"
N="${2:-$(python3 -c "import json;print(json.load(open('$BENCH/meta.json'))['arg'])")}"
OUTDIR="$ROOT/benchmarks/parity/bench/results"
BIN="$BENCH/bin"
mkdir -p "$OUTDIR" "$BIN"

export LOGOS_WORKSPACE="$ROOT"

# The suite's builders, over this capability's directory. Sourcing them points CARGO_TARGET_DIR at the benchmark
# suite's target dir, so the generated program's runtime deps (logicaffeine_data/_system) are already compiled —
# no cold multi-minute build.
. "$ROOT/benchmarks/lib/suite.sh"
SUITE_PROGRAMS="$ROOT/benchmarks/parity/bench"
SUITE_BIN="$BIN"
largo_build || exit 1

# ---- Build LOGOS (native, via largo; uses the stdlib capability by auto-import) and every counterpart present ----
declare -A CMD
for id in logos_release c rust go; do
    suite_has "$DIRNAME" "$id" || continue
    echo "Building ${LANG_LABEL[$id]} ..."
    log="$(mktemp)"
    if ! suite_build "$DIRNAME" "$id" > "$log" 2>&1; then
        cat "$log" >&2; rm -f "$log"
        echo "the ${LANG_LABEL[$id]} build of $DIRNAME failed" >&2
        exit 1
    fi
    rm -f "$log"
    CMD[${id%_release}]="$(suite_bin "$DIRNAME" "$id") $N"
done
[ -n "${CMD[logos]:-}" ] || { echo "no main.lg in $BENCH" >&2; exit 1; }
[ -f "$BENCH/main.py" ] && CMD[python]="python3 $BENCH/main.py $N"

# ---- Correctness gate: every impl must produce the expected output -----------------------
echo "Verifying correctness (expected $EXPECTED) ..."
for lang in "${!CMD[@]}"; do
    got="$(${CMD[$lang]} | tr -d '[:space:]')"
    [ "$got" = "$EXPECTED" ] || { echo "  $lang produced $got, expected $EXPECTED" >&2; exit 1; }
    echo "  $lang ok"
done

# ---- Benchmark: OP-DOMINATED timing --------------------------------------------------------
# A fast op at the default n runs sub-50ms, where PROCESS STARTUP (not the op) dominates the
# wall-clock — reporting a binary-startup edge as a false op-speed "win" (verified: sqrt/floor
# "win" at n=1M but TIE/LOSE at n=100M). Fix: scale n up until LOGOS wall >> startup (~300ms
# target), so wall-clock reflects the OP. Correctness stays gated at the meta n above; only the
# TIMING uses the larger n (its output is timed, not checked). Then subtract a tiny-n baseline to
# remove the residual startup.
ORDER=(logos c rust go python)
echo "Calibrating op-dominated n ..."
# Microsecond-precision calibration (/usr/bin/time's 0.01s resolution reads sub-10ms as 0.00 → no
# scaling → still startup-dominated). Scale n so LOGOS wall hits ~0.3s (op >> ~1-5ms startup).
read -r BASE_WALL TN < <(python3 -c "
import subprocess,time
n=int('$N')
t=time.perf_counter()
subprocess.run(['$BIN/logos', '$N'], stdout=subprocess.DEVNULL)
w=time.perf_counter()-t
scale=min(max(0.3/max(w,1e-4),1.0),1000.0)
print(round(w,4), max(min(int(n*scale),2000000000),n))
")
echo "  meta n=$N, LOGOS wall=${BASE_WALL}s -> op-dominated timing n=$TN"
# Contention-robust: 15 runs and take the MIN per command (the least-interfered sample) rather than
# the mean, so a load spike hitting one language's run-block cannot skew the ratio. All languages are
# already timed in ONE hyperfine invocation (same window/build env); min-of-many makes the ratio hold
# even under sustained box load.
HF_N=(--warmup 2 --min-runs 15 --export-json "$OUTDIR/.hf_n.json")
HF_B=(--warmup 2 --min-runs 15 --export-json "$OUTDIR/.hf_b.json")
for lang in "${ORDER[@]}"; do
    [ -n "${CMD[$lang]:-}" ] || continue
    base="${CMD[$lang]% *}"
    HF_N+=(-n "$lang" "$base $TN")
    HF_B+=(-n "$lang" "$base 1")
done
taskset -c "$(suite_cpus)" hyperfine "${HF_N[@]}"
taskset -c "$(suite_cpus)" hyperfine "${HF_B[@]}" >/dev/null

# ---- Verdict + result artifact -----------------------------------------------------------
CAP="$CAP" N="$N" TN="$TN" OUTDIR="$OUTDIR" ROOT="$ROOT" python3 - <<'PY'
import json, os, subprocess
cap, n, outdir, root = os.environ["CAP"], os.environ["N"], os.environ["OUTDIR"], os.environ["ROOT"]
timing_n = os.environ.get("TN", n)
# MIN per command (least-contended run) — contention-robust; mean would let a spike in one command's
# run-block skew the cross-command ratio.
min_n = {r["command"]: r["min"] for r in json.load(open(f"{outdir}/.hf_n.json"))["results"]}
min_b = {r["command"]: r["min"] for r in json.load(open(f"{outdir}/.hf_b.json"))["results"]}
# op-time = min-wall(N) - min-wall(1); clamp tiny/negative deltas to a 0.1ms floor (below hyperfine noise).
FLOOR = 0.0001
by = {k: max(min_n[k] - min_b.get(k, 0.0), FLOOR) for k in min_n}
logos = by["logos"]
speedups = {k: round(v / logos, 3) for k, v in by.items() if k != "logos"}
# Genuinely fastest ONLY if LOGOS's op-time beats every rival by a real margin AND the op-time
# is above the noise floor for at least one rival (else all are startup — no measurable op win).
measurable = any(v > FLOOR * 2 for k, v in by.items() if k != "logos")
fastest = measurable and all(v > logos * 1.02 for k, v in by.items() if k != "logos")
cpu = "unknown"
try:
    for line in open("/proc/cpuinfo"):
        if line.startswith("model name"):
            cpu = line.split(":", 1)[1].strip(); break
except Exception:
    pass
res = {
    "capability": cap, "arg": n, "timing_n": timing_n, "cpu": cpu,
    "method": "op-dominated, contention-robust: op-time = MIN-wall(timing_n) - MIN-wall(1) over 15 runs, timing_n scaled so wall>>startup",
    "means_ms": {k: round(v * 1000, 3) for k, v in by.items()},          # op-time (min-run, startup removed)
    "wall_ms": {k: round(min_n[k] * 1000, 3) for k in min_n},            # min raw wall(N) for reference
    "logos_ms": round(logos * 1000, 3),
    "speedups_vs_logos": speedups,
    "logos_strictly_fastest": fastest,
    "min_speedup": round(min(speedups.values()), 3) if speedups else None,
}
json.dump(res, open(f"{outdir}/{cap}.json", "w"), indent=2)
os.remove(f"{outdir}/.hf_n.json")
os.remove(f"{outdir}/.hf_b.json")
print("\n=== VERDICT ===")
print(f"  capability: {cap}   n={n}   cpu={cpu}")
print(f"  LOGOS: {res['logos_ms']} ms")
for k, s in sorted(speedups.items(), key=lambda kv: -kv[1]):
    print(f"  vs {k:8} {res['means_ms'][k]:9.2f} ms  -> LOGOS {s:.2f}x {'faster' if s>1 else 'SLOWER'}")
print(f"  LOGOS strictly fastest over ALL measured: {fastest}")
print(f"  result -> benchmarks/parity/bench/results/{cap}.json")
if not fastest:
    raise SystemExit("FAIL: LOGOS is not strictly fastest — the perf gate is not met")
PY
