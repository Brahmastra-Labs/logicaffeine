#!/usr/bin/env python3
"""Refuse a design in which a value crosses between clocks anywhere but through a declared crossing.

    cdc_check.py shell [--single-clock <module>]... <domains.json> <crossing.v> <source.v>...
        The top the map names, at the parameters the map sets, flattened with the crossing modules (the crossing file's
        and those the map's `crossings` names) as black boxes: every register is clocked by a clock port, and every
        register input, crossing input and top output is driven only from its own clock's domain, through any amount of
        logic. A FIFO crossing's ports belong to its write clock (`wclk`) when they start with `w` and to its read clock
        (`rclk`) when they start with `r`; a related crossing's to the engine's clock (`sclk`) when they start with `s`
        and to the fast clock (`fclk`) when they start with `f`. A module named --single-clock, or in the map's
        `single_clock`, is first
        proven to have one clock on its own hierarchy, unflattened, at its own parameters: every register, memory port
        and child under it clocked from one of its inputs; it then stands in the top as a black box of that clock's
        domain, every value into it driven only from that domain and every value out of it in it. An instance that sets
        its parameters is refused, as the proof was not made at them. A module in the map's `pairs` is, under a box,
        clocked by its `clk` port, its `clk2` (a clock twice `clk`, their rising edges together) wired to an input of the
        module holding it, its own hierarchy proven apart (`pair`).
    cdc_check.py fifo <module> <crossing.v>
        One crossing module alone: the only paths between its two clocks are a write-clock register's Gray pointer
        into a read-clock register (`wgray` -> `wgray_s1`), the reverse (`rgray` -> `rgray_s1`), and the RAM into the
        read register (`mem` -> `rdata_q`), each register to register with no logic between.
    cdc_check.py pair <module> <source.v>...
        One pair alone (two cores' processing elements in a clock of twice theirs): every register is clocked by the
        rising edge of `clk` or `clk2`, `clk2` reaches nothing but registers' clocks, and every output is driven only from
        `clk`'s registers and inputs: what it computes in `clk2` leaves it only through `clk`'s registers.
    cdc_check.py related <module> <source.v>...
        One related crossing alone (an engine's bridge between the compute clock the shell runs in and the user clock it
        is divided from, every register in the user clock): its slow-side clock `sclk` reaches nothing; every register
        is clocked by `fclk`; a register that reads a slow-side input (`s_*`) is a `cap_*` register enabled exactly by
        `emark2`, the register taking `emark1`, the register taking `fedge`, each bit straight from an `s_*` input with
        no logic between, its reset if any `frst`; every slow-side output (`s_*`) is straight from a `held_*` register
        enabled exactly by `mark2`, the register taking `mark1`, the register taking `fmid`.

Exit 0 and one summary line when the design is clean; exit 1 naming each path that crosses.
"""
import json
import os
import re
import subprocess
import sys
import tempfile

FLOPS = {"$dff", "$adff", "$dffe", "$adffe", "$sdff", "$sdffe", "$sdffce", "$aldff", "$aldffe", "$dffsr", "$dffsre"}
FLOP_DATA = {"D", "EN", "SRST", "ARST", "AD", "ALOAD", "SET", "CLR"}


def yosys_json(script):
    with tempfile.TemporaryDirectory() as d:
        out = os.path.join(d, "net.json")
        ys = os.path.join(d, "run.ys")
        with open(ys, "w") as f:
            f.write(script.replace("@OUT@", out))
        r = subprocess.run(["yosys", "-q", "-s", ys], capture_output=True, text=True)
        if r.returncode != 0:
            sys.exit(f"yosys failed:\n{r.stdout}\n{r.stderr}")
        return json.load(open(out))


def bit_names(module):
    """A name for every net bit: an internal register's own name before a port it drives."""
    ports = set(module["ports"])
    names = {}
    ranked = sorted(module["netnames"].items(), key=lambda kv: (kv[0] in ports, kv[1].get("hide_name", 0), kv[0]))
    for name, n in ranked:
        for i, b in enumerate(n["bits"]):
            if isinstance(b, int) and b not in names:
                names[b] = f"{name}[{i}]" if len(n["bits"]) > 1 else name
    return names


