#!/usr/bin/env python3
"""Read a GGUF file's header straight off Hugging Face with an HTTP range request.

A GGUF stores its metadata and full tensor table at the START of the file, so the architecture of a
29 GB checkpoint can be read by pulling its first few tens of MB. That is how the Qwen3.8-27B lowering
was specified before a single GPU-hour was rented: no download, no pod, no guessing.

    python3 scripts/accel/gguf_remote_header.py bartowski/Qwen3.8-27B-GGUF Qwen3.8-27B-Q8_0.gguf
    python3 scripts/accel/gguf_remote_header.py --json arch.json <repo> <file> [<repo> <file> ...]
    python3 scripts/accel/gguf_remote_header.py --local models/qwen3-8b/Qwen_Qwen3-8B-Q8_0.gguf
    python3 scripts/accel/gguf_remote_header.py --tensors --json h.json <repo> <file>
    python3 scripts/accel/gguf_remote_header.py --local --ledger benchmarks/silicon/decode/headers/<name>.json \
        models/<dir>/<file>.gguf

Tensor names are grouped by collapsing digit runs (`blk.7.attn_q` -> `blk.N.attn_q`) so a 866-tensor
checkpoint prints as ~25 lines of distinct shapes.

`--tensors` also writes the whole tensor table (name, dims, ggml type id, data offset), the offset where
tensor data begins and the file's size, so every tensor's byte count can be checked against the file
itself: the gaps between consecutive offsets, and the last tensor against the end of the file.

`--ledger OUT` writes one checkpoint's header as the decode pricer reads it (`GgufHeader::read_ledger`): the
tensor table as `--tensors` dumps it under `header`, and under `_source` the file it was read from, as the
command was given it (run it from the repository root with a relative path), how, by which tool, on which
date, and, for a local file, the SHA-256 of the whole file, so a ledger names the exact checkpoint it describes.
"""
import argparse, datetime, hashlib, json, os, re, struct, sys, tempfile, urllib.request

U8, I8, U16, I16, U32, I32, F32, BOOL, STRING, ARRAY, U64, I64, F64 = range(13)
# enum ggml_type, ggml/include/ggml.h (ids 4, 5 and 31-33, 36-38 are retired).
GGML = {0: "F32", 1: "F16", 2: "Q4_0", 3: "Q4_1", 6: "Q5_0", 7: "Q5_1", 8: "Q8_0", 9: "Q8_1", 10: "Q2_K",
        11: "Q3_K", 12: "Q4_K", 13: "Q5_K", 14: "Q6_K", 15: "Q8_K", 16: "IQ2_XXS", 17: "IQ2_XS", 18: "IQ3_XXS",
        19: "IQ1_S", 20: "IQ4_NL", 21: "IQ3_S", 22: "IQ2_S", 23: "IQ4_XS", 24: "I8", 25: "I16", 26: "I32",
        27: "I64", 28: "F64", 29: "IQ1_M", 30: "BF16", 34: "TQ1_0", 35: "TQ2_0", 39: "MXFP4", 40: "NVFP4",
        41: "Q1_0", 42: "Q2_0"}
SCALAR = {U8: ("<B", 1), I8: ("<b", 1), U16: ("<H", 2), I16: ("<h", 2), U32: ("<I", 4),
          I32: ("<i", 4), F32: ("<f", 4), BOOL: ("<?", 1), U64: ("<Q", 8), I64: ("<q", 8), F64: ("<d", 8)}
# Arrays longer than this are summarized, not materialized: a 248320-entry vocab is not architecture.
BULK = 3000
# What a `--ledger` file is, as every header ledger under benchmarks/silicon/decode/headers/ states it.
LEDGER_WHAT = ("The GGUF header of one checkpoint: its metadata and its whole tensor table (name, dims, ggml type id, "
               "data offset), with the offset where tensor data begins and the size of the file. Every tensor's "
               "byte count is checkable against the file itself.")


class Header:
    def __init__(self, f):
        self.f = f
        if f.read(4) != b"GGUF":
            raise ValueError("not a GGUF file")
        self.version = self._u32()
        n_tensors, n_kv = self._u64(), self._u64()
        self.kv = {}
        for _ in range(n_kv):
            k = self._str()
            self.kv[k] = self._value(self._u32())
        self.tensors = []
        self.table = []
        for _ in range(n_tensors):
            name = self._str()
            dims = [self._u64() for _ in range(self._u32())]
            ty = self._u32()
            offset = self._u64()
            self.tensors.append((name, dims, GGML.get(ty, str(ty))))
            self.table.append({"name": name, "dims": dims, "type": ty, "offset": offset})
        align = self.kv.get("general.alignment", 32)
        self.data_offset = (self.f.tell() + align - 1) // align * align

    def _u32(self): return struct.unpack("<I", self.f.read(4))[0]
    def _u64(self): return struct.unpack("<Q", self.f.read(8))[0]
    def _str(self): return self.f.read(self._u64()).decode("utf-8", "replace")

    def _value(self, t):
        if t == STRING:
            return self._str()
        if t == ARRAY:
            et, n = self._u32(), self._u64()
            if et == STRING:
                if n > BULK:
                    for _ in range(n):
                        self.f.seek(self._u64(), 1)
                    return f"<{n} strings>"
                return [self._str() for _ in range(n)]
            fmt, size = SCALAR[et]
            if n > BULK:
                self.f.seek(size * n, 1)
                return f"<{n} values>"
            return [struct.unpack(fmt, self.f.read(size))[0] for _ in range(n)]
        fmt, size = SCALAR[t]
        return struct.unpack(fmt, self.f.read(size))[0]

    def patterns(self):
        """Distinct tensor shapes, digit-runs collapsed to N, in first-appearance order."""
        out = {}
        for name, dims, ty in self.tensors:
            out.setdefault(re.sub(r"\b\d+\b", "N", name), []).append((name, dims, ty))
        return out


