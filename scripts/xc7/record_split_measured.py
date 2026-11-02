#!/usr/bin/env python3
"""record_split_measured.py LEDGER RESULT — put the split host's record of a run (`split-host.py --out`) into the split
ledger as its `measured`. Refuses a replayed run, one that does not say when it ran, and one whose boards ran other
bitstreams than the chips on record."""
import json
import sys

ledger_path, result = sys.argv[1], sys.argv[2]
ledger = json.load(open(ledger_path))
run = json.load(open(result))
if run.get("replayed"):
    sys.exit(f"{result}: a replayed run is no measurement")
if not run.get("date"):
    sys.exit(f"{result}: a run that does not say when it ran")
for chip in ledger["chips"]:
    ran = run["boards"].get(chip["board"], {})
    if ran.get("bitstream_sha256") != chip["sha256"]:
        sys.exit(f"{result}: board {chip['board']} ran {str(ran.get('bitstream_sha256'))[:12]}, chip {chip['chip']}'s image is {chip['sha256'][:12]}")
ledger["measured"] = run
json.dump(ledger, open(ledger_path, "w"), indent=2)
print(f"split run of {run['date']}: tokens {run['tokens']}, {run['wall_s_per_step'] * 1e3:.1f} ms a step over the host link, chips {run['chip_us_per_step']} µs")
