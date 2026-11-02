#!/usr/bin/env python3
"""The bring-up scoreboard: every local checkpoint that fits the GPU whole, the compiled lane against llama.cpp and the
serving engines, one model at a time in one boot, recorded as the ledger benchmarks/results/models/<gpu>_scoreboard.json.

    scripts/accel/model_scoreboard.py [--tag T] [--models m1,m2] [--engines vllm,exllamav3] [--repeats 5] [--ledger PATH]

For each model:
  * ours, short: `model_lane <gguf> --bench int8 128 <6 prompt tokens> 16 1024` -- the decode rate over 128 generated
    tokens after a 6-token prompt;
  * ours, long: `model_lane <gguf> --bench int8 16 <512 prompt tokens> 512 1024` -- the prefill rate of one 512-token chunk;
  * llama.cpp: `llama-bench -m <gguf> -p 512 -n 128 -r <repeats> -ngl 99 -o json` -- pp512 and tg128, each run's rate
    its tokens over its own nanoseconds (`samples_ns`, kept beside it), which llama-bench's mean and deviation are
    computed from; its printed `samples_ts` are rounded to six digits.
  * the serving engines, where the model has a checkpoint in the engine's own 8-bit weight form
    (models/<model>/engines/<form>): vLLM (scripts/accel/engine_bench_vllm.py) and ExLlamaV3
    (scripts/accel/engine_bench_exl3.py), each in its own environment (external/engines/<engine>/venv), each at batch 1
    on the same two prompts, every run timed by the wall and by the engine's own clock;
  * batches, at each count of sequences the model's batch fits the card at (BATCHES): ours, the lane's batch step
    (`model_lane --bench int8 129 <6 prompt tokens> 16 256 --batch=<B>`: sequence b's prompt the six tokens each moved
    7919·b on, every sequence's 128 decoded tokens a second), and vLLM at the same concurrency
    (`engine_bench_vllm.py --batch <B>`, the same prompts, on the wall's clock and its own).
Every side also records its agreement with the model's f32 golden (benchmarks/models/<model>/golden-f32.json: llama.cpp's
run of the checkpoint's weights in f32, its cache f32, on the CPU), over the golden's prompts as its tokens: each prompt
generated greedily for the golden's length, `top1` the steps that agree with the golden's continuation before the first
that does not, of `steps`, and `top_logit_gap` the mean over the prompts of how far the side's gap between the golden's
two top tokens at the first generated position sits from the golden's own (a difference of logits, the same from
log-probabilities). Ours from model_lane's golden run (int8, as its speed is measured); llama.cpp's from its own GPU run
(scripts/accel/model-golden.sh … device=cuda, as llama-bench runs it); each engine's from its bench's --golden run.
Both sides run one warm-up and then <repeats> timed runs, and the ledger keeps every run, so the mean and the sample
standard deviation are recomputed by whoever reads it (crates/logicaffeine_tests/tests/model_scoreboard_lock.rs). Each
row also keeps the box's one-minute load average before and after it, since the box is shared. The card's streaming
read ceiling is measured first in the same boot (scripts/accel/read_ceiling.cu, <repeats> runs), and each row records
the weight bytes its decode step reads a token and whether it routes through experts (`model_lane --plan`:
`Gguf::decode_read_bytes`), so the share of the ceiling a decode rate reaches is derived, never typed. The lane runs
llama.cpp's own activation precision (one int8 word a 32-block, its q8_1).

Every side runs with the card to itself (`run`): it starts once no other process holds the GPU (nvidia-smi's compute
processes), the card is sampled every CARD_POLL_S seconds while it runs, and a side another process joined (any
process not under the side's own) runs again once the card is free, its numbers having shared the card. Each process
waited for or that joined is named in the log by pid, binary, memory and systemd unit.

The ledger is refused (nothing written) when:
  * a run fails;
  * the card is held past CARD_WAIT_S, or another process joins every one of a side's runs;
  * our program's provenance line is not the compiled lane's;
  * a model has no program identity lines;
  * a side is missing.

llama.cpp is named by its pin in external/competitors/PINS.tsv. The binary's own build field reports the enclosing
checkout's commit, since the pinned tarball has no git history of its own. The raw output of every command goes to
logs/model-bringup/scoreboard-<tag>.log.
"""

