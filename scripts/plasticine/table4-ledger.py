#!/usr/bin/env python3
"""The Spatial PLDI'18 Table 4 credibility gate's ledger — the only writer of
benchmarks/silicon/plasticine_table4_gate.json.

    table4-ledger.py add --app Table4_GEMM_i2_j2_kk1 --gen-dir <spatial/gen/PIR_pirTest/<app>> --toolchain "<branches>"
    table4-ledger.py add --app Table4_GEMM_i2_j2_kk1 --gen-dir <spatial/gen/Tst_pirTest/<app>> --toolchain "<branches>"
    table4-ledger.py record PIR|Tst --toolchain "<branches>" [--spatial <toolchain>/spatial] [--app <app>]...
    table4-ledger.py closest
    table4-ledger.py verify

`record` walks the generated directory of every point of the pre-registered search (or the apps named): a point
the chain mapped (or simulated) becomes a row, one it ran and did not becomes a refusal row with the failing pass's
own error lines and log tail, and one never run (no generated directory) is reported and left as it is — so a
point of the search is missing only until it runs, and never refused for not having run. A row is one point of the pre-registered search (scripts/plasticine/table4/Table4.scala) run through the public
Plasticine chain on a 16 x 8 array: PIR's mapping (a PIR_pirTest run), and for a simulated point (a Tst_pirTest
run) Tungsten's cycles on the routed hybrid network twice — on the DDR4 memory every generated app carries and on
the paper's own (one DDR3-1600 channel, scripts/plasticine/table4/DDR3-1600-1ch.cfg). Each row sits beside its
Table 4 row (benchmarks/silicon/plasticine_table4.json): utilization in points, time as a ratio, inside or
outside the tolerances this file states before any run. Every number is parsed from the chain's own outputs;
the parsed excerpts are committed under benchmarks/silicon/plasticine_logs/table4/ and `verify` re-parses them.
"""
import argparse
import importlib.util
import json
import os
import re
import sys
from datetime import datetime, timezone

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
LEDGER = os.path.join(ROOT, "benchmarks", "silicon", "plasticine_table4_gate.json")
TABLE4 = os.path.join(ROOT, "benchmarks", "silicon", "plasticine_table4.json")
LOGS = os.path.join(ROOT, "benchmarks", "silicon", "plasticine_logs", "table4")
_spec = importlib.util.spec_from_file_location("gate", os.path.join(ROOT, "scripts", "accel", "plasticine-gate-ledger.py"))
gate = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gate)

HEADER = {
    "_generated_by": "scripts/plasticine/table4-ledger.py (never hand-edited; verify re-parses the committed excerpts)",
    "provenance": "simulated_tungsten",
    "machine": "PIR's 16 x 8 array (64 PCUs, 64 PMUs), the size Table 4 states; PIR's unit shapes",
    "tolerance": {"utilization_points": 5.0, "time_ratio": 0.15,
                  "stated": "before any run, the runbook's gate: within 5 points of each utilization and 15% of the time"},
    "honesty": "Rows are the public Plasticine toolchain's own outputs on Spatial's own apps at Table 2's data sizes, "
               "a simulation of the paper's architecture by its authors' toolchain, not silicon. Table 4 does not "
               "publish the unrolling it used; the search over it is pre-registered in "
               "scripts/plasticine/table4/Table4.scala. Time is cycles at Table 4's 1 GHz clock. Table 4's AG is "
               "read as PIR's DAG (its DRAM address generators).",
}
RE_POINT = re.compile(r"^Table4_(?P<family>[A-Za-z0-9]+)_(?P<point>.+)$")


def table4_row(family):
    for r in json.load(open(TABLE4))["rows"]:
        if r["abbrev"] == family or r["app"] == family:
            return r
    gate.refuse("no Table 4 row for %s" % family)


# Tungsten's runs of a simulated point: the routed hybrid network and the ideal one, each on the generated DDR4
# memory and on the paper's (`sim_<memory>` for the routed network, `sim_<memory>_ideal` for the ideal one).
SIMS = (("sim_ddr4", "runhybrid.log"), ("sim_ddr3", "runhybrid_ddr3.log"),
        ("sim_ddr4_ideal", "runp2p.log"), ("sim_ddr3_ideal", "runp2p_ddr3.log"))
