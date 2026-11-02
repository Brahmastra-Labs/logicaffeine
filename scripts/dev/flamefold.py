#!/usr/bin/env python3
"""Fold `perf script` stacks into collapsed `stack count` lines and bucket
leaf self-time into allocation / output / compute.

Reads `perf script` output on stdin. Weights each sample by its perf period so
the resulting percentages match `perf report` (which is period-weighted, not
sample-count-weighted). No third-party deps.

Usage:
    perf script -i data | flamefold.py --folded out.folded --summary out.json \
        [--label-workload SYMBOL=NEWNAME] [--tree-json out.tree.json --root SYMBOL ...]

--tree-json writes the compact tree the /benchmarks page draws as an interactive flamegraph (see `tree()`).
"""
import sys, re, json, argparse, subprocess

# Leaf-symbol classifiers. A frame is "allocation" or "output" only when it is
# the LEAF (the function actually on-CPU at sample time) — that is self-time.
ALLOC = re.compile(
    r'(?:^|[^a-zA-Z])(?:malloc|free|realloc|calloc|memalign|aligned_alloc'
    r'|_int_malloc|_int_free|_int_realloc|malloc_consolidate|tcache'
    r'|__rust_alloc|__rust_dealloc|__rust_realloc|handle_alloc_error'
    r'|do_reserve_and_handle|arena_|sysmalloc)', re.I)
# Real stdout I/O — the "printf" the accusation is about.
STDOUT = re.compile(
    r'(?:printf|__printf|vfprintf|_print\b|\bfwrite\b|\b__write\b|\bwrite\b'
    r'|writev|_IO_|puts\b)', re.I)
# In-memory formatting (int/float -> text). For string/number benchmarks this
# IS the workload, not I/O — kept separate so it is not miscounted as "printf".
FMT = re.compile(
    r'(?:core::fmt|float_to_decimal|float_to_general|Formatter|Display.*fmt'
    r'|fmt::num|fmt::float|write_str)', re.I)

HEADER_PERIOD = re.compile(r':\s+(\d+)\s+\S+:\s*$')


def parse_frame(line):
    """`\t  <hexaddr> <symbol>+0xNN (<dso>)` -> symbol (or '[unknown]')."""
    s = line.strip()
    parts = s.split(None, 1)
    if len(parts) < 2:
        return "[unknown]"
    rest = parts[1]
    rest = re.sub(r'\s+\([^()]*\)\s*$', '', rest)     # drop trailing (dso)
    rest = re.sub(r'\+0x[0-9a-fA-F]+$', '', rest)      # drop +offset
    rest = rest.strip()
    return rest if rest else "[unknown]"


RUST_HASH = re.compile(r'::h[0-9a-f]{16}$')
CRATE_DISAMBIGUATOR = re.compile(r'\[[0-9a-f]{8,16}\]')


def demangle_all(symbols):
    """Every symbol demangled in one c++filt pass (C++ and Rust legacy/v0), with the hashes that make a Rust frame
    unreadable stripped: `::h<16 hex>` suffixes and `[<hex>]` crate disambiguators."""
    symbols = sorted(set(symbols))
    try:
        out = subprocess.run(['c++filt'], input='\n'.join(symbols), capture_output=True, text=True, check=True).stdout
        plain = out.split('\n')[:len(symbols)]
    except (OSError, subprocess.CalledProcessError):
        plain = symbols
    if len(plain) != len(symbols):
        plain = symbols
    return {s: CRATE_DISAMBIGUATOR.sub('', RUST_HASH.sub('', d)) for s, d in zip(symbols, plain)}


OTHER = '(other)'
OUTSIDE = '(outside the program)'
JIT = '(JIT code, callers not unwound)'


