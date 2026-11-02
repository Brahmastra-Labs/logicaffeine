#!/usr/bin/env python3
"""Normalize the committed stdlib snapshots into benchmarks/parity/inventory/<lang>.json.

Offline and deterministic: reads ONLY benchmarks/parity/snapshots/ (never the network or a
live toolchain) and emits sorted JSON so parity_catalog_lock.rs can byte-check the result.
Every language collapses to the same entry shape: {module, symbol, kind, signature}.

Run after scripts/bench/fetch-parity-snapshots.sh refreshes the snapshots, or any time to verify
the inventories are reproducible from the committed evidence.
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
SNAP = ROOT / "benchmarks" / "parity" / "snapshots"
INV = ROOT / "benchmarks" / "parity" / "inventory"


def dump(lang, version, entries):
    entries = sorted(entries, key=lambda e: (e["module"], e["symbol"], e["kind"]))
    # dedupe identical (module, symbol, kind)
    seen, uniq = set(), []
    for e in entries:
        key = (e["module"], e["symbol"], e["kind"])
        if key in seen:
            continue
        seen.add(key)
        uniq.append(e)
    INV.mkdir(parents=True, exist_ok=True)
    path = INV / f"{lang}.json"
    json.dump(
        {"language": lang, "version": version, "count": len(uniq), "entries": uniq},
        open(path, "w"),
        indent=1,
        sort_keys=True,
    )
    print(f"  {lang}: {len(uniq)} entries -> {path.relative_to(ROOT)}")
    return len(uniq)


def norm_python():
    data = json.load(open(SNAP / "python" / "symbols.json"))
    entries = []
    for mod, syms in data["modules"].items():
        if "__import_error__" in syms:
            continue
        for name, kind in syms.items():
            entries.append({"module": mod, "symbol": name, "kind": kind, "signature": ""})
    return dump("python", data["version"], entries)


def norm_go():
    text = (SNAP / "go" / "go-doc.txt").read_text(errors="ignore")
    ver = "go?"
    m = re.search(r"# (go\S+)", text)
    if m:
        ver = m.group(1)
    entries, pkg = [], "?"
    # Allow an optional generic type-parameter list `[...]` before the arg `(` so
    # generic functions (slices.Min, maps.Keys, ...) in go1.21+ are captured.
    func_re = re.compile(r"^func(?:\s+\([^)]*\))?\s+([A-Z][A-Za-z0-9_]*)(?:\[.*?\])?\s*\(")
    type_re = re.compile(r"^type\s+([A-Z][A-Za-z0-9_]*)\b")
    const_re = re.compile(r"^(?:const|var)\s+([A-Z][A-Za-z0-9_]*)\b")
    for line in text.splitlines():
        if line.startswith("## PKG "):
            pkg = line[len("## PKG "):].strip()
            continue
        mf = func_re.match(line)
        if mf:
            entries.append({"module": pkg, "symbol": mf.group(1), "kind": "function",
                            "signature": line.strip()})
            continue
        mt = type_re.match(line)
        if mt:
            entries.append({"module": pkg, "symbol": mt.group(1), "kind": "type",
                            "signature": line.strip()})
            continue
        mc = const_re.match(line)
        if mc:
            entries.append({"module": pkg, "symbol": mc.group(1), "kind": "constant",
                            "signature": line.strip()})
    return dump("go", ver, entries)


def norm_c():
    data = json.load(open(SNAP / "c" / "c23-functions.json"))
    entries = []
    for header, funcs in data["headers"].items():
        for fn in funcs:
            entries.append({"module": header, "symbol": fn, "kind": "function", "signature": ""})
    return dump("c", data["standard"], entries)


def norm_curated(lang, filename):
    data = json.load(open(SNAP / lang / filename))
    entries = []
    for module, syms in data["modules"].items():
        for name in syms:
            entries.append({"module": module, "symbol": name, "kind": "function", "signature": ""})
    return dump(lang, data["version"], entries)


def main():
    print("Normalizing stdlib snapshots -> inventory/ ...")
    total = 0
    total += norm_c()
    total += norm_python()
    total += norm_go()
    total += norm_curated("rust", "std-curated.json")
    total += norm_curated("java", "java-curated.json")
    print(f"Total normalized entries across 5 languages: {total}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
