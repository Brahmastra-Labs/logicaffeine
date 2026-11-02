#!/usr/bin/env python3
"""Record a built D5005 accelerator image, refusing one whose build or gates did not pass.

    record_afu_image.py <tree> <gates dir> <ledger.json>
    record_afu_image.py --flat <afu tree> <fim work dir> <gates dir> <ledger.json>

A partial-reconfiguration image: <tree> is the root afu_synth_setup configured and afu_synth compiled (its Quartus
project under build/syn/syn_top), its persona output_files/iofs_pr_afu.persona1.gbs. A flat image (build-afu.sh
--flat): <fim work dir> is the work tree the FIM's build_top.sh compiled with the accelerator of <afu tree> inside it
(its project under syn/syn_top), its image output_files/d5005.sof, the FIM and the accelerator in one. <gates dir>
holds each gate's output as build-afu.sh wrote it (cdc_shell.txt, cdc_fifo_mlab.txt, cdc_fifo_m20k.txt,
sv_check.txt, ram_check.txt, and for an engine, whose clock-domain map declares the crossing logos_engine_bridge,
cdc_bridge.txt).
The image is recorded only when:
  - the image exists and the ledger holds no image of its SHA-256;
  - the fitter succeeded (the project's .fit.summary: "Fitter Status : Successful");
  - the OFS timing report (report_timing.tcl) lists no clock in clocks.sta.fail.summary and some in
    clocks.sta.pass.summary;
  - for a persona, the user clocks the flow set from timing are in output_files/user_clock_freq.txt (a flat image's
    user clock is set on the card at run time, so its row carries the clock the AFU JSON asks for);
  - every gate's output ends with its pass line.
The row carries its kind, the image's SHA-256, the AFU's UUID and user clocks, every clock's least slack of each
analysis over the corners, the fit summary as Quartus wrote it, and each gate's pass line.
"""
import hashlib
import json
import os
import re
import sys
from datetime import datetime, timezone

GATES = {
    "cdc_shell.txt": "logos_d5005_shell: clean;",
    "cdc_fifo_mlab.txt": "logos_dcfifo_mlab: clean;",
    "cdc_fifo_m20k.txt": "logos_dcfifo_m20k: clean;",
    "sv_check.txt": "ofs_plat_afu: elaborates against the PIM",
    "ram_check.txt": "logos_d5005_shell: clean;",
}
# An engine's step boundary between the shell's compute side and the engine.
BRIDGE_GATES = {"cdc_bridge.txt": "logos_engine_bridge: clean;"}


def refuse(why):
    print(f"record_afu_image: refused: {why}")
    sys.exit(1)


def summary_blocks(path):
    """The (corner type, clock) -> slack blocks of an OFS timing summary."""
    out = []
    text = open(path).read()
    for m in re.finditer(r"Type\s*:\s*(\S+)\s+(\S+)\s+'([^']+)'\s*\nSlack\s*:\s*(\S+)\s*\nTNS\s*:\s*(\S+)", text):
        corner, kind, clock, slack, tns = m.groups()
        out.append({"corner": corner, "type": kind, "clock": clock, "slack": float(slack), "tns": float(tns)})
    return out


