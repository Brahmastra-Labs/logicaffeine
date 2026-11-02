#!/usr/bin/env python3
"""transformers' Qwen3.8-Flash-Next (`qwen4_exp`) as an oracle for the Rust model code.

    qwen4exp-oracle.py ngram <config.json> <cases.json>   -> {rows: [[[row, ...] per position] per case],
                                                             multipliers, primes, rows_before_padding}

`ngram` builds the checkpoint's hashed n-gram embedding module (`Qwen4ExpTextNGramEmbedding`) on the meta device, so
its table of 320 million rows takes no memory, swaps its embedding lookup for one that returns the row ids, and runs
each case: `{"context": [previous tokens], "tokens": [tokens]}` (an empty context is the module's first call, its
history filled with the end-of-sequence token). Runs in a Python with transformers (LOGOS_PYTHON).
"""
import json
import sys

import torch
from transformers.models.qwen4_exp import configuration_qwen4_exp, modeling_qwen4_exp


class Rows(torch.nn.Module):
    """An embedding lookup that returns the row ids it is asked for."""

    def __init__(self):
        super().__init__()
        self.weight = torch.nn.Parameter(torch.zeros(1))

    def forward(self, ids):
        return ids


class Context:
    """The cache slot the module keeps its last tokens in, in the shapes both transformers revisions read it."""

    def __init__(self, context):
        self.context = context
        slot = type("Layer", (), {"conv_states": {2: context}})()
        self.layers = {i: slot for i in range(256)}

    def has_previous_state(self, layer_idx, state_idx):
        return True

    def get_conv_state(self, layer_idx, state_idx):
        return self.context

    def update_conv_state(self, *args, **kwargs):
        return None


def main():
    if sys.argv[1] != "ngram":
        sys.exit(__doc__)
    config = json.load(open(sys.argv[2]))
    text = configuration_qwen4_exp.Qwen4ExpTextConfig(**config.get("text_config", config))
    with torch.device("meta"):
        module = modeling_qwen4_exp.Qwen4ExpTextNGramEmbedding(text, text.ple_embed_dim, text.ple_layer_ids[0], 0)
    multipliers = modeling_qwen4_exp._build_layer_multipliers(text.vocab_size, text.ngram_size, 0, text.seed)
    module.layer_multipliers = multipliers
    module.ngram_heads_vocab_sizes = torch.tensor(module.head_vocab_sizes, dtype=torch.long)
    module.ngram_heads_offsets = torch.tensor(module.head_offsets, dtype=torch.long)
    module.ngram_embedding = Rows()
    out = []
    for case in json.load(open(sys.argv[3])):
        tokens = torch.tensor([case["tokens"]], dtype=torch.long)
        cache = Context(torch.tensor([case["context"]], dtype=torch.long)) if case["context"] else None
        rows = module(tokens, cache).reshape(len(case["tokens"]), -1)
        out.append(rows.tolist())
    json.dump({"rows": out, "multipliers": multipliers.tolist(), "primes": module.head_vocab_sizes,
               "rows_before_padding": module.total_vocab_size}, sys.stdout)


if __name__ == "__main__":
    main()
