#!/usr/bin/env python3
"""Judge a benchmark run against the runs before it.

    python3 benchmarks/compare-runs.py <candidate> [--against <baseline>]... [--report out.md] [--json out.json]
                                       [--tolerance 5] [--cross-tolerance 15]

<candidate> and every <baseline> is a results directory as run.sh writes it (latest.json, latest-interp.json,
latest-codec.json), or the path of its latest.json. Without --against the baseline is benchmarks/results, the
published reference run. Each baseline is its own comparison.

Every suite is measured against its yardstick (C for the cross-language suite, Node for the interpreter suite, postcard
for the wire codec) and only LOGOS rows are judged:

  same chip      Equal metadata.cpu. A row regresses when it is slower beyond the noise band and its speed relative to
                 the yardstick fell beyond the band; slower together with the yardstick is drift, the machine and not
                 the code. The band is --tolerance percent or three times the combined coefficient of variation,
                 whichever is wider. A row that had a timing and lost it regresses.
  another chip   Only yardstick-normalized ratios are compared. A row more than --cross-tolerance percent off the
                 baseline's ratio is out of line: a finding, never a failure. The chip's own speed is the geometric
                 mean of the yardstick's baseline time over its candidate time.
  codec sizes    Encoded sizes are deterministic: a change at the same commit is a regression, across commits a finding.

The report goes to stdout (and --report); --json writes the verdict for tools.
Exit 0: no regression. 1: a regression against some baseline. 2: unusable input.
"""

import argparse
import json
import math
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE / "lib"))

from assemble import geomean, ratio  # noqa: E402

PUBLISHED = HERE / "results"

SUITES = (
    {"name": "main", "suffix": "", "yardstick": "c", "title": "Cross-language suite", "against": "C"},
    {"name": "interp", "suffix": "-interp", "yardstick": "js", "title": "Interpreter suite", "against": "Node"},
    {"name": "codec", "suffix": "-codec", "yardstick": "postcard", "title": "Wire codec", "against": "postcard"},
)

CLASS_ORDER = ("regressed", "lost", "size_changed", "out_of_line_slower", "drift", "incomparable", "improved",
               "out_of_line_faster", "new")

CLASS_TEXT = {
    "regressed": "REGRESSED",
    "lost": "lost its timing",
    "size_changed": "encoded size changed",
    "out_of_line_slower": "out of line, slower",
    "drift": "slower along with the yardstick (machine drift)",
    "incomparable": "no size both runs timed",
    "improved": "improved",
    "out_of_line_faster": "out of line, faster",
    "new": "new",
}


class Unusable(Exception):
    pass


def ours(lang):
    return lang.startswith("logos")


def timings(row):
    return {lang: t for lang, t in row.items() if isinstance(t, dict) and t.get("mean_ms") is not None}


def cases_of(suite, data, source):
    """The suite as [{id, reference_size, collapse, scaling: {size: {lang: timing}}, sizes: {lang: bytes}}]."""
    try:
        if suite == "codec":
            cases = []
            for s in data["scenarios"]:
                for op in ("enc", "dec", "read_one"):
                    row = {r["codec"]: {"mean_ms": r[f"{op}_ns"], "cv": 0.0}
                           for r in s["rows"] if r.get(f"{op}_ns") is not None}
                    if row:
                        cases.append({"id": f"{s['id']}/{op}", "reference_size": "-", "collapse": None,
                                      "scaling": {"-": row}, "sizes": {}})
                cases.append({"id": f"{s['id']}/size", "reference_size": "-", "collapse": None, "scaling": {},
                              "sizes": {r["codec"]: r["size"] for r in s["rows"] if r.get("size") is not None}})
            return cases
        return [{"id": b["id"], "reference_size": b["reference_size"], "collapse": b.get("collapse"),
                 "scaling": {n: timings(row) for n, row in b["scaling"].items()}, "sizes": {}}
                for b in data["benchmarks"]]
    except (KeyError, TypeError, AttributeError) as e:
        raise Unusable(f"{source}: not a {suite} results file run.sh wrote (missing {e})")


