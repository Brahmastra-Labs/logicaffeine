#!/usr/bin/env python3
"""Generate the LOGOS-on-Tenstorrent perf-vs-cost ($/throughput) Pareto frontier.

Two clearly-separated layers, so nothing is fabricated:

  1. PEAK-COST FRONTIER (published specs).  Peak dense-compute throughput per dollar and per watt
     for the i9-14900K vs Wormhole n150/n300, from `benchmarks/silicon/tenstorrent_power.json`
     (every number sourced).  This is standard AI-accelerator economics: the accelerator's win is
     its low-precision matrix density, which a CPU lacks — the axis is annotated per device.

  2. WORKLOAD PROJECTION (measured i9 + PROJECTED Tenstorrent).  The LOGOS benchmark suite's i9
     throughput is MEASURED (from `benchmarks/results/latest.json`); the single-baby-core and
     whole-card Tenstorrent numbers are PROJECTED from the documented model in the spec file and
     flagged `projected:true`.  `scripts/accel/measure-tenstorrent-pareto.sh` replaces them with MEASURED
     numbers once a card runs the emitted kernels.

Outputs:
  benchmarks/results/tenstorrent_pareto.json          the data + computed frontier
  benchmarks/results/charts/tenstorrent_pareto.svg    a dependency-free one-pager (log-log scatter)
"""
import argparse
import json

# The Tenstorrent ledger's own label for what its counter measures (tt-gemm-ledger.py::PROVENANCE).
PROVENANCE = "sim_functional_not_timed"
import math
import os

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SPECS = os.path.join(ROOT, "benchmarks", "silicon", "tenstorrent_power.json")
BENCH = os.path.join(ROOT, "benchmarks", "results", "latest.json")
# The Tenstorrent GEMM LEDGER: ttsim-measured rows for our emitted programs, the tt-metal examples, and the
# ttnn production op, written ONLY by scripts/tt-gemm-ledger.py (never by hand). Every number here is
# sim_functional_not_timed, NOT on-silicon, and every comparison is read from the ledger, never recomputed here.
SIM = os.path.join(ROOT, "benchmarks", "silicon", "tenstorrent_gemm.json")
OUT_JSON = os.path.join(ROOT, "benchmarks", "results", "tenstorrent_pareto.json")
OUT_SVG = os.path.join(ROOT, "benchmarks", "results", "charts", "tenstorrent_pareto.svg")


def load_specs():
    with open(SPECS) as f:
        return json.load(f)


