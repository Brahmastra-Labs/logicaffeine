#!/usr/bin/env bash
# Record the MaCSad contract fixtures from REAL `largo lean` runs — one per result outcome, plus a replay
# receipt — into crates/logicaffeine_tests/tests/fixtures/lean_conformance/contract/fixtures/. These are recordings for building W08/W09 against before a
# worker exists. They are marked as recorded (FIXTURES.json: "kind": "recorded"), never presented as live results,
# and `lean_contract_lock.rs` checks every one against the contract.
#
#   scripts/lean/lean-contract-fixtures.sh              # uses target/lean-demo/release/largo
#   LARGO=/path/to/largo scripts/lean/lean-contract-fixtures.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
LARGO="${LARGO:-$ROOT/target/lean-demo/release/largo}"
EXPORTER="$ROOT/reference-kernels/lean4export"
PROJECTS="$ROOT/crates/logicaffeine_tests/tests/fixtures/lean/projects"
FIX="$ROOT/crates/logicaffeine_tests/tests/fixtures/lean_conformance/contract/fixtures"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$FIX"

record() { # <outcome> <exit expected> <project> <target> [extra args...]
  local outcome="$1" expect="$2" project="$3" target="$4"; shift 4
  local exit=0
  "$LARGO" lean import --project "$PROJECTS/$project" --target "$target" --out "$WORK/$outcome" \
    --exporter "$EXPORTER" "$@" > "$FIX/$outcome.result.json" 2> "$FIX/$outcome.diagnostics.ndjson" || exit=$?
  [[ "$exit" == "$expect" ]] || { echo "$outcome: expected exit $expect, got $exit" >&2; exit 1; }
  echo "$exit" > "$FIX/$outcome.exit"
}

record verified               0 t06_002        T06_002.transport
record invalid_source         1 invalid_syntax Broken.fine
record unsupported_feature    2 unsupported    Runtime.identity
record disallowed_assumptions 3 sorry          Gap.viaHole
record resource_limit         4 t06_002        T06_002.transport --max-export-bytes 1024
record infrastructure_failure 5 t06_002        T06_002.transport --lean /nonexistent/logos-no-such-lean

"$LARGO" lean replay "$WORK/verified" > "$FIX/replay.receipt.json"
rm -rf "$FIX/verified.package"
cp -r "$WORK/verified" "$FIX/verified.package"

python3 - "$FIX" "$LARGO" <<'EOF'
import hashlib, json, pathlib, subprocess, sys, datetime
fix, largo = pathlib.Path(sys.argv[1]), sys.argv[2]
files = sorted(p.name for p in fix.iterdir() if p.is_file() and p.name != "FIXTURES.json")
index = {
    "kind": "recorded",
    "note": "Recorded from real `largo lean` runs for building against before a worker exists. Not live results.",
    "recorded_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "largo_sha256": hashlib.sha256(pathlib.Path(largo).read_bytes()).hexdigest(),
    "files": {name: hashlib.sha256((fix / name).read_bytes()).hexdigest() for name in files},
}
(fix / "FIXTURES.json").write_text(json.dumps(index, indent=1, sort_keys=True) + "\n")
EOF
ls "$FIX"
