#!/usr/bin/env python3
"""Signoff power for one hardened block, measured on the netlist its ledger SHIPS.

    python3 scripts/measure-silicon-power.py <block> [--no-vcd] [--cycles N]

The run is the one whose GDS is byte-for-byte the ledger's `gds_bytes` — never simply the newest
run directory, which may be a later, failed attempt. The clock is the ledger's `clock_period_ns`,
the period every timing corner meets; the gate-level simulation toggles at exactly that period, so
a VCD-annotated power and the clock it is divided by are the same clock.

For each PVT corner — ss_100C_1v60 with max RC, tt_025C_1v80 with nominal RC, ff_n40C_1v95 with min
RC, each corner's library with that corner's SPEF — OpenSTA `report_power` is taken twice: with the
tool's default switching activity, and with the activity measured by the gate-level simulation of
the powered netlist (a measured block drives itself: `clk` and `rst` in, its outputs out; the witness
follows a BIST block's `sig` signature, or the block's widest output).

OpenSTA is the build of scripts/opensta/: the pinned OpenLane image's, with the upstream fix without which
`report_power` uses an annotated activity only where its activity walk from the primary inputs happens to pass, and
every other pin falls back to the default input activity — on a block whose only data input is a reset, nearly
all of them. Each VCD-annotated report proves its own activity: every pin the VCD annotated is one the walk set
from the VCD, a seeded input, or a pin on the clock network carrying the clock's own activity, which is what the
tool gives a clock pin. A report that cannot show that is refused.

The gate-level window is witnessed at RTL: the sources the run synthesized are simulated over the same
window, and the signature each leaves in its VCD must be the same one, so the measured activity is the
design's. Where a block's RTL names its workload (PROBES), that simulation also counts it — the decision
mix of the stimulus the power was measured under.

Prints one JSON object on stdout; the simulation, VCD and STA scripts land under logs/power/<block>/.
"""
import glob
import hashlib
import json
import os
import re
import subprocess
import sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
PDK = os.path.expanduser("~/.ciel/sky130A")
LIBDIR = f"{PDK}/libs.ref/sky130_fd_sc_hd/lib"
VMODELS = f"{PDK}/libs.ref/sky130_fd_sc_hd/verilog"
CORNERS = (("ss_100C_1v60", "max"), ("tt_025C_1v80", "nom"), ("ff_n40C_1v95", "min"))


def ledger_record(block):
    """The block's record in whichever registered block ledger holds it (benchmarks/silicon/sky130_ledgers.json),
    with where that ledger's runs are."""
    for led in json.load(open(f"{ROOT}/benchmarks/silicon/sky130_ledgers.json"))["ledgers"]:
        path = os.path.join(ROOT, led["path"])
        b = (json.load(open(path)).get("blocks") or {}).get(block) if os.path.exists(path) else None
        if b is not None:
            return led["path"], b, led.get("run_root", "tools/OpenLane/designs")
    sys.exit(f"{block}: in no registered block ledger")


def shipped_run(block, gds_bytes, run_root):
    for g in sorted(glob.glob(f"{ROOT}/{run_root}/*/runs/*/results/final/gds/{block}.gds"),
                    key=os.path.getmtime, reverse=True):
        if os.path.getsize(g) == gds_bytes:
            return g.split("/results/")[0]
    sys.exit(f"{block}: no run directory holds the {gds_bytes}-byte GDS its ledger ships")


