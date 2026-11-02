#!/usr/bin/env python3
"""Find the first operation a candidate of the comparative-synthesis bench gets wrong.

    trace-qor-candidate.py <kernel> <candidate|logos> [--mode shaped|edge] [--ops N]

Simulates the candidate with the tracing testbench (every folded result printed) and compares each result
with the golden; prints the first operation that differs, its operands, the golden result and the candidate's.
A diagnostic only: it records nothing.
"""
import argparse
import os
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import qorlib  # noqa: E402


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("kernel")
    ap.add_argument("candidate")
    ap.add_argument("--mode", choices=["shaped", "edge"], default="shaped")
    ap.add_argument("--ops", type=int, default=64)
    a = ap.parse_args()
    with tempfile.TemporaryDirectory(dir=os.path.join(qorlib.ROOT, "logs")) as run_dir:
        r = subprocess.run([qorlib.EMIT, run_dir, a.kernel, a.candidate, a.mode, str(a.ops), "--trace"],
                           capture_output=True, text=True)
        if r.returncode != 0:
            sys.exit(r.stderr)
        import json
        rec = json.load(open(os.path.join(run_dir, "candidate.json")))
        files = [os.path.join(run_dir, f) for f in rec["files"] + [rec["harness"], "tb_qor_trace.v"] if f.endswith((".v", ".sv"))]
        b = subprocess.run(["iverilog", "-g2012", "-s", "tb_qor", "-o", os.path.join(run_dir, "t.vvp")] + files,
                           capture_output=True, text=True)
        if b.returncode != 0:
            sys.exit(b.stderr)
        sim = subprocess.run(["vvp", "-n", os.path.join(run_dir, "t.vvp")], cwd=run_dir, capture_output=True, text=True).stdout
        golden = open(os.path.join(run_dir, "golden.txt")).read().splitlines()
        operands = open(os.path.join(run_dir, "operands.txt")).read().splitlines()
    got = [l[len("QOR r "):] for l in sim.splitlines() if l.startswith("QOR r ")]
    for line in sim.splitlines():
        if line.startswith("QOR ") and not line.startswith("QOR r "):
            print(line)
    for i, (g, w) in enumerate(zip(got, golden)):
        gw = g.split()[1:]
        ww = w.split()[1:]
        if [x.lower().lstrip("0") for x in gw] != [x.lower().lstrip("0") for x in ww]:
            print("first difference at operation %d" % i)
            print("  operands %s" % operands[i].split(" ", 1)[1])
            print("  golden   %s" % " ".join(ww))
            print("  got      %s" % " ".join(gw))
            return
    print("all %d traced results equal the golden" % min(len(got), len(golden)))


if __name__ == "__main__":
    main()
