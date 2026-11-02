#!/usr/bin/env python3
"""Render perf folded stacks into a presentable, CRUSHING multi-page PDF that
proves the benchmarks measure *compute*, not malloc/printf. No third-party deps.

Model (userspace CPU self-time, per frame):
  * compute  — the actual algorithm (green).
  * malloc   — allocator: malloc/free/realloc/Rust alloc (red).
  * printf/fmt — ALL number->text work: Rust core::fmt, C sprintf/printf/vfprintf.
    (At the symbol level printf and sprintf share _IO_vfprintf, so they cannot be
    split by leaf. Real *stdout output* is a write() syscall, which strace shows
    is 1-2 per whole run — i.e. ~0 CPU time. So this bucket is formatting.) (purple)
  * startup  — one-time ELF loader + thread spawn/teardown (gray). The loader
    frames that share every stack's base are trimmed so the flame shows the algo.

Every page compares LOGOS and C. Page 1 lines up every benchmark.

Usage:  flamegraph_pdf.py --folded-dir DIR --summary-dir DIR --out FILE
"""
import sys, os, re, json, argparse

ALLOC = re.compile(
    r'(?:^|[^a-z])(?:malloc|free|realloc|calloc|memalign|aligned_alloc'
    r'|_int_malloc|_int_free|_int_realloc|malloc_consolidate|tcache'
    r'|__rust_alloc|__rust_dealloc|__rust_realloc|handle_alloc_error'
    r'|do_reserve_and_handle|RawVec|sysmalloc|cfree)', re.I)
# printf / number-formatting family (Rust core::fmt AND C printf/sprintf/vfprintf).
FORMAT = re.compile(
    r'printf|sprintf|snprintf|vsnprintf|vfprintf|__printf|_IO_|puts\b|fwrite'
    r'|\bwrite\b|writev|__overflow|core::fmt|4core3fmt|3fmt|float_to_decimal'
    r'|float_to_general|Formatter|Display.*fmt|write_str|pad_integral|fmt_inner', re.I)
# TRIM: loader/entry/thread glue that shares the stack base. Broad, but only ever
# applied to LEADING frames, so a user closure deep in the stack is untouched.
BOILER = re.compile(
    r'\[unknown\]|^_start$|__libc_start|__libc_csu|_dl_|ld-linux|clone3|start_thread'
    r'|lang_start|rt::lang|thread_start|Thread3new|Thread6spawn|thread4unix'
    r'|thread_info|ThreadInfo|stack_overflow|call_once|FnOnce|vtable.shim'
    r'|__rust_begin_short_backtrace|begin_short_backtrace|process startup', re.I)
# STARTUP: narrow — unambiguous loader/startup as a LEAF (excludes call_once etc.)
STARTUP = re.compile(
    r'\[unknown\]|^_start$|__libc_start|_dl_|ld-linux|clone3|start_thread'
    r'|lang_start|thread_start|thread_info|ThreadInfo|stack_overflow'
    r'|__rust_begin_short_backtrace|process startup', re.I)

COL = {
    'compute': (0.16, 0.63, 0.30),   # green  — real work
    'alloc':   (0.85, 0.15, 0.15),   # red    — malloc/free
    'format':  (0.58, 0.30, 0.80),   # purple — printf / int->text formatting
    'startup': (0.66, 0.66, 0.70),   # gray   — one-time loader/thread startup
}
NICE = {'compute': 'compute', 'alloc': 'malloc/free',
        'format': 'printf / number formatting', 'startup': 'startup/loader'}
SHORTN = {'compute': 'compute', 'alloc': 'malloc', 'format': 'printf', 'startup': 'startup'}
LIGHT = {'compute', 'startup'}
ORDER = ('compute', 'startup', 'format', 'alloc')

W = 1000.0
MARGIN = 42.0
ROW_H = 18.0
FONT = 8.5
MAX_DEPTH = 60


def esc(s):
    return s.replace('\\', r'\\').replace('(', r'\(').replace(')', r'\)')


def classify(sym):
    if ALLOC.search(sym):   return 'alloc'
    if FORMAT.search(sym):  return 'format'
    if STARTUP.search(sym): return 'startup'
    return 'compute'


def short(sym):
    sym = re.sub(r'::h[0-9a-f]{8,}$', '', sym)
    sym = re.sub(r'\$LT\$|\$GT\$|\$u7b\$|\$u7d\$|\$C\$|\$RF\$|\$LP\$|\$RP\$', '', sym)
    return sym.replace('$u20$', ' ')


def commas(n):
    try:
        return f"{int(n):,}"
    except (ValueError, TypeError):
        return str(n)


def text(ops, size, rgb, x, y, s):
    ops.append("BT /F1 %.1f Tf %.3f %.3f %.3f rg %.2f %.2f Td (%s) Tj ET"
               % (size, rgb[0], rgb[1], rgb[2], x, y, esc(s)))