# The workload a block's RTL names, counted once per decision: at each rising edge of the measured
# window, the value that edge latches (read before its nonblocking updates land). `state` declares
# what `sample` needs; every name in `counters` is an integer the probe increments.
PROBES = {
    "decision_hft_realbist": {
        "state": "reg [63:0] prev; reg seen = 0;",
        "sample": ("if ($signed(dut.y) > 0) buy = buy + 1; "
                   "else if ($signed(dut.y) < 0) sell = sell + 1; "
                   "else if (dut.u_dec.v6 != 0) risk_blocked = risk_blocked + 1; "
                   "else no_trade = no_trade + 1; "
                   "if (seen && dut.y != prev) decision_changes = decision_changes + 1; "
                   "prev = dut.y; seen = 1;"),
        "counters": ("buy", "sell", "risk_blocked", "no_trade", "decision_changes"),
    },
    # The CPU twin shows signed_order * 4 + order_side once per decision (decision_workload).
    "soft_core_realbist": {
        "state": "reg signed [31:0] v;",
        "sample": ("if (dut.out_valid) begin v = $signed(dut.out); decisions = decisions + 1; "
                   "if (v >= 4) buy = buy + 1; else if (v < 0) sell = sell + 1; "
                   "else if (v != 0) risk_blocked = risk_blocked + 1; else no_trade = no_trade + 1; end"),
        "counters": ("decisions", "buy", "sell", "risk_blocked", "no_trade"),
    },
}


NO_PROBE = {"state": "", "sample": "", "counters": ()}


def run_sources(run):
    """The RTL the run synthesized — its config's VERILOG_FILES, read from the host side of the mount: the
    container's /openlane/ is the OpenLane tree whose designs/ holds the run."""
    m = re.search(r'set ::env\(VERILOG_FILES\) "([^"]*)"', open(f"{run}/config.tcl").read())
    if not m:
        sys.exit(f"{run}: config.tcl names no VERILOG_FILES")
    tree = run.rsplit("/designs/", 1)[0]
    return [f.replace("/openlane/", f"{tree}/", 1) for f in m.group(1).split()]


def vcd_signature(vcd, name):
    """The trajectory of the DUT's top-scope signal `name` in `vcd`: the value it holds at the end of
    every time step where it moved, from its first defined value on. A zero-delay gate netlist may
    pass through intermediate values inside one time step that the RTL never shows; the settled
    value at each step is what the two must share. Returns (final value as hex, number of settled
    moves, sha256 of the whole trajectory). A signature that never moves witnesses nothing."""
    ident, width, scope = None, 0, []
    settled, now = {}, 0
    with open(vcd) as f:
        for line in f:
            if line.startswith("$enddefinitions"):
                break
            t = line.split()
            if t[:1] == ["$scope"]:
                scope.append(t[2])
            elif t[:1] == ["$upscope"]:
                scope.pop()
            elif t[:1] == ["$var"] and scope == ["tb", "dut"] and t[4] == name:
                ident, width = t[3], int(t[2])
        if ident is None:
            sys.exit(f"{vcd}: no dut.{name}")
        for line in f:
            c = line[:1]
            if c == "#":
                now = int(line[1:])
            elif width == 1:
                if c in "01xzXZ" and line[1:].strip() == ident:
                    settled[now] = c
            elif c in "bB":
                bits, _, sid = line[1:].strip().partition(" ")
                if sid == ident:
                    settled[now] = bits
    trajectory = []
    for t in sorted(settled):
        v = settled[t]
        if not trajectory and any(ch in v for ch in "xzXZ"):
            continue
        if not trajectory or trajectory[-1][1] != v:
            trajectory.append((t, v))
    if not trajectory:
        sys.exit(f"{vcd}: dut.{name} never takes a defined value")
    return trajectory, width


def witnessed(gate, rtl, width):
    """The trajectory the gate netlist and its RTL share, or None if they part. Silicon powers up
    undefined where simulation `initial` blocks give the RTL values, so the two are compared from the
    gate's first defined step: there the RTL's held value must already agree, and every settled move
    after it must be the same move at the same time. Returns (final value as hex, settled moves after
    that first step, sha256 of the shared trajectory)."""
    t0 = gate[0][0]
    held = [v for t, v in rtl if t <= t0]
    if not held or held[-1] != gate[0][1]:
        return None
    if [(t, v) for t, v in rtl if t > t0] != gate[1:]:
        return None
    last = gate[-1][1]
    final = last if any(ch in last for ch in "xzXZ") else format(int(last, 2), f"0{(width + 3) // 4}x")
    return final, len(gate) - 1, hashlib.sha256(";".join(f"{t}:{v}" for t, v in gate).encode()).hexdigest()


