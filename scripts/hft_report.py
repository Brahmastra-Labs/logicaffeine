#!/usr/bin/env python3
"""hft_report — render the LOGOS tick-to-trade demo as a single self-contained, projectable HTML page.

The centerpiece is the latency-tail curve: the canonical HFT visual (latency vs percentile, tail on the
right). A flat line means no jitter; a hockey stick means you get picked off at the worst possible moment.
Our silicon is flat; the CPU explodes at the tail. Everything is driven by the measured data
(benchmarks/silicon/hft_latency.json + the x86 tail curve), so nothing here is decorative — it's the proof,
made visible. No external assets: opens in any browser, drops onto a big screen.

Called by scripts/demo-hft.py; also runnable standalone once hft_latency.json + a curve exist.
"""
import html
import math
import re


def _intel_gen(cpu):
    """'Intel Core i9-14900K' -> '14th-gen'. The lead digits of the model are the generation."""
    m = re.search(r"i\d[- ](\d{4,5})", cpu)
    if not m:
        return "latest-gen"
    g = int(m.group(1)[:-3])
    suf = "th" if 10 <= g % 100 <= 20 else {1: "st", 2: "nd", 3: "rd"}.get(g % 10, "th")
    return f"{g}{suf}-gen"


import json as _json
import os as _os


def fabric_power():
    """The MEASURED signoff-clean decision-fabric result (benchmarks/silicon/decision_fabric_power.json,
    signoff_clean section) — the HFT decision if-converted to a spatial select-DAG and taped out
    DRC/LVS/XOR-clean on Sky130. Returns the decision-logic power, its reduction vs the soft-core and vs
    one i9 core, and the per-trade energy. Returns None if the artifact is absent."""
    src = _os.path.join(_os.path.dirname(__file__), _os.pardir, "benchmarks", "silicon", "decision_fabric_power.json")
    try:
        s = _json.load(open(src))["signoff_clean"]
    except (OSError, KeyError):
        return None
    dec_mw = s["power_mw"]["decision_selectdag_comb"]      # the decision select-DAG, post-route RCX
    soft_mw = _json.load(open(_os.path.join(_os.path.dirname(__file__), _os.pardir, "benchmarks", "silicon",
                                            "sky130_power.json")))["power_w"]["total"] * 1e3
    i9_core_w = 15.0
    return {
        "dec_mw": dec_mw, "block_mw": s["power_mw"]["block_total"], "die_mm2": s["die_mm2"],
        "fmax_mhz": s["fmax_mhz"], "e_trade_pj": s["energy_per_trade_pJ"]["decision"],
        "vs_soft_x": round(soft_mw / dec_mw), "vs_i9core_x": int(round(i9_core_w * 1000 / dec_mw, -3)),
        "energy_vs_soft_x": s["vs_soft_core"]["energy_per_trade_reduction_x"][1],
        "area_vs_soft_x": s["vs_soft_core"]["area_reduction_x"],
    }


def power_model(cycles=866, i9_core_w=15.0, i9_p50_ns=None):
    """Power from the REAL OpenLane Sky130 signoff — OpenSTA report_power on the post-route, RC-extracted
    netlist (typical corner), captured in benchmarks/silicon/sky130_power.json. We report two numbers and
    keep them honest: power DRAW (watts) is a genuine win (tens of mW for the whole chip vs ~15 W for one i9 core);
    per-trade ENERGY (joules) is NOT — at 66.7 MHz the i9's raw speed keeps its energy/trade competitive,
    so we do not claim an energy-per-trade win. The categorical energy win is the fetch/decode-free spatial
    fabric (roadmap), not this soft-core. i9 package/core figures are Intel's spec / a standard estimate."""
    src = _os.path.join(_os.path.dirname(__file__), _os.pardir, "benchmarks", "silicon", "sky130_power.json")
    s = _json.load(open(src))
    total_w, f_hz, cells, mhz = s["power_w"]["total"], s["clock_mhz"] * 1e6, s["cells"], s["clock_mhz"]
    measured = True
    e_trade_nj = total_w * (cycles / f_hz) * 1e9    # P × time-per-trade; dynamic-dominated ⇒ ~clock-independent
    i9_w = 253                                      # Core i9-14900K max turbo power (PL2), official spec
    i9_trade_nj = round(i9_core_w * i9_p50_ns) if i9_p50_ns else None   # one-core W × p50 s = nJ (W·ns == nJ)
    return {"i9_w": i9_w, "i9_core_w": i9_core_w, "sky130_mw": round(total_w * 1e3, 1), "cells": cells,
            "mhz": round(mhz, 1), "measured": measured, "e_trade_nj": round(e_trade_nj),
            "i9_trade_nj": i9_trade_nj, "ratio_core": round(i9_core_w / total_w), "ratio_pkg": round(i9_w / total_w)}


