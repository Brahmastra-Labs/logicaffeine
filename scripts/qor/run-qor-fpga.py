#!/usr/bin/env python3
"""The Artix-7 flow of the comparative-synthesis bench — the only writer of benchmarks/silicon/qor_artix7.json.

    run-qor-fpga.py <kernel> <candidate|logos> [--map fabric|native] [--seeds 1,2,3,4,5] [--freq 100] [--deadline 1800]
    run-qor-fpga.py --all [--map fabric|native] [--seeds ...] [--jobs J]   every candidate whose functional gate passes
    run-qor-fpga.py verify

One row is one placement seed of one candidate on the Nexys A7 part (xc7a100tcsg324-1), through the open flow
every candidate shares: yosys `synth_xilinx` and nextpnr-xilinx at the given target frequency, via
scripts/openxc7-build.sh. The candidate is measured out of context, inside its `qorsyn_<candidate>` fold wrapper
(emit_qor_fold): every input from a register the stimulus refills, every output folded into a register, the
candidate kept whole as `u_dut` — so no harness logic sits on its paths and the frequency is the candidate's. Its
function is gated separately, in the shared harness (run-qor-function.py). nextpnr-xilinx times only LUT, CARRY4, flip-flop,
MUXF and I/O cells (arch.cc getPortTimingClass gives DSP48E1, block RAM, LUT-RAM and SRL ports TMG_IGNORE),
so a candidate maps two ways:

* `fabric` — `-nodsp -nobram -nolutram -nosrl`: every cell is one nextpnr times, and the row's maximum
  frequency covers every path;
* `native` — the DSP48E1, block-RAM, LUT-RAM and SRL mapping a design would use: the row's resources, with
  `timing_complete` false whenever a cell nextpnr cannot time is present.

Recorded per row:

* the candidate's own cells, from yosys `stat -json` of the `u_dut` module the harness keeps whole (LUTs,
  flip-flops, CARRY4, DSP48E1, block RAM in RAMB18 units, LUT-RAM, SRLs, MUXF) and the whole design's;
* nextpnr's post-route maximum frequency for the clock (an open-flow estimate, not vendor signoff) and the
  endpoints of its critical path, and where it runs: inside the candidate, from an input register into it,
  from it into the output fold, through it from an input register to the fold (a combinational path from its
  inputs to its outputs), or — never expected — within the wrapper alone;
* K7, the DDR2 memory controllers, have no harness contract (their functional gate is the K7 traffic gate,
  run-qor-dram.py): their rows carry their RTL's SHA3-256 and the command that generated it;
* or, when the open flow refuses the design, the stage that refused it. Every run has a deadline
  (`--deadline`, recorded in the row): nextpnr's router can oscillate on a few overused wires forever, and a
  run that has not finished by then is refused at the stage it was in, with the router's last iterations in
  its excerpt. A run its default router (router2) cannot finish routing is run once more with the older router
  (router1), which resolves the congestion router2 oscillates on: the row is that run's, names its router, and
  keeps router2's verdict and router log (pnr-router2.txt);
* the run's wall time.

The yosys statistics and the place-and-route summary are committed as excerpts under
benchmarks/silicon/qor_logs/artix7/<key>/; `verify` re-parses them.
"""
import argparse
import concurrent.futures
import hashlib
import json
import os
import re
import signal
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import qorlib  # noqa: E402

PART = "xc7a100tcsg324-1"
HEADER = {
    "_generated_by": "scripts/qor/run-qor-fpga.py (never hand-edited)",
    "schema": "logos.qor.artix7/1",
    "provenance": "pnr_open",
    "honesty": "Each row is one place-and-route of one candidate inside the shared harness through the open flow "
               "(yosys synth_xilinx + nextpnr-xilinx) on the Nexys A7 part. Cell counts are yosys's; the frequency "
               "is nextpnr's post-route timing estimate, not vendor signoff.",
}
LEDS = ["H17", "K15", "J13", "N14", "R18", "V17", "U17", "U16", "V16", "T15", "U14", "T16", "V15", "V14", "V12", "V11"]
TIMED = re.compile(r"^(LUT[1-6](_2)?|CARRY4|FD[RSCP]E(_1)?|MUXF[78]|BUFG|IBUF|OBUF|INV|GND|VCC)$")
CATEGORIES = {
    "lut": re.compile(r"^LUT[1-6](_2)?$"),
    "ff": re.compile(r"^FD[RSCP]E(_1)?$|^LD[CP]E$"),
    "carry4": re.compile(r"^CARRY4$"),
    "dsp48e1": re.compile(r"^DSP48E1$"),
    "lutram": re.compile(r"^RAM(32|64|128|256)(M|X\d[DS])$"),
    "srl": re.compile(r"^SRL(C)?(16|32)E$"),
    "muxf": re.compile(r"^MUXF[78]$"),
}
RE_FMAX = re.compile(r"Max frequency for clock '[^']*': ([\d.]+) MHz")
RE_UTIL = re.compile(r"^Info:\s+(\S+):\s+(\d+)/\s*(\d+)\s+\d+%$", re.M)
RE_SRC = re.compile(r"^Info:\s+[\d.]+\s+[\d.]+\s+Source (\S+)$", re.M)
RE_SINK = re.compile(r"^Info:\s+Sink (\S+)$", re.M)
RE_ITER = re.compile(r"^Info:\s+iter=")


