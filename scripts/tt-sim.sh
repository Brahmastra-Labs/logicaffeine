#!/usr/bin/env bash
# Run a tt-metal program (or a ttnn Python script) under ttsim (Tenstorrent's full-system simulator) — NO card.
#
# ttsim is cycle-accurate + bit-exact; it plugs into tt-metal via TT_METAL_SIMULATOR, so a kernel runs on a
# virtual Wormhole/Blackhole with no silicon. This is the perf-validation path (it reports CYCLES, not power).
#
#   Setup (once): a built tt-metal (TT_METAL_HOME) + the prebuilt sim .so per chip, each in ITS OWN directory
#   (ttsim reads `soc_descriptor.yaml` from the directory that holds the .so, so one directory = one chip):
#     wget -P ~/sim    https://github.com/tenstorrent/ttsim/releases/latest/download/libttsim_wh.so
#     wget -P ~/sim_bh https://github.com/tenstorrent/ttsim/releases/latest/download/libttsim_bh.so
#
#   Usage: scripts/tt-sim.sh [--chip wormhole|blackhole] [--python] <example-or-binary-or-script> [args...]
#     e.g. scripts/tt-sim.sh matmul_multi_core
#          scripts/tt-sim.sh --chip blackhole matmul_single_core
#          scripts/tt-sim.sh --python scripts/accel/tt-ttnn-matmul.py --shape 640 640 640 --config default
#
#   The first line printed is the LOGOS_TT_ENV banner (chip, simulator .so + sha256, ttsim version, tt-metal
#   sha) — `scripts/tt-gemm-ledger.py` refuses to record a run that lacks it.
set -euo pipefail

CHIP="wormhole"
PYTHON=0
while [ $# -gt 0 ]; do
  case "$1" in
    --chip) CHIP="${2:?--chip needs wormhole|blackhole}"; shift 2 ;;
    --python) PYTHON=1; shift ;;
    --) shift; break ;;
    *) break ;;
  esac
done

TT_METAL_HOME="${TT_METAL_HOME:-$HOME/tt-metal}"
# The INSTRUMENTED simulator, when it has been built. ttsim is Apache-2.0; the patch adds counters only
# (a work meter, an opcode histogram, per-core PCs, operand-stall attribution) and is verified
# timing-identical to the stock release by `tt_work_meter_lock::the_instrument_does_not_move_the_clock`:
# matmul_single_core reports 330268 cycles under both. Set TTSIM_STOCK=1 to force the release .so.
instrumented() {
  local d="${TTSIM_INSTRUMENTED_DIR:-$HOME/ttsim/src/_out/release_$1}"
  [ "${TTSIM_STOCK:-0}" = 1 ] && return 1
  [ -f "$d/libttsim.so" ] || return 1
  echo "$d/libttsim.so"
}

case "$CHIP" in
  wormhole)
    SIM_DIR="${SIM_DIR_WH:-${SIM_DIR:-$HOME/sim}}"
    SIM_SO="${TT_METAL_SIMULATOR:-$(instrumented wh || echo "$SIM_DIR/libttsim_wh.so")}"
    SOC="$TT_METAL_HOME/tt_metal/soc_descriptors/wormhole_b0_80_arch.yaml"
    CHIP_ID="wormhole_b0" ;;
  blackhole)
    SIM_DIR="${SIM_DIR_BH:-$HOME/sim_bh}"
    SIM_SO="${TT_METAL_SIMULATOR:-$(instrumented bh || echo "$SIM_DIR/libttsim_bh.so")}"
    SOC="$TT_METAL_HOME/tt_metal/soc_descriptors/blackhole_140_arch.yaml"
    CHIP_ID="blackhole" ;;
  *) echo "unknown --chip $CHIP (wormhole|blackhole)" >&2; exit 2 ;;
esac
SIM_DIR="$(dirname "$SIM_SO")"

[ -f "$SIM_SO" ] || { echo "missing sim: $SIM_SO — download libttsim_{wh,bh}.so from github.com/tenstorrent/ttsim/releases" >&2; exit 2; }
BUILD_DIR="$TT_METAL_HOME/build"
[ -d "$BUILD_DIR" ] || BUILD_DIR="$TT_METAL_HOME/build_Release"
[ -d "$BUILD_DIR" ] || { echo "tt-metal not built at $TT_METAL_HOME/build{,_Release} — run: ./build_metal.sh --build-programming-examples" >&2; exit 2; }
[ -f "$SOC" ] || { echo "missing SOC descriptor: $SOC" >&2; exit 2; }

# ttsim reads soc_descriptor.yaml from the directory that holds the .so, so one directory serves ONE chip.
# If a second simulator .so shares this directory AND the descriptor here belongs to the other chip, running
# would stomp it (the Blackhole-int8 re-check once silently ran on a Wormhole descriptor) — refuse instead.
OTHER_SO=$(ls "$SIM_DIR"/libttsim_*.so 2>/dev/null | grep -v -F "$(basename "$SIM_SO")" || true)
if ! cmp -s "$SOC" "$SIM_DIR/soc_descriptor.yaml"; then
  if [ -n "$OTHER_SO" ] && [ -f "$SIM_DIR/soc_descriptor.yaml" ]; then
    echo "REFUSING: $SIM_DIR holds $(basename "$SIM_SO") AND $(basename "$OTHER_SO"), and its soc_descriptor.yaml is not $CHIP_ID's — running would stomp the other chip's descriptor. Keep one simulator .so per directory (e.g. ~/sim for Wormhole, ~/sim_bh for Blackhole)." >&2
    exit 2
  fi
  cp -f "$SOC" "$SIM_DIR/soc_descriptor.yaml"