RE_TUNGSTEN_STOP = re.compile(r"Caught exception on cycle (\d+): (.+)")


def simulated(run):
    """A run that finished and passed its own check."""
    return bool(run) and run.get("pass") is True and run.get("cycles") is not None


def time_ratios(row, ref):
    """Each finished simulation's time (cycles at Table 4's clock) over Table 4's, inside the tolerance or not."""
    out = {}
    for key, _ in SIMS:
        run = row.get(key)
        if simulated(run):
            ratio = (run["cycles"] / (ref_clock_hz() / 1e3)) / ref["time_ms"]
            suffix = key[len("sim_"):]
            out["time_ratio_" + suffix] = round(ratio, 4)
            out["time_within_" + suffix] = abs(ratio - 1.0) <= HEADER["tolerance"]["time_ratio"]
    return out


def ref_clock_hz():
    return json.load(open(TABLE4))["machine"]["clock_ghz"] * 1e9


def compare(row, ref):
    """The row beside Table 4: utilization in points, time as a ratio, inside the tolerances or not."""
    tol = HEADER["tolerance"]
    res = row["resources"]
    out = {"pcu_points": round(res["PCU"]["usage_pct"] - ref["pcu_pct"], 2) if "PCU" in res else None,
           "pmu_points": round(res["PMU"]["usage_pct"] - ref["pmu_pct"], 2) if "PMU" in res else None,
           "ag_points": round(res["DAG"]["usage_pct"] - ref["ag_pct"], 2) if "DAG" in res else None}
    util = [v for v in out.values() if v is not None]
    out["utilization_within"] = bool(util) and all(abs(v) <= tol["utilization_points"] for v in util)
    out.update(time_ratios(row, ref))
    return out


def simulations(gen):
    """Every Tungsten run present under a point's generated directory: `(runs by key, excerpts by log name)`."""
    runs, excerpts = {}, {}
    for key, log in SIMS:
        path = os.path.join(gen, "log", log)
        if os.path.exists(path):
            text = gate.read(path)
            excerpts[log] = gate.tail(text)
            runs[key] = gate.parse_run(text)
    return runs, excerpts


def write_excerpts(row, excerpts):
    dest = os.path.join(LOGS, row["key"].replace("/", "-"))
    os.makedirs(dest, exist_ok=True)
    row["excerpts"] = {}
    for name, text in excerpts.items():
        with open(os.path.join(dest, name), "w") as f:
            f.write(text)
        row["excerpts"][name] = os.path.relpath(os.path.join(dest, name), ROOT)


def cmd_add(a):
    m = RE_POINT.match(a.app)
    if not m:
        gate.refuse("%s is not a Table4_<family>_<point> app" % a.app)
    gen = a.gen_dir.rstrip("/")
    backend = os.path.basename(os.path.dirname(gen)).split("_")[0]
    if backend not in ("PIR", "Tst"):
        gate.refuse("%s: the gen dir is neither PIR_pirTest nor Tst_pirTest" % gen)
    res_path = os.path.join(gen, "pir", "out", "resource.csv")
    if not os.path.exists(res_path):
        gate.refuse("%s: PIR wrote no resource.csv — the mapping failed" % gen)
    mesh = gate.RE_MESH.search(gate.read(os.path.join(gen, "log", "gentst.sh"))) if os.path.exists(os.path.join(gen, "log", "gentst.sh")) else None
    if not mesh or (mesh.group(1), mesh.group(2)) != ("16", "8"):
        gate.refuse("%s: not mapped onto the 16 x 8 array" % gen)
    excerpts = {"resource.csv": gate.read(res_path)}
    row = {"key": "%s/%s" % (a.app, backend), "app": a.app, "family": m.group("family"), "point": m.group("point"),
           "backend": backend, "toolchain": a.toolchain, "resources": gate.parse_resource(res_path),
           "recorded_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
           "reproduce": "scripts/plasticine/run-in-container.sh bash /table4.sh %s %s" % (backend, a.app)}
    if backend == "Tst":
        runs, logs = simulations(gen)
        row.update(runs)
        excerpts.update(logs)
    row["table4"] = compare(row, table4_row(m.group("family")))
    write_excerpts(row, excerpts)
    save_row(row)
    print("%-34s %-4s PCU %s%% PMU %s%% DAG %s%%  %s" % (a.app, backend, row["resources"].get("PCU", {}).get("usage_pct"),
                                                       row["resources"].get("PMU", {}).get("usage_pct"),
                                                       row["resources"].get("DAG", {}).get("usage_pct"), row["table4"]))