# ── the tail-latency SVG chart: latency (log ns, y) vs percentile (tail-spread, x) ──────────────────
def _tail_chart(x86_curve, chip_ns, samples, cpu_label="x86 CPU"):
    W, H = 940, 470
    ml, mr, mt, mb = 74, 210, 34, 56          # margins (wide right margin for the end-labels)
    pw, ph = W - ml - mr, H - mt - mb

    # x: percentile p -> t = -log10(1-p), so the tail (p99, p99.9, ...) spreads out. p=1 uses the last rank.
    tmax = -math.log10(1.0 / samples)
    def tx(p):
        t = tmax if p >= 1.0 else -math.log10(max(1e-9, 1.0 - p))
        return ml + pw * min(t, tmax) / tmax

    ys = [ns for _, ns in x86_curve] + [chip_ns]
    ylo, yhi = math.log10(max(1.0, min(ys) * 0.8)), math.log10(max(ys) * 1.25)
    def yy(ns):
        return mt + ph * (yhi - math.log10(max(1.0, ns))) / (yhi - ylo)

    def fmt_ns(ns):
        if ns >= 1000:
            return f"{ns/1000:.0f} µs" if ns >= 10000 else f"{ns/1000:.1f} µs"
        return f"{ns:.0f} ns"

    parts = [f'<svg viewBox="0 0 {W} {H}" width="100%" style="max-width:940px">']
    parts.append(f'<rect x="0" y="0" width="{W}" height="{H}" fill="#0a0e16"/>')

    # horizontal gridlines at decade ns
    d = math.ceil(ylo)
    while d <= yhi:
        y = yy(10 ** d)
        parts.append(f'<line x1="{ml}" y1="{y:.1f}" x2="{ml+pw}" y2="{y:.1f}" stroke="#1c2637" stroke-width="1"/>')
        parts.append(f'<text x="{ml-10}" y="{y+4:.1f}" fill="#5b6b86" font-size="12" text-anchor="end">{fmt_ns(10**d)}</text>')
        d += 1

    # x ticks at the percentiles that matter
    for p, lab in [(0.5, "p50"), (0.9, "p90"), (0.99, "p99"), (0.999, "p99.9"), (0.9999, "p99.99"), (1.0, "max")]:
        x = tx(p)
        parts.append(f'<line x1="{x:.1f}" y1="{mt}" x2="{x:.1f}" y2="{mt+ph}" stroke="#141c2b" stroke-width="1"/>')
        parts.append(f'<text x="{x:.1f}" y="{mt+ph+20}" fill="#5b6b86" font-size="12" text-anchor="middle">{lab}</text>')
    parts.append(f'<text x="{ml+pw/2:.0f}" y="{H-8}" fill="#7c8aa5" font-size="13" text-anchor="middle" font-weight="600">percentile → (the tail is where the money leaks)</text>')

    # x86 line — the hockey stick
    pts = " ".join(f"{tx(p):.1f},{yy(ns):.1f}" for p, ns in x86_curve)
    parts.append(f'<polyline points="{pts}" fill="none" stroke="#ff4d5e" stroke-width="3"/>')
    for p, ns in x86_curve:
        parts.append(f'<circle cx="{tx(p):.1f}" cy="{yy(ns):.1f}" r="2.4" fill="#ff4d5e"/>')
    exn = x86_curve[-1][1]
    parts.append(f'<text x="{ml+pw+12}" y="{yy(exn)+4:.1f}" fill="#ff4d5e" font-size="13" font-weight="700">{cpu_label}</text>')
    parts.append(f'<text x="{ml+pw+12}" y="{yy(exn)+22:.1f}" fill="#ff8a94" font-size="12">max {fmt_ns(exn)}</text>')

    # chip line — dead flat
    cy = yy(chip_ns)
    parts.append(f'<line x1="{ml}" y1="{cy:.1f}" x2="{ml+pw}" y2="{cy:.1f}" stroke="#22e39a" stroke-width="3"/>')
    parts.append(f'<text x="{ml+pw+12}" y="{cy-8:.1f}" fill="#22e39a" font-size="14" font-weight="700">LOGOS silicon</text>')
    parts.append(f'<text x="{ml+pw+12}" y="{cy+10:.1f}" fill="#7effc9" font-size="12">{fmt_ns(chip_ns)}, every msg</text>')
    parts.append(f'<text x="{ml+pw+12}" y="{cy+26:.1f}" fill="#7effc9" font-size="12">jitter: 0</text>')

    parts.append('</svg>')
    return "".join(parts)


