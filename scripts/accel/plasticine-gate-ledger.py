#!/usr/bin/env python3
"""The Plasticine TOOLCHAIN ledger — the only writer of benchmarks/silicon/plasticine_toolchain.json.

A row is one run of the public Plasticine chain (Spatial → PIR → plastiroute → Tungsten, built by
scripts/plasticine/) on one app: PIR's resource mapping, plastiroute's routing score, and Tungsten's
cycle-accurate simulation (ideal and routed hybrid network). Every number is parsed from the chain's own
outputs, never typed; the parsed excerpts are committed under benchmarks/silicon/plasticine_logs/<app>/ and
`verify` re-parses them. Provenance is `simulated_tungsten` — a simulation of the paper-parameterized
architecture by its authors' toolchain, not silicon. The lock plasticine_sim_lock compares each row to our
MODELED row for the same shape (codegen_sva::plasticine_report::gemm_row).

    plasticine-gate-ledger.py add --app GEMM_0 --gen-dir <spatial/gen/Tst_pirTest/GEMM_0> \
        --shape "128x128x128" --params "dim=128 ts=64 its=64 ip=16" --toolchain "<branches>"
    plasticine-gate-ledger.py add --kind dot --app DotProduct_3 --gen-dir <…> --params "N=65536 ts=16384 ip=16 op=2" \
        --toolchain "<branches>"
    plasticine-gate-ledger.py verify
    plasticine-gate-ledger.py drop <key>...
A row is a run whose ideal-network simulation finished clean (a cycle count, no deadlock, no exception, its gold check
passed); what the routed (hybrid) network did with it is recorded as it happened, and a routed run that did not finish
clean carries `why` (deadlocked at cycle N, threw on cycle N, printed no completion, failed its gold check). A run that
did not finish clean on the ideal network is refused.

scripts/plasticine/twins.py runs every DotProduct / OuterProduct variant and its LOGOS twin and records both.
"""
import argparse, csv, hashlib, json, os, re, shutil, sys
from datetime import datetime, timezone

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
LEDGER = os.path.join(ROOT, "benchmarks", "silicon", "plasticine_toolchain.json")
LOGS = os.path.join(ROOT, "benchmarks", "silicon", "plasticine_logs")
PROVENANCE = "simulated_tungsten"
EXCERPT_TAIL = 40

RE_CYCLE = re.compile(r"Simulation complete at cycle (\d+)")
RE_DEADLOCK = re.compile(r"Simulation complete at cycle (\d+) \(DEADLOCK\)")
RE_EXCEPTION = re.compile(r"Caught exception on cycle (\d+): (.*)")
RE_PASS = re.compile(r"PASS: (true|false)")
RE_RUNTIME = re.compile(r"Runtime:(\d+):(\d+(?:\.\d+)?)")
RE_READ_BW = re.compile(r"Average DRAM Read Bandwidth: ([\d.]+) GB/s")
RE_WRITE_BW = re.compile(r"Average DRAM Write Bandwidth: ([\d.]+) GB/s")
RE_SCORE = re.compile(r"Best score is ([\d.]+)")
RE_VCS = re.compile(r"Used (\d+) VCs\.")
RE_MESH = re.compile(r"--row=(\d+) --col=(\d+)")


