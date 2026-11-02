#!/usr/bin/env python3
"""The Hugging Face tokenizer libraries as an oracle for the Rust port of their encode (`model::hf::HfTokenizer`).

    hf-oracle.py encode <tokenizer-dir> <texts.json>   -> {"library": [[ids], ...], "library_plain": [[ids], ...]}
    hf-oracle.py split <cases.json>                    -> [[pieces], ...]   cases: [[pattern, behavior, invert, text], ...]
    hf-oracle.py nfc <texts.json>                      -> [nfc, ...]
    hf-oracle.py special <model-dir> <ignored.json>    -> {"add": {kind: bool}, "ids": {kind: id}}
    hf-oracle.py logits <model-dir> <tokens.json>      -> [logit, ...]   the last position's, after the tokens
    hf-oracle.py scaled <cases.json>                   -> [[f32 bits, ...], ...]
    hf-oracle.py tiktoken <tokenizer-dir> <texts.json> -> [[ids], ...]
    hf-oracle.py tiktoken-classes <patterns.json>      -> [[[first, last], ...], ...]

`encode` runs the `tokenizers` library's `Tokenizer` over the directory's `tokenizer.json`, with and without the special
tokens the post-processor adds (transformers' tokenizer classes may rebuild that pipeline from their own defaults; the
port is of the file). `split` runs a `Split` pre-tokenizer with an Oniguruma `Regex`; `nfc` the library's NFC
normalizer; `special` the converter's own `gguf.SpecialVocab` (the pinned llama.cpp's gguf-py) over a model directory:
the special-token ids and the add flags it reads from `tokenizer.json`, `tokenizer_config.json` and `config.json`.
`logits` runs transformers' own forward of the directory's model in float64 (its experts eager, as the grouped kernel
takes no float64, and its attention eager, the one form that takes every model's score cap, its softmax kept at the
inputs' precision where the eager forms round it to float32) over the token ids and returns the last position's
logits — the model's maintainers' computation, the oracle the f64 reference is held to.
`scaled` runs the converter's own `dequant_simple` (read out of the pinned llama.cpp's `conversion/base.py`), which
turns an FP8 checkpoint's weights and their scales into the f32 it writes, over cases `{weight, weight_dtype,
weight_dims, scale, scale_dtype, scale_dims, block}` (bytes in hex). Runs in a Python with tokenizers, and torch and
transformers for `logits` and `scaled` (LOGOS_PYTHON).

`tiktoken` encodes as a tiktoken tokenizer's own code (Kimi K2's `tokenization_kimi.py`) encodes, without running that
code: the `tiktoken` library over the directory's `tiktoken.model`, the pattern and reserved count read out of the code's
text, the reserved special tokens named by `tokenizer_config.json`, the text cut every 400,000 characters and at runs
of 25,000 spaces or non-spaces, every special token allowed. `tiktoken-classes` gives, for each pattern, the codepoints
`tiktoken` matches with it alone (every one but the surrogates), as runs.
"""
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def float64_model(path):
    """The directory's model in float64 throughout: experts and attention eager, and every softmax a model asks for in
    float32 (the eager attention's, a router's) kept at its input's precision instead."""
    import torch
    import transformers

    softmax = torch.nn.functional.softmax

    def kept(x, dim=None, _stacklevel=3, dtype=None):
        if dtype == torch.float32 and x.dtype == torch.float64:
            dtype = torch.float64
        return softmax(x, dim=dim, _stacklevel=_stacklevel, dtype=dtype)

    torch.nn.functional.softmax = kept
    model = transformers.AutoModelForCausalLM.from_pretrained(
        path, dtype=torch.float64, experts_implementation="eager", attn_implementation="eager"
    )
    model.eval()
    return model


