#!/usr/bin/env bash
# Cold-start floor measurement: time to launch each engine and run a trivial
# program (print one constant). This is the serverless / CLI metric — a native
# interpreter pays no VM warm-up to start, where V8 carries its ~20-40ms init.
#
# Measures the LOGOS interpreter (largo run --interpret) vs Node (V8), Python and
# Ruby, then merges a "startup" block into the interpreter results JSON consumed
# by the /benchmarks page. Run standalone, or it is invoked at the end of
# run-interp-vs-js.sh so a full run produces it automatically.
#
# Usage: bash benchmarks/measure-startup.sh
# Env:   OUT=results/latest-interp.json  RUNS=50  WARMUP=5

set -uo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
RESULTS_DIR="$SCRIPT_DIR/results"
OUT="${OUT:-results/latest-interp.json}"
RUNS="${RUNS:-50}"
WARMUP="${WARMUP:-5}"

export LOGOS_WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"
LOGOS_TARGET_DIR="$SCRIPT_DIR/.logos-bench-target"
LARGO="$LOGOS_TARGET_DIR/release/logicaffeine-cli"
[ -f "$LARGO" ] || LARGO="$LOGOS_TARGET_DIR/release/largo"
[ -f "$LARGO" ] || LARGO="$SCRIPT_DIR/../target/release/logicaffeine-cli"
[ -f "$LARGO" ] || LARGO="$SCRIPT_DIR/../target/release/largo"
if [ ! -f "$LARGO" ]; then echo "largo not found; build it first"; exit 1; fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/proj/src"
cat > "$TMP/proj/Largo.toml" <<'TOML'
[package]
name = "trivial"
version = "0.1.0"
entry = "src/main.lg"
TOML
printf '## Main\nShow 0.\n' > "$TMP/proj/src/main.lg"
printf 'console.log(0)\n'   > "$TMP/trivial.js"
printf 'print(0)\n'         > "$TMP/trivial.py"
printf 'puts 0\n'           > "$TMP/trivial.rb"

# (id, label, command) — only engines present on PATH are measured
declare -a IDS LABELS CMDS
add() { IDS+=("$1"); LABELS+=("$2"); CMDS+=("$3"); }
add logos_interp "LOGOS interp" "cd $TMP/proj && LOGOS_WORKSPACE=$LOGOS_WORKSPACE $LARGO run --interpret"
command -v node    >/dev/null 2>&1 && add js     "Node (V8)" "node $TMP/trivial.js"
command -v python3 >/dev/null 2>&1 && add python "Python"    "python3 $TMP/trivial.py"
command -v ruby    >/dev/null 2>&1 && add ruby   "Ruby"      "ruby $TMP/trivial.rb"

HF_ARGS=(--warmup "$WARMUP" --runs "$RUNS" --time-unit millisecond --export-json "$TMP/startup.json")
for i in "${!IDS[@]}"; do HF_ARGS+=(-n "${LABELS[$i]}" "${CMDS[$i]}"); done

echo "Measuring cold start ($RUNS runs, $WARMUP warmup)..."
hyperfine "${HF_ARGS[@]}" || { echo "hyperfine failed"; exit 1; }

# Map hyperfine command labels back to engine ids and build the startup block.
LABELS_JSON=$(printf '%s\n' "${LABELS[@]}" | jq -R . | jq -s .)
IDS_JSON=$(printf '%s\n' "${IDS[@]}" | jq -R . | jq -s .)
STARTUP=$(jq -n --slurpfile s "$TMP/startup.json" --argjson labels "$LABELS_JSON" --argjson ids "$IDS_JSON" \
  --argjson runs "$RUNS" --argjson warmup "$WARMUP" '
  ($labels | to_entries | map({(.value): $ids[.key]}) | add) as $map
  | { runs: $runs, warmup: $warmup,
      engines: ( reduce $s[0].results[] as $r ({};
        . + { ($map[$r.command]): { mean_ms: ($r.mean*1000), min_ms: ($r.min*1000), median_ms: ($r.median*1000), stddev_ms: ($r.stddev*1000) } } )) }')

if [ -f "$RESULTS_DIR/$(basename "$OUT")" ] || [ -f "$OUT" ]; then
    TARGET="$OUT"; [ -f "$TARGET" ] || TARGET="$RESULTS_DIR/$(basename "$OUT")"
    tmp=$(mktemp)
    jq --argjson sb "$STARTUP" '.startup = $sb' "$TARGET" > "$tmp" && mv "$tmp" "$TARGET"
    echo "Merged startup block into $TARGET"
    jq -r '.startup.engines | to_entries[] | "  \(.key): \(.value.mean_ms*10|round/10)ms"' "$TARGET"
else
    echo "WARN: $OUT not found; printing startup block only:"
    echo "$STARTUP" | jq .
fi
