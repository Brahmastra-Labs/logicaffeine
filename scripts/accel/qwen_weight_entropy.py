#!/usr/bin/env python3
"""Measure the Shannon entropy of Qwen3-8B's Q8_0 int8 weights — the theoretical floor on how many bytes
a LOSSLESS re-encoding must read. If entropy << 8 bits/weight, we can read less than Q8_0's 8 bits and
beat the memory-bandwidth wall (same exact computation, fewer bytes). Reuses the GGUF reader."""
import math, os, struct, sys
import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from gguf_inspect import Reader, U32, STRING, ARRAY  # noqa

def open_gguf(path):
    f = open(path, "rb"); r = Reader(f)
    assert f.read(4) == b"GGUF"; ver = r.u32(); nt = r.u64(); nkv = r.u64()
    for _ in range(nkv):
        r.string(); vt = r.u32(); r.value(vt)
    tensors = {}
    for _ in range(nt):
        name = r.string(); nd = r.u32(); dims = [r.u64() for _ in range(nd)]; t = r.u32(); off = r.u64()
        tensors[name] = (dims, t, off)
    pos = f.tell(); align = 32; data_start = ((pos + align - 1)//align)*align
    return f, tensors, data_start

def q8_0_int8(f, data_start, off, ne0, ne1):
    """Read all int8 values of a Q8_0 tensor (ne1 rows x ne0 cols)."""
    nb = ne0 // 32; blk = 34
    f.seek(data_start + off)
    raw = f.read(ne1 * nb * blk)
    a = np.frombuffer(raw, dtype=np.uint8).reshape(ne1*nb, blk)
    q = a[:, 2:].view(np.int8)  # 32 int8 per block
    return q.reshape(-1)

def entropy_bits(vals):
    counts = np.bincount((vals.astype(np.int32) + 128), minlength=256)
    p = counts[counts > 0] / counts.sum()
    return float(-(p*np.log2(p)).sum())

if __name__ == "__main__":
    path = sys.argv[1] if len(sys.argv) > 1 else "models/qwen3-8b/Qwen_Qwen3-8B-Q8_0.gguf"
    f, tensors, ds = open_gguf(path)
    names = ["blk.0.attn_q.weight", "blk.0.ffn_gate.weight", "blk.15.ffn_down.weight", "output.weight"]
    print(f"{'tensor':28} {'entropy bits/wt':>15} {'vs 8-bit':>10}")
    all_vals = []
    for n in names:
        dims, t, off = tensors[n]
        if t != 8:
            print(f"{n:28} (not Q8_0)"); continue
        q = q8_0_int8(f, ds, off, dims[0], dims[1])
        h = entropy_bits(q)
        print(f"{n:28} {h:15.3f} {h/8*100:9.1f}%")
        all_vals.append(q[::7])  # subsample for the aggregate
    agg = np.concatenate(all_vals)
    h = entropy_bits(agg)
    print(f"{'AGGREGATE':28} {h:15.3f} {h/8*100:9.1f}%")
    print(f"\n=> a lossless code reads ~{h/8*100:.0f}% of Q8_0's bytes; projected tok/s = 37.6 * {8/h:.2f} = {37.6*8/h:.1f}")