class Run:
    def __init__(self, path):
        p = pathlib.Path(path)
        directory, stem = (p, "latest") if p.is_dir() else (p.parent, p.stem)
        self.path = str(p)
        self.suites = {}
        self.metadata = {}
        for suite in SUITES:
            f = directory / f"{stem}{suite['suffix']}.json"
            if not f.is_file():
                if suite["name"] == "main":
                    raise Unusable(f"{f}: no {stem}.json here; give a results directory run.sh wrote, or its latest.json")
                self.suites[suite["name"]] = None
                continue
            try:
                data = json.loads(f.read_text())
            except ValueError as e:
                raise Unusable(f"{f}: not JSON ({e})")
            if not isinstance(data, dict):
                raise Unusable(f"{f}: not a results object")
            if suite["name"] == "main":
                self.metadata = data.get("metadata") or {}
            self.suites[suite["name"]] = cases_of(suite["name"], data, f)

    @property
    def cpu(self):
        return self.metadata.get("cpu", "unknown CPU")

    @property
    def commit(self):
        return self.metadata.get("commit")

    def card(self):
        m = self.metadata
        return {"path": self.path, "cpu": self.cpu, "commit": self.commit, "logos_version": m.get("logos_version"),
                "date": m.get("date")}


def shared_size(b, c, *langs):
    """The size both cases timed every one of langs at: the baseline's reference size if they did, else the largest."""
    def has(case, n):
        row = case["scaling"].get(n, {})
        return all(l in row for l in langs)

    ref = b["reference_size"]
    if has(b, ref) and has(c, ref):
        return ref
    common = [n for n in b["scaling"] if has(b, n) and has(c, n)]
    return max(common, key=lambda n: int(n) if n.isdigit() else 0) if common else None


def any_size(case, *langs):
    return any(all(l in row for l in langs) for row in case["scaling"].values())


def cv(t):
    return t.get("cv") or 0.0


def finding(cls, case, lang, blocking, **extra):
    return {"class": cls, "case": case, "lang": lang, "blocking": blocking, **extra}


def compare_suite(suite, base_cases, cand_cases, same_chip, same_commit, tol, xtol):
    yard = suite["yardstick"]
    unit = "ns" if suite["name"] == "codec" else "ms"
    by_id = {c["id"]: c for c in cand_cases}
    base_ids = [b["id"] for b in base_cases]
    empty = {"id": None, "reference_size": None, "collapse": None, "scaling": {}, "sizes": {}}
    pairs = [(b, by_id.get(b["id"], dict(empty, id=b["id"]))) for b in base_cases]
    pairs += [(dict(empty, id=c["id"]), c) for c in cand_cases if c["id"] not in base_ids]

    findings, chip = [], []
    for b, c in pairs:
        case = b["id"]
        for codec in sorted(set(b["sizes"]) & set(c["sizes"])):
            if ours(codec) and b["sizes"][codec] != c["sizes"][codec]:
                findings.append(finding("size_changed", case, codec, same_commit, size=None, unit="bytes",
                                        baseline=b["sizes"][codec], candidate=c["sizes"][codec]))
        n = shared_size(b, c, yard)
        if n is not None:
            chip.append(b["scaling"][n][yard]["mean_ms"] / c["scaling"][n][yard]["mean_ms"])
        langs = sorted({l for x in (b, c) for row in x["scaling"].values() for l in row if ours(l)})
        for lang in langs:
            n = shared_size(b, c, lang, yard)
            had, has = any_size(b, lang, yard), any_size(c, lang, yard)
            if n is None:
                if had and not has:
                    findings.append(finding("lost", case, lang, same_chip, size=b["reference_size"]))
                elif has and not had:
                    findings.append(finding("new", case, lang, False, size=c["reference_size"]))
                elif had and has:
                    findings.append(finding("incomparable", case, lang, False, size=None))
                continue
            bt, by, ct, cy = b["scaling"][n][lang], b["scaling"][n][yard], c["scaling"][n][lang], c["scaling"][n][yard]
            r_old, r_new = by["mean_ms"] / bt["mean_ms"], cy["mean_ms"] / ct["mean_ms"]
            rel = r_new / r_old - 1
            change = ct["mean_ms"] / bt["mean_ms"] - 1
            detail = dict(size=None if n == "-" else n, unit=unit, baseline=bt["mean_ms"], candidate=ct["mean_ms"],
                          change_pct=round(change * 100, 4),
                          vs_yardstick={"baseline": r_old, "candidate": r_new, "change_pct": round(rel * 100, 4)})
            if same_chip:
                band = max(tol, 3 * math.hypot(cv(bt), cv(ct)))
                band_r = max(tol, 3 * math.sqrt(cv(bt) ** 2 + cv(ct) ** 2 + cv(by) ** 2 + cv(cy) ** 2))
                detail["band_pct"] = round(band * 100, 4)
                if change > band and rel < -band_r:
                    findings.append(finding("regressed", case, lang, True, **detail))
                elif change > band:
                    findings.append(finding("drift", case, lang, False, **detail))
                elif change < -band and rel > band_r:
                    findings.append(finding("improved", case, lang, False, **detail))
            else:
                detail["band_pct"] = round(xtol * 100, 4)
                if rel < -xtol:
                    findings.append(finding("out_of_line_slower", case, lang, False, **detail))
                elif rel > xtol:
                    findings.append(finding("out_of_line_faster", case, lang, False, **detail))

    def severity(f):
        moved = abs(f.get("vs_yardstick", {}).get("change_pct", 0.0))
        return (not f["blocking"], CLASS_ORDER.index(f["class"]), -moved, f["case"], f["lang"])

    findings.sort(key=severity)
    return {"status": "compared", "yardstick": yard, "chip_factor": geomean(chip), "findings": findings,
            "tiers": tiers(suite, base_cases, cand_cases)}