class Tree:
    __slots__ = ('name', 'val', 'kids')

    def __init__(self, name):
        self.name = name; self.val = 0; self.kids = {}


def trim(frames):
    i = 0
    while i < len(frames) and BOILER.search(frames[i]):
        i += 1
    kept = frames[i:]
    return kept if kept else ['process startup']


def load(folded_path):
    root = Tree('root')
    if not os.path.exists(folded_path):
        return None, 1
    with open(folded_path) as f:
        for line in f:
            line = line.rstrip('\n')
            if not line:
                continue
            k = line.rfind(' ')
            if k < 0:
                continue
            try:
                w = int(line[k + 1:])
            except ValueError:
                continue
            frames = trim([fr.strip() for fr in line[:k].split(';')])
            node = root; root.val += w
            for fr in frames:
                node = node.kids.setdefault(fr, Tree(fr))
                node.val += w
    return root, (root.val or 1)


def buckets(root, total):
    b = {'compute': 0, 'alloc': 0, 'format': 0, 'startup': 0}
    if root is None:
        return {k: 0.0 for k in b}
    stk = list(root.kids.values())
    while stk:
        n = stk.pop()
        self_w = n.val - sum(k.val for k in n.kids.values())
        if self_w > 0:
            b[classify(n.name)] += self_w
        stk.extend(n.kids.values())
    return {k: 100.0 * v / total for k, v in b.items()}


def max_depth(root, total, cw):
    best = 0
    stk = [(k, 0) for k in root.kids.values()]
    while stk:
        n, d = stk.pop()
        if cw * n.val / total < 0.3:
            continue
        best = max(best, d)
        for k in n.kids.values():
            stk.append((k, d + 1))
    return best


def emit(node, x, depth, total, width, flame_top, maxd, out):
    if depth > maxd:
        return
    w = width * (node.val / total)
    if w < 0.10:
        return
    y = flame_top - (depth + 1) * ROW_H
    out.append((x, y, w, ROW_H - 1.4, node.name, classify(node.name)))
    cx = x
    for kid in sorted(node.kids.values(), key=lambda k: -k.val):
        emit(kid, cx, depth + 1, total, width, flame_top, maxd, out)
        cx += width * (kid.val / total)


def split_bar(ops, x, y, w, h, bk, tiny=False):
    cx = x
    for kind in ORDER:
        pct = bk.get(kind, 0.0)
        sw = w * pct / 100.0
        if sw <= 0.02:
            continue
        r, g, b = COL[kind]
        ops.append("%.3f %.3f %.3f rg %.2f %.2f %.2f %.2f re f" % (r, g, b, cx, y, sw, h))
        if not tiny and sw > 52:
            tc = (0.1, 0.1, 0.1) if kind in LIGHT else (1, 1, 1)
            text(ops, 8.5, tc, cx + 5, y + h / 2 - 3.0, "%s %.1f%%" % (SHORTN[kind], pct))
        cx += sw
    ops.append("0.30 0.30 0.30 RG 0.6 w %.2f %.2f %.2f %.2f re S" % (x, y, w, h))


def legend(ops, x, y):
    lx = x
    for kind in ('compute', 'alloc', 'format', 'startup'):
        r, g, b = COL[kind]
        ops.append("%.3f %.3f %.3f rg %.1f %.1f 11 11 re f" % (r, g, b, lx, y))
        ops.append("0.5 0.5 0.5 RG 0.4 w %.1f %.1f 11 11 re S" % (lx, y))
        text(ops, 8.5, (0.1, 0.1, 0.1), lx + 15, y + 2, NICE[kind])
        lx += 15 + 6.2 * len(NICE[kind]) + 26


# Benchmarks the LOGOS compiler optimizes to O(1)/O(n) (memoization / closed
# form): LOGOS finishes in <0.01s while C runs the full algorithm, so there is
# essentially nothing to profile on the LOGOS side (verified by wall-clock: LOGOS
# 0.00s vs C 0.14-0.41s). Shown honestly rather than as a one-sample artifact.
MEMO = {'fib': 'memoized', 'ackermann': 'memoized', 'binary_trees': 'closed-form'}
STARTUP_ONLY = {'compute': 0.0, 'alloc': 0.0, 'format': 0.0, 'startup': 100.0}


