#!/usr/bin/env python3
"""Build Lean Kernel Arena test exports locally, byte-identical to the Arena's, without a git checkout of any source.

The Arena's `lka.py build-test` fetches each test's source with git, builds the module with Lake and exports it with
lean4export. The Arena's caches are not published, so the large inputs (Mathlib, cslib, cedar, con-leche, Navier-Stokes,
init, std) must be regenerated here to rehearse an entry on them. This driver reproduces `create_test` step for step:

  * a `file` test is the Arena's own file, copied; a `url` + `rev` source is the GitHub tarball of that commit, not
    a clone; a `dir` source is that directory of the Arena tree; a `leanfile` source is the Arena's standalone `Test`
    module (`Test.lean`, the Arena's `tests/lean-toolchain`, the trivial lakefile);
  * lean4export is the tarball of `LEAN4EXPORT_REV` with its `lean-toolchain` set to the test's, built with Lake;
  * the test's `pre-build`, then `lake build <module>`, then `lake env lean4export <module> [-- decls]`.

Lake fetches a project's own Lean dependencies with git; that happens only inside `--work`, which must lie outside the
repository. Every export is accepted only if its SHA-256 equals the one the Arena published in `results.json`; a
mismatch keeps the file beside its receipt as `<name>.mismatch.ndjson` and exits 1.

usage: build-arena-inputs.py --arena DIR --results FILE --work DIR [--out DIR] NAME...
"""
import argparse
import datetime
import hashlib
import json
import os
import shlex
import shutil
import subprocess
import sys
import tarfile
import time
import urllib.request
from pathlib import Path

import yaml

LEAN4EXPORT_REV = "66f1fb4bc256072069767fce52d39480e4524869"
ROOT = Path(__file__).resolve().parents[3]
TRIVIAL_LAKEFILE = 'name = "test"\n\n[[lean_lib]]\nname = "Test"'


def fail(message):
    sys.exit(f"build-arena-inputs: {message}")


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 24), b""):
            h.update(chunk)
    return h.hexdigest()


def github_tarball(url, rev, into):
    """Extract the GitHub tarball of `url` at `rev` as the directory `into`."""
    parts = [p for p in url.split("github.com", 1)[1].split("/") if p]
    owner, repo = parts[0], parts[1].removesuffix(".git")
    into.parent.mkdir(parents=True, exist_ok=True)
    archive = into.parent / f"{repo}-{rev}.tar.gz"
    if not archive.exists():
        partial = archive.with_suffix(".partial")
        print(f"  fetching {owner}/{repo}@{rev}", flush=True)
        urllib.request.urlretrieve(f"https://codeload.github.com/{owner}/{repo}/tar.gz/{rev}", partial)
        partial.rename(archive)
    staging = into.parent / f".{into.name}.extract"
    if staging.exists():
        shutil.rmtree(staging)
    staging.mkdir()
    with tarfile.open(archive) as t:
        t.extractall(staging, filter="tar")
    (top,) = list(staging.iterdir())
    if into.exists():
        shutil.rmtree(into)
    top.rename(into)
    staging.rmdir()


def run(cmd, cwd, log, env):
    with open(log, "ab") as stream:
        stream.write(f"\n$ {cmd}   (in {cwd})\n".encode())
        stream.flush()
        started = time.monotonic()
        code = subprocess.run(["nice", "-n", "15", "bash", "-c", cmd], cwd=cwd, env=env, stdout=stream,
                              stderr=subprocess.STDOUT).returncode
        stream.write(f"[exit {code} after {time.monotonic() - started:.1f}s]\n".encode())
    if code:
        raise RuntimeError(f"`{cmd}` failed with exit {code}; see {log}")


def lean4export(toolchain, work, log, env):
    built = work / "lean4export" / toolchain.replace("/", "_").replace(":", "_")
    binary = built / ".lake/build/bin/lean4export"
    if binary.exists():
        return binary
    github_tarball("https://github.com/leanprover/lean4export", LEAN4EXPORT_REV, built)
    (built / "lean-toolchain").write_text(toolchain + "\n")
    run("lake build", built, log, env)
    return binary


