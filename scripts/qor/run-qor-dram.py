#!/usr/bin/env python3
"""The K7 traffic gate of the comparative-synthesis bench — the only writer of benchmarks/silicon/qor_dram.json.

    run-qor-dram.py <candidate> <sequential|scattered|probe> [--words N] [--upstream-module]
    run-qor-dram.py all [--words N]
    run-qor-dram.py verify [--rerun]

Every DDR2 controller of K7 carries one stream of 64-bit words — written in order, read back in the same order —
to the Nexys A7's part (MT47H64M16HR-25) at DDR2-400, each through its own request interface
(codegen_sva::ddr2_traffic): the sequential and the scattered streams as fast as the controller takes them, and the
probe, the scattered words read one every 128 DRAM clocks (a read that finds the controller idle). The testbench logs every command on the bus, every request the controller accepted and
every word it delivered; `emit_qor_dram judge` holds the bus to the JEDEC DDR2 rules (codegen_sva::ddr2_jedec),
matches every request to the column command that served it, checks every word read back, and measures. A row
passes only when the judge accepts the run.

LOGOS's ddr2_ctrl runs against `ddr2_device`, the part through an ideal PHY. LiteDRAM's controller, the RTL the bench
synthesises, and LOGOS's ddr2_dfi_ctrl, which drives the same DFI, run against LiteDRAM's own SDRAMPHYModel
(`litedram_controller.py --phy-model`), LiteDRAM's RTL and the model each elaborated by yosys `proc` before Icarus
Verilog simulates it: Migen writes combinational blocks as a default followed by the
value, and in the command multiplexer's structural cycle each default wakes the next block, which an event-driven
simulator never settles; `proc` turns every process into the multiplexers a synthesiser would build. `--upstream-module` drives LiteDRAM
built with its own MT47H64M16 entry, which leaves tRRD, tFAW and tRAS unset, instead of the datasheet-complete one:
a disclosure row, recorded whatever the judge finds, never the candidate's.

The bus log stays under logs/qor/dram/; the row carries its SHA3-256 and line count, and the judgement, the stage
record and the simulator's verdict are committed as excerpts. `verify` re-reads every row from its excerpts;
`verify --rerun` simulates every row again and requires the same log.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import qorlib  # noqa: E402

HEADER = {
    "_generated_by": "scripts/qor/run-qor-dram.py (never hand-edited)",
    "schema": "logos.qor.dram/1",
    "provenance": "simulated_iverilog",
    "honesty": "Each row is an RTL simulation (Icarus Verilog) of one DDR2 controller carrying the K7 word stream "
               "to a behavioural model of the part through an ideal PHY: its bus held to the JEDEC DDR2 rules, every "
               "word read back checked, latencies and spans counted in DRAM clocks. Not silicon, not a board.",
}
EMIT_DRAM = os.path.join(qorlib.TARGET, "debug", "examples", "emit_qor_dram")
GENERATOR = os.path.join(qorlib.SIL, "qor", "kernels", "K7", "litedram_controller.py")
RUNS = os.path.join(qorlib.LOGS, "qor", "dram")
WORDS = 1024
PATTERNS = ("sequential", "scattered", "probe")
METRICS = ("requests", "stream_cycles", "payload_cycles", "bus_span", "data_cycles", "row_hits", "commands",
           "write_latency", "read_latency", "read_service", "read_bus")


def emitter():
    if not os.path.exists(EMIT_DRAM):
        sys.exit("build the emitter first: cargo build -p logicaffeine-compile --example emit_qor_dram")
    return EMIT_DRAM


def run(cmd, cwd=None, what=None):
    r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("%s: %s" % (what or cmd[0], (r.stdout + r.stderr).strip()[-3000:]))
    return r.stdout


def key_of(candidate, pattern, words, upstream):
    return "%s%s/%s/%d" % (candidate, "+upstream" if upstream else "", pattern, words)


def litex(run_dir, top, args, upstream):
    """Generate `top` with litedram_controller.py `args` in `run_dir` and elaborate it by yosys `proc` for the
    simulator; the generated RTL's path."""
    extra = ["--upstream-module"] if upstream else []
    rtl = os.path.join(run_dir, top + "_rtl.v")
    run([qorlib.LITEX_PYTHON, GENERATOR] + args + [rtl] + extra, what="litedram_controller.py %s" % " ".join(args))
    run(["yosys", "-q", "-p", "read_verilog %s; hierarchy -top %s; proc; check -assert; opt_clean; write_verilog "
         "-noattr %s" % (rtl, top, os.path.join(run_dir, top + ".v"))], what="yosys %s" % top)
    return rtl


