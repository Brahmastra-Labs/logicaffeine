#!/usr/bin/env python3
"""Where logos-kernel would place on the live Lean Kernel Arena, test by test.

Runs a checker binary on every Arena test we hold locally, counting user-space instructions
exactly as the Arena does (`perf stat -e instructions`, on a pinned P-core), and compares each
verdict and count with every checker in the live `results.json`.

usage: scoreboard.py <checker> [--live DIR] [--filter SUBSTR] [--skip-heavy] [--skip NAME,...]
                                [--cores 2,4,6,8] [--timeout-scale X] [--json OUT] [--args "..."]
                                [--commit]

--commit writes the committed ratchet (benchmarks/results/lean-kernel-arena/): scoreboard.json and
ceilings.json. A ceiling (per-test instructions, 1% above the measurement) belongs to the exact
input it was measured on (its SHA-256) and only ever falls on those bytes: a commit that would
raise one is refused, as is one with a wrong verdict. A test the Arena re-exported starts a fresh
ceiling, and the first-place floor is re-based when the live revision moves, since both then
compare against different inputs and a different field. Tests not measured are listed explicitly;
`arena_scoreboard_lock` pins which may be absent.

Heavy inputs the live archive omits (init, std, mathlib, …) are read from the paths recorded in
the vendored corpus manifest. A test we cannot find is reported MISSING, never skipped silently.
"""
import argparse, concurrent.futures, json, os, subprocess, sys, tempfile, time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
DEFAULT_LIVE = os.path.join(ROOT, "reference-kernels/lean-arena-live-87512b60")
CORPUS_MANIFEST = os.path.join(ROOT, "crates/logicaffeine_tests/tests/fixtures/arena_corpus/manifest.json")
DIR_OF = {"accept": "good", "reject": "bad", "either": "either"}
LEADERS = ["mathgraph", "sokonanoda", "nanoclo", "official"]
# Not checkers: `parse-only` accepts every well-formed input without checking it.
NOT_CHECKERS = {"parse-only"}
# Ends a root-owned process group through killpg(2), run as root. Never `kill -KILL -<pgid>`:
# procps reads a `-<digits>` operand as an option cluster and keeps its first digit, so a group
# id beginning with 1 becomes kill(-1, SIGKILL), every process on the machine. `pgid > 1`
# bounds the blast radius structurally: 1 would be "everything", 0 "my own group".
KILL_GROUP = "import os, sys\npgid = int(sys.argv[1])\nif pgid > 1:\n    os.killpg(pgid, 9)"


def locate(live, test, corpus_paths):
    p = os.path.join(live, "payloads", DIR_OF[test["outcome"]], test["name"] + ".ndjson")
    if os.path.exists(p):
        return p
    h = corpus_paths.get(test["name"])
    if h and os.path.exists(h):
        return h
    return None


def snapshot(checker):
    """Copy the checker to an immutable, content-addressed path, so a rebuild mid-run cannot mix
    binaries into one scoreboard."""
    import hashlib
    digest = hashlib.sha256(open(checker, "rb").read()).hexdigest()
    d = os.path.join(ROOT, "logs", "arena-crown", "bin")
    os.makedirs(d, exist_ok=True)
    dst = os.path.join(d, f"{os.path.basename(checker)}-{digest[:16]}")
    if not os.path.exists(dst):
        import shutil
        shutil.copy2(checker, dst + ".tmp")
        os.rename(dst + ".tmp", dst)
    return dst, digest


