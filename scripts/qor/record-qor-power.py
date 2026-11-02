#!/usr/bin/env python3
"""Energy per operation of the comparative-synthesis candidates on Sky130 — the only writer of
benchmarks/silicon/qor_power.json.

    record-qor-power.py run <block>     measure one power block (a `qorpwr_` block of qor_sky130_power.json)
    record-qor-power.py all             every power block whose run is recorded and has no row yet
    record-qor-power.py verify          re-derive every row from its committed excerpts

A power block is a candidate inside the harness of its functional gate, hardened whole by
scripts/qor/run-qor-sky130.py at its kernel's iso period: the stimulus generator, the candidate kept whole as `u_dut`,
the signature register. It drives itself from its clock and reset, so its gate-level simulation is the candidate at
work on the gate's stimulus, through the netlist the run routed. The measurement:

* The window. The run's RTL is simulated first, with the reset the gate-level simulation gets. The window opens a
  quarter period after the falling edge at which the harness has folded K results, and closes the same way at K + N
  results. K is twice the operations in flight when the first result folds (at least 8), so the pipeline is full
  before the window opens. N is the operations of at least 4000 cycles and at least 64. The window holds N
  operations in whole clock cycles.
* The activity. The powered gate netlist is simulated on the same schedule, dumped over the window only, and its
  `sig` must follow the RTL's cycle for cycle there (witnessed as scripts/measure-silicon-power.py does). OpenSTA reads
  the window as scripts/measure-silicon-power.py `sta_vcd` writes it, with the build of scripts/opensta/. Every corner
  proves that `report_power` used every annotated activity (`activity_proof`).
* The attribution. The routed netlist keeps the candidate's cells under `u_dut/`. Every cell is put in one class:
  - `dut`: the candidate's own cells, resized in place or not;
  - `dut_buffers`: cells the flow inserted whose nets reach only the candidate's cells;
  - `clock`: the clock tree;
  - `harness`: the harness's own cells;
  - `harness_buffers`: cells the flow inserted whose nets reach only harness cells and ports;
  - `boundary`: cells the flow inserted between the two;
  - `physical`: taps, decaps, fill and antenna diodes.
  OpenSTA sums `instance_power` over each class at each corner, and the classes must sum to its `report_power`
  total. The candidate's energy per operation is its `dut` and `dut_buffers` power, plus the clock tree's power in
  the share of clock sinks that are the candidate's, times the window's length, divided by N. The whole block's
  energy per operation is printed beside it, never subtracted from it.

A K7 controller's power block is the controller alone, every port passed through (emit_qor_fold `wrap-pass`), so it
is driven from outside: by the K7 traffic gate's testbench (scripts/qor/run-qor-dram.py) carrying the sequential
stream of TRAFFIC_WORDS words, at the block's own clock, its model of the part behavioural beside it. The window runs
from the first request the controller accepted to the last word it delivered; the gate-level run must write the RTL
run's bus log line for line, every command, request and word at the same clock. The operation is a 64-bit word
moved, written or read, so the window holds twice the stream's words.

Each corner's figures are committed as an excerpt under benchmarks/silicon/qor_logs/power/<block>/. `verify` re-parses
them and re-derives every energy.
"""
import argparse
import importlib.util
import json
import math
import os
import re
import shutil
import subprocess
import sys
from collections import Counter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import qorlib  # noqa: E402

_spec = importlib.util.spec_from_file_location("msp", os.path.join(qorlib.ROOT, "scripts", "measure-silicon-power.py"))
msp = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(msp)
_spec = importlib.util.spec_from_file_location("dram", os.path.join(qorlib.ROOT, "scripts", "qor", "run-qor-dram.py"))
dram = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(dram)
TRAFFIC_STREAM = "sequential"
TRAFFIC_WORDS = 256

POWER_LEDGER = "benchmarks/silicon/qor_sky130_power.json"
HEADER = {
    "_generated_by": "scripts/qor/record-qor-power.py (never hand-edited; verify re-derives every energy from the excerpts)",
    "schema": "logos.qor.power/1",
    "provenance": "gds_vcd",
    "honesty": {
        "what": "Energy per operation of each candidate, measured on its power block: the candidate inside its "
                "functional harness, hardened whole on Sky130, simulated at gate level over a window of whole "
                "operations, with power read by OpenSTA from that simulation's activity on the routed netlist with "
                "its extracted parasitics.",
        "attribution": "The candidate's energy is its own cells' power and that of the buffers the flow inserted "
                       "inside it, plus the clock tree in the share of clock sinks that are its own. The harness, the "
                       "buffers between harness and candidate, and physical cells are reported beside it.",
        "tool": "OpenSTA as the pinned OpenLane image carries it, with the upstream fix that lets report_power use "
                "every annotated activity (scripts/opensta/).",
    },
}
CLASSES = ("dut", "dut_buffers", "clock", "harness", "harness_buffers", "boundary", "physical")
PHYSICAL = re.compile(r"__(tapvpwrvgnd|decap|fill|diode|fakediode)_")
TT = "tt_025C_1v80"
MIN_OPS, MIN_CYCLES, MIN_WARMUP = 64, 4000, 8
# The cycles one result may take before the RTL run is refused: emit_qor_harness's budget per result.
MAX_CYCLES_PER_OP = 20_000