def sim_workload():
    """ttsim (cycle-accurate) throughput for the CURRENT ledger rows + the ledger's own matched-numerics
    comparisons. Absent until a program has actually run under the simulator. Every number here is tagged
    `sim_functional_not_timed` — the simulator runs the program faithfully but does not advance its clocks with device execution, so these counts identify and order programs rather than timing them; NOT projected and NOT measured on silicon. The stack
    labels are the ledger's: logos (the compiler's emitted program), tt_metal_example (a stock programming
    example), ttnn (the vendor's production op); every row names its math fidelity."""
    if not os.path.exists(SIM):
        return {"available": False, "provenance": PROVENANCE,
                "note": "run scripts/tt-matmul-measure.sh all to populate benchmarks/silicon/tenstorrent_gemm.json"}
    with open(SIM) as f:
        d = json.load(f)
    clock = d.get("clock_ghz", 1.0)
    stack_label = {"logos_emitted": "logos", "tt_metal_example": "tt_metal_example", "ttnn_production": "ttnn"}
    rows = {r["id"]: r for r in d.get("rows", [])}
    current = [r for r in d.get("rows", []) if r.get("current")]
    if not current:
        return {"available": False, "provenance": PROVENANCE, "note": "the ledger has no rows yet"}
    kernels = []
    for r in current:
        m, n, k = r["shape"]["m"], r["shape"]["n"], r["shape"]["k"]
        ops = 2 * m * n * k
        cyc = r["cycles"]
        cfg = r.get("cfg") or {}
        layout = "single_core" if (cfg.get("grid") == "1x1" or "single_core" in r["program"]) else "multi_core_" + r["strategy"].lower()
        kernels.append({
            "name": "%s/%s" % (r["program"], r["strategy"]), "row": r["id"], "shape": [m, n, k],
            "stack": stack_label[r["stack"]], "program_origin": r["program_origin"], "layout": layout,
            "precision": "bf16", "fidelity": r["knobs"]["fidelity"], "fp32_dest_acc": r["knobs"]["fp32_dest_acc"],
            "packer_l1_acc": r["knobs"]["packer_l1_acc"], "ops_flop": ops, "cycles": cyc, "correct": r["pass"],
            "pcc": r["pcc"], "throughput_ops_s": ops / (cyc / (clock * 1e9)), "provenance": PROVENANCE,
        })
    # The ledger's matched-numerics comparisons, READ not recomputed (the pareto JSON can never disagree).
    ratios = []
    for c in d.get("comparisons", []):
        subj = rows[c["subject"]]
        for name, v in c["vs"].items():
            entries = v.items() if name == "tt_metal_example" else [(name, v)]
            for label, e in entries:
                o = rows[e["row"]]
                ratios.append({
                    "shape": [subj["shape"]["m"], subj["shape"]["n"], subj["shape"]["k"]],
                    "knobs": c["knobs"], "matched_numerics": True,
                    "logos_cycles": subj["cycles"], "baseline_cycles": o["cycles"],
                    "baseline_stack": stack_label[o["stack"]], "baseline_program": label,
                    "baseline_origin": o["program_origin"], "logos_over_baseline": e["ratio"],
                })
    return {"available": True, "provenance": PROVENANCE, "device": current[0]["chip"],
            "clock_ghz": clock, "kernels": kernels, "logos_vs_baseline": ratios,
            "headline": d.get("headline"), "unmatched_pairs": len(d.get("unmatched_comparisons", []))}


def peak_frontier(devices):
    """Per device: tops/$ and tops/W, plus Pareto membership on (price down, peak_tops up)."""
    rows = []
    for d in devices:
        price = d["price_usd"]
        tops = d["peak_tops"]
        tdp = d["tdp_w"]
        rows.append({
            "id": d["id"],
            "label": d["label"],
            "kind": d["kind"],
            "price_usd": price,
            "tdp_w": tdp,
            "peak_tops": tops,
            "peak_precision": d["peak_precision"],
            "tops_per_dollar": tops / price,
            "tops_per_watt": tops / tdp,
            "measured": False,
            "published": True,
            "source": d["peak_source"],
        })
    # Pareto-optimal on (minimize price, maximize peak_tops): no other point beats it on both.
    for r in rows:
        dominated = any(
            (o["price_usd"] <= r["price_usd"] and o["peak_tops"] >= r["peak_tops"] and o["id"] != r["id"]
             and (o["price_usd"] < r["price_usd"] or o["peak_tops"] > r["peak_tops"]))
            for o in rows
        )
        r["pareto_optimal"] = not dominated
    return rows


def geomean(xs):
    xs = [x for x in xs if x and x > 0]
    if not xs:
        return None
    return math.exp(sum(math.log(x) for x in xs) / len(xs))


def i9_logos_throughput():
    """MEASURED: geomean of benchmark-completions/sec (1000/mean_ms) for logos_release on the i9."""
    if not os.path.exists(BENCH):
        return None, 0
    with open(BENCH) as f:
        data = json.load(f)
    rates = []
    for b in data.get("benchmarks", []):
        ref = b.get("reference_size")
        scaling = b.get("scaling", {})
        cell = scaling.get(str(ref), {}).get("logos_release") if ref is not None else None
        if cell and cell.get("mean_ms", 0) > 0:
            rates.append(1000.0 / cell["mean_ms"])
    return geomean(rates), len(rates)


