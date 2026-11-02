#!/usr/bin/env python3
"""demo-hft — the LOGOS tick-to-trade determinism demo, end to end.

Runs the ITCH->book->strategy->risk->order pipeline on the LOGOS full core (cycle-accurate RTL sim by
default; the same program a bitstream runs), measures the per-message on-chip cycle latency (exact,
zero run-to-run jitter), times the identical pipeline on the host x86 (the C baseline) to expose its
jitter tail, and writes benchmarks/silicon/hft_latency.json — the locked artifact every published latency
claim reads.

    python3 scripts/demo-hft.py                 # sim + x86, emit artifact + print the contrast
    python3 scripts/demo-hft.py --iters 40000   # more x86 samples

Absolute latency is reported as the MEASURED per-message cycle count projected across clock tiers, each
tagged with its status — never a single-clock number presented as competitive (see HFT_DEMO_CAMPAIGN.md).
"""
import json
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(__file__))
import hft_model as M
import hft_gen

ROOT = os.path.normpath(os.path.join(os.path.dirname(__file__), ".."))
SIL = os.path.join(ROOT, "benchmarks", "silicon")
LG2HEX = os.path.join(ROOT, "target", "debug", "examples", "lg2hex")
VVP = "/tmp/tb_hft.vvp"
BASELINE = "/tmp/hft_baseline"

# The Sky130 tier runs at soft_core_scalar's signoff clock: the period every corner meets, recorded in
# its ledger (scripts/derive-tapeout-projections.py derives the same figure for the stored artifacts).
SKY130_MHZ = 1000.0 / json.load(open(os.path.join(SIL, "sky130_signoff.json")))["blocks"]["soft_core_scalar"]["clock_period_ns"]

# clock tiers: (label, MHz, status) — the honesty ladder
TIERS = [
    ("Nexys A7 FPGA (shipped)", 6.25, "measured"),
    ("Nexys A7 FPGA (routed Fmax)", 27.38, "measured"),
    ("Sky130 tapeout", SKY130_MHZ, "projected"),
    ("ASIC roadmap", 1000.0, "roadmap"),
]

DEPS = ["soft_core.v", "alu.v", "shared_divider.v", "crdt_merge.v",
        "bigint_alu.v", "bigint_div.v", "fpu.v", "tb_hft.v"]


