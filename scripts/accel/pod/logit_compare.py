#!/usr/bin/env python3
"""Rank implementations by distance from the f64 oracle logit vector.

Argument 1 is the oracle; every other argument is a candidate. Logits are only defined up to an additive
constant (softmax is shift-invariant), so each vector is mean-centred before comparison -- otherwise a
constant offset that changes no probability whatsoever would count as error."""
import sys, os, struct, math

def load(p):
    if not os.path.exists(p) or os.path.getsize(p) == 0: return None
    with open(p, 'rb') as f: raw = f.read()
    v = list(struct.unpack('<%df' % (len(raw) // 4), raw))
    m = sum(v) / len(v)
    return [x - m for x in v]

ref = load(sys.argv[1])
if ref is None: sys.exit("oracle %s missing" % sys.argv[1])
n = len(ref)
rms_ref = math.sqrt(sum(x * x for x in ref) / n)
top_ref = max(range(n), key=lambda i: ref[i])
print("oracle: %s  n=%d  argmax=%d  rms=%.4f" % (os.path.basename(sys.argv[1]), n, top_ref, rms_ref))
print("%-22s %12s %12s %12s %8s %9s" % ("candidate", "rel_L2", "max_abs", "top1_gap", "argmax", "top10"))
rows = []
for p in sys.argv[2:]:
    v = load(p)
    if v is None: print("%-22s  (missing)" % os.path.basename(p)); continue
    if len(v) != n: print("%-22s  (length %d != %d)" % (os.path.basename(p), len(v), n)); continue
    l2 = math.sqrt(sum((a - b) ** 2 for a, b in zip(v, ref)))
    rel = l2 / math.sqrt(sum(x * x for x in ref))
    mx = max(abs(a - b) for a, b in zip(v, ref))
    am = max(range(n), key=lambda i: v[i])
    # How much of the top-1 margin survives: the quantity that actually decides the token.
    order = sorted(range(n), key=lambda i: -ref[i])[:10]
    top10 = sum(1 for i in order if i in sorted(range(n), key=lambda i: -v[i])[:10])
    gap = abs((v[top_ref] - max(v[i] for i in range(n) if i != top_ref))
              - (ref[top_ref] - max(ref[i] for i in range(n) if i != top_ref)))
    rows.append((rel, os.path.basename(p), rel, mx, gap, am, top10))
    print("%-22s %12.3e %12.3e %12.3e %8d %6d/10" % (os.path.basename(p), rel, mx, gap, am, top10))
if rows:
    rows.sort()
    print("\nCLOSEST TO THE ORACLE: %s (rel_L2 %.3e)" % (rows[0][1], rows[0][2]))
    for i in range(1, len(rows)):
        print("  %s is %.2fx further" % (rows[i][1], rows[i][2] / rows[0][2]))
