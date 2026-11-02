#!/usr/bin/env bash
# What is running, and what is it costing? Run this FIRST every session and LAST before you walk away.
#
#   scripts/accel/runpod-status.sh            # pods, burn rate, balance, volume
#   scripts/accel/runpod-status.sh --stock    # + which GPUs are actually available in our datacenter

source "$(dirname "$0")/runpod-lib.sh"

echo "=== pods ==="
tsv="$(rp_pods_tsv)"
if [ -z "$tsv" ]; then
  echo "  NONE — nothing exists, nothing bills."
else
  printf '  %-16s %-24s %-10s %s\n' ID NAME STATUS '$/hr'
  echo "$tsv" | while IFS=$'\t' read -r id name st cost; do
    printf '  %-16s %-24s %-10s %s\n' "$id" "$name" "$st" "$cost"
  done
  echo
  echo "  ⚠ a pod that exists can bill even when EXITED (container disk). Terminate with runpod-down.sh."
fi

echo "=== account ==="
IFS=$'\t' read -r bal burn < <(rp_balance)
printf '  balance   $%s\n  burn      $%s/hr\n' "$bal" "$burn"

echo "=== network volume (the weights live here; pods are disposable) ==="
rp_rest GET /networkvolumes | python3 -c '
import sys, json
for v in json.load(sys.stdin):
    print("  %s  %-16s %4d GB  %s  (~$%.2f/mo)" % (v["id"], v["name"], v["size"], v["dataCenterId"], v["size"] * 0.07))'

if [ "${1:-}" = "--stock" ]; then
  echo "=== stock in $RP_DATACENTER ==="
  for role in dev a100 h100; do
    IFS='|' read -ra ids <<< "$(rp_ladder "$role")"
    for id in "${ids[@]}"; do
      IFS=$'\t' read -r st price < <(rp_stock "$id")
      [ "$st" = "none" ] && continue
      printf '  %-5s %-32s stock=%-9s $%s/hr\n' "$role" "$id" "$st" "$price"
    done
  done
fi
