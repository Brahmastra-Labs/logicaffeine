#!/usr/bin/env bash
#
# test-home-parity.sh — proves a test-home wave lost no test.
#
#   ./scripts/test/test-home-parity.sh before <crate dir> [cargo args…]
#   ./scripts/test/test-home-parity.sh after  <crate dir> [cargo args…]
#
# `before` lists every test of the crate's package (unit tests in its lib/bin targets, integration
# binaries in tests/) with `cargo nextest list`. `after` lists the same package again plus the
# binaries the wave moved into `logicaffeine-tests` (read from the mover's ledger,
# logs/test-home/<crate dir>-ledger.tsv), and compares:
#
#   unit         the package's lib/bin test cases are identical, ignored flags included (the stub keeps
#                every module path, so every name is unchanged);
#   integration  every (binary, test) that left the package arrived in logicaffeine-tests under the
#                binary's ledger name, ignored flags included, and nothing else arrived.
#
# Extra cargo args apply to the package listings; HOME_ARGS (e.g. `--features verification`) to the
# logicaffeine-tests listing, which reproduces the build condition the binaries had at home. Exit 0
# iff parity holds.
# Output: logs/test-home/<crate dir>-{before,after}.json and <crate dir>-parity.txt.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"
export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export CARGO_TARGET_DIR="${TEST_HOME_TARGET:-/mnt/build/test-home}"
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"

MODE="${1:-}"; CRATE_DIR="${2:-}"; shift 2 || true
case "$MODE" in before|after) ;; *) echo "usage: $0 <before|after> <crate dir> [cargo args…]" >&2; exit 2 ;; esac
[ -f "$CRATE_DIR/Cargo.toml" ] || { echo "$CRATE_DIR is not a package directory" >&2; exit 2; }
NAME="$(basename "$CRATE_DIR")"
PKG="$(python3 -c 'import sys,tomllib;print(tomllib.load(open(sys.argv[1],"rb"))["package"]["name"])' "$CRATE_DIR/Cargo.toml")"
OUT="logs/test-home"
mkdir -p "$OUT"

list() { # $1 = output json, rest = cargo selection args
  local json="$1"; shift
  if ! cargo nextest list --message-format json "$@" > "$json" 2> "$json.stderr"; then
    echo "cargo nextest list failed; see $json.stderr" >&2
    tail -40 "$json.stderr" >&2
    exit 1
  fi
}

if [ "$MODE" = before ]; then
  list "$OUT/$NAME-before.json" -p "$PKG" "$@"
  python3 - "$OUT/$NAME-before.json" <<'EOF'
import json, sys
d = json.load(open(sys.argv[1]))
n = sum(len(s["testcases"]) for s in d["rust-suites"].values())
print(f"before: {len(d['rust-suites'])} suite(s), {n} test(s) recorded in {sys.argv[1]}")
EOF
  exit 0
fi

LEDGER="$OUT/$NAME-ledger.tsv"
[ -f "$LEDGER" ] || { echo "no ledger at $LEDGER — run the mover's apply first" >&2; exit 2; }
[ -f "$OUT/$NAME-before.json" ] || { echo "no before listing at $OUT/$NAME-before.json" >&2; exit 2; }

MOVED=$(python3 - "$LEDGER" <<'EOF'
import sys, os
for line in open(sys.argv[1]).read().splitlines()[1:]:
    kind, old, new = line.split("\t")
    if kind == "binary":
        stem = os.path.basename(new)
        print(stem[:-3] if stem.endswith(".rs") else stem)
EOF
)
TEST_ARGS=()
for b in $MOVED; do TEST_ARGS+=(--test "$b"); done

list "$OUT/$NAME-after-pkg.json" -p "$PKG" "$@"
if [ "${#TEST_ARGS[@]}" -gt 0 ]; then
  read -r -a HOME_EXTRA <<< "${HOME_ARGS:-}"
  list "$OUT/$NAME-after-home.json" -p logicaffeine-tests "${TEST_ARGS[@]}" "${HOME_EXTRA[@]}"
else
  echo '{"rust-suites":{}}' > "$OUT/$NAME-after-home.json"
fi

python3 - "$OUT/$NAME-before.json" "$OUT/$NAME-after-pkg.json" "$OUT/$NAME-after-home.json" "$LEDGER" "$PKG" "$OUT/$NAME-parity.txt" <<'EOF'
import json, os, sys
before, after_pkg, after_home, ledger, pkg, report = sys.argv[1:]

def suites(path):
    return json.load(open(path))["rust-suites"]

def cases(suite):
    return {name: bool(tc.get("ignored")) for name, tc in suite["testcases"].items()}

renames = {}
for line in open(ledger).read().splitlines()[1:]:
    kind, old, new = line.split("\t")
    if kind == "binary":
        stem = lambda p: os.path.basename(p)[:-3] if p.endswith(".rs") else os.path.basename(p)
        renames[stem(old)] = stem(new)

b = suites(before)
unit_before = {sid: cases(s) for sid, s in b.items() if s["kind"] != "test"}
integ_before = {s["binary-name"]: cases(s) for s in b.values() if s["kind"] == "test"}
a = suites(after_pkg)
unit_after = {sid: cases(s) for sid, s in a.items() if s["kind"] != "test"}
left_behind = sorted(s["binary-name"] for s in a.values() if s["kind"] == "test")
home = {s["binary-name"]: cases(s) for s in suites(after_home).values()}

problems, lines = [], []
if unit_before != unit_after:
    for sid in sorted(set(unit_before) | set(unit_after)):
        x, y = unit_before.get(sid, {}), unit_after.get(sid, {})
        for t in sorted(set(x) - set(y)):
            problems.append(f"unit test lost: {sid} {t}")
        for t in sorted(set(y) - set(x)):
            problems.append(f"unit test appeared: {sid} {t}")
        for t in sorted(set(x) & set(y)):
            if x[t] != y[t]:
                problems.append(f"unit test ignored-flag changed: {sid} {t}")
for binary in left_behind:
    problems.append(f"integration binary still in {pkg}: {binary}")
for binary, tests in sorted(integ_before.items()):
    new = renames.get(binary)
    if new is None:
        problems.append(f"integration binary {binary} is not in the ledger")
        continue
    got = home.get(new)
    if got is None:
        problems.append(f"integration binary {binary} → {new} did not arrive in logicaffeine-tests")
        continue
    if got != tests:
        for t in sorted(set(tests) - set(got)):
            problems.append(f"{binary} → {new}: test lost: {t}")
        for t in sorted(set(got) - set(tests)):
            problems.append(f"{binary} → {new}: test appeared: {t}")
        for t in sorted(set(tests) & set(got)):
            if tests[t] != got[t]:
                problems.append(f"{binary} → {new}: ignored flag changed: {t}")

nu = sum(len(v) for v in unit_before.values())
ni = sum(len(v) for v in integ_before.values())
lines.append(f"{pkg}: unit {nu} test(s) in {len(unit_before)} target(s); integration {ni} test(s) in {len(integ_before)} binar(y/ies)")
lines.append("PARITY: OK" if not problems else f"PARITY: {len(problems)} problem(s)")
lines += problems
open(report, "w").write("\n".join(lines) + "\n")
print("\n".join(lines))
sys.exit(0 if not problems else 1)
EOF
