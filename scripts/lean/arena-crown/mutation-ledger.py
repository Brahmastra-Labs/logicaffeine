#!/usr/bin/env python3
"""Fold a cargo-mutants run of the dag core into the committed mutation ledger.

usage: mutation-ledger.py <mutants.out dir>

Every mutant the Arena corpus and adversarial suites did not catch is recorded in
`crates/logicaffeine_tests/tests/fixtures/mutation_ledger.json` and must carry a classification
(`dag_mutation_ledger_lock` fails on an unclassified one):

  equivalent          the mutant computes the same verdicts (say why in `note`)
  performance_only    it changes only speed or memory (a cache, a fast path, a capacity)
  redundant_check     another check rejects the same inputs (`note` names it)
  test_gap_fixed_by   a test now catches it (`note` names the test function)

Keys are line-independent (file, function, replacement, and the source line the mutant changes),
so a ledger survives edits that move code. Existing classifications are kept; mutants caught in the
new run leave the ledger; new survivors enter as `unclassified`.
"""
import json, os, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
LEDGER = os.path.join(ROOT, "crates/logicaffeine_tests/tests/fixtures/mutation_ledger.json")
KINDS = {"equivalent", "performance_only", "redundant_check", "test_gap_fixed_by"}


def removed_line(out_dir, diff_path):
    try:
        for line in open(os.path.join(out_dir, diff_path)):
            if line.startswith("-") and not line.startswith("---"):
                return line[1:].strip()
    except OSError:
        pass
    return ""


def key_of(m, snippet):
    fn = (m.get("function") or {}).get("function_name", "")
    return f'{m["file"]} :: {fn} :: {m.get("replacement", "")} :: {snippet}'


def main():
    out_dir = sys.argv[1]
    run = json.load(open(os.path.join(out_dir, "outcomes.json")))
    old = json.load(open(LEDGER)) if os.path.exists(LEDGER) else {"missed": {}}
    missed = {}
    for o in run["outcomes"]:
        m = o["scenario"].get("Mutant") if isinstance(o["scenario"], dict) else None
        if not m or o["summary"] != "MissedMutant":
            continue
        snippet = removed_line(out_dir, o.get("diff_path", ""))
        k = key_of(m, snippet)
        prior = old["missed"].get(k, {})
        missed[k] = {
            "mutant": m["name"],
            "changes": snippet,
            "classification": prior.get("classification", "unclassified"),
            "note": prior.get("note", ""),
        }
    doc = {
        "schema": "logos-mutation-ledger/1",
        "cargo_mutants_version": run.get("cargo_mutants_version"),
        "run": {k: run.get(k) for k in ("total_mutants", "caught", "missed", "timeout", "unviable", "start_time", "end_time")},
        "missed": dict(sorted(missed.items())),
    }
    json.dump(doc, open(LEDGER, "w"), indent=1, sort_keys=True)
    open(LEDGER, "a").write("\n")
    unclassified = sum(1 for v in missed.values() if v["classification"] == "unclassified")
    print(f"ledger: {len(missed)} surviving mutants, {unclassified} unclassified; run {doc['run']}")


if __name__ == "__main__":
    sys.exit(main())
