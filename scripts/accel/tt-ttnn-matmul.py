#!/usr/bin/env python3
"""tt-ttnn-matmul — run the VENDOR's production op (ttnn.matmul) under ttsim with EXPLICIT numerics.

The reference rows of benchmarks/silicon/tenstorrent_gemm.json come from this script, run through
`scripts/tt-sim.sh --python scripts/accel/tt-ttnn-matmul.py …` so the ledger banner is printed first.

Why explicit: ttnn picks its math fidelity by a heuristic — HiFi2 with no program config, LoFi the moment a
program config is passed (matmul_device_operation.cpp: `increase_fidelity = !has_program_config && …`).
This script ALWAYS passes a compute_kernel_config, so a row's fidelity is what the harness asked for, never
what a heuristic chose. It prints the same LOGOS_MM_CFG / LOGOS_MM_PASS lines the emitted programs print.

    --config default   ttnn's own program-factory choice (no program_config); knobs pinned to the heuristic's
                       HiFi2 / fp32 off / packer-L1-acc on unless overridden
    --config ours      MatmulMultiCoreReuseMultiCastProgramConfig with the LOGOS autotuner's tiling
    --scaffold-only    everything but the matmul (device open, uploads, read-back) — the scaffold cycles

Inputs are a deterministic host-side fill (LCG seed 0x4c4f474f, uniform bf16 in [-1,1)) uploaded from
numpy — never ttnn.ones/full (device fill programs would pollute g_clock and make PCC meaningless).
"""
import argparse
import os
import sys

import numpy as np

ORIGIN_DEFAULT = "ttnn.matmul default heuristic"
ORIGIN_OURS = "ttnn.matmul with LOGOS program config"
LCG_SEED = 0x4C4F474F
PCC_FLOOR = 0.99


def lcg_uniform_bf16(n, seed):
    """Deterministic uniform values in [-1, 1) rounded to bf16 (RNE), returned as float32."""
    x = np.empty(n, dtype=np.uint32)
    s = seed & 0xFFFFFFFF
    for i in range(n):
        s = (s * 1664525 + 1013904223) & 0xFFFFFFFF
        x[i] = s
    f = (x.astype(np.float64) / 2.0 ** 32) * 2.0 - 1.0
    f32 = f.astype(np.float32)
    u = f32.view(np.uint32)
    rounded = (u + 0x7FFF + ((u >> 16) & 1)) & 0xFFFF0000
    return rounded.astype(np.uint32).view(np.float32)


def pcc(x, y):
    x = x.astype(np.float64).ravel()
    y = y.astype(np.float64).ravel()
    n = min(x.size, y.size)
    x, y = x[:n], y[:n]
    dx, dy = x - x.mean(), y - y.mean()
    den = np.sqrt((dx * dx).sum() * (dy * dy).sum())
    return 1.0 if den == 0 else float((dx * dy).sum() / den)


def parse_pair(s, name):
    try:
        a, b = s.lower().split("x")
        return int(a), int(b)
    except Exception:
        sys.exit("%s must look like AxB, got %r" % (name, s))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--shape", type=int, nargs=3, required=True, metavar=("M", "N", "K"))
    ap.add_argument("--config", choices=("default", "ours"), required=True)
    ap.add_argument("--fidelity", choices=("LoFi", "HiFi2", "HiFi3", "HiFi4"), required=True)
    ap.add_argument("--fp32", type=int, choices=(0, 1), required=True)
    ap.add_argument("--l1acc", type=int, choices=(0, 1), required=True)
    ap.add_argument("--grid", default="5x5")
    ap.add_argument("--per-core", default="4x4")
    ap.add_argument("--in0-block-w", type=int, default=4)
    ap.add_argument("--subblock", default="2x2")
    ap.add_argument("--scaffold-only", action="store_true")
    args = ap.parse_args()

    import ttnn  # imported late so --help works without the tt-metal tree

    m, n, k = args.shape
    chip = os.environ.get("LOGOS_TT_CHIP", "unknown")
    gx, gy = parse_pair(args.grid, "--grid")
    pcm, pcn = parse_pair(args.per_core, "--per-core")
    osh, osw = parse_pair(args.subblock, "--subblock")
    if args.config == "ours":
        print("LOGOS_MM_CFG strategy=vendor grid=%dx%d per_core=%dx%d out_block=%dx%d in0_block_w=%d subblock=%dx%d origin=\"%s\""
              % (gx, gy, pcm, pcn, pcm, pcn, args.in0_block_w, osh, osw, ORIGIN_OURS), flush=True)
    else:
        print("LOGOS_MM_CFG strategy=vendor origin=\"%s\"" % ORIGIN_DEFAULT, flush=True)

    a_np = lcg_uniform_bf16(m * k, LCG_SEED).reshape(m, k)
    b_np = lcg_uniform_bf16(k * n, LCG_SEED ^ 0x5A5A5A5A).reshape(k, n)
    golden = a_np.astype(np.float32) @ b_np.astype(np.float32)

    dev = ttnn.open_device(device_id=0)
    try:
        a = ttnn.Tensor(a_np, ttnn.bfloat16, device=dev, layout=ttnn.TILE_LAYOUT, mem_config=ttnn.DRAM_MEMORY_CONFIG)
        b = ttnn.Tensor(b_np, ttnn.bfloat16, device=dev, layout=ttnn.TILE_LAYOUT, mem_config=ttnn.DRAM_MEMORY_CONFIG)
        if args.scaffold_only:
            _ = np.asarray(ttnn.to_dtype(a.cpu(), ttnn.float32).to_numpy(), dtype=np.float32)
            print("LOGOS_MM_SCAFFOLD strategy=vendor", flush=True)
            return
        ck = ttnn.WormholeComputeKernelConfig(
            math_fidelity=getattr(ttnn.MathFidelity, args.fidelity),
            math_approx_mode=False,
            fp32_dest_acc_en=bool(args.fp32),
            packer_l1_acc=bool(args.l1acc),
        )
        program_config = None
        if args.config == "ours":
            program_config = ttnn.MatmulMultiCoreReuseMultiCastProgramConfig(
                compute_with_storage_grid_size=(gx, gy),
                in0_block_w=args.in0_block_w,
                out_subblock_h=osh, out_subblock_w=osw,
                out_block_h=pcm, out_block_w=pcn,
                per_core_M=pcm, per_core_N=pcn,
                transpose_mcast=False, fused_activation=None,
            )
        c = ttnn.matmul(a, b, program_config=program_config, compute_kernel_config=ck,
                        memory_config=ttnn.DRAM_MEMORY_CONFIG, dtype=ttnn.bfloat16)
        # bf16 host tensors surface as an opaque object array; convert on the HOST to float32 first (no device work).
        out = np.asarray(ttnn.to_dtype(c.cpu(), ttnn.float32).to_numpy(), dtype=np.float32).reshape(m, n)
        p = pcc(golden, out)
        print("LOGOS ttnn.matmul (%s, %s) -- PCC = %.7f" % (chip, args.config, p), flush=True)
        if p > PCC_FLOOR:
            print("LOGOS_MM_PASS fidelity=%s fp32=%d l1acc=%d pcc=%.7f floor=%s" % (args.fidelity, args.fp32, args.l1acc, p, PCC_FLOOR), flush=True)
        else:
            print("LOGOS_MM_FAIL pcc=%.7f" % p, flush=True)
            sys.exit(1)
    finally:
        ttnn.close_device(dev)


if __name__ == "__main__":
    main()