def propagate(module, sources):
    """The domains each net bit is driven from: `sources` seeds bits, every other cell's outputs take the union of
    its inputs', to a fixed point."""
    dom = {b: set(d) for b, d in sources.items()}
    comb = []
    for cname, c in module["cells"].items():
        if c["type"] in FLOPS or not c["type"].startswith("$"):
            continue
        ins = [b for p, bits in c["connections"].items() if c["port_directions"][p] == "input" for b in bits if isinstance(b, int)]
        outs = [b for p, bits in c["connections"].items() if c["port_directions"][p] == "output" for b in bits if isinstance(b, int)]
        comb.append((ins, outs))
    changed = True
    while changed:
        changed = False
        for ins, outs in comb:
            u = set()
            for b in ins:
                u |= dom.get(b, set())
            for b in outs:
                cur = dom.setdefault(b, set())
                if not u <= cur:
                    cur |= u
                    changed = True
    return dom


def single_clock(module, crossing_v, sources, pairs=()):
    """`module`'s one clock input, proven on its elaborated hierarchy (pyslang, at its own parameters): in every module
    under it each clocked block's event is one edge of one input of that module — or that edge and one edge of another
    input, its asynchronous reset — and each child's clock is wired to that same input, so every register and memory
    port under it runs on one clock. Exits naming the first that does not."""
    import pyslang
    from pyslang import ast
    d = pyslang.driver.Driver()
    d.addStandardArgs()
    files = " ".join([crossing_v, *sources])
    if not (d.parseCommandLine(f"slang {files} --top {module} --timescale 1ps/1ps --ignore-unknown-modules",
                               pyslang.driver.CommandLineOptions()) and d.processOptions() and d.parseAllSources()):
        sys.exit(f"slang could not read the sources for {module}")
    top = d.createCompilation().getRoot().topInstances[0]

    def refuse(why):
        print(f"{module} is not single-clock: {why}")
        sys.exit(1)

    def prove(inst):
        body = inst.body
        port_of = {}
        for m in body:
            if isinstance(m, ast.PortSymbol) and m.internalSymbol is not None:
                port_of[m.internalSymbol] = m.name
        found = [None]

        def take(sym, what):
            if sym not in port_of:
                refuse(f"{inst.name} ({body.name}): {what} is clocked by {sym.name}, no input of {body.name}")
            if found[0] is None:
                found[0] = sym
            elif sym is not found[0]:
                refuse(f"{inst.name} ({body.name}): {what} is clocked by {port_of[sym]}, beside {port_of[found[0]]}")

        def walk(scope):
            for m in scope:
                if isinstance(m, ast.GenerateBlockSymbol):
                    if not m.isUninstantiated:
                        walk(m)
                elif isinstance(m, ast.GenerateBlockArraySymbol):
                    for entry in m.entries:
                        walk(entry)
                elif isinstance(m, ast.ProceduralBlockSymbol):
                    kind = m.procedureKind
                    if kind == ast.ProceduralBlockKind.AlwaysLatch:
                        refuse(f"{inst.name} ({body.name}): a latch")
                    st = m.body
                    if not isinstance(st, ast.TimedStatement):
                        continue
                    tc = st.timing
                    if isinstance(tc, ast.ImplicitEventControl):
                        continue
                    if isinstance(tc, ast.EventListControl):
                        # One edge of its clock, then one edge of an asynchronous reset, an input of the module.
                        events = list(tc.events)
                        if len(events) != 2 or not all(isinstance(e, ast.SignalEventControl) and isinstance(e.expr, ast.NamedValueExpression) for e in events):
                            refuse(f"{inst.name} ({body.name}): a block waits on more than a clock's edge and a reset's")
                        if events[1].expr.symbol not in port_of:
                            refuse(f"{inst.name} ({body.name}): a block is reset by {events[1].expr.symbol.name}, no input of {body.name}")
                        tc = events[0]
                    if not isinstance(tc, ast.SignalEventControl):
                        refuse(f"{inst.name} ({body.name}): a block waits on {type(tc).__name__}, not one edge of one clock")
                    if not isinstance(tc.expr, ast.NamedValueExpression):
                        refuse(f"{inst.name} ({body.name}): a block is clocked by an expression")
                    take(tc.expr.symbol, "a block")
                elif isinstance(m, ast.InstanceSymbol) and m.body.name in pairs:
                    # A pair: clocked by its clk, its clk2 a clock the module holding it is given.
                    conns = {c.port.name: c.expression for c in m.portConnections}
                    for port in ("clk", "clk2"):
                        e = conns.get(port)
                        if not isinstance(e, ast.NamedValueExpression):
                            refuse(f"{inst.name} ({body.name}): the pair {m.name}'s {port} is not wired to a signal")
                    if conns["clk2"].symbol not in port_of:
                        refuse(f"{inst.name} ({body.name}): the pair {m.name}'s clk2 is {conns['clk2'].symbol.name}, no input of {body.name}")
                    take(conns["clk"].symbol, m.name)
                elif isinstance(m, ast.InstanceSymbol):
                    child = prove(m)
                    if child is None:
                        continue
                    conn = next((c for c in m.portConnections if c.port.name == child), None)
                    e = conn.expression if conn is not None else None
                    if not isinstance(e, ast.NamedValueExpression):
                        refuse(f"{inst.name} ({body.name}): {m.name}'s clock {child} is not wired to a signal")
                    take(e.symbol, m.name)

        walk(body)
        return port_of[found[0]] if found[0] is not None else None

    own = prove(top)
    if own is None:
        refuse("nothing under it is clocked")
    return own


