#!/usr/bin/env python3
"""A synthetic lean4export export for measuring the reader's cost per record of one kind.

    scripts/lean/arena-crown/synthetic-records.py <count> <app|pi> > out.ndjson

Ten million dense `{"ie":k,"sort":0}` filler records come first, so the measured records carry
eight-digit ids, as most of mathlib's do (its expression ids run to ~95M), and the id map stays
dense. Then `<count>` records of the kind: each application's function is the previous record and
its argument a random earlier one (a fixed seed, so every run is the same file).

The marginal cost of one record is the difference of two counts over the difference in records:

    for n in 0 1000000 2000000; do scripts/lean/arena-crown/synthetic-records.py $n app > app$n.ndjson; done
    for n in 0 1000000 2000000; do
      CORE=5 scripts/lean/arena-crown/instr.sh app$n.ndjson --run "<logos_kernel> --parse-only \\$IN"
    done
    # (I(2M) - I(1M)) / 1M = instructions per application record; I(0) / 10M ≈ per filler record.
"""
import random
import sys

META = ('{"meta":{"exporter":{"name":"lean4export","version":"3.1.0"},"format":{"version":"3.1.0"},'
        '"lean":{"githash":"f72c35b3f637c8c6571d353742168ab66cc22c00","version":"4.29.1"}}}')
BASE = 10_000_000


def main() -> None:
    n, kind = int(sys.argv[1]), sys.argv[2]
    if kind not in ("app", "pi"):
        sys.exit("kind must be app or pi")
    random.seed(7)
    w = sys.stdout.write
    w(META + "\n")
    w('{"in":1,"str":{"pre":0,"str":"c"}}\n')
    w("".join('{"ie":%d,"sort":0}\n' % k for k in range(BASE)))
    ids = [BASE - 1]
    for k in range(n):
        i = BASE + k
        prev, other = ids[-1], ids[random.randrange(len(ids))]
        if kind == "app":
            w('{"app":{"arg":%d,"fn":%d},"ie":%d}\n' % (other, prev, i))
        else:
            w('{"forallE":{"binderInfo":"default","body":%d,"name":1,"type":%d},"ie":%d}\n' % (other, prev, i))
        ids.append(i)


if __name__ == "__main__":
    main()
