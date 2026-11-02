#!/usr/bin/env python3
"""Rehearse a Lean Kernel Arena submission with the Arena's own runner. Nothing is pushed or submitted.

Runs lean-kernel-arena's lka.py, at the pinned revision, on one of our entries the way the Arena's
pull-request CI does: the entry is validated against the Arena's schema, `build-checker` runs its
build recipe in the Arena's directory layout, and `run` checks every test pull-request CI checks,
each under its own timeout. The build uses the Arena's compiler (the rustc of the nixpkgs its
flake pins) and an empty cargo home, as a fresh CI runner has.

usage: rehearse-submission.py --work DIR [--entry logos-kernel] [--source DIR | --ref TAG --rev SHA]
                              [--heavy] [--tests GLOB] [--memory SIZE]

  --work DIR    where the Arena checkout, its builds and results go. It must lie outside every
                cargo workspace: cargo reads .cargo/config.toml from each parent directory, and a
                developer's local config (linker, rustflags) would leak into the build.
  --source DIR  build from a source tree (default: this checkout). Its published workspace (the root
                manifest and lockfile, and every workspace member's files PUBLIC_MANIFEST.toml
                classifies public, so held-back crates stay out) is copied into the Arena checkout
                and entered in the local-directory form (`dir:`), which lka.py builds exactly like
                a clone of the public repository.
  --ref, --rev  build from the public repository at that tag and commit: the YAML exactly as it
                would be submitted. lka.py clones it itself.
  --heavy       also run the skip-on-ci corpora (init, std, cedar, cslib, con-leche, navier-stokes-euler,
                mathlib), which the Arena runs only after merge; their exports must be present locally
                (build-arena-inputs.py rebuilds them, byte-identical to the Arena's).
  --tests GLOB  only the tests whose Arena name matches GLOB.
  --memory SIZE the memory the build and every test share, without swap (default 16G, the Arena
                runner's). A checker that outgrows it is killed as the runner would kill it, and
                cannot push the rest of this machine into the OOM killer.

Exits 0 exactly when every staged test is decided as the Arena expects (outcome `either` tests may
go either way) within its timeout.
"""
import argparse
import fnmatch
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tomllib
import urllib.request
from pathlib import Path

import yaml

ARENA_REVISION = "87512b60eb2483562967ebc93a5fb42eaa135389"
ARENA_TARBALL = f"https://codeload.github.com/leanprover/lean-kernel-arena/tar.gz/{ARENA_REVISION}"
# The rustc of nixpkgs 0ad6f47e (nixos-26.05), which the Arena's flake.lock pins.
ARENA_RUSTC = "1.95.0"
LKA_DEPENDENCIES = ["jinja2>=3.1.6", "jsonschema>=4.26.0", "markdown>=3.10", "pyyaml>=6.0.3"]
# The Arena's checker jobs run on nscloud-ubuntu-22.04-amd64-8x16: 8 vCPUs, 16 GB.
ARENA_MEMORY = "16G"

ROOT = Path(__file__).resolve().parents[3]
ENTRIES = ROOT / "crates/logicaffeine_kernel/arena"
CORPUS = ROOT / "crates/logicaffeine_tests/tests/fixtures/arena_corpus"
# Never part of a checkout: VCS metadata and package-manager installs (build output is never public).
UNCHECKED_OUT = {".git", "node_modules"}


def sh(cmd, **kw):
    print("+", " ".join(str(c) for c in cmd), flush=True)
    return subprocess.run([str(c) for c in cmd], check=True, **kw)


def fail(message):
    sys.exit(f"rehearsal: {message}")


def inside_a_cargo_workspace(path):
    return any((p / "Cargo.toml").exists() or (p / ".cargo").exists() for p in [path, *path.parents])


