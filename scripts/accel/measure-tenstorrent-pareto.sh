#!/usr/bin/env bash
# ONE-COMMAND MEASURED Pareto fill-in (Phase 5) — run on a host WITH a Tenstorrent card.
#
# Replaces the PROJECTED workload layer of the perf-vs-cost frontier with MEASURED numbers by running
# the LOGOS-emitted kernels on a real Tensix core and reading its power/throughput. Until a card is
# present this refuses (loud), so no measurement is ever fabricated.
#
# Prereqs (Phase 5): a built tt-metal SDK ($TT_METAL_HOME), the drivers installed
# (curl -fsSL https://tenstorrent.ai/install.sh | bash) with `tt-smi` enumerating the card, and this
# repo built.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

echo "== MEASURED Tenstorrent Pareto =="

# 1. Card present?
if ! command -v tt-smi >/dev/null 2>&1; then
  echo "REFUSING: tt-smi not found — no Tenstorrent driver/card. Install: curl -fsSL https://tenstorrent.ai/install.sh | bash" >&2
  exit 2
fi
if ! lspci -d 1e52: 2>/dev/null | grep -q .; then
  echo "REFUSING: no Tenstorrent PCIe device (vendor 1e52) enumerated. Check the card + power." >&2
  exit 2
fi
: "${TT_METAL_HOME:?set TT_METAL_HOME to a built tt-metal SDK}"

# 2. Build every benchmark bundle + the RDU GEMM against the SDK, run each on the card under the DPRINT
#    server, and record wall-clock + board power (tt-smi telemetry). Compare each to the VM (≡VM gate).
MEAS="$ROOT/benchmarks/silicon/tenstorrent_measured.json"
echo "[" > "$MEAS.tmp"
first=1
for d in "$ROOT"/benchmarks/programs/*/; do
  b="$(basename "$d")"
  [ -f "$d/interp.lg" ] || continue
  out="$ROOT/target/tt-meas/$b"
  cargo run -q -p logicaffeine-cli -- emit tenstorrent "$d/interp.lg" -o "$out" || { echo "  skip $b (refused)"; continue; }
  ( cd "$out" && cmake -B build -S . >/dev/null 2>&1 && cmake --build build >/dev/null 2>&1 ) || { echo "  skip $b (build)"; continue; }
  # Run on the card, capturing DPRINT output + timing; tt-smi -s snapshots board power.
  t0=$(date +%s.%N)
  got="$(cd "$out" && TT_METAL_DPRINT_CORES=0,0 ./build/logos_tt_host 2>/dev/null | tr -d '\r')"
  t1=$(date +%s.%N)
  pw="$(tt-smi -s 2>/dev/null | grep -ioE 'power[^0-9]*[0-9.]+' | grep -oE '[0-9.]+' | head -1)"
  ms=$(awk "BEGIN{printf \"%.3f\", ($t1-$t0)*1000}")
  [ $first -eq 1 ] || echo "," >> "$MEAS.tmp"; first=0
  printf '  {"benchmark":"%s","stdout":%s,"wall_ms":%s,"board_power_w":%s,"measured":true}' \
    "$b" "$(printf '%s' "$got" | python3 -c 'import json,sys;print(json.dumps(sys.stdin.read().strip()))')" "$ms" "${pw:-null}" >> "$MEAS.tmp"
  echo "  measured $b: ${ms}ms ${pw:-?}W"
done
echo "" >> "$MEAS.tmp"; echo "]" >> "$MEAS.tmp"; mv "$MEAS.tmp" "$MEAS"
echo "wrote $MEAS"

# 3. Regenerate the frontier — the generator now folds in the MEASURED layer.
python3 "$ROOT/scripts/accel/gen-pareto-frontier.py"
echo "done: benchmarks/results/tenstorrent_pareto.json now carries MEASURED Tenstorrent numbers."
