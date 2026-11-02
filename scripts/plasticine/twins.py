#!/usr/bin/env python3
"""Spatial's DotProduct, TPCHQ6, OuterProduct, GDA, BlackScholes and Kmeans regression variants and their LOGOS twins,
through the public Plasticine chain (Spatial → PIR → plastiroute → Tungsten).

    twins.py variants                     every variant: its app, kind and knobs
    twins.py emit                         write each variant's LOGOS twin into the toolchain's regression dir
    twins.py run [--app A]... [--deadline S]
                                          gate.sh each variant and each twin in the container, one at a time
    twins.py record --toolchain "<branches>" [--app A]...
                                          plasticine-gate-ledger.py add for every run that passed both networks
    twins.py apps run|record [--toolchain "<branches>"]
                                          Spatial's own apps whose twin the compiler does not print yet (APPS),
                                          run alone and recorded as `spatial_app` rows

A variant's knobs are its abstract class's defaults with the variant's overrides (`class DotProduct_3 extends
DotProduct(op=2)`), read from the toolchain's own spatial/pir/regression/{DotProduct,TPCHQ6,OuterProduct,GDA,
BlackScholes,Kmeans}.scala — never typed here — with the app's runtime arguments where its class sets them (Kmeans'
`runtimeArgs`, its epochs `I` and points `N`); a `pirArgs` override in a variant's body (`BlackScholes_0`'s
`--split-algo=bfs`) is its PIR arguments, which its twin carries too, and any other override is refused. Its twin is
the LOGOS program (plasticine_report::logos_dot_program / logos_tpchq6_program / logos_outer_program /
logos_gda_program / logos_blackscholes_program / logos_kmeans_program) at the variant's shape, recognized by the tensor recognizer and printed in the same app
structure at the same knobs (examples/emit_plasticine_spatial_twin), so the two runs differ only in who wrote them. Run logs go to
logs/plasticine-twins/; the toolchain is LOGOS_PLASTICINE_TOOLCHAIN (default target/plasticine-toolchain).
"""
import argparse
import importlib.util
import json
import os
import re
import shlex
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
TOOLCHAIN = os.environ.get("LOGOS_PLASTICINE_TOOLCHAIN", os.path.join(ROOT, "target", "plasticine-toolchain"))
REGRESSION = os.path.join(TOOLCHAIN, "spatial", "pir", "regression")
GEN = os.path.join(TOOLCHAIN, "spatial", "gen", "Tst_pirTest")
LOGS = os.path.join(ROOT, "logs", "plasticine-twins")
MANIFEST = os.path.join(LOGS, "twins.json")
LEDGER = os.path.join(ROOT, "scripts", "accel", "plasticine-gate-ledger.py")

# Each kind: Spatial's app, its knobs in the order the ledger records them (and the twin emitter takes them).
# What the tensor recognizer reads a twin's LOGOS program as, and the spatial_emit function that prints it.
READ_AS = {
    "dot": ("a dot product (a zip sum of a product of two columns)", "emit_spatial_zip_sum_test"),
    "tpchq6": ("TPC-H Q6 (a zip sum: the query's guarded revenue over four columns)", "emit_spatial_zip_sum_test"),
    "outer": ("an outer product", "emit_spatial_outer_test"),
    "gda": ("GDA's covariance step (a scatter accumulation: each row centred on the mean its label picks, added as a "
            "rank-1 update)", "emit_spatial_gda_test"),
    "blackscholes": ("Black-Scholes (a zip map: every option priced from its six columns by the app's own Taylor "
                     "approximations, the program's functions inlined)", "emit_spatial_zip_map_test"),
    "kmeans": ("k-means clustering (Lloyd's algorithm: every point joins its nearest centroid by the squared distance, "
               "every centroid set to the mean of its points)", "emit_spatial_kmeans_test"),
}
KINDS = {
    "dot": ("DotProduct", ["N", "ts", "ip", "op"]),
    "tpchq6": ("TPCHQ6", ["N", "ts", "ip", "op"]),
    "outer": ("OuterProduct", ["M", "N", "ts1", "ts2", "ip", "ip1", "ip2", "op1", "op2"]),
    "gda": ("GDA", ["C", "R", "ts", "op", "mp1", "mp2", "ip"]),
    "blackscholes": ("BlackScholes", ["N", "ts", "op", "ip"]),
    "kmeans": ("Kmeans", ["K", "D", "ts", "ip", "op", "mp1", "mp2", "mp3"]),
}

