#!/usr/bin/env python3
"""Build and install the pinned lean4export exporters `largo lean import` produces results with.

Every Lean release LOGOS imports from has one import profile (`interface::lean::profile` in the
kernel crate): a toolchain, a lean4export revision, the structural-key patch for the releases after
4.14.0, and the SHA-256 of every exporter source and, where the platform is pinned, of the built
executable. `lean-exporters.json` beside this script is the build recipe of each profile; the
`lean_exporter_registry_lock` test holds it to the kernel's registry, so the two can never drift.

    scripts/lean/install-lean-exporters.py install              # every profile
    scripts/lean/install-lean-exporters.py install 4.33.1 4.34.1
    scripts/lean/install-lean-exporters.py verify               # re-hash what is installed
    scripts/lean/install-lean-exporters.py list

An installed exporter is a directory `<registry>/<lean version>/`: the exporter's sources (the
upstream archive at the pinned revision, the structural patch applied, `lean-toolchain` naming the
profile's toolchain), its build (`.lake/build/bin/lean4export` for an executable profile,
`.lake/build/lib/` for the interpreted 4.14.0 profile) and `receipt.json`, which records what was
built from what, with which Lean. The registry is `--registry`, else `$LOGOS_LEAN_EXPORTERS`, else
`~/.logos/lean/exporters` — the places `largo lean import --project` looks.

Nothing is trusted by name. The archive must have its pinned digest, every source its pinned digest
after patching, the toolchain must report the profile's version and commit, and a built executable
must have the digest pinned for this platform (the builds are reproducible: the same toolchain and
sources give the same bytes). A failed step installs nothing; an earlier installation is replaced
only by a complete, verified one. This tool never runs git.
"""
from __future__ import annotations

import argparse
import datetime
import hashlib
import io
import json
import os
import pathlib
import platform
import shutil
import subprocess
import sys
import tarfile
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
RECIPES = HERE / "lean-exporters.json"
RECEIPT_SCHEMA = "logos-lean-exporter-receipt/1"


class Refusal(Exception):
    pass


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def load_recipes() -> dict:
    recipes = json.loads(RECIPES.read_text())
    if recipes.get("schema") != "logos-lean-exporter-recipes/1":
        raise Refusal(f"{RECIPES}: unknown recipe schema {recipes.get('schema')!r}")
    patch_path = HERE / recipes["structural_patch"]
    actual = sha256_file(patch_path)
    if actual != recipes["structural_patch_sha256"]:
        raise Refusal(f"{patch_path} has sha256 {actual}; the recipes pin {recipes['structural_patch_sha256']}")
    recipes["patch"] = json.loads(patch_path.read_text())
    return recipes


def registry_dir(flag: str | None) -> pathlib.Path:
    if flag:
        return pathlib.Path(flag).expanduser().resolve()
    env = os.environ.get("LOGOS_LEAN_EXPORTERS")
    if env:
        return pathlib.Path(env).expanduser().resolve()
    return pathlib.Path.home() / ".logos" / "lean" / "exporters"


def platform_triple() -> str:
    machine = platform.machine().lower()
    system = platform.system().lower()
    if system == "linux":
        return f"{machine}-unknown-linux-gnu"
    if system == "darwin":
        return f"{'aarch64' if machine == 'arm64' else machine}-apple-darwin"
    return f"{machine}-{system}"


def toolchain_dir(elan_home: pathlib.Path, toolchain: str) -> pathlib.Path:
    return elan_home / "toolchains" / toolchain.replace("/", "--").replace(":", "---")


def clean_env(toolchain: str, bin_dir: pathlib.Path) -> dict:
    env = {
        "PATH": f"{bin_dir}{os.pathsep}/usr/bin{os.pathsep}/bin",
        "HOME": os.environ.get("HOME", "/tmp"),
        "ELAN_TOOLCHAIN": toolchain,
        "LANG": "C.UTF-8",
    }
    for key in ("TMPDIR", "LEAN_NUM_THREADS"):
        if key in os.environ:
            env[key] = os.environ[key]
    return env


def check_toolchain(recipe: dict, version: str, elan_home: pathlib.Path) -> tuple[pathlib.Path, str]:
    directory = toolchain_dir(elan_home, recipe["toolchain"])
    lean = directory / "bin" / "lean"
    lake = directory / "bin" / "lake"
    if not lean.is_file() or not lake.is_file():
        raise Refusal(f"the toolchain {recipe['toolchain']} is not installed at {directory} (elan toolchain install {recipe['toolchain']})")
    ran = subprocess.run([str(lean), "--version"], capture_output=True, text=True,
                         env=clean_env(recipe["toolchain"], directory / "bin"), timeout=120)
    reported = ran.stdout.strip()
    if ran.returncode != 0 or f"version {version}," not in reported:
        raise Refusal(f"{lean} is not Lean {version}: {reported or ran.stderr.strip()}")
    commit = reported.split("commit ", 1)[1].split(",", 1)[0].strip() if "commit " in reported else ""
    if len(commit) < 12 or not recipe["lean_githash"].startswith(commit):
        raise Refusal(f"{lean} reports commit {commit!r}; Lean {version} is commit {recipe['lean_githash']}")
    return directory, reported


