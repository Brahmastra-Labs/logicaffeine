#!/usr/bin/env python3
"""Distil an mpw_precheck run log into a Caravel project's precheck ledger.

This is the SHUTTLE ACCEPTANCE gate — the 19-check battery ChipFoundry/efabless runs to decide
whether a submission is even looked at. Passing OpenLane's own signoff is necessary but not
sufficient: the precheck additionally runs the golden Magic DRC, the KLayout FEOL/BEOL/offgrid/
density/pin-label/zeroarea decks, device-level LVS against the extracted GDS, plus license, SPDX,
Makefile, defaults, documentation, netlist-consistency, GPIO-defines, XOR-vs-golden, spike,
illegal-cellname and OEB checks on the submission REPOSITORY.

Fail-closed: a check the log does not mention as PASSED is not recorded as passed.

Usage: record-mpw-precheck.py <precheck.log> <submission_dir> <out.json> [landing_dir]

The verdict names `landing_dir` as the submission when it is given: a submission checked where a batch of re-signs
staged it lands in its project's own directory, and is recorded there.
"""
import json
import os
import re
import sys


def _repo_relative(path):
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    return os.path.relpath(os.path.abspath(path), root)


def main():
    if len(sys.argv) not in (4, 5):
        sys.exit(__doc__)
    log_path, sub, out = sys.argv[1:4]
    lands = sys.argv[4] if len(sys.argv) == 5 else sub
    txt = open(log_path, encoding="utf-8", errors="replace").read()

    total = 0
    m = re.search(r"Executing Check \d+ of (\d+):", txt)
    if m:
        total = int(m.group(1))

    executed = re.findall(r"Executing Check (\d+) of \d+: (.+)", txt)
    passed = set(re.findall(r"\{\{([A-Za-z0-9 _.\-]+?) CHECK PASSED\}\}", txt))
    passed |= set(re.findall(r"\{\{([A-Za-z0-9 _.\-]+?) PASSED\}\}", txt))
    failed = sorted(set(re.findall(r"\{\{([A-Za-z0-9 _.\-]+?) CHECK FAILED\}\}", txt)))

    checks = {}
    for _, name in executed:
        name = name.strip()
        hit = any(name.upper() in p.upper() or p.upper() in name.upper() for p in passed)
        checks[name] = "passed" if hit else "not-recorded-as-passed"

    success = "{{SUCCESS}} All Checks Passed" in txt

    gds = os.path.join(sub, "gds", "user_project_wrapper.gds")
    doc = {
        "meta": {
            "gate": "efabless/ChipFoundry mpw_precheck — the shuttle ACCEPTANCE battery",
            "tool": "efabless/mpw_precheck:latest (docker) against tools/mpw_precheck",
            "pdk": os.environ.get("PDK", "sky130A"),
            "submission": f"{_repo_relative(lands)} (assembled by scripts/caravel-submission.sh)",
        },
        "checks_total": total,
        "checks_executed": len(executed),
        "checks_passed": sum(1 for v in checks.values() if v == "passed"),
        "checks_failed": failed,
        "checks": checks,
        "submitted_gds_bytes": os.path.getsize(gds) if os.path.exists(gds) else 0,
        "all_checks_passed": bool(success and not failed),
    }
    json.dump(doc, open(out, "w"), indent=2)
    open(out, "a").write("\n")
    print(f"== mpw_precheck: {doc['checks_passed']}/{doc['checks_executed']} checks passed "
          f"({'ACCEPTED' if doc['all_checks_passed'] else 'NOT ACCEPTED'}) ==")
    for n, v in checks.items():
        if v != "passed":
            print(f"   !! {n}: {v}")
    sys.exit(0 if doc["all_checks_passed"] else 1)


if __name__ == "__main__":
    main()
