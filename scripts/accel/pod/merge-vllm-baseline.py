#!/usr/bin/env python3
"""Fold a `vllm-baseline.sh` output directory into benchmarks/results/qwen38_27b.json.

Both sides are read from the artifacts the session actually produced -- vLLM's own `--save-result`
JSON (its metrics, by construction) and our lanes' stdout/stderr -- so no number is retyped by hand.

    python3 scripts/accel/pod/merge-vllm-baseline.py <outdir> [--block vllm_baseline_2026_09_19] [--write]

Without --write it prints the block and changes nothing.
"""
import argparse, glob, json, os, re, sys

OURS_RE = {
    "prefill_ms":     re.compile(r"PREFILL_MS\s+([0-9.]+)"),
    "ttft_ms":        re.compile(r"TTFT_MS\s+([0-9.]+)"),
    "tok_s":          re.compile(r"(?:AGGREGATE_TOK_S|GEN_TOK_S_TG128)\s+([0-9.]+)"),
    "ms_per_step":    re.compile(r"MS_PER_STEP\s+([0-9.]+)"),
}
AGREE_RE = re.compile(r"BATCH_AGREEMENT:\s*(\d+)/(\d+)")


def read_vllm(outdir):
    rows = []
    for f in sorted(glob.glob(os.path.join(outdir, "vllm_*_c*.json"))):
        try:
            d = json.load(open(f))
        except Exception:
            continue
        if isinstance(d, list):
            d = d[0] if d else {}
        base = os.path.basename(f)[len("vllm_"):-len(".json")]
        quant, spec, conc = base.rsplit("_", 2)
        rows.append({
            "quant": quant, "spec": spec, "concurrency": int(conc[1:]),
            "output_throughput": d.get("output_throughput"),
            "request_throughput": d.get("request_throughput"),
            "median_ttft_ms": d.get("median_ttft_ms"),
            "median_tpot_ms": d.get("median_tpot_ms"),
            "median_itl_ms": d.get("median_itl_ms"),
            "p99_ttft_ms": d.get("p99_ttft_ms"),
        })
    rows.sort(key=lambda r: (r["quant"], r["spec"], r["concurrency"]))
    return rows


def read_ours(outdir):
    rows = []
    for f in sorted(glob.glob(os.path.join(outdir, "ours_*.out"))):
        lane = os.path.basename(f)[len("ours_"):-len(".out")]
        text = open(f, errors="replace").read()
        err = f[:-4] + ".err"
        if os.path.exists(err):
            text += open(err, errors="replace").read()
        row = {"lane": lane}
        m = re.match(r"w(\d+)_b(\d+)_(\w+)", lane)
        if m:
            row.update(input_len=int(m.group(1)), concurrency=int(m.group(2)), precision=m.group(3))
        for k, rx in OURS_RE.items():
            hit = rx.search(text)
            if hit:
                row[k] = float(hit.group(1))
        agree = AGREE_RE.search(text)
        if agree:
            row["batch_agreement"] = f"{agree.group(1)}/{agree.group(2)}"
            row["golden"] = agree.group(1) == agree.group(2)
        rows.append(row)
    rows.sort(key=lambda r: (r.get("concurrency", 0), r.get("precision", "")))
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("outdir")
    ap.add_argument("--block", default="vllm_baseline_2026_09_19")
    ap.add_argument("--results", default="benchmarks/results/qwen38_27b.json")
    ap.add_argument("--gpu", default="NVIDIA H100 NVL, sm_90")
    ap.add_argument("--write", action="store_true")
    a = ap.parse_args()

    vllm, ours = read_vllm(a.outdir), read_ours(a.outdir)
    best = max((r for r in vllm if r.get("output_throughput")),
               key=lambda r: r["output_throughput"], default=None)

    block = {
        "_what": "Phase 1 BASELINE: vLLM's best config against our existing static-batch lane, same chip, "
                 "same boot. This session wrote no kernels -- it exists to measure the real gap before "
                 "any optimisation is ranked.",
        "gpu": a.gpu,
        "date": "2026-09-19",
        "measured": True,
        "workload": "512 prompt tokens, 128 generated, greedy; vLLM --dataset-name random with "
                    "--ignore-eos, our w512_* lanes carry a 512-token prompt and n_gen=128",
        "metric_definitions": "vLLM benchmarks/serve.py:748-751 -- output_throughput = "
                              "sum(output_lens)/dur_s; tpot = (latency-ttft)/(output_len-1)",
        "vllm": {
            "version": "0.29.0", "torch": "2.13.0+cu130",
            "caveat": "VLLM_USE_FLASHINFER_SAMPLER=0 -- FlashInfer's sampling JIT could not be built in "
                      "this image under either CUDA toolkit. It is vLLM's own supported fallback and "
                      "touches only the argmax over 248320 logits, not the attention/GDN/GEMM kernels "
                      "this comparison is about.",
            "rows": vllm, "best": best,
        },
        "ours": {
            "lane_note": "two-pass int8 (hl) is the golden lane; single-pass (fast) trades activation "
                         "precision for GEMM work. Accuracy is reported separately and never assumed.",
            "known_deficit": "prefill replays the T=1 decode graph once per prompt token, so TTFT at 512 "
                             "input is ~512 forwards. PREFILL_MS/TTFT_MS make that a measured number.",
            "rows": ours,
        },
    }
    print(json.dumps(block, indent=1)[:4000])
    if not a.write:
        print("\n(dry run -- pass --write to fold into %s)" % a.results, file=sys.stderr)
        return
    doc = json.load(open(a.results))
    doc.setdefault("logos", {})[a.block] = block
    json.dump(doc, open(a.results, "w"), indent=1)
    print("\nwrote %s -> logos.%s" % (a.results, a.block), file=sys.stderr)


if __name__ == "__main__":
    main()
