#!/usr/bin/env bash
# tt-matmul-measure — the ONLY path from a ttsim run to a row in benchmarks/silicon/tenstorrent_gemm.json.
#
# Every mode emits/builds/runs ONE program under ttsim (scripts/tt-sim.sh), captures the full log, and hands it
# to scripts/tt-gemm-ledger.py, which refuses to record anything without a PASS, with a ttsim ERROR, or whose
# printed numerics differ from what was requested. Nothing here types a number.
#
#   scripts/tt-matmul-measure.sh check-env
#   scripts/tt-matmul-measure.sh spatial --shape M N K --chip … [--candidate best|streamed|fused|unfused] \
#                                        --fidelity F --fp32 0|1 --l1acc 0|1 [--headline]   # the spatial-IR lowering (every kernel generated)
#   scripts/tt-matmul-measure.sh p4      --workload rmsnorm|layernorm|swiglu|softmax|elementwise [--rows R --cols C] --chip … \
#                                        [--bundle DIR]   # P4 correctness (PCC only, no ledger row); --bundle installs an already-printed bundle instead of emitting
#   scripts/tt-matmul-measure.sh example --name matmul_single_core|matmul_multi_core|matmul_multicore_reuse|matmul_multicore_reuse_mcast --chip …
#   scripts/tt-matmul-measure.sh ttnn    --shape M N K --chip … --config default|ours --fidelity F --fp32 0|1 --l1acc 0|1 \
#                                        [--grid GxG --per-core AxB --in0-block-w N --subblock AxB] [--scaffold]
#   scripts/tt-matmul-measure.sh all     --shape 640 640 640 --chip wormhole      # the canonical campaign, in order
#
# Env: TT_METAL_HOME (default ~/tt-metal), LEDGER (default benchmarks/silicon/tenstorrent_gemm.json),
#      LOGOS_TT_LOG_DIR (default logs/tt). cmake 3.28 is forced onto PATH (4.0.x breaks the tt-metal build).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TT_METAL_HOME="${TT_METAL_HOME:-$HOME/tt-metal}"
LEDGER="${LEDGER:-$ROOT/benchmarks/silicon/tenstorrent_gemm.json}"
LOG_DIR="${LOGOS_TT_LOG_DIR:-$ROOT/logs/tt}"
BUILD_DIR="$TT_METAL_HOME/build_Release"
EXAMPLES="$TT_METAL_HOME/tt_metal/programming_examples/matmul"
LOGOS_DIR="$EXAMPLES/logos_matmul"
SPATIAL_DIR="$EXAMPLES/logos_spatial"
LEDGER_PY="$ROOT/scripts/tt-gemm-ledger.py"
SIM_SH="$ROOT/scripts/tt-sim.sh"
TTNN_PY="$ROOT/scripts/accel/tt-ttnn-matmul.py"
export PATH="/usr/bin:$PATH"     # cmake 3.28 before /usr/local's 4.0.2
export TT_METAL_HOME

die() { echo "tt-matmul-measure: $*" >&2; exit 2; }
stamp() { date -u +%Y%m%dT%H%M%SZ; }

check_env() {
  local chip="${1:-wormhole}"
  [ -d "$BUILD_DIR" ] || die "tt-metal not built at $BUILD_DIR"
  [ -x "$(command -v cmake)" ] || die "cmake missing"
  case "$(cmake --version | head -1)" in *" 3."*) ;; *) die "cmake must be 3.x (got: $(cmake --version | head -1)); /usr/bin/cmake 3.28 is required, 4.0.x breaks the tt-metal build" ;; esac
  [ -f "$LEDGER_PY" ] && [ -f "$SIM_SH" ] || die "ledger tool / sim script missing"
  "$SIM_SH" --chip "$chip" /bin/true >/dev/null || die "scripts/tt-sim.sh --chip $chip cannot run (simulator .so / SOC descriptor / build dir)"
  local venv="${TTNN_PYTHON:-$HOME/ttnn-venv/bin/python}"
  [ -x "$venv" ] || die "ttnn venv python missing: $venv"
  "$venv" -c "import numpy" 2>/dev/null || die "numpy missing in $venv"
  PYTHONPATH="$TT_METAL_HOME/ttnn:$TT_METAL_HOME/tools" "$venv" -c "import ttnn" 2>/dev/null || die "ttnn does not import from $TT_METAL_HOME/ttnn under $venv"
  grep -q 'add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/logos_matmul)' "$EXAMPLES/CMakeLists.txt" || die "logos_matmul is not registered in $EXAMPLES/CMakeLists.txt (sed -i '1i add_subdirectory(\${CMAKE_CURRENT_SOURCE_DIR}/logos_matmul)' …)"
  echo "check-env ok: chip=$chip build=$BUILD_DIR ledger=$LEDGER"
}