def main():
    cmd = sys.argv[1]
    if cmd == "encode":
        from tokenizers import Tokenizer
        directory, texts = sys.argv[2], json.load(open(sys.argv[3]))
        library = Tokenizer.from_file(f"{directory}/tokenizer.json")
        out = {
            "library": [library.encode(t).ids for t in texts],
            "library_plain": [library.encode(t, add_special_tokens=False).ids for t in texts],
        }
    elif cmd == "split":
        from tokenizers import Regex, pre_tokenizers
        out = []
        for pattern, behavior, invert, text in json.load(open(sys.argv[2])):
            split = pre_tokenizers.Split(Regex(pattern), behavior, invert=invert)
            out.append([piece for piece, _ in split.pre_tokenize_str(text)])
    elif cmd == "nfc":
        from tokenizers import normalizers
        nfc = normalizers.NFC()
        out = [nfc.normalize_str(t) for t in json.load(open(sys.argv[2]))]
    elif cmd == "special":
        sys.path.insert(0, str(ROOT / "external" / "competitors" / "llama.cpp" / "gguf-py"))
        import gguf
        vocab = gguf.SpecialVocab(Path(sys.argv[2]), load_merges=True)
        out = {"add": vocab.add_special_token, "ids": vocab.special_token_ids}
    elif cmd == "scaled":
        import re
        import textwrap
        from typing import Sequence
        import torch
        source = (ROOT / "external" / "competitors" / "llama.cpp" / "conversion" / "base.py").read_text()
        found = re.search(r"^( *)def dequant_simple\(.*?(?=^\1def |^\1#)", source, re.S | re.M)
        scope = {"torch": torch, "Tensor": torch.Tensor, "Sequence": Sequence}
        exec(textwrap.dedent(found.group(0)), scope)
        tensor = lambda hex_, dtype, dims: torch.frombuffer(bytearray.fromhex(hex_), dtype=torch.uint8).view(getattr(torch, dtype)).reshape(dims)
        out = []
        for case in json.load(open(sys.argv[2])):
            weight = tensor(case["weight"], case["weight_dtype"], case["weight_dims"])
            scale = tensor(case["scale"], case["scale_dtype"], case["scale_dims"])
            result = scope["dequant_simple"](weight, scale, case["block"]).contiguous()
            out.append([b & 0xFFFFFFFF for b in result.view(torch.int32).flatten().tolist()])
    elif cmd == "tiktoken":
        import re
        import tiktoken
        from tiktoken.load import load_tiktoken_bpe
        directory = Path(sys.argv[2])
        config = json.loads((directory / "tokenizer_config.json").read_text())
        auto = config["auto_map"]["AutoTokenizer"]
        module = (auto[0] if isinstance(auto, list) else auto).split(".")[0]
        code = (directory / f"{module}.py").read_text()
        listed = code[code.index('pat_str = "|".join('):]
        listed = listed[:listed.index("])")]
        pattern = "|".join(re.findall(r'r"""(.*?)"""', listed, re.S))
        reserved = int(re.search(r"^\s*num_reserved_special_tokens = (\d+)", code, re.M).group(1))
        ranks = load_tiktoken_bpe(str(directory / "tiktoken.model"))
        named = {int(i): t["content"] for i, t in config.get("added_tokens_decoder", {}).items()}
        base = len(ranks)
        specials = {named.get(i, f"<|reserved_token_{i}|>"): i for i in range(base, base + reserved)}
        enc = tiktoken.Encoding(name="tiktoken.model", pat_str=pattern, mergeable_ranks=ranks, special_tokens=specials)

        def runs(s, most):
            length, space, start = 0, (s[0].isspace() if s else False), 0
            for i, c in enumerate(s):
                now = c.isspace()
                if space ^ now:
                    length, space = 1, now
                else:
                    length += 1
                    if length > most:
                        yield s[start:i]
                        start, length = i, 1
            yield s[start:]

        out = []
        for text in json.load(open(sys.argv[3])):
            ids = []
            for i in range(0, len(text), 400_000):
                for piece in runs(text[i:i + 400_000], 25_000):
                    ids.extend(enc.encode(piece, allowed_special="all"))
            out.append(ids)
    elif cmd == "tiktoken-classes":
        import tiktoken
        ranks = {bytes([b]): b for b in range(256)}
        everything = "".join(chr(c) for c in range(0x110000) if not 0xD800 <= c <= 0xDFFF)
        out = []
        for pattern in json.load(open(sys.argv[2])):
            enc = tiktoken.Encoding(name="classes", pat_str=pattern, mergeable_ranks=ranks, special_tokens={})
            members = []
            for i in range(0, len(everything), 100_000):
                members.append(bytes(enc.encode(everything[i:i + 100_000], disallowed_special=())).decode("utf-8"))
            found = []
            for c in map(ord, "".join(members)):
                if found and found[-1][1] == c - 1:
                    found[-1][1] = c
                else:
                    found.append([c, c])
            out.append(found)
    elif cmd == "logits":
        import torch
        model = float64_model(sys.argv[2])
        tokens = json.load(open(sys.argv[3]))
        with torch.no_grad():
            out = model(torch.tensor([tokens]))
        out = [float(x) for x in out.logits[0, -1]]
    elif cmd == "logits-at":
        # One pass over the prompt; a position's logits are the prefix's own (the model is causal).
        import torch
        model = float64_model(sys.argv[2])
        request = json.load(open(sys.argv[3]))
        with torch.no_grad():
            logits = model(torch.tensor([request["tokens"]])).logits[0]
        out = [[float(x) for x in logits[p]] for p in request["positions"]]
    else:
        sys.exit(f"unknown command {cmd}")
    json.dump(out, sys.stdout, ensure_ascii=False)


if __name__ == "__main__":
    main()
