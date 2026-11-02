#!/usr/bin/env python3
"""The functional gate of the comparative-synthesis bench — the only writer of benchmarks/silicon/qor_function.json.

    run-qor-function.py <kernel> <candidate|logos> [--mode shaped|edge] [--ops N]
    run-qor-function.py --all [--ops N] [--jobs J]   every LOGOS candidate and every manifest candidate, both modes
    run-qor-function.py verify

One run materialises the candidate inside its `qorbist_<candidate>` harness (emit_qor_harness), simulates it
with iverilog for N results, and records: the signature it folded and the golden one (the LOGOS VM's, or the
oracle the RTL generator is locked to), PASS only when they are equal; the cycle the first result arrived;
and the steady-state cycles per result over the second half of the run. The simulator's verdict lines and the
candidate record are committed as excerpts under benchmarks/silicon/qor_logs/<key>/.
"""
import argparse
import concurrent.futures
import json
import os
import re
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import qorlib  # noqa: E402

HEADER = {
    "_generated_by": "scripts/qor/run-qor-function.py (never hand-edited)",
    "schema": "logos.qor.function/1",
    "provenance": "simulated_iverilog",
    "honesty": "Each row is an RTL simulation (Icarus Verilog) of one candidate inside the shared harness: "
               "functional equality with the golden on the seeded stimulus, and cycle counts. Not silicon.",
}
RE_OPS = re.compile(r"^QOR ops (\d+) misr ([0-9a-fx]{16}) cycles (\d+)$", re.M)
RE_FIRST = re.compile(r"^QOR first cycles (\d+)$", re.M)
RE_MID = re.compile(r"^QOR mid ops (\d+) cycles (\d+)$", re.M)
RE_BAD = re.compile(r"^QOR (X op \d+|TIMEOUT.*)$", re.M)


def parse(sim_out):
    m = RE_OPS.search(sim_out)
    first = RE_FIRST.search(sim_out)
    mid = RE_MID.search(sim_out)
    bad = RE_BAD.search(sim_out)
    if bad or not m:
        return {"verdict": bad.group(1) if bad else "no verdict", "ops": None, "signature": None, "cycles": None,
                "first_result_cycle": None, "steady_cycles_per_op": None}
    ops, cycles = int(m.group(1)), int(m.group(3))
    steady = None
    if mid and ops > int(mid.group(1)):
        steady = (cycles - int(mid.group(2))) / (ops - int(mid.group(1)))
    return {"verdict": "complete", "ops": ops, "signature": m.group(2), "cycles": cycles,
            "first_result_cycle": int(first.group(1)) if first else None, "steady_cycles_per_op": steady}


def gate(kernel, candidate, mode, ops):
    """Simulate one candidate and return its ledger row, the excerpts it names under `_excerpts` (the caller records
    the row and commits them with it)."""
    with tempfile.TemporaryDirectory(dir=os.path.join(qorlib.ROOT, "logs")) as run_dir:
        rec = qorlib.emit(run_dir, kernel, candidate, mode, ops)
        files = [os.path.join(run_dir, f) for f in rec["files"] + [rec["harness"], rec["testbench"]]
                 if f.endswith((".v", ".sv"))]
        build = subprocess.run(["iverilog", "-g2012", "-s", "tb_qor", "-o", os.path.join(run_dir, "sim.vvp")] + files,
                               capture_output=True, text=True)
        if build.returncode != 0:
            sys.exit("%s: iverilog refused the candidate:\n%s" % (rec["candidate"]["name"], build.stderr[-3000:]))
        sim = subprocess.run(["vvp", "-n", os.path.join(run_dir, "sim.vvp")], cwd=run_dir, capture_output=True, text=True)
        verdict_lines = "\n".join(l for l in sim.stdout.splitlines() if l.startswith("QOR ")) + "\n"
        record = json.dumps(rec, indent=1, sort_keys=True) + "\n"
        rtl = qorlib.rtl_files(run_dir, rec)
    got = parse(verdict_lines)
    name = rec["candidate"]["name"]
    key = "%s/%s" % (name, mode)
    excerpts = ("function/%s" % key.replace("/", "-"), [("sim.txt", verdict_lines), ("candidate.json", record)])
    paths, digests = qorlib.excerpt_record(*excerpts)
    row = {
        "key": key, "kernel": kernel, "candidate": name, "source_kind": rec["source_kind"], "mode": mode,
        "ops_requested": ops, "golden_signature": rec["golden_signature"], **got,
        "pass": got["verdict"] == "complete" and got["ops"] == ops and got["signature"] == rec["golden_signature"],
        "simulator": qorlib.tool_version(["iverilog", "-V"]), "rtl_files": rtl,
        "provenance": HEADER["provenance"], "recorded_at": qorlib.now(),
        "reproduce": "cargo build -p logicaffeine-compile --example emit_qor_harness && "
                     "python3 scripts/qor/run-qor-function.py %s %s --mode %s --ops %d" % (kernel, candidate, mode, ops),
        "excerpts": paths, "excerpt_sha3": digests, "_excerpts": excerpts,
    }
    return row


