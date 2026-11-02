#!/usr/bin/env bash
# SN50 INTAKE — the first thing we run on the SambaNova box: probe what SDK surface the environment actually
# has, and print a filled INTAKE CARD with the day's decision tree. Read-only; prints environment variable
# NAMES only, never values.
#
#   scripts/accel/sn50-intake.sh            probe this machine and print the card
#   scripts/accel/sn50-intake.sh --dry-run  print the card with every probe marked "not probed" (no commands run)
#   scripts/accel/sn50-intake.sh --help
#
# Exit 0 whether or not an SDK is found — the card IS the answer; 2 on a bad flag.
set -uo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/accel/sn50-intake.sh [--dry-run] [--help]

Probes an SN50/SambaNova environment for its SDK surface and prints the SN50 INTAKE CARD:
  python3 -c "import sambaflow"; snrun / samba / vllm / sambastack / snfadm on PATH; vllm --version;
  pip list (samba|vllm); /opt/sambaflow and /opt/sambanova; SAMBA*/SN* environment variable NAMES; docs dirs;
  RDU device nodes. Values of environment variables are never printed.

  --dry-run   print the card with no probe executed ("not probed" on every line)
  --help      this text
EOF
}

DRY=0
for a in "$@"; do
    case "$a" in
        --help|-h) usage; exit 0 ;;
        --dry-run) DRY=1 ;;
        *) echo "sn50-intake.sh: unknown option: $a" >&2; usage >&2; exit 2 ;;
    esac
done

# probe LABEL CMD... — one line per probe: "yes: <first line of output>" / "no" / "not probed".
probe() {
    local label="$1"; shift
    if [ "$DRY" = 1 ]; then
        printf '  %-38s %s\n' "$label" "not probed (dry-run)"
        return
    fi
    local out
    if out=$("$@" 2>/dev/null) && [ -n "$out" ]; then
        printf '  %-38s %s\n' "$label" "yes: $(printf '%s' "$out" | head -1 | cut -c1-90)"
    elif out=$("$@" 2>/dev/null); then
        printf '  %-38s %s\n' "$label" "yes"
    else
        printf '  %-38s %s\n' "$label" "no"
    fi
}

env_names() {
    # NAMES only — the value side of `env` is cut away before anything is matched or printed.
    env | cut -d= -f1 | grep -E '^(SAMBA|SN)' | sort | tr '\n' ' '
}

echo "SN50 INTAKE CARD — $(date -u +%Y-%m-%dT%H:%MZ) — host $(hostname 2>/dev/null || echo '?')"
echo "  (probes are read-only; environment variable names only, never values)"
echo
echo "PROBES"
probe "python3 -c 'import sambaflow'"      python3 -c 'import sambaflow, sys; print(getattr(sambaflow, "__version__", "present"))'
probe "which snrun"                        which snrun
probe "which samba"                        which samba
probe "which vllm"                         which vllm
probe "which sambastack"                   which sambastack
probe "which snfadm"                       which snfadm
probe "vllm --version"                     vllm --version
probe "pip list | grep -i samba|vllm"      bash -c 'pip list 2>/dev/null | grep -i -E "samba|vllm"'
probe "ls /opt/sambaflow"                  ls /opt/sambaflow
probe "ls /opt/sambanova"                  ls /opt/sambanova
probe "docs dirs"                          bash -c 'ls -d /opt/sambaflow/doc* /opt/sambanova/doc* /usr/share/doc/sambaflow* /usr/share/doc/sambanova* 2>/dev/null'
probe "RDU device nodes (/dev/sn*, /dev/rdu*)" bash -c 'ls /dev/sn* /dev/rdu* 2>/dev/null'
probe "SAMBA*/SN* variable NAMES"          bash -c "$(declare -f env_names); env_names"
echo
cat <<'EOF'
INTAKE QUESTIONS (ask, then tick)
  [ ] SDK tier on this box:  SambaFlow (PEF compile)  |  vLLM RDU backend  |  RDA / Spatial-IR docs  |  API only
  [ ] Does the vLLM RDU backend support model_type qwen3_5 (Qwen3.8's Gated DeltaNet + gated attention)?
  [ ] PEF compile time for a 27B?  (start the TINY config the moment SambaFlow is confirmed)
  [ ] TP presets?  (16 chips per SambaRack — the TP=16 plan is benchmarks/silicon/qwen38_vllm_plan.json)
  [ ] How are placements exposed?  (neutral.json v2 + certificate → their compiler engineers; --bridge sambaflow)

DECISION TREE (hour by hour)
  0:00  intake — this card; hand over benchmarks/silicon/qwen38_sn50.json (the fit) + the vLLM plan
  0:30  route A — vLLM RDU backend supports qwen3_5:
            vllm serve Qwen/Qwen3.8-27B -tp 16 --dtype bfloat16 --max-model-len 32768 ; measure with their bench
        route B — SambaFlow in hand:
            largo emit rdu --format sambaflow <tiny Qwen3.8 program> -o out/   → compile.py compile → run (TINY first, then real)
            implement --bridge sambaflow in crates/logicaffeine_compile/src/spatial/lower/rdu_sdk.rs against the real API;
            their placement → certify_back → the proof-carrying certificate on THEIR mapping
        route C — RDA / Spatial-IR docs:
            largo emit rdu --format spatial (Spatial source) + neutral.json v2 are the inputs; bridge from the envelope
        route D — API only:
            the SambaCloud baseline (scripts/accel/sambacloud-smoke.sh: TTFT + tok/s) + the `largo emit rdu` demo stand alone
  measure: tok/s per user, batch, TTFT, MBU against the 2 TB/s scale-up + HBM numbers
  HONESTY: never quote a tok/s we did not measure on their hardware that day; everything else is `modeled`.
EOF