def stage_row(block):
    for s in qorlib.load("sky130_stage", {"rows": []})["rows"]:
        if s["block"] == block and s["role"] == "power":
            return s
    sys.exit(f"{block}: no power block of that name in qor_sky130_stage.json")


def function_row(candidate):
    """The candidate's passing functional-gate row on the stimulus its power block runs (shaped)."""
    for r in qorlib.load("function", {"rows": []})["rows"]:
        if r["candidate"] == candidate and r["mode"] == "shaped" and r["pass"]:
            return r
    sys.exit(f"{candidate}: no passing functional-gate row on shaped stimulus")


def window_ops(first_result_cycle, steady):
    """(K, N): the results folded before the window opens, and the operations it holds."""
    in_flight = math.ceil(first_result_cycle / steady)
    return max(MIN_WARMUP, 2 * in_flight), max(MIN_OPS, math.ceil(MIN_CYCLES / steady))


def testbench(block, port_list, period, powered, body, vcd, dump):
    """The block driven as scripts/measure-silicon-power.py drives a self-driving block — `clk` at `period`, `rst` held
    five rising edges and released on a falling edge — with `body` (Verilog) added and `dump` (Verilog) placing the
    VCD."""
    ins = {n for d, _, n in port_list if d == "input"}
    if ins - {"clk", "rst"}:
        sys.exit(f"{block}: inputs {sorted(ins - {'clk', 'rst'})} have no stimulus; a power block drives itself")
    wires = "".join(f"  wire [{w - 1}:0] {n}_w;\n" for d, w, n in port_list if d == "output")
    conns = (["VPWR(VPWR)", "VGND(VGND)"] if powered else []) + \
        [f"{n}({n + '_w' if d == 'output' else n})" for d, _, n in port_list]
    return f"""`timescale 1ns/1ps
module tb;
  reg clk = 0, rst = 1;
  supply1 VPWR; supply0 VGND;
{wires}  {block} dut({", ".join("." + c for c in conns)});
  always #{period / 2.0:g} clk = ~clk;
{body}
  initial begin
    $dumpfile("{vcd}");
{dump}
  end
  initial begin
    repeat (5) @(posedge clk); @(negedge clk) rst = 0;
  end
endmodule
"""


def rtl_window(block, run, period, k, n, work):
    """The window, from the RTL the run synthesized: (start ns, end ns, cycles in it, the RTL's `sig` trajectory)."""
    port_list = msp.ports(f"{run}/results/final/verilog/gl/{block}.v", block)
    limit = MAX_CYCLES_PER_OP * (k + n)
    body = f"""  integer cycles = 0, c_a = -1, c_b = -1;
  realtime t_a = -1.0, t_b = -1.0;
  always @(posedge clk) if (!rst) begin
    cycles = cycles + 1;
    if (cycles > {limit}) begin $display("QOR-TIMEOUT ops %0d", dut.ops); $finish; end
  end
  always @(negedge clk) if (!rst) begin
    if (c_a < 0 && dut.ops >= {k}) begin c_a = cycles; t_a = $realtime + {period / 4.0:g}; end
    if (c_b < 0 && dut.ops >= {k + n}) begin
      c_b = cycles; t_b = $realtime + {period / 4.0:g};
      $display("QOR-WINDOW %0d %0d %0d %0d %0d", $rtoi(t_a * 1000.0), $rtoi(t_b * 1000.0), c_b - c_a, dut.ops, c_a);
      #{period / 2.0:g} $finish;
    end
  end"""
    tb = testbench(block, port_list, period, False, body, f"{work}/rtl.vcd", "    $dumpvars(1, dut);")
    open(f"{work}/tb_rtl.v", "w").write(tb)
    out = sim(msp.run_sources(run), f"{work}/tb_rtl.v", f"{work}/rtl.vvp", False)
    m = re.search(r"QOR-WINDOW (\d+) (\d+) (\d+) (\d+) (\d+)", out)
    if not m:
        sys.exit(f"{block}: the RTL never folded {k + n} results:\n{out[-2000:]}")
    t_a, t_b, cycles, ops = (int(x) for x in m.groups()[:4])
    if ops != k + n:
        sys.exit(f"{block}: the harness folded {ops} results at the window's end, not {k + n}")
    trajectory, _ = msp.vcd_signature(f"{work}/rtl.vcd", "sig")
    return t_a / 1000.0, t_b / 1000.0, cycles, trajectory