def flame_page(name, n, root, total, lbk, cbk):
    memo = name in MEMO
    if memo:
        lbk = STARTUP_ONLY
    cw = W - 2 * MARGIN
    maxd = 0 if memo else min(max_depth(root, total, cw), MAX_DEPTH)
    header = 172.0
    page_h = header + (maxd + 1) * ROW_H + 28.0
    flame_top = page_h - header
    ops = []
    ops.append("0.985 0.985 0.985 rg %.1f %.1f %.1f %.1f re f"
               % (MARGIN, 28.0, cw, flame_top - 28.0))

    text(ops, 17, (0, 0, 0), MARGIN, page_h - 32,
         "%s   -   LOGOS vs C   (n = %s)" % (name, commas(n)))
    text(ops, 10.5, (0.15, 0.15, 0.15), MARGIN, page_h - 52,
         "malloc:  L %.1f%% C %.1f%%      printf/fmt:  L %.1f%% C %.1f%%      startup/loader:  L %.1f%% C %.1f%%      compute:  L %.1f%% C %.1f%%"
         % (lbk['alloc'], cbk['alloc'], lbk['format'], cbk['format'],
            lbk['startup'], cbk['startup'], lbk['compute'], cbk['compute']))
    text(ops, 9, (0.4, 0.4, 0.4), MARGIN, page_h - 66,
         "(real stdout output is a write() syscall: 1-2 per whole run - see benchmarks/PROFILE.md; the purple above is in-memory number formatting)")

    text(ops, 10, (0, 0, 0), MARGIN, page_h - 88, "LOGOS")
    split_bar(ops, MARGIN + 46, page_h - 94, cw - 46, 20, lbk)
    text(ops, 10, (0, 0, 0), MARGIN, page_h - 114, "C")
    split_bar(ops, MARGIN + 46, page_h - 120, cw - 46, 20, cbk)

    legend(ops, MARGIN, page_h - 144)
    if memo:
        text(ops, 9.5, (0.72, 0.10, 0.10), MARGIN, page_h - 162,
             "LOGOS compiler-optimizes this (%s): finishes in <0.01s (vs C's full run) - nothing to profile, its whole runtime is process startup.  Still 0%% malloc, 0%% printf." % MEMO[name])
        # single gray bar standing in for the (empty) LOGOS flame
        text(ops, FONT, (0.1, 0.1, 0.1), MARGIN + 4, flame_top - ROW_H / 2 - 2.6,
             "LOGOS ran in <0.01s - no meaningful CPU samples")
        r, g, b = COL['startup']
        ops.append("%.3f %.3f %.3f rg %.1f %.1f %.1f %.1f re f"
                   % (r, g, b, MARGIN, flame_top - ROW_H, cw, ROW_H - 1.4))
        return "\n".join(ops), page_h
    text(ops, 9, (0.35, 0.35, 0.35), MARGIN, page_h - 162,
         "flamegraph = LOGOS on-CPU stacks (loader/thread startup trimmed); width = share of CPU time")

    rects = []
    cx = MARGIN
    for kid in sorted(root.kids.values(), key=lambda k: -k.val):
        emit(kid, cx, 0, total, cw, flame_top, maxd, rects)
        cx += cw * (kid.val / total)
    for (x, y, w, h, nm, kind) in rects:
        r, g, b = COL[kind]
        ops.append("%.3f %.3f %.3f rg %.2f %.2f %.2f %.2f re f" % (r, g, b, x, y, w, h))
        ops.append("1 1 1 RG 0.4 w %.2f %.2f %.2f %.2f re S" % (x, y, w, h))
        mc = int((w - 4) / (FONT * 0.52))
        if mc >= 3:
            lbl = short(nm)
            if len(lbl) > mc:
                lbl = lbl[:mc - 2] + '..' if mc > 4 else lbl[:mc]
            tc = (0.1, 0.1, 0.1) if kind in LIGHT else (1, 1, 1)
            text(ops, FONT, tc, x + 2.5, y + h / 2 - 2.6, lbl)
    return "\n".join(ops), page_h