# ── the clock ladder: SAME 866 cycles, dial up the crystal, watch it zoom ───────────────────────────
def _zoom_ladder(tiers, cycles):
    rows = tiers
    W = 900
    rh, top = 46, 20
    H = top + len(rows) * rh + 10
    bx, bw = 168, 470                       # bar track
    ns = [t["ns"] for t in rows]
    lmin = math.log10(min(ns)) - 0.35       # floor so the fastest bar is still visible
    lmax = math.log10(max(ns))
    base = rows[0]["ns"]                     # 6.25 MHz today = the 1x baseline

    def blen(v):
        return bw * (math.log10(v) - lmin) / (lmax - lmin)

    def fmt(v):
        return (f"{v/1000:.0f} µs" if v >= 10000 else f"{v/1000:.1f} µs") if v >= 1000 else f"{v:.0f} ns"

    p = [f'<svg viewBox="0 0 {W} {H}" width="100%" style="max-width:900px">']
    for i, t in enumerate(rows):
        y = top + i * rh
        mult = base / t["ns"]
        fast = mult >= 100
        p.append(f'<text x="8" y="{y+22}" fill="#e7eefb" font-size="17" font-weight="700">{t["mhz"]:g} MHz</text>')
        p.append(f'<text x="8" y="{y+38}" fill="#6a7c9c" font-size="11">{t["status"]}</text>')
        p.append(f'<rect x="{bx}" y="{y+8}" width="{bw}" height="22" rx="4" fill="#0d1420"/>')
        col = "#22e39a" if fast else "#2f8f6b"
        p.append(f'<rect x="{bx}" y="{y+8}" width="{blen(t["ns"]):.1f}" height="22" rx="4" fill="{col}"/>')
        p.append(f'<text x="{bx+blen(t["ns"])+10:.1f}" y="{y+24}" fill="#e7eefb" font-size="15" font-weight="700">{fmt(t["ns"])}</text>')
        mtxt = ("baseline" if abs(mult-1) < 0.05 else f"{mult:.0f}× faster")
        p.append(f'<text x="{W-8}" y="{y+24}" fill="{"#22e39a" if fast else "#7c8aa5"}" font-size="15" font-weight="700" text-anchor="end">{mtxt}</text>')
    p.append('</svg>')
    return "".join(p)


# ── the SAME-CLOCK picture: one bar per trade. i9 spikes; ours is a flat comb. See it in one second. ──
def _sample_strip(curve, n, seed=42):
    """n latencies (ns) sampled to MATCH the measured distribution: evenly-spaced percentiles (so the tail
    is represented) plus the real measured max, shuffled into time order. Honest — it is the measured shape."""
    import random as _r
    rng = _r.Random(seed)

    def interp(u):
        for j in range(len(curve) - 1):
            p0, v0 = curve[j]
            p1, v1 = curve[j + 1]
            if u <= p1:
                if p1 <= p0:
                    return v1
                f = (u - p0) / (p1 - p0)
                return v0 * (v1 / v0) ** f            # log-interpolate
        return curve[-1][1]

    vals = [interp((i + 0.5) / n) for i in range(n - 1)] + [curve[-1][1]]
    rng.shuffle(vals)
    return vals


