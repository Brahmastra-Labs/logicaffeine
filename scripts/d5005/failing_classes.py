#!/usr/bin/env python3
"""The failing endpoints of a fitted image grouped by the net they share.

    failing_classes.py <failing-paths.tsv> [--top N] [--worst N] [--set-aside PATTERN]...

`failing-paths.tsv` is what `failing-paths.tcl` writes (run by `quartus.sh sta`): an endpoint a line, with its slack,
its data delay, its logic levels and the widest fan-out on its path with the node driving it. Each endpoint's class is
that node's name below the shell, its indices and numbers made `[*]` and `#`, so the copies of one net across cores,
channels, lanes and banks are one class. Printed, widest classes first: the class's endpoints, their share and the
running share, its worst slack, its most common logic levels and data delay, and the nodes its paths start and end at
most often. `--worst N` classes only the N endpoints of least slack (the ones that set the clock); `--set-aside`
leaves out every endpoint whose class or first node contains PATTERN (a class already fixed in the tree being built),
and says how many it left out.
"""
import argparse
import collections
import csv
import re
import sys


def below_shell(name):
    return name.split("|shell|")[-1]


def class_of(name):
    name = re.sub(r"\[\d+\]", "[*]", name)
    name = re.sub(r"\d+", "#", name)
    return below_shell(name)


def main(argv):
    p = argparse.ArgumentParser(description="the failing endpoints of a fitted image grouped by the net they share")
    p.add_argument("tsv")
    p.add_argument("--top", type=int, default=30, help="classes printed")
    p.add_argument("--worst", type=int, default=0, help="class only the endpoints of least slack, this many")
    p.add_argument("--set-aside", action="append", default=[], metavar="PATTERN", help="leave out a class or first node containing it")
    args = p.parse_args(argv)
    with open(args.tsv, newline="") as f:
        rows = list(csv.DictReader(f, delimiter="\t"))
    if not rows:
        print("no failing endpoints")
        return 0
    rows.sort(key=lambda r: float(r["slack"]))
    total = len(rows)
    kept = [r for r in rows if not any(s in class_of(r["fanout_node"]) or s in class_of(r["from"]) for s in args.set_aside)]
    if args.set_aside:
        print(f"{total - len(kept)} of {total} endpoints set aside ({', '.join(args.set_aside)})")
    if args.worst:
        kept = kept[: args.worst]
    if not kept:
        print("no endpoints left")
        return 0
    print(f"{len(kept)} endpoints, slack {kept[0]['slack']} to {kept[-1]['slack']} ns")
    classes = collections.OrderedDict()
    for r in kept:
        classes.setdefault(class_of(r["fanout_node"]), []).append(r)
    running = 0
    for name, members in sorted(classes.items(), key=lambda kv: -len(kv[1]))[: args.top]:
        running += len(members)
        worst = min(float(m["slack"]) for m in members)
        levels = collections.Counter(m["levels"] for m in members).most_common(1)[0][0]
        delay = collections.Counter(round(float(m["data_delay"]), 1) for m in members if m["data_delay"] != "?").most_common(1)
        fanout = max((int(m["fanout"]) for m in members if m["fanout"].isdigit()), default=0)
        starts = collections.Counter(class_of(m["from"]) for m in members).most_common(1)[0][0]
        ends = collections.Counter(class_of(m["to"]) for m in members).most_common(1)[0][0]
        print(f"{len(members):7d} {100 * len(members) / len(kept):5.1f}% {100 * running / len(kept):5.1f}%  worst {worst:8.3f}  "
              f"levels {levels}  delay {delay[0][0] if delay else '?'}  fan-out {fanout}  {name}")
        print(f"          from {starts}")
        print(f"          to   {ends}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