def workload_projection(specs):
    model = specs["projection_model"]
    i9_rate, n = i9_logos_throughput()
    scale = (model["baby_core_clock_ghz"] / model["i9_pcore_clock_ghz"]) * model["ipc_ratio_babycore_vs_i9"]
    out = {
        "metric": "benchmark completions/sec (LOGOS, geomean over the ≡VM benchmark suite)",
        "note": "i9 is MEASURED; every Tenstorrent number here is PROJECTED from the stated model — replaced by MEASURED once a card runs the emitted kernels.",
        "model": model,
    }
    if i9_rate is None:
        out["available"] = False
        return out
    out["available"] = True
    out["benchmarks_counted"] = n
    babycore = i9_rate * scale
    out["i9_measured"] = {"throughput": i9_rate, "measured": True, "source": "benchmarks/results/latest.json logos_release"}
    out["tt_babycore_projected"] = {"throughput": babycore, "projected": True}
    for dev, cores in model["compute_baby_cores_per_card"].items():
        out[f"tt_{dev}_card_projected"] = {
            "throughput": babycore * cores,
            "baby_cores": cores,
            "projected": True,
            "speedup_vs_i9": (babycore * cores) / i9_rate,
        }
    return out


def power_layer(sim, frontier):
    """Power stays `pending_hardware` (ttsim gives cycles, not watts). But we can bound perf/watt from
    the REAL sim cycle-time × the PUBLISHED TDP: a TDP-upper-bound energy → a perf/watt LOWER bound.
    Clearly labeled `projected_tdp_upper_bound` — real power is lower, filled in by a Koyeb tt-smi run."""
    power = {"provenance": "pending_hardware",
             "note": "ttsim reports cycles, not watts — measured power needs a card or Tenstorrent Cloud (tt-smi)"}
    if not sim.get("available"):
        return power
    head = sim.get("headline") or {}
    pe = next((k for k in sim["kernels"] if k.get("row") == head.get("subject")), None)
    n150 = next((r for r in frontier if r["id"] == "wormhole_n150"), None)
    if pe and n150 and pe.get("throughput_ops_s") and pe.get("ops_flop"):
        time_s = pe["cycles"] / (sim["clock_ghz"] * 1e9)
        e_upper_j = n150["tdp_w"] * time_s  # upper bound: full TDP for the whole kernel duration
        power["projected_upper_bound"] = {
            "provenance": "projected_tdp_upper_bound",
            "basis": "n150 TDP (published) x the REAL ttsim kernel-time — real matmul power is LOWER, so these bound the truth",
            "energy_joules": e_upper_j,
            "energy_pj_per_flop": e_upper_j / pe["ops_flop"] * 1e12,
            "tflops_per_watt_lower_bound": (pe["throughput_ops_s"] / 1e12) / n150["tdp_w"],
            "note": "perf/watt LOWER bound (TDP is an over-estimate of matmul power); replaced by measured_hw on a Koyeb tt-smi run",
        }
    return power


def svg(frontier, headline):
    """A dependency-free log-log scatter of price (x) vs peak_tops (y), annotated with tops/$."""
    W, H, pad = 720, 460, 70
    xs = [r["price_usd"] for r in frontier]
    ys = [r["peak_tops"] for r in frontier]
    lx0, lx1 = math.log10(min(xs) * 0.8), math.log10(max(xs) * 1.25)
    ly0, ly1 = math.log10(min(ys) * 0.5), math.log10(max(ys) * 2.0)

    def px(x):
        return pad + (math.log10(x) - lx0) / (lx1 - lx0) * (W - 2 * pad)

    def py(y):
        return H - pad - (math.log10(y) - ly0) / (ly1 - ly0) * (H - 2 * pad)

    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" font-family="monospace">',
        f'<rect width="{W}" height="{H}" fill="#0b0f14"/>',
        f'<text x="{pad}" y="30" fill="#00d4ff" font-size="18">LOGOS on Tenstorrent — peak throughput per dollar</text>',
        f'<text x="{pad}" y="50" fill="#8aa" font-size="12">x: price (USD, log)   y: peak dense-compute TOPS (log, native AI precision)   published specs</text>',
        f'<line x1="{pad}" y1="{H-pad}" x2="{W-pad}" y2="{H-pad}" stroke="#334"/>',
        f'<line x1="{pad}" y1="{pad}" x2="{pad}" y2="{H-pad}" stroke="#334"/>',
    ]
    for r in frontier:
        cx, cy = px(r["price_usd"]), py(r["peak_tops"])
        color = "#00d4ff" if r["kind"] == "accelerator" else "#c98"
        parts.append(f'<circle cx="{cx:.1f}" cy="{cy:.1f}" r="7" fill="{color}"/>')
        parts.append(
            f'<text x="{cx+11:.1f}" y="{cy-6:.1f}" fill="#dde" font-size="12">{r["label"]}</text>'
        )
        parts.append(
            f'<text x="{cx+11:.1f}" y="{cy+9:.1f}" fill="#7c9" font-size="11">'
            f'{r["peak_tops"]:.0f} TOPS ({r["peak_precision"]}) · {r["tops_per_dollar"]:.3f} TOPS/$ · {r["tops_per_watt"]:.2f} TOPS/W</text>'
        )
    parts.append(
        f'<text x="{pad}" y="{H-20}" fill="#00d4ff" font-size="13">'
        f'{headline["best_tops_per_dollar_label"]}: best TOPS/$ — {headline["tt_vs_cpu_tops_per_dollar_ratio"]:.0f}x the CPU</text>'
    )
    parts.append('</svg>')
    return "\n".join(parts)


