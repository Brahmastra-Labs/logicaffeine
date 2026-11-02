#!/usr/bin/env python3
"""Extract the module-instantiation graph from the synthesized silicon RTL, so the SoC
block diagram's topology can be PROVEN against the netlist instead of hand-asserted.

For every `module NAME ... endmodule` in benchmarks/silicon/*.v (testbenches excluded), we
record which OTHER design modules it instantiates (a Verilog instantiation is
`MODNAME [#(params)] instance_name ( ...`). The result is the real containment graph:
soft_core -> {alu, shared_divider}, coproc_core -> {bigint_alu, bigint_mul, crdt_merge},
coproc_soc -> {coproc_core}, keccak_top -> {keccak_core, keccak_round}, ...

Writes benchmarks/silicon/netlist_topology.json. `netlist_topology_lock` pins the diagram's
structural claims to this artifact, so a block the diagram draws as containing another must
actually instantiate it in the RTL that synthesizes.
"""
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SIL = os.path.join(ROOT, "benchmarks", "silicon")
OUT = os.path.join(SIL, "netlist_topology.json")

# Verilog keywords / primitives that can appear where an instantiation would and must never
# be mistaken for a module instance.
NON_INSTANCE = {
    "input", "output", "inout", "reg", "wire", "logic", "integer", "genvar", "parameter",
    "localparam", "assign", "always", "initial", "begin", "end", "if", "else", "case",
    "endcase", "for", "while", "generate", "endgenerate", "module", "endmodule", "function",
    "endfunction", "task", "endtask", "posedge", "negedge", "default", "signed", "unsigned",
}


def strip_comments(src: str) -> str:
    src = re.sub(r"/\*.*?\*/", " ", src, flags=re.S)   # block comments
    src = re.sub(r"//[^\n]*", " ", src)                 # line comments
    return src


def module_bodies(src: str):
    """Yield (name, body) for each `module NAME ... endmodule`."""
    for m in re.finditer(r"\bmodule\s+([A-Za-z_]\w*)\b(.*?)\bendmodule\b", src, flags=re.S):
        yield m.group(1), m.group(2)


def main() -> int:
    files = sorted(
        f for f in os.listdir(SIL)
        if f.endswith(".v") and not f.startswith("tb_") and f != "tb.v"
    )

    # Pass 1: every design module name (so we only count instantiations of REAL modules).
    modules = {}          # name -> file
    for fname in files:
        src = strip_comments(open(os.path.join(SIL, fname)).read())
        for name, _ in module_bodies(src):
            if not name.startswith("tb") and name not in ("tb",):
                modules.setdefault(name, fname)

    known = set(modules)

    # Pass 2: within each module body, find instantiations of other known modules.
    topo = {}
    for fname in files:
        src = strip_comments(open(os.path.join(SIL, fname)).read())
        for name, body in module_bodies(src):
            if name not in known:
                continue
            insts = {}
            # MODNAME [#( ... )] instname (   — instname distinguishes it from a bare type use.
            for mm in re.finditer(
                r"\b([A-Za-z_]\w*)\s+(?:#\s*\([^;]*?\)\s*)?([A-Za-z_]\w*)\s*\(", body
            ):
                sub, inst = mm.group(1), mm.group(2)
                if sub in known and sub != name and sub not in NON_INSTANCE \
                        and inst not in NON_INSTANCE:
                    insts.setdefault(sub, []).append(inst)
            entry = topo.setdefault(name, {"file": modules[name], "instantiates": {}})
            for sub, names in insts.items():
                entry["instantiates"].setdefault(sub, [])
                entry["instantiates"][sub].extend(names)

    doc = {
        "note": "module-instantiation graph extracted from benchmarks/silicon/*.v "
                "by scripts/extract-netlist-topology.py; pinned by netlist_topology_lock",
        "modules": sorted(known),
        "graph": {k: topo[k] for k in sorted(topo)},
    }
    with open(OUT, "w") as f:
        json.dump(doc, f, indent=2)
        f.write("\n")

    print(f"== {OUT} ==")
    print(f"  {len(known)} design modules")
    for name in sorted(topo):
        subs = topo[name]["instantiates"]
        if subs:
            print(f"  {name} -> " + ", ".join(
                f"{s} x{len(v)}" for s, v in sorted(subs.items())))
    return 0


if __name__ == "__main__":
    sys.exit(main())