def xdc(period_ns):
    lines = ["set_property -dict {PACKAGE_PIN E3 IOSTANDARD LVCMOS33} [get_ports clk]",
             "create_clock -period %.3f -name clk [get_ports clk]" % period_ns,
             "set_property -dict {PACKAGE_PIN J15 IOSTANDARD LVCMOS33} [get_ports rst]"]
    lines += ["set_property -dict {PACKAGE_PIN %s IOSTANDARD LVCMOS33} [get_ports sig[%d]]" % (p, i) for i, p in enumerate(LEDS)]
    return "\n".join(lines) + "\n"


def module_cells(stat, name, seen=None):
    """The cells of module `name` with every non-primitive submodule expanded."""
    mods = stat["modules"]
    m = mods.get("\\" + name) or mods.get(name)
    if m is None:
        return None
    out = {}
    for typ, n in m.get("num_cells_by_type", {}).items():
        sub = typ.lstrip("\\")
        if ("\\" + sub) in mods or sub in mods:
            inner = module_cells(stat, sub)
            for t2, n2 in inner.items():
                out[t2] = out.get(t2, 0) + n * n2
        else:
            out[sub] = out.get(sub, 0) + n
    return out


def categorise(cells):
    cat = {k: 0 for k in CATEGORIES}
    cat["bram18"] = 0
    for typ, n in cells.items():
        if typ == "RAMB18E1":
            cat["bram18"] += n
        elif typ == "RAMB36E1":
            cat["bram18"] += 2 * n
        for k, rx in CATEGORIES.items():
            if rx.match(typ):
                cat[k] += n
    return cat


def router_started(summary):
    """Does a pnr.txt excerpt show the router at work: its start, or an iteration of it?"""
    return any(RE_ITER.match(l) or "Running router" in l for l in summary.splitlines())


def pnr_summary(log):
    """The utilisation block, the router's start, its last five iterations, every maximum-frequency line and the
    last critical-path report."""
    lines = log.splitlines()
    out = []
    try:
        i = next(j for j, l in enumerate(lines) if "Device utilisation:" in l)
        out.append(lines[i])
        for l in lines[i + 1:]:
            if not RE_UTIL.match(l):
                break
            out.append(l)
    except StopIteration:
        pass
    out += [l for l in lines if "Running router" in l][:1]
    out += [l for l in lines if RE_ITER.match(l)][-5:]
    out += [l for l in lines if RE_FMAX.search(l) or "Routing complete" in l]
    starts = [j for j, l in enumerate(lines) if "Critical path report for clock" in l]
    if starts:
        for l in lines[starts[-1]:]:
            if l.startswith("Info: Max frequency"):
                break
            if re.search(r"Source |Sink |Critical path report", l):
                out.append(l)
    return "\n".join(out) + "\n"


def parse_pnr(summary):
    """Post-route figures only: the frequency nextpnr reports after `Routing complete` (a run that never
    routed has none; its placement estimate is not a result)."""
    routed = summary.split("Routing complete", 1)
    fmax = [float(x) for x in RE_FMAX.findall(routed[1])] if len(routed) == 2 else []
    util = {m.group(1): int(m.group(2)) for m in RE_UTIL.finditer(summary)}
    srcs = RE_SRC.findall(summary)
    sinks = RE_SINK.findall(summary)
    path = None
    if srcs and sinks:
        src_in, sink_in = srcs[0].startswith("u_dut."), sinks[-1].startswith("u_dut.")
        crosses = any(c.startswith("u_dut.") for c in srcs + sinks)
        kind = {(True, True): "candidate", (False, True): "input", (True, False): "output",
                (False, False): "through" if crosses else "wrapper"}[(src_in, sink_in)]
        path = {"from": srcs[0], "to": sinks[-1], "in_candidate": src_in and sink_in, "kind": kind}
    keep = ("SLICE_LUTX", "SLICE_FFX", "CARRY4", "DSP48E1_DSP48E1", "RAMB18E1_RAMB18E1", "RAMB36E1_RAMB36E1")
    return {"fmax_mhz": fmax[-1] if fmax else None,
            "utilisation": {k: util[k] for k in keep if k in util}, "critical_path": path}