SEARCH = os.path.join(ROOT, "scripts", "plasticine", "table4", "Table4.scala")
TOOLCHAIN_DIR = os.path.join(ROOT, "target", "plasticine-toolchain", "spatial")


def search_points():
    """Every app of the pre-registered search, in its order."""
    return re.findall(r"^class (Table4_[A-Za-z0-9_]+) extends", gate.read(SEARCH), re.M)


RE_PIR_ERROR = re.compile(r"^\[error\] (?!\s*at |Compilation failed|java\.|\(Compile|Total time|Nonzero)(.+)$", re.M)


def refusal(app, backend, gen, toolchain):
    """A point the chain ran and produced nothing for. The reason is the failing stage's own words: where the
    routed simulation stopped, Tungsten's exception; otherwise the failing pass's PIR `[error]` messages (its
    stack frames and sbt's wrapper lines left out); where the simulation finished but the app's own check failed, that.
    A simulated point keeps every Tungsten run that reached a cycle count — the ideal network's when the routed one
    stopped, an unchecked run's with its PASS: false — beside the refusal; only a run that passed is compared."""
    logs = sorted((os.path.join(gen, "log", f) for f in os.listdir(os.path.join(gen, "log"))
                   if not f[0].isdigit() and f.endswith(".log")), key=os.path.getmtime) if os.path.isdir(os.path.join(gen, "log")) else []
    last = logs[-1] if logs else None
    text = gate.strip_ansi(gate.read(last)) if last else ""
    hybrid = os.path.join(gen, "log", "runhybrid.log")
    stop = RE_TUNGSTEN_STOP.search(gate.strip_ansi(gate.read(hybrid))) if backend == "Tst" and os.path.exists(hybrid) else None
    # PIR names a unit kind by its JVM object (`PCUParam559688568`); the hash differs every run, the kind does not.
    errors = [re.sub(r"(Param)\d+", r"\1", m.strip()) for m in RE_PIR_ERROR.findall(text)]
    what = "resource.csv" if backend == "PIR" else "simulation"
    checked = gate.parse_run(gate.read(hybrid)) if backend == "Tst" and os.path.exists(hybrid) else {}
    if stop:
        reason = "runhybrid.log: the routed network stopped on cycle %s: %s" % (stop.group(1), stop.group(2).strip())
    elif checked.get("cycles") is not None and checked.get("pass") is False:
        reason = "runhybrid.log: the simulation finished on cycle %d and the app's own check failed (PASS: false)" % checked["cycles"]
    elif errors:
        reason = "%s produced no %s: %s" % (os.path.basename(last), what, "; ".join(errors))
    else:
        reason = "%s produced no %s" % (os.path.basename(last) if last else "the chain", what)
    row = {"key": "%s/%s" % (app, backend), "app": app, "family": RE_POINT.match(app).group("family"),
           "point": RE_POINT.match(app).group("point"), "backend": backend, "toolchain": toolchain,
           "refused": reason, "refusal_tail": gate.tail(text, 12) if last else "",
           "recorded_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
           "reproduce": "scripts/plasticine/run-in-container.sh bash /table4.sh %s %s" % (backend, app)}
    if backend == "Tst":
        runs, excerpts = simulations(gen)
        if any(r.get("cycles") is not None for r in runs.values()):
            row.update(runs)
            row["table4"] = time_ratios(row, table4_row(row["family"]))
            write_excerpts(row, excerpts)
    return row


