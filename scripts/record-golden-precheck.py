#!/usr/bin/env python3
"""Count golden-deck DRC violations from KLayout RDB reports and merge into golden_precheck.json.

mpw_precheck counts violations as the number of `<item>` entries in the KLayout report database
(each `<item>` is one flagged geometry; the `<categories>` block just lists every rule the deck
defines). We count identically. Fail-CLOSED: a missing/unreadable report becomes -1, which trips
the lock.

The verdict is bound to the GDS it checked (`gds_bytes`), so it cannot outlive that file: a
re-hardened block's old golden entry would otherwise keep reading clean for a stream nobody checked.

Usage:  record-golden-precheck.py <block> <golden_drc.xml> <zeroarea.xml> <out.json> <checked.gds>
"""
import json
import os
import sys


_PDK = os.environ.get("PDK", "sky130A")


def count_items(path):
    try:
        return open(path, encoding="utf-8", errors="replace").read().count("<item>")
    except OSError:
        return -1  # sentinel -> lock RED


def main():
    if len(sys.argv) != 6:
        sys.exit("usage: record-golden-precheck.py <block> <golden.xml> <zeroarea.xml> <out.json> <checked.gds>")
    block, golden, zero, out, gds = sys.argv[1:6]
    gds_bytes = os.path.getsize(gds) if os.path.isfile(gds) else -1
    g, z = count_items(golden), count_items(zero)
    clean = g == 0 and z == 0

    doc = {}
    if os.path.exists(out):
        try:
            doc = json.load(open(out))
        except ValueError:
            doc = {}
    doc.setdefault("meta", {})
    doc["meta"]["deck"] = (
        f"{_PDK}_mr.drc (foundry/efabless maximal-rule deck) + zeroarea"
    )
    doc["meta"]["source"] = "efabless/mpw_precheck golden decks, run under OpenLane KLayout"
    doc.setdefault("blocks", {})[block] = {
        "golden_drc_violations": g,
        "zeroarea_violations": z,
        "feol": True,
        "beol": True,
        "golden_clean": clean,
        "gds_bytes": gds_bytes,
    }
    json.dump(doc, open(out, "w"), indent=2)
    open(out, "a").write("\n")
    print(f"  {block}: golden {_PDK}_mr DRC = {g}  zeroarea = {z}  -> "
          f"{'GOLDEN CLEAN' if clean else 'NOT CLEAN'}")
    sys.exit(0 if clean else 1)


if __name__ == "__main__":
    main()
