#!/usr/bin/env python3
"""decode_gpu_sustain.py BENCH_CU OUT_CU SECONDS US — a LOGOS bench program with a sustained run added after its best-of loop: the
same launch back to back for SECONDS at about US per launch, between two events and with no synchronization between
launches, its average time per launch printed as SUSTAINED_US_PER_STEP. The power sampled over that run is the
program's draw while it decodes."""
import re
import sys

src, out, seconds, us = sys.argv[1], sys.argv[2], float(sys.argv[3]), float(sys.argv[4])
text = open(src).read()
m = re.search(r"cudaEventRecord\(e0\); (logos_spatial_\w+<<<[^;]*;) cudaEventRecord\(e1\);", text)
if not m:
    sys.exit(f"{src}: no timed launch")
n = int(seconds * 1e6 / us)
tail = 'printf("US_PER_STEP %.3f\\n", best * 1000.0f);'
if text.count(tail) != 1:
    sys.exit(f"{src}: no single best-of report")
block = (tail + f"\n    {{ const int SUST = {n}; cudaEventRecord(e0); for (int it = 0; it < SUST; ++it) {m.group(1)} "
         "cudaEventRecord(e1); cudaEventSynchronize(e1); float ms = 0; cudaEventElapsedTime(&ms, e0, e1); "
         "{ cudaError_t err = cudaGetLastError(); if (err != cudaSuccess) { fprintf(stderr, \"%s\\n\", cudaGetErrorString(err)); return 1; } } "
         'printf("SUSTAINED_US_PER_STEP %.3f over %d launches\\n", ms * 1000.0f / SUST, SUST); }')
open(out, "w").write(text.replace(tail, block))
