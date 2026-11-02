#!/usr/bin/env python3
"""decode_ledger.py OUT_JSON SILICON_DIR IMAGE_DIR... — benchmarks/silicon/decode_images.json from routed decode board images:
each image's record (image.json) and what every routed image states (image_record.routed), its bitstream copied into
SILICON_DIR as decode_<top>.bit."""
import json, os, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from image_record import routed  # noqa: E402

out, silicon, dirs = sys.argv[1], sys.argv[2], sys.argv[3:]
images = []
for d in dirs:
    rec, common = routed(d, silicon, f"decode_{json.load(open(os.path.join(d, 'image.json')))['top']}.bit")
    images.append({
        "top": rec["top"],
        "part": rec["part"],
        "target": rec["target"],
        "mem_banks": rec["mem_banks"],
        "vector_lanes": rec.get("vector_lanes", 1),
        "read_latency": rec["read_latency"],
        "cycles_per_step": rec["cycles_per_step"],
        "loop_cycles_per_step": rec["loop_cycles_per_step"],
        "prompt_next_tokens": rec["prompt_next_tokens"],
        "sources_sha256": rec["sources_sha256"],
        "pll": common["pll"],
        "core_mhz": common["core_mhz"],
        "fmax_mhz": common["fmax_mhz"],
        "timing": common["timing"],
        "step_us": round(rec["cycles_per_step"] / common["core_mhz"], 6),
        "pnr": common["pnr"],
        "lutram_contents": common["lutram_contents"],
        "utilization": common["utilization"],
        "bitstream": common["bitstream"],
        "bytes": common["bytes"],
        "sha256": common["sha256"],
        "measured": {},
    })
json.dump({"images": images}, open(out, "w"), indent=2)
print(json.dumps(images, indent=2)[:1500])