def sim(sources, tb, vvp, powered):
    subprocess.run(["iverilog", "-g2012", "-o", vvp] + (["-DUSE_POWER_PINS", "-DFUNCTIONAL", "-I", msp.VMODELS]
                   if powered else []) + [tb] + sources +
                   ([f"{msp.VMODELS}/primitives.v", f"{msp.VMODELS}/sky130_fd_sc_hd.v"] if powered else []),
                   check=True, capture_output=True)
    return subprocess.run(["vvp", vvp], capture_output=True, text=True, timeout=12 * 3600,
                          cwd=os.path.dirname(sources[0])).stdout


def gate_window(block, run, period, start, end, work):
    """The powered gate netlist simulated on the same schedule, dumped over [start, end] only. Returns the VCD."""
    net = f"{run}/results/final/verilog/gl/{block}.v"
    dump = f"    #{start:.3f} $dumpvars(0, dut);\n    #{end - start:.3f} begin\n" \
           f"      if (^sig_w === 1'bx) $display(\"SIG-X\"); else $display(\"SIG-OK\");\n      $finish;\n    end"
    tb = testbench(block, msp.ports(net, block), period, True, "", f"{work}/{block}.vcd", dump)
    open(f"{work}/tb.v", "w").write(tb)
    out = sim([net], f"{work}/tb.v", f"{work}/sim.vvp", True)
    if "SIG-OK" not in out:
        sys.exit(f"{block}: the gate-level window ends with an unknown signature:\n{out[-2000:]}")
    return f"{work}/{block}.vcd"


def topology(block, run, work):
    """Every cell of the routed netlist with its cell type and each pin's direction and net, and every port, as OpenSTA
    reads them: ({instance: (cell, [(pin, direction, net)])}, {port: (direction, net)}). A pin with no net (a tie cell's
    unused output, for which OpenSTA hands back a null net) reads with net ''. The read ends with END, so a read OpenSTA
    broke off is refused rather than taken for the netlist."""
    out = f"{work}/topology.tsv"
    tcl = [f"read_liberty {msp.LIBDIR}/sky130_fd_sc_hd__{TT}.lib",
           f"read_verilog {run}/results/final/verilog/gl/{block}.nl.v", f"link_design {block}",
           f"set f [open {out} w]",
           "foreach i [get_cells *] {",
           "  puts $f \"I\\t[get_full_name $i]\\t[get_property $i ref_name]\"",
           "  foreach p [get_pins -of_objects $i] {",
           "    if {[get_property $p direction] == {internal}} continue",
           "    set n [get_nets -quiet -of_objects $p]",
           "    if {![llength $n] || [catch {get_full_name $n} nn]} { set nn {} }",
           "    puts $f \"P\\t[get_full_name $i]\\t[get_property $p lib_pin_name]\\t[get_property $p direction]\\t$nn\"",
           "  }",
           "}",
           "foreach p [get_ports *] {",
           "  set n [get_nets -quiet [get_full_name $p]]",
           "  puts $f \"T\\t[get_full_name $p]\\t[get_property $p direction]\\t[expr {[llength $n] ? [get_full_name $n] : [get_full_name $p]}]\"",
           "}",
           "puts $f END",
           "close $f"]
    script = f"{work}/topology.tcl"
    open(script, "w").write("\n".join(tcl + ["exit"]) + "\n")
    msp.run_sta(script, f"{work}/topology.log")
    lines = open(out).read().split("\n") if os.path.exists(out) else []
    if "END" not in lines:
        sys.exit("%s: OpenSTA broke off its read of the routed netlist (%s/topology.log)" % (block, work))
    cells, ports = {}, {}
    for line in lines[:lines.index("END")]:
        f = line.split("\t")
        if f[0] == "I":
            cells[f[1]] = (f[2], [])
        elif f[0] == "P":
            cells[f[1]][1].append((f[2], f[3], f[4]))
        elif f[0] == "T":
            ports[f[1]] = (f[2], f[3])
    return cells, ports


def synthesized_top(run, block):
    """The cell names synthesis gave the harness itself (the top module of the run's synthesized netlist)."""
    text = open(f"{run}/results/synthesis/{block}.v").read()
    body = text[text.index(f"module {block}"):]
    body = body[:body.index("endmodule")]
    return {name.lstrip("\\").replace("\\", "") for name in
            re.findall(r"^\s*sky130_\w+\s+(\\\S+|[A-Za-z_][\w$\[\]]*)\s*\(", body, re.M)}


