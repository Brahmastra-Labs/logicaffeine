#!/usr/bin/env python3
"""Render collapsed stacks (`a;b;c count` lines, e.g. from scripts/dev/flamefold.py) as a flamegraph SVG.

usage: folded-to-svg.py in.folded out.svg [--title T] [--top N]
Also prints the top-N frames by self weight and by inclusive weight, so a profile can be read
without opening the SVG.
"""
import argparse, html, sys
from collections import defaultdict


class Node:
    __slots__ = ("name", "total", "kids")

    def __init__(self, name):
        self.name, self.total, self.kids = name, 0, {}


def load(path):
    root = Node("all")
    self_w, incl_w = defaultdict(int), defaultdict(int)
    for line in open(path):
        line = line.rstrip()
        if not line:
            continue
        stack, _, n = line.rpartition(" ")
        try:
            n = int(float(n))
        except ValueError:
            continue
        frames = [f for f in stack.split(";") if f]
        root.total += n
        cur = root
        seen = set()
        for f in frames:
            cur = cur.kids.setdefault(f, Node(f))
            cur.total += n
            if f not in seen:
                incl_w[f] += n
                seen.add(f)
        if frames:
            self_w[frames[-1]] += n
    return root, self_w, incl_w


def color(name):
    h = abs(hash(name)) % 60
    if "dag::" in name:
        return f"rgb({205 + h % 50},{90 + h},{40})"
    if "alloc" in name or "malloc" in name or "free" in name:
        return f"rgb(80,{140 + h},{200})"
    return f"rgb({180 + h % 70},{150 + h},{60})"


def emit(node, x, depth, total, width, out, height, h):
    w = width * node.total / total
    if w < 0.3:
        return
    y = height - (depth + 1) * h
    label = node.name if len(node.name) * 6.5 < w else node.name[: max(0, int(w / 6.5) - 2)] + ".."
    pct = 100.0 * node.total / total
    out.append(
        f'<g><title>{html.escape(node.name)} ({pct:.2f}%)</title>'
        f'<rect x="{x:.1f}" y="{y}" width="{w:.1f}" height="{h - 1}" fill="{color(node.name)}" rx="2"/>'
        f'<text x="{x + 3:.1f}" y="{y + h - 5}" font-size="11" font-family="monospace">{html.escape(label) if w > 20 else ""}</text></g>'
    )
    cx = x
    for k in sorted(node.kids.values(), key=lambda k: k.name):
        emit(k, cx, depth + 1, total, width, out, height, h)
        cx += width * k.total / total


def depth(node):
    return 1 + max((depth(k) for k in node.kids.values()), default=0)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("folded")
    ap.add_argument("svg")
    ap.add_argument("--title", default="flamegraph")
    ap.add_argument("--top", type=int, default=30)
    a = ap.parse_args()
    sys.setrecursionlimit(100000)
    root, self_w, incl_w = load(a.folded)
    width, h = 1600, 16
    height = (min(depth(root), 120) + 3) * h
    out = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height + 30}" viewBox="0 0 {width} {height + 30}">',
           f'<text x="10" y="20" font-size="16" font-family="sans-serif">{html.escape(a.title)}</text>']
    emit(root, 0, 0, max(root.total, 1), width, out, height + 30, h)
    out.append("</svg>")
    open(a.svg, "w").write("\n".join(out))
    tot = max(root.total, 1)
    print(f"total weight {root.total}")
    print("--- self ---")
    for f, w in sorted(self_w.items(), key=lambda kv: -kv[1])[: a.top]:
        print(f"{100.0 * w / tot:6.2f}%  {f}")
    print("--- inclusive ---")
    for f, w in sorted(incl_w.items(), key=lambda kv: -kv[1])[: a.top]:
        print(f"{100.0 * w / tot:6.2f}%  {f}")


if __name__ == "__main__":
    main()
