#!/usr/bin/env python3
"""Export unchanged conformance-corpus source declarations using pinned Lean4.14/lean4export.

This creates source evidence, not a conformance verdict. Existing output directories
are never overwritten. Rust builds/tests and Arena submission are not performed.
"""
from __future__ import annotations
import argparse, datetime, hashlib, json, os, pathlib, shutil, subprocess

ROOT = pathlib.Path(__file__).resolve().parents[2]
TOOLCHAIN = "leanprover/lean4:v4.14.0"
EXPORTER_REVISION = "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"

def digest(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()

def structural_exporter(exporter: pathlib.Path, out: pathlib.Path, lean: str) -> pathlib.Path:
    """Use Lean's existing exact expression key without changing upstream checkout."""
    target = out / "structural-exporter"
    library = target / ".lake/build/lib"
    library.mkdir(parents=True)
    shutil.copyfile(exporter / "Main.lean", target / "Main.lean")
    source = (exporter / "Export.lean").read_text()
    replacements = {
        "visitedExprs : Std.HashMap Expr Nat": "visitedExprs : Std.HashMap ExprStructEq Nat",
        "noMDataExprs : Std.HashMap Expr Expr": "noMDataExprs : Std.HashMap ExprStructEq Expr",
        'getIdx e "ie"': 'getIdx (ExprStructEq.mk e) "ie"',
        "noMDataExprs[e]?": "noMDataExprs[ExprStructEq.mk e]?",
        "noMDataExprs.insert e e'": "noMDataExprs.insert (ExprStructEq.mk e) e'",
    }
    for before, after in replacements.items():
        if source.count(before) != 1:
            raise ValueError("pinned exporter structural-key patch no longer applies exactly")
        source = source.replace(before, after)
    (target / "Export.lean").write_text(source)
    env = {**os.environ, "ELAN_TOOLCHAIN": TOOLCHAIN, "LEAN_PATH": str(library)}
    with (target / "build.log").open("wb") as log:
        subprocess.run([lean, "-o", str(library / "Export.olean"), "Export.lean"],
                       cwd=target, env=env, stdout=log, stderr=subprocess.STDOUT,
                       timeout=180, check=True)
    (target / "patch-provenance.json").write_text(json.dumps({
        "base_commit": EXPORTER_REVISION,
        "equality": "Lean.ExprStructEq / Expr.equal (names and binder annotations included)",
        "replacements": replacements,
        "base_source_sha256": digest(exporter / "Export.lean"),
        "patched_source_sha256": digest(target / "Export.lean"),
    }, indent=2) + "\n")
    return target

def select_declarations(ledger: dict, requested: list[str] | None) -> list[str]:
    """Select only names independently recorded by the original source ledger."""
    authored = {row["lean"] for row in ledger["declarations"]}
    if requested is None:
        return sorted(authored)
    if not requested:
        raise ValueError("empty declaration selection")
    selected = set(requested)
    unknown = selected - authored
    if unknown:
        raise ValueError("requested declarations are not authored in " + ledger["case"] + ": " + ", ".join(sorted(unknown)))
    return sorted(selected)

def export_case(case: dict, out: pathlib.Path, exporter: pathlib.Path, lean: str, structural: bool = False,
                requested: list[str] | None = None) -> dict:
    case_id = case["id"]
    source = ROOT / "crates/logicaffeine_tests/tests/fixtures/lean_conformance" / case["path"]
    ledger_path = ROOT / "crates/logicaffeine_tests/tests/fixtures/lean_conformance/bridge" / (case_id + ".json")
    ledger = json.loads(ledger_path.read_text())
    declarations = select_declarations(ledger, requested)
    target = out / case_id
    target.mkdir()
    module = case_id.replace("-", "_")
    copied = target / (module + ".lean")
    shutil.copyfile(source, copied)
    assert copied.read_bytes() == source.read_bytes()
    (target / "declarations.json").write_text(json.dumps(declarations, indent=2) + "\n")
    provenance = {
        "case": case_id, "source": str(source.relative_to(ROOT)),
        "source_sha256": digest(source), "ledger_sha256": digest(ledger_path),
        "toolchain": TOOLCHAIN, "exporter_commit": EXPORTER_REVISION,
        "expression_equality": "structural" if structural else "alpha_equivalent",
        "exporter_source_sha256": {p: digest(exporter / p) for p in ["Main.lean", "Export.lean"]},
        "exporter_olean_sha256": digest(exporter / ".lake/build/lib/Export.olean"),
        "exporter_flags": ["--export-mdata", "--export-unsafe"],
        "declarations": declarations, "declaration_selection": "all_authored" if requested is None else "explicit_authored_roots",
        "conformance_status": "not_evaluated",
    }
    env = os.environ.copy()
    env["ELAN_TOOLCHAIN"] = TOOLCHAIN
    env["LEAN_PATH"] = os.pathsep.join([str(exporter / ".lake/build/lib"), str(target)])
    with (target / "compile.log").open("wb") as log:
        result = subprocess.run([lean, "-o", module + ".olean", module + ".lean"],
                                cwd=target, env=env, stdout=log, stderr=subprocess.STDOUT,
                                timeout=180)
    provenance["compiler_exit"] = result.returncode
    if result.returncode == 0 and declarations:
        export = target / (case_id + ".ndjson")
        command = [lean, "--run", str(exporter / "Main.lean"), "--",
                   "--export-mdata", "--export-unsafe", module, "--", *declarations]
        with export.open("wb") as output, (target / "export.log").open("wb") as log:
            result = subprocess.run(command, cwd=target, env=env, stdout=output, stderr=log,
                                    timeout=180)
        provenance["exporter_exit"] = result.returncode
        if result.returncode == 0:
            with export.open() as stream:
                header = json.loads(next(stream))
            if header["meta"]["lean"]["version"] != "4.14.0":
                raise ValueError("export used the wrong Lean version")
            provenance["export_sha256"] = digest(export)
            provenance["export_header"] = header
            provenance["records"] = len(export.read_text().splitlines())
    elif not declarations:
        provenance["exporter_status"] = "no_authored_declarations"
    (target / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    return provenance

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    which = parser.add_mutually_exclusive_group(required=True)
    which.add_argument("--case", action="append", dest="cases")
    which.add_argument("--all", action="store_true")
    equality = parser.add_mutually_exclusive_group()
    equality.add_argument("--preserve-binder-metadata", action="store_true", default=True,
                          help="preserve source binder metadata using structural expression keys (default)")
    equality.add_argument("--kernel-expression-sharing", action="store_false", dest="preserve_binder_metadata",
                          help="use unmodified upstream alpha-equivalent sharing; binder annotations may be lost")
    parser.add_argument("--declaration", action="append", dest="declarations",
                        help="export one authored declaration root; repeat within exactly one --case")
    parser.add_argument("--exporter", type=pathlib.Path, default=ROOT / "reference-kernels/lean4export")
    parser.add_argument("--output", type=pathlib.Path)
    parser.add_argument("--lean", default=shutil.which("lean"))
    args = parser.parse_args()
    if args.declarations and (args.all or len(args.cases or []) != 1):
        parser.error("--declaration requires exactly one --case")
    if not args.lean:
        parser.error("Lean is not on PATH; provide --lean")
    exporter = args.exporter.resolve()
    actual = subprocess.check_output(["git", "-C", str(exporter), "rev-parse", "HEAD"], text=True).strip()
    if actual != EXPORTER_REVISION:
        parser.error("exporter revision differs from the pinned exporter")
    subprocess.run(["git", "-C", str(exporter), "diff", "--exit-code", "HEAD", "--",
                    "Main.lean", "Export.lean"], check=True)
    if not (exporter / ".lake/build/lib/Export.olean").is_file():
        parser.error("build the pinned exporter under Lean4.14.0 first")
    rows = json.loads((ROOT / "crates/logicaffeine_tests/tests/fixtures/lean_conformance/manifest.json").read_text())["cases"]
    if not args.all:
        wanted = set(args.cases)
        known = {row["id"] for row in rows}
        if not wanted <= known:
            parser.error("unknown cases: " + ", ".join(sorted(wanted - known)))
        rows = [row for row in rows if row["id"] in wanted]
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%d-%H%M%S-%f")
    out = (args.output or ROOT / "logs" / ("conformance-exports-" + stamp)).resolve()
    out.mkdir(parents=True, exist_ok=False)
    print("Artifacts:", out, flush=True)
    if args.preserve_binder_metadata:
        exporter = structural_exporter(exporter, out, args.lean)
    results = []
    for row in rows:
        try:
            result = export_case(row, out, exporter, args.lean, args.preserve_binder_metadata, args.declarations)
        except (OSError, ValueError, subprocess.SubprocessError) as exc:
            result = {"case": row["id"], "error": str(exc), "conformance_status": "not_evaluated"}
        results.append(result)
        (out / "index.json").write_text(json.dumps(results, indent=2) + "\n")
        print(row["id"], "exported" if result.get("exporter_exit") == 0 else result, flush=True)
    return int(any(row.get("compiler_exit") != 0
                   or row.get("exporter_exit", 0) != 0 or "error" in row for row in results))

if __name__ == "__main__":
    raise SystemExit(main())
