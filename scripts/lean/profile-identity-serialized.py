#!/usr/bin/env python3
"""Profile one local Lean Arena input with optimized Rust symbols.

No Arena submission or system security changes. Run this alone, after tests.
Example: python3 scripts/lean/profile-lean-arena.py --sudo-perf INPUT.ndjson
Outputs are ignored logs: perf data, folded stacks, SVG, timing and provenance.
"""
import argparse
import fcntl
from collections import Counter
from datetime import datetime, timezone
import hashlib
import html
import importlib.util
import json
import os
from pathlib import Path
import pwd
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent.parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sources():
    paths = [ROOT / "Cargo.toml", ROOT / "Cargo.lock"]
    for crate in ("logicaffeine_kernel", "logicaffeine_base"):
        paths.append(ROOT / "crates" / crate / "Cargo.toml")
        paths.extend((ROOT / "crates" / crate / "src").rglob("*.rs"))
    paths.extend((ROOT / "crates/logicaffeine_kernel/examples").rglob("*.rs"))
    return {str(p.relative_to(ROOT)): digest(p) for p in sorted(paths)}


def render(folded, output, title, note):
    # Share the repository's existing folded-stack tree and layout.
    spec = importlib.util.spec_from_file_location("flamegraph_pdf", ROOT / "scripts/dev/flamegraph_pdf.py")
    fg = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fg)
    tree, weight = fg.load(str(folded))
    if tree is None or not tree.kids:
        raise RuntimeError("No stacks captured; refusing to draw an empty profile")
    width = 1600
    depth = fg.max_depth(tree, weight, width)
    height = 110 + (depth + 1) * fg.ROW_H
    rects = []
    fg.emit(tree, 10, -1, weight, width, height - 30, depth, rects)
    svg = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="1620" height="{height}" viewBox="0 0 1620 {height}">',
        '<style>text{font-family:monospace;font-size:11px}g:hover rect{stroke:#111;stroke-width:1.5}</style>',
        '<rect width="100%" height="100%" fill="#fafafa"/>',
        f'<text x="10" y="24" style="font-size:18px">{html.escape(title)}</text>',
        '<text x="10" y="46">Width = weighted user-space CPU samples. Hover for full frame names. Red: allocator; green: compute.</text>',
        f'<text x="10" y="64">{html.escape(note)}</text>',
    ]
    for x, y, w, h, name, kind in rects:
        if w < 0.5:
            continue
        color = tuple(round(v * 255) for v in fg.COL[kind])
        svg.append(
            f'<g><title>{html.escape(name)} — {w / width * 100:.3f}% of weighted samples</title>'
            f'<rect x="{x:.3f}" y="{y:.3f}" width="{w:.3f}" height="{h:.3f}" fill="rgb{color}"/>'
        )
        if w > 32:
            label = fg.short(name)[:max(0, int((w - 6) / 6.6))]
            svg.append(f'<text x="{x + 3:.3f}" y="{y + 11:.3f}">{html.escape(label)}</text>')
        svg.append("</g>")
    svg.append("</svg>")
    output.write_text("\n".join(svg))

    inclusive, leaf, total = Counter(), Counter(), 0
    for line in folded.read_text().splitlines():
        stack, count = line.rsplit(" ", 1)
        count = int(count)
        frames = stack.split(";")
        total += count
        leaf[frames[-1]] += count
        for frame in set(frames):
            inclusive[frame] += count
    return {
        "inclusive": [{"symbol": f, "percent": 100 * w / total} for f, w in inclusive.most_common(40)],
        "self": [{"symbol": f, "percent": 100 * w / total} for f, w in leaf.most_common(40)],
    }


def measured_command(program, timing, seconds):
    """Run a checker with a time limit and collect its resource measurements."""
    return [
        "/usr/bin/time", "-q",
        "-f", '{"wall_seconds":%e,"user_seconds":%U,"system_seconds":%S,"max_rss_kib":%M}',
        "-o", str(timing), "timeout", str(seconds) + "s",
    ] + list(program)


import os, time, select

