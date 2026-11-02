"""image_record — one routed board image's record, as every ledger of board images states it: its routed core-clock
Fmax and utilization (pnr.log), what the router did not time, the FASM's LUT-RAM contents checked against the netlist
(fasm_dram_check.py), how its placement and route were run (pnr.recipe), and its bitstream, copied into the silicon
directory under `bit_name` with its size and sha256. Refuses an image whose core clock exceeds its routed Fmax or its
timing contract, that does not say how it was placed and routed, whose FASM does not hold every LUT-RAM's contents, or
whose netlist feeds a DSP slice an unknown operand (netlist_x_check.py)."""
import hashlib, json, os, re, shutil, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))

# The core clock's own uncertainty the router does not model: the PLL's output jitter and the clock tree's skew.
UNCERTAINTY_NS = 0.5
# nextpnr-xilinx's Arch::getPortTimingClass gives these cells no timing class: a path through one is never timed.
UNTIMED = ["DSP48E1", "RAMB18E1", "RAMB36E1", "RAM32M", "RAM64M", "RAM32X1D", "RAM64X1D", "RAM128X1D", "RAM256X1S", "SRL16E", "SRLC32E"]


def routed(d, silicon, bit_name, db=None):
    """The image in `d`, its own record (image.json) beside the fields every routed image states; `db` names prjxray-db's
    artix7 directory where it is not the one scripts/setup-openxc7.sh installs."""
    rec = json.load(open(os.path.join(d, "image.json")))
    recipe_path = os.path.join(d, "pnr.recipe")
    if not os.path.exists(recipe_path):
        sys.exit(f"{d}: no pnr.recipe saying how its placement and route were run")
    recipe = json.load(open(recipe_path))
    log = open(os.path.join(d, "pnr.log")).read()
    fmax = [float(m) for m in re.findall(r"Max frequency for clock +'coreclk': ([0-9.]+) MHz", log)]
    if not fmax:
        sys.exit(f"{d}: no routed coreclk Fmax in pnr.log")
    fmax = fmax[-1]
    util = {}
    block = log[log.rfind("Device utilisation:"):]
    for name, used, total in re.findall(r"Info: \t +(\S+): +(\d+)/ *(\d+)", block)[:12]:
        util[name] = {"used": int(used), "of": int(total)}
    # A nextpnr-xilinx built with hard-cell timing (NEXTPNR_XC7_TIMINGS) times every class above and names the
    # residual it bounds instead: a path through a LUT-RAM's write-through or a driven-address shift register's
    # clock-to-out is at most that much longer than one it times.
    # The arch reports once per assignment of cell info; the last, after packing, is the design's.
    hard = re.findall(r"Hard-cell timing residual: ([0-9.]+) ns", log)
    residual = float(hard[-1]) if hard else None
    stat = open(os.path.join(d, "stat.txt")).read()
    untimed = {} if hard else {c: int(n) for c, n in re.findall(r"^\s+(\w+)\s+(\d+)\s*$", stat, re.M) if c in UNTIMED and int(n) > 0}
    device = re.match(r"(xc7a\d+t)", rec["part"]).group(1)
    check = subprocess.run([sys.executable, os.path.join(HERE, "fasm_dram_check.py"), d, device] + ([db] if db else []),
                           capture_output=True, text=True)
    if check.returncode != 0:
        sys.exit(f"{d}: its FASM does not hold the netlist's LUT-RAM contents\n{check.stdout}{check.stderr}")
    luts, bits = (int(x) for x in re.match(r"(\d+) LUT-RAM LUTs, (\d+) bits checked", check.stdout).groups())
    dsp = subprocess.run([sys.executable, os.path.join(HERE, "netlist_x_check.py"), d],
                         capture_output=True, text=True)
    if dsp.returncode != 0:
        sys.exit(f"{d}: its netlist feeds a DSP slice an unknown operand\n{dsp.stdout}{dsp.stderr}")
    core = rec["core_mhz"]
    if core > fmax:
        sys.exit(f"{d}: the core clock {core} MHz exceeds the routed Fmax {fmax} MHz")
    if residual is not None and 1000.0 / core < 1000.0 / fmax + residual + UNCERTAINTY_NS:
        sys.exit(f"{d}: the core clock's period {1000.0 / core:.3f} ns is shorter than the routed path {1000.0 / fmax:.3f} ns plus the residual {residual} ns and the clock's {UNCERTAINTY_NS} ns")
    bit = os.path.join(d, rec["top"] + ".bit")
    shutil.copyfile(bit, os.path.join(silicon, bit_name))
    data = open(bit, "rb").read()
    return rec, {
        "pll": rec["pll"],
        "core_mhz": core,
        "fmax_mhz": fmax,
        "timing": {
            "complete": residual is not None,
            "tool": "nextpnr-xilinx (openXC7) with hard-cell timing from prjxray-db's SDF, speed grade -1" if hard else "nextpnr-xilinx (openXC7)",
            "untimed_cells": untimed,
            "residual_ns": residual,
            "clock_uncertainty_ns": UNCERTAINTY_NS,
            "note": ("fmax_mhz times every path but a LUT-RAM's write-through and a driven-address shift register's clock-to-out; a path through one is at most residual_ns longer than a path it times, and the core clock's period covers both." if hard else
                     "fmax_mhz is the routed design's over the paths the tool times: LUTs, carry chains and flip-flops. A path into, through or out of an untimed cell is not in it."),
        },
        "pnr": recipe,
        "lutram_contents": {"luts": luts, "bits_checked": bits, "check": "scripts/xc7/fasm_dram_check.py: every LUT-RAM INIT bit in the FASM equals the netlist's RAM32M/RAM64M INIT through its routed pins"},
        "utilization": util,
        "bitstream": bit_name,
        "bytes": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
    }