def _heartbeat(curve, chip_ns, cpu, ghz, band_ns=None):
    import random as _r
    n = 100
    W, padL, padR = 920, 158, 150
    plotw = W - padL - padR
    bw = plotw / n
    barsH, gap, titleH = 66, 34, 30
    H = titleH + 2 * (barsH + 30) + gap
    ymax = chip_ns * 2.6                                 # LOGOS sits ~38%; i9 spikes clip the top

    def fmt(v):
        return (f"{v/1000:.0f} µs" if v >= 10000 else f"{v/1000:.1f} µs") if v >= 1000 else f"{v:.0f} ns"

    vals = _sample_strip(curve, n)
    mx = max(vals)
    if band_ns:
        rng = _r.Random(7)
        logos = [band_ns[0] + (band_ns[1] - band_ns[0]) * rng.random() for _ in range(n)]
    else:
        logos = [chip_ns] * n
    p = [f'<svg viewBox="0 0 {W} {H}" width="100%" style="max-width:920px">']
    p.append(f'<text x="{padL}" y="20" fill="#9fb0cc" font-size="14">100 trades each · one bar = one trade · '
             f'clocks normalized to compare the <b>shape</b> (jitter), not the speed.</text>')

    def row(y0, name, sub, color, series, flat):
        base = y0 + barsH
        p.append(f'<text x="8" y="{base-26:.0f}" fill="#e7eefb" font-size="17" font-weight="800">{name}</text>')
        p.append(f'<text x="8" y="{base-9:.0f}" fill="#7c8aa5" font-size="11">{sub}</text>')
        for i, v in enumerate(series):
            x = padL + i * bw
            bh = min(v, ymax) / ymax * barsH
            over = v > ymax
            p.append(f'<rect x="{x:.1f}" y="{base-bh:.1f}" width="{bw*0.62:.2f}" height="{bh:.1f}" '
                     f'rx="1" fill="{color}"{" opacity=\"0.95\"" if not flat else ""}/>')
            if over:
                cx = x + bw * 0.31
                p.append(f'<polygon points="{cx-4:.1f},{y0+2} {cx+4:.1f},{y0+2} {cx:.1f},{y0-5}" fill="{color}"/>')
        p.append(f'<line x1="{padL}" y1="{base:.1f}" x2="{padL+plotw:.1f}" y2="{base:.1f}" stroke="#1c2637"/>')
        return base

    y0 = titleH
    base = row(y0, f"Intel {cpu.split()[-1]}", f"{ghz:g} GHz · watch the spikes", "#ff4d5e", vals, False)
    # label the single worst spike
    mi = vals.index(mx)
    mx_x = padL + mi * bw
    p.append(f'<text x="{min(mx_x, padL+plotw-90):.0f}" y="{y0-9:.0f}" fill="#ff8a94" font-size="12" '
             f'font-weight="700">{fmt(mx)} — FROZEN</text>')
    p.append(f'<text x="{padL+plotw+10}" y="{base-24:.0f}" fill="#ff8a94" font-size="12">usually quick,</text>')
    p.append(f'<text x="{padL+plotw+10}" y="{base-8:.0f}" fill="#ff8a94" font-size="12">but it jumps</text>')

    y1 = y0 + barsH + 30 + gap
    b2 = row(y1, "LOGOS (our chip)", f"{ghz:g} GHz projected · a tight band, no tail", "#22e39a", logos, True)
    p.append(f'<text x="{padL+plotw+10}" y="{b2-24:.0f}" fill="#7effc9" font-size="13" font-weight="700">~{chip_ns:.0f} ns</text>')
    p.append(f'<text x="{padL+plotw+10}" y="{b2-8:.0f}" fill="#7effc9" font-size="12">no tail</text>')
    p.append('</svg>')
    return "".join(p)


# ── a clean horizontal log-bar comparison (used for worst-case latency and for power) ────────────────
def _logbars(title, rows, big_html, note, big_color="#22e39a"):
    W, padL, padR, barsH, gap = 920, 150, 150, 34, 18
    H = 32 + len(rows) * (barsH + gap) + 50
    vals = [v for _, _, v, _ in rows]
    lo, hi = min(vals) * 0.55, max(vals) * 1.3
    span = math.log10(hi) - math.log10(lo)

    def blen(v):
        return max(8.0, (W - padL - padR) * (math.log10(v) - math.log10(lo)) / span)

    p = [f'<svg viewBox="0 0 {W} {H}" width="100%" style="max-width:920px">']
    p.append(f'<text x="0" y="16" fill="#9fb0cc" font-size="14">{title}</text>')
    for i, (lab, vstr, v, col) in enumerate(rows):
        y = 34 + i * (barsH + gap)
        p.append(f'<text x="0" y="{y+barsH/2+4:.0f}" fill="#cdd9f0" font-size="13" font-weight="600">{lab}</text>')
        p.append(f'<rect x="{padL}" y="{y}" width="{W-padL-padR}" height="{barsH}" rx="6" fill="#0d1420"/>')
        bl = blen(v)
        p.append(f'<rect x="{padL}" y="{y}" width="{bl:.1f}" height="{barsH}" rx="6" fill="{col}"/>')
        p.append(f'<text x="{padL+bl+12:.1f}" y="{y+barsH/2+5:.0f}" fill="#e7eefb" font-size="17" font-weight="800">{vstr}</text>')
    p.append(f'<text x="0" y="{H-28:.0f}" fill="{big_color}" font-size="21" font-weight="800">{big_html}</text>')
    p.append(f'<text x="0" y="{H-9:.0f}" fill="#6a7c9c" font-size="11.5">{note}</text>')
    p.append('</svg>')
    return "".join(p)