# A crossing's two clock ports, and the first letter of the ports on each one's side.
CROSSING_CLOCKS = (("wclk", "rclk"), ("sclk", "fclk"))


def check_shell(domains_path, crossing_v, sources, boxes=()):
    spec = json.load(open(domains_path))
    top = spec["top"]
    boxes = [*boxes, *spec.get("single_clock", [])]
    pairs = spec.get("pairs", [])
    box_clock = {b: single_clock(b, crossing_v, sources, pairs) for b in boxes}
    script = f"read_verilog -sv -lib {crossing_v}\n" + "".join(f"read_verilog -sv {s}\n" for s in sources)
    script += "".join(f"blackbox {b}\n" for b in boxes)
    script += "".join(f"blackbox {c}\n" for c in spec.get("crossings", []))
    chparams = "".join(f" -chparam {name} {value}" for name, value in spec.get("parameters", {}).items())
    script += f"hierarchy -top {top}{chparams}\nproc\nflatten\nopt_dff\nopt_clean\nwrite_json @OUT@\n"
    net = yosys_json(script)
    m = net["modules"][top]
    names = bit_names(m)
    ports = m["ports"]
    # Clocks: every bit of a clock port is a domain of its own.
    clock_of = {}
    for clk in spec["clocks"]:
        if clk not in ports:
            sys.exit(f"the map's clock {clk} is not a port of {top}")
        bits = ports[clk]["bits"]
        for i, b in enumerate(bits):
            clock_of[b] = clk if len(bits) == 1 else f"{clk}[{i}]"
    banks = len(ports["mem_clk"]["bits"]) if "mem_clk" in ports else 1
    sources_dom = {}
    declared = {}
    for row in spec["ports"]:
        p = row["port"]
        if p not in ports:
            sys.exit(f"the map names {p}, which {top} does not have")
        bits = ports[p]["bits"]
        if "per_bank" in row:
            width = len(bits) // banks
            for i, b in enumerate(bits):
                declared[b] = f"{row['clock']}[{i // width}]"
        else:
            for b in bits:
                declared[b] = row["clock"]
    missing = [p for p, v in ports.items() if p not in spec["clocks"] and any(b not in declared for b in v["bits"])]
    if missing:
        print(f"{top}: ports with no clock in {domains_path}: {', '.join(missing)}")
        sys.exit(1)
    for p, v in ports.items():
        if v["direction"] == "input" and p not in spec["clocks"]:
            for b in v["bits"]:
                sources_dom[b] = {declared[b]}
    flops, crossings, boxed = [], [], []
    bad = []
    for cname, c in m["cells"].items():
        if c["type"] in FLOPS:
            clk = c["connections"]["CLK"][0]
            flops.append((cname, c, clk))
            for b in c["connections"]["Q"]:
                if isinstance(b, int) and clk in clock_of:
                    sources_dom[b] = {clock_of[clk]}
        elif c["type"] in box_clock or c["type"].split("\\")[-1] in box_clock:
            kind = c["type"] if c["type"] in box_clock else c["type"].split("\\")[-1]
            if c.get("parameters"):
                bad.append(f"{kind} is boxed at the parameters it was proven at, but {cname} sets "
                           f"{', '.join(sorted(c['parameters']))}")
            clk = c["connections"][box_clock[kind]][0]
            if clk not in clock_of:
                bad.append(f"{cname}: its clock {box_clock[kind]} is no clock port")
            own = clock_of.get(clk)
            boxed.append((cname, c, kind, own))
            for p, bits in c["connections"].items():
                if c["port_directions"].get(p) == "output":
                    for b in bits:
                        if isinstance(b, int):
                            sources_dom[b] = {own}
        elif not c["type"].startswith("$"):
            crossings.append((cname, c))
    side_clock, pair_of = {}, {}
    for cname, c in list(crossings):
        conn = c["connections"]
        pair = next((p for p in CROSSING_CLOCKS if p[0] in conn and p[1] in conn), None)
        if pair is None:
            bad.append(f"{cname}: {c['type']} is neither a crossing nor boxed")
            crossings.remove((cname, c))
            continue
        pair_of[cname] = pair
        side_clock[cname] = {}
        for clk_port in pair:
            clk = conn[clk_port][0]
            if clk not in clock_of:
                bad.append(f"{cname}: its {clk_port} is no clock port")
            side_clock[cname][clk_port[0]] = clock_of.get(clk)
        for p, bits in conn.items():
            if p in pair or c["port_directions"].get(p) != "output":
                continue
            if p[0] not in side_clock[cname]:
                bad.append(f"{cname}.{p} is on neither side of the crossing")
                continue
            for b in bits:
                if isinstance(b, int):
                    sources_dom[b] = {side_clock[cname][p[0]]}
    dom = propagate(m, sources_dom)
    for cname, c, clk in flops:
        q = [b for b in c["connections"]["Q"] if isinstance(b, int)]
        cname = re.sub(r"\[\d+\]$", "", names[q[0]]) if q else cname
        if clk not in clock_of:
            bad.append(f"{cname}: clocked by {names.get(clk, clk)}, which is no clock port")
            continue
        own = clock_of[clk]
        for p, bits in c["connections"].items():
            if p not in FLOP_DATA:
                continue
            for b in bits:
                other = dom.get(b, set()) - {own} if isinstance(b, int) else set()
                if other:
                    bad.append(f"{cname} ({own}) reads {names.get(b, b)} from {', '.join(sorted(other))}")
    for cname, c in crossings:
        for p, bits in c["connections"].items():
            if p in pair_of[cname] or c["port_directions"].get(p) != "input":
                continue
            if p[0] not in side_clock[cname]:
                bad.append(f"{cname}.{p} is on neither side of the crossing")
                continue
            own = side_clock[cname][p[0]]
            for b in bits:
                other = dom.get(b, set()) - {own} if isinstance(b, int) else set()
                if other:
                    bad.append(f"{cname}.{p} ({own}) is driven from {', '.join(sorted(other))}")
    for cname, c, kind, own in boxed:
        for p, bits in c["connections"].items():
            if p == box_clock[kind] or c["port_directions"].get(p) != "input":
                continue
            for b in bits:
                other = dom.get(b, set()) - {own} if isinstance(b, int) else set()
                if other:
                    bad.append(f"{cname}.{p} ({own}) is driven from {', '.join(sorted(other))}")
    for p, v in ports.items():
        if v["direction"] != "output":
            continue
        for b in v["bits"]:
            other = dom.get(b, set()) - {declared[b]} if isinstance(b, int) else set()
            if other:
                bad.append(f"output {p} ({declared[b]}) is driven from {', '.join(sorted(other))}")
    summary = f"{len(flops)} registers, {len(crossings)} crossings, {len(set(clock_of.values()))} clocks"
    if boxed:
        summary += f", {len(boxed)} single-clock box{'es' if len(boxed) > 1 else ''}"
    report(top, bad, summary)