def tree(folds, roots, min_frac=0.005, max_depth=48, max_nodes=400):
    """The flamegraph the page draws, from `stack → weight` folds.

    Each stack is re-rooted at its first frame named in `roots` (the program's own entry: `main`, `_logos_main
    [workload]`, the VM's run loop), so the startup and runtime frames above it are not the story. A stack that stops
    inside JIT code (a `… [jit]` frame from the LOGOS perf map: DWARF unwinding cannot climb out of generated code) is
    filed under `(JIT code, callers not unwound)` from that frame down; any other stack that never reaches a root is
    filed under `(outside the program)`, so the totals stay whole. Runs of `[unknown]` frames merge into
    one. Nodes below `min_frac` of the total fold into one `(other)` per parent (and, when even that is below the
threshold, into the parent's self weight — a node's total never changes), the depth stops at `max_depth`, and
    the threshold doubles until the tree has at most `max_nodes` nodes. Returns `{total, frames, kinds, tree, trim}`
    where a node is `[frame index, total weight, [children…]]`, self weight is total minus the children's, `kinds[i]` is
    `frame_kind(frames[i])`, and `trim` records the bounds and the fraction below which nodes were finally folded.
    """
    total = sum(folds.values())
    root = {}

    def insert(path, weight):
        node = root
        for name in path:
            child = node.setdefault(name, [0, {}])
            child[0] += weight
            node = child[1]

    for stack, weight in folds.items():
        frames = stack.split(';')
        at = next((i for i, f in enumerate(frames) if f in roots), None)
        jit = next((i for i, f in enumerate(frames) if f.endswith(' [jit]')), None)
        if at is not None:
            path = frames[at:]
        elif jit is not None:
            path = [JIT] + frames[jit:]
        else:
            path = [OUTSIDE] + frames
        merged = []
        for f in path:
            if f == '[unknown]' and merged and merged[-1] == '[unknown]':
                continue
            merged.append(f)
        insert(merged[:max_depth], weight)

    def trim(children, threshold):
        kept, other = {}, 0
        for name, (weight, grand) in children.items():
            if weight < threshold:
                other += weight
            else:
                kept[name] = [weight, trim(grand, threshold)]
        if other >= threshold:
            prev = kept.get(OTHER, [0, {}])
            kept[OTHER] = [prev[0] + other, prev[1]]
        return kept

    def count(children):
        return sum(1 + count(grand) for _, (_, grand) in children.items())

    threshold = max(1, total * min_frac)
    trimmed = trim(root, threshold)
    while count(trimmed) > max_nodes:
        threshold *= 2
        trimmed = trim(root, threshold)

    frames, index = [], {}

    def encode(name, weight, children):
        if name not in index:
            index[name] = len(frames)
            frames.append(name)
        kids = sorted(children.items(), key=lambda kv: -kv[1][0])
        return [index[name], weight, [encode(n, w, g) for n, (w, g) in kids]]

    trim = {'min_frac': min_frac, 'folded_below_frac': round(threshold / total, 6) if total else 0.0,
            'max_depth': max_depth, 'max_nodes': max_nodes}
    encoded = encode('all', total, trimmed)
    return {'total': total, 'frames': frames, 'kinds': [frame_kind(f) for f in frames], 'tree': encoded, 'trim': trim}


SYNTHETIC = {'all', OTHER, OUTSIDE, JIT, '[unknown]'}


def leaf_bucket(sym):
    """Where a frame's self time goes: `alloc`, `fmt` (number→text in memory — `core::fmt::write` is formatting, not
    I/O, so formatting is recognised before the I/O names), `stdout`, or `compute`. One classifier for the summary's
    percentages and the flamegraph's colours."""
    if ALLOC.search(sym):
        return 'alloc'
    if FMT.search(sym):
        return 'fmt'
    if STDOUT.search(sym):
        return 'stdout'
    return 'compute'


def frame_kind(name):
    """What a frame is, for colouring the flamegraph: `synthetic` (the root, a trim bucket, an unwound stretch),
    `jit` (code the LOGOS JIT generated), or its `leaf_bucket` — `alloc`, `output`, `fmt`, `compute`."""
    if name in SYNTHETIC:
        return 'synthetic'
    if name.endswith(' [jit]'):
        return 'jit'
    kind = leaf_bucket(name)
    return 'output' if kind == 'stdout' else kind