import argparse
import datetime
import statistics
import glob
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import time

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
# The lane the scoreboard times: the executable its build reports writing (`build_lane`), set once that build has run.
LANE = None
LLAMA_BENCH = os.path.join(ROOT, "external", "competitors", "llama.cpp", "build", "bin", "llama-bench")
# The serving engines measured beside llama.cpp, and the bench each runs in its own environment.
ENGINES = {"vllm": "engine_bench_vllm.py", "exllamav3": "engine_bench_exl3.py"}
# Each model's checkpoints in an engine's own 8-bit weight form: the form's directory under models/<model>/engines/ and
# the flags the engine's bench takes. vLLM reads 8-bit weights as FP8 (Marlin on Ampere): RedHat's FP8 checkpoints, and
# Qwen3.5-0.8B's bf16 one quantized to FP8 as it loads; ExLlamaV3 its own 8.0 bits a weight. Qwen3-8B's FP8 profiling
# batch is held to 1024 tokens so its bf16 head fits beside the weights.
ENGINE_CHECKPOINTS = {
    "llama3.2-1b": {"vllm": ("vllm-fp8", []), "exllamav3": ("exl3-8.0bpw", [])},
    "gemma3-1b": {"vllm": ("vllm-fp8", [])},
    "qwen3.5-0.8b": {"vllm": ("bf16", ["--quantization", "fp8"])},
    "qwen3-8b": {"vllm": ("vllm-fp8", ["--max-batched-tokens", "1024", "--gpu-memory", "0.92"]), "exllamav3": ("exl3-8.0bpw", [])},
}
PINS = os.path.join(ROOT, "external", "competitors", "PINS.tsv")
COMPILED = "// LOGOS: compiled from the program's tensor statements — every launch is a stage lifted from the source."

MODELS = [
    ("llama3.2-1b", "models/llama3.2-1b/Llama-3.2-1B-Instruct-Q8_0.gguf"),
    ("gemma3-1b", "models/gemma3-1b/gemma-3-1b-it-Q8_0.gguf"),
    ("qwen3.5-0.8b", "models/qwen3.5-0.8b/Qwen3.5-0.8B-Q8_0.gguf"),
    ("qwen3-8b", "models/qwen3-8b/Qwen_Qwen3-8B-Q8_0.gguf"),
    ("deepseek-v2-lite", "models/deepseek-v2-lite/DeepSeek-V2-Lite-Chat.Q4_K_M.gguf"),
    ("gpt-oss-20b", "models/gpt-oss-20b/gpt-oss-20b-MXFP4.gguf"),
]
# The counts of sequences each model's batch is measured at: its batch step and its caches fit the card at these, and
# so does vLLM's engine, which stands beside every count. vLLM 0.32 does not start Qwen3-8B's FP8 checkpoint at 8
# sequences on the 12 GB card: beside the weights (8.99 GiB) and its cache, cuBLAS cannot allocate its own handle
# (CUBLAS_STATUS_ALLOC_FAILED in cublasCreate, logs/model-bringup/scoreboard-20261101-rerun.log).
BATCHES = {"llama3.2-1b": [4, 8, 16, 32, 64], "gemma3-1b": [4, 8, 16, 32, 64], "qwen3.5-0.8b": [4, 8, 16, 32, 64], "qwen3-8b": [4]}
BATCH_CTX = 256
SHORT_PROMPT = "100000,549,6077,280,7239,317"
LONG_PROMPT = ",".join(str(100 + (i * 37) % 9000) for i in range(512))
CTX = 1024


# The card is the scoreboard's alone while a side runs: it is sampled every CARD_POLL_S seconds, a side waits at most
# CARD_WAIT_S for another process to leave it, and a side another process joined runs again, at most CARD_RETRIES times.
CARD_POLL_S = 2.0
CARD_WAIT_S = 3600.0
CARD_RETRIES = 3


class Refused(Exception):
    pass


def card_processes():
    """The processes holding the GPU, as nvidia-smi lists them: (pid, binary, MiB) each."""
    p = subprocess.run(["nvidia-smi", "--query-compute-apps=pid,process_name,used_memory", "--format=csv,noheader,nounits"],
                       capture_output=True, text=True)
    if p.returncode != 0:
        raise Refused(f"nvidia-smi exited {p.returncode}: {(p.stdout + p.stderr).strip()}")
    held = []
    for line in p.stdout.splitlines():
        if line.strip():
            cols = [c.strip() for c in line.split(",")]
            held.append((int(cols[0]), ",".join(cols[1:-1]), cols[-1]))
    return held


def holder(pid, name, mib):
    """A process on the card as a log names it: its pid, binary and memory, and the systemd unit it runs under, so a
    peer's job is named by the unit its session posted."""
    unit = ""
    try:
        with open(f"/proc/{pid}/cgroup") as f:
            units = re.findall(r"([\w@.:-]+\.service)", f.read())
        if units:
            unit = f", unit {units[-1]}"
    except OSError:
        unit = ", gone"
    return f"pid {pid} ({name}, {mib} MiB{unit})"


