#!/usr/bin/env python3
"""Qwen3.8-Flash-Next's tensor table and integer constants, read from the published shards by range request.

    qwen4exp-headers.py [revision]

Writes into external/papers/qwen38-flash-next-headers/ (the default revision is the one the Samba book pins):
  model.safetensors.index.json        the checkpoint's shard index
  <shard>.header.json                 each shard's safetensors header, byte for byte (8-byte length prefix dropped)
  constants.json                      {tensor name: [values]} for every integer tensor (the n-gram hash's multipliers,
                                      head primes and offsets), read from its byte range

Nothing but the headers and the integer tensors is downloaded: about 2 MB of a 360 GB checkpoint. A shard already
fetched is kept. `qwen38_flash_next_facts_lock` reads the directory.
"""
import json
import struct
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "external" / "papers" / "qwen38-flash-next-headers"
REVISION = "de4b8e4d43b917e7706784d8bb445c9af86a3540"
INTEGER = {"I8": "b", "I16": "h", "I32": "i", "I64": "q", "U8": "B", "U16": "H", "U32": "I", "U64": "Q"}


def get(url: str, first: int, last: int) -> bytes:
    out = subprocess.run(["curl", "-fsSL", "-r", f"{first}-{last}", url], capture_output=True, check=True).stdout
    if len(out) != last - first + 1:
        sys.exit(f"{url}: asked for bytes {first}-{last}, got {len(out)}")
    return out


def main():
    revision = sys.argv[1] if len(sys.argv) > 1 else REVISION
    base = f"https://huggingface.co/Qwen/Qwen3.8-Flash-Next/resolve/{revision}"
    OUT.mkdir(parents=True, exist_ok=True)
    index = OUT / "model.safetensors.index.json"
    if not index.is_file():
        index.write_bytes(subprocess.run(["curl", "-fsSL", f"{base}/model.safetensors.index.json"],
                                         capture_output=True, check=True).stdout)
    shards = sorted(set(json.loads(index.read_text())["weight_map"].values()))
    constants = {}
    for shard in shards:
        url = f"{base}/{shard}"
        path = OUT / shard.replace(".safetensors", ".header.json")
        if not path.is_file():
            length = struct.unpack("<Q", get(url, 0, 7))[0]
            head = get(url, 8, 7 + length)
            json.loads(head)
            path.write_bytes(head)
            time.sleep(0.2)
        length = path.stat().st_size
        for name, entry in json.loads(path.read_bytes()).items():
            if name == "__metadata__" or entry["dtype"] not in INTEGER:
                continue
            first, last = entry["data_offsets"]
            code = INTEGER[entry["dtype"]]
            raw = get(url, 8 + length + first, 8 + length + last - 1)
            constants[name] = list(struct.unpack(f"<{len(raw) // struct.calcsize(code)}{code}", raw))
    (OUT / "constants.json").write_text(json.dumps(constants, indent=1, sort_keys=True) + "\n")
    print(f"{len(shards)} headers, {len(constants)} integer tensors")


if __name__ == "__main__":
    main()
