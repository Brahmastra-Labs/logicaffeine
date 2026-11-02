#!/usr/bin/env python3
"""Elaborate the accelerator against the D5005 FIM's Platform Interface Manager, as the PR build will.

    sv_check.py <pr-template dir> <afu rtl dir>

<pr-template dir> is the FIM's unpacked PR build template (`scripts/d5005/fetch-d5005-tools.sh` puts it at
tools/d5005/pr-template); <afu rtl dir> holds the emitted `sources.txt` and the files it names. Run it with a Python
that has pyslang (tools/d5005/venv/bin/python).

The PIM's sources are its own simulation list (`platform/ofs_plat_if/sim/platform_if_addenda.txt`) without the
simulation-only modules, the FIM's afu_main, and the Agilex copy of an interface the Stratix 10 tree defines too. The
top is `ofs_plat_afu`, instantiated with an `ofs_plat_if` the way afu_main does, under the configuration header
afu_synth_setup writes for the `ofs_plat_afu` class. The primitives Quartus supplies (scfifo, dcfifo, altera_syncram,
...) are not in the template and stand unresolved. Any other error refuses, and so does any warning in the
accelerator's own files of a kind that hides a wrong connection or a lost bit.
"""
import os
import sys
import tempfile

import pyslang

QUARTUS_PRIMITIVES = {"scfifo", "dcfifo", "dcfifo_mixed_widths", "altsyncram", "altera_syncram", "altera_std_synchronizer",
                      "fourteennm_fp_mac"}
REFUSED_WARNINGS = {"range-oob", "width-trunc", "port-width-trunc", "sign-compare", "unconnected-port",
                    "empty-output-connection", "implicit-net-port", "unconnected-unnamed-port"}

CONFIG = """`ifndef __PLATFORM_AFU_TOP_CONFIG_VH__
`define __PLATFORM_AFU_TOP_CONFIG_VH__
`define AFU_TOP_REQUIRES_OFS_PLAT_IF_AFU 1
`define PLATFORM_SHIM_MODULE_NAME ofs_plat_afu
`endif
"""

CHECK_TOP = """module logos_afu_check;
  ofs_plat_if plat_ifc();
  ofs_plat_afu afu(.plat_ifc(plat_ifc));
endmodule
"""


def expand(path, out):
    base = os.path.dirname(path)
    for raw in open(path):
        line = raw.split("##")[0].strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("-F "):
            expand(os.path.normpath(os.path.join(base, line[3:].strip())), out)
        elif line.startswith("+incdir+"):
            out.append("+incdir+" + os.path.normpath(os.path.join(base, line[len("+incdir+"):])))
        elif line.startswith("+define+"):
            out.append(line)
        else:
            out.append(os.path.normpath(os.path.join(base, line)))


def pim_sources(template):
    listing = os.path.join(template, "hw/lib/build/platform/ofs_plat_if/sim/platform_if_addenda.txt")
    if not os.path.isfile(listing):
        sys.exit(f"{template} holds no PIM file list ({listing})")
    out = []
    expand(listing, out)
    keep = []
    for e in out:
        name = os.path.basename(e)
        if e.startswith("+"):
            keep.append(e)
        elif name.startswith("ase_") or "/sim/" in e or name == "afu_main.sv" or "/fpga_family/agilex/" in e:
            continue
        else:
            keep.append(e)
    return keep


def afu_sources(rtl):
    listing = os.path.join(rtl, "sources.txt")
    files = []
    for raw in open(listing):
        line = raw.split("#")[0].strip()
        if line.endswith((".v", ".sv")):
            files.append(os.path.join(rtl, line))
    return files


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    template, rtl = sys.argv[1], sys.argv[2]
    ours = afu_sources(rtl)
    with tempfile.TemporaryDirectory() as d:
        open(os.path.join(d, "platform_afu_top_config.vh"), "w").write(CONFIG)
        top = os.path.join(d, "logos_afu_check.sv")
        open(top, "w").write(CHECK_TOP)
        args = ["slang"] + pim_sources(template) + [f"+incdir+{d}"] + ours + [top, "--top", "logos_afu_check",
                "--timescale", "1ps/1ps", "--allow-hierarchical-const"]
        driver = pyslang.driver.Driver()
        driver.addStandardArgs()
        if not driver.parseCommandLine(" ".join(args), pyslang.driver.CommandLineOptions()):
            sys.exit("slang refused its arguments")
        if not (driver.processOptions() and driver.parseAllSources()):
            sys.exit("slang could not read the sources")
        comp = driver.createCompilation()
        refused = []
        for diag in comp.getAllDiagnostics():
            text = pyslang.DiagnosticEngine.reportAll(comp.sourceManager, [diag])
            # The file a diagnostic is in: its location line's, which an "in instance:" line comes before when the
            # diagnostic is an instance's.
            lines = text.strip().splitlines()
            located = next((l for l in lines if ": warning:" in l or ": error:" in l), lines[0] if lines else "")
            where = located.split(":")[0]
            mine = any(os.path.basename(f) in where for f in ours)
            if diag.isError():
                if "unknown module" in text and any(f"'{p}'" in text for p in QUARTUS_PRIMITIVES):
                    continue
                refused.append(text)
            elif mine and any(f"[-W{w}]" in text for w in REFUSED_WARNINGS):
                refused.append(text)
    if refused:
        print(f"ofs_plat_afu: {len(refused)} problems elaborating against the PIM:")
        for t in refused[:30]:
            print(t)
        sys.exit(1)
    print(f"ofs_plat_afu: elaborates against the PIM of {template}, {len(ours)} files of its own")


if __name__ == "__main__":
    main()