def main():
    args = sys.argv[1:]
    flat = bool(args) and args[0] == "--flat"
    if flat:
        if len(args) != 5:
            sys.exit(__doc__)
        afu_tree, root, gates_dir, ledger_path = args[1:]
        q = os.path.join(root, "syn/syn_top/output_files")
        image, project = os.path.join(q, "d5005.sof"), "d5005"
    else:
        if len(args) != 3:
            sys.exit(__doc__)
        root, gates_dir, ledger_path = args
        afu_tree = root
        q = os.path.join(root, "build/syn/syn_top/output_files")
        image, project = os.path.join(q, "iofs_pr_afu.persona1.gbs"), "iofs_pr_afu"
    if not os.path.isfile(image):
        refuse(f"no image at {image}")
    sha = hashlib.sha256(open(image, "rb").read()).hexdigest()
    ledger = json.load(open(ledger_path)) if os.path.isfile(ledger_path) else {
        "_what": "D5005 accelerator images, each recorded by scripts/d5005/record_afu_image.py only when its fit, its "
                 "timing on every clock and every gate passed: partial-reconfiguration personas (kind pr, a GBS) and "
                 "the FIM built with the accelerator inside it (kind flat, a SOF loaded over JTAG).",
        "images": [],
    }
    if any(i["sha256"] == sha for i in ledger["images"]):
        refuse(f"the ledger already holds the image {sha}")

    fit_path = os.path.join(q, f"{project}.fit.summary")
    if not os.path.isfile(fit_path):
        refuse(f"no fit summary at {fit_path}")
    fit = {}
    for line in open(fit_path):
        if " : " in line:
            k, v = line.split(" : ", 1)
            fit[k.strip()] = v.strip()
    if not fit.get("Fitter Status", "").startswith("Successful"):
        refuse(f"the fitter did not succeed: {fit.get('Fitter Status', 'no status')}")

    timing_dir = os.path.join(q, "timing_report")
    fail, ok = os.path.join(timing_dir, "clocks.sta.fail.summary"), os.path.join(timing_dir, "clocks.sta.pass.summary")
    if not (os.path.isfile(fail) and os.path.isfile(ok)):
        refuse(f"no OFS timing report in {timing_dir}")
    failing = summary_blocks(fail)
    if failing or open(fail).read().strip():
        worst = min(failing, key=lambda b: b["slack"]) if failing else None
        refuse(f"{len(failing)} clock analyses fail timing" + (f", worst {worst['clock']} {worst['type']} {worst['slack']}" if worst else ""))
    passing = summary_blocks(ok)
    if not passing:
        refuse(f"{ok} times no clock")
    least = {}
    for b in passing:
        key = (b["clock"], b["type"])
        least[key] = min(least.get(key, b["slack"]), b["slack"])

    afu_json = json.load(open(os.path.join(afu_tree, "hw/rtl/logos_d5005.json")))
    uclk = {}
    if not flat:
        uclk_path = os.path.join(q, "user_clock_freq.txt")
        if not os.path.isfile(uclk_path):
            refuse(f"no user clock record at {uclk_path}")
        for line in open(uclk_path):
            line = line.split("#")[0].strip()
            m = re.match(r"afu-image/clock-frequency-(high|low)\s*:\s*([0-9.]+)", line)
            if m:
                uclk[m.group(1)] = float(m.group(2))
        if set(uclk) != {"high", "low"}:
            refuse(f"{uclk_path} gives no high and low user clock")

    # The shell's gate reads the clock-domain map, so a tree without one passed no shell gate and built no bridge.
    domains_path = os.path.join(afu_tree, "hw/logos_d5005_domains.json")
    domains = json.load(open(domains_path)) if os.path.isfile(domains_path) else {}
    required = {**GATES, **(BRIDGE_GATES if "logos_engine_bridge" in domains.get("crossings", []) else {})}
    gates = {}
    for name, line in required.items():
        p = os.path.join(gates_dir, name)
        last = open(p).read().strip().splitlines()[-1] if os.path.isfile(p) and open(p).read().strip() else ""
        if not last.startswith(line):
            refuse(f"the gate {name} did not pass: {last or 'no output'}")
        gates[name] = last

    ledger["images"].append({
        "recorded": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "kind": "flat" if flat else "pr",
        "image": os.path.relpath(image, root),
        "sha256": sha,
        "afu_uuid": afu_json["afu-image"]["accelerator-clusters"][0]["accelerator-type-uuid"],
        **({"user_clock_asked": afu_json["afu-image"]["clock-frequency-high"]} if flat else {"user_clock_mhz": uclk}),
        "least_slack_ns": [{"clock": c, "type": t, "slack": s} for (c, t), s in sorted(least.items())],
        "fit": fit,
        "gates": gates,
    })
    os.makedirs(os.path.dirname(os.path.abspath(ledger_path)), exist_ok=True)
    with open(ledger_path, "w") as f:
        json.dump(ledger, f, indent=1)
        f.write("\n")
    clock = f"a user clock set at run time, {afu_json['afu-image']['clock-frequency-high']} asked" if flat else f"{uclk['high']} MHz"
    print(f"record_afu_image: recorded the {'flat' if flat else 'pr'} image {sha[:12]} at {clock}, least slack {min(least.values()):.3f} ns")


if __name__ == "__main__":
    main()
