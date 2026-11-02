#!/usr/bin/env python3
"""Generate benchmarks/parity/PARITY_LEDGER.md from the catalog + inventories.

Pure, deterministic function of benchmarks/parity/catalog/superset.json and
benchmarks/parity/inventory/*.json. Reports correctness-parity % (green / in-scope),
performance-parity % (faster / perf-gated), and upstream triage (accounted / total).
parity_ledger_lock.rs recomputes these and fails if the committed ledger drifts.
"""
import json
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
BASE = ROOT / "benchmarks" / "parity"
LANGS = ["c", "python", "go", "rust", "java"]


def pct(num, den):
    return f"{(100.0 * num / den):.1f}%" if den else "n/a"


def main():
    cat = json.load(open(BASE / "catalog" / "superset.json"))
    caps = cat["capabilities"]

    # Per-domain tallies.
    dom = defaultdict(lambda: {"green": 0, "inscope": 0, "na": 0, "faster": 0, "perfgated": 0})
    for c in caps:
        d = dom[c["domain"]]
        if c["status"] == "na":
            d["na"] += 1
        else:
            d["inscope"] += 1
            if c["status"] == "green":
                d["green"] += 1
        perf = c.get("perf") or {}
        if perf.get("status") in ("pending", "faster"):
            d["perfgated"] += 1
            if perf.get("status") == "faster":
                d["faster"] += 1

    green = sum(d["green"] for d in dom.values())
    inscope = sum(d["inscope"] for d in dom.values())
    na = sum(d["na"] for d in dom.values())
    faster = sum(d["faster"] for d in dom.values())
    perfgated = sum(d["perfgated"] for d in dom.values())

    # Upstream triage from inventories.
    universe = {l: set() for l in LANGS}
    for l in LANGS:
        inv = json.load(open(BASE / "inventory" / f"{l}.json"))
        for e in inv["entries"]:
            universe[l].add(f"{e['module']}.{e['symbol']}")
    accounted = {l: set() for l in LANGS}
    for c in caps:
        for l, syms in c["provenance"].items():
            for s in syms:
                accounted[l].add(s)
    total = sum(len(universe[l]) for l in LANGS)
    acc = sum(len(accounted[l]) for l in LANGS)
    # In-scope backlog = unclaimed symbols in in-scope modules, MINUS per-symbol N/A-by-design
    # (na_symbols: language API surfaces LOGOS covers via a value-semantic equivalent, or that are
    # genuinely out of scope). Out-of-scope modules are N/A-by-design too. Boundary = LAST dot.
    scope_doc = json.load(open(BASE / "catalog" / "scope.json"))
    scope = scope_doc["in_scope_modules"]
    na_symbols = scope_doc.get("na_symbols", {})
    backlog = na_by_cat = 0
    for l in LANGS:
        ins = set(scope.get(l, []))
        na_lang = na_symbols.get(l, {})
        for key in universe[l]:
            if key in accounted[l]:
                continue
            module = key.rsplit(".", 1)[0]
            if module not in ins or key in na_lang:
                na_by_cat += 1
            else:
                backlog += 1

    lines = [
        "# LOGOS Standard-Library Parity Ledger",
        "",
        "_Generated from catalog/superset.json by scripts/bench/gen-parity-ledger.py — do not edit by hand._",
        "",
        "The stdlib is pure LOGOS and must be FASTER than every C/Python/Rust/Go counterpart.",
        "A capability is fully landed only when it is correct on all tiers AND benchmarked faster.",
        "",
        f"- **Correctness parity:** {pct(green, inscope)} ({green} green / {inscope} in-scope; {na} n/a excluded)",
        f"- **Performance parity:** {pct(faster, perfgated)} ({faster} faster / {perfgated} perf-gated)",
        f"- **Upstream triage:** {backlog} in-scope backlog · {na_by_cat} N/A-by-design · {acc} claimed (of {total})",
        "",
        "| Domain | Green | In-scope | Correct % | Faster | Perf-gated | Faster % |",
        "|--------|-------|----------|-----------|--------|------------|----------|",
    ]
    for name in sorted(dom):
        d = dom[name]
        lines.append(
            f"| {name} | {d['green']} | {d['inscope']} | {pct(d['green'], d['inscope'])} "
            f"| {d['faster']} | {d['perfgated']} | {pct(d['faster'], d['perfgated'])} |"
        )
    lines += [
        "",
        "## Provenance",
        "",
        "Pinned upstream sources (see sources.lock.toml): C = C23 (WG14 N3220) / glibc 2.39, "
        "Python = 3.12.3, Rust = 1.96.0, Go = go1.22.2, Java = curated (secondary).",
        "",
    ]
    (BASE / "PARITY_LEDGER.md").write_text("\n".join(lines) + "\n")
    print(f"Wrote PARITY_LEDGER.md — correctness {pct(green, inscope)}, "
          f"performance {pct(faster, perfgated)}, {backlog} in-scope backlog, {na_by_cat} N/A-by-design")
    return 0


if __name__ == "__main__":
    sys.exit(main())