def save_row(row):
    doc = json.load(open(LEDGER)) if os.path.exists(LEDGER) else dict(HEADER, rows=[])
    doc["rows"] = sorted([r for r in doc["rows"] if r["key"] != row["key"]] + [row], key=lambda r: r["key"])
    with open(LEDGER, "w") as f:
        json.dump(doc, f, indent=2, sort_keys=True)
        f.write("\n")


def cmd_record(a):
    apps = a.apps or search_points()
    for app in apps:
        gen = os.path.join(a.spatial, "gen", "%s_pirTest" % a.backend, app)
        if not os.path.isdir(gen):
            # Never run is not refused: the point keeps whatever row it has, and a point with none stays missing.
            print("%-34s %-4s NOT RUN (no %s)" % (app, a.backend, os.path.relpath(gen, a.spatial)))
            continue
        ok = os.path.exists(os.path.join(gen, "pir", "out", "resource.csv"))
        if a.backend == "Tst":
            ok = ok and simulated(simulations(gen)[0].get("sim_ddr4"))
        if ok:
            cmd_add(argparse.Namespace(app=app, gen_dir=gen, toolchain=a.toolchain))
        else:
            save_row(refusal(app, a.backend, gen, a.toolchain))
            print("%-34s %-4s REFUSED" % (app, a.backend))


def closest():
    """Per family, the mapped PIR point nearest Table 4's utilization (the sum of the three gaps in points;
    the first in key order on a tie) — the point the gate simulates."""
    best = {}
    for r in json.load(open(LEDGER))["rows"]:
        if r["backend"] != "PIR" or "refused" in r:
            continue
        gap = sum(abs(r["table4"][k]) for k in ("pcu_points", "pmu_points", "ag_points"))
        if r["family"] not in best or gap < best[r["family"]][0]:
            best[r["family"]] = (gap, r["app"])
    return [app for _, app in best.values()]


def cmd_closest(_a):
    print(" ".join(closest()))


def cmd_verify(_a):
    doc = json.load(open(LEDGER))
    bad = []
    for r in doc["rows"]:
        if "excerpts" not in r:
            continue
        texts = {n: gate.read(os.path.join(ROOT, p)) for n, p in r["excerpts"].items()}
        if "resource.csv" in texts:
            tmp = os.path.join(LOGS, ".verify.csv")
            with open(tmp, "w") as f:
                f.write(texts["resource.csv"])
            if gate.parse_resource(tmp) != r["resources"]:
                bad.append("%s: resources disagree with resource.csv" % r["key"])
            os.remove(tmp)
        for key, log in SIMS:
            if log in texts and gate.parse_run(texts[log]) != r.get(key):
                bad.append("%s: %s disagrees with its excerpt %s" % (r["key"], key, log))
        ref = table4_row(r["family"])
        if (time_ratios(r, ref) if "refused" in r else compare(r, ref)) != r["table4"]:
            bad.append("%s: the Table 4 comparison is stale" % r["key"])
    if bad:
        for b in bad:
            print("FAIL: " + b, file=sys.stderr)
        sys.exit(1)
    print("verified %d rows" % len(doc["rows"]))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    add = sub.add_parser("add")
    add.add_argument("--app", required=True)
    add.add_argument("--gen-dir", required=True)
    add.add_argument("--toolchain", required=True)
    rec = sub.add_parser("record")
    rec.add_argument("backend", choices=["PIR", "Tst"])
    rec.add_argument("--toolchain", required=True)
    rec.add_argument("--spatial", default=TOOLCHAIN_DIR)
    rec.add_argument("--app", dest="apps", action="append", default=[], help="only these apps (repeatable)")
    sub.add_parser("closest", help="the point of each family the gate simulates")
    sub.add_parser("verify")
    a = ap.parse_args()
    {"add": cmd_add, "record": cmd_record, "closest": cmd_closest, "verify": cmd_verify}[a.cmd](a)


if __name__ == "__main__":
    main()