fi

# Identities for the ledger: the simulator binary, its release tag, and the tt-metal commit.
SO_SHA=$(sha256sum "$SIM_SO" | cut -d' ' -f1)
TTSIM_SRC="${TTSIM_SRC:-$HOME/ttsim}"
TTSIM_VERSION="unknown"
if [ -f "$SIM_DIR/VERSION" ]; then
  TTSIM_VERSION=$(tr -d '[:space:]' < "$SIM_DIR/VERSION")
elif [ -d "$TTSIM_SRC/.git" ]; then
  # The release tag: loose ref files first, then packed-refs (no git invocation).
  TTSIM_VERSION=$( { ls "$TTSIM_SRC/.git/refs/tags" 2>/dev/null || true; { grep -oE 'refs/tags/v[0-9]+\.[0-9]+\.[0-9]+' "$TTSIM_SRC/.git/packed-refs" 2>/dev/null || true; } | sed 's#refs/tags/##'; } | { grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' || true; } | sort -V | tail -1)
  [ -n "$TTSIM_VERSION" ] || TTSIM_VERSION="unknown"
fi
TT_METAL_SHA="unknown"
if [ -f "$TT_METAL_HOME/.git/HEAD" ]; then
  REF=$(cat "$TT_METAL_HOME/.git/HEAD")
  case "$REF" in
    ref:*) R="${REF#ref: }"
           if [ -f "$TT_METAL_HOME/.git/$R" ]; then TT_METAL_SHA=$(cat "$TT_METAL_HOME/.git/$R");
           elif [ -f "$TT_METAL_HOME/.git/packed-refs" ]; then TT_METAL_SHA=$(grep " $R\$" "$TT_METAL_HOME/.git/packed-refs" | cut -d' ' -f1); fi ;;
    *) TT_METAL_SHA="$REF" ;;
  esac
fi

export TT_METAL_HOME
export TT_METAL_SIMULATOR="$SIM_SO"
export TT_METAL_SLOW_DISPATCH_MODE=1      # fast dispatch is not sufficiently tested under sim
export TT_METAL_DISABLE_SFPLOADMACRO=1    # SFPLOADMACRO is unsupported in ttsim's SFPU
# The work meter. The stock release ignores it; the instrumented build prints TTSIM_OP_STATS / TTSIM_DONE /
# TTSIM_STALLS at exit, which is how a run records WHAT THE MACHINE DID and not merely how long a counter
# that does not track work happened to read. Every measurement carries it, so no comparison can be made on
# an instrument only one side was run under.
export TTSIM_OP_STATS=1
export LOGOS_TT_CHIP="$CHIP_ID"

echo "LOGOS_TT_ENV chip=$CHIP_ID so=$(basename "$SIM_SO") so_sha256=$SO_SHA ttsim_version=$TTSIM_VERSION tt_metal_sha=$TT_METAL_SHA"

TARGET="${1:?usage: tt-sim.sh [--chip wormhole|blackhole] [--python] <example-or-binary-or-script> [args...]}"; shift || true

if [ "$PYTHON" = 1 ]; then
  # A ttnn script: the venv interpreter + the in-tree ttnn package (tracy lives under tools/).
  VENV_PY="${TTNN_PYTHON:-$HOME/ttnn-venv/bin/python}"
  [ -x "$VENV_PY" ] || { echo "missing ttnn venv python: $VENV_PY (set TTNN_PYTHON)" >&2; exit 2; }
  SCRIPT="$(cd "$(dirname "$TARGET")" && pwd)/$(basename "$TARGET")"
  [ -f "$SCRIPT" ] || { echo "script '$TARGET' not found" >&2; exit 2; }
  export PYTHONPATH="$TT_METAL_HOME/ttnn:$TT_METAL_HOME/tools${PYTHONPATH:+:$PYTHONPATH}"
  echo "== ttsim ($CHIP_ID) : $VENV_PY $SCRIPT =="
  cd "$TT_METAL_HOME"
  exec "$VENV_PY" "$SCRIPT" "$@"
fi

# Resolve the binary under the build tree (programming examples, tests, or an absolute path).
EXE=""
for p in \
  "$TARGET" \
  "$BUILD_DIR/programming_examples/$TARGET" \
  "$BUILD_DIR/programming_examples/metal_example_$TARGET" \
  "$BUILD_DIR/programming_examples/$TARGET/$TARGET" \
  "$BUILD_DIR/test/tt_metal/$TARGET"; do
  [ -x "$p" ] && { EXE="$p"; break; }
done
if [ -z "$EXE" ]; then
  echo "binary '$TARGET' not found. Available programming examples:" >&2
  ls "$BUILD_DIR/programming_examples/" 2>/dev/null >&2 || true
  exit 2
fi

echo "== ttsim ($CHIP_ID) : $EXE =="
# tt-metal's runtime derives its root dir from CWD/TT_METAL_HOME — run from the repo root.
cd "$TT_METAL_HOME"
exec "$EXE" "$@"
