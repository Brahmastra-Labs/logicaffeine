#!/usr/bin/env python3
"""decode_gpu_ledger.py RUN_DIR MODEL_JSON OUT_JSON — benchmarks/silicon/decode_gpu.json from one run of
scripts/accel/decode-gpu-measure.sh: the boot it ran in, and for each program its time per token and its draw over the
same sustained run, so a token's energy is the time it took at the power the part drew meanwhile.

Refuses a run that did not finish, whose reference did not generate the golden tokens, whose LOGOS programs did not
match the evaluator, during which another process used the GPU, or whose sustained runs held the GPU below 90%
utilization in most samples."""
import datetime
import json
import os
import re
import statistics
import sys

d, model_path, out = sys.argv[1], sys.argv[2], sys.argv[3]
log = open(os.path.join(d, "log.txt")).read()
if not log.rstrip().endswith("done"):
    sys.exit(f"{d}: the run did not finish")
model = json.load(open(model_path))
trace = model["golden_trace"]
boot, driver = re.search(r"^boot (\S+ \S+) UTC; driver (\S+)", log, re.M).groups()

ref = open(os.path.join(d, "ref.out")).read()
gen = [int(m) for m in re.findall(r"^GEN \d+ (\d+)", ref, re.M)]
if gen != trace["generated_tokens"]:
    sys.exit(f"the reference generated {gen}, the trace {trace['generated_tokens']}")
logos = open(os.path.join(d, "logos.out")).read()
for prog in ("flat", "general"):
    if not re.search(rf"^{prog}: \d+ tensors MATCH", logos, re.M):
        sys.exit(f"LOGOS {prog} program: no match with the evaluator on record")


def draw(name):
    """The samples of a sustained run: (power W, utilization %) each, and the other GPU processes seen meanwhile."""
    rows = []
    for line in open(os.path.join(d, name)):
        f = [x.strip() for x in line.split(",")]
        if len(f) >= 2 and f[0] and f[1]:
            rows.append((float(f[0]), float(f[1])))
    # The sampler splits nvidia-smi's process list on whitespace; its `[No data]`, printed while no compute process
    # holds a context (the run's start and end), is no process.
    others = open(os.path.join(d, name + ".others")).read().replace("[No data]", "").split()
    others = " ".join(o for o in others if o != "none") or "none"
    if others != "none":
        sys.exit(f"{name}: another process used the GPU during the run ({others})")
    busy = [w for w, u in rows if u >= 90]
    if len(busy) * 2 < len(rows) or len(busy) < 20:
        sys.exit(f"{name}: {len(busy)} of {len(rows)} samples at 90% utilization or more")
    return rows, busy


idle = [float(l.split(",")[0]) for l in open(os.path.join(d, "idle.txt")) if l.strip()]
idle_w = round(statistics.mean(idle), 2)


def power_how(rows, busy, what, seconds):
    """How a draw was measured: the samples the sampler took over the run's `seconds`, one about every that many over
    their count (each sample is two nvidia-smi calls and a 0.25 s pause, starting a second into the run)."""
    return (f"nvidia-smi power.draw sampled {len(rows)} times while {what} ran ({seconds:.1f} s, a sample about every "
            f"{seconds / len(rows):.2f} s), the {len(busy)} at 90% utilization or more averaged, standard deviation "
            f"{statistics.pstdev(busy):.2f} W; idle {idle_w} W in P8 before the runs")


# The steps the reference runs per repetition: every prompt token, then every generated token but the last.
steps = len(trace["prompt_tokens"]) + len(trace["generated_tokens"]) - 1
programs = []
long_out = open(os.path.join(d, "ref_long.out")).read()
ref_us = float(re.search(r"US_PER_TOKEN ([0-9.]+)", long_out).group(1))
rows, busy = draw("ref_long.power")
w = round(statistics.mean(busy), 2)
programs.append({
    "program": "hand-written reference",
    "how": f"Int8Model::reference_cuda: one warp, the trace's {steps} steps on the device in one launch, cudaEvent "
           f"over 400,000 repetitions in a window with no other process on the GPU",
    "us_per_token": ref_us,
    "power_w": w,
    "idle_power_w": idle_w,
    "power_how": power_how(rows, busy, "those 400,000 repetitions", 400_000 * steps * ref_us / 1e6),
    "energy_uj_per_token": round(w * ref_us, 3),
})
for prog, name, what in (("flat", "LOGOS one-block program", "spatial::lower::cuda one-block program of the decode step (16x16 tiles)"),
                         ("general", "LOGOS one block per site", "spatial::lower::cuda general program of the decode step")):
    best = float(re.search(rf"^{prog}: .*US_PER_STEP ([0-9.]+)", logos, re.M).group(1))
    sus = open(os.path.join(d, f"{prog}_sustain.out")).read()
    m = re.search(r"SUSTAINED_US_PER_STEP ([0-9.]+) over (\d+) launches", sus)
    if not m:
        sys.exit(f"{prog}: no sustained run on record")
    us, n = float(m.group(1)), int(m.group(2))
    rows, busy = draw(f"{prog}_sustain.power")
    w = round(statistics.mean(busy), 2)
    programs.append({
        "program": name,
        "how": f"{what}, one step a launch: cudaEvent over {n:,} launches back to back, in a window with no other "
               f"process on the GPU",
        "us_per_token": us,
        "best_us_per_step": best,
        "best_how": "its bench: best of 200 launches of one step, each between two events",
        "power_w": w,
        "idle_power_w": idle_w,
        "power_how": power_how(rows, busy, f"those {n:,} launches", n * us / 1e6),
        "energy_uj_per_token": round(w * us, 3),
    })

ledger = {
    "part": f"NVIDIA GeForce RTX 3060 (sm_86), driver {driver}",
    "boot": f"{boot} UTC",
    "measured": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d"),
    "model": os.path.basename(model_path),
    "trace": {"prompt_tokens": trace["prompt_tokens"], "generated_tokens": trace["generated_tokens"]},
    "programs": programs,
}
json.dump(ledger, open(out, "w"), indent=1)
for p in programs:
    print(f"{p['program']}: {p['us_per_token']} µs/token at {p['power_w']} W, {p['energy_uj_per_token']} µJ/token")