def classify(cells, ports, synthesized, dut_prefix="u_dut/", clock_port="clk"):
    """Every cell's class (CLASSES), and the clock tree's sinks: ({instance: class}, {"dut": n, "harness": n}).

    The candidate's cells are those under `dut_prefix`; the harness's are those synthesis gave the top module; every
    other cell is one the flow inserted (buffers, clock tree, ties it duplicated) or a physical cell. Inserted cells
    that share a net form one tree, and a tree is classed by what it reaches through its nets — its anchors, the
    candidate's cells, the harness's cells and the ports: the clock tree reaches the clock port; a tree that reaches
    only the candidate's cells is the candidate's; only harness cells and ports, the harness's; anything else is the
    boundary between them."""
    out = {}
    side = {}
    for name, (cell, _) in cells.items():
        if PHYSICAL.search(cell):
            out[name] = "physical"
        elif name.startswith(dut_prefix):
            out[name], side[name] = "dut", "dut"
        elif name.replace("\\", "") in synthesized:
            out[name], side[name] = "harness", "harness"
    inserted = [n for n in cells if n not in out]
    on_net = {}
    for name, (_, pins) in cells.items():
        for _, _, net in pins:
            if net:
                on_net.setdefault(net, []).append(name)
    parent = {n: n for n in inserted}

    def find(x):
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x
    for net, names in on_net.items():
        mine = [n for n in names if n in parent]
        for n in mine[1:]:
            parent[find(n)] = find(mine[0])
    anchors = {}
    port_nets = {}
    for p, (_, net) in ports.items():
        port_nets.setdefault(net, []).append(p)
    for n in inserted:
        root = find(n)
        a = anchors.setdefault(root, {"dut": set(), "harness": set(), "ports": set()})
        for _, _, net in cells[n][1]:
            if not net:
                continue
            for m in on_net.get(net, []):
                if m in side:
                    a[side[m]].add(m)
            a["ports"].update(port_nets.get(net, []))
    sinks = {"dut": 0, "harness": 0}
    for root, a in anchors.items():
        if clock_port in a["ports"]:
            kind = "clock"
            sinks["dut"] += len(a["dut"])
            sinks["harness"] += len(a["harness"])
        elif a["dut"] and not a["harness"] and not a["ports"]:
            kind = "dut_buffers"
        elif not a["dut"] and (a["harness"] or a["ports"]):
            kind = "harness_buffers"
        else:
            kind = "boundary"
        anchors[root]["kind"] = kind
    for n in inserted:
        out[n] = anchors[find(n)]["kind"]
    return out, sinks


def corner_power(block, run, lib, rc, window_vcd, classes_tsv, work, scope="tb/dut"):
    """One corner: the excerpt (report_power over the window, each class's power, the activity proof, the parasitics'
    annotation) and its parse."""
    tcl = [f"read_liberty {msp.LIBDIR}/sky130_fd_sc_hd__{lib}.lib",
           f"read_verilog {run}/results/final/verilog/gl/{block}.nl.v", f"link_design {block}",
           f"read_spef {run}/results/final/spef/multicorner/{block}.{rc}.spef",
           f"read_sdc {run}/results/final/sdc/{block}.sdc",
           'puts "LOGOS-PINS [llength [get_pins -hierarchical *]] [llength [get_ports *]]"',
           "report_parasitic_annotation"]
    tcl += msp.TRACE_ON + [f"read_power_activities -scope {scope} -vcd {window_vcd}"] + msp.WALK_ON
    tcl += ['puts "LOGOS-VCD"', "report_power -digits 6"] + msp.TRACE_OFF
    tcl += [f"set f [open {classes_tsv} r]",
            "while {[gets $f line] >= 0} { set l [split $line \"\\t\"]; set cls([lindex $l 1]) [lindex $l 0] }",
            "close $f",
            "set corner [sta::cmd_corner]",
            "foreach k {" + " ".join(CLASSES) + " unclassified} { set n($k) 0; set s($k) {0.0 0.0 0.0 0.0} }",
            "foreach i [get_cells *] {",
            "  set name [get_full_name $i]",
            "  set k [expr {[info exists cls($name)] ? $cls($name) : {unclassified}}]",
            "  set p [sta::instance_power $i $corner]",
            "  incr n($k)",
            "  set s($k) [list [expr {[lindex $s($k) 0] + [lindex $p 0]}] [expr {[lindex $s($k) 1] + [lindex $p 1]}]"
            " [expr {[lindex $s($k) 2] + [lindex $p 2]}] [expr {[lindex $s($k) 3] + [lindex $p 3]}]]",
            "}",
            "foreach k {" + " ".join(CLASSES) + " unclassified} {",
            "  puts \"LOGOS-CLASS $k $n($k) [format {%.9e %.9e %.9e %.9e} {*}$s($k)]\"",
            "}"]
    script = f"{work}/power_{lib}.tcl"
    open(script, "w").write("\n".join(tcl + ["exit"]) + "\n")
    text = msp.run_sta(script, f"{work}/sta_{lib}.log", timeout=6 * 3600)
    proof = msp.activity_proof(text)
    if proof["dropped"]:
        sys.exit(f"{block}: report_power at {lib} did not use the VCD's activity on {len(proof['dropped'])} pins "
                 f"(first {proof['dropped'][:5]}; see {work}/sta_{lib}.log)")
    keep = []
    for line in text.splitlines():
        if line.startswith(("LOGOS-", "Annotated ", "Found ")) and not line.startswith("LOGOS-HIER "):
            keep.append(line)
    vcd_part = text.split("LOGOS-VCD", 1)[-1]
    if not msp.parse_power(vcd_part):
        sys.exit(f"{block}: no report_power table at {lib} (see {work}/sta_{lib}.log)")
    table = [l for l in vcd_part.splitlines() if re.match(r"\s*(Group|Sequential|Combinational|Clock|Macro|Pad|Total)\b", l)]
    keep += ["LOGOS-TABLE " + l for l in table]
    keep.append("LOGOS-PROOF annotated %d set %d seeded %s hierarchical %d clock %d dropped 0"
                % (proof["annotated"], proof["set"], ",".join(proof["seeded"]) or "-", proof["hierarchical"],
                   proof["clock"]))
    return "\n".join(keep) + "\n"