# The runtime arguments a kind's app sets in its class (`override def runtimeArgs = "2 256"`), named, in order: knobs
# of the run as much as its class parameters, so its twin is held to them too.
RUNTIME = {"kmeans": ["I", "N"]}


# Spatial's own apps the bench runs without a twin: (regression file, the variant run).
APPS = [("SGD", "SGD_0"), ("LogReg", "LogReg_0")]


def fail(msg):
    print("twins.py: " + msg, file=sys.stderr)
    sys.exit(1)


def int_value(expr, where):
    """A Scala integer expression of literals (`1024 * 128`), evaluated; anything else is refused."""
    if not re.fullmatch(r"[0-9+\-*/() \t]+", expr):
        fail("%s: `%s` is not an integer expression of literals" % (where, expr.strip()))
    return int(eval(expr, {"__builtins__": {}}, {}))


EXPR = r"([^,\n)]+(?:\([^)]*\)[^,\n)]*)?)"


def int_knobs(text, where, header):
    """A class header's integer knobs (`name: scala.Int = <expr>`; knobs of other types are not the bench's), or a
    variant's overrides (`name = <expr>`)."""
    pattern = r"(\w+)\s*:\s*scala\.Int\s*=\s*" + EXPR if header else r"(\w+)\s*=\s*" + EXPR
    return {k: int_value(v, "%s %s" % (where, k)) for k, v in re.findall(pattern, text)}


def knobs(app, variant):
    """A variant's integer knobs: its abstract class's defaults with the variant's own overrides."""
    text = open(os.path.join(REGRESSION, app + ".scala")).read()
    head = re.search(r"abstract class %s\((.*?)\)\s*extends" % app, text, re.S)
    if not head:
        fail("%s.scala: no `abstract class %s(...)`" % (app, app))
    params = int_knobs(head.group(1), app, header=True)
    m = re.search(r"^\s*class %s extends %s(?:\(([^)]*)\))?" % (variant, app), text, re.M)
    if not m:
        fail("%s.scala: no `class %s extends %s`" % (app, variant, app))
    params.update(int_knobs(m.group(1) or "", variant, header=False))
    return params


def variants():
    """Every uncommented `class <App>_<k> extends <App>(...)` with its knobs, in source order."""
    out = []
    for kind, (app, knobs) in KINDS.items():
        text = open(os.path.join(REGRESSION, app + ".scala")).read()
        head = re.search(r"abstract class %s\((.*?)\)\s*extends" % app, text, re.S)
        if not head:
            fail("%s.scala: no `abstract class %s(...)`" % (app, app))
        defaults = int_knobs(head.group(1), app, header=True)
        if sorted(defaults) != sorted(knobs):
            fail("%s's knobs are %s, not %s" % (app, sorted(defaults), sorted(knobs)))
        runtime = runtime_args(text, app, RUNTIME.get(kind, []))
        for line in text.splitlines():
            m = re.match(r"\s*class (%s_\w+) extends %s(?:\(([^)]*)\))?\s*(?:\{(.*)\})?\s*$" % (app, app), line)
            if not m:
                continue
            pir_args = pir_args_of(m.group(3) or "", m.group(1))
            if pir_args is not None and kind != "blackscholes":
                fail("%s overrides its PIR arguments, which the %s twin does not carry" % (m.group(1), kind))
            params = dict(defaults)
            for k, v in int_knobs(m.group(2) or "", m.group(1), header=False).items():
                if k not in params:
                    fail("%s sets %s, which %s does not take" % (m.group(1), k, app))
                params[k] = v
            if kind == "outer" and params["M"] != params["N"]:
                fail("%s: OuterProduct sizes both vectors by N (its sizeA = N), so M != N is not the app it names" % m.group(1))
            out.append({"app": m.group(1), "kind": kind, "params": dict({k: params[k] for k in knobs}, **runtime),
                        "pir_args": pir_args})
    return out


