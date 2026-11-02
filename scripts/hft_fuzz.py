#!/usr/bin/env python3
"""hft_fuzz — differential fuzz of the LOGOS tick-to-trade pipeline against the Python reference.

For many random ITCH-lite feeds, generate the `.lg` (hft_gen), run it on the LOGOS VM (the lgvm example),
and assert the Show output is byte-identical to hft_model.run_pipeline. Random feeds exercise the edges the
demo feed can't: empty book, over-subtracted (clamped) levels, out-of-ladder prices, position-limit hits.

    python3 scripts/hft_fuzz.py [N]     # default 150 random feeds

Requires the built example: target/debug/examples/lgvm.
"""
import os
import random
import subprocess
import sys

sys.path.insert(0, os.path.dirname(__file__))
import hft_model as M
import hft_gen

ROOT = os.path.normpath(os.path.join(os.path.dirname(__file__), ".."))
LGVM = os.path.join(ROOT, "target", "debug", "examples", "lgvm")


def random_feed(rng):
    nmsgs = rng.randint(1, 12)
    msgs = []
    for k in range(nmsgs):
        mtype = rng.choice([M.TYPE_ADD, M.TYPE_ADD, M.TYPE_ADD, M.TYPE_DEL, M.TYPE_EXE])  # add-heavy
        side = rng.choice([1, 2])
        price = rng.randint(90, 111)          # spans and slightly overflows the [93,108] ladder
        qty = rng.randint(0, 200)
        oid = rng.randint(1, 250)
        msgs.append((mtype, side, price, qty, oid))
    return M.encode(msgs)


def lgvm_run(src):
    p = subprocess.run([LGVM, "/dev/stdin"], input=src, capture_output=True, text=True)
    if p.returncode != 0:
        raise RuntimeError(f"lgvm failed: {p.stderr.strip()}")
    return [int(x) for x in p.stdout.split()]


def main(argv):
    n = int(argv[1]) if len(argv) > 1 else 150
    if not os.path.exists(LGVM):
        sys.exit(f"missing {LGVM} — build it: cargo build -p logicaffeine-compile --example lgvm")
    rng = random.Random(0xC0FFEE)     # fixed seed: deterministic, reproducible fuzz
    fails = 0
    for k in range(n):
        feed = random_feed(rng)
        expect = M.run_pipeline(feed)
        got = lgvm_run(hft_gen.gen(feed))
        if got != expect:
            fails += 1
            print(f"FAIL feed#{k} ({len(feed)//8} msgs)\n  feed={feed}\n  vm  ={got}\n  ref ={expect}")
            if fails >= 5:
                break
    if fails:
        sys.exit(f"\n{fails} mismatch(es) of {n} — VM diverged from the reference")
    print(f"OK: {n} random feeds — LOGOS VM ≡ Python reference, byte-identical")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