def parse_excerpt(text):
    """A corner's figures from its excerpt: {total: {...}, classes: {class: {count, internal, switching, leakage,
    total}}, annotated_pins, design_pins, proof, unannotated_drivers}. The proof counts the pins the activity walk
    seeded; their names stay in the excerpt."""
    out = {"classes": {}}
    for line in text.splitlines():
        f = line.split()
        if f[0] == "LOGOS-CLASS":
            out["classes"][f[1]] = dict(zip(("count", "internal", "switching", "leakage", "total"),
                                            [int(f[2])] + [float(x) for x in f[3:7]]))
        elif f[0] == "LOGOS-TABLE" and len(f) >= 6 and f[1] == "Total":
            out["total"] = dict(zip(("internal", "switching", "leakage", "total"), map(float, f[2:6])))
        elif f[0] == "LOGOS-PINS":
            out["design_pins"] = int(f[1]) + int(f[2])
        elif f[0] == "Annotated":
            out["annotated_pins"] = int(f[1])
        elif f[0] == "LOGOS-PROOF":
            out["proof"] = {"annotated": int(f[2]), "set": int(f[4]), "seeded": len(f[6].split(",")) if f[6] != "-" else 0,
                            "hierarchical": int(f[8]), "clock": int(f[10]), "dropped": int(f[12])}
        elif f[0] == "Found" and len(f) >= 4 and f[2] == "unannotated":
            out["unannotated_drivers"] = int(f[1])
        elif f[0] == "Found" and len(f) >= 5 and f[2:4] == ["partially", "unannotated"]:
            out["partially_unannotated_drivers"] = int(f[1])
    return out


def energies(figures, span_ns, ops, sinks):
    """Energy per operation, in pJ, from one corner's figures: the candidate's, each class's, the whole block's."""
    per = span_ns * 1e3 / ops
    cls = {k: v["total"] for k, v in figures["classes"].items()}
    share = sinks["dut"] / (sinks["dut"] + sinks["harness"]) if sinks["dut"] + sinks["harness"] else 0.0
    dut_w = cls["dut"] + cls["dut_buffers"] + share * cls["clock"]
    return {"dut_pj": round(dut_w * per, 6), "block_pj": round(figures["total"]["total"] * per, 6),
            "class_pj": {k: round(v * per, 6) for k, v in cls.items() if k != "unclassified"},
            "clock_share": round(share, 6)}


def check(figures, where):
    """The classes cover every cell and sum to the report's total."""
    problems = []
    if figures["classes"].get("unclassified", {}).get("count"):
        problems.append(f"{where}: {figures['classes']['unclassified']['count']} cells in no class")
    s = sum(v["total"] for v in figures["classes"].values())
    if abs(s - figures["total"]["total"]) > 1e-5 * figures["total"]["total"] + 1e-12:
        problems.append(f"{where}: the classes sum to {s:.6e} W, the report to {figures['total']['total']:.6e} W")
    if figures["proof"]["dropped"]:
        problems.append(f"{where}: annotated activity dropped")
    return problems


def traffic_window(work, counter, inst, period):
    """The RTL run in `work` gives the window as testbench clocks: the clock at which the controller accepted its first
    request and the one at which it delivered its last word (DRAM clocks in the log; the testbench's own clock is the
    DRAM clock's half for a 1:2 controller). Returns (first, last, the DRAM clocks per testbench clock)."""
    log = open(os.path.join(work, "dram.log")).read().splitlines()
    first = min(int(l.split()[1]) for l in log if l.startswith("Q "))
    last = max(int(l.split()[1]) for l in log if l.startswith("D "))
    per = 2 if counter == "k" else 1
    return first // per, last // per, per