def runtime_args(text, app, names):
    """The app's runtime arguments, named `names`, from its class's `override def runtimeArgs = "<ints>"` — or none,
    for a kind that takes none."""
    m = re.findall(r'override def runtimeArgs = "([^"]*)"', text)
    if not names:
        if m:
            fail("%s sets runtime arguments its twin does not take" % app)
        return {}
    if len(m) != 1:
        fail("%s.scala: not one `override def runtimeArgs`" % app)
    values = m[0].split()
    if len(values) != len(names):
        fail("%s's runtime arguments are %r, not %s" % (app, m[0], " ".join(names)))
    return {n: int_value(v, "%s runtime %s" % (app, n)) for n, v in zip(names, values)}


def pir_args_of(body, variant):
    """What a variant's `override def pirArgs = super.pirArgs + "<args>"` appends, or None; any other statement in the
    variant's body is refused, since its twin would not reproduce it."""
    stmts = [x.strip() for x in body.split(";") if x.strip()]
    args = None
    for st in stmts:
        m = re.fullmatch(r'override def pirArgs = super\.pirArgs \+ "([^"\\]*)"', st)
        if not m:
            fail("%s: `%s` is not an override the twin reproduces" % (variant, st))
        args = m.group(1)
    return args


def cmd_variants(_a):
    for v in variants():
        print("%-18s %-5s %s%s" % (v["app"], v["kind"], " ".join("%s=%d" % kv for kv in v["params"].items()),
                                    "" if v["pir_args"] is None else "  pirArgs +%r" % v["pir_args"]))


def cmd_emit(_a):
    pairs = []
    for v in variants():
        args = ["cargo", "run", "-q", "-p", "logicaffeine-compile", "--example", "emit_plasticine_spatial_twin", "--",
                v["kind"], REGRESSION] + [str(x) for x in v["params"].values()]
        if v["pir_args"] is not None:
            args.append(v["pir_args"])
        p = subprocess.run(args, cwd=ROOT, capture_output=True, text=True)
        if p.returncode != 0:
            fail("%s: the twin emitter refused: %s" % (v["app"], p.stderr.strip()))
        twin = p.stdout.strip().splitlines()[-1]
        pairs.append(dict(v, twin=twin, emit=" ".join(args[7:])))
        print("%-18s → %s" % (v["app"], twin))
    os.makedirs(LOGS, exist_ok=True)
    with open(MANIFEST, "w") as f:
        json.dump(pairs, f, indent=2)
        f.write("\n")


def manifest():
    if not os.path.exists(MANIFEST):
        fail("no %s — run `twins.py emit` first" % os.path.relpath(MANIFEST, ROOT))
    return json.load(open(MANIFEST))


def apps(pairs, only):
    every = [a for p in pairs for a in (p["app"], p["twin"])]
    for a in only:
        if a not in every:
            fail("%s is neither a variant nor a twin" % a)
    return [a for a in every if not only or a in only]


def ledger_tool():
    """scripts/accel/plasticine-gate-ledger.py as a module: its own reading of a run's logs."""
    spec = importlib.util.spec_from_file_location("plasticine_gate_ledger", LEDGER)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def run_verdict(gen_dir):
    """A run as the ledger would take it: PASS when both networks' simulations completed (a cycle count, no deadlock)
    and printed PASS: true; otherwise the first network that did not, and how."""
    ledger = ledger_tool()
    for name, log in (("ideal network", "runp2p.log"), ("hybrid network", "runhybrid.log")):
        path = os.path.join(gen_dir, "log", log)
        if not os.path.exists(path):
            return "%s: no log" % name
        text = ledger.read(path)
        stuck = ledger.deadlocked_at(text)
        if stuck is not None:
            return "%s: deadlock at cycle %d" % (name, stuck)
        run = ledger.parse_run(text)
        if run["cycles"] is None:
            return "%s: no completion" % name
        if run["pass"] is not True:
            return "%s: gold check failed" % name
    return "PASS"


def cmd_run(a, every=None):
    os.makedirs(LOGS, exist_ok=True)
    for app in (every if every is not None else apps(manifest(), a.apps)):
        log = os.path.join(LOGS, app + ".log")
        name = "logos-twin-%s-%d" % (app.lower().replace("_", "-"), os.getpid())
        env = dict(os.environ, LOGOS_PLASTICINE_TOOLCHAIN=TOOLCHAIN, LOGOS_PLASTICINE_NAME=name)
        start = time.time()
        with open(log, "w") as f:
            p = subprocess.Popen(["bash", os.path.join(ROOT, "scripts", "plasticine", "run-in-container.sh"),
                                  "bash", "/gate.sh", app], cwd=ROOT, env=env, stdout=f, stderr=subprocess.STDOUT)
            try:
                rc = p.wait(timeout=a.deadline)
            except subprocess.TimeoutExpired:
                subprocess.run(["docker", "kill", name], capture_output=True)
                p.wait()
                rc = "deadline %ds" % a.deadline
        print("%-48s %-40s rc %s  %4.0fs" % (app, run_verdict(os.path.join(GEN, app)), rc, time.time() - start), flush=True)