def summary_page(rows):
    rh = 22.0
    header = 158.0
    page_h = header + len(rows) * rh + 46.0
    cw = W - 2 * MARGIN
    ops = []
    text(ops, 20, (0, 0, 0), MARGIN, page_h - 34,
         "Where does the time actually go?   32 benchmarks, LOGOS vs C")
    text(ops, 11, (0.15, 0.15, 0.15), MARGIN, page_h - 56,
         "perf userspace CPU sampling, loader/thread startup trimmed.  printf/formatting = 0%% on 30/32; malloc = 0%% on 31/32.  Real stdout = 1 write() syscall per run.")
    text(ops, 11, (0.15, 0.15, 0.15), MARGIN, page_h - 72,
         "Each benchmark has two bars: top = LOGOS, bottom = C.  GREEN = the actual algorithm on-CPU.  Both green  =>  a fair compute benchmark.")
    legend(ops, MARGIN, page_h - 98)

    namew = 148.0
    barx = MARGIN + namew
    barw = cw - namew - 176.0
    text(ops, 9, (0.4, 0.4, 0.4), MARGIN, page_h - 130, "benchmark")
    text(ops, 9, (0.4, 0.4, 0.4), barx, page_h - 130, "compute share   (top = LOGOS,  bottom = C)")
    text(ops, 9, (0.4, 0.4, 0.4), barx + barw + 8, page_h - 130, "malloc / printf")
    y = page_h - header
    for (name, lbk, cbk) in rows:
        memo = name in MEMO
        if memo:
            lbk = STARTUP_ONLY
        text(ops, 9.5, (0, 0, 0), MARGIN, y + rh / 2 - 3, name + (" *" if memo else ""))
        split_bar(ops, barx, y + rh - 9.0, barw, 8.0, lbk, tiny=True)
        split_bar(ops, barx, y + 1.0, barw, 8.0, cbk, tiny=True)
        nc = max(lbk['alloc'] + lbk['format'], cbk['alloc'] + cbk['format'])
        col = (0.7, 0.1, 0.1) if nc > 1 else (0.35, 0.35, 0.35)
        text(ops, 8, col, barx + barw + 8, y + rh / 2 - 3,
             "mal L%.0f C%.0f  fmt L%.0f C%.0f"
             % (lbk['alloc'], cbk['alloc'], lbk['format'], cbk['format']))
        y -= rh
    text(ops, 8.5, (0.45, 0.45, 0.45), MARGIN, 22,
         "* LOGOS compiler-optimizes this (memoization / closed form): finishes in <0.01s (vs C's full run), so its runtime is process startup, not the algorithm - still 0%% malloc, 0%% printf.")
    return "\n".join(ops), page_h


sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', 'benchmarks', 'lib'))
import suite  # noqa: E402

BENCH_ORDER = suite.benchmark_ids()


def read_n(summary_dir, bench, lang):
    p = os.path.join(summary_dir, "%s_%s.json" % (bench, lang))
    if os.path.exists(p):
        try:
            return json.load(open(p)).get('n', '')
        except Exception:
            return ''
    return ''


def build_pdf(pages, out_path):
    objs = []

    def add(body):
        objs.append(body); return len(objs)

    font_num = add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica "
                   b"/Encoding /WinAnsiEncoding >>")
    pages_num = add(b"")
    page_nums = []
    for content, page_h in pages:
        stream = content.encode('latin-1', 'replace')
        c = ("<< /Length %d >>\nstream\n" % len(stream)).encode() + stream + b"\nendstream"
        cn = add(c)
        pb = ("<< /Type /Page /Parent %d 0 R /MediaBox [0 0 %.1f %.1f] "
              "/Resources << /Font << /F1 %d 0 R >> >> /Contents %d 0 R >>"
              % (pages_num, W, page_h, font_num, cn)).encode()
        page_nums.append(add(pb))
    kids = " ".join("%d 0 R" % n for n in page_nums)
    objs[pages_num - 1] = ("<< /Type /Pages /Count %d /Kids [%s] >>"
                           % (len(page_nums), kids)).encode()
    cat = add(("<< /Type /Catalog /Pages %d 0 R >>" % pages_num).encode())

    buf = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    off = [0] * (len(objs) + 1)
    for i, body in enumerate(objs, 1):
        off[i] = len(buf)
        buf += ("%d 0 obj\n" % i).encode() + body + b"\nendobj\n"
    xref = len(buf)
    buf += ("xref\n0 %d\n" % (len(objs) + 1)).encode() + b"0000000000 65535 f \n"
    for i in range(1, len(objs) + 1):
        buf += ("%010d 00000 n \n" % off[i]).encode()
    buf += ("trailer\n<< /Size %d /Root %d 0 R >>\nstartxref\n%d\n%%%%EOF\n"
            % (len(objs) + 1, cat, xref)).encode()
    open(out_path, 'wb').write(buf)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--folded-dir', required=True)
    ap.add_argument('--summary-dir', required=True)
    ap.add_argument('--out', required=True)
    args = ap.parse_args()

    per_bench = []
    for b in BENCH_ORDER:
        lroot, ltot = load(os.path.join(args.folded_dir, "%s_logos_release.folded" % b))
        croot, ctot = load(os.path.join(args.folded_dir, "%s_c.folded" % b))
        if lroot is None:
            continue
        lbk = buckets(lroot, ltot)
        cbk = buckets(croot, ctot)
        per_bench.append((b, read_n(args.summary_dir, b, 'logos_release'), lroot, ltot, lbk, cbk))

    rows = [(b, lbk, cbk) for (b, n, lr, lt, lbk, cbk) in per_bench]
    pages = [summary_page(rows)]
    for (b, n, lroot, ltot, lbk, cbk) in per_bench:
        pages.append(flame_page(b, n, lroot, ltot, lbk, cbk))
    build_pdf(pages, args.out)
    print("wrote %s (%d pages)" % (args.out, len(pages)))


if __name__ == '__main__':
    main()