def main():
    ap = argparse.ArgumentParser(description="The Tenstorrent Pareto frontier from the committed specs, benchmarks and ledger.")
    ap.add_argument("--out-dir", default=os.path.dirname(OUT_JSON),
                    help="where tenstorrent_pareto.json and charts/tenstorrent_pareto.svg go (default: the committed "
                         "benchmarks/results/; a lock writes a scratch directory and compares it to the committed copy)")
    out_json = os.path.join(ap.parse_args().out_dir, os.path.basename(OUT_JSON))
    out_svg = os.path.join(os.path.dirname(out_json), "charts", os.path.basename(OUT_SVG))
    specs = load_specs()
    frontier = peak_frontier(specs["devices"])
    best = max(frontier, key=lambda r: r["tops_per_dollar"])
    cpu = next((r for r in frontier if r["kind"] == "cpu"), None)
    ratio = (best["tops_per_dollar"] / cpu["tops_per_dollar"]) if cpu else None
    headline = {
        "best_tops_per_dollar": best["id"],
        "best_tops_per_dollar_label": best["label"],
        "tt_vs_cpu_tops_per_dollar_ratio": ratio,
        "cpu_id": cpu["id"] if cpu else None,
    }
    sim = sim_workload()
    status = (
        "SIM — cycle-accurate ttsim numbers present; power still pending_hardware."
        if sim.get("available")
        else "PROJECTED — no ttsim run yet. Peak layer is published specs; workload layer is a labeled projection."
    )
    result = {
        "generated_by": "scripts/accel/gen-pareto-frontier.py",
        "status": status,
        "axes": {"x": "price_usd", "y": "peak_tops", "efficiency": "tops_per_dollar"},
        "peak_frontier": frontier,
        "headline": headline,
        # The labeled projection (guessed-IPC) — superseded per-kernel by `sim_workload` once ttsim runs.
        "workload_projection": workload_projection(specs),
        # Real simulated cycles (Phase C). `available:false` until a kernel runs under ttsim.
        "sim_workload": sim,
        "power": power_layer(sim, frontier),
    }
    os.makedirs(os.path.dirname(out_json), exist_ok=True)
    os.makedirs(os.path.dirname(out_svg), exist_ok=True)
    with open(out_json, "w") as f:
        json.dump(result, f, indent=2)
    with open(out_svg, "w") as f:
        f.write(svg(frontier, headline))

    print(f"wrote {os.path.relpath(out_json, ROOT)} and {os.path.relpath(out_svg, ROOT)}")
    print(f"headline: {best['label']} = {best['tops_per_dollar']:.3f} TOPS/$ "
          f"({ratio:.0f}x the i9-14900K)" if ratio else "")
    wl = result["workload_projection"]
    if wl.get("available"):
        print(f"workload (PROJECTED): i9 {wl['i9_measured']['throughput']:.1f} → "
              f"n150 card {wl['tt_wormhole_n150_card_projected']['throughput']:.1f} "
              f"({wl['tt_wormhole_n150_card_projected']['speedup_vs_i9']:.1f}x, projected)")


if __name__ == "__main__":
    main()
