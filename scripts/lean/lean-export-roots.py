#!/usr/bin/env python3
"""Export explicit declaration roots of one Lean source with the pinned Lean 4.14.0 and the pinned
lean4export (structural expression keys), each root alone, with its provenance.

For every root the script writes `<out>/<stem>.<name>.ndjson` and `<out>/<stem>.<name>.provenance.json`,
where `<name>` is the root's last component. The provenance records the source digest, the toolchain,
the exporter revision and source digests (the structural-key patch applied exactly as
`scripts/lean/export-conformance.py` applies it), the exporter's olean digest, Lean's own `#print axioms`
for the root, the export digest, header and record count.

Pins are read from files; this tool never runs git. It never writes into `crates/logicaffeine_tests/tests/fixtures/lean_conformance/cases/` and never
overwrites an existing fixture unless `--replace` is given.

    scripts/lean/lean-export-roots.py --source crates/logicaffeine_tests/tests/fixtures/lean_conformance/cases/t08_tactics/T08-014_decide.lean \\
        --module T08_014 --root T08_014.viaNative --stem T08-014 \\
        --selection explicit_authored_roots --out crates/logicaffeine_tests/tests/fixtures/lean_conformance/cases_lean414/native_decide
"""
from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
TOOLCHAIN = "leanprover/lean4:v4.14.0"
LEAN_VERSION = "4.14.0"
EXPORTER_REVISION = "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
EXPORTER_SOURCES = {
    "Main.lean": "bd8e350903f1382b25fe767f9086dd9882000f01085f7046ea06545e7848842a",
    "Export.lean": "16fa1fec370500f4ca03f042fc3e65cac27839b640b32c5a921e085206c7031b",
}
STRUCTURAL_EXPORT_SOURCE = "5fecc2ebc45c62b618ec47705d47479117533676ae2f8e1e7e6740d941153224"
STRUCTURAL_EXPORT_OLEAN = "1fa13a6a1051da6ee408ef0e558fa0dd73a2ce8d7aab25748081f9c75ed9e747"
EXPORTER_FLAGS = ["--export-mdata", "--export-unsafe"]
STRUCTURAL_REPLACEMENTS = {
    "visitedExprs : Std.HashMap Expr Nat": "visitedExprs : Std.HashMap ExprStructEq Nat",
    "noMDataExprs : Std.HashMap Expr Expr": "noMDataExprs : Std.HashMap ExprStructEq Expr",
    'getIdx e "ie"': 'getIdx (ExprStructEq.mk e) "ie"',
    "noMDataExprs[e]?": "noMDataExprs[ExprStructEq.mk e]?",
    "noMDataExprs.insert e e'": "noMDataExprs.insert (ExprStructEq.mk e) e'",
}


