#!/usr/bin/env python3
"""What a full run's tests cost, from its ledger (scripts/test/test-ledger.sh writes one line a test).

    test-ledger-report.py LEDGER [--top N] [--window SECONDS]
    test-ledger-report.py LEDGER --widths CORES [--min-cpu SECONDS] [--max-threads N]

The ledger can hold lines of a run nested inside the one recorded (a test that runs nextest itself); the report reads
the run with the most lines. It prints the run's totals, the heaviest tests by CPU, the widest (the tests that keep more
than one core busy, which nextest counts as one thread each), the most crowded stretch of the run and the tests that
crowded it, the largest peak memory, and the tests a signal ended. A test's CPU counts every process it waited for, and
is spread evenly over its wall clock when a stretch takes part of it.

With --widths it prints instead the tests at least that many cores wide, with at least --min-cpu seconds of CPU, as
full-profile nextest overrides: `threads-required` is the test's CPU over its wall clock, rounded, and at most
--max-threads. A test's wall clock in the gate is stretched by the gate's load and its CPU is not, so the width it
declares is never more than it kept busy there.
"""

import argparse
import collections
import sys

WIDE_CORES = 1.5
WIDE_WALL_S = 30.0


class Test:
    def __init__(self, fields):
        run, binary, name, start, end, wall, user, sys_, rss, outcome = fields
        self.run = run
        self.binary = binary
        self.test = name
        self.name = f"{binary}::{name}"
        self.start = float(start)
        self.end = float(end)
        self.wall = float(wall)
        self.cpu = None if user == "-" or sys_ == "-" else float(user) + float(sys_)
        self.rss_gb = None if rss == "-" else int(rss) / (1024 * 1024)
        self.outcome = outcome

    def cores(self):
        return self.cpu / self.wall if self.cpu is not None and self.wall > 0 else 0.0

    def cpu_within(self, lo, hi):
        if self.cpu is None or self.wall <= 0:
            return 0.0
        overlap = min(self.end, hi) - max(self.start, lo)
        return self.cpu * max(overlap, 0.0) / self.wall


def read(path):
    tests = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            fields = line.rstrip("\n").split("\t")
            if len(fields) != 10 or fields[0] == "run_id":
                continue
            tests.append(Test(fields))
    if not tests:
        return []
    run = collections.Counter(t.run for t in tests).most_common(1)[0][0]
    return [t for t in tests if t.run == run]


def clock(seconds):
    s = int(round(seconds))
    return f"+{s // 3600}:{s % 3600 // 60:02d}:{s % 60:02d}"


def crowded(tests, window):
    origin = min(t.start for t in tests)
    span = max(t.end for t in tests) - origin
    step = window / 10
    best = (-1.0, origin)
    at = origin
    while True:
        busy = sum(t.cpu_within(at, at + window) for t in tests)
        if busy > best[0]:
            best = (busy, at)
        if at + window >= origin + span:
            break
        at += step
    busy, at = best
    return at - origin, busy / window, sorted(tests, key=lambda t: -t.cpu_within(at, at + window)), at


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("ledger")
    ap.add_argument("--top", type=int, default=10)
    ap.add_argument("--window", type=float, default=600.0)
    ap.add_argument("--widths", type=float)
    ap.add_argument("--min-cpu", type=float, default=120.0)
    ap.add_argument("--max-threads", type=int, default=24)
    args = ap.parse_args()

    tests = read(args.ledger)
    if args.widths is not None:
        wide = [t for t in tests if t.cpu is not None and t.cpu >= args.min_cpu and t.cores() >= args.widths]
        for t in sorted(wide, key=lambda t: (t.binary, t.test)):
            print("[[profile.full.overrides]]")
            print(f"filter = 'binary_id({t.binary}) & test(={t.test})'")
            print(f"threads-required = {min(args.max_threads, int(t.cores() + 0.5))}")
            print()
        return 0
    if not tests:
        print(f"ledger: no tests in {args.ledger}")
        return 0
    cpu = sum(t.cpu or 0.0 for t in tests)
    span = max(t.end for t in tests) - min(t.start for t in tests)
    print(f"ledger: {len(tests)} tests, CPU {cpu / 3600:.2f} h over {span / 3600:.2f} h of wall clock "
          f"({cpu / span if span > 0 else 0.0:.1f} cores busy on average)")

    print("heaviest by CPU (CPU s, wall s, cores, peak GB, test):")
    for t in sorted((t for t in tests if t.cpu is not None), key=lambda t: -t.cpu)[: args.top]:
        rss = f"{t.rss_gb:6.2f}" if t.rss_gb is not None else "     -"
        print(f"  {t.cpu:8.0f} {t.wall:9.1f} {t.cores():5.1f} {rss}  {t.name}")

    print(f"widest (at least {WIDE_CORES} cores over {WIDE_WALL_S:.0f} s or more; cores, CPU s, wall s, test):")
    wide = [t for t in tests if t.wall >= WIDE_WALL_S and t.cores() >= WIDE_CORES]
    for t in sorted(wide, key=lambda t: -t.cores())[: args.top]:
        print(f"  {t.cores():5.1f} {t.cpu:8.0f} {t.wall:9.1f}  {t.name}")

    offset, cores, by, at = crowded(tests, args.window)
    print(f"most crowded {args.window:.0f} s: {clock(offset)} to {clock(offset + args.window)} at {cores:.1f} cores "
          f"(cores within it, test):")
    for t in by[: args.top]:
        share = t.cpu_within(at, at + args.window) / args.window
        if share <= 0:
            break
        print(f"  {share:5.1f}  {t.name}")

    print("largest peak memory (GB, test):")
    for t in sorted((t for t in tests if t.rss_gb is not None), key=lambda t: -t.rss_gb)[: args.top]:
        print(f"  {t.rss_gb:6.2f}  {t.name}")

    signalled = [t for t in tests if t.outcome.startswith("signal")]
    if signalled:
        print("ended by a signal (how, wall s, test):")
        for t in sorted(signalled, key=lambda t: -t.wall):
            print(f"  {t.outcome:10} {t.wall:9.1f}  {t.name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