def parse_power(text):
    """Every `report_power` table in `text`, in order: {group: {internal, switching, leakage, total}}, watts."""
    tables, cur = [], None
    for line in text.splitlines():
        f = line.split()
        if f[:1] == ["Group"]:
            cur = {}
            tables.append(cur)
        elif cur is not None and len(f) >= 5 and f[0] in ("Sequential", "Combinational", "Clock", "Macro", "Pad", "Total"):
            try:
                cur[f[0].lower()] = dict(zip(("internal", "switching", "leakage", "total"), map(float, f[1:5])))
            except ValueError:
                pass
            if f[0] == "Total":
                cur = None
    return tables


def ports(netlist, block):
    """The block's signal ports from its gate netlist, in order: [(direction, width, name)], power
    pins excluded."""
    text = open(netlist).read()
    body = text[text.index(f"module {block}"):]
    body = body[:body.index("endmodule")]
    out = []
    for d, msb, lsb, name in re.findall(r"^\s*(input|output)\s*(?:\[(\d+):(\d+)\])?\s*(\w+)\s*;", body, re.M):
        if name not in ("VPWR", "VGND", "VPB", "VNB"):
            out.append((d, abs(int(msb) - int(lsb)) + 1 if msb else 1, name))
    return out


def signature_port(block, port_list):
    """The output the witness follows: a BIST block's `sig`, else the block's widest output."""
    outs = [(w, n) for d, w, n in port_list if d == "output"]
    if not outs:
        sys.exit(f"{block}: no output to witness")
    return "sig" if any(n == "sig" for _, n in outs) else max(outs)[1]


def testbench(block, port_list, period, cycles, vcd, powered, probe):
    """One window, for the gate netlist (`powered`) and for its RTL alike: `clk` at the signoff period,
    `rst` held five rising edges and released on a falling edge, `cycles` rising edges, closed on a
    falling edge — never in the time step of a rising one, so which edges run the design is fixed
    rather than an accident of event order."""
    sig = signature_port(block, port_list)
    ins = {n for d, _, n in port_list if d == "input"}
    if ins - {"clk", "rst"}:
        sys.exit(f"{block}: inputs {sorted(ins - {'clk', 'rst'})} have no stimulus; only self-driving blocks are measured")
    wires = "".join(f"  wire [{w - 1}:0] {n}_w;\n" for d, w, n in port_list if d == "output")
    conns = ", ".join((["VPWR(VPWR)", "VGND(VGND)"] if powered else []) +
                      [f"{n}({n + '_w' if d == 'output' else n})" for d, _, n in port_list])
    counters = probe["counters"]
    return f"""`timescale 1ns/1ps
module tb;
  reg clk = 0, rst = 1;
  supply1 VPWR; supply0 VGND;
{wires}  {block} dut({", ".join("." + c for c in conns.split(", "))});
  always #{period / 2.0:g} clk = ~clk;
  integer i;
  {"".join(f"integer {c} = 0; " for c in counters)}
  {probe["state"]}
  always @(posedge clk) if (!rst) begin {probe["sample"]} end
  initial begin
    $dumpfile("{vcd}"); $dumpvars({0 if powered else 1}, dut);
    repeat (5) @(posedge clk); @(negedge clk) rst = 0;
    for (i = 0; i < {cycles}; i = i + 1) @(posedge clk);
    @(negedge clk);
    if (^{sig}_w === 1'bx) $display("SIG-X"); else $display("SIG-OK");
    $display("LOGOS-PROBE{"".join(f" {c}=%0d" for c in counters)}"{"".join(f", {c}" for c in counters)});
    $finish;
  end
endmodule
"""


