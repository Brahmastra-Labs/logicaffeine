#!/usr/bin/env python3
"""fasm_dram_check.py DIR DEVICE [DB_ROOT] — every LUT-RAM's contents in DIR/top.fasm are the synthesized netlist's.

The expectation is computed from the netlist (DIR/top.json): each RAM32M's and RAM64M's INIT_A..D, never the packer's
own copy of it. The packer splits a RAM32M port into two RAMD32 LUTs (DOx[j] at INITx bit 2k + j for address k) and a
RAM64M port into one RAMD64E; the routed design (DIR/top_routed.json) says where each landed and which logical address
bit each physical pin A1..A6 carries (X_ORIG_PORT_A*). A LUT's 64 INIT bits are then, at physical index p, the
contents at the logical address p's pins spell; a 5LUT holds the lower half, a 6LUT beside one the upper. Sites map to
FASM tiles through prjxray's tilegrid for DEVICE (xc7a100t, xc7a200t), under DB_ROOT (prjxray-db's artix7 directory,
the one scripts/setup-openxc7.sh installs by default).

A LUT-RAM's write address enters on WA1..WA6 in a fixed order, so the cell a write reaches is the one a read reaches
only if every live address bit sits on the same pin position for both. That is checked too: for each LUT-RAM, each
logical address bit its read pins carry (X_ORIG_PORT_A*) is at the position its write pins carry it (X_ORIG_PORT_WA*).
A router that permutes a LUT-RAM's read pins leaves INIT consistent and every write in the wrong cell.

Exit 0 when every LUT-RAM bit matches and every LUT-RAM's read and write addresses agree, 1 with the first mismatches
named, 2 when an input is missing or a LUT-RAM of the routed design has no INIT line."""
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
d, device = sys.argv[1], sys.argv[2]
DB = sys.argv[3] if len(sys.argv) > 3 else os.path.join(ROOT, "tools/openxc7/nextpnr-xilinx/xilinx/external/prjxray-db/artix7")


def top_module(path):
    j = json.load(open(path))
    mods = j["modules"]
    for name, m in mods.items():
        if m.get("attributes", {}).get("top"):
            return m
    return next(iter(mods.values()))


def bits_lsb_first(s):
    """A yosys/nextpnr binary parameter (most significant first) as bits, bit 0 first."""
    return [c == "1" for c in reversed(s)]


src = top_module(os.path.join(d, "top.json"))
routed = top_module(os.path.join(d, "top_routed.json"))
rams = {n: c for n, c in src["cells"].items() if c["type"] in ("RAM32M", "RAM64M")}

site_tile = {}
for tile, v in json.load(open(os.path.join(DB, device, "tilegrid.json"))).items():
    sites = v.get("sites", {})
    for idx, (site, kind) in enumerate(sorted(sites.items(), key=lambda kv: int(re.search(r"X(\d+)Y", kv[0]).group(1)))):
        if kind in ("SLICEM", "SLICEL"):
            site_tile[site] = (tile, f"{kind}_X{idx}")

fasm = {}
for line in open(os.path.join(d, "top.fasm")):
    m = re.match(r"(\S+)\.(SLICE[ML]_X\d)\.([A-D])LUT\.INIT\[63:0\] = 64'b([01]{64})", line)
    if m:
        fasm[(m.group(1), m.group(2), m.group(3))] = bits_lsb_first(m.group(4))

# LUT location -> {6 or 5: (logical bits, read pin map)}
routed_net_name = {}
for nn, info in routed["netnames"].items():
    for b in info["bits"]:
        routed_net_name.setdefault(b, nn)
address_bad = []
luts = {}
for name, c in routed["cells"].items():
    m = re.match(r"(.*)/DPR(\d)(?:_(\d))?$", name)
    attrs = c.get("attributes", {})
    if not m or attrs.get("X_ORIG_TYPE") not in ("RAMD32", "RAMD64E"):
        continue
    base, i, j = m.group(1), int(m.group(2)), m.group(3)
    if base not in rams:
        print(f"{name}: no {base} among the netlist's RAM32M/RAM64M cells")
        sys.exit(2)
    # yosys leaves out a parameter at its default, and INITx defaults to zero.
    init = bits_lsb_first(rams[base]["parameters"].get(f"INIT_{'ABCD'[i]}", "0" * 64).rjust(64, "0"))
    if j is None:
        logical = init[:64]
        width = 6
    else:
        logical = [init[2 * k + int(j)] for k in range(32)]
        width = 5
    bel = attrs["NEXTPNR_BEL"]
    site, lutbel = bel.split("/")
    tile, half = site_tile[site]
    key = (tile, half, lutbel[0])
    pins = {}
    for k in range(6):
        names = attrs.get(f"X_ORIG_PORT_A{k + 1}", "")
        pins[k] = [int(n) for n in re.findall(r"(?:RADR|A)(\d)", names)]
    # The logical address bits that vary: a read pin on a constant net carries a tied bit, which no address moves.
    conns = c.get("connections", {})
    def live(port):
        bits = conns.get(port) or []
        return bool(bits) and not any(routed_net_name.get(x) in ("$PACKER_GND_NET", "$PACKER_VCC_NET") or isinstance(x, str) for x in bits)
    read_at = {n: k for k in range(6) for n in pins[k] if live(f"A{k + 1}")}
    write_at = {}
    for k in range(8):
        for n in re.findall(r"WADR(\d)", attrs.get(f"X_ORIG_PORT_WA{k + 1}", "")):
            if live(f"WA{k + 1}"):
                write_at[int(n)] = k
    for n, k in read_at.items():
        if n in write_at and write_at[n] != k:
            address_bad.append(f"{name}: address bit {n} is read on A{k + 1} and written on WA{write_at[n] + 1}")
    luts.setdefault(key, {})[int(lutbel[1])] = (logical, width, pins, name)

bad, checked = [], 0
for key, halves in sorted(luts.items()):
    if key not in fasm:
        print(f"{key}: a LUT-RAM with no INIT line in top.fasm")
        sys.exit(2)
    got = fasm[key]
    for size, (logical, width, pins, name) in halves.items():
        if size == 5 or (5 in halves):
            lo, hi = (0, 32) if size == 5 else (32, 64)
        else:
            lo, hi = 0, 64
        for p in range(lo, hi):
            a = 0
            for k in range(6):
                if p >> k & 1:
                    for n in pins[k]:
                        a |= 1 << n
            want = logical[a % len(logical)]
            checked += 1
            if got[p] != want:
                bad.append(f"{key} {name} bit {p}: fasm {int(got[p])}, netlist {int(want)} (address {a})")
nonzero = sum(1 for key in luts if any(fasm[key]))
print(f"{len(luts)} LUT-RAM LUTs, {checked} bits checked, {len(bad)} differ; {nonzero} LUTs hold nonzero contents; "
      f"{len(address_bad)} LUT-RAM address bits read on another pin than written")
for b in (bad + address_bad)[:10]:
    print("  " + b)
sys.exit(1 if bad or address_bad else 0)
