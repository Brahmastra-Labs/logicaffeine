#!/usr/bin/env python3
"""Run a checker binary over the vendored Arena corpus and report verdicts against expectations.

usage: run-corpus.py <checker> [--include-heavy] [--timeout S] [--jobs N] [--filter SUBSTR] [--json OUT]
                      [--args "--check sem"]
Exit codes follow the Arena protocol: 0 accept, 1 reject, 2 decline. A case is WRONG when an
`accept` case is rejected or a `reject` case is accepted; declines and timeouts are reported
separately and never counted as correct.
"""
import argparse, concurrent.futures, json, os, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
CORPUS = os.path.join(ROOT, "crates/logicaffeine_tests/tests/fixtures/arena_corpus")


def run(checker, case, timeout, extra=()):
    path = os.path.join(CORPUS, case["path"]) if case["location"] == "vendored" else os.path.join(ROOT, case["path"])
    t0 = time.monotonic()
    try:
        p = subprocess.run([checker, *extra, path], capture_output=True, text=True, timeout=timeout)
        code, err = p.returncode, p.stderr.strip()
    except subprocess.TimeoutExpired:
        code, err = None, f"timeout after {timeout}s"
    dt = time.monotonic() - t0
    got = {0: "accept", 1: "reject", 2: "decline"}.get(code, "timeout" if code is None else f"error({code})")
    exp = case["outcome"]
    if got in ("decline", "timeout") or got.startswith("error"):
        status = got if not got.startswith("error") else "error"
    elif exp == "either" or got == exp:
        status = "correct"
    else:
        status = "WRONG"
    return {"name": case["name"], "expected": exp, "got": got, "status": status, "seconds": round(dt, 3), "stderr": err[-600:]}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("checker")
    ap.add_argument("--include-heavy", action="store_true")
    ap.add_argument("--timeout", type=float, default=60)
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--filter", default="")
    ap.add_argument("--json")
    ap.add_argument("--args", default="", help="extra checker arguments, before the input path")
    a = ap.parse_args()
    cases = json.load(open(os.path.join(CORPUS, "manifest.json")))["cases"]
    cases = [c for c in cases if a.filter in c["name"]]
    cases = [c for c in cases if c["location"] == "vendored" or (a.include_heavy and c["location"] == "heavy")]
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as ex:
        extra = a.args.split()
        results = list(ex.map(lambda c: run(a.checker, c, a.timeout, extra), cases))
    tally = {}
    for r in results:
        tally[r["status"]] = tally.get(r["status"], 0) + 1
    for r in sorted(results, key=lambda r: r["name"]):
        if r["status"] != "correct":
            print(f"{r['status']:8} {r['name']:55} expected {r['expected']:7} got {r['got']:8} {r['seconds']:8.2f}s  {r['stderr'].splitlines()[0] if r['stderr'] else ''}")
    slow = sorted((r for r in results if r["status"] == "correct"), key=lambda r: -r["seconds"])[:8]
    print("slowest correct:", ", ".join(f"{r['name']} {r['seconds']}s" for r in slow))
    print("TALLY", json.dumps(tally, sort_keys=True), "of", len(results))
    if a.json:
        json.dump(results, open(a.json, "w"), indent=1)
    return 0 if tally.get("WRONG", 0) == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