def wait_for_test_window():
 deadline=time.monotonic()+600;previous=None
 while True:
  busy=[]
  for entry in os.scandir('/proc'):
   if not entry.name.isdecimal():continue
   try:
    proc=Path(entry.path)
    comm=(proc/'comm').read_text().strip()
    argv=(proc/'cmdline').read_bytes().split(b'\0')
    is_test=bool(argv and b'/target' in argv[0] and b'/deps/' in argv[0] and (b'--exact' in argv or any(a.startswith(b'--test-threads') for a in argv)))
    if comm not in ('cargo-nextest','lean_arena') and not is_test:continue
    stat=(proc/'stat').read_text();state=stat[stat.rfind(')')+2:].split()[0]
    if state!='Z':busy.append(int(entry.name))
   except (FileNotFoundError,ProcessLookupError):continue
  busy.sort()
  if not busy:return
  if busy!=previous:print(json.dumps({'waiting_for_other_test_pids':busy}),flush=True);previous=busy
  left=deadline-time.monotonic()
  if left<=0:raise RuntimeError('test window unavailable; no concurrency exception authorized')
  descriptors=[]
  try:
   poller=select.poll()
   for pid in busy:
    try:fd=os.pidfd_open(pid)
    except ProcessLookupError:continue
    descriptors.append(fd);poller.register(fd,select.POLLIN)
   if descriptors:poller.poll(int(min(left,30)*1000))
  finally:
   for fd in descriptors:os.close(fd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--sudo-perf", action="store_true",
                        help="Use sudo only for perf; execute the checker as the invoking user")
    parser.add_argument("--seconds", type=int, default=30)
    parser.add_argument("--trace-stages", action="store_true",
                        help="Record importer declaration stages on stderr; diagnostic timings include tracing")
    parser.add_argument("--audit-declarations", action="store_true",
                        help="Independently recheck declaration types and bodies and audit axiom dependencies")
    args = parser.parse_args()
    if args.seconds <= 0:
        parser.error("--seconds must be positive")
    fixture = args.input.resolve(strict=True)
    (ROOT / "logs").mkdir(exist_ok=True)
    # Share the conformance harness's lock; measuring two checkers together
    # corrupts both the performance evidence and timeout classifications.
    measurement_lock = (ROOT / "logs/conformance-ratchet.lock").open("w")
    try:
        fcntl.flock(measurement_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        raise SystemExit("Another conformance/profile measurement is running")
    wait_for_test_window()
    stamp = datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S-%f")
    out = ROOT / "logs" / ("arena-profile-" + stamp)
    out.mkdir(parents=True)
    before = sources()
    fixture_hash = digest(fixture)
    env = os.environ.copy()
    env.update(CARGO_TARGET_DIR=str(ROOT / "target/arena-profile"),
               CARGO_PROFILE_RELEASE_DEBUG="1", CARGO_PROFILE_RELEASE_STRIP="false")
    env["RUSTFLAGS"] = env.get("RUSTFLAGS", "") + " -C force-frame-pointers=yes"
    with (out / "build.log").open("w") as log:
        subprocess.run(["cargo", "build", "--release", "-p", "logicaffeine-kernel",
                        "--example", "lean_arena"], cwd=ROOT, env=env, stdout=log,
                       stderr=subprocess.STDOUT, check=True)
    binary = ROOT / "target/arena-profile/release/examples/lean_arena"
    elevated = ["sudo", "-n"] if args.sudo_perf else []
    user = pwd.getpwuid(os.getuid()).pw_name
    drop = ["sudo", "-n", "-u", user] if args.sudo_perf else []
    command = elevated + [
        "perf", "record", "-o", str(out / "perf.data"), "-e", "cycles:u",
        "-F", "499", "--call-graph", "fp", "--",
    ] + drop + measured_command(
        [str(binary)] + (["--trace-stages"] if args.trace_stages else [])
        + (["--audit-declarations"] if args.audit_declarations else []) + [str(fixture)],
        out / "timing.json", args.seconds,
    )
    wait_for_test_window()
    with (out / "stdout.log").open("w") as stdout, (out / "stderr.log").open("w") as stderr:
        result = subprocess.run(command, cwd=ROOT, stdout=stdout, stderr=stderr)
    with (out / "perf.script").open("w") as stream:
        subprocess.run(elevated + ["perf", "script", "-i", str(out / "perf.data")],
                       stdout=stream, check=True)
    with (out / "perf.script").open() as stream:
        subprocess.run([sys.executable, str(ROOT / "scripts/dev/flamefold.py"),
                        "--folded", str(out / "stacks.folded"),
                        "--summary", str(out / "buckets.json"),
                        "--name", fixture.name, "--lang", "Rust"],
                       stdin=stream, check=True)
    after = sources()
    same = before == after and fixture_hash == digest(fixture)
    note = f"Checker exit {result.returncode}; limit {args.seconds}s. Exit 124 is a partial run. This profile is not an Arena score."
    hotspots = render(out / "stacks.folded", out / "flamegraph.svg",
                      "LOGOS Rust checker — " + fixture.name, note)
    report = dict(command=command, checker_exit=result.returncode, trace_stages=args.trace_stages,
                  audit_declarations=args.audit_declarations,
                  source_stable=same, source_sha256=before, input=str(fixture),
                  input_sha256=fixture_hash, binary_sha256=digest(binary),
                  rustc=subprocess.check_output(["rustc", "-Vv"], text=True),
                  perf=subprocess.check_output(["perf", "--version"], text=True).strip(),
                  rustflags=env["RUSTFLAGS"], hotspots=hotspots)
    timing = out / "timing.json"
    report["timing"] = json.loads(timing.read_text()) if timing.exists() and timing.stat().st_size else None
    (out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(out)
    if not same:
        raise SystemExit("Source changed during profiling; do not compare this run")
    if result.returncode not in (0, 1, 2, 124):
        raise SystemExit(f"Checker/profile failed with exit {result.returncode}")


if __name__ == "__main__":
    main()