def traffic_testbench(text, top, inst, block, period, counter, first, last):
    """The traffic testbench `text` at the block's clock, its controller `top` instance `inst` replaced by the powered
    block, and the window dumped: from the testbench clock `first` to `last`, its times printed."""
    if top == "litedram_ctrl":
        old = f"litedram_ctrl {inst}(.sys_clk(clk), .sys_rst(rst),"
        new = f"{block} {inst}(.VPWR(VPWR), .VGND(VGND), .clk(clk), .rst(rst),"
    else:
        old = f"ddr2_ctrl {inst}(.clk(clk), .rst(rst),"
        new = f"{block} {inst}(.VPWR(VPWR), .VGND(VGND), .clk(clk), .rst(rst),"
    clock = re.search(r"always #(\S+) clk = ~clk;", text)
    if text.count(old) != 1 or not clock:
        sys.exit(f"{block}: the traffic testbench does not instantiate {top} as the power path expects")
    text = text.replace(old, new).replace(clock.group(0), f"always #{period / 2.0:g} clk = ~clk;")
    dump = f"""  supply1 VPWR; supply0 VGND;
  always @(negedge clk) if (!rst) begin
    if ({counter} == {first}) begin
      $dumpfile("window.vcd"); $dumpvars(0, tb_qor_dram.{inst}); $display("QOR-WINDOW-START %0d", $rtoi($realtime * 1000.0));
    end
    if ({counter} == {last}) begin $dumpoff; $display("QOR-WINDOW-END %0d", $rtoi($realtime * 1000.0)); end
  end
endmodule
"""
    head, tail = text.rsplit("endmodule", 1)
    return head + dump + tail