def cmd_record(a):
    pairs = manifest()
    for p in pairs:
        app_name, knobs = KINDS[p["kind"]]
        params = " ".join("%s=%d" % kv for kv in p["params"].items())
        for app, ours in ((p["app"], False), (p["twin"], True)):
            if a.apps and app not in a.apps:
                continue
            gen = os.path.join(GEN, app)
            args = [sys.executable, LEDGER, "add", "--kind", p["kind"], "--app", app, "--gen-dir", gen,
                    "--params", params, "--toolchain", a.toolchain]
            if ours:
                args += ["--logos-source", os.path.join(REGRESSION, app + ".lg"),
                         "--app-source", "emitted from LOGOS by spatial_emit::%s (%s.lg recognized as %s; Spatial's "
                                         "%s structure at %s's knobs)"
                                         % (READ_AS[p["kind"]][1], app, READ_AS[p["kind"]][0], app_name, p["app"]),
                         "--reproduce", "cargo run -p logicaffeine-compile --example emit_plasticine_spatial_twin -- "
                                        "%s target/plasticine-toolchain/spatial/pir/regression %s && "
                                        "scripts/plasticine/run-in-container.sh bash /gate.sh %s"
                                        % (p["kind"], " ".join([str(x) for x in p["params"].values()]
                                                               + ([shlex.quote(p["pir_args"])] if p.get("pir_args") is not None else [])), app)]
            else:
                args += ["--reproduce", "scripts/plasticine/fetch.sh && scripts/plasticine/run-in-container.sh && "
                                        "scripts/plasticine/run-in-container.sh bash /gate.sh %s" % app]
            r = subprocess.run(args, cwd=ROOT, capture_output=True, text=True)
            print("%-48s %s" % (app, (r.stdout or r.stderr).strip().splitlines()[-1] if (r.stdout or r.stderr) else r.returncode))


def cmd_apps(a):
    if a.action == "run":
        return cmd_run(argparse.Namespace(apps=[v for _, v in APPS], deadline=a.deadline), every=[v for _, v in APPS])
    if not a.toolchain:
        fail("apps record needs --toolchain")
    for app, variant in APPS:
        args = [sys.executable, LEDGER, "add", "--kind", "spatial_app", "--app", variant, "--gen-dir",
                os.path.join(GEN, variant), "--toolchain", a.toolchain,
                "--params", " ".join("%s=%d" % kv for kv in sorted(knobs(app, variant).items())),
                "--app-source", "spatial/pir/regression/%s.scala (Spatial's own regression app, unmodified)" % app,
                "--reproduce", "scripts/plasticine/fetch.sh && scripts/plasticine/run-in-container.sh && "
                               "scripts/plasticine/run-in-container.sh bash /gate.sh %s" % variant]
        r = subprocess.run(args, cwd=ROOT, capture_output=True, text=True)
        print("%-18s %s" % (variant, (r.stdout or r.stderr).strip().splitlines()[-1] if (r.stdout or r.stderr) else r.returncode))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("variants").set_defaults(fn=cmd_variants)
    sub.add_parser("emit").set_defaults(fn=cmd_emit)
    run = sub.add_parser("run")
    run.add_argument("--app", dest="apps", action="append", default=[])
    run.add_argument("--deadline", type=int, default=3600, help="seconds one app may take before its container is killed")
    run.set_defaults(fn=cmd_run)
    rec = sub.add_parser("record")
    rec.add_argument("--toolchain", required=True)
    rec.add_argument("--app", dest="apps", action="append", default=[])
    rec.set_defaults(fn=cmd_record)
    apps_ = sub.add_parser("apps")
    apps_.add_argument("action", choices=["run", "record"])
    apps_.add_argument("--toolchain")
    apps_.add_argument("--deadline", type=int, default=3600)
    apps_.set_defaults(fn=cmd_apps)
    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