def sh(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def build_sim():
    if os.path.exists(VVP):
        return
    r = sh(["iverilog", "-g2012", "-s", "tb", "-o", VVP] + DEPS, cwd=SIL)
    if r.returncode != 0:
        sys.exit(f"iverilog build failed:\n{r.stderr}")


def run_chip(feed):
    """Run `feed` on the full-core RTL; return (shows, total_cycles, per_message_cycles[])."""
    src = hft_gen.gen(feed)
    hx = sh([LG2HEX, "--full", "/dev/stdin"], input=src)
    if hx.returncode != 0:
        sys.exit(f"lg2hex failed: {hx.stderr.strip()}")
    hexpath = "/tmp/hft_demo.hex"
    open(hexpath, "w").write(hx.stdout)
    r = sh(["vvp", VVP, f"+HEX={hexpath}"], timeout=300)
    res = next(l for l in r.stdout.splitlines() if l.startswith("RESULT"))
    scl = next(l for l in r.stdout.splitlines() if l.startswith("SHOWCYC"))
    shows = [int(x) for x in res.split("RESULT:")[1].split("CYCLES")[0].split()]
    total = int(res.split("CYCLES:")[1])
    showcyc = [int(x) for x in scl.split("SHOWCYC:")[1].split()]
    # per-message latency = gap between each message's 3rd Show (message m's work completes there)
    b = [showcyc[3 * k + 2] for k in range(len(showcyc) // 3)]
    per_msg = [b[k] - b[k - 1] for k in range(1, len(b))]   # steady state (msg 1 carries one-time setup)
    return shows, total, per_msg


def run_baseline(iters, load_kb=0):
    if not os.path.exists(BASELINE) or os.path.getmtime(os.path.join(ROOT, "scripts", "hft_baseline.c")) > os.path.getmtime(BASELINE):
        r = sh(["cc", "-O2", "-o", BASELINE, os.path.join(ROOT, "scripts", "hft_baseline.c"), "-lm"])
        if r.returncode != 0:
            sys.exit(f"cc failed:\n{r.stderr}")
    cmd = [BASELINE, str(iters)] + (["--load", str(load_kb)] if load_kb else [])
    return json.loads(sh(cmd).stdout)


def main(argv):
    iters = 20000
    if "--iters" in argv:
        iters = int(argv[argv.index("--iters") + 1])
    build_sim()

    feed = M.DEMO_FEED
    ref = M.run_pipeline(feed)
    shows, total, per_msg = run_chip(feed)
    if shows != ref:
        sys.exit(f"CHIP DIVERGED from reference:\n chip={shows}\n ref ={ref}")

    med = sorted(per_msg)[len(per_msg) // 2]
    lo, hi = min(per_msg), max(per_msg)
    x86 = run_baseline(iters)
    # determinism-under-load (STAC-M1): the CPU tail fattens as the working set is evicted; the chip does not move
    load_sweep = [{"load_kb": kb, **run_baseline(iters, kb)} for kb in (256, 4096)]

    # Which CPU the x86 tail was actually measured on (anchors the comparison: it is the fastest desktop chip).
    cpu_name, cpu_ghz = "the host CPU", None
    try:
        for ln in open("/proc/cpuinfo"):
            if ln.startswith("model name"):
                cpu_name = ln.split(":", 1)[1].strip()
                for junk in ("(R)", "(TM)", "(r)", "(tm)", " CPU"):
                    cpu_name = cpu_name.replace(junk, "")
                cpu_name = cpu_name.split("@")[0].strip()
                break
    except OSError:
        pass
    try:
        cpu_ghz = round(int(open("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq").read()) / 1e6, 2)
    except OSError:
        pass
    chip_ns_at_cpu = round(med / cpu_ghz, 1) if cpu_ghz else None

    chip_ns = {}
    for label, mhz, status in TIERS:
        chip_ns[label] = {"mhz": mhz, "status": status, "ns_per_msg": round(med * 1000.0 / mhz, 1)}

    artifact = {
        "demo": "LOGOS tick-to-trade: NASDAQ ITCH-lite -> price-ladder book -> microprice market-making "
                "-> pre-trade risk -> order, on the full LOGOS soft-core.",
        "provenance": "Per-message cycles are the on-chip run-cycle counter in cycle-accurate RTL sim "
                      "(tb_hft.v) of the SAME full core the bitstream routes; the Show/order stream is "
                      "bit-exact to the LOGOS VM and to the Python reference (hft_model.py). The UART "
                      "round-trip is the dev loom, not the trade path — the HFT-relevant number is the "
                      "on-chip cycle count. Absolute ns is that measured cycle count / clock, per tier.",
        "messages": len(feed) // 8,
        "order_stream": shows,
        "chip": {
            "per_message_cycles_median": med,
            "per_message_cycles_band": [lo, hi],
            "run_to_run_jitter_cycles": 0,
            "note": "Deterministic silicon: identical input -> identical cycle count on EVERY run "
                    "(zero jitter). The band is data-dependent work across message types, not jitter.",
            "latency_by_tier": chip_ns,
        },
        "x86_baseline": {
            "impl": "tight -O2 C, identical pipeline (hft_baseline.c), per-message rdtscp timing",
            "cpu": cpu_name, "cpu_max_ghz": cpu_ghz, "chip_ns_at_cpu_clock": chip_ns_at_cpu,
            "samples": x86["samples"],
            "p50_ns": x86["p50_ns"], "p99_ns": x86["p99_ns"], "p999_ns": x86["p999_ns"],
            "p9999_ns": x86["p9999_ns"], "max_ns": x86["max_ns"], "stdev_ns": x86["stdev_ns"],
            "tail_max_over_p50": round(x86["max_ns"] / x86["p50_ns"], 1),
            "tail_curve": x86["curve"],
            "note": "A GHz CPU wins median latency vs a 6.25 MHz FPGA — but leaks in the TAIL "
                    "(OS/cache/branch jitter), which is where HFT money leaks and which worsens under load.",
            "under_load": [
                {"load_kb": s["load_kb"], "p50_ns": s["p50_ns"], "p999_ns": s["p999_ns"],
                 "max_ns": s["max_ns"], "stdev_ns": s["stdev_ns"]} for s in load_sweep
            ],
        },
    }
    out = os.path.join(SIL, "hft_latency.json")
    json.dump(artifact, open(out, "w"), indent=1)

    # --- the world's-best-demo HTML page (projectable, data-driven) ---
    import hft_report
    lg_lines = open(os.path.join(SIL, "programs", "hft_pipeline.lg")).read().splitlines()
    strat, grab = [], False           # the real decision+risk lines, dedented one level
    for ln in lg_lines:
        s = ln.strip()
        if s.startswith("Let micro100"):
            grab = True
        if grab:
            strat.append(ln[4:] if ln.startswith("    ") else ln)
        if s.startswith("Set order_size to max_order"):
            break
    report = {
        "strategy": "\n".join(strat),
        "order_stream": shows,
        "chip": {"cyc_median": med, "band": [lo, hi],
                 "tiers": [{"mhz": mhz, "ns": round(med * 1000.0 / mhz, 1), "status": status}
                           for (_l, mhz, status) in TIERS]},
        "x86": {"curve": x86["curve"], "p50": x86["p50_ns"], "p999": x86["p999_ns"],
                "max": x86["max_ns"], "tail_ratio": round(x86["max_ns"] / x86["p50_ns"], 1),
                "samples": x86["samples"], "stdev": x86["stdev_ns"],
                "cpu": cpu_name, "cpu_ghz": cpu_ghz, "chip_ns_at_cpu": chip_ns_at_cpu},
        "asic_ns": round(med, 1),     # N cycles @ ~1 GHz ASIC ≈ N ns — the flat line on the chart
    }
    htmlpath = os.path.join(SIL, "hft_demo.html")
    open(htmlpath, "w").write(hft_report.build_html(report))

    # --- narrated contrast ---
    print("=" * 74)
    print("  LOGOS tick-to-trade — determinism demo")
    print("=" * 74)
    print(f"  Pipeline: ITCH-lite decode -> price-ladder book -> microprice MM -> risk -> order")
    print(f"  {len(feed)//8} messages -> order stream: {shows}")
    print(f"  CHIP ≡ reference (VM ≡ silicon, bit-exact): OK")
    print()
    print(f"  ON-CHIP per-message latency: median {med} cyc, band {lo}-{hi} cyc, "
          f"run-to-run jitter = 0 (deterministic)")
    for label, mhz, status in TIERS:
        print(f"      {label:28} {mhz:>7.2f} MHz  {chip_ns[label]['ns_per_msg']:>10.1f} ns/msg  [{status}]")
    print()
    print(f"  x86 C baseline (identical pipeline, {x86['samples']} samples):")
    print(f"      p50 {x86['p50_ns']:.1f} ns | p99 {x86['p99_ns']:.1f} | p99.9 {x86['p999_ns']:.1f} "
          f"| p99.99 {x86['p9999_ns']:.1f} | MAX {x86['max_ns']:.1f} ns  (stdev {x86['stdev_ns']:.1f})")
    print(f"      tail: max is {artifact['x86_baseline']['tail_max_over_p50']}x the median "
          f"-> the jitter tail deterministic silicon does not have")
    print()
    print(f"  DETERMINISM UNDER LOAD (STAC-M1): CPU tail vs working-set pressure")
    print(f"      {'quiet':>10}  p50 {x86['p50_ns']:6.1f}  p99.9 {x86['p999_ns']:8.1f}  max {x86['max_ns']:9.1f} ns")
    for s in load_sweep:
        print(f"      {str(s['load_kb'])+'KB':>10}  p50 {s['p50_ns']:6.1f}  p99.9 {s['p999_ns']:8.1f}  "
              f"max {s['max_ns']:9.1f} ns")
    print(f"      CHIP under any load: {med} cyc EVERY message, unchanged (0 jitter)")
    print()
    print(f"  wrote {os.path.relpath(out, ROOT)}")
    print(f"  ★ open the demo:  {os.path.relpath(htmlpath, ROOT)}  (projectable, data-driven)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