def build(name, test, arena, work, out, published, env):
    folder = out / name
    folder.mkdir(parents=True, exist_ok=True)
    log = folder / "build.log"
    src = work / "tests" / name / "src"
    src.parent.mkdir(parents=True, exist_ok=True)
    receipt = {"name": name, "started": datetime.datetime.now(datetime.timezone.utc).isoformat(),
               "lean4export_rev": LEAN4EXPORT_REV, "published_sha256": published.get("sha256"),
               "published_size": published.get("size"), "published_lean_version": published.get("lean_version")}
    began = time.monotonic()
    tmp = folder / f"{name.split('/')[-1]}.ndjson.tmp"
    if test.get("file"):
        shutil.copyfile(arena / test["file"], tmp)
        receipt.update(file=test["file"])
        return settle(name, folder, tmp, receipt, began)
    if test.get("url") and test.get("rev"):
        github_tarball(test["url"], test["rev"], src)
        receipt.update(url=test["url"], rev=test["rev"])
        module = test["module"]
    elif test.get("dir") and test.get("module"):
        if src.exists():
            shutil.rmtree(src)
        shutil.copytree(arena / test["dir"], src)
        receipt.update(dir=test["dir"])
        module = test["module"]
    elif test.get("leanfile"):
        if src.exists():
            shutil.rmtree(src)
        src.mkdir(parents=True)
        shutil.copy(arena / test["leanfile"], src / "Test.lean")
        shutil.copy(arena / "tests/lean-toolchain", src / "lean-toolchain")
        (src / "lakefile.toml").write_text(TRIVIAL_LAKEFILE)
        receipt.update(leanfile=test["leanfile"])
        module = "Test"
    else:
        fail(f"{name}: a test is built from file, url+rev, dir+module or leanfile")
    toolchain = (src / "lean-toolchain").read_text().strip()
    receipt["toolchain"] = toolchain
    exporter = lean4export(toolchain, work, log, env)
    receipt["lean4export_sha256"] = sha256(exporter)
    if test.get("pre-build"):
        run(test["pre-build"], src, log, env)
    run(f"lake build {module}", src, log, env)
    decls = test.get("export-decls") or []
    command = f"lake env {shlex.quote(str(exporter))} {module}"
    if decls:
        command += " -- " + " ".join(shlex.quote(str(d)) for d in decls)
    run(f"{command} > {shlex.quote(str(tmp))}", src, log, env)
    return settle(name, folder, tmp, receipt, began)


def settle(name, folder, tmp, receipt, began):
    """Keep the export as `<name>.ndjson` exactly when its bytes are the Arena's."""
    digest = sha256(tmp)
    receipt.update(sha256=digest, size=tmp.stat().st_size, seconds=round(time.monotonic() - began, 1),
                   identical=digest == receipt["published_sha256"])
    final = folder / f"{name.split('/')[-1]}.ndjson"
    if receipt["identical"]:
        tmp.rename(final)
    else:
        tmp.rename(folder / f"{name.split('/')[-1]}.mismatch.ndjson")
    (folder / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt), flush=True)
    return receipt["identical"]


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--arena", type=Path, required=True, help="an extracted lean-kernel-arena tree")
    p.add_argument("--results", type=Path, required=True, help="the Arena's published results.json")
    p.add_argument("--work", type=Path, required=True, help="build directory, outside the repository")
    p.add_argument("--out", type=Path, help="where verified exports go (default reference-kernels/arena-inputs-<rev>)")
    p.add_argument("names", nargs="+")
    a = p.parse_args()
    work = a.work.resolve()
    if work == ROOT or ROOT in work.parents:
        fail("--work must lie outside the repository: Lake runs git there")
    results = json.loads(a.results.read_text())
    published = {t["name"]: t for t in results["tests"]}
    revision = results["meta"]["git_revision"]
    out = (a.out or ROOT / f"reference-kernels/arena-inputs-{revision[:8]}").resolve()
    env = dict(os.environ, PATH=f"{Path.home() / '.elan/bin'}:{os.environ['PATH']}", GIT_TERMINAL_PROMPT="0")
    for var in ("RUSTFLAGS", "CARGO_TARGET_DIR", "LEAN_PATH"):
        env.pop(var, None)
    ok = True
    for name in a.names:
        declaration = a.arena / "tests" / f"{name}.yaml"
        if not declaration.exists():
            fail(f"no test declaration {declaration}")
        if name not in published:
            fail(f"{name} is not in the published results")
        test = yaml.safe_load(declaration.read_text())
        try:
            ok &= build(name, test, a.arena, work, out, published[name], env)
        except Exception as error:
            print(f"{name}: FAILED: {error}", flush=True)
            ok = False
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
