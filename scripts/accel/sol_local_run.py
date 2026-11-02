#!/usr/bin/env python3
"""Emit + build + validate the compiler-generated SOL-ExecBench solutions for the norm family ON THE LOCAL
RTX 3060 — the local proxy for the B200 harness's correctness stage.

    ~/kb_venv/bin/python scripts/accel/sol_local_run.py

For each op it shells out to `cargo run --example emit_sol_solution` (English LOGOS → kernel.cu +
solution.json), appends the pybind glue the harness would add for the `entry_point`, JIT-builds via
torch.utils.cpp_extension, then runs our `run(...)` on random bf16 inputs at real (batch × hidden) shapes and
compares to the PyTorch reference within the FIB tolerances (atol=rtol=1e-2, matched_ratio >= 0.99).
"""
import os
import subprocess
import sys

import torch
from torch.utils.cpp_extension import load

EPS = 1e-6


def ref_fused_add(h, r, w):
    x = h.float() + r.float()
    inv = torch.rsqrt(x.pow(2).mean(-1, keepdim=True) + EPS)
    return ((x * inv) * w.float()).to(h.dtype)


def ref_weighted(h, w):
    inv = torch.rsqrt(h.float().pow(2).mean(-1, keepdim=True) + EPS)
    return ((h.float() * inv) * w.float()).to(h.dtype)


def ref_layernorm(h, w, b):
    x = h.float()
    mean = x.mean(-1, keepdim=True)
    var = (x - mean).pow(2).mean(-1, keepdim=True)
    return (((x - mean) * torch.rsqrt(var + EPS)) * w.float() + b.float()).to(h.dtype)


OPS = {
    "fused_add_rmsnorm": (["hidden_states", "residual", "weight"], ref_fused_add),
    "rmsnorm_weighted": (["hidden_states", "weight"], ref_weighted),
    "layernorm": (["hidden_states", "weight", "bias"], ref_layernorm),
}
SHAPES = [(32, 2048), (7, 4096), (128, 7168)]  # Qwen3-30B / Llama-3.1-8B / DeepSeek-V3 hidden sizes
dt = torch.bfloat16
dev = "cuda"


def make_input(name, B, H):
    return torch.randn(B, H, dtype=dt, device=dev) if name in ("hidden_states", "residual") else torch.randn(H, dtype=dt, device=dev)


def main():
    torch.manual_seed(0)
    all_pass = True
    for op, (inputs, ref) in OPS.items():
        out_dir = f"target/sol-execbench/{op}"
        print(f"\n== {op} ==")
        r = subprocess.run(
            ["cargo", "run", "-q", "-p", "logicaffeine-compile", "--example", "emit_sol_solution", "--", op, out_dir],
            capture_output=True, text=True,
        )
        if r.returncode != 0:
            print(f"  emit FAILED:\n{r.stderr[-800:]}")
            all_pass = False
            continue
        src = open(os.path.join(out_dir, "kernel.cu")).read()
        src += "\nPYBIND11_MODULE(TORCH_EXTENSION_NAME, m){ m.def(\"run\", &run, \"run\"); }\n"
        tmp = os.path.join(out_dir, "_kernel_pybind.cu")
        open(tmp, "w").write(src)
        mod = load(name=f"logos_{op}", sources=[tmp], extra_cuda_cflags=["-O3", "--use_fast_math"], verbose=False)
        for (B, H) in SHAPES:
            tensors = [make_input(n, B, H) for n in inputs]
            out = torch.empty(B, H, dtype=dt, device=dev)
            mod.run(*tensors, out)
            torch.cuda.synchronize()
            golden = ref(*tensors)
            diff = (out.float() - golden.float()).abs()
            ok = diff <= (1e-2 + 1e-2 * golden.float().abs())
            matched = ok.float().mean().item()
            verdict = "PASS" if matched >= 0.99 else "FAIL"
            all_pass &= matched >= 0.99
            print(f"  [{B}x{H}] matched_ratio={matched:.4f} max_abs_err={diff.max().item():.4g}  {verdict}")
        os.remove(tmp)

    print()
    if all_pass:
        print("LOCAL RTX 3060 RUN: PASS — every compiler-generated norm-family solution ≡ PyTorch reference (bf16 tol).")
        sys.exit(0)
    print("LOCAL RTX 3060 RUN: FAIL")
    sys.exit(1)


if __name__ == "__main__":
    main()