def phy_model(run_dir, row_bits, upstream=False):
    """LiteDRAM's SDRAMPHYModel storing `row_bits` row bits, the PHY and part every DFI controller of K7 runs against."""
    litex(run_dir, "litedram_phy_model", ["--phy-model", "--row-bits", str(row_bits)], upstream)


def litedram(run_dir, row_bits, upstream):
    """LiteDRAM's controller as the bench synthesises it and LiteDRAM's SDRAMPHYModel, generated in `run_dir` and each
    elaborated by yosys `proc` for the simulator; the controller RTL's SHA3-256."""
    ctrl = litex(run_dir, "litedram_ctrl", [], upstream)
    phy_model(run_dir, row_bits, upstream)
    return qorlib.sha3_file(ctrl)


def prepare(run_dir, candidate, pattern, words, upstream=False):
    """Stage one run in `run_dir`: the testbench, the stream, the controller and its model of the part. Returns (the
    stage record, the controller RTL's SHA3-256, the simulation's sources, the controller's instance in the testbench
    and the testbench's clock counter)."""
    run([emitter(), "stage", run_dir, candidate, pattern, str(words)], what="emit_qor_dram stage")
    stage = json.load(open(os.path.join(run_dir, "dram.json")))
    if candidate == "k7_litedram_ctrl":
        rtl_sha3 = litedram(run_dir, stage["row_bits"], upstream)
        return stage, rtl_sha3, ["tb_qor_dram.v", "litedram_ctrl.v", "litedram_phy_model.v"], ("litedram_ctrl", "ctrl"), "k"
    if upstream:
        sys.exit("--upstream-module builds LiteDRAM; %s is LOGOS's" % candidate)
    if candidate == "k7_logos_dfi_ctrl":
        phy_model(run_dir, stage["row_bits"])
        rtl_sha3 = qorlib.sha3_file(os.path.join(run_dir, "ddr2_dfi_ctrl.v"))
        return stage, rtl_sha3, ["tb_qor_dram.v", "ddr2_dfi_ctrl.v", "litedram_phy_model.v"], ("ddr2_dfi_ctrl", "ctrl"), "k"
    rtl_sha3 = qorlib.sha3_file(os.path.join(run_dir, "ddr2_ctrl.v"))
    return stage, rtl_sha3, ["tb_qor_dram.v", "ddr2_ctrl.v", "ddr2_device.v"], ("ddr2_ctrl", "dut"), "cyc"


def run_sim(run_dir, sources, extra=()):
    """Icarus Verilog over `sources` (and any `extra` arguments) in `run_dir`; the simulator's verdict lines."""
    run(["iverilog", "-g2005", "-s", "tb_qor_dram", "-o", "sim.vvp"] + list(extra) + sources, cwd=run_dir, what="iverilog")
    sim = subprocess.run(["vvp", "-n", "sim.vvp"], cwd=run_dir, capture_output=True, text=True)
    return "\n".join(l for l in sim.stdout.splitlines() if l.startswith(("PASS", "FAIL"))) + "\n"


def simulate(candidate, pattern, words, upstream):
    """Stage, simulate and judge one run in its directory under logs/qor/dram/; returns (stage record, controller
    RTL SHA3, simulator verdict, the judge's output or refusal, the log's SHA3 and lines)."""
    run_dir = os.path.join(RUNS, key_of(candidate, pattern, words, upstream).replace("/", "-"))
    shutil.rmtree(run_dir, ignore_errors=True)
    os.makedirs(run_dir)
    stage, rtl_sha3, sources, _, _ = prepare(run_dir, candidate, pattern, words, upstream)
    verdict = run_sim(run_dir, sources)
    log = os.path.join(run_dir, "dram.log")
    log_sha3, log_lines = (qorlib.sha3_file(log), sum(1 for _ in open(log))) if os.path.exists(log) else (None, 0)
    j = subprocess.run([emitter(), "judge", run_dir], capture_output=True, text=True)
    judged = json.loads(j.stdout) if j.returncode == 0 else {"refusal": j.stderr.strip().removeprefix("emit_qor_dram: ")}
    return stage, rtl_sha3, verdict, judged, log_sha3, log_lines