scan_examples() {
  python3 "$LEDGER_PY" scan-examples --ledger "$LEDGER" --tt-metal-home "$TT_METAL_HOME"
}

# run_sim <slug> <chip> <sim args…>  → sets RUN_LOG and RUN_EXIT (never aborts on the program's exit code).
# The device clock ticks while the host JIT-compiles a kernel (measured: 9,735 warm vs 13,543 cold for one
# program), so a run whose `JIT cache stats` line shows misses is a WARM-UP: it is kept as a log but the run is
# repeated once and the warm repeat is what gets recorded. The ledger refuses a cold row regardless.
run_sim() {
  local slug="$1" chip="$2"; shift 2
  mkdir -p "$LOG_DIR"
  local attempt
  for attempt in 1 2; do
    local suffix=""
    if [ "$attempt" = 2 ]; then suffix="__warm"; fi
    RUN_LOG="$LOG_DIR/${slug}__$(stamp)${suffix}.full.log"
    set +e
    "$SIM_SH" --chip "$chip" "$@" > "$RUN_LOG" 2>&1
    RUN_EXIT=$?
    set -e
    echo "run: $RUN_LOG (exit $RUN_EXIT)"
    if [ "$RUN_EXIT" = 0 ] && grep -qE 'JIT cache stats: ([0-9]+)/\1 hits' "$RUN_LOG"; then
      return 0
    fi
    if [ "$RUN_EXIT" != 0 ]; then
      return 0
    fi
    echo "  JIT cache was cold ($(grep -oE 'JIT cache stats: [0-9]+/[0-9]+ hits' "$RUN_LOG")) — warm-up only; re-running for the record"
  done
}

build_target() {
  local target="$1"
  cmake --build "$BUILD_DIR" --target "$target" 2>&1 | tail -3
}

# The spatial-IR lowering: `emit_spatial_tensix` writes the whole bundle (host + every kernel + CMake + oracle);
# the bundle is registered as a programming example once (idempotent), built, run, and recorded as
# program `logos_spatial` with the arrangement the host printed as its strategy.
register_spatial_example() {
  local line='add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/logos_spatial)'
  grep -qF "$line" "$EXAMPLES/CMakeLists.txt" || sed -i "1i $line" "$EXAMPLES/CMakeLists.txt"
}

