#!/usr/bin/env python3
"""Render a flamegraph SVG from the LOGOS CUDA forward's `PIN` timing lines.

The emitted Qwen forward (codegen_cuda::qwen_emit), when run with `LOGOS_CUDA_PINS=1`,
brackets every kernel launch with CUDA events and accumulates GPU time into per-phase
buckets — GPU hardware counters (`ncu`) are permission-gated on cloud pods, so these
host-side event pins ARE the profile. Each `PIN <phase> <ms> ms x<count> <pct>%` line
is one child frame under the whole-forward root; widths are proportional to GPU time.

Usage:
    LOGOS_CUDA_PINS=1 ./qwen30b_full 2>&1 >/dev/null | grep '^PIN' | \
        python3 scripts/accel/cuda-pins-flamegraph.py --title "Qwen3-30B forward (A100)" -o forward.svg
"""
import sys
import argparse

PALETTE = ["#e8564a", "#e8834a", "#e8b04a", "#a7c957", "#4ab0e8", "#6a5acd", "#c74ae8", "#9e9e9e"]


def parse(lines):
    phases, total = [], None
    for ln in lines:
        ln = ln.strip()
        if not ln.startswith("PIN "):
            continue
        rest = ln[4:].strip()
        # split name (may contain '+') from the numeric tail
        toks = rest.split()
        # find the ms value: the token before the literal 'ms'
        try:
            ms_i = toks.index("ms")
        except ValueError:
            continue
        name = " ".join(toks[:ms_i - 1])
        ms = float(toks[ms_i - 1])
        if name.upper() == "TOTAL":
            total = ms
            continue
        if ms <= 0.0:
            continue
        phases.append((name, ms))
    return phases, total


def svg(phases, title, out):
    W, ROW, PAD, HEAD = 1200, 44, 2, 64
    total = sum(ms for _, ms in phases) or 1.0
    phases = sorted(phases, key=lambda p: -p[1])
    H = HEAD + ROW * 2 + 24
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" font-family="Menlo,Consolas,monospace">',
        f'<rect width="{W}" height="{H}" fill="#0f1116"/>',
        f'<text x="{W/2}" y="26" fill="#e6e6e6" font-size="17" text-anchor="middle" font-weight="bold">{title}</text>',
        f'<text x="{W/2}" y="46" fill="#8a8f98" font-size="12" text-anchor="middle">'
        f'{total:.3f} ms of event-serialized GPU work across {len(phases)} phases '
        f'(relative shares; real graph-replay is faster)</text>',
    ]
    # root frame (whole forward)
    parts.append(f'<rect x="{PAD}" y="{HEAD}" width="{W-2*PAD}" height="{ROW}" fill="#2b2f3a" stroke="#0f1116"/>')
    parts.append(f'<text x="{PAD+8}" y="{HEAD+ROW/2+5}" fill="#e6e6e6" font-size="13">qwen3-30B-A3B forward — one token</text>')
    # child frames (phases) proportional to time
    x = PAD
    inner = W - 2 * PAD
    y = HEAD + ROW
    for i, (name, ms) in enumerate(phases):
        w = inner * ms / total
        col = PALETTE[i % len(PALETTE)]
        parts.append(f'<rect x="{x:.1f}" y="{y}" width="{max(w-1,1):.1f}" height="{ROW}" fill="{col}" stroke="#0f1116"/>')
        pct = 100.0 * ms / total
        if w > 120:
            parts.append(f'<text x="{x+6:.1f}" y="{y+ROW/2-2:.0f}" fill="#0f1116" font-size="12" font-weight="bold">{name}</text>')
            parts.append(f'<text x="{x+6:.1f}" y="{y+ROW/2+13:.0f}" fill="#0f1116" font-size="11">{ms:.3f} ms · {pct:.1f}%</text>')
        elif w > 34:
            parts.append(f'<text x="{x+4:.1f}" y="{y+ROW/2+4:.0f}" fill="#0f1116" font-size="10">{pct:.0f}%</text>')
        x += w
    parts.append('</svg>')
    with open(out, "w") as f:
        f.write("\n".join(parts))
    return H


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("-o", "--out", default="cuda_forward_flamegraph.svg")
    ap.add_argument("--title", default="LOGOS Qwen3-30B forward — GPU phase flamegraph")
    args = ap.parse_args()
    phases, total = parse(sys.stdin.readlines())
    if not phases:
        sys.exit("no PIN lines on stdin — run the forward with LOGOS_CUDA_PINS=1 and pipe its stderr here")
    svg(phases, args.title, args.out)
    tot = total if total else sum(ms for _, ms in phases)
    print(f"wrote {args.out} — {len(phases)} phases, {tot:.3f} ms serialized total", file=sys.stderr)
    for name, ms in sorted(phases, key=lambda p: -p[1]):
        print(f"  {name:20s} {ms:8.4f} ms  {100.0*ms/(tot or 1):5.1f}%", file=sys.stderr)


if __name__ == "__main__":
    main()