def measure_traffic(block, stage, rec, run, period):
    """A K7 controller's energy per word moved, from its power block driven by the K7 traffic testbench."""
    work = os.path.join(qorlib.LOGS, "qor", "power", block)
    shutil.rmtree(work, ignore_errors=True)
    rtl_dir, gate_dir = os.path.join(work, "rtl"), os.path.join(work, "gate")
    os.makedirs(rtl_dir)
    staged, rtl_sha3, sources, (top, inst), counter = dram.prepare(rtl_dir, stage["candidate"], TRAFFIC_STREAM, TRAFFIC_WORDS)
    if rtl_sha3 != stage["rtl_sha3_256"]:
        sys.exit(f"{block}: the traffic gate stages another RTL than the block was hardened from")
    verdict = dram.run_sim(rtl_dir, sources)
    if "PASS" not in verdict:
        sys.exit(f"{block}: the RTL traffic run did not pass:\n{verdict}")
    first, last, per = traffic_window(rtl_dir, counter, inst, period)
    shutil.copytree(rtl_dir, gate_dir)
    tb = open(os.path.join(gate_dir, "tb_qor_dram.v")).read()
    open(os.path.join(gate_dir, "tb_qor_dram.v"), "w").write(traffic_testbench(tb, top, inst, block, period, counter, first, last))
    net = f"{run}/results/final/verilog/gl/{block}.v"
    models = [s for s in sources if s not in ("tb_qor_dram.v", top + ".v")]
    subprocess.run(["iverilog", "-g2012", "-DUSE_POWER_PINS", "-DFUNCTIONAL", "-I", msp.VMODELS, "-s", "tb_qor_dram",
                    "-o", "sim.vvp", "tb_qor_dram.v", net] + models +
                   [f"{msp.VMODELS}/primitives.v", f"{msp.VMODELS}/sky130_fd_sc_hd.v"], cwd=gate_dir, check=True,
                   capture_output=True)
    out = subprocess.run(["vvp", "sim.vvp"], cwd=gate_dir, capture_output=True, text=True, timeout=12 * 3600).stdout
    if open(os.path.join(gate_dir, "dram.log")).read() != open(os.path.join(rtl_dir, "dram.log")).read():
        sys.exit(f"{block}: the gate-level run's bus log differs from the RTL run's (see {gate_dir}/dram.log)")
    a = re.search(r"QOR-WINDOW-START (\d+)", out)
    b = re.search(r"QOR-WINDOW-END (\d+)", out)
    if not (a and b):
        sys.exit(f"{block}: the gate-level run never opened and closed its window:\n{out[-2000:]}")
    start, end = int(a.group(1)) / 1000.0, int(b.group(1)) / 1000.0
    cycles, ops = last - first, 2 * TRAFFIC_WORDS
    cells, ports = topology(block, run, work)
    classes, sinks = classify(cells, ports, synthesized_top(run, block))
    counts = Counter(classes.values())
    if not counts["dut"]:
        sys.exit(f"{block}: no cell under u_dut/ — the controller did not survive synthesis whole")
    classes_tsv = f"{work}/classes.tsv"
    with open(classes_tsv, "w") as f:
        for name, k_ in sorted(classes.items()):
            f.write(f"{k_}\t{name}\n")
    vcd = os.path.join(gate_dir, "window.vcd")
    window_vcd = msp.sta_vcd(vcd, f"{work}/window.vcd", (start, end))
    log_sha3 = qorlib.sha3_file(os.path.join(rtl_dir, "dram.log"))
    window_line = (f"LOGOS-WINDOW start_ns {start:.3f} end_ns {end:.3f} cycles {cycles} ops {ops} warmup_ops 0 "
                   f"period_ns {period:g}\n")
    sinks_line = f"LOGOS-SINKS dut {sinks['dut']} harness {sinks['harness']}\n"
    traffic_line = (f"LOGOS-TRAFFIC stream {TRAFFIC_STREAM} words {TRAFFIC_WORDS} dram_clocks_per_clock {per} "
                    f"log_sha3 {log_sha3}\n")
    excerpts, corners = [], {}
    for lib, rc in msp.CORNERS:
        text = window_line + sinks_line + traffic_line + corner_power(block, run, lib, rc, window_vcd, classes_tsv, work,
                                                                      scope=f"tb_qor_dram/{inst}")
        figures = parse_excerpt(text)
        problems = check(figures, f"{block} {lib}")
        if problems:
            sys.exit("\n".join(problems))
        excerpts.append((f"{lib}.txt", text))
        corners[lib] = dict(figures, energy=energies(figures, end - start, ops, sinks))
    paths, digests = qorlib.excerpt_record(f"power/{block}", excerpts)
    row = {"key": block, "block": block, "kernel": stage["kernel"], "candidate": stage["candidate"],
           "source_kind": stage["source_kind"], "period_ns": period, "run": qorlib.rel(run),
           "gds_bytes": rec["gds_bytes"], "signoff_clean": rec["signoff_clean"],
           "setup_worst_slack_ns": rec["setup_worst_slack_ns"],
           "window": {"start_ns": start, "end_ns": end, "span_ns": round(end - start, 6), "cycles": cycles, "ops": ops,
                      "warmup_ops": 0, "cycles_per_op": round(cycles / ops, 6)},
           "traffic": {"stream": TRAFFIC_STREAM, "words": TRAFFIC_WORDS, "dram_clocks_per_clock": per,
                       "log_sha3_256": log_sha3, "rtl_sha3_256": rtl_sha3},
           "cells": dict(sorted(counts.items())), "clock_sinks": sinks, "corners": corners,
           "excerpts": paths, "excerpt_sha3": digests, "opensta": msp.opensta(), "recorded_at": qorlib.now(),
           "reproduce": "python3 scripts/qor/record-qor-power.py run %s" % block}
    qorlib.update("power", HEADER, row, (f"power/{block}", excerpts))
    for big in {vcd, window_vcd}:
        os.remove(big)
    tt = corners[TT]["energy"]
    print(f"{block}: {tt['dut_pj']:.4f} pJ/word candidate, {tt['block_pj']:.4f} pJ/word block (tt, "
          f"{ops} words moved over {cycles} clocks)", flush=True)


