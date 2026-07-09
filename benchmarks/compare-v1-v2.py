#!/usr/bin/env python3
"""Compare two run-logos-vs-c.sh result JSONs (v1 vs v2/e-graph).

The question this answers: does turning the e-graph on cost any performance?
For each benchmark it reports the LOGOS time under each pipeline, the v2/v1
speed ratio (the headline — <1.0 means v2 is faster, >1.0 slower), and whether
the generated Rust actually changed (identical codegen ⟹ identical perf, the
delta is noise).

Usage: python3 compare-v1-v2.py results/v1-vs-c.json results/v2-vs-c.json
"""
import json
import sys
import math


def load(path):
    with open(path) as f:
        return json.load(f)


def bench_map(doc):
    return {b["id"]: b for b in doc["benchmarks"]}


def logos_c_at(entry, size):
    s = entry.get("scaling", {}).get(size, {})
    lg = s.get("logos_release", {}).get("mean_ms")
    c = s.get("c", {}).get("mean_ms")
    return lg, c


def pick_size(entry):
    """The most compute-dominated size present (largest C mean)."""
    best, best_c = None, -1.0
    for size, langs in entry.get("scaling", {}).items():
        c = langs.get("c", {}).get("mean_ms")
        if c is not None and c > best_c:
            best, best_c = size, c
    return best


def main():
    v1 = bench_map(load(sys.argv[1]))
    v2 = bench_map(load(sys.argv[2]))
    ids = [i for i in v1 if i in v2]

    rows = []
    for i in ids:
        size = pick_size(v2[i]) or pick_size(v1[i])
        if size is None:
            continue
        lg1, c1 = logos_c_at(v1[i], size)
        lg2, c2 = logos_c_at(v2[i], size)
        if not lg1 or not lg2:
            continue
        changed = v1[i].get("generated_rust") != v2[i].get("generated_rust")
        rows.append({
            "id": i, "size": size,
            "lg1": lg1, "lg2": lg2,
            "c": c2 or c1,
            "ratio": lg2 / lg1,           # v2/v1 LOGOS time
            "vsc1": lg1 / c1 if c1 else None,
            "vsc2": lg2 / c2 if c2 else None,
            "changed": changed,
        })

    rows.sort(key=lambda r: r["ratio"], reverse=True)

    print(f"{'benchmark':<16}{'size':>10}  {'v1 ms':>9}{'v2 ms':>9}  "
          f"{'v2/v1':>7} {'cg':>3}  {'v1/C':>6}{'v2/C':>6}")
    print("-" * 78)
    for r in rows:
        flag = "WORSE" if r["ratio"] > 1.03 else ("better" if r["ratio"] < 0.97 else "")
        cg = "Δ" if r["changed"] else "="
        print(f"{r['id']:<16}{r['size']:>10}  {r['lg1']:>9.2f}{r['lg2']:>9.2f}  "
              f"{r['ratio']:>7.3f} {cg:>3}  "
              f"{(r['vsc1'] or 0):>6.2f}{(r['vsc2'] or 0):>6.2f}  {flag}")

    def geomean(xs):
        xs = [x for x in xs if x and x > 0]
        return math.exp(sum(math.log(x) for x in xs) / len(xs)) if xs else float("nan")

    print("-" * 78)
    ratios = [r["ratio"] for r in rows]
    changed = [r for r in rows if r["changed"]]
    worse = [r for r in rows if r["ratio"] > 1.03]
    better = [r for r in rows if r["ratio"] < 0.97]
    print(f"benchmarks compared      : {len(rows)}")
    print(f"codegen changed by v2    : {len(changed)}  ({', '.join(r['id'] for r in changed) or 'none'})")
    print(f"geomean v2/v1 LOGOS time : {geomean(ratios):.4f}   (<1.0 = e-graph faster overall)")
    print(f"  among codegen-changed  : {geomean([r['ratio'] for r in changed]):.4f}" if changed else "  among codegen-changed  : n/a")
    print(f"regressions (>1.03x)     : {len(worse)}  ({', '.join(r['id'] for r in worse) or 'none'})")
    print(f"improvements (<0.97x)    : {len(better)}  ({', '.join(r['id'] for r in better) or 'none'})")
    print(f"geomean v1 vs C          : {geomean([r['vsc1'] for r in rows]):.4f}")
    print(f"geomean v2 vs C          : {geomean([r['vsc2'] for r in rows]):.4f}   (lower = LOGOS faster vs C)")


if __name__ == "__main__":
    main()