def run_sim(block, sources, tb, vvp, powered):
    """Compile and run one window. The simulation runs where the sources live, so a file the RTL reads
    by relative path (a `$readmemh` boot ROM) is the one synthesis read."""
    subprocess.run(["iverilog", "-g2012", "-o", vvp] + (["-DUSE_POWER_PINS", "-DFUNCTIONAL", "-I", VMODELS] if powered else [])
                   + [tb] + sources + ([f"{VMODELS}/primitives.v", f"{VMODELS}/sky130_fd_sc_hd.v"] if powered else []),
                   check=True, capture_output=True)
    # A 141k-cell array toggling every register each cycle simulates for over an hour at 4000 cycles.
    out = subprocess.run(["vvp", vvp], capture_output=True, text=True, timeout=6 * 3600,
                         cwd=os.path.dirname(sources[0])).stdout
    if "LOGOS-PROBE" not in out:
        sys.exit(f"{block}: the {'gate' if powered else 'RTL'} simulation did not finish:\n{out[-2000:]}")
    return out


def simulate(block, run, period, cycles, work):
    """Gate-level simulation of the powered netlist at the signoff clock -> VCD. Returns (vcd, sig_x)."""
    net = f"{run}/results/final/verilog/gl/{block}.v"
    tb = testbench(block, ports(net, block), period, cycles, f"{work}/{block}.vcd", True, NO_PROBE)
    open(f"{work}/tb.v", "w").write(tb)
    out = run_sim(block, [net], f"{work}/tb.v", f"{work}/sim.vvp", True)
    return f"{work}/{block}.vcd", "SIG-X" in out


def rtl_witness(block, run, period, cycles, work):
    """The same window simulated on the RTL the run synthesized: its signature trajectory, and the
    workload PROBES counts for the block."""
    port_list = ports(f"{run}/results/final/verilog/gl/{block}.v", block)
    tb = testbench(block, port_list, period, cycles, f"{work}/rtl.vcd", False, PROBES.get(block, NO_PROBE))
    open(f"{work}/tb_rtl.v", "w").write(tb)
    out = run_sim(block, run_sources(run), f"{work}/tb_rtl.v", f"{work}/rtl.vvp", False)
    counts = {k: int(v) for k, v in re.findall(r"(\w+)=(\d+)", re.search(r"LOGOS-PROBE(.*)", out).group(1))}
    trajectory, _ = vcd_signature(f"{work}/rtl.vcd", signature_port(block, port_list))
    return {"sources": [os.path.relpath(f, ROOT) for f in run_sources(run)], "cycles": cycles,
            "signature_port": signature_port(block, port_list), "trajectory": trajectory, **counts}


def vcd_header(f):
    """The definitions of the open VCD `f`, through `$enddefinitions`: (their lines, the nanoseconds one time unit is,
    or None without a `$timescale`)."""
    lines = []
    for line in f:
        lines.append(line)
        if line.startswith("$enddefinitions"):
            break
    m = re.search(r"\$timescale\s+(\d+)\s*(s|ms|us|ns|ps|fs)\s+\$end", "".join(lines))
    units = {"s": 1e9, "ms": 1e6, "us": 1e3, "ns": 1.0, "ps": 1e-3, "fs": 1e-6}
    return lines, int(m.group(1)) * units[m.group(2)] if m else None


