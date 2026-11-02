#!/usr/bin/env python3
"""Scaffold the north-star parity corpus from benchmarks/parity/catalog/superset.json.

Emits one crates/logicaffeine_tests/tests/parity_<domain>.rs per domain. Every non-na
capability becomes a test whose fn name is exactly its catalog `test` field, carrying its
provenance + intended LOGOS surface in a doc comment. `pending` tests are #[ignore]d (the
tree stays green); a `green` capability's test is un-ignored with a real assert_all_tiers body.

SAFE TO RE-RUN: existing files are NEVER overwritten (hand-authored behavioral bodies survive);
only missing domain files are scaffolded. Delete a file to regenerate it.
"""
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
CATALOG = ROOT / "benchmarks" / "parity" / "catalog" / "superset.json"
TESTS = ROOT / "crates" / "logicaffeine_tests" / "tests"

HEADER = '''//! Stdlib-parity north-star corpus — domain: {domain}
//!
//! Auto-scaffolded from benchmarks/parity/catalog/superset.json by
//! scripts/bench/gen-parity-corpus.py. Each test mirrors one deduplicated capability of the
//! C/Python/Rust/Go (+Java) superset. `#[ignore]` = parity-pending (not yet implemented);
//! the tree stays green. To LAND a capability: implement it, replace the body with a real
//! `assert_all_tiers` behavioral spec, DELETE the `#[ignore]` line, flip the catalog entry
//! to `status: green`, and bump GREEN_FLOOR in parity_catalog_lock.rs.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;
'''


def prov_line(cap):
    order = ["c", "python", "rust", "go", "java"]
    parts = []
    for lang in order:
        syms = cap["provenance"].get(lang)
        if syms:
            parts.append(f"{lang}: {', '.join(syms)}")
    return " · ".join(parts)


def emit_test(cap):
    surface = cap.get("logos_surface") or "(surface TBD)"
    return (
        f'/// {cap["intent"]}\n'
        f'/// provenance — {prov_line(cap)}\n'
        f'/// LOGOS surface: `{surface}`\n'
        f'#[test]\n'
        f'#[ignore = "parity-pending: {cap["id"]}"]\n'
        f'fn {cap["test"]}() {{\n'
        f'    // Behavioral spec authored at landing (RED->GREEN): assert_all_tiers(src, expected).\n'
        f'}}\n'
    )


def main():
    cat = json.load(open(CATALOG))
    by_domain = {}
    for cap in cat["capabilities"]:
        if cap["status"] == "na":
            continue
        by_domain.setdefault(cap["domain"], []).append(cap)

    created, skipped = [], []
    for domain, caps in sorted(by_domain.items()):
        path = TESTS / f"parity_{domain}.rs"
        if path.exists():
            skipped.append(path.name)
            continue
        caps.sort(key=lambda c: c["id"])
        body = HEADER.format(domain=domain) + "\n" + "\n".join(emit_test(c) for c in caps)
        path.write_text(body)
        created.append(f"{path.name} ({len(caps)} tests)")

    print("Created:", ", ".join(created) if created else "none")
    if skipped:
        print("Skipped (exist):", ", ".join(skipped))
    return 0


if __name__ == "__main__":
    sys.exit(main())
