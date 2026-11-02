#!/usr/bin/env python3
"""netlist_x_check.py DIR — every DSP48E1 of DIR's synthesized netlist (netlist.v) takes known data operands: no A, B, C
or D port tied to a literal holding `x`. yosys 0.33 folds a registered sum into the pre-adder of only the top slice of a
product it splits over a cascade, and wires the bottom slice's A to `x`, which the bitstream then reads as a constant:
the image computes another product than its RTL does."""
import re, sys

d = sys.argv[1]
text = open(f"{d}/netlist.v").read()
bad = []
cells = re.findall(r"DSP48E1 #\((?:.*?)\)\s+(\S+)\s+\((.*?)\);", text, re.S)
for name, body in cells:
    for port in ("A", "B", "C", "D"):
        m = re.search(r"\." + port + r"\(([^()]*)\)", body)
        if m and re.search(r"\d+'[hbd][0-9a-fA-F_]*[xX]", m.group(1)):
            bad.append(f"{name}.{port} = {m.group(1).strip()[:40]}")
print(f"{len(cells)} DSP48E1 slices, {len(bad)} with an unknown data operand")
for b in bad[:20]:
    print("  ", b)
sys.exit(1 if bad else 0)
