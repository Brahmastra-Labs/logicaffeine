#!/usr/bin/env python3
"""Every published crate that changed since its last release says so in its CHANGELOG.

For each workspace member that publishes (no `publish = false`), the previous release is the newest version
crates.io holds (the sparse index) and its content is that version's `.crate` — before a bump that is the version
the tree is developing past, after a partial publish it is the tree itself (no difference, nothing to say). When the crate's shipped code —
`src/`, `build.rs`, `assets/`, `stencils/`, `templates/`, or the manifest's `[features]` — differs from that
release, the crate's pending section must carry at least one bullet that is not the roll's
"Synced to workspace version" line. The pending section is `## [<workspace version>]` once rolled, else
`## [Unreleased]`. Run by publish.yml before anything uploads, and when the CHANGELOGs are rolled.

    check-changelogs.py            # against crates.io
    CHECK_CHANGELOGS_BASELINE=DIR  # offline: DIR/<package>/ holds each previous release, extracted
    LOGOS_ROOT=DIR                 # the workspace to check (default: this checkout)

Exit 0 when every changed crate says what changed; 1 naming each one that does not.
"""
import io
import json
import os
import sys
import tarfile
import tempfile
import tomllib
import urllib.request

ROOT = os.environ.get("LOGOS_ROOT") or os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BASELINE = os.environ.get("CHECK_CHANGELOGS_BASELINE")
SHIPPED_DIRS = ("src", "assets", "stencils", "templates")
SYNCED = "Synced to workspace version"


def workspace_version():
    with open(os.path.join(ROOT, "Cargo.toml"), "rb") as f:
        return tomllib.load(f)["workspace"]["package"]["version"]


def published_members():
    """(package name, crate directory) for every crate and app that publishes."""
    out = []
    for parent in ("crates", "apps"):
        base = os.path.join(ROOT, parent)
        for d in sorted(os.listdir(base)):
            manifest = os.path.join(base, d, "Cargo.toml")
            if not os.path.isfile(manifest):
                continue
            with open(manifest, "rb") as f:
                package = tomllib.load(f).get("package", {})
            if package.get("publish", True) is not False:
                out.append((package["name"], os.path.join(base, d)))
    return out


def index_path(name):
    n = name.lower()
    if len(n) <= 2:
        return f"{len(n)}/{n}"
    if len(n) == 3:
        return f"3/{n[0]}/{n}"
    return f"{n[:2]}/{n[2:4]}/{n}"


def released_version(name):
    """The newest non-yanked version crates.io holds, or None for a crate never published."""
    try:
        with urllib.request.urlopen(f"https://index.crates.io/{index_path(name)}", timeout=30) as r:
            lines = r.read().decode().splitlines()
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None
        raise
    versions = [json.loads(l) for l in lines if l.strip()]
    live = [v["vers"] for v in versions if not v.get("yanked")]
    return live[-1] if live else None


def fetch_release(name, version, into):
    url = f"https://static.crates.io/crates/{name}/{name}-{version}.crate"
    with urllib.request.urlopen(url, timeout=60) as r:
        data = r.read()
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tar:
        tar.extractall(into, filter="data")
    return os.path.join(into, f"{name}-{version}")


def shipped_files(crate_dir):
    out = {}
    for d in SHIPPED_DIRS:
        top = os.path.join(crate_dir, d)
        for dirpath, dirnames, files in os.walk(top):
            dirnames[:] = [x for x in dirnames if x != "target"]
            for fn in files:
                path = os.path.join(dirpath, fn)
                with open(path, "rb") as f:
                    out[os.path.relpath(path, crate_dir)] = f.read()
    build = os.path.join(crate_dir, "build.rs")
    if os.path.isfile(build):
        with open(build, "rb") as f:
            out["build.rs"] = f.read()
    return out


def features(manifest_path):
    if not os.path.isfile(manifest_path):
        return {}
    with open(manifest_path, "rb") as f:
        return tomllib.load(f).get("features", {})


def differences(crate_dir, release_dir):
    now, then = shipped_files(crate_dir), shipped_files(release_dir)
    added = sorted(set(now) - set(then))
    removed = sorted(set(then) - set(now))
    changed = sorted(p for p in set(now) & set(then) if now[p] != then[p])
    original = os.path.join(release_dir, "Cargo.toml.orig")
    feats = features(os.path.join(crate_dir, "Cargo.toml")) != features(
        original if os.path.isfile(original) else os.path.join(release_dir, "Cargo.toml"))
    return added, removed, changed, feats


def pending_bullets(crate_dir, version):
    path = os.path.join(crate_dir, "CHANGELOG.md")
    if not os.path.isfile(path):
        return None
    text = open(path, encoding="utf-8").read()
    head = f"## [{version}]" if f"## [{version}]" in text else "## [Unreleased]"
    at = text.find(head)
    if at < 0:
        return []
    body = text[at + len(head):]
    nxt = body.find("\n## [")
    body = body[:nxt] if nxt >= 0 else body
    return [l for l in body.splitlines() if l.startswith("- ") and SYNCED not in l]


def main():
    version = workspace_version()
    failures, checked = [], 0
    with tempfile.TemporaryDirectory() as tmp:
        for name, crate_dir in published_members():
            if BASELINE:
                release_dir = os.path.join(BASELINE, name)
                if not os.path.isdir(release_dir):
                    print(f"  {name}: never released (no baseline) — skipped")
                    continue
            else:
                released = released_version(name)
                if released is None:
                    print(f"  {name}: never released on crates.io — skipped")
                    continue
                release_dir = fetch_release(name, released, tmp)
            added, removed, changed, feats = differences(crate_dir, release_dir)
            checked += 1
            if not (added or removed or changed or feats):
                continue
            bullets = pending_bullets(crate_dir, version)
            if not bullets:
                what = f"{len(added)} added, {len(removed)} removed, {len(changed)} changed files" + (", features changed" if feats else "")
                where = "has no CHANGELOG.md" if bullets is None else "has no bullet in its pending section"
                failures.append(f"{name}: shipped code changed since its last release ({what}) but {os.path.relpath(crate_dir, ROOT)}/CHANGELOG.md {where}")
    for f in failures:
        print(f"FAIL {f}")
    print(f"check-changelogs: {checked} published crates compared, {len(failures)} changed without a CHANGELOG entry")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
