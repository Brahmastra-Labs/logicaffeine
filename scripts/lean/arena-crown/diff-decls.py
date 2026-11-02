#!/usr/bin/env python3
"""Where our checker spends more than a rival, declaration by declaration.

    scripts/lean/arena-crown/diff-decls.py <ours.tsv> <rival prefix> [--top N] [--joined out.tsv]

<ours.tsv> is `logos_kernel --decl-costs` output (index instructions kind admitted name); the rival
files are `<rival prefix>.*` from rival-decl-costs.py (index instructions kind group name, one file
per worker thread). The rival's rows are summed by group — an inductive block's types,
constructors and recursors are one declaration to us, named by its first type — and joined with
ours by name. Prints the totals, the declarations and families ranked by our excess
(ours − rival), and the families ranked by our total, so a cost that the rival shares is told
apart from one it avoids.
"""
import argparse
import collections
import glob
import re


def read_ours(path):
    rows = {}
    with open(path) as f:
        next(f)
        for line in f:
            idx, instr, kind, _admitted, name = line.rstrip("\n").split("\t", 4)
            rows[name] = (int(idx), int(instr), kind)
    return rows


def read_rival(prefix):
    rows = collections.defaultdict(lambda: [0, "", 1 << 62])
    files = sorted(f for f in glob.glob(prefix + ".*") if f.rsplit(".", 1)[1].isdigit())
    if not files:
        raise SystemExit(f"no rival files match {prefix}.*")
    for path in files:
        with open(path) as f:
            next(f)
            for line in f:
                idx, instr, kind, group, _name = line.rstrip("\n").split("\t", 4)
                r = rows[group]
                r[0] += int(instr)
                if kind in ("inductive", "def", "thm", "axiom", "opaque", "quot") or not r[1]:
                    r[1] = kind
                r[2] = min(r[2], int(idx))
    return rows


SUFFIX_FAMILIES = [
    (re.compile(r"\.eq_\d+$|\.eq_def$|\.eq_unfold$"), "equation lemma"),
    (re.compile(r"\._?proof_\d+$"), "auxiliary proof"),
    (re.compile(r"\.match_\d+$|\.match_\d+\.splitter$"), "matcher"),
    (re.compile(r"\.(noConfusion|noConfusionType|ctorElim|ctorIdx|recOn|casesOn|brecOn|below|binductionOn|ibelow)(\.|$)"), "structural auxiliary"),
    (re.compile(r"\.inj(Eq)?$|\.injEq$|\.sizeOf_spec$|\._sizeOf_\d+$"), "constructor lemma"),
    (re.compile(r"(^|\.)inst[A-Z][^.]*$"), "instance"),
    (re.compile(r"\._simp_\d+$|\._simpArg|\._auxLemma\.\d+$"), "simp auxiliary"),
]


def family(name, kind):
    for pat, label in SUFFIX_FAMILIES:
        if pat.search(name):
            return label
    return {"thm": "theorem", "def": "definition", "inductive": "inductive", "mutual": "mutual block"}.get(kind, kind)


def prefix(name, depth):
    parts = name.split(".")
    return ".".join(parts[:depth]) if len(parts) > depth else name


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("ours")
    ap.add_argument("rival")
    ap.add_argument("--top", type=int, default=40)
    ap.add_argument("--joined")
    a = ap.parse_args()
    ours = read_ours(a.ours)
    rival = read_rival(a.rival)
    both = [n for n in ours if n in rival]
    only_ours = [n for n in ours if n not in rival]
    only_rival = [n for n in rival if n not in ours]
    to = sum(v[1] for v in ours.values())
    tr = sum(v[0] for v in rival.values())
    jo = sum(ours[n][1] for n in both)
    jr = sum(rival[n][0] for n in both)
    G = 1e9
    print(f"ours  {to / G:10.3f} G over {len(ours)} declarations")
    print(f"rival {tr / G:10.3f} G over {len(rival)} groups")
    print(f"joined {len(both)}: ours {jo / G:.3f} G, rival {jr / G:.3f} G, excess {(jo - jr) / G:+.3f} G ({(jo / jr - 1) * 100 if jr else 0:+.1f}%)")
    if only_ours:
        print(f"only ours: {len(only_ours)} ({sum(ours[n][1] for n in only_ours) / G:.3f} G), e.g. {only_ours[:5]}")
    if only_rival:
        print(f"only rival: {len(only_rival)} ({sum(rival[n][0] for n in only_rival) / G:.3f} G), e.g. {only_rival[:5]}")

    joined = sorted(((ours[n][1] - rival[n][0], ours[n][1], rival[n][0], ours[n][2], n) for n in both), reverse=True)
    print(f"\n-- top {a.top} declarations by our excess (G) --")
    print(f"{'excess':>9} {'ours':>9} {'rival':>9}  kind       name")
    for ex, o, r, k, n in joined[: a.top]:
        print(f"{ex / G:9.4f} {o / G:9.4f} {r / G:9.4f}  {k:10} {n}")
    print(f"\n-- top {a.top // 2} declarations where we beat the rival (G) --")
    for ex, o, r, k, n in sorted(joined)[: a.top // 2]:
        print(f"{ex / G:9.4f} {o / G:9.4f} {r / G:9.4f}  {k:10} {n}")

    for label, key in [("family", lambda n: family(n, ours[n][2])), ("namespace (2)", lambda n: prefix(n, 2)), ("namespace (3)", lambda n: prefix(n, 3))]:
        agg = collections.defaultdict(lambda: [0, 0, 0])
        for n in both:
            g = agg[key(n)]
            g[0] += ours[n][1]
            g[1] += rival[n][0]
            g[2] += 1
        rows = sorted(agg.items(), key=lambda kv: kv[1][1] - kv[1][0])
        print(f"\n-- {label}: by our excess (G) --")
        print(f"{'excess':>9} {'ours':>9} {'rival':>9} {'ratio':>6} {'count':>7}  {label}")
        for k, (o, r, c) in rows[: a.top]:
            print(f"{(o - r) / G:9.3f} {o / G:9.3f} {r / G:9.3f} {o / r if r else float('inf'):6.2f} {c:7}  {k}")

    if a.joined:
        with open(a.joined, "w") as f:
            f.write("excess\tours\trival\tkind\tfamily\tname\n")
            for ex, o, r, k, n in joined:
                f.write(f"{ex}\t{o}\t{r}\t{k}\t{family(n, k)}\t{n}\n")


if __name__ == "__main__":
    main()