cmd_spatial() {
  local shape=() chip=wormhole candidate=best fidelity=HiFi4 fp32=1 l1acc=0 packing="" percore="" blk="" headline=()
  while [ $# -gt 0 ]; do
    case "$1" in
      --shape) shape=("$2" "$3" "$4"); shift 4 ;;
      --chip) chip="$2"; shift 2 ;;
      --candidate) candidate="$2"; shift 2 ;;
      --fidelity) fidelity="$2"; shift 2 ;;
      --fp32) fp32="$2"; shift 2 ;;
      --l1acc) l1acc="$2"; shift 2 ;;
      --packing) packing="$2"; shift 2 ;;
      --per-core) percore="$2"; shift 2 ;;
      --block) blk="$2"; shift 2 ;;
      --headline) headline=(--headline); shift ;;
      *) die "spatial: unknown arg $1" ;;
    esac
  done
  [ ${#shape[@]} -eq 3 ] || die "spatial: --shape M N K required"
  # The pipeline emits the fidelity its TARGET is declared at, and the target can now be declared at any of
  # them (`SpatialTarget::at_fidelity`). This used to hard-refuse anything but HiFi4, which meant we could not
  # emit at ttnn's own bf16 default and had nothing of ours to compare at those knobs. The invariant that
  # mattered is untouched: the program still prints only what it ran, because the fidelity is passed to the
  # emitter rather than asserted over it.
  case "$fidelity" in LoFi|HiFi2|HiFi3|HiFi4) ;; *) die "spatial: --fidelity LoFi|HiFi2|HiFi3|HiFi4" ;; esac
  case "$candidate" in best|streamed|fused|unfused) ;; *) die "spatial: --candidate best|streamed|fused|unfused" ;; esac
  local policy=exact
  [ "$fp32" = 1 ] || policy=relaxed
  local repro="scripts/tt-matmul-measure.sh spatial --shape ${shape[*]} --chip $chip --candidate $candidate --fidelity $fidelity --fp32 $fp32 --l1acc $l1acc${packing:+ --packing $packing}${percore:+ --per-core $percore}${blk:+ --block $blk}"
  cargo run -q -p logicaffeine-compile --features codegen --example emit_spatial_tensix -- "$SPATIAL_DIR" "${shape[@]}" "$chip" --fidelity "$fidelity" --grid "$(sim_grid "$chip")" --policy "$policy" --candidate "$candidate" --l1acc "$l1acc" ${packing:+--packing "$packing"} ${percore:+--per-core "$percore"} ${blk:+--block "$blk"} \
    || die "emit failed"
  register_spatial_example
  build_target metal_example_logos_spatial
  run_sim "spatial__${shape[0]}x${shape[1]}x${shape[2]}__${candidate}__${fidelity}-fp32_${fp32}-l1acc_${l1acc}" "$chip" logos_spatial
  python3 "$LEDGER_PY" add --ledger "$LEDGER" --stack logos_emitted --log "$RUN_LOG" --exit-code "$RUN_EXIT" \
    --program logos_spatial --program-origin "compiler-emitted, all kernels generated" --generated compute,dataflow,host \
    --shape "${shape[@]}" --chip "$(chip_id "$chip")" --reproduce "$repro" \
    --requested-knobs "$fidelity,$fp32,$l1acc" --program-src "$SPATIAL_DIR/host.cpp" "$SPATIAL_DIR/kernels" \
    --input-fill "deterministic integer fills (p%7-3, p%5-2) carried in bf16, golden = fp32 host matmul" \
    "${headline[@]}"
}

cmd_example() {
  local name="" chip=wormhole
  while [ $# -gt 0 ]; do
    case "$1" in
      --name) name="$2"; shift 2 ;;
      --chip) chip="$2"; shift 2 ;;
      *) die "example: unknown arg $1" ;;
    esac
  done
  [ -n "$name" ] || die "example: --name required"
  scan_examples
  local src="$EXAMPLES/$name/$name.cpp"
  [ -f "$src" ] || die "no example source $src"
  # The example's active shape is the constexpr branch its build takes (TT_METAL_CI_MODE for the mcast one).
  local shape
  shape=$(python3 - "$src" "$BUILD_DIR" "$name" <<'PY'
import re, sys, os
src, build, name = sys.argv[1], sys.argv[2], sys.argv[3]
text = open(src).read()
# The example's CMakeLists.txt is the source of truth for the define it is built with (ninja keeps no
# per-target flags file); an unconditional `target_compile_definitions(... TT_METAL_CI_MODE)` = CI shape.
cml = os.path.join(os.path.dirname(src), "CMakeLists.txt")
ci = os.path.exists(cml) and re.search(r"^\s*target_compile_definitions\([^)]*TT_METAL_CI_MODE", open(cml).read(), re.M) is not None
ninja = os.path.join(build, "build.ninja")
if os.path.exists(ninja):
    ci = ci or any("-DTT_METAL_CI_MODE" in l and name in l for l in open(ninja) if l.lstrip().startswith("DEFINES"))
triples, cur = [], {}
for m in re.finditer(r"constexpr\s+uint32_t\s+([MNK])\s*=\s*(\d+)", text):
    if m.group(1) == "M": cur = {}
    cur[m.group(1)] = int(m.group(2))
    if all(k in cur for k in "MNK"):
        triples.append((cur["M"], cur["N"], cur["K"])); cur = {}
if not triples: sys.exit("no shape in " + src)
t = triples[0] if (ci or len(triples) == 1) else triples[-1]
print("%d %d %d" % t)
PY
)
  build_target "metal_example_$name"
  run_sim "example__${name}" "$chip" "$name"
  python3 "$LEDGER_PY" add --ledger "$LEDGER" --stack tt_metal_example --log "$RUN_LOG" --exit-code "$RUN_EXIT" \
    --program "$name" --shape $shape --chip "$(chip_id "$chip")" \
    --reproduce "scripts/tt-matmul-measure.sh example --name $name --chip $chip" \
    --input-fill "the example's own create_random_vector_of_bfloat16 fill, golden = its host matmul"
}

