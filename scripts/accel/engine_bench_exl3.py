"""ExLlamaV3 at batch 1 on the bring-up scoreboard's protocol, run inside its own environment by
scripts/accel/model_scoreboard.py, through its own generator (what its users run): one 512-token prefill, and 128 tokens
decoded after a 6-token prompt, greedy with the end of text ignored; two warm-ups, then N timed runs of each. Every
prompt differs run to run, so the generator's page cache never serves one from cache.

    <exllamav3 python> engine_bench_exl3.py <exl3 checkpoint dir> [--runs 5] [--golden golden.json] [--out result.json]

`--golden G` generates instead, for the scoreboard's quality column: for each prompt of the golden G (a model's
benchmarks/models/<model>/golden-f32.json), its tokens as given, greedy for its continuation's length with no stop condition,
one job at a time; the record keeps each prompt's generated tokens and its first generated position's 20 most likely
tokens with their log-probabilities (from the job's pre-sampling logits).

Each run is timed two ways. By the wall: prefill 512 / (a 512-token prompt generating 1 token), decode 128 / (a 6-token
prompt generating 129 - generating 1). By the generator's own clock (the job's timestamps): prefill 512 / (its first
token - its prefill's start), decode 128 / (its last token - its first). Prints `RESULT <json>`.
"""
import argparse
import json
import statistics
import time
from importlib.metadata import version

import torch
from exllamav3 import Cache, Config, Generator, Job, Model, Tokenizer
from exllamav3.generator.sampler import ArgmaxSampler


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("model")
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--golden")
    ap.add_argument("--out")
    a = ap.parse_args()
    config = Config.from_directory(a.model)
    model = Model.from_config(config)
    cache = Cache(model, max_num_tokens=4096)
    model.load()
    tokenizer = Tokenizer.from_config(config)
    gen = Generator(model=model, cache=cache, tokenizer=tokenizer, max_batch_size=1)
    vocab = config.vocab_size
    serial = [0]
    if a.golden:
        golden(gen, a)
        return

    def prompt(n):
        serial[0] += 1
        s = serial[0]
        return torch.tensor([[(100 + (i * 37 + s * 7919) % 9000) % vocab for i in range(n)]], dtype=torch.long)

    def timed(n_prompt, n_new):
        job = Job(input_ids=prompt(n_prompt), max_new_tokens=n_new, min_new_tokens=n_new, sampler=ArgmaxSampler(),
                  stop_conditions=[])
        torch.cuda.synchronize()
        t0 = time.perf_counter()
        gen.enqueue(job)
        while gen.num_remaining_jobs():
            gen.iterate()
        torch.cuda.synchronize()
        wall = time.perf_counter() - t0
        assert job.new_tokens == n_new, ("every token generated", job.new_tokens, n_new)
        return wall, job.time_first_token - job.time_first_prefill, job.time_last_token - job.time_first_token

    for _ in range(2):
        timed(512, 1)
        timed(6, 129)
        timed(6, 1)
    prefill, decode, prefill_engine, decode_engine = [], [], [], []
    for _ in range(a.runs):
        long1 = timed(512, 1)
        one = timed(6, 1)
        full = timed(6, 129)
        prefill.append(512 / long1[0])
        decode.append(128 / (full[0] - one[0]))
        prefill_engine.append(512 / long1[1])
        decode_engine.append(128 / full[2])
    rec = {
        "engine": "exllamav3",
        "version": version("exllamav3"),
        "checkpoint": a.model,
        "prefill_tok_s": prefill,
        "decode_tok_s": decode,
        "prefill_engine_tok_s": prefill_engine,
        "decode_engine_tok_s": decode_engine,
        "prefill_mean": statistics.mean(prefill),
        "decode_mean": statistics.mean(decode),
    }
    print("RESULT " + json.dumps(rec))
    if a.out:
        with open(a.out, "w") as f:
            json.dump(rec, f, indent=1)


def golden(gen, a):
    """Each golden prompt's tokens generated greedily for its continuation's length, and the first position's top 20."""
    with open(a.golden) as f:
        g = json.load(f)
    prompts = []
    for p in g["prompts"]:
        n = len(p["continuation"])
        ids = torch.tensor([p["tokens"]], dtype=torch.long)
        job = Job(input_ids=ids, max_new_tokens=n, min_new_tokens=n, sampler=ArgmaxSampler(), stop_conditions=[], return_logits=True)
        gen.enqueue(job)
        tokens, logits = [], []
        while gen.num_remaining_jobs():
            for r in gen.iterate():
                if r.get("job") is job and "token_ids" in r:
                    tokens.extend(int(t) for t in r["token_ids"][0].tolist())
                    logits.append(r["logits"][0].float())
        assert len(tokens) == n, ("every token generated", len(tokens))
        first = torch.log_softmax(torch.cat(logits)[0], dim=-1)
        top = torch.topk(first, 20)
        prompts.append({"tokens": tokens, "first_top": [[int(i), float(v)] for i, v in zip(top.indices.tolist(), top.values.tolist())]})
    rec = {"engine": "exllamav3", "version": version("exllamav3"), "checkpoint": a.model, "golden": a.golden, "prompts": prompts}
    print("RESULT " + json.dumps(rec))
    if a.out:
        with open(a.out, "w") as f:
            json.dump(rec, f, indent=1)


if __name__ == "__main__":
    main()
