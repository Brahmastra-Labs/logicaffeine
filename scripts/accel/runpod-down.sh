#!/usr/bin/env bash
# Terminate every pod on the account and prove nothing is left billing.
#
#   scripts/accel/runpod-down.sh            # terminate all
#   scripts/accel/runpod-down.sh <podId>    # terminate one
#
# TERMINATE, never "stop": a stopped pod keeps its container disk and keeps charging for it. The weights
# live on the network volume, which is untouched here, so re-provisioning later costs minutes not hours.

source "$(dirname "$0")/runpod-lib.sh"

tsv="$(rp_pods_tsv)"
if [ -z "$tsv" ]; then echo "nothing to terminate — no pods exist."; exit 0; fi

if [ $# -gt 0 ]; then
  tsv="$(printf '%s\n' "$tsv" | awk -F'\t' -v id="$1" '$1==id')"
  [ -n "$tsv" ] || { echo "no such pod: $1" >&2; exit 1; }
fi

printf '%s\n' "$tsv" | while IFS=$'\t' read -r id name st cost; do
  printf 'terminating %s (%s, %s, $%s/hr) ... ' "$id" "$name" "$st" "$cost"
  code="$(rp_rest DELETE "/pods/$id" >/dev/null 2>&1; echo $?)"
  [ "$code" = "0" ] && echo "ok" || echo "FAILED (exit $code)"
done

sleep 5
echo "=== after ==="
left="$(rp_pods_tsv)"
if [ -z "$left" ]; then
  echo "  clean — no pods exist."
else
  printf '%s\n' "$left" | while IFS=$'\t' read -r id name st cost; do
    printf '  STILL PRESENT: %s %s %s $%s/hr\n' "$id" "$name" "$st" "$cost"
  done
  exit 1
fi
IFS=$'\t' read -r bal burn < <(rp_balance)
printf '  balance $%s   burn $%s/hr (volume only)\n' "$bal" "$burn"
