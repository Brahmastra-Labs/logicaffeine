#!/usr/bin/env bash
# conformance-lean-oracle.sh — extract GROUND TRUTH from Lean 4 for the conformance corpus.
#
# WHY THIS EXISTS. A LOGOS-only test proves we do *something*; it cannot prove we do what LEAN
# does, because the expected value was written by the same person who wrote the implementation.
# That is the gameable shape, and it is exactly what the corpus specification warns about: "the target
# kernel cannot be your oracle."
#
# So we ask Lean. For every case in the corpus this runs the real Lean 4.14.0 compiler over the
# real case file and records, per declaration Lean actually elaborated:
#
#   name · kind · universe parameters · the elaborated type · the TRANSITIVE axiom set
#
# Those recorded facts are the expected values our tests assert against. A LOGOS test that claims
# "we state this the way Lean does" must agree with a fact Lean itself emitted — nobody on our side
# gets to choose it.
#
# Usage:
#   scripts/lean/conformance-lean-oracle.sh            # all 248 cases
#   scripts/lean/conformance-lean-oracle.sh T01-007    # one case, by id prefix
#
# Output: crates/logicaffeine_tests/tests/fixtures/lean_conformance/lean_ground_truth/<CASE_ID>.json  (checked in — it is the oracle)
#         crates/logicaffeine_tests/tests/fixtures/lean_conformance/lean_ground_truth/SUMMARY.json

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CASES_DIR="$REPO_ROOT/crates/logicaffeine_tests/tests/fixtures/lean_conformance/cases"
OUT_DIR="$REPO_ROOT/crates/logicaffeine_tests/tests/fixtures/lean_conformance/lean_ground_truth"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

export PATH="$HOME/.elan/bin:$PATH"

if ! command -v lean >/dev/null 2>&1; then
  echo "FATAL: lean is not on PATH. Install with:" >&2
  echo "  curl -sSf https://raw.githubusercontent.com/leanprover/elan/master/elan-init.sh | sh -s -- -y --default-toolchain leanprover/lean4:v4.14.0" >&2
  exit 127
fi

LEAN_VERSION="$(lean --version 2>/dev/null | tail -1)"
case "$LEAN_VERSION" in
  *"4.14.0"*) ;;
  *) echo "FATAL: the corpus is pinned to Lean 4.14.0; found: $LEAN_VERSION" >&2; exit 1 ;;
esac

FILTER="${1:-}"
mkdir -p "$OUT_DIR"

# Be a good citizen: this box routinely has a concurrent agent session compiling Rust.
JOBS="${ORACLE_JOBS:-4}"
NICE=(nice -n 10)

