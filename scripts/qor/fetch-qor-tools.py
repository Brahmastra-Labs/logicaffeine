#!/usr/bin/env python3
"""Install the comparative-synthesis bench's third-party tools and designs into tools/qor/.

    fetch-qor-tools.py [--pin] [tool...]

Reads every pin from scripts/qor/qor-tools.toml. Each download lands in tools/qor/downloads/ and must hash
to its pinned sha256; a mismatch stops the install before anything is unpacked. `--pin` records the sha256 of
a download whose pin is still empty (the first fetch of a new version) and refuses to overwrite a set pin.
Sources arrive as release archives, never as repository clones. With no tool named, every tool installs.

    bambu    the PandA-Bambu AppImage, extracted to tools/qor/bambu-<version>/ (the box has no FUSE 2), and the
             logos-qor-bambu image it runs in (scripts/qor/bambu-image.Dockerfile on the pinned base)
    calyx    the pinned Calyx image (Calyx, Dahlia, fud) pulled by digest
    sv2v     tools/qor/sv2v-<version>/sv2v
    gemmini  the Gemmini sources at tools/qor/gemmini/, and the berkeley-hardfloat they import at
             tools/qor/gemmini-hardfloat/
    chisel   the logos-qor-chisel image (scripts/qor/chisel-image.Dockerfile): the pinned JDK image and sbt archive
    vortex   the Vortex sources with every submodule at tools/qor/vortex/
    litex    a venv at tools/qor/litex-venv/ holding migen, LiteX and LiteDRAM at their pins
    spatial  the Spatial sources at tools/qor/spatial/ (the commit the Plasticine toolchain runs), and the Plasticine
             toolchain's image they build in (scripts/plasticine/Dockerfile)
    openlane-stock
             OpenLane as released at tools/qor/openlane-stock/, at the commit the repo's overlay is written over —
             stamped with it, the one tree scripts/openlane-run.sh runs without the overlay (OL_STOCK=1)
    openlane-bench
             the same release at tools/qor/openlane-bench/, stamped the same: the tree every other run of the bench
             flows in, the overlay laid over it (scripts/openlane-run.sh OL_BENCH=1), so no bench design sits in
             tools/OpenLane
"""
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tomllib
import urllib.request
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
PINS = os.path.join(HERE, "qor-tools.toml")
TOOLS = os.path.join(ROOT, "tools", "qor")
DOWNLOADS = os.path.join(TOOLS, "downloads")


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def write_pin(table, key, value):
    text = open(PINS).read()
    pattern = re.compile(rf'(\[{re.escape(table)}\][^\[]*?^{re.escape(key)} = )""', re.M | re.S)
    new, n = pattern.subn(rf'\g<1>"{value}"', text, count=1)
    if n != 1:
        sys.exit(f"{table}.{key}: no empty pin to record (a set pin is never overwritten)")
    open(PINS, "w").write(new)


def fetch(url, name, table, key, pin, record):
    os.makedirs(DOWNLOADS, exist_ok=True)
    dest = os.path.join(DOWNLOADS, name)
    if not os.path.exists(dest):
        part = dest + ".part"
        print(f"fetch {url}", flush=True)
        with urllib.request.urlopen(url, timeout=600) as r, open(part, "wb") as f:
            shutil.copyfileobj(r, f, 1 << 20)
        os.replace(part, dest)
    got = sha256(dest)
    if pin:
        if got != pin:
            sys.exit(f"{name}: sha256 {got} is not the pinned {pin}")
    elif record:
        write_pin(table, key, got)
        print(f"pinned {table}.{key} = {got}")
    else:
        sys.exit(f"{name}: no pin for {table}.{key}; rerun with --pin to record {got}")
    return dest


def untar(archive, dest):
    if os.path.isdir(dest):
        shutil.rmtree(dest)
    os.makedirs(dest)
    with tarfile.open(archive) as t:
        members = t.getmembers()
        top = members[0].name.split("/")[0]
        for m in members:
            parts = m.name.split("/", 1)
            if parts[0] != top or len(parts) == 1:
                continue
            m.name = parts[1]
            t.extract(m, dest, filter="data")


def bambu(p, record):
    t = p["bambu"]
    img = fetch(t["url"], f"bambu-{t['version']}.AppImage", "bambu", "sha256", t["sha256"], record)
    build_image(os.path.join(HERE, "bambu-image.Dockerfile"), {"BASE_IMAGE": t["base_image"]}, t["image"])
    dest = os.path.join(TOOLS, f"bambu-{t['version']}")
    if os.path.isdir(dest):
        return
    os.chmod(img, 0o755)
    work = os.path.join(TOOLS, ".bambu-extract")
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    subprocess.run([img, "--appimage-extract"], cwd=work, check=True, stdout=subprocess.DEVNULL)
    os.replace(os.path.join(work, "squashfs-root"), dest)
    shutil.rmtree(work)


def calyx(p, record):
    subprocess.run(["docker", "pull", p["calyx"]["image"]], check=True)


def sv2v(p, record):
    t = p["sv2v"]
    z = fetch(t["url"], f"sv2v-{t['version']}.zip", "sv2v", "sha256", t["sha256"], record)
    dest = os.path.join(TOOLS, f"sv2v-{t['version']}")
    os.makedirs(dest, exist_ok=True)
    with zipfile.ZipFile(z) as zf:
        member = next(n for n in zf.namelist() if n.endswith("/sv2v") or n == "sv2v")
        with zf.open(member) as src, open(os.path.join(dest, "sv2v"), "wb") as out:
            shutil.copyfileobj(src, out)
    os.chmod(os.path.join(dest, "sv2v"), 0o755)