def read(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read()


def strip_ansi(s):
    return re.sub(r"\x1b\[[0-9;]*m", "", s)


def tail(s, n=EXCERPT_TAIL):
    """The last `n` lines that say something (the simulator's matrix dumps — lines of nothing but numbers —
    are dropped so the excerpt keeps the cycle count, bandwidth and PASS lines)."""
    lines = [l for l in strip_ansi(s).splitlines() if re.search(r"[A-Za-z]", l)]
    return "\n".join(lines[-n:]) + "\n"


def parse_run(log):
    cycles = [int(m) for m in RE_CYCLE.findall(log)]
    passed = RE_PASS.search(log)
    rt = RE_RUNTIME.search(log)
    return {
        "cycles": cycles[-1] if cycles else None,
        "pass": (passed.group(1) == "true") if passed else None,
        "wall_s": (int(rt.group(1)) * 60 + float(rt.group(2))) if rt else None,
        "dram_read_gbps": float(RE_READ_BW.search(log).group(1)) if RE_READ_BW.search(log) else None,
        "dram_write_gbps": float(RE_WRITE_BW.search(log).group(1)) if RE_WRITE_BW.search(log) else None,
    }


def deadlocked_at(log):
    """The cycle at which the simulator gave up on a deadlock, when it says so: the app never finished, so that cycle
    is not its run time, whatever the gold check printed after it. Tungsten prints the verdict in colour."""
    stuck = RE_DEADLOCK.findall(strip_ansi(log))
    return int(stuck[-1]) if stuck else None


def outcome(log):
    """What a simulation came to, when it did not finish clean: deadlocked on a cycle, threw on one, printed no
    completion, or failed its gold check; None for a clean finish. Tungsten prints its verdicts in colour."""
    text = strip_ansi(log)
    stuck = deadlocked_at(text)
    if stuck is not None:
        return "deadlocked at cycle %d" % stuck
    threw = RE_EXCEPTION.findall(text)
    if threw:
        return "threw on cycle %s: %s" % (threw[-1][0], threw[-1][1].strip())
    run = parse_run(text)
    if run["cycles"] is None:
        return "printed no completion"
    if run["pass"] is not True:
        return "failed its gold check"
    return None


def parse_resource(path):
    out = {}
    with open(path) as f:
        for r in csv.DictReader(f):
            out[r["Type"]] = {"used": int(r["Used"]), "total": int(r["Total"]), "usage_pct": float(r["Usage"])}
    return out


def parse_proute(log):
    s = RE_SCORE.search(log)
    v = RE_VCS.findall(log)
    return {"best_score": float(s.group(1)) if s else None, "vcs": int(v[-1]) if v else None}


# The tensor kinds a twin compares: each one's knobs (Spatial's parameter names), its shape as a GEMM's (a dot
# product or TPC-H Q6's guarded sum is a 1 x 1 x N reduction, an outer product the M x N x 1, GDA's covariance the
# C x C x R), and Spatial's app.
TENSOR_KINDS = {
    "dot": (["N", "ts", "ip", "op"], lambda p: {"m": 1, "n": 1, "k": p["N"]}, "DotProduct"),
    "tpchq6": (["N", "ts", "ip", "op"], lambda p: {"m": 1, "n": 1, "k": p["N"]}, "TPCHQ6"),
    "outer": (["M", "N", "ts1", "ts2", "ip", "ip1", "ip2", "op1", "op2"],
              lambda p: {"m": p["M"], "n": p["N"], "k": 1}, "OuterProduct"),
    "gda": (["C", "R", "ts", "op", "mp1", "mp2", "ip"], lambda p: {"m": p["C"], "n": p["C"], "k": p["R"]}, "GDA"),
    "blackscholes": (["N", "ts", "op", "ip"], lambda p: {"m": 1, "n": p["N"], "k": 1}, "BlackScholes"),
    "kmeans": (["K", "D", "ts", "ip", "op", "mp1", "mp2", "mp3", "I", "N"], lambda p: {"m": p["K"], "n": p["D"], "k": p["N"]},
               "Kmeans"),
}


def parse_shape(shape):
    m, n, k = (int(x) for x in shape.lower().split("x"))
    return {"m": m, "n": n, "k": k}


def parse_params(params):
    out = {}
    for kv in params.split():
        k, v = kv.split("=")
        out[k] = int(v)
    return out


def load():
    if os.path.exists(LEDGER):
        with open(LEDGER) as f:
            return json.load(f)
    return {"_generated_by": "scripts/accel/plasticine-gate-ledger.py (never hand-edited; verify re-parses the committed excerpts)",
            "provenance": PROVENANCE,
            "honesty": "Rows are the public Plasticine toolchain's own outputs (PIR mapping, plastiroute routing, Tungsten cycle-accurate simulation) on the apps named — a simulation of the paper's architecture, not silicon; the modeled counterpart of each row is recomputed by plasticine_sim_lock.",
            "rows": []}


def save(doc):
    doc["rows"].sort(key=lambda r: r["key"])
    with open(LEDGER, "w") as f:
        json.dump(doc, f, indent=2, sort_keys=True)
        f.write("\n")


def refuse(msg):
    print("REFUSING: " + msg, file=sys.stderr)
    sys.exit(3)


def cmd_add(a):
    gen = a.gen_dir
    logs = os.path.join(gen, "log")
    need = {n: os.path.join(logs, n) for n in ("gentst.sh", "runp2p.log", "runproute.log", "runhybrid.log")}
    for n, p in need.items():
        if not os.path.exists(p):
            refuse("%s missing under %s — the chain did not reach that pass" % (n, logs))
    resource_csv = os.path.join(gen, "pir", "out", "resource.csv")
    if not os.path.exists(resource_csv):
        refuse("pir/out/resource.csv missing — PIR did not map the app")
    gentst = read(need["gentst.sh"])
    mesh = RE_MESH.search(gentst)
    if not mesh:
        refuse("gentst.sh carries no --row/--col: the mesh PIR mapped onto is unknown")
    ideal_why = outcome(read(need["runp2p.log"]))
    if ideal_why is not None:
        refuse("the ideal-network simulation %s: the app never ran clean" % ideal_why)
    p2p = parse_run(read(need["runp2p.log"]))
    hyb = parse_run(read(need["runhybrid.log"]))
    routed_why = outcome(read(need["runhybrid.log"]))
    if routed_why is not None:
        hyb["why"] = routed_why
    proute = parse_proute(read(need["runproute.log"]))
    rows, cols = int(mesh.group(1)), int(mesh.group(2))
    key = "%s/%dx%d" % (a.app, rows, cols)
    if a.kind == "gemm":
        if not (a.shape and a.params):
            refuse("a gemm row needs --shape and --params")
        described = {"shape": parse_shape(a.shape), "params": parse_params(a.params),
                     "app_source": a.app_source or "spatial/pir/regression/GEMM.scala (Spatial's own regression app, unmodified)"}
    elif a.kind == "spatial_app":
        if not (a.app_source and a.params is not None):
            refuse("a spatial_app row needs --app-source (the app's regression file) and --params (its knobs)")
        described = {"params": parse_params(a.params), "app_source": a.app_source}
    elif a.kind in TENSOR_KINDS:
        knobs, shape_of, app = TENSOR_KINDS[a.kind]
        params = parse_params(a.params or "")
        if sorted(params) != sorted(knobs):
            refuse("a %s row's --params name exactly %s" % (a.kind, " ".join(knobs)))
        described = {"shape": shape_of(params), "params": params,
                     "app_source": a.app_source or "spatial/pir/regression/%s.scala (Spatial's own regression app, unmodified)" % app}
        if a.logos_source:
            described["logos_source"] = read(a.logos_source)
    else:
        if not (a.logos_source and a.fn_name and a.params is not None):
            refuse("a counted_loop row needs --logos-source, --fn and --params (the runtime args)")
        described = {"logos_source": read(a.logos_source), "fn_name": a.fn_name,
                     "params": [int(x) for x in a.params.split()],
                     "app_source": a.app_source or "emitted by codegen_sva::spatial_emit::emit_spatial_counted_loop_test (largo emit plasticine / emit_plasticine_spatial_tests)"}
    row = {
        "key": key,
        "app": a.app,
        "kind": a.kind,
        "mesh": {"rows": rows, "cols": cols, "pcus": rows * cols // 2, "pmus": rows * cols - rows * cols // 2},
        **described,
        "pir": parse_resource(resource_csv),
        "plastiroute": proute,
        "ideal_network": p2p,
        "hybrid_network": hyb,
        "provenance": PROVENANCE,
        "toolchain": a.toolchain,
        "reproduce": a.reproduce,
        "recorded_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "excerpts": {},
    }
    dest = os.path.join(LOGS, a.app)
    os.makedirs(dest, exist_ok=True)
    shutil.copy(resource_csv, os.path.join(dest, "resource.csv"))
    row["excerpts"]["resource.csv"] = os.path.relpath(os.path.join(dest, "resource.csv"), ROOT)
    for n in ("gentst.sh", "runp2p.log", "runproute.log", "runhybrid.log"):
        text = read(need[n]) if n == "gentst.sh" else tail(read(need[n]))
        with open(os.path.join(dest, n), "w") as f:
            f.write(text)
        row["excerpts"][n] = os.path.relpath(os.path.join(dest, n), ROOT)
    row["excerpt_sha256"] = {n: hashlib.sha256(read(os.path.join(ROOT, p)).encode()).hexdigest()[:16] for n, p in row["excerpts"].items()}
    doc = load()
    doc["rows"] = [r for r in doc["rows"] if r["key"] != key] + [row]
    save(doc)
    print("recorded %s: PIR PCU %d/%d PMU %d/%d, ideal %d cycles, hybrid %s, proute score %s" % (
        key, row["pir"]["PCU"]["used"], row["pir"]["PCU"]["total"], row["pir"]["PMU"]["used"], row["pir"]["PMU"]["total"],
        p2p["cycles"], hyb.get("why") or "%d cycles" % hyb["cycles"], proute["best_score"]))


def cmd_verify(_a):
    doc = load()
    bad = []
    for r in doc["rows"]:
        ex = {n: os.path.join(ROOT, p) for n, p in r["excerpts"].items()}
        for n, p in ex.items():
            if not os.path.exists(p):
                bad.append("%s: excerpt %s missing" % (r["key"], n))
                continue
            h = hashlib.sha256(read(p).encode()).hexdigest()[:16]
            if h != r["excerpt_sha256"].get(n):
                bad.append("%s: excerpt %s was edited (sha %s != %s)" % (r["key"], n, h, r["excerpt_sha256"].get(n)))
        if all(os.path.exists(p) for p in ex.values()):
            if parse_resource(ex["resource.csv"]) != r["pir"]:
                bad.append("%s: resource.csv disagrees with the row" % r["key"])
            if outcome(read(ex["runp2p.log"])) is not None:
                bad.append("%s: runp2p.log shows %s, which the ledger never takes" % (r["key"], outcome(read(ex["runp2p.log"]))))
            for which, name in (("ideal_network", "runp2p.log"), ("hybrid_network", "runhybrid.log")):
                got = parse_run(read(ex[name]))
                why = outcome(read(ex[name]))
                if (got["cycles"], got["pass"], why) != (r[which]["cycles"], r[which]["pass"], r[which].get("why")):
                    bad.append("%s: %s disagrees with the row (%s, %s vs %s)" % (r["key"], name, got, why, r[which]))
            if parse_proute(read(ex["runproute.log"])) != r["plastiroute"]:
                bad.append("%s: runproute.log disagrees with the row" % r["key"])
            m = RE_MESH.search(read(ex["gentst.sh"]))
            if not m or (int(m.group(1)), int(m.group(2))) != (r["mesh"]["rows"], r["mesh"]["cols"]):
                bad.append("%s: gentst.sh mesh disagrees with the row" % r["key"])
        if r["provenance"] != PROVENANCE:
            bad.append("%s: provenance must be %s" % (r["key"], PROVENANCE))
    if bad:
        for b in bad:
            print("FAIL: " + b, file=sys.stderr)
        sys.exit(1)
    print("verified %d rows" % len(doc["rows"]))


def cmd_drop(a):
    """Remove each named row and its committed excerpts: a row the ledger should never have taken."""
    doc = load()
    keys = {r["key"]: r for r in doc["rows"]}
    for k in a.keys:
        if k not in keys:
            refuse("no row %s in the ledger" % k)
    for k in a.keys:
        dirs = {os.path.dirname(os.path.join(ROOT, p)) for p in keys[k]["excerpts"].values()}
        for d in dirs:
            if os.path.isdir(d):
                shutil.rmtree(d)
        print("dropped %s" % k)
    doc["rows"] = [r for r in doc["rows"] if r["key"] not in a.keys]
    save(doc)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    add = sub.add_parser("add")
    add.add_argument("--app", required=True)
    add.add_argument("--kind", choices=["gemm", "counted_loop", "spatial_app"] + sorted(TENSOR_KINDS), default="gemm",
                     help="gemm: a GEMM (Spatial's regression app or our twin); dot / tpchq6 / outer: a dot product, "
                          "TPC-H Q6 or an outer product (Spatial's DotProduct / TPCHQ6 / OuterProduct or our twin, "
                          "the LOGOS source given); spatial_app: one of Spatial's own apps whose twin the compiler "
                          "does not print yet; counted_loop: one of OUR emitted loops")
    add.add_argument("--app-source", default=None)
    add.add_argument("--gen-dir", required=True)
    add.add_argument("--shape", default=None, help="gemm: MxNxK")
    add.add_argument("--params", default=None, help="gemm: 'dim=128 ts=64 its=64 ip=16'; dot: 'N=65536 ts=16384 ip=16 op=1'; "
                                                      "outer: 'M=256 N=256 ts1=64 ts2=32 ip=16 ip1=16 ip2=1 op1=1 op2=1'; "
                                                      "counted_loop: the runtime args, e.g. '64'")
    add.add_argument("--logos-source", default=None, help="counted_loop, and a dot / outer twin: the .lg file the app was emitted from")
    add.add_argument("--fn", dest="fn_name", default=None, help="counted_loop: the LOGOS function")
    add.add_argument("--toolchain", required=True)
    add.add_argument("--reproduce", default="scripts/plasticine/fetch.sh && scripts/plasticine/run-in-container.sh && scripts/plasticine/run-in-container.sh bash /gate.sh <app>")
    add.set_defaults(fn=cmd_add)
    ver = sub.add_parser("verify")
    ver.set_defaults(fn=cmd_verify)
    drop = sub.add_parser("drop", help="remove rows the ledger should never have taken, with their excerpts")
    drop.add_argument("keys", nargs="+", help="row keys, e.g. OuterProduct_11/20x20")
    drop.set_defaults(fn=cmd_drop)
    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
