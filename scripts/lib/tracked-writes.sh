#!/usr/bin/env bash
# tracked-writes.sh — the tracked files a test run changed.
#
#   tracked-writes.sh snapshot <repo> <out>     every tracked file that differs from HEAD, with its content hash
#   tracked-writes.sh compare  <before> <after> the tracked files whose state differs between two snapshots;
#                                               exit 1 naming each, 0 when the run changed none
#
# A test compares against a committed file and never rewrites it; recording a ledger or a golden is an explicit
# act outside the suite. The fast runner snapshots before its first pass and after its last, so a run that
# writes a tracked file is RED however the file was dirty before (the hash catches a rewrite of an already
# modified file). Untracked files are not tracked writes. Outside a git checkout the snapshot says so and the
# comparison reports the run unchecked rather than clean.
set -u

snapshot() {
  local repo="$1" out="$2"
  if ! git -C "$repo" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "# not a git checkout" > "$out"
    return 0
  fi
  git -C "$repo" status --porcelain=v1 -z --untracked-files=no \
    | python3 -c '
import hashlib, os, sys
repo, entries = sys.argv[1], sys.stdin.buffer.read().split(b"\0")
rows, i = [], 0
while i < len(entries):
    e = entries[i]
    i += 1
    if not e:
        continue
    status, path = e[:2].decode(), e[3:].decode("utf-8", "surrogateescape")
    if status[0] in "RC":
        i += 1
    full = os.path.join(repo, path)
    digest = hashlib.sha256(open(full, "rb").read()).hexdigest() if os.path.isfile(full) else "absent"
    rows.append(f"{path}\t{digest}")
print("\n".join(sorted(rows)))
' "$repo" > "$out"
}

compare() {
  local before="$1" after="$2"
  if grep -qx "# not a git checkout" "$before" "$after" 2>/dev/null; then
    echo "not a git checkout: tracked writes unchecked"
    return 2
  fi
  python3 - "$before" "$after" <<'PY'
import sys
def load(p):
    return dict(l.split("\t", 1) for l in open(p).read().splitlines() if l and not l.startswith("#"))
before, after = load(sys.argv[1]), load(sys.argv[2])
changed = sorted(p for p in set(before) | set(after) if before.get(p) != after.get(p))
for p in changed:
    print(p)
sys.exit(1 if changed else 0)
PY
}

case "${1:-}" in
  snapshot) snapshot "$2" "$3" ;;
  compare) compare "$2" "$3" ;;
  *) sed -n '2,6p' "$0" >&2; exit 64 ;;
esac
