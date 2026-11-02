#!/usr/bin/env bash
# Shared helpers for the RunPod campaign scripts (runpod-up/down/status).
#
# ONE POD AT A TIME is the invariant these scripts exist to enforce: a forgotten pod bills by the
# second, and the previous Qwen campaign left four running at once. `runpod-up.sh` refuses to deploy
# while any pod exists, and `runpod-down.sh` terminates rather than stops (a stopped pod still bills
# its container disk; the network volume that holds the weights persists on its own).
#
# The API key is read from ~/.runpod/key (mode 600) or $RUNPOD_API_KEY. It is never stored in the repo.

set -euo pipefail

RP_REST="https://rest.runpod.io/v1"
RP_GQL="https://api.runpod.io/graphql"

# The persistent home of the model weights. Pods are disposable; this is not.
RP_VOLUME_ID="${RP_VOLUME_ID:-rvq94a2gft}"
RP_DATACENTER="${RP_DATACENTER:-US-KS-2}"   # the volume's DC — a pod must be here to mount it
RP_MOUNT="/workspace"
RP_IMAGE="${RP_IMAGE:-runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04}"
RP_SSH_KEY="${RP_SSH_KEY:-$HOME/.ssh/id_ed25519_alex_sync}"

rp_key() {
  if [ -n "${RUNPOD_API_KEY:-}" ]; then printf '%s' "$RUNPOD_API_KEY"; return; fi
  if [ -f "$HOME/.runpod/key" ]; then tr -d '\n' < "$HOME/.runpod/key"; return; fi
  echo "runpod: no API key — put it in ~/.runpod/key (chmod 600) or set RUNPOD_API_KEY" >&2
  exit 1
}

# rp_rest <METHOD> <path> [json-body]
rp_rest() {
  local method="$1" path="$2" body="${3:-}" key; key="$(rp_key)"
  if [ -n "$body" ]; then
    curl -sS --max-time 120 -X "$method" -H "Authorization: Bearer $key" \
         -H "Content-Type: application/json" -d "$body" "$RP_REST$path"
  else
    curl -sS --max-time 120 -X "$method" -H "Authorization: Bearer $key" "$RP_REST$path"
  fi
}

# rp_gql <query-string> — GraphQL is only needed for stock/pricing, which REST does not expose.
rp_gql() {
  local key; key="$(rp_key)"
  python3 -c "import json,sys; sys.stdout.write(json.dumps({'query': sys.argv[1]}))" "$1" > /tmp/rp_gql_$$.json
  curl -sS --max-time 60 -X POST "$RP_GQL?api_key=$key" -H "Content-Type: application/json" \
       --data @/tmp/rp_gql_$$.json
  rm -f /tmp/rp_gql_$$.json
}

# Every pod on the account, as TSV: id, name, status, $/hr. Empty output == nothing exists.
rp_pods_tsv() {
  rp_rest GET /pods | python3 -c '
import sys, json
d = json.load(sys.stdin)
pods = d if isinstance(d, list) else d.get("data", [])
for p in pods:
    print("\t".join(str(p.get(k, "")) for k in ("id", "name", "desiredStatus", "costPerHr")))'
}

rp_balance() {
  rp_gql 'query{myself{clientBalance currentSpendPerHr}}' | python3 -c '
import sys, json
m = json.load(sys.stdin)["data"]["myself"]
print("%.2f\t%s" % (m["clientBalance"], m["currentSpendPerHr"]))'
}

# Stock + price for one gpuTypeId in our datacenter. Prints "<stock>\t<price>" or "none\t-".
rp_stock() {
  rp_gql "query{gpuTypes(input:{id:\"$1\"}){lowestPrice(input:{gpuCount:1,dataCenterId:\"$RP_DATACENTER\"}){stockStatus uninterruptablePrice}}}" \
  | python3 -c '
import sys, json
try:
    t = json.load(sys.stdin)["data"]["gpuTypes"]
    lp = (t[0].get("lowestPrice") or {}) if t else {}
    print("%s\t%s" % (lp.get("stockStatus") or "none", lp.get("uninterruptablePrice") or "-"))
except Exception:
    print("none\t-")'
}

# GPU ladders. Only 4 types have stock in US-KS-2, and all are "Low" — hence the fallbacks.
# dev  = big enough to RUN the 29 GB Q8_0 model, cheap enough to develop and download on
# a100 / h100 = the measurement chips; we rent these ONLY to take numbers
rp_ladder() {
  case "$1" in
    dev)  echo "NVIDIA L40|NVIDIA RTX A6000|NVIDIA A40|NVIDIA L40S" ;;
    a100) echo "NVIDIA A100-SXM4-80GB|NVIDIA A100 80GB PCIe" ;;
    h100) echo "NVIDIA H100 NVL|NVIDIA H100 80GB HBM3|NVIDIA H100 PCIe" ;;
    *)    echo "$1" ;;   # an explicit gpuTypeId passes straight through
  esac
}