def gemmini(p, record):
    t = p["gemmini"]
    a = fetch(t["url"], "gemmini.tar.gz", "gemmini", "sha256", t["sha256"], record)
    untar(a, os.path.join(TOOLS, "gemmini"))
    h = fetch(t["hardfloat_url"], "gemmini-hardfloat.tar.gz", "gemmini", "hardfloat_sha256", t["hardfloat_sha256"], record)
    untar(h, os.path.join(TOOLS, "gemmini-hardfloat"))


def build_image(dockerfile, args, tag, files=()):
    """Build image `tag` from `dockerfile` with build `args`, its context holding only `files`."""
    ctx = os.path.join(TOOLS, ".image-context")
    shutil.rmtree(ctx, ignore_errors=True)
    os.makedirs(ctx)
    for f in files:
        shutil.copy(f, ctx)
    shutil.copy(dockerfile, os.path.join(ctx, "Dockerfile"))
    cmd = ["docker", "build", "-t", tag]
    for k, v in args.items():
        cmd += ["--build-arg", f"{k}={v}"]
    subprocess.run(cmd + [ctx], check=True)
    shutil.rmtree(ctx)


def chisel(p, record):
    t = p["chisel"]
    a = fetch(t["sbt_url"], "sbt.tgz", "chisel", "sbt_sha256", t["sbt_sha256"], record)
    build_image(os.path.join(HERE, "chisel-image.Dockerfile"), {"JDK_IMAGE": t["jdk_image"]}, t["image"], files=[a])


def vortex(p, record):
    t = p["vortex"]
    a = fetch(t["url"], "vortex.tar.gz", "vortex", "sha256", t["sha256"], record)
    dest = os.path.join(TOOLS, "vortex")
    untar(a, dest)
    pins = t["submodule_sha256"]
    for path, url in t["submodules"].items():
        name = "vortex-" + path.replace("/", "-") + ".tar.gz"
        sub = fetch(url, name, "vortex.submodule_sha256", f'"{path}"', pins[path], record)
        untar(sub, os.path.join(dest, path))


def litex(p, record):
    t = p["litex"]
    ma = fetch(t["migen_url"], "migen.tar.gz", "litex", "migen_sha256", t["migen_sha256"], record)
    la = fetch(t["litex_url"], "litex.tar.gz", "litex", "litex_sha256", t["litex_sha256"], record)
    da = fetch(t["litedram_url"], "litedram.tar.gz", "litex", "litedram_sha256", t["litedram_sha256"], record)
    venv = os.path.join(TOOLS, "litex-venv")
    if not os.path.isdir(venv):
        subprocess.run([sys.executable, "-m", "venv", venv], check=True)
    pip = os.path.join(venv, "bin", "pip")
    subprocess.run([pip, "install", "--quiet", "--force-reinstall", "--no-deps", ma], check=True)
    subprocess.run([pip, "install", "--quiet", la, da], check=True)


def spatial(p, record):
    t = p["spatial"]
    a = fetch(t["url"], "spatial.tar.gz", "spatial", "sha256", t["sha256"], record)
    untar(a, os.path.join(TOOLS, "spatial"))
    subprocess.run(["docker", "build", "-q", "-t", t["image"], os.path.join(ROOT, "scripts", "plasticine")], check=True)


def openlane_release(p, record, tree):
    """OpenLane as released at the overlay's base, unpacked to tools/qor/<tree>/ and stamped with that commit in
    .logos-base, which scripts/openlane-run.sh and scripts/openlane-overlay/apply.sh read a release's commit from."""
    t = p["openlane_release"]
    base = open(os.path.join(ROOT, "scripts", "openlane-overlay", "BASE")).read().strip()
    if t["commit"] != base:
        sys.exit(f"openlane_release pins {t['commit']}, but the overlay is written over {base}: the bench's trees are "
                 "the overlay's own base")
    a = fetch(t["url"], f"openlane-{base}.tar.gz", "openlane_release", "sha256", t["sha256"], record)
    dest = os.path.join(TOOLS, tree)
    stamp = os.path.join(dest, ".logos-base")
    # Its designs/ holds the runs the bench's ledgers re-derive from: a tree already at the base stays.
    if os.path.exists(stamp) and open(stamp).read().strip() == base:
        print(f"  {dest} is already OpenLane at {base}")
        return
    untar(a, dest)
    open(stamp, "w").write(base + "\n")


STEPS = {"bambu": bambu, "calyx": calyx, "sv2v": sv2v, "gemmini": gemmini, "chisel": chisel, "vortex": vortex,
         "litex": litex, "spatial": spatial,
         "openlane-stock": lambda p, record: openlane_release(p, record, "openlane-stock"),
         "openlane-bench": lambda p, record: openlane_release(p, record, "openlane-bench")}


def main():
    args = sys.argv[1:]
    record = "--pin" in args
    names = [a for a in args if a != "--pin"] or list(STEPS)
    unknown = [n for n in names if n not in STEPS]
    if unknown:
        sys.exit(f"unknown tool(s) {unknown}; known: {sorted(STEPS)}")
    os.makedirs(TOOLS, exist_ok=True)
    for n in names:
        print(f"== {n}", flush=True)
        STEPS[n](tomllib.load(open(PINS, "rb")), record)
    print("installed:", ", ".join(names))


if __name__ == "__main__":
    main()