def under(pid, root):
    """Whether process `pid` is `root` or runs under it, by /proc's parent links; None when `pid` is already gone."""
    first = True
    while pid != root:
        if pid <= 1:
            return False
        try:
            with open(f"/proc/{pid}/stat") as f:
                pid = int(f.read().rsplit(")", 1)[1].split()[1])
        except (OSError, ValueError, IndexError):
            return None if first else False
        first = False
    return True


def wait_for_the_card(log):
    """Return once no process holds the card, naming in the log each holder the side waited for; a card still held
    after CARD_WAIT_S is refused, its holders named."""
    deadline = time.monotonic() + CARD_WAIT_S
    named = set()
    while True:
        held = card_processes()
        if not held:
            return
        for h in held:
            if h[0] not in named:
                named.add(h[0])
                log.write(f"waiting for the card: {holder(*h)}\n")
                log.flush()
        if time.monotonic() >= deadline:
            raise Refused(f"the card is held past {CARD_WAIT_S:g} s by {'; '.join(holder(*h) for h in held)}")
        time.sleep(CARD_POLL_S)


def run(cmd, log, timeout, env=None, card=True):
    """Run one step, its output into the log. A step on the card (every step but a build) starts once the card is
    free and has it to itself: the card is sampled while it runs, and a step another process joined (any process not
    under the step's own) is run again from the start once the card is free, its numbers having shared the card."""
    joined = {}
    for _ in range(CARD_RETRIES + 1 if card else 1):
        if card:
            wait_for_the_card(log)
        log.write(f"$ {' '.join(cmd[:6])}{' …' if len(cmd) > 6 else ''}\n")
        log.flush()
        p = subprocess.Popen(cmd, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
        deadline = time.monotonic() + timeout
        joined, ours = {}, set()
        while True:
            try:
                out, err = p.communicate(timeout=CARD_POLL_S if card else timeout)
                break
            except subprocess.TimeoutExpired:
                if time.monotonic() >= deadline:
                    p.kill()
                    p.communicate()
                    raise subprocess.TimeoutExpired(cmd, timeout)
                for pid, name, mib in card_processes():
                    if pid in ours or pid in joined:
                        continue
                    if under(pid, p.pid):
                        ours.add(pid)
                    else:
                        joined[pid] = holder(pid, name, mib)
        log.write(out)
        log.write(err)
        log.flush()
        if joined:
            log.write(f"another process joined the card during this step: {'; '.join(joined.values())}; it runs again\n")
            log.flush()
            continue
        if p.returncode != 0:
            raise Refused(f"{cmd[0]} exited {p.returncode}")
        return out
    raise Refused(f"another process joined the card on each of {CARD_RETRIES + 1} runs of {cmd[0]}: {'; '.join(joined.values())}")


def build_lane(log):
    """Build the lane in release and return the executable cargo reports writing for it: the `compiler-artifact`
    message naming `model_lane`, wherever CARGO_TARGET_DIR or the cargo configuration put it. A build that names no
    such executable, or more than one, is refused: the scoreboard never times a binary its build did not write."""
    out = run(["cargo", "build", "--release", "-p", "logicaffeine-compile", "--example", "model_lane",
               "--message-format=json-render-diagnostics"], log, 7200, card=False)
    lanes = []
    for line in out.splitlines():
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get("reason") == "compiler-artifact" and m.get("target", {}).get("name") == "model_lane" and m.get("executable"):
            lanes.append(m["executable"])
    if len(lanes) != 1:
        raise Refused(f"the build reported {len(lanes)} model_lane executables, not one")
    return lanes[0]


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 24), b""):
            h.update(block)
    return h.hexdigest()


def lane_bench(gguf, n_gen, prompt, prefill, repeats, log):
    out = run([LANE, gguf, "--bench", "int8", str(n_gen), prompt, str(prefill), str(CTX), f"--repeats={repeats}"], log, 3600)
    provenance = [l[len("PROVENANCE "):] for l in out.splitlines() if l.startswith("PROVENANCE ")]
    programs = [l[len("PROGRAM "):] for l in out.splitlines() if l.startswith("PROGRAM ")]
    runs = [re.match(r"RUN (\d+) decode ([0-9.]+) tok/s  prefill ([0-9.]+) tok/s", l) for l in out.splitlines() if l.startswith("RUN ")]
    if provenance != [COMPILED]:
        raise Refused(f"{gguf}: the program's provenance is {provenance}, not the compiled lane's")
    if not programs or not all(re.fullmatch(r"LOGOS program: .+, sha256 [0-9a-f]{64}", p) for p in programs):
        raise Refused(f"{gguf}: no program identity lines ({programs})")
    if len(runs) != repeats or not all(runs):
        raise Refused(f"{gguf}: {len(runs)} runs, {repeats} asked")
    return {
        "n_gen": n_gen,
        "prompt_tokens": len(prompt.split(",")),
        "prefill_chunk": prefill,
        "ctx": CTX,
        "provenance": provenance[0],
        "programs": programs,
        "decode_tok_s": [float(m.group(2)) for m in runs],
        "prefill_tok_s": [float(m.group(3)) for m in runs],
    }