cmd_ttnn() {
  local shape=() chip=wormhole config=default fidelity=HiFi2 fp32=0 l1acc=1 grid="" per_core="" ibw="" subblock="" scaffold=0
  while [ $# -gt 0 ]; do
    case "$1" in
      --shape) shape=("$2" "$3" "$4"); shift 4 ;;
      --chip) chip="$2"; shift 2 ;;
      --config) config="$2"; shift 2 ;;
      --fidelity) fidelity="$2"; shift 2 ;;
      --fp32) fp32="$2"; shift 2 ;;
      --l1acc) l1acc="$2"; shift 2 ;;
      --grid) grid="$2"; shift 2 ;;
      --per-core) per_core="$2"; shift 2 ;;
      --in0-block-w) ibw="$2"; shift 2 ;;
      --subblock) subblock="$2"; shift 2 ;;
      --scaffold) scaffold=1; shift ;;
      *) die "ttnn: unknown arg $1" ;;
    esac
  done
  [ ${#shape[@]} -eq 3 ] || die "ttnn: --shape M N K required"
  # "Our config" means THE CONFIG OUR COMPILER DISPATCHES WITH, at this shape — derived from the autotuner,
  # never a default carried over from another shape. (A hardcoded 5x5/4x4 default silently produced an
  # illegal config at 2048x1024x512: ttnn refuses `num_blocks_x <= grid.x`.)
  if [ "$config" = "ours" ] && [ -z "$grid" ] && [ -z "$per_core" ]; then
    local tiling
    tiling=$(cd "$ROOT" && cargo run -q -p logicaffeine-compile --features codegen --example emit_tt_tiling -- \
             "${shape[@]}" "$chip" 2>/dev/null | grep '^LOGOS_TILING' | head -1)
    [ -n "$tiling" ] || die "ttnn: could not derive the compiler's tiling for ${shape[*]} on $chip"
    grid=$(echo "$tiling" | sed -n 's/.*grid=\([0-9x]*\).*/\1/p')
    per_core=$(echo "$tiling" | sed -n 's/.*per_core=\([0-9x]*\).*/\1/p')
    [ -n "$ibw" ] || ibw=$(echo "$tiling" | sed -n 's/.*in0_block_w=\([0-9]*\).*/\1/p')
    [ -n "$subblock" ] || subblock=$(echo "$tiling" | sed -n 's/.*subblock=\([0-9x]*\).*/\1/p')
    echo "tt-matmul-measure: the compiler tiling for ${shape[*]} on $chip is grid=$grid per_core=$per_core in0_block_w=$ibw subblock=$subblock" >&2
  fi
  local args=(--shape "${shape[@]}" --config "$config" --fidelity "$fidelity" --fp32 "$fp32" --l1acc "$l1acc")
  [ -n "$grid" ] && args+=(--grid "$grid")
  [ -n "$per_core" ] && args+=(--per-core "$per_core")
  [ -n "$ibw" ] && args+=(--in0-block-w "$ibw")
  [ -n "$subblock" ] && args+=(--subblock "$subblock")
  local repro="scripts/tt-matmul-measure.sh ttnn --shape ${shape[*]} --chip $chip --config $config --fidelity $fidelity --fp32 $fp32 --l1acc $l1acc${grid:+ --grid $grid}${per_core:+ --per-core $per_core}${ibw:+ --in0-block-w $ibw}${subblock:+ --subblock $subblock}"
  local scaffold_args=()
  if [ "$scaffold" = 1 ]; then
    run_sim "ttnn__${shape[0]}x${shape[1]}x${shape[2]}__${config}__scaffold" "$chip" --python "$TTNN_PY" "${args[@]}" --scaffold-only
    [ "$RUN_EXIT" = 0 ] || die "scaffold run failed (see $RUN_LOG)"
    scaffold_args=(--scaffold-log "$RUN_LOG")
  fi
  run_sim "ttnn__${shape[0]}x${shape[1]}x${shape[2]}__${config}__${fidelity}-fp32_${fp32}-l1acc_${l1acc}" "$chip" --python "$TTNN_PY" "${args[@]}"
  python3 "$LEDGER_PY" add --ledger "$LEDGER" --stack ttnn_production --log "$RUN_LOG" --exit-code "$RUN_EXIT" \
    --program ttnn_matmul --shape "${shape[@]}" --chip "$(chip_id "$chip")" --reproduce "$repro" \
    --requested-knobs "$fidelity,$fp32,$l1acc" --program-src "$TTNN_PY" \
    --input-fill "numpy LCG seed 0x4c4f474f, uniform bf16 in [-1,1), golden = fp32 host matmul" "${scaffold_args[@]}"
}

chip_id() { case "$1" in wormhole) echo wormhole_b0 ;; blackhole) echo blackhole ;; *) die "chip $1" ;; esac; }
# The compute-with-storage grid each simulated chip exposes under slow dispatch (what the vendor examples ran on).
sim_grid() { case "$1" in wormhole) echo 8x9 ;; blackhole) echo 13x10 ;; *) die "chip $1" ;; esac; }