def fetch_arena(work):
    tarball = work / f"lean-kernel-arena-{ARENA_REVISION}.tar.gz"
    if not tarball.exists():
        partial = tarball.with_suffix(".partial")
        print(f"fetching lean-kernel-arena@{ARENA_REVISION}", flush=True)
        urllib.request.urlretrieve(ARENA_TARBALL, partial)
        partial.rename(tarball)
    arena = work / "arena"
    if arena.exists():
        shutil.rmtree(arena)
    with tarfile.open(tarball) as t:
        t.extractall(work, filter="data")
    (work / f"lean-kernel-arena-{ARENA_REVISION}").rename(arena)
    ours = (ENTRIES / "checker.schema.json").read_bytes()
    theirs = (arena / "schemas/checker.json").read_bytes()
    if ours != theirs:
        fail("the Arena's checker schema differs from arena/checker.schema.json; re-vendor it and re-read "
             "crates/logicaffeine_tests/tests/arena_entry_lock.rs before trusting the entries")
    return arena


def lka_python(work):
    venv = work / "venv"
    python = venv / "bin/python"
    if not python.exists():
        sh([sys.executable, "-m", "venv", venv])
        sh([venv / "bin/pip", "install", "--quiet", *LKA_DEPENDENCIES])
    return python


def perf_counts_instructions():
    probe = subprocess.run(["perf", "stat", "-e", "instructions", "--", "true"], capture_output=True)
    return probe.returncode == 0


def arena_environment(work):
    """The environment lka.py runs in: the Arena's rustc and llvm-profdata, an empty cargo home,
    and no cargo or rustc settings inherited from this shell."""
    try:
        sysroot = subprocess.run(["rustc", f"+{ARENA_RUSTC}", "--print", "sysroot"],
                                 capture_output=True, text=True, check=True).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        fail(f"the Arena builds with rustc {ARENA_RUSTC}: "
             f"rustup toolchain install {ARENA_RUSTC} --profile minimal --component llvm-tools")
    profdata = Path(sysroot) / "lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-profdata"
    if not profdata.exists():
        fail(f"llvm-profdata of rustc {ARENA_RUSTC}: rustup component add llvm-tools --toolchain {ARENA_RUSTC}")
    shims = work / "shims"
    if shims.exists():
        shutil.rmtree(shims)
    shims.mkdir()
    (shims / "llvm-profdata").symlink_to(profdata)
    # lka.py hashes each checker's and test's sources for result reuse by asking `git ls-files` which are tracked. The
    # Arena tree here is the pinned revision's tarball, every file of which is tracked, so a stand-in answers with each
    # named file and every file under each named directory (build output aside); anything else goes to git itself.
    real_git = shutil.which("git") or "/usr/bin/git"
    git = shims / "git"
    git.write_text(f"""#!{sys.executable}
import os, sys
args = sys.argv[1:]
if not args or args[0] != "ls-files":
    os.execv({real_git!r}, [{real_git!r}, *args])
paths = args[args.index("--") + 1:] if "--" in args else []
found = []
for p in paths:
    if os.path.isfile(p):
        found.append(os.path.normpath(p))
    elif os.path.isdir(p):
        for d, dirs, files in os.walk(p):
            dirs[:] = sorted(x for x in dirs if x not in (".git", ".lake", "_build", "target", "pgo"))
            found.extend(os.path.normpath(os.path.join(d, f)) for f in sorted(files))
end = "\\0" if "-z" in args else "\\n"
sys.stdout.write("".join(f + end for f in found))
""")
    git.chmod(0o755)
    if not perf_counts_instructions():
        # lka.py wraps each run in `perf stat ... -- time ... -- sh -c <run>`. Where this user may
        # not count events, a stand-in answers `--version` and runs what follows `--`, so lka.py
        # keeps its measuring path: GNU time still reports wall clock and peak RSS.
        perf = shims / "perf"
        perf.write_text('#!/bin/sh\n'
                        '[ "$1" = "--version" ] && { echo "perf stand-in (no counters)"; exit 0; }\n'
                        'while [ $# -gt 0 ] && [ "$1" != "--" ]; do shift; done\n'
                        'shift\n'
                        'exec "$@"\n')
        perf.chmod(0o755)
        print("perf cannot count instructions here: wall clock and peak RSS only (instr.sh counts instructions)")
    inherited = ("CARGO_", "RUSTFLAGS", "RUSTC", "RUSTDOC", "RUSTUP_TOOLCHAIN")
    env = {k: v for k, v in os.environ.items() if not k.startswith(inherited)}
    env["RUSTUP_TOOLCHAIN"] = ARENA_RUSTC
    env["CARGO_HOME"] = str(work / "cargo-home")
    env["PATH"] = f"{shims}:{os.environ['PATH']}"
    return env