def measure(block):
    stage = stage_row(block)
    ledger, rec, run_root = msp.ledger_record(block)
    if ledger != POWER_LEDGER:
        sys.exit(f"{block}: recorded in {ledger}, not {POWER_LEDGER}")
    if rec["signoff_clean"] is not True:
        sys.exit(f"{block}: its run is not signoff-clean; energy is measured at a clock the block closes at")
    run = msp.shipped_run(block, rec["gds_bytes"], run_root)
    period = float(rec["clock_period_ns"])
    if stage["kernel"] == "K7":
        return measure_traffic(block, stage, rec, run, period)
    fn = function_row(stage["candidate"])
    k, n = window_ops(fn["first_result_cycle"], fn["steady_cycles_per_op"])
    work = os.path.join(qorlib.LOGS, "qor", "power", block)
    os.makedirs(work, exist_ok=True)
    start, end, cycles, rtl = rtl_window(block, run, period, k, n, work)
    vcd = gate_window(block, run, period, start, end, work)
    gate, width = msp.vcd_signature(vcd, "sig")
    shared = msp.witnessed(gate, [(t, v) for t, v in rtl if t <= round(end * 1000)], width)
    if shared is None:
        sys.exit(f"{block}: the gate-level sig does not follow its RTL's over the window")
    signature, changes, trajectory = shared
    if changes == 0:
        sys.exit(f"{block}: the signature never moves in the window; it witnesses nothing")
    cells, ports = topology(block, run, work)
    classes, sinks = classify(cells, ports, synthesized_top(run, block))
    counts = Counter(classes.values())
    if not counts["dut"]:
        sys.exit(f"{block}: no cell under u_dut/ — the candidate did not survive synthesis whole")
    classes_tsv = f"{work}/classes.tsv"
    with open(classes_tsv, "w") as f:
        for name, k_ in sorted(classes.items()):
            f.write(f"{k_}\t{name}\n")
    window_vcd = msp.sta_vcd(vcd, f"{work}/window.vcd", (start, end))
    excerpts, corners = [], {}
    window_line = (f"LOGOS-WINDOW start_ns {start:.3f} end_ns {end:.3f} cycles {cycles} ops {n} warmup_ops {k} "
                   f"period_ns {period:g}\n")
    sinks_line = f"LOGOS-SINKS dut {sinks['dut']} harness {sinks['harness']}\n"
    for lib, rc in msp.CORNERS:
        text = window_line + sinks_line + corner_power(block, run, lib, rc, window_vcd, classes_tsv, work)
        figures = parse_excerpt(text)
        problems = check(figures, f"{block} {lib}")
        if problems:
            sys.exit("\n".join(problems))
        excerpts.append((f"{lib}.txt", text))
        corners[lib] = dict(figures, energy=energies(figures, end - start, n, sinks))
    paths, digests = qorlib.excerpt_record(f"power/{block}", excerpts)
    row = {"key": block, "block": block, "kernel": stage["kernel"], "candidate": stage["candidate"],
           "source_kind": stage["source_kind"], "period_ns": period, "run": qorlib.rel(run),
           "gds_bytes": rec["gds_bytes"], "signoff_clean": rec["signoff_clean"],
           "setup_worst_slack_ns": rec["setup_worst_slack_ns"],
           "window": {"start_ns": start, "end_ns": end, "span_ns": round(end - start, 6), "cycles": cycles, "ops": n,
                      "warmup_ops": k, "cycles_per_op": round(cycles / n, 6)},
           "witness": {"signature": signature, "changes": changes, "trajectory_sha256": trajectory},
           "cells": dict(sorted(counts.items())), "clock_sinks": sinks, "corners": corners,
           "excerpts": paths, "excerpt_sha3": digests, "opensta": msp.opensta(), "recorded_at": qorlib.now(),
           "reproduce": "python3 scripts/qor/record-qor-power.py run %s" % block}
    qorlib.update("power", HEADER, row, (f"power/{block}", excerpts))
    for big in {vcd, window_vcd}:
        os.remove(big)
    tt = corners[TT]["energy"]
    print(f"{block}: {tt['dut_pj']:.4f} pJ/op candidate, {tt['block_pj']:.4f} pJ/op block (tt, "
          f"{n} ops over {cycles} cycles)", flush=True)


def verify():
    bad = []
    rows = qorlib.load("power", {"rows": []})["rows"]
    for row in rows:
        texts, problems = qorlib.read_excerpts(row)
        bad += problems
        for lib, _ in msp.CORNERS:
            text = texts.get(f"{lib}.txt")
            if text is None:
                continue
            figures = parse_excerpt(text)
            bad += check(figures, f"{row['key']} {lib}")
            w = re.search(r"LOGOS-WINDOW start_ns (\S+) end_ns (\S+) cycles (\d+) ops (\d+)", text)
            sk = re.search(r"LOGOS-SINKS dut (\d+) harness (\d+)", text)
            span, ops = float(w.group(2)) - float(w.group(1)), int(w.group(4))
            again = energies(figures, span, ops, {"dut": int(sk.group(1)), "harness": int(sk.group(2))})
            if again != row["corners"][lib]["energy"]:
                bad.append(f"{row['key']} {lib}: the energies differ from what the excerpt gives")
    for b in bad:
        print("FAIL: " + b, file=sys.stderr)
    if bad:
        sys.exit(1)
    print("verified %d power rows" % len(rows))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("mode", choices=["run", "all", "verify"])
    ap.add_argument("block", nargs="?")
    a = ap.parse_args()
    if a.mode == "verify":
        return verify()
    if a.mode == "run":
        return measure(a.block or ap.error("name the power block"))
    done = {r["block"] for r in qorlib.load("power", {"rows": []})["rows"]}
    led = os.path.join(qorlib.ROOT, POWER_LEDGER)
    blocks = (json.load(open(led)).get("blocks") or {}) if os.path.exists(led) else {}
    for b in sorted(blocks):
        if b in done:
            continue
        if blocks[b].get("signoff_clean") is not True:
            print(f"{b}: its run is not signoff-clean; passed over (the runner ran the block again at a clock it "
                  f"closes at)", flush=True)
            continue
        measure(b)


if __name__ == "__main__":
    main()
