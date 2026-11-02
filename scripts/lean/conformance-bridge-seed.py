#!/usr/bin/env python3
"""conformance-bridge-seed.py — seed (and re-seed) the Lean→LOGOS bridge ledger.

THE POINT. `lean_ground_truth/` records what Lean 4.14.0 actually elaborated, declaration by
declaration: its universe parameters, its elaborated type, and the transitive axioms it needed.
Those are facts nobody on our side chose. The bridge ledger answers, for each of them, the only
question that matters:

    show me the LOGOS that proves this Lean.

Every Lean declaration starts `open`. A declaration moves to `bridged` only when a named LOGOS test
exhibits a term whose type is the translation of the type Lean recorded — so a test cannot be
satisfied by asserting something easier than what Lean did. `lean_bridge_lock.rs` ratchets the open
count downward and prints the worklist. Zero open is parity.

Re-running is safe and idempotent: entries already `bridged` or `impossible` are preserved, newly
appearing Lean declarations are added as `open`, and declarations Lean no longer emits are dropped.

Usage:
    scripts/lean/conformance-bridge-seed.py            # seed/refresh every case
    scripts/lean/conformance-bridge-seed.py T01-007    # one case
"""

from __future__ import annotations

import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
TRUTH = ROOT / "crates/logicaffeine_tests/tests/fixtures/lean_conformance/lean_ground_truth"
BRIDGE = ROOT / "crates/logicaffeine_tests/tests/fixtures/lean_conformance/bridge"

# Lean synthesises these alongside every inductive/def. They are Lean *artifacts*, not mathematics
# the case author wrote. We track them separately because our own story for them differs by
# construction: LOGOS derives recursors rather than declaring them, has no `noConfusion`/`injEq` at
# all, and has no `sizeOf`. Counting them as authored obligations would drown the real signal.
GENERATED_SUFFIXES = (
    ".rec", ".recOn", ".casesOn", ".brecOn", ".below", ".ibelow", ".binductionOn",
    ".noConfusion", ".noConfusionType", ".injEq", ".inj", ".sizeOf_spec", ".ctorIdx",
    ".toCtorIdx", ".eq_def", ".match_1", ".proof_1", ".ofNat", ".induct", ".fun_cases",
)


def is_generated(name: str) -> bool:
    if any(name.endswith(s) for s in GENERATED_SUFFIXES):
        return True
    # `f.eq_1`, `f.eq_2`, … equation lemmas; `_sunfold`/`_unsafe_rec` internals.
    tail = name.rsplit(".", 1)[-1]
    if tail.startswith("eq_") and tail[3:].isdigit():
        return True
    if tail.startswith("match_") and tail[6:].isdigit():
        return True
    if tail.startswith("proof_") and tail[6:].isdigit():
        return True
    return tail.startswith("_")


def seed_case(truth_path: pathlib.Path) -> tuple[int, int, int]:
    truth = json.loads(truth_path.read_text())
    case = truth["case"]
    out = BRIDGE / f"{case}.json"

    previous: dict[str, dict] = {}
    if out.exists():
        for entry in json.loads(out.read_text()).get("declarations", []):
            previous[entry["lean"]] = entry

    rows, kept, added = [], 0, 0
    for decl in truth["declarations"]:
        name = decl["name"]
        if is_generated(name):
            continue
        prior = previous.get(name)
        if prior and prior.get("status") in ("bridged", "impossible"):
            # Preserve the work, but always refresh Lean's side — the oracle is authoritative.
            prior["lean_type"] = decl["type"]
            prior["lean_levels"] = decl["levels"]
            prior["lean_kind"] = decl["kind"]
            prior["lean_axioms"] = decl["axioms"]
            rows.append(prior)
            kept += 1
        else:
            rows.append({
                "lean": name,
                "lean_kind": decl["kind"],
                "lean_levels": decl["levels"],
                "lean_type": decl["type"],
                "lean_axioms": decl["axioms"],
                "status": "open",
                "test": "",
                "logos": "",
                "why": "",
            })
            added += 1

    rows.sort(key=lambda r: r["lean"])
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps({
        "case": case,
        "lean": truth["lean"],
        "lean_compiles": truth["compiles"],
        "declarations": rows,
    }, indent=2) + "\n")
    return len(rows), kept, added


def main() -> int:
    if not TRUTH.exists():
        print(f"FATAL: {TRUTH} does not exist — run scripts/lean/conformance-lean-oracle.sh first",
              file=sys.stderr)
        return 1

    wanted = sys.argv[1] if len(sys.argv) > 1 else None
    paths = sorted(p for p in TRUTH.glob("T*.json"))
    if wanted:
        paths = [p for p in paths if p.stem.startswith(wanted)]
    if not paths:
        print(f"no ground-truth files matched {wanted!r}", file=sys.stderr)
        return 1

    total = kept_total = added_total = 0
    for p in paths:
        n, kept, added = seed_case(p)
        total += n
        kept_total += kept
        added_total += added

    print(f"  cases            : {len(paths)}")
    print(f"  authored decls   : {total}")
    print(f"  preserved        : {kept_total} (already bridged or argued impossible)")
    print(f"  newly open       : {added_total}")
    print()
    print(f"  OPEN = {total - kept_total}. That is the number to drive to zero.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