cmd_all() {
  local shape=(640 640 640) chip=wormhole
  while [ $# -gt 0 ]; do
    case "$1" in
      --shape) shape=("$2" "$3" "$4"); shift 4 ;;
      --chip) chip="$2"; shift 2 ;;
      *) die "all: unknown arg $1" ;;
    esac
  done
  check_env "$chip"
  # 1. the tt-metal examples that run THIS shape, unmodified (all HiFi4 by their own source). The mcast example
  #    only runs 2048x1024x512 (CI build) / 3584x3072x768 (tutorial build): record it separately with
  #    `example --name matmul_multicore_reuse_mcast` for the 2048-class comparison.
  for ex in matmul_single_core matmul_multi_core matmul_multicore_reuse; do
    cmd_example --name "$ex" --chip "$chip"
  done
  # 2. ttnn's production op: its default heuristic (HiFi2 + L1-acc), then OUR config at LoFi / HiFi2 / HiFi4
  cmd_ttnn --shape "${shape[@]}" --chip "$chip" --config default --fidelity HiFi2 --fp32 0 --l1acc 1
  cmd_ttnn --shape "${shape[@]}" --chip "$chip" --config ours --fidelity LoFi  --fp32 0 --l1acc 1
  cmd_ttnn --shape "${shape[@]}" --chip "$chip" --config ours --fidelity HiFi2 --fp32 0 --l1acc 1
  cmd_ttnn --shape "${shape[@]}" --chip "$chip" --config ours --fidelity HiFi4 --fp32 0 --l1acc 0
  # 3. our emitted programs at the same shape, through the spatial-IR lowering — the only lane that
  #    produces a LOGOS row. Every `logos_emitted` row in the ledger is program `logos_spatial`; the
  #    retired `logos` mode drove the hand-written template bundle and never produced a surviving row.
  cmd_spatial --shape "${shape[@]}" --chip "$chip" --candidate best --fidelity HiFi4 --fp32 0 --l1acc 0 --headline
  cmd_spatial --shape "${shape[@]}" --chip "$chip" --candidate streamed --fidelity HiFi4 --fp32 1 --l1acc 0
  python3 "$LEDGER_PY" verify --ledger "$LEDGER"
  python3 "$LEDGER_PY" show --ledger "$LEDGER"
}