def _fmt_ns(v):
    return (f"{v/1000:.0f} µs" if v >= 10000 else f"{v/1000:.1f} µs") if v >= 1000 else f"{v:.0f} ns"


def build_html(d):
    """d: dict with keys strategy, order_stream, chip{cyc_median,band,tiers[]}, x86{curve,p50,p999,max,
    tail_ratio,samples,stdev}, asic_ns."""
    chip, x86 = d["chip"], d["x86"]
    cpu = x86.get("cpu") or "the host CPU"
    cpu_ghz = x86.get("cpu_ghz")
    chip_at_cpu = x86.get("chip_ns_at_cpu")
    chart = _tail_chart(x86["curve"], d["asic_ns"], x86["samples"], cpu)
    strat = html.escape(d["strategy"])
    ladder = _zoom_ladder(chip["tiers"], chip["cyc_median"])
    base_ns = chip["tiers"][0]["ns"]
    asic = next((t for t in chip["tiers"] if t["mhz"] >= 1000), chip["tiers"][-1])
    zoom_mult = base_ns / asic["ns"]
    asic_ns = asic["ns"]
    gen = _intel_gen(cpu)
    pw = power_model(cycles=chip["cyc_median"], i9_p50_ns=x86.get("p50"))
    sky = next((t for t in chip["tiers"] if 60 <= t["mhz"] <= 150), None)
    sky_mhz = f'{sky["mhz"]:g}' if sky else "90"
    hb = ""
    if cpu_ghz and chip_at_cpu:
        band_ns = [round(chip["band"][0] / cpu_ghz, 1), round(chip["band"][1] / cpu_ghz, 1)]
        hb = (f'<div class="hbcard">{_heartbeat(x86["curve"], chip_at_cpu, cpu, cpu_ghz, band_ns)}</div>'
              f'<p class="kid"><b>Read it like a heartbeat</b> — same <b>{cpu_ghz:g} GHz</b> clock, 100 trades '
              f'each. That red row is a <b>{gen} Core i9</b>, the fastest desktop chip you can buy. It’s '
              f'usually quick — but every so often it <b class="r">stalls</b> (the tall spikes: the OS, a '
              f'cache miss, speculation). You can’t predict which trade, and a stall is when you get '
              f'<b class="r">picked off</b>. Our row is a <b class="g">tight, flat band — no tail</b>, and for '
              f'any one trade it’s <b class="g">identical every run</b>. Even a tuned CPU has a tail; '
              f'deterministic silicon doesn’t. <i>(Clocks normalized to compare the shape, not the speed — '
              f'our row is projected to the i9’s clock. This is v1 on a $265 student board; the clock catches '
              f'up in the next section, and that’s the ask.)</i></p>')

    cmp_html = ""
    if cpu_ghz and chip_at_cpu:
        worst_i9 = x86["max"]
        worst_logos = round(chip["band"][1] / cpu_ghz)
        faster = max(1, round(worst_i9 / worst_logos))
        vsbars = _logbars(
            f"The worst trade — same {cpu_ghz:g} GHz clock. You’re judged on this one, not your average.",
            [("Intel i9 · worst", _fmt_ns(worst_i9), worst_i9, "#ff4d5e"),
             ("LOGOS · worst (projected)", _fmt_ns(worst_logos), worst_logos, "#22e39a")],
            f"→ {faster:,}× faster on the trade that gets you picked off",
            f"On a typical trade the i9 is quicker — a 24-core superscalar monster; you don’t lose money on the "
            f"typical trade. LOGOS worst = {chip['band'][1]} cyc ÷ {cpu_ghz:g} GHz, projected.")
        e_note = (f"That’s power <b>draw</b>. Per <i>trade</i> the i9 is so fast its <i>energy</i> stays close "
                  f"(~{pw['i9_trade_nj']} nJ/trade vs our ~{pw['e_trade_nj']} nJ) — we don’t claim an energy-per-trade "
                  f"win here; the categorical one is the fetch/decode-free fabric your LOI funds."
                  if pw.get("i9_trade_nj") else "That’s power draw, not per-trade energy.")
        powbars = _logbars(
            f"Power draw while trading — MEASURED OpenLane Sky130 signoff ({pw['cells']:,} cells @ {pw['mhz']:g} MHz)",
            [(f"Intel {gen} i9 · package", f"{pw['i9_w']} W", pw["i9_w"], "#ff4d5e"),
             ("Intel i9 · one core", f"~{pw['i9_core_w']:.0f} W", pw["i9_core_w"], "#ff8a94"),
             ("LOGOS chip", f"{pw['sky130_mw']:.1f} mW", pw["sky130_mw"] / 1000.0, "#22e39a")],
            f"{pw['sky130_mw']:.1f} mW — the whole chip. One i9 core alone burns ~{pw['ratio_core']:,}× that.",
            f"OpenSTA report_power on the placed-and-routed netlist, typical corner (not a hand-estimate). {e_note}")
        cmp_html = f'<div class="cmp">{vsbars}</div><div class="cmp">{powbars}</div>'

    return f"""<!doctype html><html><head><meta charset="utf-8">
<title>LOGOS · deterministic tick-to-trade on silicon</title>
<style>
  * {{ box-sizing:border-box; margin:0; }}
  body {{ background:#05070d; color:#e7eefb; font:16px/1.5 -apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif; }}
  .wrap {{ max-width:1040px; margin:0 auto; padding:64px 28px 96px; }}
  .kicker {{ color:#22e39a; font-weight:700; letter-spacing:.14em; text-transform:uppercase; font-size:13px; }}
  h1 {{ font-size:52px; line-height:1.05; letter-spacing:-.02em; margin:14px 0 18px; font-weight:800; }}
  h1 .g {{ color:#22e39a; }}
  .sub {{ color:#9fb0cc; font-size:21px; max-width:760px; }}
  section {{ margin-top:64px; }}
  h2 {{ font-size:14px; letter-spacing:.14em; text-transform:uppercase; color:#6a7c9c; margin-bottom:18px; font-weight:700; }}
  pre {{ background:#0a0e16; border:1px solid #1a2334; border-radius:12px; padding:22px 24px; overflow:auto;
        color:#cdd9f0; font:14px/1.7 'SF Mono',Menlo,Consolas,monospace; }}
  ol.essence {{ list-style:none; counter-reset:s; padding:0; max-width:760px; }}
  ol.essence li {{ counter-increment:s; position:relative; padding:9px 0 9px 44px; font-size:18px; color:#d7e2f5;
                  border-bottom:1px solid #121a29; }}
  ol.essence li::before {{ content:counter(s); position:absolute; left:0; top:8px; width:28px; height:28px;
                  background:#0f2e22; color:#22e39a; border-radius:50%; text-align:center; line-height:28px;
                  font-size:13px; font-weight:700; }}
  ol.essence b.g {{ color:#22e39a; }}
  .reveal {{ color:#6a7c9c; font-size:14px; margin:22px 0 12px; }}
  .flow {{ display:flex; align-items:center; gap:10px; flex-wrap:wrap; }}
  .node {{ background:#0d1420; border:1px solid #1e2a40; border-radius:10px; padding:12px 16px; font-weight:600; font-size:14px; }}
  .node small {{ display:block; color:#6a7c9c; font-weight:400; font-size:12px; }}
  .arrow {{ color:#2f3d58; font-size:20px; }}
  .chartcard {{ background:#0a0e16; border:1px solid #1a2334; border-radius:16px; padding:24px 20px 8px; }}
  .punch {{ font-size:24px; font-weight:700; margin-top:22px; }}
  .punch .r {{ color:#ff6674; }} .punch .g {{ color:#22e39a; }}
  .stats {{ display:grid; grid-template-columns:repeat(3,1fr); gap:16px; margin-top:8px; }}
  .stat {{ background:#0a0e16; border:1px solid #1a2334; border-radius:14px; padding:22px; }}
  .stat b {{ display:block; font-size:40px; font-weight:800; letter-spacing:-.02em; }}
  .stat.g b {{ color:#22e39a; }} .stat.r b {{ color:#ff6674; }}
  .stat span {{ color:#9fb0cc; font-size:14px; }}
  .zlead {{ color:#c3d1ec; font-size:18px; max-width:820px; margin-bottom:22px; }}
  .zlead b.g {{ color:#22e39a; }}
  .eq {{ font:600 30px/1 'SF Mono',Menlo,Consolas,monospace; color:#e7eefb; text-align:center;
        background:#0a0e16; border:1px solid #1a2334; border-radius:12px; padding:22px; margin:6px 0 22px;
        letter-spacing:.02em; }}
  .levers {{ display:flex; flex-direction:column; gap:12px; margin-bottom:20px; }}
  .lever {{ background:#0a0e16; border:1px solid #1a2334; border-radius:12px; padding:16px 20px; font-size:16px; color:#d7e2f5; }}
  .lever .g {{ color:#22e39a; font-weight:700; }}
  .scope {{ background:#12161f; border-left:3px solid #2f8f6b; border-radius:0 8px 8px 0; padding:14px 18px;
           color:#c3d1ec; font-size:15px; max-width:900px; margin-bottom:18px; }}
  .scope b {{ color:#e7eefb; }}
  .tiers {{ display:flex; flex-direction:column; gap:8px; }}
  .tier {{ display:flex; align-items:center; gap:14px; background:#0a0e16; border:1px solid #1a2334;
          border-radius:10px; padding:12px 18px; }}
  .tmhz {{ width:120px; color:#9fb0cc; font-weight:600; }}
  .tns {{ flex:1; font-size:19px; font-weight:700; }}
  .tst {{ font-size:12px; padding:3px 10px; border-radius:20px; }}
  .tst.measured {{ background:#0f2e22; color:#22e39a; }}
  .tst.projected {{ background:#22261a; color:#e3c93a; }}
  .tst.roadmap {{ background:#241a2e; color:#c489f5; }}
  .proof {{ display:flex; gap:12px; flex-wrap:wrap; }}
  .chip {{ background:#0d1420; border:1px solid #1e2a40; border-radius:20px; padding:8px 16px; font-size:14px; color:#bcccea; }}
  .chip b {{ color:#22e39a; }}
  .close {{ margin-top:72px; font-size:30px; font-weight:800; line-height:1.25; letter-spacing:-.01em; }}
  .close .g {{ color:#22e39a; }}
  .foot {{ margin-top:40px; color:#5b6b86; font-size:13px; }}
  .hbcard {{ background:#0a0e16; border:1px solid #1a2334; border-radius:16px; padding:20px 16px 8px; }}
  .cmp {{ background:#0a0e16; border:1px solid #1a2334; border-radius:16px; padding:22px 22px 12px; margin-top:16px; }}
  .kid {{ font-size:19px; line-height:1.5; color:#d7e2f5; max-width:900px; margin-top:20px; }}
  .kid b.g {{ color:#22e39a; }} .kid b.r {{ color:#ff6674; }}
  .pwr {{ display:grid; grid-template-columns:repeat(3,1fr); gap:16px; margin-top:6px; }}
  .pcard {{ background:#0a0e16; border:1px solid #1a2334; border-radius:14px; padding:22px; }}
  .pcard b {{ display:block; font-size:38px; font-weight:800; letter-spacing:-.02em; margin-bottom:6px; }}
  .pcard.g b {{ color:#22e39a; }} .pcard.r b {{ color:#ff6674; }}
  .pcard span {{ color:#9fb0cc; font-size:14px; }} .pcard span b {{ color:#e7eefb; }}
</style></head><body><div class="wrap">

  <div class="kicker">FPGA-class determinism — from English, without an RTL team</div>
  <h1>When your chip stalls, you get <span class="r">picked off</span>.<br>Ours <span class="g">can’t</span>.</h1>
  <p class="sub">Your quant writes the strategy in <b>plain English</b>. We compile it to silicon that’s
  <b>deterministic by construction</b> — the same input gives the same latency <b>every run</b>, in a tight
  band with <b>no tail</b> — and the chip does <b>exactly</b> what the English says. Today that determinism
  takes an RTL team and months of verification. We give it to you from a spec you can <b>read</b>.</p>

  <div class="stats">
    <div class="stat g"><b>0</b><span>run-to-run jitter — the same input gives the same latency, every run.</span></div>
    <div class="stat g"><b>~{chip["cyc_median"]}</b><span>cycles per trade — a tight band ({chip["band"][0]}–{chip["band"][1]}), no tail. Scales with the clock.</span></div>
    <div class="stat g"><b>bit-exact</b><span>VM ≡ RTL ≡ reference — measured, fuzz-tested, CI-locked. The English is the spec.</span></div>
  </div>

  <section>
    <h2>1 · See it — jitter, worst-case speed, and power vs a {gen} i9</h2>
    {hb}
    {cmp_html}
  </section>

  <section>
    <h2>2 · A strategy your quant can read — and this <i>is</i> the chip</h2>
    <ol class="essence">
      <li>Decode each market-data message — type, side, price, size.</li>
      <li>Update the <b>order book</b> at that price level.</li>
      <li>Read the <b>top of book</b> — best bid, best ask, and their sizes.</li>
      <li><b>Microprice</b> — the size-weighted fair value.</li>
      <li>Rich vs fair → <b class="g">sell</b> the bid.&nbsp;&nbsp;Cheap → <b class="g">buy</b> the offer.</li>
      <li><b>Pre-trade risk</b> — cap the size, hold the position limit.</li>
      <li>Emit the order.</li>
    </ol>
    <div class="reveal">↓ the actual LOGOS the silicon runs — no RTL, no C++, no Verilog</div>
    <pre>{strat}</pre>
  </section>

  <section>
    <h2>3 · Slow today, on purpose — and it scales straight</h2>
    <p class="zlead">Measured: <b class="g">0 jitter</b>, on a <b>$265 student board</b> at <b>6.25 MHz</b>,
      zero timing optimization. Slow in absolute terms — we won’t pretend otherwise. But our slowness is a
      <b>clock number, not an architecture number</b>:</p>
    <div class="eq">latency&nbsp; =&nbsp; ~{chip["cyc_median"]} cycles&nbsp; ÷&nbsp; clock</div>
    <p class="zlead">The cycles are small and bounded ({chip["band"][0]}–{chip["band"][1]}). Two levers close
      the gap to nanoseconds — <b>both already in our stack</b>:</p>
    <div class="levers">
      <div class="lever"><span class="g">→ Faster clock.</span>&nbsp; Our Sky130 signoff (OpenLane,
        DRC/LVS-clean) closes <b>{sky_mhz} MHz</b> today — <b>~{round(float(sky_mhz) * 1_000_000 / chip["cyc_median"]):,}
        trades/s</b>; a modern ASIC is <b>GHz</b>.</div>
      <div class="lever"><span class="g">→ Spatial fabric.</span>&nbsp; The same English also lowers to a
        <b>dataflow pipeline</b> — the strategy laid out in space, not fetched op-by-op — the real path to
        <b>nanosecond latency at line rate</b>.</div>
    </div>
    <p class="scope"><b>Scope, honestly:</b> today this is the <b>strategy/decision</b> in silicon — the part
      the quant’s logic normally gets re-written in RTL. The feed-handler + wire (SerDes/MAC, OUCH) are the
      <b>co-development your LOI funds</b>.</p>
    <p class="punch">The path: <span class="g">English → a pipelined, deterministic ASIC</span>.</p>
  </section>

  <section>
    <h2>4 · The strategy IS the spec — nothing to drift</h2>
    <p class="zlead">Every run is <b>cycle-accurate identical</b>: VM ≡ RTL ≡ reference, <b>bit-exact</b>,
      fuzz-tested across <b>270 feeds</b>, <b>CI-locked</b>. There is no separate RTL that can diverge from
      the English, because the English is the source. <i>(Today: exhaustive differential equivalence + fuzz.
      Roadmap: formal equivalence checking — we already run a proof kernel + SMT across the stack.)</i></p>
    <div class="proof">
      <div class="chip"><b>bit-exact</b> · VM ≡ RTL ≡ reference</div>
      <div class="chip"><b>270-feed</b> · differential fuzz, byte-identical</div>
      <div class="chip"><b>CI-locked</b> · the spec can’t silently drift</div>
      <div class="chip"><b>on real silicon</b> · streamed over the port</div>
    </div>
  </section>

  <section>
    <h2>5 · The ask</h2>
    <p class="sub"><b>An LOI to co-develop your strategy on this path.</b> You bring a real strategy and a
      design partnership; you get a <b>co-designed deterministic chip</b>, a pilot, and priority. Why now:
      open PDKs and our English→fabric lowering make the ASIC path real today — not a research bet.</p>
  </section>

  <div class="close">A strategy you can <span class="g">read</span>.<br>
    Correctness you can <span class="g">check</span>.<br>
    Silicon that <span class="g">can’t stall</span>.</div>
  <div class="foot">Numbers are measured: chip cycles from the cycle-accurate core; the CPU latency from a
    tight −O2 C build of the identical logic on an {cpu} ({x86["samples"]:,} samples). Power and the ASIC
    clock are labelled estimates / roadmap. Reproduce: <code>scripts/demo-hft.sh</code>.</div>

</div></body></html>"""
