#!/usr/bin/env python3
"""Export the per-profile fixture project with every registered Lean profile's own toolchain and
pinned exporter, recording each export's provenance.

For every profile in `lean-exporters.json` the project
`crates/logicaffeine_tests/tests/fixtures/lean/projects/profile-<version>/` (its `lean-toolchain`
names the profile) is compiled in an isolated copy with that toolchain, Lean's own `#print axioms`
is recorded for every target, and the targets are exported with the profile's exporter from the
registry (`--registry`, else `$LOGOS_LEAN_EXPORTERS`, else `~/.logos/lean/exporters`; see
`install-lean-exporters.py`), each target alone, so each export is exactly that target's closure.
The exports land in `crates/logicaffeine_tests/tests/fixtures/lean/profiles/<version>/<target>.ndjson`
and their provenance in `PROVENANCE.json` beside the version directories. The exporter is verified
against its pins before it runs. This tool never runs git.

    scripts/lean/export-profile-fixtures.py            # every profile
    scripts/lean/export-profile-fixtures.py 4.33.1
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
HERE = pathlib.Path(__file__).resolve().parent
PROJECTS = ROOT / "crates/logicaffeine_tests/tests/fixtures/lean/projects"
OUT = ROOT / "crates/logicaffeine_tests/tests/fixtures/lean/profiles"
MODULE = "Profile"
# Each export and the targets it is the closure of: one target alone where its closure is small,
# and the four statement shapes whose placeholder proofs share Lean core's `Name` closure together.
EXPORTS = {
    "Profile.andSwap": ["Profile.andSwap"],
    "Profile.doubleTwo": ["Profile.doubleTwo"],
    "Profile.greetingIsHi": ["Profile.greetingIsHi"],
    "Profile.addZeroViaHave": ["Profile.addZeroViaHave"],
    "Profile.IsEven": ["Profile.IsEven"],
    "Profile.EveryDoubleIsEven": ["Profile.EveryDoubleIsEven"],
    "Profile.statements": [
        "Profile.unbounded",
        "Profile.answered",
        "Profile.unfinished",
        "Profile.RestsOnUnfinished",
    ],
}
TARGETS = [target for targets in EXPORTS.values() for target in targets]


def installer():
    spec = importlib.util.spec_from_file_location("install_lean_exporters", HERE / "install-lean-exporters.py")
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("versions", nargs="*")
    parser.add_argument("--registry")
    parser.add_argument("--elan-home")
    args = parser.parse_args()
    tools = installer()
    recipes = tools.load_recipes()
    registry = tools.registry_dir(args.registry)
    elan_home = pathlib.Path(args.elan_home or os.environ.get("ELAN_HOME") or pathlib.Path.home() / ".elan")
    if tools.verify(args.versions, registry) != 0:
        print("the exporter registry does not verify; run install-lean-exporters.py install", file=sys.stderr)
        return 1
    OUT.mkdir(parents=True, exist_ok=True)
    provenance_path = OUT / "PROVENANCE.json"
    provenance = json.loads(provenance_path.read_text()) if provenance_path.exists() else {
        "purpose": "one export per registered Lean import profile, produced by that profile's own toolchain and pinned exporter",
        "exports": {},
    }
    for version in args.versions or sorted(recipes["exporters"]):
        recipe = recipes["exporters"][version]
        project = PROJECTS / f"profile-{version}"
        pin = (project / "lean-toolchain").read_text().strip()
        if pin != recipe["toolchain"]:
            print(f"{project} pins {pin}, not {recipe['toolchain']}", file=sys.stderr)
            return 1
        toolchain, reported = tools.check_toolchain(recipe, version, elan_home)
        lean = toolchain / "bin" / "lean"
        entry = registry / version
        source_bytes = (project / f"{MODULE}.lean").read_bytes()
        with tempfile.TemporaryDirectory(prefix=f"logos-profile-{version}-") as work:
            work = pathlib.Path(work)
            shutil.copy(project / f"{MODULE}.lean", work / f"{MODULE}.lean")
            env = {"PATH": f"{toolchain / 'bin'}{os.pathsep}/usr/bin{os.pathsep}/bin",
                   "HOME": os.environ.get("HOME", "/tmp"), "ELAN_TOOLCHAIN": recipe["toolchain"],
                   "LEAN_PATH": str(work)}
            compiled = subprocess.run([str(lean), "-o", f"{MODULE}.olean", f"{MODULE}.lean"], cwd=work,
                                      env=env, capture_output=True, text=True, timeout=600)
            if compiled.returncode != 0:
                print(f"{version}: Lean did not compile the fixture:\n{compiled.stdout}{compiled.stderr}", file=sys.stderr)
                return 1
            audit = work / "audit"
            audit.mkdir()
            (audit / "Audit.lean").write_text(f"import {MODULE}\n" + "".join(f"#print axioms {t}\n" for t in TARGETS))
            axioms = subprocess.run([str(lean), "Audit.lean"], cwd=audit, env=env, capture_output=True,
                                    text=True, timeout=600)
            if axioms.returncode != 0:
                print(f"{version}: #print axioms failed:\n{axioms.stdout}{axioms.stderr}", file=sys.stderr)
                return 1
            flags = ["--export-mdata", "--export-unsafe"]
            reported_axioms = axioms.stdout.strip().splitlines()
            receipt = json.loads((entry / "receipt.json").read_text())
            (OUT / version).mkdir(parents=True, exist_ok=True)
            for stem, targets in EXPORTS.items():
                if recipe["form"] == "executable":
                    command = [str(entry / ".lake/build/bin/lean4export"), *flags, MODULE, "--", *targets]
                    run_env = {**env, "LEAN_SYSROOT": str(toolchain), "LEAN_PATH": f"{work}{os.pathsep}{toolchain / 'lib/lean'}"}
                    recorded = ["<registry>/" + version + "/.lake/build/bin/lean4export", *flags, MODULE, "--", *targets]
                else:
                    command = [str(lean), "--run", str(entry / "Main.lean"), "--", *flags, MODULE, "--", *targets]
                    run_env = {**env, "LEAN_PATH": f"{entry / '.lake/build/lib'}{os.pathsep}{work}"}
                    recorded = ["lean", "--run", "<registry>/" + version + "/Main.lean", "--", *flags, MODULE, "--", *targets]
                exported = subprocess.run(command, cwd=work, env=run_env, capture_output=True, timeout=1200)
                if exported.returncode != 0 or b"PANIC" in exported.stderr or b"uncaught exception" in exported.stderr:
                    print(f"{version} {stem}: the exporter failed:\n{exported.stderr.decode(errors='replace')}", file=sys.stderr)
                    return 1
                export = exported.stdout
                name = f"{version}/{stem}.ndjson"
                (OUT / name).write_bytes(export)
                records = export.count(b"\n")
                provenance["exports"][name] = {
                    "lean_version": version,
                    "toolchain": recipe["toolchain"],
                    "lean_reported": reported,
                    "exporter": {
                        "revision": recipe["revision"],
                        "upstream_tag": recipe["upstream_tag"],
                        "form": recipe["form"],
                        "structural_patch_sha256": recipes["structural_patch_sha256"] if recipe["structural"] else None,
                        "sources": recipe["sources"],
                        "binary_sha256": (receipt.get("binary") or {}).get("sha256"),
                    },
                    "command": recorded,
                    "source": f"crates/logicaffeine_tests/tests/fixtures/lean/projects/profile-{version}/{MODULE}.lean",
                    "source_sha256": sha256(source_bytes),
                    "targets": targets,
                    "print_axioms": [reported_axioms[TARGETS.index(t)] for t in targets],
                    "export_sha256": sha256(export),
                    "export_bytes": len(export),
                    "records": records,
                }
                print(f"{version}: {name} ({len(export)} bytes, {records} records)")
    provenance_path.write_text(json.dumps(provenance, indent=2, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
