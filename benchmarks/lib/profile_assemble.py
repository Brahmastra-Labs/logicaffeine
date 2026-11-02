#!/usr/bin/env python3
"""Assemble latest-profile.json, the file the /benchmarks page reads for each benchmark's "under the hood" section,
from what scripts/bench/profile-benchmarks.sh leaves behind:

    <prof>/tree/<bench>_<tier>.json      flamefold.py --tree-json: the trimmed flamegraph, buckets, top self frames
    <prof>/counters/<bench>_<tier>.csv   perf stat -x, (absent when perf stat failed: counters null)
    <rows>                               profile.json: one census row per (benchmark, tier)

    python3 benchmarks/lib/profile_assemble.py --prof DIR --rows profile.json --events E --cpus 0-15 \\
        --gate passed --paranoid 1 > latest-profile.json

Schema 1:
    {schema, generated, environment{cpus, quiet_box, perf_event_paranoid, perf, kernel, cpu}, method{…},
     benchmarks{<id>: {n, tiers{<tier>: {seconds, sample_hz, stat_repeats, total, frames, kinds, tree, trim, buckets,
                                         top_self, counters, census, scaling_n_to_2n}}}}}

Benchmarks come in registry order, tiers in lib/languages.tsv order. Anything inconsistent — a row whose tree is
missing, two sizes for one benchmark, a tier the registry does not profile — stops the assembly with its name.
"""
import argparse
import datetime
import json
import pathlib
import platform
import re
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import suite  # noqa: E402

PMU_EVENT = re.compile(r'[\w-]+/([\w.-]+)/[A-Za-z]*')


def fail(message):
    sys.exit(f"profile_assemble: {message}")


def event_name(raw):
    """`cpu_core/instructions/u` and `instructions:u` are both `instructions`."""
    m = PMU_EVENT.fullmatch(raw)
    if m:
        return m.group(1)
    return re.sub(r':[A-Za-z]+$', '', raw)


def number(text):
    try:
        return int(text)
    except ValueError:
        pass
    try:
        return float(text)
    except ValueError:
        return None


def counters(path):
    """perf stat -x, output as {event: {value, unit, variance_pct}} in the order perf printed them. The value is null
    for `<not counted>` / `<not supported>`; variance is null for a single run. Fields are located by shape, not by
    position: perf's documentation and perf's output disagree about where the variance goes."""
    if not path.exists():
        return None
    out = {}
    for line in path.read_text().splitlines():
        if not line.strip() or line.startswith('#'):
            continue
        fields = line.split(',')
        if len(fields) < 3 or not fields[2]:
            continue
        value, unit, event = fields[0], fields[1], fields[2]
        variance = next((f for f in fields[3:] if f.endswith('%')), None)
        out[event_name(event)] = {
            'value': number(value),
            'unit': unit or None,
            'variance_pct': number(variance[:-1]) if variance else None,
        }
    return out or None


def census(row):
    static, trace = row.get('B_static') or {}, row.get('C_strace') or {}
    fields = {
        'alloc_call_sites': static.get('alloc_call_sites'),
        'output_call_sites': static.get('output_call_sites'),
        'write_calls': trace.get('write_calls'),
        'syscall_seconds': trace.get('mem_write_syscall_seconds'),
    }
    return None if all(v is None for v in fields.values()) else fields


def scaling(row):
    ratio = (row.get('C_strace') or {}).get('scaling_ratio_n_to_2n')
    return ratio if isinstance(ratio, (int, float)) else None


def environment(args):
    def perf_version():
        try:
            return subprocess.run(['perf', '--version'], capture_output=True, text=True, check=True).stdout.strip()
        except (OSError, subprocess.CalledProcessError):
            return None

    def cpu_model():
        try:
            for line in pathlib.Path('/proc/cpuinfo').read_text().splitlines():
                if line.startswith('model name'):
                    return line.split(':', 1)[1].strip()
        except OSError:
            pass
        return None

    return {
        'cpus': args.cpus,
        'quiet_box': args.gate,
        'perf_event_paranoid': number(args.paranoid) if args.paranoid else None,
        'perf': perf_version(),
        'kernel': platform.release(),
        'cpu': cpu_model(),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--prof', required=True)
    ap.add_argument('--rows', required=True)
    ap.add_argument('--events', required=True)
    ap.add_argument('--cpus', required=True)
    ap.add_argument('--gate', required=True)
    ap.add_argument('--paranoid', default='')
    args = ap.parse_args()

    prof = pathlib.Path(args.prof)
    rows = json.loads(pathlib.Path(args.rows).read_text())['rows']
    tiers = [l['id'] for l in suite.languages() if l['profile'] == '1']
    bench_order = suite.benchmark_ids()

    by_bench = {}
    for row in rows:
        bench, tier = row['bench'], row['lang']
        if tier not in tiers:
            fail(f"{bench}: tier '{tier}' is not a profiled tier in lib/languages.tsv ({', '.join(tiers)})")
        by_bench.setdefault(bench, []).append(row)

    benchmarks = {}
    for bench in sorted(by_bench, key=lambda b: (bench_order.index(b) if b in bench_order else len(bench_order), b)):
        sizes = sorted({row['n'] for row in by_bench[bench]}, key=lambda n: int(n))
        if len(sizes) != 1:
            fail(f"{bench}: the tiers were profiled at different sizes ({', '.join(sizes)}) and do not compare")
        entry = {}
        for row in sorted(by_bench[bench], key=lambda r: tiers.index(r['lang'])):
            tree_path = prof / 'tree' / f"{bench}_{row['lang']}.json"
            if not tree_path.exists():
                fail(f"{bench}/{row['lang']}: a census row without its flamegraph ({tree_path})")
            tree = json.loads(tree_path.read_text())
            entry[row['lang']] = {
                'seconds': row.get('seconds'),
                'sample_hz': row.get('sample_hz'),
                'stat_repeats': row.get('stat_repeats'),
                'total': tree['total'],
                'frames': tree['frames'],
                'kinds': tree.get('kinds'),
                'tree': tree['tree'],
                'trim': tree.get('trim'),
                'buckets': tree.get('buckets'),
                'top_self': tree.get('top_self'),
                'counters': counters(prof / 'counters' / f"{bench}_{row['lang']}.csv"),
                'census': census(row),
                'scaling_n_to_2n': scaling(row),
            }
        benchmarks[bench] = {'n': int(sizes[0]), 'tiers': entry}

    doc = {
        'schema': 1,
        'generated': datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'),
        'environment': environment(args),
        'method': {
            'sampling': 'perf record -e cycles:u --call-graph=dwarf, pinned, the rate scaled so every tier yields '
                        'about the same number of samples',
            'counters': f'perf stat -x, -e {args.events}',
            'jit_symbols': 'LOGOS_PERF_MAP=1: /tmp/perf-<pid>.map names generated code logos::<function> [jit]',
        },
        'benchmarks': benchmarks,
    }
    json.dump(doc, sys.stdout, separators=(',', ':'))
    sys.stdout.write('\n')


if __name__ == '__main__':
    main()