def check_fifo(module, crossing_v):
    net = yosys_json(f"read_verilog -sv {crossing_v}\nhierarchy -top {module}\nproc\nopt_dff\nopt_clean\nwrite_json @OUT@\n")
    m = net["modules"][module]
    names = bit_names(m)
    ports = m["ports"]
    clock_of = {ports["wclk"]["bits"][0]: "wclk", ports["rclk"]["bits"][0]: "rclk"}
    sources = {}
    for p, v in ports.items():
        if v["direction"] == "input" and p not in ("wclk", "rclk"):
            for b in v["bits"]:
                sources[b] = {"wclk" if p.startswith("w") else "rclk"}
    flops = []
    bad = []
    reg_of = {}
    for cname, c in m["cells"].items():
        if c["type"] in FLOPS:
            clk = c["connections"]["CLK"][0]
            if clk not in clock_of:
                bad.append(f"{cname}: clocked by {names.get(clk, clk)}, which is neither clock")
                continue
            flops.append((cname, c, clock_of[clk]))
            q = [b for b in c["connections"]["Q"] if isinstance(b, int)]
            reg = re.sub(r"\[\d+\]$", "", names.get(q[0], cname)) if q else cname
            for b in q:
                sources[b] = {clock_of[clk]}
                reg_of[b] = reg
        elif c["type"] in ("$memwr", "$memwr_v2"):
            pass
    # A RAM's read data carries the clock it was written in, and the RAM is its source.
    mem_writes = {}
    for cname, c in m["cells"].items():
        if c["type"] in ("$memwr", "$memwr_v2"):
            memid = c["parameters"]["MEMID"]
            clk = c["connections"]["CLK"][0]
            mem_writes[memid] = clock_of.get(clk)
    mem_reads = {}
    for cname, c in m["cells"].items():
        if c["type"] in ("$memrd", "$memrd_v2"):
            memid = c["parameters"]["MEMID"]
            for b in c["connections"]["DATA"]:
                if isinstance(b, int):
                    sources[b] = {mem_writes.get(memid)}
                    reg_of[b] = memid.lstrip("\\")
                    mem_reads[b] = memid
    legal = {("wgray", "wgray_s1"), ("rgray", "rgray_s1"), ("mem", "rdata_q")}
    dom = propagate(m, sources)
    # Which register (or RAM) each bit comes straight from, with no logic between.
    direct = dict(reg_of)
    for cname, c, own in flops:
        q = [b for b in c["connections"]["Q"] if isinstance(b, int)]
        dest = reg_of[q[0]] if q else cname
        for p, bits in c["connections"].items():
            if p not in FLOP_DATA:
                continue
            for b in bits:
                if not isinstance(b, int):
                    continue
                other = dom.get(b, set()) - {own}
                if not other:
                    continue
                src = direct.get(b)
                if (src, dest) in legal or (src is not None and (src.split(".")[-1], dest) in legal):
                    continue
                bad.append(f"{dest} ({own}) reads {names.get(b, b)} from {', '.join(sorted(x or '?' for x in other))}"
                           + ("" if src else ", through logic"))
    report(module, bad, f"{len(flops)} registers, crossings only {sorted(legal)}")