def fetch_archive(recipes: dict, recipe: dict, archives: pathlib.Path | None) -> bytes:
    revision = recipe["revision"]
    if archives is not None and (archives / f"{revision}.tar.gz").is_file():
        data = (archives / f"{revision}.tar.gz").read_bytes()
        origin = str(archives / f"{revision}.tar.gz")
    else:
        url = recipes["archive_url"].format(revision=revision)
        with urllib.request.urlopen(url, timeout=120) as response:
            data = response.read()
        origin = url
    actual = sha256(data)
    if actual != recipe["archive_sha256"]:
        raise Refusal(f"the lean4export archive {origin} has sha256 {actual}; the profile pins {recipe['archive_sha256']}")
    return data


def extract(data: bytes, destination: pathlib.Path) -> None:
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        members = archive.getmembers()
        roots = {pathlib.PurePosixPath(m.name).parts[0] for m in members if m.name}
        if len(roots) != 1:
            raise Refusal("the archive does not hold exactly one top-level directory")
        for member in members:
            parts = pathlib.PurePosixPath(member.name).parts
            if member.name.startswith("/") or ".." in parts:
                raise Refusal(f"the archive holds an unsafe path {member.name!r}")
            if not (member.isdir() or member.isfile()):
                raise Refusal(f"the archive holds {member.name!r}, which is neither a file nor a directory")
            relative = pathlib.Path(*parts[1:]) if len(parts) > 1 else None
            if relative is None:
                continue
            target = destination / relative
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            source = archive.extractfile(member)
            assert source is not None
            target.write_bytes(source.read())


def apply_structural_patch(patch: dict, checkout: pathlib.Path) -> None:
    path = checkout / patch["file"]
    text = path.read_text()
    for before, after in patch["replacements"]:
        count = text.count(before)
        if count != 1:
            raise Refusal(f"the structural patch expects {before!r} exactly once in {patch['file']}, found {count}")
        text = text.replace(before, after)
    path.write_text(text)


def check_sources(recipe: dict, checkout: pathlib.Path) -> dict:
    found = {}
    for name, pinned in sorted(recipe["sources"].items()):
        path = checkout / name
        if not path.is_file():
            raise Refusal(f"{path} is missing")
        actual = sha256_file(path)
        if actual != pinned:
            raise Refusal(f"{name} has sha256 {actual}; the profile pins {pinned}")
        found[name] = actual
    return found


def build(recipe: dict, version: str, recipes: dict, staging: pathlib.Path, elan_home: pathlib.Path,
          archives: pathlib.Path | None) -> dict:
    started = datetime.datetime.now(datetime.timezone.utc).isoformat()
    directory, reported = check_toolchain(recipe, version, elan_home)
    data = fetch_archive(recipes, recipe, archives)
    extract(data, staging)
    if recipe["structural"]:
        apply_structural_patch(recipes["patch"], staging)
    (staging / "lean-toolchain").write_text(recipe["toolchain"] + "\n")
    sources = check_sources(recipe, staging)
    target = "lean4export" if recipe["form"] == "executable" else "Export"
    command = [str(directory / "bin" / "lake"), "build", target]
    ran = subprocess.run(command, cwd=staging, capture_output=True, text=True,
                         env=clean_env(recipe["toolchain"], directory / "bin"), timeout=1800)
    (staging / "build.log").write_text(ran.stdout + ran.stderr)
    if ran.returncode != 0:
        raise Refusal(f"lake build {target} failed ({ran.returncode}):\n{ran.stdout}{ran.stderr}")
    triple = platform_triple()
    binary = None
    if recipe["form"] == "executable":
        executable = staging / ".lake" / "build" / "bin" / "lean4export"
        if not executable.is_file():
            raise Refusal(f"lake build produced no {executable}")
        digest = sha256_file(executable)
        pinned = recipe["binary"].get(triple)
        if pinned is not None and digest != pinned:
            raise Refusal(f"the built lean4export has sha256 {digest}; the profile pins {pinned} for {triple}")
        binary = {"path": ".lake/build/bin/lean4export", "sha256": digest,
                  "bytes": executable.stat().st_size, "platform": triple,
                  "pinned": pinned is not None}
        shutil.rmtree(staging / ".lake" / "build" / "ir", ignore_errors=True)
        shutil.rmtree(staging / ".lake" / "build" / "lib", ignore_errors=True)
    else:
        olean = staging / ".lake" / "build" / "lib" / "Export.olean"
        if not olean.is_file():
            raise Refusal(f"lake build produced no {olean}")
        shutil.rmtree(staging / ".lake" / "build" / "ir", ignore_errors=True)
    return {
        "schema": RECEIPT_SCHEMA,
        "lean_version": version,
        "toolchain": recipe["toolchain"],
        "lean_githash": recipe["lean_githash"],
        "lean_reported": reported,
        "exporter": {
            "repository": recipes["repository"],
            "revision": recipe["revision"],
            "upstream_tag": recipe["upstream_tag"],
            "archive_sha256": recipe["archive_sha256"],
            "structural_patch_sha256": recipes["structural_patch_sha256"] if recipe["structural"] else None,
            "form": recipe["form"],
        },
        "sources": sources,
        "binary": binary,
        "build": {"command": ["lake", "build", target], "started": started,
                  "finished": datetime.datetime.now(datetime.timezone.utc).isoformat()},
    }