def lane_batch_bench(gguf, sequences, repeats, log):
    """Our batch step at `sequences` sequences: every sequence's decoded tokens a second, `repeats` runs."""
    out = run([LANE, gguf, "--bench", "int8", "129", SHORT_PROMPT, "16", str(BATCH_CTX), f"--batch={sequences}", f"--repeats={repeats}"], log, 3600)
    provenance = [l[len("PROVENANCE "):] for l in out.splitlines() if l.startswith("PROVENANCE ")]
    programs = [l[len("PROGRAM "):] for l in out.splitlines() if l.startswith("PROGRAM ")]
    runs = [re.match(r"RUN (\d+) decode ([0-9.]+) tok/s", l) for l in out.splitlines() if l.startswith("RUN ")]
    if provenance != [COMPILED]:
        raise Refused(f"{gguf}: the program's provenance is {provenance}, not the compiled lane's")
    if not any(p.startswith(f"LOGOS program: batch of {sequences} sequences, ctx {BATCH_CTX}, sha256 ") for p in programs):
        raise Refused(f"{gguf}: no batch of {sequences} among its programs ({programs})")
    if len(runs) != repeats or not all(runs):
        raise Refused(f"{gguf}: {len(runs)} batch runs, {repeats} asked")
    return {"ctx": BATCH_CTX, "provenance": provenance[0], "programs": programs, "decode_tok_s": [float(m.group(2)) for m in runs]}


def engine_batch_bench(model, sequences, repeats, log):
    """vLLM at `sequences` concurrent requests on the batch protocol's prompts, on the wall's clock and its own."""
    form, flags = ENGINE_CHECKPOINTS[model]["vllm"]
    rel = os.path.join("models", model, "engines", form)
    python, env = engine_env("vllm")
    out = run([python, os.path.join(ROOT, "scripts", "accel", ENGINES["vllm"]), os.path.join(ROOT, rel), "--runs", str(repeats),
               "--batch", str(sequences), "--prompt", SHORT_PROMPT, *flags], log, 3600, env)
    line = next((l for l in out.splitlines() if l.startswith("RESULT ")), None)
    if line is None:
        raise Refused(f"vllm {model} at {sequences}: no RESULT line")
    rec = json.loads(line[len("RESULT "):])
    if rec.get("batch") != sequences or any(len(rec[k]) != repeats for k in ["decode_tok_s", "decode_engine_tok_s"]):
        raise Refused(f"vllm {model} at {sequences}: {rec.get('batch')} sequences, runs {[len(rec[k]) for k in ['decode_tok_s', 'decode_engine_tok_s']]}")
    return {"checkpoint": rel, "version": rec["version"], "decode_tok_s": rec["decode_tok_s"], "decode_engine_tok_s": rec["decode_engine_tok_s"]}


def golden_path(model):
    """The model's f32 golden, which every side's agreement is measured against."""
    rel = os.path.join("benchmarks", "models", model, "golden-f32.json")
    if not os.path.exists(os.path.join(ROOT, rel)):
        raise Refused(f"{model}: no f32 golden at {rel}")
    return rel


def agreement(rel, generated, what):
    """A side's agreement with the f32 golden `rel`: `generated` gives, for each of the golden's prompts, the side's
    greedy tokens and its logits (or log-probabilities) at the first generated position, by token id. A prompt's steps
    are the golden continuation's, compared position by position from the first."""
    with open(os.path.join(ROOT, rel)) as f:
        golden = json.load(f)
    if len(generated) != len(golden["prompts"]):
        raise Refused(f"{what}: {len(generated)} prompts generated, the golden has {len(golden['prompts'])}")
    steps, per_prompt, gaps = 0, [], []
    for i, (p, (tokens, first)) in enumerate(zip(golden["prompts"], generated)):
        want = p["continuation"]
        # The steps before the first the side does not take the golden's way; a step it never generated (its run ended
        # first) is one it does not take.
        agreed = next((j for j in range(len(want)) if j >= len(tokens) or tokens[j] != want[j]), len(want))
        steps += len(want)
        per_prompt.append(agreed)
        (g1, l1), (g2, l2) = p["top"][:2]
        if g1 not in first or g2 not in first:
            raise Refused(f"{what} prompt {i}: the golden's top tokens {g1}, {g2} are not among the side's first logits")
        gaps.append(abs((first[g1] - first[g2]) - (l1 - l2)))
    return {
        "golden": rel,
        "golden_sha256": sha256(os.path.join(ROOT, rel)),
        "steps": steps,
        "top1": sum(per_prompt),
        "per_prompt": per_prompt,
        "top_logit_gap": statistics.mean(gaps),
    }