def fetch(repo, filename, nbytes):
    """The first `nbytes` of the file, and the file's whole size from the response's Content-Range."""
    url = f"https://huggingface.co/{repo}/resolve/main/{filename}"
    req = urllib.request.Request(url, headers={"Range": f"bytes=0-{nbytes - 1}"})
    tmp = tempfile.NamedTemporaryFile(suffix=".gguf", delete=False)
    with urllib.request.urlopen(req, timeout=300) as r:
        size = int(r.headers["Content-Range"].rsplit("/", 1)[1])
        tmp.write(r.read())
    tmp.close()
    return tmp.name, size


def report(label, h):
    print(f"=== {label} ===")
    print(f"  gguf v{h.version}  {len(h.tensors)} tensors  {len(h.kv)} metadata keys")
    for k, v in h.kv.items():
        s = str(v).replace("\n", " ")
        print(f"    {k} = {s[:110]}")
    print(f"  --- {len(h.tensors)} tensors, {len(h.patterns())} distinct shapes ---")
    for g, items in h.patterns().items():
        _, dims, ty = items[0]
        print(f"    {len(items):>4}x  {g:<44} {dims} {ty}")
    print()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--json", help="write a machine-readable dump here")
    ap.add_argument("--bytes", type=int, default=48 << 20, help="header bytes to pull (default 48 MiB)")
    ap.add_argument("--local", action="store_true", help="args are local paths, not repo/file pairs")
    ap.add_argument("--tensors", action="store_true",
                    help="also dump the whole tensor table, the data offset and the file size")
    ap.add_argument("--ledger", help="write one checkpoint's header ledger here (implies --tensors)")
    ap.add_argument("args", nargs="+")
    a = ap.parse_args()
    if a.ledger:
        a.tensors = True
        if len(a.args) != (1 if a.local else 2):
            sys.exit("--ledger writes one checkpoint: one local path, or one <repo> <file> pair")

    dump, tmps = {}, []
    try:
        if a.local:
            targets = [(p, p, os.path.getsize(p)) for p in a.args]
        else:
            if len(a.args) % 2:
                sys.exit("expected <repo> <file> pairs")
            targets = []
            for i in range(0, len(a.args), 2):
                path, size = fetch(a.args[i], a.args[i + 1], a.bytes)
                targets.append((f"{a.args[i]}/{a.args[i+1]}", path, size))
            tmps = [p for _, p, _ in targets]
        for label, path, size in targets:
            with open(path, "rb") as f:
                h = Header(f)
            report(label, h)
            dump[label] = {
                "gguf_version": h.version,
                "n_tensors": len(h.tensors),
                "metadata": {k: v for k, v in h.kv.items()},
                "tensor_patterns": {g: {"count": len(i), "example": i[0][0], "dims": i[0][1], "type": i[0][2]}
                                    for g, i in h.patterns().items()},
            }
            if a.tensors:
                dump[label]["file_bytes"] = size
                dump[label]["data_offset"] = h.data_offset
                dump[label]["tensors"] = h.table
        if a.json:
            with open(a.json, "w") as f:
                json.dump(dump, f, indent=2, default=str)
            print(f"wrote {a.json}")
        if a.ledger:
            label, path, _ = targets[0]
            source = {
                "file": label,
                "method": "read from the local file" if a.local else "an HTTP range request for the file's head",
                "tool": "scripts/accel/gguf_remote_header.py --ledger",
                "read_on": datetime.date.today().isoformat(),
            }
            if a.local:
                digest = hashlib.sha256()
                with open(path, "rb") as f:
                    for chunk in iter(lambda: f.read(1 << 24), b""):
                        digest.update(chunk)
                source["sha256"] = digest.hexdigest()
            ledger = {"_what": LEDGER_WHAT, "_source": source, "header": dump[label]}
            with open(a.ledger, "w") as f:
                json.dump(ledger, f, indent=2, default=str)
                f.write("\n")
            print(f"wrote {a.ledger}")
    finally:
        for p in tmps:
            os.unlink(p)


if __name__ == "__main__":
    main()