def runner_memory(size):
    """The command prefix that confines what follows to `size` of memory and no swap, in a
    transient systemd scope: the memory controller kills the largest process inside it, which is
    the checker, exactly where the Arena runner would run out."""
    probe = subprocess.run(["systemd-run", "--user", "--scope", "--quiet", "-p", "MemoryMax=64M",
                            "-p", "MemorySwapMax=0", "--", "python3", "-c", "bytearray(256 << 20)"],
                           capture_output=True)
    if probe.returncode == 0:
        fail("systemd-run --user cannot enforce a memory limit here (is the memory controller delegated?)")
    return ["systemd-run", "--user", "--scope", "--quiet", "-p", f"MemoryMax={size}", "-p", "MemorySwapMax=0", "--"]


def published_files(source):
    """The files of `source`'s published workspace, relative to it: the root manifest and lockfile and every
    workspace member's files. The repository is the public tree, so in a git checkout that is what its next commit
    would hold under them (`git add -A`): every file git tracks or would track, less those deleted from the working
    tree; a tree without git contributes every file but checkout debris. An older tree that still carries
    PUBLIC_MANIFEST.toml keeps its held-back crates out through that manifest's classifier."""
    members = tomllib.loads((source / "Cargo.toml").read_text())["workspace"]["members"]
    roots = sorted({m[: -len("/*")] if m.endswith("/*") else m for m in members})
    if not (source / "PUBLIC_MANIFEST.toml").is_file():
        if (source / ".git").exists():
            listed = subprocess.run(["git", "-C", str(source), "ls-files", "-z", "--cached", "--others", "--exclude-standard",
                                     "--", "Cargo.toml", "Cargo.lock", *roots],
                                    capture_output=True, check=True).stdout.decode()
            return sorted({f for f in listed.split("\0") if f and (source / f).is_file()})
        files = ["Cargo.toml", "Cargo.lock"]
        for root in roots:
            for dirpath, dirnames, filenames in os.walk(source / root):
                rel_dir = Path(dirpath).relative_to(source).as_posix()
                dirnames[:] = sorted(d for d in dirnames if d not in UNCHECKED_OUT and d != "target")
                files.extend(f"{rel_dir}/{f}" for f in sorted(filenames))
        return files
    spec = importlib.util.spec_from_file_location("manifest", source / "scripts/manifest.py")
    manifest = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(manifest)
    rules = manifest.load_rules(str(source / "PUBLIC_MANIFEST.toml"))

    def public(rel):
        return manifest.classify(rel, rules)[0] == "public"

    files = [f for f in ("Cargo.toml", "Cargo.lock") if public(f)]
    for root in roots:
        for dirpath, dirnames, filenames in os.walk(source / root):
            rel_dir = Path(dirpath).relative_to(source).as_posix()
            dirnames[:] = sorted(d for d in dirnames if d not in UNCHECKED_OUT
                                 and manifest.classify(f"{rel_dir}/{d}/", rules)[0] != "confidential")
            files.extend(f"{rel_dir}/{f}" for f in sorted(filenames) if public(f"{rel_dir}/{f}"))
    return files


