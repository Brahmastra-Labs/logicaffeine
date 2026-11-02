#!/usr/bin/env python3
"""Record a bring-up receipt the card ran, refusing one that did not pass or ran on an image nobody recorded.

    record_receipt.py <receipt.json> <gbs> <afu_images.json> <receipts.json>

<receipt.json> is logos_receipt's output on the card; <gbs> the image it ran on. The receipt is recorded only when:
  - every check held (`checks_failed` 0), every channel read back what it wrote, and the host round trip held;
  - on SECDED lines, every channel's code held on every line: a flipped data bit and a flipped check bit corrected,
    two flips in one word found uncorrectable and no other word touched, a flip in each of two words both corrected,
    nothing corrected or lost before the flips or after them;
  - the image is one afu_images.json recorded (by SHA-256), and the receipt names that image's AFU ID;
  - the receipts ledger holds no receipt of the same image at the same sizes.
The row carries the image's hash, the user clock, each channel's and all channels' write and read bandwidth, the
host copies' cycles, and on SECDED lines each channel's code.
"""
import hashlib
import json
import os
import sys
from datetime import datetime, timezone


def refuse(why):
    print(f"record_receipt: refused: {why}")
    sys.exit(1)


def main():
    if len(sys.argv) != 5:
        sys.exit(__doc__)
    receipt_path, gbs, images_path, ledger_path = sys.argv[1:]
    try:
        receipt = json.load(open(receipt_path))
    except (OSError, ValueError) as e:
        refuse(f"{receipt_path} is no receipt: {e}")
    if receipt.get("checks_failed") != 0:
        refuse(f"{receipt.get('checks_failed')} checks failed")
    channels = receipt.get("channels", [])
    if not channels or any(c.get("mismatches") != 0 for c in channels):
        refuse("a channel read back other lines than it wrote")
    if receipt.get("host", {}).get("round_trip") is not True:
        refuse("host lines did not come back unchanged")
    for c in receipt.get("secded", []):
        n = c.get("lines")
        if not isinstance(n, int) or n <= 0:
            refuse(f"channel {c.get('channel')}'s SECDED cases name no lines")
        held = {
            "before": (None, 0, 0),
            "data_bit": (0, n, 0),
            "check_bit": (0, n, 0),
            "two_bits_one_word": (n, 0, n),
            "two_words": (0, 2 * n, 0),
            "clean": (0, 0, 0),
        }
        for case, want in held.items():
            got = c.get(case, {})
            if (got.get("mismatches"), got.get("corrected"), got.get("uncorrectable")) != want:
                refuse(f"channel {c.get('channel')}'s SECDED code did not hold: {case} {got}")
    sha = hashlib.sha256(open(gbs, "rb").read()).hexdigest()
    images = json.load(open(images_path)) if os.path.isfile(images_path) else {"images": []}
    image = next((i for i in images["images"] if i["sha256"] == sha), None)
    if image is None:
        refuse(f"the image {sha[:12]} is not in {images_path}")
    if image["afu_uuid"] != receipt.get("afu_id"):
        refuse(f"the receipt names the AFU {receipt.get('afu_id')}, the image is {image['afu_uuid']}")
    ledger = json.load(open(ledger_path)) if os.path.isfile(ledger_path) else {
        "_what": "Bring-up receipts the D5005 ran (scripts/d5005/run-receipt.sh), each recorded by "
                 "scripts/d5005/record_receipt.py against the image it ran on, only when every check held.",
        "receipts": [],
    }
    key = (sha, channels[0]["lines"], receipt["host"]["lines"])
    if any((r["gbs_sha256"], r["lines"], r["host_lines"]) == key for r in ledger["receipts"]):
        refuse(f"the ledger already holds a receipt of {sha[:12]} at these sizes")
    ledger["receipts"].append({
        "recorded": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "gbs_sha256": sha,
        "afu_id": receipt["afu_id"],
        "user_clock_hz": receipt["user_clock_hz"],
        "line_bytes": receipt["line_bytes"],
        "lines": channels[0]["lines"],
        "host_lines": receipt["host"]["lines"],
        "channels": [{k: c[k] for k in ("mask", "write_gb_per_s", "read_gb_per_s", "fill_cycles", "check_cycles")} for c in channels],
        "host": receipt["host"],
        **({"secded": receipt["secded"]} if "secded" in receipt else {}),
    })
    os.makedirs(os.path.dirname(os.path.abspath(ledger_path)), exist_ok=True)
    with open(ledger_path, "w") as f:
        json.dump(ledger, f, indent=1)
        f.write("\n")
    best = max(channels, key=lambda c: c["mask"])
    print(f"record_receipt: recorded {sha[:12]}: all channels {best['write_gb_per_s']} GB/s write, {best['read_gb_per_s']} GB/s read")


if __name__ == "__main__":
    main()