# ---- the trailer: walks the environment and emits one JSON row per declaration ---------------
# `$NS` is substituted per case. Only constants under the case's own namespace are reported, so
# Lean core never leaks in. `collectAxioms` is the programmatic form of `#print axioms`.
emit_trailer() {
  local ns="$1"
  cat <<TRAILER

open Lean Elab Command in
#eval show CommandElabM Unit from do
  let env ← getEnv
  let ns : Name := \`$ns
  let mut arr : Array Json := #[]
  for (n, ci) in env.constants.toList do
    if ns.isPrefixOf n && !n.isInternal then
      let kind := match ci with
        | .axiomInfo _  => "axiom"
        | .defnInfo _   => "def"
        | .thmInfo _    => "theorem"
        | .opaqueInfo _ => "opaque"
        | .quotInfo _   => "quot"
        | .inductInfo _ => "inductive"
        | .ctorInfo _   => "ctor"
        | .recInfo _    => "rec"
      let tyStr ← liftTermElabM do
        let f ← Meta.ppExpr ci.type
        pure (toString f)
      let axs ← liftCoreM <| collectAxioms n
      arr := arr.push <| Json.mkObj [
        ("name",   Json.str (toString n)),
        ("kind",   Json.str kind),
        ("levels", Json.arr (ci.levelParams.toArray.map (fun l => Json.str (toString l)))),
        ("type",   Json.str tyStr),
        ("axioms", Json.arr (axs.map (fun a => Json.str (toString a))))
      ]
  IO.println ("CONFORMANCE_ORACLE_JSON " ++ (Json.arr arr).compress)
TRAILER
}

one_case() {
  local src="$1"
  local base case_id ns tmp out raw json_line rc stderr_txt

  base="$(basename "$src" .lean)"
  case_id="${base%%_*}"          # T00-001_de_bruijn_… -> T00-001
  ns="${case_id//-/_}"           # T00-001 -> T00_001
  tmp="$WORK_DIR/$ns.lean"
  out="$OUT_DIR/$case_id.json"

  # Imports must be the first commands in a Lean module. Five corpus files already open with
  # `import Lean`; the rest import nothing, so we add it for the trailer's metaprogramming.
  if head -1 "$src" | grep -q '^import '; then
    cp "$src" "$tmp"
  else
    { echo "import Lean"; echo; cat "$src"; } > "$tmp"
  fi
  emit_trailer "$ns" >> "$tmp"

  raw="$("${NICE[@]}" timeout 300 lean "$tmp" 2>&1)"
  rc=$?

  json_line="$(printf '%s\n' "$raw" | grep -m1 '^CONFORMANCE_ORACLE_JSON ' | sed 's/^CONFORMANCE_ORACLE_JSON //')"
  # Diagnostics are everything that is not our sentinel line. `sorry` warnings are expected on a
  # handful of cases and are recorded, not hidden.
  stderr_txt="$(printf '%s\n' "$raw" | grep -v '^CONFORMANCE_ORACLE_JSON ' | head -40)"

  local compiles="true"
  if [ "$rc" -ne 0 ]; then compiles="false"; fi
  if [ -z "$json_line" ]; then json_line="[]"; fi

  python3 - "$out" "$case_id" "$compiles" "$rc" "$json_line" <<'PY' "$stderr_txt"
import json, sys
out, case_id, compiles, rc, decls_raw = sys.argv[1:6]
diagnostics = sys.argv[6] if len(sys.argv) > 6 else ""
try:
    decls = json.loads(decls_raw)
except Exception as e:
    decls, diagnostics = [], f"{diagnostics}\n[oracle] undecodable payload: {e}"
decls.sort(key=lambda d: d.get("name", ""))
json.dump({
    "case": case_id,
    "lean": "4.14.0",
    "compiles": compiles == "true",
    "exit_code": int(rc),
    "declaration_count": len(decls),
    "declarations": decls,
    "diagnostics": diagnostics.strip(),
}, open(out, "w"), indent=2, sort_keys=False)
PY

  if [ "$compiles" = "true" ]; then
    printf '  ok   %-10s %3d decls\n' "$case_id" "$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['declaration_count'])" "$out")"
  else
    printf '  FAIL %-10s exit %s\n' "$case_id" "$rc"
  fi
}
export -f one_case emit_trailer
export WORK_DIR OUT_DIR

mapfile -t FILES < <(find "$CASES_DIR" -name '*.lean' | sort)
if [ -n "$FILTER" ]; then
  mapfile -t FILES < <(printf '%s\n' "${FILES[@]}" | grep "/$FILTER")
fi

echo "Lean ground truth: ${#FILES[@]} case(s), $LEAN_VERSION, ${JOBS} at a time"
printf '%s\n' "${FILES[@]}" | xargs -P "$JOBS" -I{} bash -c 'one_case "$@"' _ {}

python3 - "$OUT_DIR" <<'PY'
import json, sys, pathlib, collections
out = pathlib.Path(sys.argv[1])
rows = [json.load(open(p)) for p in sorted(out.glob("T*.json"))]
failed = [r["case"] for r in rows if not r["compiles"]]
by_axiom = collections.Counter()
levels_used = 0
for r in rows:
    for d in r["declarations"]:
        for a in d.get("axioms", []):
            by_axiom[a] += 1
        if d.get("levels"):
            levels_used += 1
summary = {
    "lean": "4.14.0",
    "cases": len(rows),
    "compiled": len(rows) - len(failed),
    "failed": failed,
    "declarations": sum(r["declaration_count"] for r in rows),
    "universe_polymorphic_declarations": levels_used,
    "axiom_usage": dict(by_axiom.most_common()),
}
json.dump(summary, open(out / "SUMMARY.json", "w"), indent=2)
print()
print(f"  cases            : {summary['cases']}")
print(f"  compiled         : {summary['compiled']}")
print(f"  failed           : {len(failed)} {failed if failed else ''}")
print(f"  declarations     : {summary['declarations']}")
print(f"  universe-poly    : {levels_used}")
print(f"  axioms Lean used : {summary['axiom_usage']}")
PY
