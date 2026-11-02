#!/usr/bin/env bash
# Bring up EXACTLY ONE pod, with the weights volume attached, and wire up `ssh pod`.
#
#   scripts/accel/runpod-up.sh --gpu dev     # cheapest in-stock card that can hold a 29 GB model (>=40 GB VRAM)
#   scripts/accel/runpod-up.sh --gpu a100    # the A100 measurement chip
#   scripts/accel/runpod-up.sh --gpu h100    # the H100 measurement chip
#   scripts/accel/runpod-up.sh --gpu "NVIDIA L40S" --name staging
#
# Refuses to deploy while ANY pod exists. That refusal is the whole point of this script: the previous
# campaign left four pods running concurrently. If you really mean it, terminate first with runpod-down.sh.

source "$(dirname "$0")/runpod-lib.sh"

ROLE=dev NAME="" DISK=80 MIN_VRAM=40
while [ $# -gt 0 ]; do
  case "$1" in
    --gpu)      ROLE="$2"; shift 2 ;;
    --name)     NAME="$2"; shift 2 ;;
    --disk)     DISK="$2"; shift 2 ;;
    --min-vram) MIN_VRAM="$2"; shift 2 ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done
[ -n "$NAME" ] || NAME="logos-$ROLE"

# ---- invariant: one pod at a time -------------------------------------------------------------
existing="$(rp_pods_tsv)"
if [ -n "$existing" ]; then
  echo "REFUSING to deploy — a pod already exists:" >&2
  echo "$existing" | while IFS=$'\t' read -r id name st cost; do
    printf '  %s  %s  %s  $%s/hr\n' "$id" "$name" "$st" "$cost" >&2
  done
  echo "Terminate it first:  scripts/accel/runpod-down.sh" >&2
  exit 1
fi

[ -f "$RP_SSH_KEY.pub" ] || { echo "no ssh pubkey at $RP_SSH_KEY.pub" >&2; exit 1; }
PUBKEY="$(cat "$RP_SSH_KEY.pub")"

# ---- pick the GPU ------------------------------------------------------------------------------
# "dev" is resolved from LIVE stock rather than a fixed list: US-KS-2 carries only a handful of types
# and they come and go within minutes (an L40 at $0.69 disappeared between two runs of this script).
if [ "$ROLE" = "dev" ]; then
  echo "resolving cheapest in-stock card with >= ${MIN_VRAM}GB in $RP_DATACENTER ..."
  CANDIDATES="$(rp_gql "query{gpuTypes{id memoryInGb lowestPrice(input:{gpuCount:1,dataCenterId:\"$RP_DATACENTER\"}){stockStatus uninterruptablePrice}}}" \
    | python3 -c '
import sys, json
min_vram = int(sys.argv[1])
rows = []
for t in json.load(sys.stdin)["data"]["gpuTypes"]:
    lp = t.get("lowestPrice") or {}
    if lp.get("stockStatus") and (t.get("memoryInGb") or 0) >= min_vram:
        rows.append((lp.get("uninterruptablePrice") or 999, t["id"]))
rows.sort()
print("|".join(r[1] for r in rows))' "$MIN_VRAM")"
  [ -n "$CANDIDATES" ] || { echo "nothing in stock with >= ${MIN_VRAM}GB in $RP_DATACENTER" >&2; exit 1; }
else
  CANDIDATES="$(rp_ladder "$ROLE")"
fi

# ---- deploy, walking the ladder on SUPPLY_CONSTRAINT ---------------------------------------------
POD_ID=""
IFS='|' read -ra GPUS <<< "$CANDIDATES"
for GPU in "${GPUS[@]}"; do
  IFS=$'\t' read -r stock price < <(rp_stock "$GPU")
  printf '  try %-34s stock=%-9s $%s/hr ... ' "$GPU" "$stock" "$price"
  [ "$stock" = "none" ] && { echo "skip (no stock)"; continue; }
  BODY="$(python3 -c '
import json, sys
gpu, name, image, dc, vol, mount, disk, pub = sys.argv[1:9]
print(json.dumps({
  "name": name, "imageName": image,
  "gpuTypeIds": [gpu], "gpuCount": 1, "cloudType": "SECURE",
  "dataCenterIds": [dc], "networkVolumeId": vol, "volumeMountPath": mount,
  "containerDiskInGb": int(disk), "ports": ["22/tcp"],
  "env": {"PUBLIC_KEY": pub}, "supportPublicIp": True,
}))' "$GPU" "$NAME" "$RP_IMAGE" "$RP_DATACENTER" "$RP_VOLUME_ID" "$RP_MOUNT" "$DISK" "$PUBKEY")"
  RESP="$(rp_rest POST /pods "$BODY" || true)"
  POD_ID="$(printf '%s' "$RESP" | python3 -c 'import sys,json
try: print(json.load(sys.stdin).get("id") or "")
except Exception: print("")')"
  if [ -n "$POD_ID" ]; then echo "DEPLOYED $POD_ID"; GPU_USED="$GPU"; break; fi
  echo "failed: $(printf '%s' "$RESP" | head -c 200)"
done
[ -n "$POD_ID" ] || { echo "could not deploy on any candidate" >&2; exit 1; }

# ---- wait for the public SSH endpoint -----------------------------------------------------------
echo -n "waiting for ssh endpoint "
IP=""; PORT=""
for _ in $(seq 1 60); do
  sleep 10; echo -n "."
  read -r IP PORT < <(rp_rest GET "/pods/$POD_ID" | python3 -c '
import sys, json
p = json.load(sys.stdin)
for pm in (p.get("portMappings") or {}).items():
    pass
pm = p.get("portMappings") or {}
ip = p.get("publicIp") or ""
port = pm.get("22") or ""
print(ip, port)') || true
  [ -n "$IP" ] && [ -n "$PORT" ] && break
done
echo
[ -n "$IP" ] && [ -n "$PORT" ] || { echo "pod $POD_ID came up but exposed no ssh port; check the console" >&2; exit 1; }

# ---- rewrite the `pod` ssh stanza ----------------------------------------------------------------
python3 - "$IP" "$PORT" "$RP_SSH_KEY" <<'PY'
import os, re, sys
ip, port, key = sys.argv[1:4]
cfg = os.path.expanduser("~/.ssh/config")
text = open(cfg).read() if os.path.exists(cfg) else ""
text = re.sub(r"(?ms)^Host pod\n(?:[ \t]+.*\n|\n)*?(?=^Host |\Z)", "", text)
stanza = (f"Host pod\n    HostName {ip}\n    Port {port}\n    User root\n"
          f"    IdentityFile {key}\n    StrictHostKeyChecking no\n"
          f"    UserKnownHostsFile /dev/null\n    ServerAliveInterval 30\n\n")
open(cfg, "w").write(stanza + text.lstrip("\n"))
print(f"~/.ssh/config: Host pod -> {ip}:{port}")
PY

echo
echo "  pod    $POD_ID  ($GPU_USED)"
echo "  ssh    ssh pod"
echo "  vol    $RP_VOLUME_ID mounted at $RP_MOUNT (weights persist; the pod does not)"
echo "  ⚠ TERMINATE WHEN DONE:  scripts/accel/runpod-down.sh"