def record(row):
    excerpts = row.pop("_excerpts")
    qorlib.update("function", HEADER, row, excerpts)
    print("%-40s %-6s %s %s steady %s cycles/op" % (row["candidate"], row["mode"], "PASS" if row["pass"] else "FAIL",
                                                    row["signature"], row["steady_cycles_per_op"]), flush=True)
    return row["pass"]


def verify():
    doc = qorlib.load("function", HEADER)
    bad = ["%s: no row names this excerpt" % d for d in qorlib.orphan_excerpts("function", doc["rows"])]
    for row in doc["rows"]:
        texts, problems = qorlib.read_excerpts(row)
        bad += problems
        if "sim.txt" in texts:
            got = parse(texts["sim.txt"])
            for f in ("verdict", "ops", "signature", "cycles", "first_result_cycle", "steady_cycles_per_op"):
                if got[f] != row[f]:
                    bad.append("%s: %s is %r in the excerpt, %r in the row" % (row["key"], f, got[f], row[f]))
        if "candidate.json" in texts:
            rec = json.loads(texts["candidate.json"])
            if rec["golden_signature"] != row["golden_signature"] or rec["candidate"]["name"] != row["candidate"]:
                bad.append("%s: the candidate record disagrees with the row" % row["key"])
        want = row["verdict"] == "complete" and row["ops"] == row["ops_requested"] and row["signature"] == row["golden_signature"]
        if row["pass"] != want:
            bad.append("%s: pass is %r but the evidence says %r" % (row["key"], row["pass"], want))
    if bad:
        for b in bad:
            print("FAIL: " + b, file=sys.stderr)
        sys.exit(1)
    print("verified %d rows" % len(doc["rows"]))


def every_candidate():
    pairs = list(qorlib.logos_candidates())
    manifest = os.path.join(qorlib.SIL, "qor", "candidates.json")
    if os.path.exists(manifest):
        pairs += [(e["candidate"]["kernel"], e["candidate"]["name"]) for e in json.load(open(manifest))["candidates"]]
    return pairs


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("kernel", nargs="?")
    ap.add_argument("candidate", nargs="?")
    ap.add_argument("--mode", choices=["shaped", "edge"], default="shaped")
    ap.add_argument("--ops", type=int, default=1000)
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--jobs", type=int, default=1, help="simulations at once (the ledger is written by this process alone)")
    a = ap.parse_args()
    if a.kernel == "verify":
        return verify()
    if a.all:
        runs = [(k, c, m, a.ops) for k, c in every_candidate() for m in ("shaped", "edge")]
        with concurrent.futures.ProcessPoolExecutor(max_workers=a.jobs) as pool:
            ok = all([record(f.result()) for f in [pool.submit(gate, *r) for r in runs]])
        sys.exit(0 if ok else 1)
    if not (a.kernel and a.candidate):
        ap.error("name a kernel and a candidate, or --all, or verify")
    sys.exit(0 if record(gate(a.kernel, a.candidate, a.mode, a.ops)) else 1)


if __name__ == "__main__":
    main()