def measure(checker, extra, path, core, timeout):
    """(exit code or None on timeout, instructions, wall seconds)."""
    fd, out = tempfile.mkstemp(dir=os.path.join(ROOT, "logs", "arena-crown"), prefix="score.")
    os.close(fd)
    os.unlink(out)
    rss_out = out + ".time"
    # Exactly lka.py's measurement: `perf stat -e instructions -- time -f … -- <checker>`, counting
    # the checker and the GNU time wrapper (as the Arena does for every checker). perf itself is
    # started under taskset, so the pinning to a P-core is inherited without an extra counted exec.
    cmd = ["sudo", "-n", "taskset", "-c", str(core), "perf", "stat", "-x,", "-e", "cpu_core/instructions/u",
           "-o", out, "--", "/usr/bin/time", "-f", "%e %M", "-o", rss_out, "--", checker] + extra + [path]
    t0 = time.monotonic()
    p = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    try:
        code = p.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        code = None
        # The checker runs as root under perf: kill its whole process group, by group id.
        subprocess.run(["sudo", "-n", sys.executable, "-c", KILL_GROUP, str(p.pid)], capture_output=True)
        p.wait()
    wall = time.monotonic() - t0
    instr = None
    rss_kb = None
    try:
        text = subprocess.run(["sudo", "-n", "cat", rss_out], capture_output=True, text=True).stdout.split()
        rss_kb = int(text[-1]) if text else None
        subprocess.run(["sudo", "-n", "rm", "-f", rss_out])
    except Exception:
        pass
    try:
        text = subprocess.run(["sudo", "-n", "cat", out], capture_output=True, text=True).stdout
        for line in text.splitlines():
            if "instructions" in line and line.split(",")[0].strip().isdigit():
                instr = int(line.split(",")[0])
        subprocess.run(["sudo", "-n", "rm", "-f", out])
    except Exception:
        pass
    return code, instr, wall, rss_kb


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("checker")
    ap.add_argument("--live", default=DEFAULT_LIVE)
    ap.add_argument("--filter", default="")
    ap.add_argument("--skip-heavy", action="store_true")
    ap.add_argument("--cores", default="2,4,6,8")
    ap.add_argument("--timeout-scale", type=float, default=1.0)
    ap.add_argument("--json")
    ap.add_argument("--args", default="")
    ap.add_argument("--skip", default="", help="comma-separated test names not to run")
    ap.add_argument("--commit", action="store_true")
    a = ap.parse_args()

    live = json.load(open(os.path.join(a.live, "results.json")))
    tests = {t["name"]: t for t in live["tests"]}
    theirs = {}
    for x in live["results"]:
        theirs.setdefault(x["test"], {})[x["checker"]] = x
    corpus_dir = os.path.dirname(CORPUS_MANIFEST)
    corpus_paths = {}
    if os.path.exists(CORPUS_MANIFEST):
        for c in json.load(open(CORPUS_MANIFEST))["cases"]:
            if c.get("location") == "heavy":
                corpus_paths[c["name"]] = os.path.join(ROOT, c["path"])
            elif c.get("location") == "vendored":
                corpus_paths[c["name"]] = os.path.join(corpus_dir, c["path"])
    checker, checker_sha = snapshot(a.checker)
    print(f"checker {a.checker} -> {checker} (sha256 {checker_sha})")
    heavy = {n for n, t in tests.items() if t.get("size", 0) > 4 * 1024 * 1024}
    skip = {x for x in a.skip.split(",") if x}
    names = sorted(n for n in tests if a.filter in n and not (a.skip_heavy and n in heavy) and n not in skip)
    cores = [int(c) for c in a.cores.split(",")]
    extra = a.args.split()

    def one(i_name):
        i, name = i_name
        t = tests[name]
        path = locate(a.live, t, corpus_paths)
        if path is None:
            return name, {"status": "MISSING"}
        limit = (t.get("timeout") or 300) * a.timeout_scale
        code, instr, wall, rss_kb = measure(checker, extra, path, cores[i % len(cores)], limit)
        got = {0: "accept", 1: "reject", 2: "decline"}.get(code, "timeout" if code is None else f"error({code})")
        exp = t["outcome"]
        if got in ("accept", "reject"):
            status = "correct" if exp in ("either", got) else "WRONG"
        else:
            status = got
        return name, {"status": status, "got": got, "expected": exp, "instructions": instr, "wall": round(wall, 2),
                      "max_rss_kb": rss_kb, "input_sha256": t["sha256"],
                      "timeout": t.get("timeout"), "checker_sha256": checker_sha}

    heavy_first = sorted(names, key=lambda n: -(tests[n].get("size") or 0))
    ours = {}
    with concurrent.futures.ThreadPoolExecutor(len(cores)) as ex:
        for name, row in ex.map(one, enumerate(heavy_first)):
            ours[name] = row

    def g(x):
        return f"{x / 1e9:9.3f}" if x else "        -"

    print(f"{'test':42} {'ours':>9} {'best':>9} {'best by':12} {'mathgraph':>9} {'sokonanoda':>10} {'official':>9}  rank  status")
    ranks = []
    for name in names:
        row = ours[name]
        field = {c: x for c, x in theirs.get(name, {}).items()
                 if c not in NOT_CHECKERS and x.get("correctness") == "correct" and x.get("instructions")}
        best_c, best = min(((c, x["instructions"]) for c, x in field.items()), key=lambda kv: kv[1], default=(None, None))
        rank = None
        if row.get("status") == "correct" and row.get("instructions"):
            rank = 1 + sum(1 for x in field.values() if x["instructions"] < row["instructions"])
            ranks.append(rank)
        row["rank"] = rank
        row["best"] = {"checker": best_c, "instructions": best}
        lead = {c: (theirs.get(name, {}).get(c) or {}).get("instructions") for c in LEADERS}
        print(f"{name:42} {g(row.get('instructions'))} {g(best)} {str(best_c):12} {g(lead['mathgraph'])} {g(lead['sokonanoda']):>10} {g(lead['official'])}  "
              f"{str(rank or '-'):>4}  {row['status']}")
    tally = {}
    for r in ours.values():
        tally[r["status"]] = tally.get(r["status"], 0) + 1
    firsts = sum(1 for r in ranks if r == 1)
    print(f"\nTALLY {json.dumps(tally, sort_keys=True)}; ranked first on {firsts} of {len(ranks)} measured correct tests")
    if a.json:
        json.dump({"live_revision": live["meta"]["git_revision"], "checker_sha256": checker_sha, "args": extra,
                   "results": ours}, open(a.json, "w"), indent=1)
    if a.commit:
        return commit(live, tests, ours, checker_sha, extra, firsts)
    return 0 if tally.get("WRONG", 0) == 0 else 1