def sta_vcd(vcd, out, window_ns=None):
    """`vcd` as OpenSTA's `read_power_activities` reads it: written to `out` and returned, or `vcd` itself when it
    already is.

    OpenSTA finds a pin through the scope of its instance, splitting the scope path at `/`. A flat netlist whose
    instance names carry the divider (a kept hierarchy: `u_dut/_123_`) would have every such scope read as a hierarchy
    the netlist does not have, and its pins would fall back to propagated activity. So those scopes are written as the
    netlist reader names the instance: the Verilog escape mark dropped, the divider escaped (`u_dut\\/_123_`).

    Such a netlist's nets keep escaped names (`\\u_dut/u_k.zeta [15:0]`). A net is no pin, so OpenSTA annotates nothing
    from one, and its reader leaves the bit range of an escaped vector unset and then walks it, which never ends under
    some builds. So every variable with an escaped name is left out, and so are its value changes unless another
    variable shares its identifier.

    With `window_ns` = (start, end), only that window, in ns. The values held at `start` become the initial dump at
    time 0, every later change moves back by `start`, and the file ends with a time step at `end`. OpenSTA reads
    activity over [0, last time step], so the activity it reads is the window's. A window the simulation did not
    reach is refused."""
    scope = re.compile(r"(\s*\$scope\s+\S+\s+)\\?(\S*(?<!\\)/\S*)(\s.*)?$")
    var = re.compile(r"\s*\$var\s+\S+\s+\d+\s+(\S+)\s+(\S+)")
    with open(vcd) as f:
        header, unit = vcd_header(f)
        rewritten, kept, dropped = [], set(), set()
        for line in header:
            m = scope.match(line.rstrip("\n"))
            if m:
                line = m.group(1) + re.sub(r"(?<!\\)/", r"\\/", m.group(2)) + (m.group(3) or "") + "\n"
            v = var.match(line)
            if v and v.group(2).startswith("\\"):
                dropped.add(v.group(1))
                continue
            if v:
                kept.add(v.group(1))
            rewritten.append(line)
        dropped -= kept
        if rewritten == header and window_ns is None:
            return vcd
        if window_ns is not None and unit is None:
            sys.exit(f"{vcd}: no $timescale to place the window by")

        def ident(line):
            return line[1:].strip() if line[:1] in "01xzXZ" else line.split()[-1]

        with open(out, "w") as o:
            o.writelines(rewritten)
            if window_ns is None:
                for line in f:
                    c = line[:1]
                    if c in "#$" or not line.strip() or ident(line) not in dropped:
                        o.write(line)
                return out
            start, end = (round(t / unit) for t in window_ns)
            held, now, opened, stamped = {}, 0, False, None
            for line in f:
                c = line[:1]
                if c == "#":
                    now = int(line[1:])
                    if now > start and not opened:
                        o.write("#0\n$dumpvars\n")
                        o.writelines(held.values())
                        o.write("$end\n")
                        opened, stamped = True, 0
                    if now > end:
                        break
                    if opened and now - start != stamped:
                        stamped = now - start
                        o.write("#%d\n" % stamped)
                elif c == "$" or not line.strip():
                    continue
                elif ident(line) in dropped:
                    continue
                elif opened:
                    o.write(line)
                else:
                    held[ident(line)] = line.rstrip("\n") + "\n"
            if not opened or now < end:
                sys.exit(f"{vcd}: the simulation ends at {now * unit:g} ns, before the window's end at {end * unit:g} ns")
            if stamped != end - start:
                o.write("#%d\n" % (end - start))
    return out


def vcd_rises(vcd):
    """Rising transitions (0 -> 1) of every scalar signal in the VCD, and the simulated span in ns.
    Switching power over the span, divided by V^2 and by the rises, is the capacitance each rise
    charged: the first-principles check that a report_power number is physics, not a default."""
    last, rises, t_end = {}, 0, 0
    with open(vcd) as f:
        scale = vcd_header(f)[1] or 1.0
        for line in f:
            c = line[:1]
            if c == "#":
                t_end = int(line[1:])
            elif c in "01xzXZ" and len(line) > 1:
                sid = line[1:].strip()
                if c == "1" and last.get(sid) == "0":
                    rises += 1
                last[sid] = c
    return rises, t_end * scale


def opensta():
    """The image of the OpenSTA every power figure is read with (scripts/opensta/), built on first use."""
    r = subprocess.run(["bash", f"{ROOT}/scripts/opensta/build-opensta.sh"], capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"scripts/opensta/build-opensta.sh: {r.stderr[-2000:]}")
    return r.stdout.split()[-1]


