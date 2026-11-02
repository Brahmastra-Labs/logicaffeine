#!/usr/bin/env bash
# Re-sign a recorded block under the current flow: run its OWN OpenLane configuration again, unchanged,
# over the overlay as it stands, then anchor IR, record it into the ledger that holds it and run the
# golden deck on the GDS it ships.
#
#   scripts/resign-block.sh <block>            re-sign
#   scripts/resign-block.sh --check <block>    only the refusals below; prints the evidence run
#
# `openlane-run.sh` re-emits a block's config.json from today's `ClosureSettings`, so a flow fix that must
# be re-proven on blocks already signed off would rebuild them with a different recipe. This keeps the
# recipe: the design's config.json, refused unless it is exactly what the run its ledger entry reproduces
# from started with (runs/<RUN>/config_in.tcl), and unless no file in src/ is newer than that run.
#
# THE WHOLE BODY IS ONE FUNCTION, called on the last line, for the reason openlane-run.sh gives: bash
# reads a plain script incrementally, and a function body is parsed whole before its first command runs.
set -euo pipefail

logos_resign_main() {
  local CHECK=0
  if [ "${1:-}" = "--check" ]; then CHECK=1; shift; fi
  local BLOCK="${1:?usage: resign-block.sh [--check] <block>}"
  local ROOT; ROOT="$(cd "$(dirname "$0")/.." && pwd)"
  local OL="$ROOT/tools/OpenLane"
  local SIL="$ROOT/benchmarks/silicon"
  [ -d "$OL" ] || { echo "FATAL: OpenLane not installed at $OL" >&2; return 1; }

  # The ledger that holds the block, and the run its entry reproduces from — found by the recorder's own
  # verifier, so the run is the one whose reports the ledger's numbers are, not the newest by name or mtime.
  local FOUND
  FOUND=$(python3 - "$ROOT" "$SIL" "$BLOCK" <<'PYFIND'
import json, os, subprocess, sys
root, sil, block = sys.argv[1:4]
for entry in json.load(open(os.path.join(sil, "sky130_ledgers.json")))["ledgers"]:
    led = os.path.join(root, entry["path"])
    if block not in (json.load(open(led)).get("blocks") or {}):
        continue
    r = subprocess.run([sys.executable, os.path.join(root, "scripts/record-sky130-signoff.py"), "--verify", led],
                       capture_output=True, text=True, cwd=root)
    b = json.loads(r.stdout)["blocks"].get(block, {})
    if b.get("status") != "reproduces":
        sys.exit(f"FATAL: {block}'s entry in {led} does not reproduce from any run ({b.get('status')}); "
                 "there is no recorded recipe to re-sign")
    print(led, b["run_dir"])
    break
else:
    sys.exit(f"FATAL: {block} is in no registered block ledger ({sil}/sky130_ledgers.json)")
PYFIND
  ) || return 4
  local LEDGER="${FOUND%% *}" EVIDENCE="${FOUND#* }"
  local D; D="$(dirname "$(dirname "$EVIDENCE")")"
  local DES; DES="$(basename "$D")"

  # The recipe is the one the evidence was built with, or nothing is re-signed.
  python3 - "$D/config.json" "$EVIDENCE/config_in.tcl" "$D" <<'PYRECIPE' || return 4
import json, os, re, sys
cfg_path, cin_path, design = sys.argv[1:4]
cfg = json.load(open(cfg_path))
cin = dict(re.findall(r'set ::env\((\w+)\)\s+"?(.*?)"?\s*$', open(cin_path).read(), re.M))
container = "/openlane/designs/" + design.rstrip("/").split("/")[-1] + "/"
def same(k, v):
    sv = " ".join(map(str, v)) if isinstance(v, list) else str(v)
    got = cin.get(k)
    if got is None:
        return False
    if sv.startswith("dir::"):
        # OpenLane expands `dir::` against the design directory, globs included.
        pat = re.escape(container + sv[len("dir::"):]).replace(r"\*", r"[^ ]*")
        return all(re.fullmatch(pat, tok) for tok in got.split()) and got.strip() != ""
    if sv.strip() == got.strip():
        return True
    try:
        return abs(float(sv) - float(got)) < 1e-9
    except ValueError:
        return False
bad = [k for k, v in cfg.items() if not same(k, v)]
# OpenLane adds these four to config_in.tcl itself; any other key it holds that config.json lacks was
# removed from the recipe and would now fall back to a default.
bad += [f"{k} (removed)" for k in cin if k not in cfg and k not in ("DESIGN_DIR", "PDK", "PDKPATH", "SCLPATH")]
if cin.get("PDK", "") != os.environ.get("PDK", "sky130A"):
    bad.append(f"PDK ({cin.get('PDK')} recorded, {os.environ.get('PDK', 'sky130A')} now)")
if bad:
    sys.exit(f"FATAL: {cfg_path} is not the recipe {cin_path} was built from ({', '.join(sorted(bad))}); "
             "re-sign only re-runs a recorded recipe — use openlane-run.sh for a new one")
PYRECIPE
  local NEWER; NEWER=$(find "$D/src" -type f -newer "$EVIDENCE/config_in.tcl" | head -5)
  if [ -n "$NEWER" ]; then
    echo "FATAL: RTL in $D/src changed after the evidence run $(basename "$EVIDENCE"):" >&2
    echo "$NEWER" >&2
    return 4
  fi
  echo "== $BLOCK: recipe of $(basename "$EVIDENCE") unchanged ($LEDGER) =="
  [ "$CHECK" = 1 ] && return 0

  # The memory headroom openlane-run.sh requires, for the reason it records (an OOM kill at 98.7 GB).
  local FREE; FREE=$(free -g | awk '/^Mem:/{print $7}' || true)
  if [ "${FREE:-0}" -lt "${OL_MIN_FREE_GB:-80}" ]; then
    echo "FATAL: only ${FREE}GB available; a flow needs headroom up to ~99GB at peak (OL_MIN_FREE_GB overrides)" >&2
    return 4
  fi

  local BEFORE; BEFORE=$(ls -1d "$D/runs/"*/ 2>/dev/null | sort || true)
  bash "$ROOT/scripts/openlane-overlay/apply.sh"
  ( cd "$OL" && sg docker -c "make quick_run QUICK_RUN_DESIGN=$DES" )
  local RUN; RUN=$(comm -13 <(printf '%s\n' "$BEFORE") <(ls -1d "$D/runs/"*/ 2>/dev/null | sort || true) | tail -1)
  [ -n "$RUN" ] || { echo "FATAL: the flow created no run under $D/runs" >&2; return 2; }
  RUN="${RUN%/}"
  echo "== run: $RUN =="
  if ! bash "$ROOT/scripts/anchor-ir-drop.sh" "$RUN" 2>&1 | grep -E "anchoring|ANCHORED|FATAL"; then
    echo "WARNING: IR anchoring failed for $RUN — the recorded ir_drop_worst_mv is UNANCHORED" >&2
  fi
  local OK=0
  python3 "$ROOT/scripts/record-sky130-signoff.py" "$RUN" "$BLOCK" "$LEDGER" || OK=1
  local GDS="$RUN/results/final/gds/$BLOCK.gds"
  bash "$ROOT/scripts/run-golden-precheck-drc.sh" "$BLOCK" "$GDS" || OK=1
  return $OK
}
logos_resign_main "$@"; exit $?