def untimed(stat_text, top):
    """The cell types of the whole design nextpnr-xilinx does not time, with their counts."""
    cells = module_cells(json.loads(stat_text), top) or {}
    return {t: n for t, n in sorted(cells.items()) if not TIMED.match(t)}


def parse_stat(stat_text, top, dut):
    stat = json.loads(stat_text)
    dut_cells = module_cells(stat, dut)
    top_cells = module_cells(stat, top)
    return {"candidate_cells": categorise(dut_cells) if dut_cells is not None else None,
            "candidate_cells_by_type": dict(sorted(dut_cells.items())) if dut_cells is not None else None,
            "design_cells": categorise(top_cells) if top_cells is not None else None}


# The place-and-route binary every run uses. Its version string names a commit, not the build: a rebuild in place keeps
# it, so each row also records the binary's SHA3-256.
NEXTPNR = os.path.join(qorlib.ROOT, "tools", "openxc7", "nextpnr-xilinx", "nextpnr-xilinx")


def patches_digest():
    d = os.path.join(qorlib.ROOT, "tools", "openxc7", "patches")
    h = hashlib.sha3_256()
    for f in sorted(os.listdir(d)) if os.path.isdir(d) else []:
        h.update(f.encode())
        h.update(open(os.path.join(d, f), "rb").read())
    return h.hexdigest()


def build(env, deadline):
    """Run openxc7-build.sh in its own process group; past `deadline` seconds the whole group is killed.
    Returns (stdout, returncode, timed_out, wall seconds)."""
    t0 = time.monotonic()
    p = subprocess.Popen(["bash", os.path.join(qorlib.ROOT, "scripts", "openxc7-build.sh")], env=env,
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, start_new_session=True)
    try:
        out, _ = p.communicate(timeout=deadline)
        return out, p.returncode, False, time.monotonic() - t0
    except subprocess.TimeoutExpired:
        if p.pid > 1:
            os.killpg(p.pid, signal.SIGKILL)
        out, _ = p.communicate()
        return out, p.returncode, True, time.monotonic() - t0


def verdict_of(stat_present, pnr_text, returncode, stdout, timed_out, deadline):
    if timed_out:
        stage = "synthesis" if not stat_present else ("routing" if pnr_text and "Running router" in pnr_text else "placement")
        return "refused: %s did not finish within %d s" % (stage, deadline)
    if not stat_present:
        return "refused: synthesis"
    if returncode != 0 or "PLACED AND ROUTED" not in stdout:
        return "refused: place-and-route"
    return "complete"


# nextpnr-xilinx's routers in the order a run tries them: its default, and the older router, which resolves the
# congestion the default can oscillate on forever (a few overused wires, iteration after iteration).
ROUTERS = ("router2", "router1")


def ladder(run, kernel, candidate, mapping, seed, freq, deadline):
    """One seed of a candidate on the router ladder: `run` with the default router, and when that cannot finish
    routing within the deadline, once more with the older router. The row is the last run's; a run again says what
    the default router did and keeps its router log as the excerpt pnr-router2.txt. Any other refusal stands."""
    row = run(kernel, candidate, mapping, seed, freq, deadline, router=ROUTERS[0])
    log = row.pop("_pnr", None)
    if not row["verdict"].startswith("refused: routing did not finish"):
        return row
    prior = [("pnr-%s.txt" % ROUTERS[0], log)] if log is not None else []
    again = run(kernel, candidate, mapping, seed, freq, deadline, router=ROUTERS[1], prior=prior)
    again.pop("_pnr", None)
    again["router_ladder"] = [{"router": ROUTERS[0], "verdict": row["verdict"], "wall_s": row["wall_s"]}]
    return again