def linked_targets():
    """The target of each top-level symbolic link of the checkout that points outside it (a copy whose tools/ links
    the checkout's): mounted read-only beside the checkout, a path through the link resolves in the container too."""
    real = os.path.realpath(ROOT)
    out = set()
    for name in os.listdir(ROOT):
        path = os.path.join(ROOT, name)
        if os.path.islink(path):
            target = os.path.realpath(path)
            if not (target == real or target.startswith(real + os.sep)):
                out.add(target)
    return sorted(out)


def run_sta(script, log, timeout=7200):
    """Run the Tcl `script` in OpenSTA; its whole output is kept at `log` and returned."""
    links = [a for t in linked_targets() for a in ("-v", f"{t}:{t}:ro")]
    r = subprocess.run(["docker", "run", "--rm", "-v", f"{ROOT}:{ROOT}", "-v", f"{PDK}:{PDK}:ro"] + links +
                       ["--user", f"{os.getuid()}:{os.getgid()}", opensta(), "sta", "-exit", script],
                       capture_output=True, text=True, timeout=timeout)
    text = r.stdout + r.stderr
    open(log, "w").write(text)
    return text


# The Tcl that reads a VCD's activity with both traces on, and the report_power after it, naming every hierarchical
# instance: what activity_proof reads.
TRACE_ON = ['foreach c [get_cells -hierarchical *] { if {[get_property $c liberty_cell] == "NULL"} '
            '{ puts "LOGOS-HIER [get_full_name $c]" } }',
            "sta::set_debug read_vcd_activities 1"]
WALK_ON = ["sta::set_debug read_vcd_activities 0", "sta::set_debug power_activity 3"]
TRACE_OFF = ["sta::set_debug power_activity 0"]
CLOCK = (2.0, 0.5)


def activity_proof(text):
    """What OpenSTA did with a VCD's activities, from the traces TRACE_ON and WALK_ON print: every pin the VCD
    annotated (`read_vcd_activities`), each pin its activity walk set (`visit`), each input it seeded (`seed`) and
    every hierarchical instance (`LOGOS-HIER`). A pin of a hierarchical instance is a boundary, no cell's pin, and burns
    nothing. Any other pin the walk neither set nor seeded keeps no activity of its own; on the clock network that is
    the clock's (two transitions a period, duty 0.5), so an annotation there carrying exactly that loses nothing. Any
    other is a dropped annotation. Returns {annotated, set, seeded, hierarchical, clock, dropped: [pin, ...]}."""
    listed, visited, seeded, hier = {}, set(), set(), set()
    for line in text.splitlines():
        if line.startswith("read_vcd_activities: "):
            f = line.split()
            listed[f[1]] = (float(f[5]), float(f[7]))
        elif line.startswith("power_activity: visit "):
            visited.add(line.split()[2])
        elif line.startswith("power_activity: seed "):
            seeded.add(line.split()[2])
        elif line.startswith("LOGOS-HIER "):
            hier.add(line.split(" ", 1)[1].strip())
    rest = set(listed) - visited - seeded
    boundary = {p for p in rest if "/" in p and p.rsplit("/", 1)[0] in hier}
    rest -= boundary
    clock = {p for p in rest if abs(listed[p][0] - CLOCK[0]) <= 0.01 and abs(listed[p][1] - CLOCK[1]) <= 0.01}
    return {"annotated": len(listed), "set": len(set(listed) & visited), "seeded": sorted(set(listed) & seeded),
            "hierarchical": len(boundary), "clock": len(clock), "dropped": sorted(rest - clock)}


