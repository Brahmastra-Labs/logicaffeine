#!/usr/bin/env python3
"""Extract the Tenstorrent SoC facts our chip presets must agree with, from the vendor's own descriptors.

The descriptors live in the tt-metal tree (`tt_metal/soc_descriptors/<chip>_arch.yaml`), which is NOT in
this repo and is not present on every machine. So the facts are extracted ONCE, written to
`benchmarks/silicon/tt_soc_facts.json` with the source path and its sha256, and locked from there:

  • `spatial_chip_facts_lock` reads the JSON on every build and holds the presets to it;
  • the same lock, when the SDK IS present, re-extracts and requires the JSON to still be true — so the
    repo's copy of a vendor fact cannot rot silently.

Nothing here is estimated. Every field below is read out of the yaml.

    python3 scripts/accel/tt-soc-facts.py [--check]

`--check` re-extracts and diffs against the checked-in file without writing (exit 1 on drift).
"""

import hashlib
import json
import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(REPO, "benchmarks/silicon/tt_soc_facts.json")

# (our SpatialTarget name, SoC descriptor, tensix_types.h relative to tt_metal/hw/inc/internal)
CHIPS = [
    ("wormhole_b0", "wormhole_b0_80_arch.yaml", "tt-1xx/wormhole/wormhole_b0_defines/tensix_types.h"),
    ("blackhole", "blackhole_140_arch.yaml", "tt-1xx/blackhole/tensix_types.h"),
]


def tt_metal_home():
    return os.environ.get("TT_METAL_HOME") or os.path.join(os.path.expanduser("~"), "tt-metal")


def dest_tiles(path):
    """The accumulator register file, from the part's own `tensix_types.h`.

    ★ THE ADDRESSABLE ACCUMULATOR IS THE HALF, NOT THE FILE. `DEST_NUM_TILES_FP16` is the whole DST;
    tt-metal compute kernels (ours included) bracket their math with `tile_regs_acquire` /
    `tile_regs_commit`, which is DstSync::SyncHalf — the packer drains one half while math fills the
    other — so only `DEST_NUM_TILES_FP16 / 2` tiles are reachable in a single acquire.

    Both parts measure 16 and 8. Our Wormhole preset had always said 8; Blackhole said 16, and the
    first shape whose plan crossed eight tiles returned PCC 0.872 with exactly the right retired
    arithmetic — a kernel indexing past the half-window computes the problem and writes it nowhere
    useful. The number is read here so no preset has to remember it."""
    s = open(path).read()

    def v(n):
        m = re.search(r"%s\s*=\s*([0-9]+)" % n, s)
        if not m:
            raise SystemExit("%s: no %s" % (path, n))
        return int(m.group(1))

    full_size = 64 * v("DEST_FACE_HEIGHT")          # DEST_REGISTER_FULL_SIZE, per the vendor header
    tiles = full_size * v("DEST_FACE_WIDTH") // (32 * 32)
    return {
        "dest_types_source": os.path.relpath(path, tt_metal_home()),
        "dest_types_sha256": hashlib.sha256(s.encode()).hexdigest(),
        "dest_num_tiles_fp16": tiles,
        "dest_num_tiles_fp16_half": tiles // 2,
    }


def extract(path):
    """Every field is read out of the descriptor by a real YAML parser. Hand-rolled regexes over this
    file gave 5 channels where there are 6 — the one thing an extractor must not do is guess."""
    import yaml

    src = open(path).read()
    d = yaml.safe_load(src)
    workers = d["functional_workers"]
    xs = sorted({int(w.split("-")[0]) for w in workers})
    ys = sorted({int(w.split("-")[1]) for w in workers})
    bank = int(d["dram_bank_size"])
    return {
        "source": os.path.relpath(path, tt_metal_home()),
        "sha256": hashlib.sha256(src.encode()).hexdigest(),
        "arch_name": d["arch_name"],
        "worker_l1_bytes": int(d["worker_l1_size"]),
        "eth_l1_bytes": int(d["eth_l1_size"]),
        "dram_channels": len(d["dram"]),
        "dram_noc_endpoints_per_channel": len(d["dram"][0]),
        "dram_bank_size_bytes": bank,
        "dram_total_gib": (len(d["dram"]) * bank) // (1 << 30),
        "functional_workers": len(workers),
        "worker_grid_x": len(xs),
        "worker_grid_y": len(ys),
        "harvested_workers": len(d.get("harvested_workers") or []),
    }


def build():
    home = tt_metal_home()
    out = {
        "_what": "Tenstorrent SoC facts, extracted from the vendor's own descriptors by "
                 "scripts/accel/tt-soc-facts.py. Every field is READ from the yaml named in `source`; none is "
                 "estimated. `spatial_chip_facts_lock` holds SpatialTarget presets to these numbers, and "
                 "re-derives them whenever the SDK is present so this file cannot rot.",
        # The tree the facts were read from, named by the variable that locates it on any machine.
        "tt_metal_home": "$TT_METAL_HOME",
        "chips": {},
    }
    for name, f, tx in CHIPS:
        p = os.path.join(home, "tt_metal/soc_descriptors", f)
        if not os.path.exists(p):
            raise SystemExit("missing descriptor: %s (set TT_METAL_HOME)" % p)
        t = os.path.join(home, "tt_metal/hw/inc/internal", tx)
        if not os.path.exists(t):
            raise SystemExit("missing tensix_types.h: %s (set TT_METAL_HOME)" % t)
        row = extract(p)
        row.update(dest_tiles(t))
        out["chips"][name] = row
    return out


def main():
    fresh = build()
    if "--check" in sys.argv:
        have = json.load(open(OUT))
        if have.get("chips") != fresh["chips"]:
            print("DRIFT between %s and the vendor descriptors:" % os.path.relpath(OUT, REPO))
            for k in sorted(set(have.get("chips", {})) | set(fresh["chips"])):
                a, b = have.get("chips", {}).get(k), fresh["chips"].get(k)
                if a != b:
                    print("  %s:\n    checked in: %s\n    vendor    : %s" % (k, a, b))
            return 1
        print("ok: checked-in SoC facts match the vendor descriptors")
        return 0
    with open(OUT, "w") as fh:
        json.dump(fresh, fh, indent=2, sort_keys=True)
        fh.write("\n")
    print("wrote %s" % os.path.relpath(OUT, REPO))
    for name, c in sorted(fresh["chips"].items()):
        print("  %-12s L1 %d B  DRAM %dch x %.2f GiB = %d GiB  workers %d (%dx%d)  DST %d (half %d)"
              % (name, c["worker_l1_bytes"], c["dram_channels"],
                 c["dram_bank_size_bytes"] / (1 << 30), c["dram_total_gib"],
                 c["functional_workers"], c["worker_grid_x"], c["worker_grid_y"],
                 c["dest_num_tiles_fp16"], c["dest_num_tiles_fp16_half"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
