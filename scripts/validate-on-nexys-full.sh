#!/usr/bin/env bash
# validate-on-nexys-full — TDD validation of the full-core silicon: for each `.lg`, the on-chip Show
# output MUST equal the VM golden reference (`lgvm`, the same vm_outcome the whole suite locks against).
# Reprograms the board before each program (the loader latches at L_DONE — one program per bitstream load).
#
#   scripts/validate-on-nexys-full.sh <prog1.lg> [prog2.lg ...]
set -uo pipefail
cd "$(dirname "$0")/.."
ROOT_DIR="$PWD"
PORT=$(python3 scripts/lib/ftdi_ports.py port B) || exit 1; BIT="${BIT:-benchmarks/silicon/full_core.bit}"
[ -s "$BIT" ] || { echo "no $BIT — build it first (run-on-nexys-full.sh --build)"; exit 1; }
pass=0; fail=0
# Build the compiler examples ONCE as the invoking user (a login shell so tristen's ~/.cargo is on PATH —
# this script runs under sudo for the USB board), then call the BINARIES directly. That sidesteps both the
# root-owned-target and the sudo-PATH problems that break a bare `cargo run` under sudo.
LGVM="$ROOT_DIR/target/debug/examples/lgvm"; LG2HEX="$ROOT_DIR/target/debug/examples/lg2hex"
if [ ! -x "$LGVM" ] || [ ! -x "$LG2HEX" ]; then
  echo "== building lgvm + lg2hex examples =="
  ${SUDO_USER:+sudo -u "$SUDO_USER"} bash -lc "cd '$ROOT_DIR' && cargo build -q -p logicaffeine-compile --example lgvm --example lg2hex"
fi
printf "%-24s %-28s %-28s %8s  %s\n" PROGRAM "VM GOLDEN" "SILICON" CYCLES RESULT
for lg in "$@"; do
  name=$(basename "$lg" .lg)
  vm=$("$LGVM" "$lg" 2>/dev/null | tr '\n' ',' | sed 's/,$//')
  "$LG2HEX" --full "$lg" > "/tmp/val_$name.hex" 2>/dev/null \
    || { printf "%-24s %-28s %-28s %8s  ENCODE-ERR\n" "$name" "$vm" "-" "-"; fail=$((fail+1)); continue; }
  python3 scripts/lib/ftdi_ports.py flash B "$BIT" >/dev/null 2>&1
  out=$(timeout 120 python3 scripts/nexys-host.py "/tmp/val_$name.hex" --port $PORT --clock-mhz 6.25 2>&1)
  sil=$(echo "$out" | grep -oE 'Show values: \[[^]]*\]' | sed "s/Show values: \[//;s/\]//;s/, /,/g;s/'//g")
  cyc=$(echo "$out" | grep -oE 'on-chip: [0-9]+' | grep -oE '[0-9]+'); cyc=${cyc:-0}
  if [ "$sil" = "$vm" ]; then r="PASS"; pass=$((pass+1)); else r="*** FAIL"; fail=$((fail+1)); fi
  [ -z "$sil" ] && sil="(no reply)"
  printf "%-24s %-28s %-28s %8s  %s\n" "$name" "$vm" "$sil" "$cyc" "$r"
done
echo "──────────────────────────────────────────────────────────────────────────────"
echo "SILICON ≡ VM VALIDATION: $pass passed, $fail failed of $((pass+fail))"
[ "$fail" -eq 0 ] && echo "ALL GREEN — the full LOGOS CPU on the Artix-7 matches the VM exactly." || exit 1
