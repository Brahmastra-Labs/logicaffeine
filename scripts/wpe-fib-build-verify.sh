#!/usr/bin/env bash
# wpe-fib-build-verify — rebuild the over-the-wire whole-program engine bitstream WITH the fib_iterative
# write-ordering fix (file0 block-entry snapshot) and verify it on real Nexys A7 silicon. The open flow does NO
# hold-time fixing, so a proven-correct netlist can glitch on a hold-marginal FF->FF path; a different placement
# SEED clears it (the "hold lottery"). This builds per seed, flashes Board B, streams bench_fib_iterative.wire,
# and stops at the FIRST seed that returns fib(20)=6765 on hardware (the pre-fix engine returned 524288 = 2^19).
# Then it runs the integer cross-checks (the deployed engine is WITH_FPU=0 — no float lane, so pi_leibniz is
# excluded). No PLACERELAX (it bloated the netlist to 92 RAMB / congested); default router.
#
#   scripts/wpe-fib-build-verify.sh [seed1 seed2 ...]     (default: 5 7 3 9 1)
set -uo pipefail
cd "$(dirname "$0")/.."
SIL=benchmarks/silicon
PORT=$(python3 scripts/lib/ftdi_ports.py port B) || exit 1
HOST=(--port "$PORT" --baud 9600 --clock-mhz 0.78125)
SEEDS=("$@"); [ ${#SEEDS[@]} -eq 0 ] && SEEDS=(5 7 3 9 1)

# name:wire:expected — integer-only (no FPU on the deployed WITH_FPU=0 engine).
CROSS=(
  "loop_sum:bench_loop_sum.wire:5050"
  "fib_recursive:bench_fib.wire:21"
  "array_fill:bench_array_fill.wire:220"
  "sumdown_f5:wpe_loop.wire:15"
)

stream() { # wire expected  ->  echoes host tail, returns 0 iff "OK: matches"
  local wire="$1" exp="$2" out
  out=$(timeout 40 python3 scripts/nexys-wpe-host.py "$SIL/programs/$wire" "${HOST[@]}" --expect "$exp" 2>&1)
  echo "$out" | tail -3
  echo "$out" | grep -q "OK: matches"
}

for s in "${SEEDS[@]}"; do
  echo "############## SEED=$s ##############"
  echo "== [build] nextpnr-xilinx seed=$s (no PLACERELAX, default router) — this takes ~2h, DO NOT KILL =="
  if ! SEED="$s" TOP=nexys_wpe_soc SRC="nexys_wpe_soc.v wpe_soc_fpga.v" BIT="wpe_soc_fix_s$s.bit" \
       bash scripts/openxc7-build.sh; then
    echo "  seed $s: BUILD FAILED"; continue
  fi
  [ -s "$SIL/wpe_soc_fix_s$s.bit" ] || { echo "  seed $s: no bitstream produced"; continue; }
  echo "== [flash] Board B =="
  python3 scripts/lib/ftdi_ports.py flash B "$SIL/wpe_soc_fix_s$s.bit" 2>&1 | tail -1
  sleep 4
  echo "== [verify] PRIMARY: bench_fib_iterative → 6765 (buggy engine = 524288) =="
  if ! stream bench_fib_iterative.wire 6765; then
    echo "  seed $s: fib MISMATCH on hardware (hold-marginal placement) — next seed"
    continue
  fi
  echo "  seed $s: fib(20)=6765 ON SILICON — the write-ordering fix is live. Running cross-checks:"
  allok=1
  for c in "${CROSS[@]}"; do
    IFS=: read -r nm wire exp <<<"$c"
    echo "== [cross] $nm ($wire) → $exp =="
    stream "$wire" "$exp" || { echo "  cross-check $nm FAILED"; allok=0; }
  done
  if [ "$allok" = 1 ]; then
    cp "$SIL/wpe_soc_fix_s$s.bit" "$SIL/wpe_soc.bit"
    echo "############## SUCCESS: seed $s — fib=6765 + all integer cross-checks pass on silicon =="
    echo "############## deployed -> $SIL/wpe_soc.bit =="
    exit 0
  fi
  echo "  seed $s: fib passed but a cross-check failed — keeping bitstream, trying next seed for a clean sweep"
done
echo "no seed in {${SEEDS[*]}} produced a fully-passing hold-safe bitstream — try more seeds"
exit 1
