#!/usr/bin/env bash
# demo-hft — the narrated LOGOS tick-to-trade determinism demo (the one you run live for an LOI).
#
#   scripts/demo-hft.sh
#
# 1. shows the market-making strategy in plain English (the .lg the silicon runs),
# 2. proves it on the LOGOS full core: order stream bit-exact to the reference (VM ≡ silicon), with the
#    per-message on-chip cycle latency (deterministic, zero jitter),
# 3. times the identical pipeline on this x86 to expose its jitter tail — quiet and under load,
# 4. reports absolute latency across the clock tiers (FPGA measured -> Sky130 -> ASIC), honestly tagged.
#
# The determinism proof runs in cycle-accurate RTL simulation (no board needed, reproducible). A live
# run on the real Nexys A7 is the separate scripts/run-on-nexys-*.sh path; this wrapper flags whether a
# board is free (it never fights the co-agent's routes).
set -uo pipefail
cd "$(dirname "$0")/.."
bar() { printf '%.0s─' $(seq 1 74); echo; }

bar; echo "  LOGOS — deterministic, English-verified tick-to-trade on silicon"; bar
cat <<'EOF'

  You write the strategy in English. It compiles through certified tiers to our
  soft-core, and we PROVE the silicon does exactly what the English says. It runs
  with zero run-to-run jitter — the determinism HFT pays for, without the RTL.

  The strategy (benchmarks/silicon/programs/hft_pipeline.lg, market-making core):
EOF
# show the human-meaningful core of the strategy (skip the book-init plumbing)
sed -n '/Let micro100/,/Show order_size/p' benchmarks/silicon/programs/hft_pipeline.lg | sed 's/^/    /'
echo
echo "  ITCH-lite market data -> price-ladder order book -> microprice market-making"
echo "  -> pre-trade risk (fat-finger + position limit) -> order.  Proving it now:"
echo

python3 scripts/demo-hft.py "$@"

PAGE="benchmarks/silicon/hft_demo.html"
echo
echo "  ┌────────────────────────────────────────────────────────────────────────┐"
echo "  │  PROJECT THIS:   $PAGE"
echo "  └────────────────────────────────────────────────────────────────────────┘"
# open it in a browser if this box has one (harmless if not)
for o in xdg-open open; do command -v "$o" >/dev/null && { "$o" "$PAGE" >/dev/null 2>&1 & break; }; done

echo
if pgrep -x nextpnr >/dev/null || pgrep -x openFPGALoader >/dev/null; then
  echo "  [board] FPGA toolchain busy (co-agent routing) — skipping the live board run."
  echo "          The proof above is cycle-accurate RTL of the same core the bitstream ships."
else
  echo "  [board] A Nexys A7 is free — a live on-silicon run is available via:"
  echo "          sudo scripts/run-on-nexys-full.sh <bounded-feed>.lg   (see benchmarks/silicon/HFT_DEMO_README.md)"
fi
echo
bar
echo "  The wedge: determinism you can PROVE + a strategy your quants can READ."
echo "  Incumbents (Exegy/NovaSparks/Enyx) give you the determinism after months of"
echo "  RTL + verification. We give you both from English, on a path to ASIC latency."
bar