def param(cell, name, default):
    """A cell parameter's value: yosys writes a parameter as a string of its bits."""
    v = cell["parameters"].get(name, default)
    return int(v, 2) if isinstance(v, str) else int(v)


def check_pair(module, sources):
    net = yosys_json("".join(f"read_verilog -sv {s}\n" for s in sources)
                     + f"hierarchy -top {module}\nproc\nflatten\nopt_dff\nopt_clean\nwrite_json @OUT@\n")
    m = net["modules"][module]
    names = bit_names(m)
    ports = m["ports"]
    for p in ("clk", "clk2"):
        if p not in ports:
            sys.exit(f"{module} has no port {p}")
    clk, clk2 = ports["clk"]["bits"][0], ports["clk2"]["bits"][0]
    bad = []
    seeds, flops = {}, []
    for p, v in ports.items():
        if v["direction"] == "input" and p not in ("clk", "clk2"):
            for b in v["bits"]:
                seeds[b] = {"clk"}
    for cname, c in m["cells"].items():
        if c["type"] in FLOPS:
            q = [b for b in c["connections"]["Q"] if isinstance(b, int)]
            reg = re.sub(r"\[\d+\]$", "", names.get(q[0], cname)) if q else cname
            on = c["connections"]["CLK"]
            if on not in ([clk], [clk2]) or param(c, "CLK_POLARITY", 1) != 1:
                bad.append(f"{reg}: not on the rising edge of clk or clk2")
            own = "clk2" if on == [clk2] else "clk"
            flops.append((reg, own))
            for b in q:
                seeds[b] = {own}
        else:
            for p, bits in c["connections"].items():
                if clk2 in bits:
                    bad.append(f"{cname} ({c['type']}) takes clk2 at {p}: clk2 clocks registers alone")
    for cname, c in m["cells"].items():
        if c["type"] in FLOPS:
            for p, bits in c["connections"].items():
                if p != "CLK" and clk2 in bits:
                    bad.append(f"{cname} takes clk2 at {p}: clk2 clocks registers alone")
    dom = propagate(m, seeds)
    for p, v in ports.items():
        if v["direction"] != "output":
            continue
        for b in v["bits"]:
            if isinstance(b, int) and "clk2" in dom.get(b, set()):
                bad.append(f"output {p} is driven from a clk2 register")
                break
    fast = sum(1 for _, own in flops if own == "clk2")
    report(module, bad, f"{len(flops)} registers, {fast} on clk2, every output from clk's")