def top_self(folds, count=5):
    """The `count` frames with the most self time — each frame's weight summed over every stack it is the leaf of,
    whatever called it — as `{frame, self_pct}`, largest first. Taken from the untrimmed profile, so a hot leaf the
    tree folds into `(other)` under several callers still shows here at its full weight."""
    total = sum(folds.values()) or 1
    self_weight = {}
    for stack, weight in folds.items():
        leaf = stack.rsplit(';', 1)[-1]
        self_weight[leaf] = self_weight.get(leaf, 0) + weight
    ranked = sorted(self_weight.items(), key=lambda kv: (-kv[1], kv[0]))[:count]
    return [{'frame': name, 'self_pct': round(100 * weight / total, 3)} for name, weight in ranked]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--folded', required=True)
    ap.add_argument('--summary', required=True)
    ap.add_argument('--label-workload', default=None,
                    help='SYMBOL=NEWNAME rename for the inlined workload frame')
    ap.add_argument('--name', default='')
    ap.add_argument('--lang', default='')
    ap.add_argument('--n', default='')
    ap.add_argument('--tree-json', default=None, help='write the trimmed flamegraph tree here')
    ap.add_argument('--root', action='append', default=[], help='a frame the program starts at (repeatable)')
    args = ap.parse_args()

    rename = {}
    if args.label_workload and '=' in args.label_workload:
        k, v = args.label_workload.split('=', 1)
        rename[k] = v

    folds = {}
    stack = []
    period = 1

    def flush():
        if stack:
            key = ";".join(reversed(stack))
            folds[key] = folds.get(key, 0) + period

    for line in sys.stdin:
        if line.strip() == "":
            flush(); stack = []; period = 1
            continue
        if line[0].isspace():
            sym = parse_frame(line)
            sym = rename.get(sym, sym)
            stack.append(sym)
        else:
            # header/comm line: pull the sample period out of it
            m = HEADER_PERIOD.search(line)
            period = int(m.group(1)) if m else 1
    flush()

    total = sum(folds.values()) or 1

    def leaf(k):
        return k.split(";")[-1]

    tally = {'alloc': 0, 'stdout': 0, 'fmt': 0, 'compute': 0}
    for k, c in folds.items():
        tally[leaf_bucket(leaf(k))] += c
    alloc, sout, fmt = tally['alloc'], tally['stdout'], tally['fmt']
    compute = total - alloc - sout - fmt

    with open(args.folded, 'w') as f:
        for k in sorted(folds, key=lambda x: -folds[x]):
            f.write(f"{k} {folds[k]}\n")

    summary = {
        "name": args.name, "lang": args.lang, "n": args.n,
        "samples_weight": total,
        "alloc_pct": round(100 * alloc / total, 3),
        "stdout_pct": round(100 * sout / total, 3),
        "fmt_pct": round(100 * fmt / total, 3),
        "output_pct": round(100 * (sout + fmt) / total, 3),
        "compute_pct": round(100 * compute / total, 3),
    }
    with open(args.summary, 'w') as f:
        json.dump(summary, f, indent=2)
    if args.tree_json:
        names = demangle_all(sym for k in folds for sym in k.split(';'))
        plain = {}
        for k, c in folds.items():
            key = ';'.join(names[sym] for sym in k.split(';'))
            plain[key] = plain.get(key, 0) + c
        roots = set(args.root) | {names.get(r, r) for r in args.root}
        with open(args.tree_json, 'w') as f:
            json.dump({**tree(plain, roots), 'top_self': top_self(plain),
                       'buckets': {k: summary[k] for k in
                                   ('alloc_pct', 'stdout_pct', 'fmt_pct', 'output_pct', 'compute_pct')}},
                      f, separators=(',', ':'))
    print(json.dumps(summary))


if __name__ == '__main__':
    main()