def entry_text(entry):
    return (ENTRIES / f"{entry}.yaml").read_text()


def stage_from_source(arena, entry, source):
    # Beside checkers/, not in it: lka.py reads every *.yaml under checkers/ recursively as a
    # checker, and the source tree carries its own entry files.
    snapshot = arena / f"{entry}-src"
    files = published_files(source)
    for rel in files:
        (snapshot / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source / rel, snapshot / rel)
    print(f"staged the published workspace of {source}: {len(files)} files", flush=True)
    lines = []
    for line in entry_text(entry).splitlines():
        if line.startswith("url: "):
            lines.append(f"dir: ../{entry}-src")
        elif not line.startswith(("ref: ", "rev: ")):
            lines.append(line)
    (arena / "checkers" / f"{entry}.yaml").write_text("\n".join(lines) + "\n")


def stage_from_revision(arena, entry, ref, rev):
    text = entry_text(entry)
    if "ref: SUBMISSION_REF\n" not in text or "rev: SUBMISSION_REV\n" not in text:
        fail(f"{entry}.yaml is already pinned; rehearse the pin by passing it as --ref/--rev")
    text = text.replace("ref: SUBMISSION_REF\n", f"ref: {ref}\n").replace("rev: SUBMISSION_REV\n", f"rev: {rev}\n")
    (arena / "checkers" / f"{entry}.yaml").write_text(text)


def arena_tests(arena):
    """The Arena's test definitions, by the name lka.py gives each."""
    tests = {}
    for f in sorted((arena / "tests").rglob("*.yaml")):
        tests[str(f.relative_to(arena / "tests").with_suffix(""))] = yaml.safe_load(f.read_text())
    return tests


def definition_of(tests, name):
    if name in tests:
        return tests[name]
    for prefix, d in tests.items():
        if d.get("multiple") and name.startswith(prefix + "/"):
            return d
    fail(f"{name}: no Arena test definition produces it; re-vendor the corpus (vendor-corpus.py)")


def stage_tests(arena, heavy, pattern):
    """Stage the built-test layout lka.py reads (`_build/tests/<name>.ndjson` + `.stats.json`) from
    the vendored corpus, so no Lean toolchain or export is rebuilt."""
    tests = arena_tests(arena)
    cases = json.loads((CORPUS / "manifest.json").read_text())["cases"]
    names = {c["name"] for c in cases}
    missing = sorted(n for n, d in tests.items() if not d.get("multiple") and n not in names)
    if missing:
        fail(f"the Arena has tests the vendored corpus lacks: {missing}; re-vendor the corpus")
    out = arena / "_build" / "tests"
    staged, skipped = [], []
    for c in cases:
        name = c["name"]
        d = definition_of(tests, name)
        if pattern and not fnmatch.fnmatch(name, pattern):
            continue
        if d.get("skip-on-ci") and not heavy:
            skipped.append(name)
            continue
        if not d.get("multiple") and d.get("outcome") != c["outcome"]:
            fail(f"{name}: the Arena expects {d.get('outcome')}, the corpus {c['outcome']}; re-vendor the corpus")
        source = CORPUS / c["path"] if c["location"] == "vendored" else ROOT / c["path"]
        if not source.is_file():
            fail(f"{name}: the export is not present at {source}")
        ndjson = out / f"{name}.ndjson"
        ndjson.parent.mkdir(parents=True, exist_ok=True)
        ndjson.symlink_to(source.resolve())
        stats = {"name": name, "outcome": c["outcome"]}
        for key in ("timeout", "compare-perf"):
            if d.get(key):
                stats[key] = d[key]
        (out / f"{name}.stats.json").write_text(json.dumps(stats, indent=2))
        staged.append(name)
    return staged, skipped


def declared_declines(entry):
    declines = yaml.safe_load(entry_text(entry)).get("declines", [])
    return {declines} if isinstance(declines, str) else set(declines)