def tiers(suite, base_cases, cand_cases):
    yard = suite["yardstick"]
    langs = sorted({l for c in base_cases + cand_cases for row in c["scaling"].values() for l in row if ours(l)})
    groups = {"all": lambda c: True}
    if suite["name"] == "main":
        groups["same_algorithm"] = lambda c: not c["collapse"]
    out = {}
    for group, keep in groups.items():
        out[group] = {}
        for lang in langs:
            row = {}
            for side, cases in (("baseline", base_cases), ("candidate", cand_cases)):
                rs = [r for r in (ratio(c, lang, yard) for c in cases if c["scaling"] and keep(c)) if r]
                row[side] = round(geomean([r for r, _ in rs]), 6) if rs else None
            out[group][lang] = row
    return out


def toolchain_differences(base, cand):
    bv, cv = base.metadata.get("versions") or {}, cand.metadata.get("versions") or {}
    return {k: {"baseline": bv.get(k), "candidate": cv.get(k)} for k in sorted(set(bv) | set(cv)) if bv.get(k) != cv.get(k)}


def environment_warnings(run, role):
    env = run.metadata.get("environment")
    if not env:
        return [f"the {role} ({run.path}) records no machine environment, so its quiet-box state is unknown"]
    out = []
    gate = (env.get("quiet_box") or {}).get("gate")
    if gate != "passed":
        out.append(f"the {role} ({run.path}) was not measured on a quiet machine (quiet-box gate: {gate})")
    gov = env.get("governor")
    if gov and gov != "performance":
        out.append(f"the {role} ({run.path}) ran on the {gov} CPU governor, not performance")
    return out


def compare(cand, bases, tol, xtol):
    comparisons, warnings = [], environment_warnings(cand, "candidate")
    for base in bases:
        warnings += environment_warnings(base, "baseline")
        same_chip = base.cpu == cand.cpu
        same_commit = base.commit is not None and base.commit == cand.commit
        suites = {}
        for suite in SUITES:
            b, c = base.suites[suite["name"]], cand.suites[suite["name"]]
            if c is None:
                suites[suite["name"]] = {"status": "missing_in_candidate", "yardstick": suite["yardstick"], "findings": []}
            elif b is None:
                suites[suite["name"]] = {"status": "missing_in_baseline", "yardstick": suite["yardstick"], "findings": []}
            else:
                suites[suite["name"]] = compare_suite(suite, b, c, same_chip, same_commit, tol, xtol)
        comparisons.append({"baseline": base.card(), "kind": "same_chip" if same_chip else "cross_chip",
                            "same_commit": same_commit, "toolchain_differences": toolchain_differences(base, cand),
                            "suites": suites})
    blocking = sum(f["blocking"] for c in comparisons for s in c["suites"].values() for f in s["findings"])
    return {"verdict": "regression" if blocking else "pass", "blocking_findings": blocking,
            "candidate": cand.card(), "tolerance_pct": tol * 100, "cross_tolerance_pct": xtol * 100,
            "warnings": list(dict.fromkeys(warnings)), "comparisons": comparisons}


def fmt_time(v, unit):
    if v is None:
        return "-"
    if unit == "ns":
        return f"{v:,.0f} ns"
    return f"{v:,.1f} ms" if v < 10_000 else f"{v / 1000:,.2f} s"


def fmt_x(v):
    return "-" if v is None else f"{v:.3f}x"


