#!/usr/bin/env python3
"""record_measured.py LEDGER RESULT... — put each board run the host recorded (`decode-host.py --out`) into the image
ledger as that image's `measured`, dated by the host when the run began. Refuses a replayed run, one that does not say
when it ran, or one whose bitstream is not the image on record for its part."""
import json
import sys

ledger_path, results = sys.argv[1], sys.argv[2:]
ledger = json.load(open(ledger_path))
for path in results:
    run = json.load(open(path))
    if run.get("replayed"):
        sys.exit(f"{path}: a replayed run is no measurement")
    image = next((i for i in ledger["images"] if i["part"] == run["part"]), None)
    if image is None:
        sys.exit(f"{path}: no image on record for {run['part']}")
    if run["bitstream_sha256"] != image["sha256"]:
        sys.exit(f"{path}: ran bitstream {run['bitstream_sha256'][:12]}, the record's is {image['sha256'][:12]}")
    if not run.get("date"):
        sys.exit(f"{path}: a run that does not say when it ran")
    image["measured"] = run
    print(f"{run['part']}: {run['us_per_token']} µs/token, {run['tokens_per_s']} tokens/s on the board")
json.dump(ledger, open(ledger_path, "w"), indent=2)
