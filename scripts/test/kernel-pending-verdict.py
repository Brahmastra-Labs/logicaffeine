#!/usr/bin/env python3
"""kernel-pending-verdict.py — the verdict of scripts/run-kernel-pending-tests.sh: each gap is exactly its gap.

    kernel-pending-verdict.py <junit.xml> <gaps> <cargo-nextest-exit>

<gaps> is the runner's KERNEL_PENDING_GAPS: one `<spec>|<signature>` per line, a spec (the last path segment of a
kernel test name) and a stable substring of the panic that names the capability it waits for. The run reads green
iff every spec ran, FAILED, and its failure carries its signature. Every other outcome is red and says what to do:

  * a spec PASSES — the capability landed: un-ignore it and drop it from the exclusion, the selector and the table;
  * a spec fails some other way — its panic is shown;
  * a spec did not run, or a test ran that the table does not name — the selector and the table disagree;
  * the run did not complete (cargo's exit is neither 0 nor nextest's 100 for failed tests), or left no report.

The report is nextest's JUnit, not its human output: a spec's failure text is its <failure> and its captured
stdout/stderr.
"""
import sys
import xml.etree.ElementTree as ET

TESTS_FAILED = 100


def gap_table(text):
    gaps = {}
    for line in text.splitlines():
        if not line.strip():
            continue
        spec, sep, signature = line.partition("|")
        if not sep or not spec.strip() or not signature.strip():
            raise SystemExit(f"RED: malformed KERNEL_PENDING_GAPS row {line!r}: want `<spec>|<signature>`")
        gaps[spec.strip()] = signature.strip()
    if not gaps:
        raise SystemExit("RED: KERNEL_PENDING_GAPS is empty")
    return gaps


def case_text(case):
    parts = []
    for child in case:
        if child.tag in ("failure", "error", "system-out", "system-err"):
            parts.append(child.get("message", ""))
            parts.append("".join(child.itertext()))
    return "\n".join(parts)


def verdict(junit, gaps):
    try:
        root = ET.parse(junit).getroot()
    except (OSError, ET.ParseError) as e:
        return [f"no JUnit report at {junit} ({e}), so no spec was judged"], []
    ran = {}
    for case in root.iter("testcase"):
        ran[case.get("name", "").rsplit("::", 1)[-1]] = case
    problems, confirmed = [], []
    for spec, signature in gaps.items():
        case = ran.get(spec)
        if case is None:
            problems.append(f"{spec}: did not run — renamed, or KERNEL_PENDING_ONLY no longer selects it")
            continue
        failed = any(child.tag in ("failure", "error") for child in case)
        if not failed:
            problems.append(
                f"{spec}: PASSES — the capability landed. Un-ignore it, and drop it from KERNEL_PENDING_EXCLUDE "
                f"(run-all-tests-fast.sh) and from KERNEL_PENDING_ONLY and KERNEL_PENDING_GAPS "
                f"(run-kernel-pending-tests.sh)"
            )
            continue
        text = case_text(case)
        if signature in text:
            confirmed.append(f"{spec}: gap confirmed — {signature}")
        else:
            problems.append(f"{spec}: fails, but not with its gap `{signature}`:\n{text.strip()[-4000:]}")
    for spec in sorted(set(ran) - set(gaps)):
        problems.append(f"{spec}: ran, but KERNEL_PENDING_GAPS has no row for it")
    return problems, confirmed


def main():
    if len(sys.argv) != 4:
        raise SystemExit(__doc__.split("\n\n")[1])
    junit, gaps, cargo_exit = sys.argv[1], gap_table(sys.argv[2]), int(sys.argv[3])
    if cargo_exit not in (0, TESTS_FAILED):
        print(f"RED the run did not complete: cargo nextest exited {cargo_exit}, so no spec was judged")
        print("VERDICT: RED — the run did not complete")
        return cargo_exit
    problems, confirmed = verdict(junit, gaps)
    for line in confirmed:
        print(line)
    for line in problems:
        print(f"RED {line}")
    if problems:
        print(f"VERDICT: RED — {len(problems)} of the kernel-pending checks did not hold")
        return 1
    print(f"VERDICT: {len(confirmed)}/{len(gaps)} gaps confirmed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