def gate(candidate, pattern, words, upstream):
    stage, rtl_sha3, verdict, judged, log_sha3, log_lines = simulate(candidate, pattern, words, upstream)
    key = key_of(candidate, pattern, words, upstream)
    excerpts = ("dram/%s" % key.replace("/", "-"),
                [("dram.json", json.dumps(stage, indent=1, sort_keys=True) + "\n"), ("sim.txt", verdict),
                 ("judgement.json", json.dumps(judged, indent=1, sort_keys=True) + "\n")])
    paths, digests = qorlib.excerpt_record(*excerpts)
    row = {
        "key": key, "kernel": "K7", "candidate": candidate, "pattern": pattern, "words": words,
        "module": "upstream" if upstream else ("datasheet" if candidate == "k7_litedram_ctrl" else None),
        "part": stage["part"], "tck_ps": stage["tck_ps"], "bl": stage["bl"], "cl": stage["cl"], "every": stage["every"],
        "interface": {"parts": stage["parts"], "cols_per_part": stage["cols_per_part"]},
        "rtl_sha3_256": rtl_sha3, "log_sha3_256": log_sha3, "log_lines": log_lines,
        "pass": "refusal" not in judged, "refusal": judged.get("refusal"),
        **{m: judged.get(m) for m in METRICS},
        "simulator": qorlib.tool_version(["iverilog", "-V"]),
        "provenance": HEADER["provenance"], "recorded_at": qorlib.now(),
        "reproduce": "cargo build -p logicaffeine-compile --example emit_qor_dram && "
                     "python3 scripts/qor/run-qor-dram.py %s %s --words %d%s"
                     % (candidate, pattern, words, " --upstream-module" if upstream else ""),
        "excerpts": paths, "excerpt_sha3": digests, "_excerpts": excerpts,
    }
    return row


def record(row):
    excerpts = row.pop("_excerpts")
    qorlib.update("dram", HEADER, row, excerpts)
    if row["pass"]:
        print("%-36s PASS stream %d clocks, read latency %s..%s" % (row["key"], row["stream_cycles"],
                                                                  row["read_latency"]["min"], row["read_latency"]["max"]), flush=True)
    else:
        print("%-36s REFUSED %s" % (row["key"], row["refusal"]), flush=True)
    return row["pass"]


def verify(rerun):
    doc = qorlib.load("dram", HEADER)
    bad = ["%s: no row names this excerpt" % d for d in qorlib.orphan_excerpts("dram", doc["rows"])]
    for row in doc["rows"]:
        texts, problems = qorlib.read_excerpts(row)
        bad += problems
        if "judgement.json" in texts:
            judged = json.loads(texts["judgement.json"])
            if ("refusal" not in judged) != row["pass"] or judged.get("refusal") != row["refusal"]:
                bad.append("%s: the verdict disagrees with the judgement" % row["key"])
            for m in METRICS:
                if judged.get(m) != row[m]:
                    bad.append("%s: %s is %r in the judgement, %r in the row" % (row["key"], m, judged.get(m), row[m]))
        if "dram.json" in texts:
            stage = json.loads(texts["dram.json"])
            for f in ("part", "tck_ps", "bl", "cl", "words", "pattern", "every"):
                if stage[f] != row[f]:
                    bad.append("%s: %s is %r staged, %r in the row" % (row["key"], f, stage[f], row[f]))
        if rerun:
            stage, rtl_sha3, verdict, judged, log_sha3, log_lines = simulate(
                row["candidate"], row["pattern"], row["words"], row["module"] == "upstream")
            for f, got in (("rtl_sha3_256", rtl_sha3), ("log_sha3_256", log_sha3), ("log_lines", log_lines)):
                if got != row[f]:
                    bad.append("%s: %s is %r again, %r recorded" % (row["key"], f, got, row[f]))
            if texts.get("judgement.json") and json.loads(texts["judgement.json"]) != judged:
                bad.append("%s: the judgement differs on a rerun" % row["key"])
    if bad:
        for b in bad:
            print("FAIL: " + b, file=sys.stderr)
        sys.exit(1)
    print("verified %d rows%s" % (len(doc["rows"]), " (simulated again)" if rerun else ""))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("candidate")
    ap.add_argument("pattern", nargs="?", choices=PATTERNS)
    ap.add_argument("--words", type=int, default=WORDS)
    ap.add_argument("--upstream-module", action="store_true")
    ap.add_argument("--rerun", action="store_true", help="verify: simulate every row again")
    a = ap.parse_args()
    if a.candidate == "verify":
        return verify(a.rerun)
    if a.candidate == "all":
        names = run([emitter(), "--list"]).split()
        ok = [record(gate(c, p, a.words, False)) for c in names for p in PATTERNS]
        for p in PATTERNS:
            record(gate("k7_litedram_ctrl", p, a.words, True))
        sys.exit(0 if all(ok) else 1)
    if not a.pattern:
        ap.error("name a candidate and a stream, or all, or verify")
    sys.exit(0 if record(gate(a.candidate, a.pattern, a.words, a.upstream_module)) else 1)


if __name__ == "__main__":
    main()
