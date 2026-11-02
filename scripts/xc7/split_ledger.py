#!/usr/bin/env python3
"""split_ledger.py OUT_JSON SILICON_DIR SPLIT_JSON DIR_A DIR_B — benchmarks/silicon/decode_split_images.json from the
two routed chip images of the decode step split over the boards (`decode_split_images`): the compiler's transfers and relay
(SPLIT_JSON), and for each chip what every routed image states (image_record.routed), its bitstream copied into
SILICON_DIR as split_<top>.bit, beside its own record — its board, its twin's count at each halt and at its end, its
halts with the words they send and receive, the words of its step's inputs and of its result."""
import json, os, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from image_record import routed  # noqa: E402

out, silicon, split_path, dirs = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4:]
split = json.load(open(split_path))
BOARD = {"xc7a200tsbg484-1": "A", "xc7a100tcsg324-1": "B"}
chips = []
for d in dirs:
    rec, common = routed(d, silicon, f"split_{json.load(open(os.path.join(d, 'image.json')))['top']}.bit")
    printed = next(c for c in split["chips"] if c["chip"] == rec["chip"])
    if printed["sources_sha256"] != rec["sources_sha256"]:
        sys.exit(f"{d}: chip {rec['chip']}'s image is not the one {split_path} was printed with")
    chips.append({
        "chip": rec["chip"],
        "board": BOARD[rec["part"]],
        "top": rec["top"],
        "part": rec["part"],
        "target": rec["target"],
        "mem_banks": rec["mem_banks"],
        "vector_lanes": rec.get("vector_lanes", 1),
        "read_latency": rec["read_latency"],
        "segments": rec["segments"],
        "links": rec["links"],
        "inputs": rec["inputs"],
        "next": rec["next"],
        "prompt_next_tokens": rec["prompt_next_tokens"],
        "sources_sha256": rec["sources_sha256"],
        **common,
    })
if sorted(c["chip"] for c in chips) != list(range(len(split["chips"]))):
    sys.exit(f"the chips routed are {[c['chip'] for c in chips]}, the split has {len(split['chips'])}")
chips.sort(key=lambda c: c["chip"])
json.dump({"transfers": split["transfers"], "relay": split["relay"], "chips": chips, "measured": {}}, open(out, "w"), indent=2)
for c in chips:
    print(f"chip {c['chip']} on board {c['board']}: {c['core_mhz']:.3f} MHz (routed {c['fmax_mhz']}), segments {c['segments']}, {c['bitstream']} {c['sha256'][:12]}")
