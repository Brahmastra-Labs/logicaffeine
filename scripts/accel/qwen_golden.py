#!/usr/bin/env python3
"""Golden next-token reference for the Qwen3-8B Q8_0 checkpoint, via llama.cpp (CPU).

Emits the exact prompt token ids and the greedy (argmax) next-token id, so the Rust
forward can be fed identical tokens and validated to predict the same next token.
Writes `benchmarks/silicon/qwen_golden.json` (tiny, committed).

Usage: python3 scripts/accel/qwen_golden.py [prompt]
"""
import json
import sys

import numpy as np
from llama_cpp import Llama

MODEL = "models/qwen3-8b/Qwen_Qwen3-8B-Q8_0.gguf"
PROMPT = sys.argv[1] if len(sys.argv) > 1 else "The capital of France is"

N_GEN = 8  # greedy continuation length

llm = Llama(model_path=MODEL, n_ctx=256, logits_all=True, verbose=False)
toks = llm.tokenize(PROMPT.encode("utf-8"), add_bos=True, special=False)


def greedy_next(all_toks):
    llm.reset()
    llm.eval(all_toks)
    scores = np.asarray(llm.scores, dtype=np.float64)
    if scores.ndim == 1:
        scores = scores.reshape(-1, llm.n_vocab())
    logits = scores[llm.n_tokens - 1]
    return int(np.argmax(logits)), logits


nxt, logits = greedy_next(list(toks))
order = np.argsort(-logits)[:5]

# Greedy autoregressive continuation (re-eval the growing sequence each step — matches our KV-cache-free forward).
gen = []
seq = list(toks)
for _ in range(N_GEN):
    g, _ = greedy_next(seq)
    gen.append(g)
    seq.append(g)

out = {
    "model": MODEL,
    "prompt": PROMPT,
    "prompt_tokens": [int(t) for t in toks],
    "next_token_id": nxt,
    "next_token_str": llm.detokenize([nxt]).decode("utf-8", "replace"),
    "top5": [[int(i), float(logits[i])] for i in order],
    "generated_tokens": gen,
    "generated_text": llm.detokenize(gen).decode("utf-8", "replace"),
}
with open("benchmarks/silicon/qwen_golden.json", "w") as f:
    json.dump(out, f, indent=2)
print(json.dumps(out, indent=2))