def one_run(kernel, candidate, mapping, seed, freq, deadline, router=ROUTERS[0], prior=()):
    """Place and route one seed of one candidate with nextpnr's `router` and return its ledger row (the caller records
    it), the excerpts it names (and `prior`, the excerpts of an earlier attempt) under `_excerpts` for the record to
    commit with it, its router log summary under `_pnr` for the ladder."""
    with tempfile.TemporaryDirectory(dir=os.path.join(qorlib.ROOT, "logs")) as run_dir:
        if kernel == "K7":
            rec = qorlib.emit_fold(run_dir, candidate)
        else:
            rec = qorlib.emit(run_dir, kernel, candidate, "shaped", 16)
            rec.update(qorlib.wrap_candidate(run_dir))
        name = rec["candidate"]["name"]
        top, dut = rec["harness_top"], rec["candidate"]["top"]
        rtl = qorlib.rtl_files(run_dir, rec)
        with open(os.path.join(run_dir, "top.xdc"), "w") as f:
            f.write(xdc(1000.0 / freq))
        reports = os.path.join(run_dir, "reports")
        flow = os.path.join(run_dir, "flow")
        os.makedirs(flow)
        src = " ".join(os.path.join(run_dir, f) for f in rec["files"] + [rec["harness"]] if f.endswith((".v", ".sv")))
        env = dict(os.environ, TOP=top, SRC=src, BIT="qor_unused.bit", STOP_AFTER="pnr", REPORT_DIR=reports, WORK=flow,
                   FREQ=str(freq), SEED=str(seed), XDC=os.path.join(run_dir, "top.xdc"), READ_FLAGS="-sv -defer")
        if router != ROUTERS[0]:
            env["ROUTER"] = router
        if mapping == "native":
            env["NODSP"] = ""
        else:
            env.pop("NODSP", None)
            env["SYNTH_EXTRA"] = "-nobram -nolutram -nosrl"
        stdout, returncode, timed_out, wall = build(env, deadline)
        # The flow directory holds the reports too: a run killed at its deadline never copied them out.
        stat_path, pnr_path = os.path.join(flow, "stat.json"), os.path.join(flow, "pnr.log")
        stat_text = open(stat_path).read() if os.path.exists(stat_path) else None
        pnr_log = open(pnr_path).read() if os.path.exists(pnr_path) else None
    key = "%s/artix7/%s/s%d" % (name, mapping, seed)
    excerpts = []
    if stat_text is not None:
        excerpts.append(("stat.json", stat_text))
    if pnr_log is not None:
        excerpts.append(("pnr.txt", pnr_summary(pnr_log)))
    verdict = verdict_of(stat_text is not None, pnr_log, returncode, stdout, timed_out, deadline)
    excerpts.extend(prior)
    committed = ("artix7/%s" % key.replace("/", "-"), excerpts)
    paths, digests = qorlib.excerpt_record(*committed)
    row = {"key": key, "kernel": kernel, "candidate": name, "source_kind": rec["source_kind"], "part": PART,
           "mapping": mapping, "seed": seed, "target_mhz": freq, "verdict": verdict, "harness_top": top, "candidate_top": dut,
           "deadline_s": deadline, "wall_s": round(wall, 1), "rtl_files": rtl,
           "flow": "qorsyn fold wrapper; yosys synth_xilinx -family xc7 -flatten%s; nextpnr-xilinx --freq %s --seed %d%s" % (
               "" if mapping == "native" else " -nodsp -nobram -nolutram -nosrl", freq, seed,
               "" if router == ROUTERS[0] else " --router %s" % router),
           "router": router,
           "yosys": qorlib.tool_version(["yosys", "-V"]),
           "nextpnr": qorlib.tool_version([NEXTPNR, "--version"]),
           "nextpnr_patches_sha3": patches_digest(),
           "nextpnr_sha3": qorlib.sha3_file(NEXTPNR),
           "provenance": HEADER["provenance"], "recorded_at": qorlib.now(),
           "reproduce": "python3 scripts/qor/run-qor-fpga.py %s %s --map %s --seeds %d --freq %s --deadline %d" % (
               kernel, candidate, mapping, seed, freq, deadline),
           "excerpts": paths, "excerpt_sha3": digests}
    if kernel == "K7":
        row.update({"rtl_sha3_256": rec["rtl_sha3_256"], "generate": rec["generate"], "note": rec["note"]})
    if stat_text is not None:
        row.update(parse_stat(stat_text, top, dut))
        row["untimed_cells"] = untimed(stat_text, top)
        row["timing_complete"] = not row["untimed_cells"]
    if pnr_log is not None:
        row.update(parse_pnr(pnr_summary(pnr_log)))
    row["_pnr"] = pnr_summary(pnr_log) if pnr_log is not None else None
    row["_excerpts"] = committed
    return row