# Install an ALREADY-PRINTED bundle into the box's single build tree, replacing whatever was there.
#
# ★ THE EMISSION MOVED OUT; THE LOCK DID NOT. `spatial_tensix_p4_ttsim_lock` prints its bundle
# IN-PROCESS, because a nested `cargo run --example emit_spatial_tensix` launched from inside a cargo
# test run blocks on cargo's global target-directory lock — held by the very test binary that spawned
# it — and dies at exit 2 having never reached the simulator. The copy still happens HERE, inside
# logs/tt/.measure.lock, so a peer session's GEMM emission can never land between it and the build.
install_bundle() {
  local from="$1"
  [ -d "$from" ] || die "p4: --bundle $from is not a directory"
  [ -f "$from/host.cpp" ] && [ -d "$from/kernels" ] || die "p4: --bundle $from is not a bundle (host.cpp + kernels/)"
  mkdir -p "$SPATIAL_DIR/kernels"
  # Leave no stale kernel behind: the bundle IS the artifact, so the directory is replaced, never merged.
  rm -f "$SPATIAL_DIR"/kernels/* "$SPATIAL_DIR"/host.cpp "$SPATIAL_DIR"/logos_oracle.c "$SPATIAL_DIR"/spatial.json
  find "$from" -maxdepth 1 -type f -exec cp {} "$SPATIAL_DIR/" \;
  find "$from/kernels" -maxdepth 1 -type f -exec cp {} "$SPATIAL_DIR/kernels/" \;
  echo "installed bundle $from -> $SPATIAL_DIR ($(find "$SPATIAL_DIR/kernels" -maxdepth 1 -type f | wc -l) kernels)"
}

# The P4 workloads — a normalization, a gated activation, an elementwise map — emitted from the SAME
# spatial IR as the GEMM and run under the SAME simulator. They are CORRECTNESS runs: each prints its
# PCC against the host golden, and nothing is written to the GEMM ledger, whose rows are keyed on a
# (m, n, k) that a row op does not have. The cycle counter is deliberately not reported here — see
# `tt_ledger::assert_cycles_are_not_quoted_as_throughput` for why it is not a measure of work.
cmd_p4() {
  local workload="" rows=32 cols=64 chip=wormhole bundle=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --workload) workload="$2"; shift 2 ;;
      --rows) rows="$2"; shift 2 ;;
      --cols) cols="$2"; shift 2 ;;
      --chip) chip="$2"; shift 2 ;;
      --bundle) bundle="$2"; shift 2 ;;
      *) die "p4: unknown arg $1" ;;
    esac
  done
  case "$workload" in rmsnorm|layernorm|swiglu|softmax|elementwise) ;; *) die "p4: --workload rmsnorm|layernorm|swiglu|softmax|elementwise" ;; esac
  if [ -n "$bundle" ]; then
    install_bundle "$bundle"
  else
    cargo run -q -p logicaffeine-compile --features codegen --example emit_spatial_tensix -- \
      "$SPATIAL_DIR" "$rows" "$cols" 1 "$chip" --workload "$workload" --grid "$(sim_grid "$chip")" || die "emit failed"
  fi
  register_spatial_example
  build_target metal_example_logos_spatial
  run_sim "p4__${workload}__${rows}x${cols}" "$chip" logos_spatial
  echo "LOGOS_P4_LOG $RUN_LOG"
  # The host names the program it is about to run before it runs it; echoing it here is what lets a HAND run
  # see which bundle the shared build tree actually held. (`spatial_tensix_p4_ttsim_lock` reads the log itself
  # and compares this line to the bundle it printed, which is what makes a stale tree impossible to mistake
  # for a measurement.) Informational only — the refusal below is still the verdict grep.
  grep -E "LOGOS_MM_CFG" "$RUN_LOG" || true
  grep -E "LOGOS_CHECKSUM|LOGOS_MM_PASS|LOGOS_MM_FAIL|PCC = " "$RUN_LOG" || die "p4: the run printed no verdict (see $RUN_LOG)"
  # A P4 run that printed LOGOS_MM_FAIL exits 1, and this mode used to swallow it: the script said nothing and
  # returned 0, so a HAND run of a wrong kernel looked like a successful measurement. The exit code is the
  # host's own verdict — carry it.
  [ "$RUN_EXIT" = 0 ] || die "p4: $workload ${rows}x${cols} did NOT pass its own comparison (exit $RUN_EXIT) — see $RUN_LOG"
}

# The box is single-tenant for ttsim + build_Release: one measurement at a time across ALL sessions. Every
# invocation holds logs/tt/.measure.lock for its whole build/run/record so two sessions can never interleave a
# build, a run, or a ledger write (peers running ttsim directly take the same flock).
mkdir -p "$LOG_DIR"
exec 9>"$LOG_DIR/.measure.lock"
if ! flock -n 9; then
  echo "tt-matmul-measure: another measurement holds $LOG_DIR/.measure.lock — waiting for it to finish" >&2
  flock 9
fi

MODE="${1:-}"; shift || true
case "$MODE" in
  check-env) check_env "${1:-wormhole}" ;;
  scan-examples) scan_examples ;;
  spatial) cmd_spatial "$@" ;;
  p4) cmd_p4 "$@" ;;
  example) cmd_example "$@" ;;
  ttnn) cmd_ttnn "$@" ;;
  all) cmd_all "$@" ;;
  *) sed -n '2,20p' "$0"; exit 2 ;;
esac