def lane_agreement(model, gguf, log):
    """Ours against the f32 golden: model_lane's golden run at the precision its speed is measured at."""
    rel = golden_path(model)
    cpu = os.path.join(ROOT, "benchmarks", "models", model, "golden-cpu.json")
    out = run([LANE, gguf, os.path.join(ROOT, rel), cpu, "int8", "16", "128"], log, 3600)
    generated = []
    for line in out.splitlines():
        m = re.match(r"GOLDEN (\d+) TOKENS ([0-9 ]*) FIRST (.*)$", line)
        if m:
            first = {int(k): float(v) for k, v in (kv.split("=") for kv in m.group(3).split())}
            generated.append(([int(t) for t in m.group(2).split()], first))
    return agreement(rel, generated, f"{model} ours")


def llama_agreement(model, gguf, log):
    """llama.cpp's own GPU run against the f32 golden: the golden program on CUDA, as llama-bench runs the model."""
    rel = golden_path(model)
    with open(os.path.join(ROOT, rel)) as f:
        golden = json.load(f)
    out_json = os.path.join(ROOT, "logs", "model-bringup", f"golden-cuda-{model}.json")
    run([os.path.join(ROOT, "scripts", "accel", "model-golden.sh"), gguf, out_json, str(golden["n_gen"]), "16", "auto", "f16",
         os.path.join(ROOT, "benchmarks", "models", "prompts.txt"), "0", "cuda", "20"], log, 3600)
    with open(out_json) as f:
        ran = json.load(f)
    if [p["tokens"] for p in ran["prompts"]] != [p["tokens"] for p in golden["prompts"]]:
        raise Refused(f"{model} llama.cpp: its prompts' tokens are not the golden's")
    generated = [(p["continuation"], {int(i): float(v) for i, v in p["top"]}) for p in ran["prompts"]]
    return agreement(rel, generated, f"{model} llama.cpp")


def engine_agreement(engine, model, log):
    """An engine against the f32 golden: its bench's --golden run of the golden's prompts."""
    rel = golden_path(model)
    form, flags = ENGINE_CHECKPOINTS[model][engine]
    ckpt = os.path.join(ROOT, "models", model, "engines", form)
    python, env = engine_env(engine)
    out = run([python, os.path.join(ROOT, "scripts", "accel", ENGINES[engine]), ckpt, "--golden", os.path.join(ROOT, rel), *flags], log, 3600, env)
    line = next((l for l in out.splitlines() if l.startswith("RESULT ")), None)
    if line is None:
        raise Refused(f"{engine} {model}: no golden RESULT line")
    rec = json.loads(line[len("RESULT "):])
    generated = [(p["tokens"], {int(i): float(v) for i, v in p["first_top"]}) for p in rec["prompts"]]
    return agreement(rel, generated, f"{model} {engine}")


def read_ceiling(repeats, log):
    """The card's streaming read ceiling in GB/s, `repeats` runs of scripts/accel/read_ceiling.cu."""
    probe = os.path.join(ROOT, "logs", "model-bringup", "read_ceiling")
    run(["nvcc", "-O3", "-arch=sm_86", "-o", probe, os.path.join(ROOT, "scripts", "accel", "read_ceiling.cu")], log, 600, card=False)
    rates = []
    for _ in range(repeats):
        m = re.search(r"^READ_GB_S ([0-9.]+)$", run([probe], log, 600), re.M)
        if not m:
            raise Refused("read_ceiling printed no READ_GB_S line")
        rates.append(float(m.group(1)))
    return rates


def decode_reads(gguf, log):
    """The weight bytes the lane's decode step reads a token, and whether it routes through experts."""
    out = run([LANE, gguf, "--plan", "int8"], log, 1800)
    b = re.search(r"^DECODE_READ_BYTES: (\d+)$", out, re.M)
    r = re.search(r"^DECODE_ROUTED: (true|false)$", out, re.M)
    if not (b and r):
        raise Refused(f"{gguf}: model_lane --plan printed no decode reads")
    return int(b.group(1)), r.group(1) == "true"