def render(verdict):
    cand = verdict["candidate"]
    lines = [f"# Benchmark comparison: {cand['cpu']}", ""]
    lines.append(f"Candidate: `{cand['path']}`, LOGOS {cand['logos_version']} at {cand['commit']}, measured {cand['date']}")
    lines.append("")
    if verdict["verdict"] == "pass":
        lines.append("**Verdict: PASS**: no LOGOS row regressed against any baseline.")
    else:
        lines.append(f"**Verdict: REGRESSION**: {verdict['blocking_findings']} blocking finding(s), listed first below.")
    lines.append(f"Tolerance {verdict['tolerance_pct']:g}% on the same chip (wider where the runs are noisier), "
                 f"{verdict['cross_tolerance_pct']:g}% across chips.")
    for w in verdict["warnings"]:
        lines.append(f"- Warning: {w}")
    for comp in verdict["comparisons"]:
        base = comp["baseline"]
        kind = "same chip" if comp["kind"] == "same_chip" else "another chip, compared by speed relative to each suite's yardstick"
        lines += ["", f"## Against {base['cpu']} ({kind})", "",
                  f"Baseline: `{base['path']}`, LOGOS {base['logos_version']} at {base['commit']}, measured {base['date']}"]
        diffs = comp["toolchain_differences"]
        if diffs:
            lines.append("Toolchains that differ: " + "; ".join(
                f"{k}: {d['baseline'] or 'absent'} -> {d['candidate'] or 'absent'}" for k, d in diffs.items()))
        else:
            lines.append("Toolchains: identical.")
        for suite in SUITES:
            s = comp["suites"][suite["name"]]
            lines += ["", f"### {suite['title']} (yardstick: {suite['against']})", ""]
            if s["status"] != "compared":
                where = "candidate" if s["status"] == "missing_in_candidate" else "baseline"
                lines.append(f"Not compared: the {where} has no {suite['title'].lower()} results.")
                continue
            if s["chip_factor"] is not None:
                lines.append(f"{suite['against']} runs at {s['chip_factor']:.3f}x the baseline's speed on this machine.")
                lines.append("")
            for group, title in (("all", "every benchmark"), ("same_algorithm", "same-algorithm benchmarks")):
                tier = s["tiers"].get(group)
                if not tier:
                    continue
                lines += [f"Speed relative to {suite['against']}, geometric mean over {title}:", "",
                          "| Tier | Baseline | Candidate | Change |", "|---|---:|---:|---:|"]
                for lang, row in tier.items():
                    b, c = row["baseline"], row["candidate"]
                    change = f"{(c / b - 1) * 100:+.1f}%" if b and c else "-"
                    lines.append(f"| {lang} | {fmt_x(b)} | {fmt_x(c)} | {change} |")
                lines.append("")
            fs = s["findings"]
            if not fs:
                lines.append("No findings.")
                continue
            lines += ["| Finding | Case | Row | Size | Baseline | Candidate | Time | vs yardstick |",
                      "|---|---|---|---|---:|---:|---:|---:|"]
            for f in fs:
                name = CLASS_TEXT[f["class"]] + (" (blocking)" if f["blocking"] else "")
                unit = f.get("unit", "ms")
                if unit == "bytes":
                    b, c, t, y = f"{f['baseline']:,} B", f"{f['candidate']:,} B", "-", "-"
                else:
                    b, c = fmt_time(f.get("baseline"), unit), fmt_time(f.get("candidate"), unit)
                    t = f"{f['change_pct']:+.1f}%" if "change_pct" in f else "-"
                    y = f"{f['vs_yardstick']['change_pct']:+.1f}%" if "vs_yardstick" in f else "-"
                lines.append(f"| {name} | {f['case']} | {f['lang']} | {f.get('size') or '-'} | {b} | {c} | {t} | {y} |")
    return "\n".join(lines) + "\n"


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("candidate")
    ap.add_argument("--against", action="append", default=[])
    ap.add_argument("--report")
    ap.add_argument("--json", dest="json_out")
    ap.add_argument("--tolerance", type=float, default=5.0)
    ap.add_argument("--cross-tolerance", type=float, default=15.0)
    args = ap.parse_args(argv[1:])
    try:
        cand = Run(args.candidate)
        bases = [Run(a) for a in (args.against or [PUBLISHED])]
    except Unusable as e:
        print(f"compare-runs: {e}", file=sys.stderr)
        return 2
    verdict = compare(cand, bases, args.tolerance / 100, args.cross_tolerance / 100)
    report = render(verdict)
    sys.stdout.write(report)
    if args.report:
        pathlib.Path(args.report).write_text(report)
    if args.json_out:
        pathlib.Path(args.json_out).write_text(json.dumps(verdict, indent=2) + "\n")
    return 1 if verdict["verdict"] == "regression" else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