def install(versions: list[str], registry: pathlib.Path, elan_home: pathlib.Path,
            archives: pathlib.Path | None) -> int:
    recipes = load_recipes()
    chosen = versions or sorted(recipes["exporters"])
    failures = 0
    registry.mkdir(parents=True, exist_ok=True)
    for version in chosen:
        recipe = recipes["exporters"].get(version)
        if recipe is None:
            print(f"{version}: no profile (registered: {', '.join(sorted(recipes['exporters']))})", file=sys.stderr)
            failures += 1
            continue
        staging = registry / f".staging-{version}-{os.getpid()}"
        shutil.rmtree(staging, ignore_errors=True)
        staging.mkdir(parents=True)
        try:
            receipt = build(recipe, version, recipes, staging, elan_home, archives)
            (staging / "receipt.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
            final = registry / version
            retired = registry / f".retired-{version}-{os.getpid()}"
            if final.exists():
                final.rename(retired)
            staging.rename(final)
            shutil.rmtree(retired, ignore_errors=True)
            binary = receipt["binary"]
            print(f"{version}: installed at {final}" + (f" (lean4export sha256 {binary['sha256']})" if binary else ""))
        except (Refusal, OSError, subprocess.SubprocessError) as error:
            shutil.rmtree(staging, ignore_errors=True)
            print(f"{version}: {error}", file=sys.stderr)
            failures += 1
    return 1 if failures else 0


def verify(versions: list[str], registry: pathlib.Path) -> int:
    recipes = load_recipes()
    chosen = versions or sorted(recipes["exporters"])
    failures = 0
    triple = platform_triple()
    for version in chosen:
        recipe = recipes["exporters"].get(version)
        entry = registry / version
        try:
            if recipe is None:
                raise Refusal("no profile")
            if not entry.is_dir():
                raise Refusal(f"not installed at {entry}")
            check_sources(recipe, entry)
            receipt = json.loads((entry / "receipt.json").read_text())
            if receipt.get("schema") != RECEIPT_SCHEMA or receipt.get("lean_version") != version \
                    or receipt.get("toolchain") != recipe["toolchain"] \
                    or receipt.get("exporter", {}).get("revision") != recipe["revision"]:
                raise Refusal("receipt.json does not describe this profile")
            if recipe["form"] == "executable":
                digest = sha256_file(entry / ".lake" / "build" / "bin" / "lean4export")
                pinned = recipe["binary"].get(triple)
                if digest != receipt["binary"]["sha256"] or (pinned is not None and digest != pinned):
                    raise Refusal(f"lean4export has sha256 {digest}; receipt {receipt['binary']['sha256']}, pin {pinned}")
            elif not (entry / ".lake" / "build" / "lib" / "Export.olean").is_file():
                raise Refusal("the interpreted exporter is not built")
            print(f"{version}: verified at {entry}")
        except (Refusal, OSError, ValueError, KeyError) as error:
            print(f"{version}: {error}", file=sys.stderr)
            failures += 1
    return 1 if failures else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--registry", help="the exporter registry (default $LOGOS_LEAN_EXPORTERS, else ~/.logos/lean/exporters)")
    sub = parser.add_subparsers(dest="command", required=True)
    install_parser = sub.add_parser("install", help="build and install exporters")
    install_parser.add_argument("versions", nargs="*")
    install_parser.add_argument("--elan-home", help="where elan keeps toolchains (default $ELAN_HOME, else ~/.elan)")
    install_parser.add_argument("--archives", help="a directory of <revision>.tar.gz archives to use instead of downloading")
    verify_parser = sub.add_parser("verify", help="re-hash installed exporters against their pins")
    verify_parser.add_argument("versions", nargs="*")
    sub.add_parser("list", help="the registered profiles")
    args = parser.parse_args()
    registry = registry_dir(args.registry)
    try:
        if args.command == "list":
            for version, recipe in sorted(load_recipes()["exporters"].items()):
                state = "installed" if (registry / version).is_dir() else "not installed"
                print(f"{version}\t{recipe['toolchain']}\tlean4export {recipe['revision'][:12]}\t{recipe['form']}\t{state}")
            return 0
        if args.command == "verify":
            return verify(args.versions, registry)
        elan_home = pathlib.Path(args.elan_home or os.environ.get("ELAN_HOME") or pathlib.Path.home() / ".elan")
        archives = pathlib.Path(args.archives).resolve() if args.archives else None
        return install(args.versions, registry, elan_home, archives)
    except Refusal as error:
        print(error, file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
