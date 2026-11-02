#!/usr/bin/env python3
"""Read a GGUF file's header + metadata and print the transformer architecture.

The Phase-3 grounding tool for the CUDA model-lowering tier: it parses the real
downloaded Qwen3-8B Q8_0 checkpoint (no external deps) so the block-lowering plan
is driven by the checkpoint's own dimensions, not assumptions. Also lists the
tensors of block 0 (the one we lower first) with their shapes and quant types.

Usage:
    python3 scripts/accel/gguf_inspect.py models/qwen3-8b/Qwen_Qwen3-8B-Q8_0.gguf
"""
import struct
import sys

# GGUF metadata value type tags (gguf spec v2/v3).
U8, I8, U16, I16, U32, I32, F32, BOOL, STRING, ARRAY, U64, I64, F64 = range(13)

# GGML tensor quant type -> name (the subset we care about).
GGML_TYPE = {0: "F32", 1: "F16", 8: "Q8_0", 12: "Q4_K", 14: "Q6_K", 15: "Q8_K"}


class Reader:
    def __init__(self, f):
        self.f = f

    def _u(self, n, sfmt):
        return struct.unpack(sfmt, self.f.read(n))[0]

    def u32(self):
        return self._u(4, "<I")

    def u64(self):
        return self._u(8, "<Q")

    def string(self):
        n = self.u64()
        return self.f.read(n).decode("utf-8", "replace")

    def scalar(self, t):
        fmt = {U8: ("<B", 1), I8: ("<b", 1), U16: ("<H", 2), I16: ("<h", 2),
               U32: ("<I", 4), I32: ("<i", 4), F32: ("<f", 4), BOOL: ("<?", 1),
               U64: ("<Q", 8), I64: ("<q", 8), F64: ("<d", 8)}
        if t == STRING:
            return self.string()
        s, n = fmt[t]
        return struct.unpack(s, self.f.read(n))[0]

    def value(self, t):
        if t == ARRAY:
            et = self.u32()
            count = self.u64()
            return [self.value(et) if et in (STRING, ARRAY) else self.scalar(et) for _ in range(count)]
        return self.scalar(t)


def inspect(path):
    with open(path, "rb") as f:
        r = Reader(f)
        magic = f.read(4)
        assert magic == b"GGUF", f"not a GGUF file: {magic!r}"
        version = r.u32()
        n_tensors = r.u64()
        n_kv = r.u64()

        meta = {}
        for _ in range(n_kv):
            key = r.string()
            vtype = r.u32()
            meta[key] = r.value(vtype)

        arch = meta.get("general.architecture", "?")
        g = lambda k, d=None: meta.get(f"{arch}.{k}", d)
        cfg = {
            "arch": arch,
            "name": meta.get("general.name", "?"),
            "block_count": g("block_count"),
            "embedding_length": g("embedding_length"),
            "feed_forward_length": g("feed_forward_length"),
            "head_count": g("attention.head_count"),
            "head_count_kv": g("attention.head_count_kv"),
            "key_length": g("attention.key_length"),
            "value_length": g("attention.value_length"),
            "rope_freq_base": g("rope.freq_base"),
            "rms_eps": g("attention.layer_norm_rms_epsilon"),
            "context_length": g("context_length"),
            "vocab_size": len(meta.get("tokenizer.ggml.tokens", [])) or g("vocab_size"),
        }

        # Tensor infos: name, ndims, dims[], ggml_type, offset.
        block0 = []
        for _ in range(n_tensors):
            name = r.string()
            ndims = r.u32()
            dims = [r.u64() for _ in range(ndims)]
            ttype = r.u32()
            _off = r.u64()
            if name.startswith("blk.0.") or name in ("token_embd.weight", "output_norm.weight", "output.weight"):
                block0.append((name, dims, GGML_TYPE.get(ttype, str(ttype))))

    print(f"GGUF v{version}  tensors={n_tensors}  metadata_kv={n_kv}")
    print("── architecture ─────────────────────────────")
    for k, v in cfg.items():
        print(f"  {k:22} {v}")
    gqa = f"{cfg['head_count']}Q / {cfg['head_count_kv']}KV"
    print(f"  {'attention':22} GQA {gqa}  (group size {cfg['head_count'] // max(cfg['head_count_kv'], 1)})")
    print("── block 0 tensors (the first block we lower) ─")
    for name, dims, t in sorted(block0):
        print(f"  {t:5} {str(dims):18} {name}")
    return cfg


if __name__ == "__main__":
    path = sys.argv[1] if len(sys.argv) > 1 else "models/qwen3-8b/Qwen_Qwen3-8B-Q8_0.gguf"
    inspect(path)