def llama_bench(gguf, repeats, log):
    out = run([LLAMA_BENCH, "-m", gguf, "-p", "512", "-n", "128", "-r", str(repeats), "-ngl", "99", "-o", "json"], log, 3600)
    return llama_bench_runs(json.loads(out), gguf, repeats)


def llama_bench_runs(tests, gguf, repeats):
    """pp512 and tg128 from llama-bench's JSON: every run, its rate from its own nanoseconds, and llama-bench's
    reported mean and deviation."""
    by = {}
    for t in tests:
        key = "pp512" if t["n_prompt"] == 512 and t["n_gen"] == 0 else "tg128" if t["n_gen"] == 128 and t["n_prompt"] == 0 else None
        if key:
            ns = t.get("samples_ns")
            if not ns or len(ns) != repeats:
                raise Refused(f"{gguf}: llama-bench {key} has {ns!r}, {repeats} asked")
            tokens = t["n_prompt"] + t["n_gen"]
            by[key] = {
                "tokens": tokens,
                "ns": ns,
                "tok_s": [tokens * 1e9 / x for x in ns],
                "reported_avg": t["avg_ts"],
                "reported_stddev": t["stddev_ts"],
            }
    if set(by) != {"pp512", "tg128"}:
        raise Refused(f"{gguf}: llama-bench gave {sorted(by)}")
    return by


def engine_env(engine):
    """An engine's Python and the environment it runs in: its venv's CUDA 13 libraries on the loader's path — NVRTC,
    which compiles vLLM's FP8 kernels as they load, finds its builtins beside it — and for vLLM the system's CUDA toolkit
    (/usr/local/cuda), for the kernels its libraries compile as they load; for ExLlamaV3 the CUDA 13.0
    compiler its torch was built by, building its kernels once for this GPU into the engine's own directory (by its
    real path, so a build already there is the one torch finds)."""
    home = os.path.join(ROOT, "external", "engines", engine)
    python = os.path.join(home, "venv", "bin", "python")
    cu13 = sorted(glob.glob(os.path.join(home, "venv", "lib", "python3.*", "site-packages", "nvidia", "cu13")))
    if not os.path.exists(python) or not cu13:
        raise Refused(f"{engine}: no environment at {os.path.relpath(home, ROOT)}/venv")
    cu13 = cu13[-1]
    env = dict(os.environ, HF_HUB_OFFLINE="1")
    env["LD_LIBRARY_PATH"] = os.path.join(cu13, "lib") + (":" + env["LD_LIBRARY_PATH"] if env.get("LD_LIBRARY_PATH") else "")
    if engine == "vllm":
        toolkit = "/usr/local/cuda"
        if not os.path.exists(os.path.join(toolkit, "bin", "nvcc")):
            raise Refused(f"vllm: no CUDA toolkit at {toolkit} for the kernels its libraries compile")
        env.update(CUDA_HOME=toolkit, PATH=os.path.join(toolkit, "bin") + ":" + env["PATH"])
    if engine == "exllamav3":
        cc = subprocess.run(["nvidia-smi", "--query-gpu=compute_cap", "--format=csv,noheader"], capture_output=True, text=True).stdout.split()[0]
        env.update(CUDA_HOME=cu13, PATH=os.path.join(cu13, "bin") + ":" + env["PATH"], TORCH_CUDA_ARCH_LIST=cc,
                   TORCH_EXTENSIONS_DIR=os.path.realpath(os.path.join(home, "torch_extensions")))
    return python, env


def checkpoint_sha256(directory):
    """A checkpoint directory's identity: the digest of its weight files' names and digests, in name order."""
    h = hashlib.sha256()
    files = sorted(f for f in os.listdir(directory) if f.endswith(".safetensors"))
    if not files:
        raise Refused(f"{directory}: no weight files")
    for f in files:
        h.update(f"{f} {sha256(os.path.join(directory, f))}\n".encode())
    return h.hexdigest(), sum(os.path.getsize(os.path.join(directory, f)) for f in files)