def sta_power(block, run, vcd, work):
    """Default and VCD-annotated report_power at every corner, one STA process per corner so no
    library or parasitics ever mix between corners. Returns {corner: {default, vcd, annotated}}."""
    results = {}
    read = sta_vcd(vcd, f"{work}/sta.vcd") if vcd else None
    for lib, rc in CORNERS:
        tcl = [f"read_liberty {LIBDIR}/sky130_fd_sc_hd__{lib}.lib",
               f"read_verilog {run}/results/final/verilog/gl/{block}.nl.v", f"link_design {block}",
               f"read_spef {run}/results/final/spef/multicorner/{block}.{rc}.spef",
               f"read_sdc {run}/results/final/sdc/{block}.sdc",
               'puts "LOGOS-PINS [llength [get_pins -hierarchical *]] [llength [get_ports *]]"',
               'puts "LOGOS-DEFAULT"', "report_power"]
        if vcd:
            tcl += TRACE_ON + [f"read_power_activities -scope tb/dut -vcd {read}"] + WALK_ON
            tcl += ['puts "LOGOS-VCD"', "report_power"] + TRACE_OFF
        script = f"{work}/power_{lib}.tcl"
        open(script, "w").write("\n".join(tcl + ["exit"]) + "\n")
        text = run_sta(script, f"{work}/sta_{lib}.log")
        got = {}
        if vcd:
            proof = activity_proof(text)
            if proof["dropped"]:
                sys.exit(f"{block}: report_power at {lib} did not use the VCD's activity on {len(proof['dropped'])} "
                         f"pins (first {proof['dropped'][:5]}; see {work}/sta_{lib}.log)")
            got["activity_proof"] = dict(proof, dropped=0)
        parts = re.split(r"LOGOS-(DEFAULT|VCD)", text)
        for tag, chunk in zip(parts[1::2], parts[2::2]):
            t = parse_power(chunk)
            if t:
                got[tag.lower()] = t[0]
        if "default" not in got or (vcd and "vcd" not in got):
            sys.exit(f"{block}: no report_power table at {lib} (see {work}/sta_{lib}.log)")
        annotated = re.findall(r"Annotated (\d+) pin activities", text)
        pins = re.findall(r"LOGOS-PINS (\d+) (\d+)", text)
        if pins:
            got["design_pins"] = int(pins[-1][0]) + int(pins[-1][1])
        if annotated:
            got["annotated_pins"] = int(annotated[-1])
        results[lib] = got
    if read and read != vcd:
        os.remove(read)
    return results


def main():
    args = sys.argv[1:]
    if not args:
        sys.exit(__doc__)
    block = args[0]
    vcd_on = "--no-vcd" not in args
    cycles = int(args[args.index("--cycles") + 1]) if "--cycles" in args else 4000
    ledger, rec, run_root = ledger_record(block)
    run = shipped_run(block, rec["gds_bytes"], run_root)
    period = float(rec["clock_period_ns"])
    work = f"{ROOT}/logs/power/{block}"
    os.makedirs(work, exist_ok=True)
    vcd, sig_x = simulate(block, run, period, cycles, work) if vcd_on else (None, None)
    witness = rtl_witness(block, run, period, cycles, work) if vcd_on else None
    signature = changes = trajectory = None
    if vcd:
        gate, width = vcd_signature(vcd, witness["signature_port"])
        shared = witnessed(gate, witness.pop("trajectory"), width)
        if shared is None:
            sys.exit(f"{block}: the gate-level {witness['signature_port']} does not follow its RTL's cycle for cycle "
                     f"— the netlist does not compute its RTL over this window")
        signature, changes, trajectory = shared
        witness.update(signature=signature, signature_changes=changes, signature_trajectory=trajectory)
    corners = sta_power(block, run, vcd, work)
    rises, span_ns = vcd_rises(vcd) if vcd else (0, 0.0)
    print(json.dumps({
        "block": block, "ledger": ledger, "run": os.path.relpath(run, ROOT), "gds_bytes": rec["gds_bytes"],
        "clock_period_ns": period, "clock_mhz": round(1000.0 / period, 3),
        "worst_setup_slack_ns": rec["setup_worst_slack_ns"],
        "std_cells": rec["std_cells"], "die_mm2": rec["die_area_mm2"],
        "simulated_cycles": cycles if vcd_on else 0, "signature_has_x": sig_x, "signature": signature,
        "signature_changes": changes, "signature_trajectory": trajectory,
        "rtl_witness": witness,
        "vcd_rises": rises, "vcd_span_ns": span_ns,
        "corners": corners,
    }, indent=1))


if __name__ == "__main__":
    main()
