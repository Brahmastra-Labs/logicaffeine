"""vLLM at batch 1 on the bring-up scoreboard's protocol, run inside vLLM's own environment by
scripts/accel/model_scoreboard.py: one 512-token prefill, and 128 tokens decoded after a 6-token prompt, greedy with the
end of text ignored, prefix caching off so no prompt is served from cache; two warm-ups, then N timed runs of each.

    <vllm python> engine_bench_vllm.py <checkpoint dir> [--quantization Q] [--runs 5] [--max-batched-tokens T]
        [--gpu-memory F] [--batch B] [--prompt t,t,…] [--golden golden.json] [--out result.json]

`--prompt` gives the six-token prompt's tokens (the scoreboard passes its own, so both sides decode after the same
tokens); `2,651,6077,280,7239,317` otherwise.

`--batch B` measures decode at concurrency B instead, as `model_lane --bench --batch=<B>` does ours: B requests at once
(`max_num_seqs` B), sequence b's prompt the 6 tokens each moved 7919 b on modulo the vocabulary, each generating 129;
the decode rate is every sequence's 128 tokens a second. By the wall, B 128 / (the B generating 129 - the B generating
1); by the engine's clock, B 128 / (the last request's last token - the last request's first), the window in which every
sequence decodes, the shortest the timestamps allow. No prefill is timed at a batch.

`--golden G` generates instead, for the scoreboard's quality column: for each prompt of the golden G (a model's
benchmarks/models/<model>/golden-f32.json), its tokens as given, greedy for its continuation's length with the end of text
ignored, one request at a time; the record keeps each prompt's generated tokens and its first generated position's 20
most likely tokens with their log-probabilities.

Each run is timed two ways. By the wall: prefill 512 / (a 512-token prompt generating 1 token), decode 128 / (the 6-token
prompt generating 129 - generating 1). By the engine's own clock (the request's `metrics`): prefill 512 / (its first
token - the moment it was scheduled), decode 128 / (its last token - its first), free of the frontend's cost per
request, which on a shared box is tens of milliseconds. Prints `RESULT <json>`.
"""
import argparse
import json
import statistics
import time


def main():
    from vllm import LLM, SamplingParams
    import vllm

    ap = argparse.ArgumentParser()
    ap.add_argument("model")
    ap.add_argument("--quantization")
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--max-batched-tokens", type=int)
    ap.add_argument("--gpu-memory", type=float, default=0.88)
    ap.add_argument("--batch", type=int, default=1)
    ap.add_argument("--prompt", default="2,651,6077,280,7239,317")
    ap.add_argument("--golden")
    ap.add_argument("--out")
    a = ap.parse_args()
    extra = {"max_num_batched_tokens": a.max_batched_tokens} if a.max_batched_tokens else {}
    llm = LLM(model=a.model, quantization=a.quantization, max_model_len=1024, max_num_seqs=a.batch, gpu_memory_utilization=a.gpu_memory,
              enable_prefix_caching=False, seed=0, disable_log_stats=False, **extra)
    vocab = llm.llm_engine.model_config.get_vocab_size()
    long_prompt = [(100 + (i * 37) % 9000) % vocab for i in range(512)]
    short_prompt = [int(t) % vocab for t in a.prompt.split(",")]

    def timed(prompt, n):
        sp = SamplingParams(max_tokens=n, temperature=0.0, ignore_eos=True)
        t0 = time.perf_counter()
        out = llm.generate([{"prompt_token_ids": prompt}], sp, use_tqdm=False)
        wall = time.perf_counter() - t0
        assert len(out[0].outputs[0].token_ids) == n, "every token generated"
        m = out[0].metrics
        assert m is not None, "the engine reports the request's timestamps"
        return wall, m.first_token_ts - m.scheduled_ts, m.last_token_ts - m.first_token_ts

    if a.golden:
        golden(llm, a)
        return
    if a.batch > 1:
        batch(llm, a, vocab, short_prompt)
        return
    for _ in range(2):
        timed(long_prompt, 1)
        timed(short_prompt, 129)
        timed(short_prompt, 1)
    prefill, decode, prefill_engine, decode_engine = [], [], [], []
    for _ in range(a.runs):
        long1 = timed(long_prompt, 1)
        one = timed(short_prompt, 1)
        full = timed(short_prompt, 129)
        prefill.append(512 / long1[0])
        decode.append(128 / (full[0] - one[0]))
        prefill_engine.append(512 / long1[1])
        decode_engine.append(128 / full[2])
    rec = {
        "engine": "vllm",
        "version": vllm.__version__,
        "checkpoint": a.model,
        "quantization": a.quantization,
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


def golden(llm, a):
    """Each golden prompt's tokens generated greedily for its continuation's length, and the first position's top 20."""
    import vllm
    from vllm import SamplingParams

    with open(a.golden) as f:
        g = json.load(f)
    prompts = []
    for p in g["prompts"]:
        n = len(p["continuation"])
        sp = SamplingParams(max_tokens=n, temperature=0.0, ignore_eos=True, logprobs=20)
        out = llm.generate([{"prompt_token_ids": p["tokens"]}], sp, use_tqdm=False)[0].outputs[0]
        assert len(out.token_ids) == n, "every token generated"
        first = sorted(((int(t), float(lp.logprob)) for t, lp in out.logprobs[0].items()), key=lambda x: -x[1])
        prompts.append({"tokens": list(out.token_ids), "first_top": first})
    rec = {"engine": "vllm", "version": vllm.__version__, "checkpoint": a.model, "quantization": a.quantization, "golden": a.golden, "prompts": prompts}
    print("RESULT " + json.dumps(rec))
    if a.out:
        with open(a.out, "w") as f:
            json.dump(rec, f, indent=1)


def batch(llm, a, vocab, short_prompt):
    """Decode at concurrency `a.batch`: the batch's prompts generating 129 and generating 1, two warm-ups, N runs."""
    import vllm
    from vllm import SamplingParams

    prompts = [{"prompt_token_ids": [(t + 7919 * b) % vocab for t in short_prompt]} for b in range(a.batch)]

    def timed(n):
        sp = SamplingParams(max_tokens=n, temperature=0.0, ignore_eos=True)
        t0 = time.perf_counter()
        out = llm.generate(prompts, sp, use_tqdm=False)
        wall = time.perf_counter() - t0
        assert len(out) == a.batch and all(len(o.outputs[0].token_ids) == n for o in out), "every sequence generates every token"
        assert all(o.metrics is not None for o in out), "the engine reports every request's timestamps"
        return wall, max(o.metrics.last_token_ts for o in out) - max(o.metrics.first_token_ts for o in out)

    for _ in range(2):
        timed(129)
        timed(1)
    decode, decode_engine = [], []
    for _ in range(a.runs):
        one = timed(1)
        full = timed(129)
        decode.append(a.batch * 128 / (full[0] - one[0]))
        decode_engine.append(a.batch * 128 / full[1])
    rec = {
        "engine": "vllm",
        "version": vllm.__version__,
        "checkpoint": a.model,
        "quantization": a.quantization,
        "batch": a.batch,
        "decode_tok_s": decode,
        "decode_engine_tok_s": decode_engine,
        "decode_mean": statistics.mean(decode),
    }
    print("RESULT " + json.dumps(rec))
    if a.out:
        with open(a.out, "w") as f:
            json.dump(rec, f, indent=1)


if __name__ == "__main__":
    main()