def check_related(module, sources):
    net = yosys_json("".join(f"read_verilog -sv {s}\n" for s in sources)
                     + f"hierarchy -top {module}\nproc\nopt_dff\nopt_clean\nwrite_json @OUT@\n")
    m = net["modules"][module]
    names = bit_names(m)
    ports = m["ports"]
    for p in ("sclk", "fclk", "fmid", "fedge"):
        if p not in ports:
            sys.exit(f"{module} has no port {p}")
    bit = {p: ports[p]["bits"][0] for p in ("sclk", "fclk", "fmid", "fedge")}
    frst = ports["frst"]["bits"][0] if "frst" in ports else None
    s_in = {b for p, v in ports.items() if p.startswith("s_") and v["direction"] == "input" for b in v["bits"]}
    bad = []
    used = {b for c in m["cells"].values() for bits in c["connections"].values() for b in bits}
    if bit["sclk"] in used:
        bad.append("sclk reaches a cell: every register of the crossing is the user clock's")
    flops, reg_of = [], {}
    for cname, c in m["cells"].items():
        if c["type"] not in FLOPS:
            continue
        q = [b for b in c["connections"]["Q"] if isinstance(b, int)]
        reg = re.sub(r"\[\d+\]$", "", names.get(q[0], cname)) if q else cname
        for b in q:
            reg_of[b] = reg
        flops.append((reg, c))
        if c["connections"]["CLK"] != [bit["fclk"]] or param(c, "CLK_POLARITY", 1) != 1:
            bad.append(f"{reg}: not on fclk's rising edge")
    dom = propagate(m, {b: {"s"} for b in s_in})
    regs = {reg: c for reg, c in flops}

    def chain(first, second, source):
        """`second`'s Q when `first` takes `source` and `second` takes `first`, each every user cycle; else None."""
        a, b = regs.get(first), regs.get(second)
        if a is None or b is None:
            bad.append(f"no {first} and {second} registers")
            return None
        if a["connections"]["D"] != [bit[source]] or "EN" in a["connections"]:
            bad.append(f"{first} does not take {source} every user cycle")
        if b["connections"]["D"] != a["connections"]["Q"] or "EN" in b["connections"]:
            bad.append(f"{second} does not take {first} every user cycle")
        return b["connections"]["Q"]

    em2 = chain("emark1", "emark2", "fedge")
    m2 = chain("mark1", "mark2", "fmid")

    for reg, c in flops:
        conn = c["connections"]
        reads = any("s" in dom.get(b, set()) for p in FLOP_DATA for b in conn.get(p, []) if isinstance(b, int))
        if not reads:
            continue
        if not reg.startswith("cap_"):
            bad.append(f"{reg} reads the slow side outside a cap_ register")
            continue
        if "EN" not in conn or em2 is None or conn["EN"] != em2:
            bad.append(f"{reg}: not enabled exactly by emark2")
        if param(c, "EN_POLARITY", 1) != 1:
            bad.append(f"{reg}: enabled low")
        if "SRST" in conn and param(c, "SRST_POLARITY", 1) != 1:
            bad.append(f"{reg}: reset low")
        if any(not (isinstance(b, int) and b in s_in) for b in conn["D"]):
            bad.append(f"{reg}: a bit not straight from a slow-side input")
        if "SRST" in conn and conn["SRST"] != [frst]:
            bad.append(f"{reg}: reset by other than frst")
        for p in ("ARST", "AD", "ALOAD", "SET", "CLR"):
            if p in conn:
                bad.append(f"{reg}: an asynchronous {p}")
    for reg, c in flops:
        if reg.startswith("held_") and (m2 is None or c["connections"].get("EN") != m2):
            bad.append(f"{reg}: not enabled exactly by mark2")
    for p, v in ports.items():
        if not p.startswith("s_") or v["direction"] != "output":
            continue
        for b in v["bits"]:
            src = reg_of.get(b) if isinstance(b, int) else None
            if src is None or not src.startswith("held_"):
                bad.append(f"output {p} not straight from a held_ register")
                break
    report(module, bad, f"{len(flops)} registers, slow-side values only through cap_/held_ registers")


def report(what, bad, summary):
    bad = sorted(set(bad))
    if bad:
        print(f"{what}: {len(bad)} paths cross clocks outside a declared crossing:")
        for b in bad[:60]:
            print("  " + b)
        sys.exit(1)
    print(f"{what}: clean; {summary}")


def main():
    if len(sys.argv) >= 4 and sys.argv[1] == "shell":
        args, boxes = sys.argv[2:], []
        while len(args) >= 2 and args[0] == "--single-clock":
            boxes.append(args[1])
            args = args[2:]
        if len(args) < 2:
            sys.exit(__doc__)
        check_shell(args[0], args[1], args[2:], boxes)
    elif len(sys.argv) == 4 and sys.argv[1] == "fifo":
        check_fifo(sys.argv[2], sys.argv[3])
    elif len(sys.argv) >= 4 and sys.argv[1] == "related":
        check_related(sys.argv[2], sys.argv[3:])
    elif len(sys.argv) >= 4 and sys.argv[1] == "pair":
        check_pair(sys.argv[2], sys.argv[3:])
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