def engine_bench(engine, model, repeats, log):
    """An engine's runs of the model at batch 1: by the wall and by the engine's own clock, every run kept."""
    form, flags = ENGINE_CHECKPOINTS[model][engine]
    rel = os.path.join("models", model, "engines", form)
    if not os.path.isdir(os.path.join(ROOT, rel)):
        raise Refused(f"{engine}: {rel} is not on this machine")
    python, env = engine_env(engine)
    script = os.path.join(ROOT, "scripts", "accel", ENGINES[engine])
    out = run([python, script, os.path.join(ROOT, rel), "--runs", str(repeats), *flags, *(["--prompt", SHORT_PROMPT] if engine == "vllm" else [])], log, 3600, env)
    line = next((l for l in out.splitlines() if l.startswith("RESULT ")), None)
    if line is None:
        raise Refused(f"{engine} {model}: no RESULT line")
    rec = json.loads(line[len("RESULT "):])
    keys = ["decode_tok_s", "prefill_tok_s", "decode_engine_tok_s", "prefill_engine_tok_s"]
    if any(len(rec[k]) != repeats for k in keys):
        raise Refused(f"{engine} {model}: {[len(rec[k]) for k in keys]} runs, {repeats} asked")
    digest, size = checkpoint_sha256(os.path.join(ROOT, rel))
    return dict({"checkpoint": rel, "checkpoint_bytes": size, "checkpoint_sha256": digest, "flags": flags, "version": rec["version"]},
                **{k: rec[k] for k in keys})


def llama_pin():
    with open(PINS) as f:
        for line in f:
            cols = line.rstrip("\n").split("\t")
            if cols[0] == "llama.cpp":
                return {"repo": cols[1], "tag": cols[2], "commit": cols[3]}
    raise Refused("llama.cpp has no pin in external/competitors/PINS.tsv")


def machine():
    gpu = subprocess.run(
        ["nvidia-smi", "--query-gpu=name,memory.total,driver_version", "--format=csv,noheader"], capture_output=True, text=True
    ).stdout.strip()
    nvcc = subprocess.run(["nvcc", "--version"], capture_output=True, text=True).stdout.strip().splitlines()[-1]
    with open("/proc/sys/kernel/random/boot_id") as f:
        boot = f.read().strip()
    cpu = next((l.split(":", 1)[1].strip() for l in open("/proc/cpuinfo") if l.startswith("model name")), "?")
    return {"gpu": gpu, "nvcc": nvcc, "boot_id": boot, "cpu": cpu}