def summarize(arena, entry, staged, skipped, work):
    results = [json.loads(p.read_text()) for p in sorted((arena / "_results").glob(f"{entry}_*.json"))]
    declared = declared_declines(entry)
    judged = [r for r in results if not (r["test"] in declared and r["correctness"] == "declined")]
    tally = {}
    for r in results:
        tally[r["correctness"]] = tally.get(r["correctness"], 0) + 1
    print(f"\n{entry} at lean-kernel-arena@{ARENA_REVISION[:8]}: {len(judged)} of {len(staged)} staged tests run, "
          f"{len(skipped)} skip-on-ci not staged, declared declines: {sorted(declared) or 'none'}")
    print("  " + ", ".join(f"{k}: {v}" for k, v in sorted(tally.items())))
    wrong = [r for r in judged if r["correctness"] not in ("correct", "either")]
    for r in wrong:
        tail = (r.get("stderr") or "").strip().splitlines()[-1:] or [""]
        print(f"  {r['correctness']:9} {r['test']:50} {r['status']:9} exit {r['exit_code']:>4} "
              f"{r['wall_time']:8.2f}s  {tail[0][:140]}")
    timed = sorted((r for r in results if r.get("timeout")), key=lambda r: -r["wall_time"] / r["timeout"])
    heaviest = max(results, key=lambda r: r.get("max_rss") or 0, default=None)
    if heaviest and heaviest.get("max_rss"):
        print(f"  peak RSS: {heaviest['max_rss'] / 2**30:.2f} GiB on {heaviest['test']} (the runner has 16 GB)")
    print("  closest to their timeout (wall / limit):")
    for r in timed[:10]:
        print(f"    {r['test']:50} {r['wall_time']:8.1f}s / {r['timeout']:5d}s  {100 * r['wall_time'] / r['timeout']:5.1f}%")
    summary = {"entry": entry, "arena_revision": ARENA_REVISION, "rustc": ARENA_RUSTC, "staged": len(staged),
               "skipped_skip_on_ci": skipped, "tally": tally,
               "results": [{k: r.get(k) for k in ("test", "status", "correctness", "exit_code", "wall_time",
                                                  "timeout", "max_rss", "instructions")} for r in results]}
    (work / "summary.json").write_text(json.dumps(summary, indent=2))
    print(f"  summary: {work / 'summary.json'}")
    return not wrong and {r["test"] for r in judged} == set(staged) - declared


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--work", required=True, type=Path)
    ap.add_argument("--entry", default="logos-kernel", choices=["logos-kernel", "logos-dual"])
    ap.add_argument("--source", type=Path, default=ROOT)
    ap.add_argument("--ref")
    ap.add_argument("--rev")
    ap.add_argument("--heavy", action="store_true")
    ap.add_argument("--tests")
    ap.add_argument("--memory", default=ARENA_MEMORY)
    a = ap.parse_args()
    work = a.work.resolve()
    if inside_a_cargo_workspace(work):
        fail(f"{work} lies inside a cargo workspace or under a .cargo/ config; choose a --work outside it")
    if (a.ref is None) != (a.rev is None):
        fail("--ref and --rev pin together")
    work.mkdir(parents=True, exist_ok=True)
    arena = fetch_arena(work)
    python = lka_python(work)
    env = arena_environment(work)
    if a.rev:
        stage_from_revision(arena, a.entry, a.ref, a.rev)
    else:
        stage_from_source(arena, a.entry, a.source.resolve())
    staged, skipped = stage_tests(arena, a.heavy, a.tests)
    runner = runner_memory(a.memory)
    sh([*runner, python, "lka.py", "build-checker", a.entry], cwd=arena, env=env)
    sh([*runner, python, "lka.py", "run", "--checker", a.entry], cwd=arena, env=env)
    sys.exit(0 if summarize(arena, a.entry, staged, skipped, work) else 1)


if __name__ == "__main__":
    main()