RATCHET = os.path.join(ROOT, "benchmarks", "results", "lean-kernel-arena")


def commit(live, tests, ours, checker_sha, extra, firsts):
    os.makedirs(RATCHET, exist_ok=True)
    bad = {n: r["status"] for n, r in ours.items() if r["status"] not in ("correct",)}
    if any(s == "WRONG" for s in bad.values()):
        print("refusing to commit: a wrong verdict", {n: s for n, s in bad.items() if s == "WRONG"})
        return 1
    cpath = os.path.join(RATCHET, "ceilings.json")
    revision = live["meta"]["git_revision"]
    old = json.load(open(cpath)) if os.path.exists(cpath) else {}
    if old.get("schema") != "logos-arena-ceilings/2":
        old = {}
    ceilings = {n: c for n, c in old.get("ceilings", {}).items() if n in tests and c["input_sha256"] == tests[n]["sha256"]}
    raised = []
    for n, r in ours.items():
        if r["status"] != "correct" or not r.get("instructions"):
            continue
        new = int(r["instructions"] * 1.01) + 1
        prev = ceilings.get(n, {}).get("instructions")
        if prev is not None and r["instructions"] > prev:
            raised.append(f"{n}: {r['instructions']:,} > ceiling {prev:,}")
            continue
        ceilings[n] = {"input_sha256": tests[n]["sha256"], "instructions": new if prev is None else min(prev, new)}
    if raised:
        print("refusing to commit: instruction regressions\n  " + "\n  ".join(raised))
        return 1
    # Merge into the committed board: tests not re-measured keep their committed results.
    bpath = os.path.join(RATCHET, "scoreboard.json")
    merged = {}
    if os.path.exists(bpath):
        prev = json.load(open(bpath))
        if prev.get("live_revision") == live["meta"]["git_revision"]:
            merged.update(prev.get("results", {}))
    merged.update({n: r for n, r in ours.items() if r["status"] == "correct"})
    for n in [n for n, r in ours.items() if r["status"] != "correct"]:
        merged.pop(n, None)
    firsts = sum(1 for r in merged.values() if r.get("rank") == 1)
    floor = max(old.get("first_place_floor", 0), firsts) if old.get("live_revision") == revision else firsts
    measured = sorted(merged)
    not_measured = sorted(n for n in tests if n not in merged)
    board = {
        "schema": "logos-arena-scoreboard/1",
        "live_revision": live["meta"]["git_revision"],
        "checker_sha256": checker_sha,
        "checker_args": extra,
        "measured_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "measurement": "perf stat -e cpu_core/instructions/u -- /usr/bin/time -- checker, pinned P-core (as lka.py)",
        "first_place": firsts,
        "not_measured": {n: (ours.get(n) or {}).get("status", "not run") for n in not_measured},
        "results": {n: merged[n] for n in measured},
    }
    json.dump(board, open(os.path.join(RATCHET, "scoreboard.json"), "w"), indent=1, sort_keys=True)
    json.dump({"schema": "logos-arena-ceilings/2", "live_revision": revision,
               "ceilings": dict(sorted(ceilings.items())), "first_place_floor": floor},
              open(cpath, "w"), indent=1, sort_keys=True)
    print(f"committed: {len(measured)} measured, first on {firsts} (floor {floor}), not measured: {not_measured}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
