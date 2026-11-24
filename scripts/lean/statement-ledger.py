#!/usr/bin/env python3
"""Import the statement of every problem of a pinned corpus with `largo lean import --statement`,
and record each problem's outcome in a ledger.

The corpus is the formalization-lab manifest (`--manifest`, its `problems`): each problem
names its repository, project directory, Lean module and declaration. Each repository's checkout is
found under `--references` (`<references>/<repository>/<project directory>`), built at its pinned
toolchain. Every problem is imported in project mode — the profile its `lean-toolchain` pins, its
Lake dependencies verified, only its module compiled — and the result document, the published
statement package (a verified problem's) and the run's wall time are recorded:

    scripts/lean/statement-ledger.py --manifest logs/alejandro200/references/formalization-lab/open-problems/manifest.json \\
        --references logs/alejandro200/references --largo <largo> --runs <dir> \\
        --ledger crates/logicaffeine_tests/tests/fixtures/lean/ledgers/alejandro200.statement.json

`<runs>/<id>/` keeps each problem's result document, its diagnostics and — for a verified problem —
its package; `<runs>/runs.ndjson` records every run as it finishes, so a long ledger resumes where it
stopped (`--resume`). The ledger itself is the summary `lean_statement_ledger_lock` holds: one entry
per problem, its outcome, its stages and the typed codes of its refusal. This tool never runs git.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import datetime
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import threading
import time

LEDGER_SCHEMA = "logos-lean-statement-ledger/1"


def sha256_file(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run_problem(problem: dict, args, lock: threading.Lock) -> dict:
    project = pathlib.Path(args.references) / problem["repository"] / problem["project_directory"]
    runs = pathlib.Path(args.runs) / problem["id"]
    runs.mkdir(parents=True, exist_ok=True)
    out = runs / "package"
    command = [
        args.largo, "lean", "import",
        "--project", str(project.resolve()),
        "--module", problem["lean_module"],
        "--target", problem["declaration"],
        "--statement",
        "--out", str(out),
        "--timeout-secs", str(args.timeout),
    ]
    started = time.monotonic()
    try:
        ran = subprocess.run(command, capture_output=True, timeout=args.timeout + 120)
        stdout, stderr, exit_code = ran.stdout, ran.stderr, ran.returncode
    except subprocess.TimeoutExpired as expired:
        stdout, stderr, exit_code = expired.stdout or b"", expired.stderr or b"", None
    wall = time.monotonic() - started
    (runs / "result.json").write_bytes(stdout)
    (runs / "diagnostics.ndjson").write_bytes(stderr)
    try:
        result = json.loads(stdout)
    except ValueError:
        result = None
    entry = {
        "id": problem["id"],
        "repository": problem["repository"],
        "declaration": problem["declaration"],
        "declaration_kind": problem["declaration_kind"],
        "module": problem["lean_module"],
        "toolchain": problem["lean_toolchain"],
        "exit": exit_code,
        "wall_seconds": round(wall, 1),
    }
    if result is None:
        entry.update({"outcome": "no_result", "stages": None, "codes": [],
                      "first": {"code": "no_result", "stage": None,
                                "message": stderr.decode(errors="replace")[-2000:]}})
    else:
        diagnostics = result.get("diagnostics", [])
        first = next((d for d in diagnostics), None)
        entry.update({
            "schema": result.get("schema"),
            "outcome": result.get("outcome"),
            "proof_status": result.get("proof_status"),
            "stages": result.get("stages"),
            "codes": sorted({d["code"] for d in diagnostics}),
            "first": None if first is None else {
                "code": first["code"], "stage": first["stage"],
                "declaration": first.get("declaration"),
                "message": first["message"][:2000],
            },
            "statement": result.get("statement"),
            "assumptions": result.get("assumptions"),
            "package_manifest_sha256": (result.get("package") or {}).get("manifest_sha256"),
            "logos": (result.get("target") or {}).get("logos"),
        })
    with lock:
        with open(pathlib.Path(args.runs) / "runs.ndjson", "a") as log:
            log.write(json.dumps(entry, sort_keys=True) + "\n")
        print(f"{entry['id']} {entry['outcome']} ({entry['wall_seconds']} s)"
              + ("" if entry["outcome"] == "verified" else f" {entry['first']['code'] if entry['first'] else ''}"),
              flush=True)
    return entry


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--references", required=True)
    parser.add_argument("--largo", required=True)
    parser.add_argument("--runs", required=True)
    parser.add_argument("--ledger", required=True)
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--timeout", type=int, default=3600)
    parser.add_argument("--only", nargs="*", help="problem ids to run (default: every problem)")
    parser.add_argument("--resume", action="store_true", help="keep the runs already recorded")
    parser.add_argument("--no-run", action="store_true",
                        help="write the ledger from the runs already recorded, running nothing")
    args = parser.parse_args()
    manifest_path = pathlib.Path(args.manifest)
    manifest = json.loads(manifest_path.read_text())
    problems = manifest["problems"]
    runs = pathlib.Path(args.runs)
    runs.mkdir(parents=True, exist_ok=True)
    recorded: dict[str, dict] = {}
    log = runs / "runs.ndjson"
    if (args.resume or args.no_run) and log.exists():
        for line in log.read_text().splitlines():
            entry = json.loads(line)
            recorded[entry["id"]] = entry
    elif log.exists():
        log.unlink()
    chosen = [p for p in problems if not args.only or p["id"] in args.only]
    pending = [] if args.no_run else [p for p in chosen if p["id"] not in recorded]
    lock = threading.Lock()
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for entry in pool.map(lambda p: run_problem(p, args, lock), pending):
            recorded[entry["id"]] = entry
    entries = [recorded[p["id"]] for p in problems if p["id"] in recorded]
    counts: dict[str, int] = {}
    for entry in entries:
        counts[entry["outcome"]] = counts.get(entry["outcome"], 0) + 1
    refusals: dict[str, int] = {}
    for entry in entries:
        if entry["outcome"] != "verified" and entry.get("first"):
            key = f"{entry['first']['stage']}:{entry['first']['code']}"
            refusals[key] = refusals.get(key, 0) + 1
    largo = pathlib.Path(args.largo)
    ledger = {
        "schema": LEDGER_SCHEMA,
        "corpus": {
            "manifest": str(manifest_path),
            "manifest_sha256": sha256_file(manifest_path),
            "problems": len(problems),
        },
        "largo_sha256": sha256_file(largo) if largo.is_file() else None,
        "recorded": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "complete": len(entries) == len(problems),
        "counts": dict(sorted(counts.items())),
        "refusals": dict(sorted(refusals.items())),
        "entries": entries,
    }
    pathlib.Path(args.ledger).parent.mkdir(parents=True, exist_ok=True)
    pathlib.Path(args.ledger).write_text(json.dumps(ledger, indent=1, sort_keys=True) + "\n")
    print(json.dumps({"counts": ledger["counts"], "refusals": ledger["refusals"]}, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
