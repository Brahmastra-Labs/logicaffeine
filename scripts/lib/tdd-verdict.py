#!/usr/bin/env python3
"""tdd-verdict.py — the RED/GREEN verdict, as a pure function of JUnit reports.

    scripts/lib/tdd-verdict.py red   <report.xml> [<report.xml> ...]
    scripts/lib/tdd-verdict.py green <report.xml> [<report.xml> ...]

RED holds only when at least one test ran and EVERY test that ran failed: a test that
already passes before the fix proves nothing, so one passing test makes the verdict NOT RED.
GREEN holds only when at least one test ran and every one passed; a skipped test is neither.
Reports come from `cargo nextest run --profile tdd` (target/nextest/tdd/junit.xml) or from
the browser suite (scripts/web/test-learn-e2e.sh --junit), which use the same JUnit shape.

Each test is printed with its status and, for a failure, the reason it failed — the panic
message or assertion — so a RED can be checked for failing for the RIGHT reason.

Exit status: 0 when the expectation holds, 1 when it does not, 2 when the reports are
missing, unreadable, or select no tests at all.
Locked by crates/logicaffeine_tests/tests/tdd_verdict_lock.rs against fixtures.
"""
import sys
import xml.etree.ElementTree as ET

REASON_WIDTH = 220


def reason_of(case):
    """The line that says why a failed test failed: the panic's message, not its header,
    followed by how many more lines of message it has (the rest of a listing, an assertion's
    left/right values), so one run shows that there is more to read.

    Each chunk of the report (the failure body, stderr, stdout, the message attribute) is read
    on its own, because every chunk repeats the `thread '…' panicked at <file>:` header — the
    line after a header is only ever looked for within the same chunk."""
    chunks = []
    for tag in ("failure", "error"):
        for el in case.findall(tag):
            chunks.append(el.text or "")
            chunks.append(el.get("message") or "")
    for tag in ("system-err", "system-out"):
        el = case.find(tag)
        if el is not None:
            chunks.append(el.text or "")

    for chunk in chunks:
        lines = [l.strip() for l in chunk.splitlines()]
        for i, line in enumerate(lines):
            if "panicked at" not in line:
                continue
            body = []
            for l in lines[i + 1:]:
                if l.startswith("note: run with") or "panicked at" in l:
                    break
                if l:
                    body.append(l)
            if not body:
                continue
            message, more = body[0], len(body) - 1
            if "not yet implemented" in message or "not implemented" in message:
                return "todo!() stub — " + message, more
            return message, more
    return next((l.strip() for c in chunks for l in c.splitlines() if l.strip()), "failed"), 0


def shown(reason):
    message, more = reason
    return message[:REASON_WIDTH] + (f" (+{more} more lines)" if more else "")


def status_of(case):
    if case.find("skipped") is not None:
        return "SKIP"
    if case.find("failure") is not None or case.find("error") is not None:
        return "FAIL"
    return "PASS"


def load(paths):
    cases = []
    for path in paths:
        try:
            root = ET.parse(path).getroot()
        except (OSError, ET.ParseError) as e:
            raise SystemExit(f"tdd-verdict: INVALID — cannot read report {path}: {e}")
        for case in root.iter("testcase"):
            suite = case.get("classname") or ""
            name = case.get("name") or "?"
            full = f"{suite}::{name}" if suite else name
            status = status_of(case)
            reason = shown(reason_of(case)) if status == "FAIL" else ""
            cases.append((full, status, reason))
    cases.sort()
    return cases


def main(argv):
    if len(argv) < 3 or argv[1] not in ("red", "green"):
        print(__doc__.strip().splitlines()[2], file=sys.stderr)
        print(__doc__.strip().splitlines()[3], file=sys.stderr)
        return 2
    mode, paths = argv[1], argv[2:]
    try:
        cases = load(paths)
    except SystemExit as e:
        print(e, file=sys.stderr)
        return 2
    if not cases:
        print("tdd-verdict: INVALID — the reports select no tests; a filter that matches nothing proves nothing")
        return 2

    width = max(len(c[0]) for c in cases)
    for full, status, reason in cases:
        line = f"  {status}  {full.ljust(width)}"
        if reason:
            line += f"  — {reason}"
        print(line.rstrip())

    passed = [c for c in cases if c[1] == "PASS"]
    failed = [c for c in cases if c[1] == "FAIL"]
    skipped = [c for c in cases if c[1] == "SKIP"]
    n = len(cases)

    if mode == "red":
        if not passed and not skipped:
            print(f"RED CONFIRMED — {n}/{n} failing")
            return 0
        if passed:
            print(f"NOT RED — {len(passed)} of {n} already pass (a RED test must fail before the fix):")
            for full, _, _ in passed:
                print(f"    {full}")
        if skipped:
            print(f"NOT RED — {len(skipped)} of {n} skipped:")
            for full, _, _ in skipped:
                print(f"    {full}")
        return 1

    if not failed and not skipped:
        print(f"GREEN — {n}/{n} passing")
        return 0
    if failed:
        print(f"NOT GREEN — {len(failed)} of {n} failing:")
        for full, _, reason in failed:
            print(f"    {full}  — {reason}")
    if skipped:
        print(f"NOT GREEN — {len(skipped)} of {n} skipped:")
        for full, _, _ in skipped:
            print(f"    {full}")
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