def main():
    global LANE
    ap = argparse.ArgumentParser()
    ap.add_argument("--tag", default=datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%d-%H%M"))
    ap.add_argument("--models", default=",".join(m for m, _ in MODELS))
    ap.add_argument("--engines", default=",".join(ENGINES))
    ap.add_argument("--repeats", type=int, default=5)
    ap.add_argument("--ledger", default=os.path.join(ROOT, "benchmarks", "results", "models", "rtx3060_scoreboard.json"))
    a = ap.parse_args()
    wanted = a.models.split(",")
    unknown = [m for m in wanted if m not in dict(MODELS)]
    if unknown:
        sys.exit(f"unknown models {unknown}; known: {[m for m, _ in MODELS]}")
    engines = [e for e in a.engines.split(",") if e]
    if any(e not in ENGINES for e in engines):
        sys.exit(f"unknown engines {engines}; known: {list(ENGINES)}")
    os.makedirs(os.path.join(ROOT, "logs", "model-bringup"), exist_ok=True)
    log_path = os.path.join(ROOT, "logs", "model-bringup", f"scoreboard-{a.tag}.log")
    with open(log_path, "w") as log:
        try:
            LANE = build_lane(log)
            ledger = {
                "_what": "The bring-up scoreboard: the compiled lane (codegen_cuda::lane, the model's one LOGOS program) "
                "against llama.cpp and the serving engines on the same GPU, one boot, every run kept. Written by "
                "scripts/accel/model_scoreboard.py; held by model_scoreboard_lock.",
                "schema": 1,
                "measured_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
                "machine": dict(machine(), read_ceiling_gb_s=read_ceiling(a.repeats, log)),
                "llama_cpp": llama_pin(),
                "protocol": {
                    "ours_decode": f"model_lane --bench int8 128 <{SHORT_PROMPT}> 16 {CTX}: the decode rate over 128 "
                    "generated tokens after a 6-token prompt, one warm-up then the timed runs",
                    "ours_prefill": f"model_lane --bench int8 16 <512 tokens> 512 {CTX}: one 512-token chunk",
                    "llama_cpp": "llama-bench -p 512 -n 128 -r <repeats> -ngl 99: pp512 and tg128, every layer on the GPU",
                    "read_ceiling": "scripts/accel/read_ceiling.cu: a 4 GiB buffer read once with 16-byte loads, the fastest "
                    "launch shape, <repeats> runs; decode_bytes_per_token is model_lane --plan's Gguf::decode_read_bytes",
                    "numerics": "the lane's one-word int8 activations, llama.cpp's own q8_1 precision",
                    "engines": "scripts/accel/engine_bench_<engine>.py in the engine's own environment, batch 1, greedy, "
                    "the end of text ignored, prefix caching off: a 512-token prompt generating 1 token, and a 6-token prompt "
                    "generating 129 and 1, every run timed by the wall (prefill 512 / its time, decode 128 / the difference) "
                    "and by the engine's own clock (prefill 512 / scheduled or prefill start to first token, decode 128 / "
                    "first to last token); each engine's 8-bit weight form of the model, its checkpoint's weight files hashed",
                    "batch": f"model_lane --bench int8 129 <{SHORT_PROMPT}> 16 {BATCH_CTX} --batch=<B>: B sequences prefilled each "
                    "into its own state, then decoded a token each a step together, sequence b's prompt the six tokens each moved "
                    "7919·b on modulo the vocabulary, the rate every sequence's 128 decoded tokens over the batch steps' time; vLLM "
                    "at B concurrent requests of the same prompts (engine_bench_vllm.py --batch B): the wall's B·128 over (the B "
                    "generating 129 − the B generating 1) and its own clock's over the last request's first to last token",
                    "batch_sequences": sorted({n for counts in BATCHES.values() for n in counts}),
                    "quality": "each side against benchmarks/models/<model>/golden-f32.json (llama.cpp's f32 run on the CPU), "
                    "over its prompts as its tokens, each generated greedily for the golden's length: top1 the steps agreeing "
                    "with the golden's continuation before the first that does not, of steps; top_logit_gap the mean over the "
                    "prompts of |(the side's logit of the golden's top token - of its second) - (the golden's)| at the first "
                    "generated position. Ours: model_lane's golden run, int8. llama.cpp: model-golden.sh device=cuda flash=auto "
                    "kv=f16, as llama-bench runs it. Engines: engine_bench_<engine>.py --golden, their logprobs",
                    "repeats": a.repeats,
                },
                "engines": {},
                "rows": [],
            }
            for name, rel in MODELS:
                if name not in wanted:
                    continue
                gguf = os.path.join(ROOT, rel)
                if not os.path.exists(gguf):
                    raise Refused(f"{rel} is not on this machine")
                log.write(f"== {name}\n")
                load_before = os.getloadavg()[0]
                reads, routed = decode_reads(gguf, log)
                row = {
                    "model": name,
                    "file": rel,
                    "file_bytes": os.path.getsize(gguf),
                    "file_sha256": sha256(gguf),
                    "decode_bytes_per_token": reads,
                    "decode_routed": routed,
                    "ours": {
                        "short": lane_bench(gguf, 128, SHORT_PROMPT, 16, a.repeats, log),
                        "long": lane_bench(gguf, 16, LONG_PROMPT, 512, a.repeats, log),
                    },
                    "llama_cpp": llama_bench(gguf, a.repeats, log),
                }
                row["ours"]["agreement"] = lane_agreement(name, gguf, log)
                row["llama_cpp"]["agreement"] = llama_agreement(name, gguf, log)
                for engine in engines:
                    if engine in ENGINE_CHECKPOINTS.get(name, {}):
                        side = engine_bench(engine, name, a.repeats, log)
                        known = ledger["engines"].setdefault(engine, {"version": side["version"]})
                        if known["version"] != side["version"]:
                            raise Refused(f"{engine}: version {side['version']} beside {known['version']}")
                        side["agreement"] = engine_agreement(engine, name, log)
                        row[engine] = side
                batches = []
                for sequences in BATCHES.get(name, []):
                    batch = {"sequences": sequences, "ours": lane_batch_bench(gguf, sequences, a.repeats, log)}
                    if "vllm" in engines and "vllm" in row:
                        side = engine_batch_bench(name, sequences, a.repeats, log)
                        if side["version"] != ledger["engines"]["vllm"]["version"]:
                            raise Refused(f"vllm: version {side['version']} beside {ledger['engines']['vllm']['version']}")
                        batch["vllm"] = side
                    batches.append(batch)
                if batches:
                    row["batches"] = batches
                row["load_average"] = [round(load_before, 2), round(os.getloadavg()[0], 2)]
                ledger["rows"].append(row)
        except (Refused, subprocess.TimeoutExpired, json.JSONDecodeError) as e:
            log.write(f"REFUSED: {e}\n")
            sys.exit(f"refused, nothing recorded: {e} (log {log_path})")
    os.makedirs(os.path.dirname(a.ledger), exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=os.path.dirname(a.ledger), suffix=".tmp")
    with os.fdopen(fd, "w") as f:
        json.dump(ledger, f, indent=1)
        f.write("\n")
    os.chmod(tmp, 0o644)
    os.replace(tmp, a.ledger)
    print(f"recorded {len(ledger['rows'])} models to {os.path.relpath(a.ledger, ROOT)} (log {os.path.relpath(log_path, ROOT)})")


if __name__ == "__main__":
    main()