def digest(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def lean_env(library_path: list[pathlib.Path]) -> dict:
    env = {**os.environ, "ELAN_TOOLCHAIN": TOOLCHAIN,
           "LEAN_PATH": os.pathsep.join(str(p) for p in library_path)}
    env.pop("LEAN_SRC_PATH", None)
    return env


def verify_lean(lean: str) -> str:
    version = subprocess.check_output([lean, "--version"], env=lean_env([]), text=True).strip()
    if f"version {LEAN_VERSION}," not in version:
        raise SystemExit(f"wrong Lean: {version}")
    return version


def verify_exporter(exporter: pathlib.Path) -> None:
    head = (exporter / ".git/HEAD").read_text().strip()
    if head != EXPORTER_REVISION:
        raise SystemExit(f"the exporter checkout is at {head}, pinned {EXPORTER_REVISION}")
    for name, expected in EXPORTER_SOURCES.items():
        if digest(exporter / name) != expected:
            raise SystemExit(f"the exporter's {name} differs from the pinned source")


def structural_exporter(exporter: pathlib.Path, work: pathlib.Path, lean: str) -> pathlib.Path:
    target = work / "structural-exporter"
    library = target / ".lake/build/lib"
    library.mkdir(parents=True)
    shutil.copyfile(exporter / "Main.lean", target / "Main.lean")
    source = (exporter / "Export.lean").read_text()
    for before, after in STRUCTURAL_REPLACEMENTS.items():
        if source.count(before) != 1:
            raise SystemExit("the structural-key patch no longer applies exactly")
        source = source.replace(before, after)
    (target / "Export.lean").write_text(source)
    if digest(target / "Export.lean") != STRUCTURAL_EXPORT_SOURCE:
        raise SystemExit("the patched exporter source is not the pinned structural source")
    subprocess.run([lean, "-o", str(library / "Export.olean"), "Export.lean"], cwd=target,
                   env=lean_env([library]), check=True, timeout=600)
    if digest(library / "Export.olean") != STRUCTURAL_EXPORT_OLEAN:
        raise SystemExit("the structural exporter did not build to the pinned olean")
    return target


def print_axioms(lean: str, module: str, root: str, stage: pathlib.Path, library: list[pathlib.Path]) -> list[str]:
    audit = stage / "Audit.lean"
    audit.write_text(f"import {module}\n#print axioms {root}\n")
    out = subprocess.run([lean, "Audit.lean"], cwd=stage, env=lean_env(library), capture_output=True,
                         text=True, timeout=600)
    if out.returncode != 0:
        raise SystemExit(f"#print axioms {root} failed: {out.stdout}{out.stderr}")
    return [line for line in out.stdout.splitlines() if line.strip()]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--source", type=pathlib.Path, required=True)
    parser.add_argument("--module", required=True, help="the module name the source is compiled as")
    parser.add_argument("--root", action="append", required=True, dest="roots")
    parser.add_argument("--stem", required=True)
    parser.add_argument("--selection", required=True, choices=["explicit_authored_roots", "explicit_witness_roots"])
    parser.add_argument("--out", type=pathlib.Path, required=True)
    parser.add_argument("--exporter", type=pathlib.Path, default=ROOT / "reference-kernels/lean4export")
    parser.add_argument("--lean", default=shutil.which("lean"))
    parser.add_argument("--replace", action="store_true")
    args = parser.parse_args()
    if not args.lean:
        parser.error("Lean is not on PATH; pass --lean")
    lean_version = verify_lean(args.lean)
    exporter = args.exporter.resolve()
    verify_exporter(exporter)
    source = args.source.resolve()
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%d-%H%M%S-%f")
    work = ROOT / "logs" / f"lean-export-roots-{stamp}"
    work.mkdir(parents=True)
    structural = structural_exporter(exporter, work, args.lean)
    library = structural / ".lake/build/lib"
    stage = work / "stage"
    stage.mkdir()
    shutil.copyfile(source, stage / f"{args.module}.lean")
    compiled = subprocess.run([args.lean, "-o", f"{args.module}.olean", f"{args.module}.lean"], cwd=stage,
                              env=lean_env([library, stage]), capture_output=True, text=True, timeout=1800)
    (work / "compile.log").write_text(compiled.stdout + compiled.stderr)
    if compiled.returncode != 0:
        raise SystemExit(f"Lean did not compile {source}: {compiled.stdout}{compiled.stderr}")
    args.out.mkdir(parents=True, exist_ok=True)
    for root in args.roots:
        name = root.split(".")[-1]
        export = args.out / f"{args.stem}.{name}.ndjson"
        provenance_path = args.out / f"{args.stem}.{name}.provenance.json"
        if not args.replace and (export.exists() or provenance_path.exists()):
            raise SystemExit(f"{export} exists; pass --replace to regenerate it")
        axioms = print_axioms(args.lean, args.module, root, stage, [library, stage])
        with export.open("wb") as output:
            exported = subprocess.run([args.lean, "--run", str(structural / "Main.lean"), "--", *EXPORTER_FLAGS,
                                       args.module, "--", root], cwd=stage, env=lean_env([library, stage]),
                                      stdout=output, stderr=subprocess.PIPE, timeout=1800)
        stderr = exported.stderr.decode("utf-8", "replace")
        if exported.returncode != 0 or "PANIC" in stderr or "uncaught exception" in stderr:
            raise SystemExit(f"the exporter failed on {root}: {stderr}")
        with export.open() as stream:
            header = json.loads(next(stream))
        if header["meta"]["lean"]["version"] != LEAN_VERSION:
            raise SystemExit("the export was not produced by the pinned Lean")
        try:
            relative_source = str(source.relative_to(ROOT))
        except ValueError:
            raise SystemExit(f"{source} is outside the repository")
        provenance = {
            "source": relative_source,
            "source_sha256": digest(source),
            "module": args.module,
            "toolchain": TOOLCHAIN,
            "lean_version": lean_version,
            "exporter_commit": EXPORTER_REVISION,
            "expression_equality": "structural",
            "exporter_source_sha256": {"Main.lean": digest(structural / "Main.lean"),
                                       "Export.lean": digest(structural / "Export.lean")},
            "exporter_olean_sha256": digest(library / "Export.olean"),
            "exporter_flags": EXPORTER_FLAGS,
            "declarations": [root],
            "declaration_selection": args.selection,
            "compiler_exit": compiled.returncode,
            "exporter_exit": exported.returncode,
            "lean_print_axioms": axioms,
            "export_sha256": digest(export),
            "export_header": header,
            "records": len(export.read_text().splitlines()),
            "conformance_status": "not_evaluated",
        }
        provenance_path.write_text(json.dumps(provenance, indent=2) + "\n")
        print(root, provenance["export_sha256"], provenance["records"], axioms, flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