def record(row):
    excerpts = row.pop("_excerpts")
    qorlib.update("artix7", HEADER, row, excerpts)
    cells = row.get("candidate_cells") or {}
    print("%-34s %-6s s%d %-26s LUT %5s FF %5s DSP %3s  %s MHz (timing complete: %s)  critical path: %s  %ss" % (
        row["candidate"], row["mapping"], row["seed"], row["verdict"], cells.get("lut"), cells.get("ff"), cells.get("dsp48e1"),
        row.get("fmax_mhz"), row.get("timing_complete"), (row.get("critical_path") or {}).get("kind"), row["wall_s"]),
        flush=True)
    return row["verdict"] == "complete"


def verify():
    doc = qorlib.load("artix7", HEADER)
    bad = ["%s: no row names this excerpt" % d for d in qorlib.orphan_excerpts("artix7", doc["rows"])]
    for row in doc["rows"]:
        texts, problems = qorlib.read_excerpts(row)
        bad += problems
        if not row["flow"].startswith("qorsyn fold wrapper;"):
            bad.append("%s: not measured in the fold wrapper (%s)" % (row["key"], row["flow"]))
        if "stat.json" in texts:
            got = parse_stat(texts["stat.json"], row["harness_top"], row["candidate_top"])
            got["untimed_cells"] = untimed(texts["stat.json"], row["harness_top"])
            got["timing_complete"] = not got["untimed_cells"]
            for f, v in got.items():
                if row.get(f) != v:
                    bad.append("%s: %s disagrees with stat.json" % (row["key"], f))
        if "pnr.txt" in texts:
            got = parse_pnr(texts["pnr.txt"])
            for f, v in got.items():
                if row.get(f) != v:
                    bad.append("%s: %s disagrees with pnr.txt" % (row["key"], f))
        routed = "Routing complete" in texts.get("pnr.txt", "")
        if (row["verdict"] == "complete") != (routed and row.get("fmax_mhz") is not None):
            bad.append("%s: verdict %r but pnr.txt %s" % (row["key"], row["verdict"], "routed" if routed else "never routed"))
        if row["verdict"].startswith("refused: routing did not finish") and not router_started(texts.get("pnr.txt", "")):
            bad.append("%s: a routing timeout whose pnr.txt never shows the router start" % row["key"])
    if bad:
        for b in bad:
            print("FAIL: " + b, file=sys.stderr)
        sys.exit(1)
    print("verified %d rows" % len(doc["rows"]))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("kernel", nargs="?")
    ap.add_argument("candidate", nargs="?")
    ap.add_argument("--map", dest="mapping", choices=["fabric", "native"], default="fabric")
    ap.add_argument("--seeds", default="1,2,3,4,5")
    ap.add_argument("--freq", type=float, default=100.0)
    ap.add_argument("--deadline", type=int, default=1800, help="seconds one place-and-route may take")
    ap.add_argument("--jobs", type=int, default=1, help="place-and-routes at once (the ledger is written by this process alone)")
    ap.add_argument("--all", action="store_true")
    a = ap.parse_args()
    if a.kernel == "verify":
        return verify()
    seeds = [int(s) for s in a.seeds.split(",")]
    pairs = [(a.kernel, a.candidate)]
    if a.all:
        rows = qorlib.load("function", {"rows": []})["rows"]
        passing = {}
        for r in rows:
            passing.setdefault((r["kernel"], r["candidate"]), set()).add(r["mode"] if r["pass"] else "fail")
        pairs = sorted((k, c) for (k, c), modes in passing.items() if modes == {"shaped", "edge"})
        pairs += [("K7", name) for name in sorted(qorlib.fold_designs())]
    elif not (a.kernel and a.candidate):
        ap.error("name a kernel and a candidate, or --all, or verify")
    runs = [(k, c, a.mapping, s, a.freq, a.deadline) for k, c in pairs for s in seeds]
    with concurrent.futures.ProcessPoolExecutor(max_workers=a.jobs) as pool:
        for f in [pool.submit(ladder, one_run, *r) for r in runs]:
            record(f.result())


if __name__ == "__main__":
    main()
