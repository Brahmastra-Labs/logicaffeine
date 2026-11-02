#!/usr/bin/env python3
"""tt-gemm-ledger — the ONLY writer of `benchmarks/silicon/tenstorrent_gemm.json`.

Every Tenstorrent GEMM cycle number the repo quotes is a ROW of that ledger, and every row is parsed
from a ttsim run log by this tool — never typed. A row carries the math fidelity the PROGRAM printed
(fidelity / fp32 accumulate / packer L1-accumulate), the PCC, the exact reproduce command, the ttsim
and tt-metal identities, a sha256 of the program sources, and a committed EXCERPT of the log that
re-parses to the same facts. Everything derived (comparisons, unmatched pairs, spatial speedups, the
headline) is regenerated on every write and recomputed independently by `tt_sim_lock`.

    add            record one run (refuses without a PASS, on a ttsim ERROR, on a knob mismatch, …)
    scan-examples  record which shapes / knobs / PCC floors the tt-metal example programs can run
    verify         re-derive everything + re-parse every excerpt; exit 1 on any drift
    show           print the matched-numerics comparisons

Exit codes: 0 ok · 1 verify found drift · 2 usage · 3 refused (nothing written).
"""
import argparse
import datetime as _dt
import hashlib
import json
import os
import re
import sys

STACKS = ("logos_emitted", "tt_metal_example", "ttnn_production")
FIDELITIES = ("LoFi", "HiFi2", "HiFi3", "HiFi4")
CHIPS = ("wormhole_b0", "blackhole")
KNOB_KEYS = ("fidelity", "fp32_dest_acc", "packer_l1_acc", "math_approx_mode")
ORIGIN_TTNN_DEFAULT = "ttnn.matmul default heuristic"
ORIGIN_TTNN_OURS = "ttnn.matmul with LOGOS program config"
HEAD_LINES, TAIL_LINES = 40, 200

RE_ENV = re.compile(r"^LOGOS_TT_ENV chip=(\S+) so=(\S+) so_sha256=([0-9a-f]{64}) ttsim_version=(\S+) tt_metal_sha=([0-9a-f]{40})\s*$")
RE_HEARTBEAT = re.compile(r"^\[(\d+)\] (\d+\.\d+) seconds \(\d+\.\d+ [MK]Hz\)\s*$")
RE_ERROR = re.compile(r"^\[\d+\] ERROR:")
RE_CFG = re.compile(r"^LOGOS_MM_CFG strategy=(\S+)"
                    r"(?: grid=(\d+)x(\d+) per_core=(\d+)x(\d+) out_block=(\d+)x(\d+) in0_block_w=(\d+) subblock=(\d+)x(\d+))?"
                    r'(?: origin="([^"]*)")?'
                    r"(?: mesh=(\d+)x(\d+) sites=(\d+))?\s*$")
RE_PASS = re.compile(r"^LOGOS_MM_PASS fidelity=(LoFi|HiFi2|HiFi3|HiFi4) fp32=(0|1) l1acc=(0|1) pcc=([0-9.]+) floor=([0-9.]+)\s*$")
RE_PASS_BARE = re.compile(r"^LOGOS_MM_PASS\s*$")
RE_FAIL = re.compile(r"^LOGOS_MM_FAIL")

# The metric's own validity, stated in the artifact rather than in someone's head. Established by a work
# sweep: hold M and N, double K, and see whether the counter moves. It does not — not the whole-program
# clock, not either movement engine's wall-clock span, not any of the three compute TRISCs — while host
# wall-clock rises almost linearly and the program provably does the work (emitted compute loop 6 -> 14
# iterations, runtime K extent 32 -> 64, PCC 1.0 throughout). The simulator is functionally faithful and
# temporally blind. Cycle counts here order and identify programs; they DO NOT measure execution time, and
# no ratio between them is a speed claim.
PROVENANCE = "sim_functional_not_timed"



CLOCK_VALIDITY = {
    "tracks_work": False,
    "evidence": "K doubled at fixed M,N moves no kernel-reachable counter: 640x640x320/640/1280 -> "
                "5678/5562/5562; the 1024 family -> 6205 at K=512,1024,2048; compute TRISC spans "
                "660->658, 690->682, 919->921, while host wall-clock rises almost linearly",
    "consequence": "ratios computed from `cycles` are NOT speed claims; a defensible speed number needs "
                   "silicon, where the device wall-clock register is real",
    "what_survives": "correctness (PCC), program identity, and orderings that do not assert time",
}
# The slowest core's own cycle counter, stamped by the emitted kernel and read back by the emitted host.
# ★ IT DOES NOT TRACK WORK EITHER. Measured 2026-09-10, 512x512xK with K swept 512->4096 (8x the work):
# 372, 373, 373, 373 — a 0.3% spread against an 8x sweep, the same refutation that condemned g_clock
# (5180, 5182, 5182, 5184). ttsim is a FUNCTIONAL model: it does not simulate per-core timing, so no cycle
# register anywhere on it is a function of the program. The instrumentation is correct and needs silicon to
# mean anything. Kept, never fitted. See `work` below for the meters that DO measure the program.
RE_DEVICE_CYCLES = re.compile(r"^LOGOS_MM_DEVICE_CYCLES=(\d+)\s*$")

# ── THE WORK METER ────────────────────────────────────────────────────────────────────────────────
# ttsim reports no time, but it can report WHAT THE MACHINE DID. The instrumented build (ttsim + a
# counters-only patch, verified timing-identical to the stock release) prints these at simulator exit.
# Measured over the same 8x K sweep that flattened both clocks: tensix_inst 8.11x, matmul_retired
# 8.000x (exact), noc_txns 7.40x, noc_bytes 7.59x. THESE are functions of the program, so these are what
# a cost model may be fitted to and what a compiler may be judged on.
RE_WORK = re.compile(r"^(?:\[\d+\] )?TTSIM_OP_STATS matmul=(\d+) elw=(\d+) tensix_inst=(\d+) noc_txns=(\d+) noc_bytes=(\d+) clock=(\d+)\s*$")
RE_RETIRED = re.compile(r"^(?:\[\d+\] )?TTSIM_DONE matmul_retired=(\d+)\s*$")
RE_STALLS = re.compile(r"^(?:\[\d+\] )?TTSIM_STALLS srca=(\d+) srcb=(\d+) both=(\d+)\s*$")
RE_PCC_LINE = re.compile(r"-- PCC = ([0-9.]+)\s*$")
RE_EX_PCC = re.compile(r"^Metalium vs Golden -- PCC = ([0-9.]+)\s*$")
RE_EX_PASS = re.compile(r"^Test Passed\s*$")
RE_JIT = re.compile(r"JIT cache stats: (\d+)/(\d+) hits")
RE_SRC_FID = re.compile(r"MathFidelity::(LoFi|HiFi2|HiFi3|HiFi4)")
RE_SRC_FP32 = re.compile(r"fp32_dest_acc_en\s*=\s*(true|false)")
RE_SRC_L1 = re.compile(r"packer_l1_acc\s*=\s*(true|false)")
RE_SRC_SHAPE = re.compile(r"constexpr\s+uint32_t\s+([MNK])\s*=\s*(\d+)")
RE_SRC_PCC_FLOOR = re.compile(r"TT_FATAL\(\s*pearson\s*>\s*([0-9.]+)")
RE_SRC_LOGOS_FLOOR = re.compile(r"pcc\s*>\s*([0-9.]+)")

COMMENT = ("GENERATED by scripts/tt-gemm-ledger.py from scripts/tt-matmul-measure.sh runs. NEVER hand-edit: "
           "every row is re-parsed from its committed ttsim log excerpt by tt_sim_lock, and every comparison "
           "ratio is recomputed there. cycles = ttsim g_clock at simulator exit (the `[N] … seconds (… MHz)` "
           "heartbeat printed by ttsim_exit): whole program — device init + operand upload + program + read-back "
           "— on one global 1 GHz device timebase. No hardware, no watts. Rows whose knobs (math fidelity / fp32 "
           "accumulate / packer L1-accumulate) differ are NEVER a head-to-head: they live in "
           "unmatched_comparisons and may be quoted only with both fidelity labels.")

# What the work meter is, recorded beside the numbers so no reader has to take the counters on trust.
WORK_METER = {
    "instrument": "ttsim (Apache-2.0) + a counters-only patch: TTSIM_OP_STATS / TTSIM_DONE / TTSIM_STALLS",
    "timing_identical_to_stock": True,
    "timing_identity_evidence": "matmul_single_core reports 330268 cycles under the stock release .so and "
                                "under the instrumented build — the patch adds counters and moves no clock",
    "tracks_work": True,
    "evidence": "512x512xK, K swept 512->4096 (8x): tensix_inst 140048->1136400 (8.11x), matmul_retired "
                "262144->2097152 (8.000x, exact), noc_txns 4480->33152 (7.40x), noc_bytes 8914816->67635072 "
                "(7.59x) — while g_clock moved 5180->5184 and device_cycles 372->373",
    "fields": {
        "matmul_issued": "MVMUL issues INCLUDING the retry-spin an operand-starved core performs; a stall "
                         "meter, not a work meter — `matmul_retired` is the work",
        "matmul_retired": "MVMULs that actually retired. A CONSERVATION LAW: two programs computing the same "
                          "GEMM retire the same count, whoever emitted them, so every other counter beside it "
                          "is a like-for-like efficiency comparison and not a difference in what was computed",
        "elementwise": "SFPU/vector op issues",
        "tensix_inst": "Tensix instructions issued across the device",
        "noc_txns": "NOC transactions initiated",
        "noc_bytes": "bytes moved over the NOC",
        "stall_srca": "cycles a math op waited on source register A", "stall_srcb": "…on B", "stall_both": "…on both",
    },
    "consequence": "these are functions of the program, so these — never `cycles` — are what the cost model "
                   "is fitted to and what a compiler comparison may be made on",
    # ★ Instrument defects found by PREDICTING a program's work and being wrong. A meter nobody predicts
    # against is a meter nobody can catch, and this one was wrong by three orders of magnitude on a term
    # that is not neutral between stacks.
    "corrections": [
        {
            "date": "2026-09-10",
            "what": "noc_bytes counted an atomic's instruction word and an inline write's byte-enable mask "
                    "as if they were transfer lengths",
            "detail": "ttsim's `noc_at_len_be` register is a LENGTH only on reads, writes and multicasts. "
                      "On a semaphore atomic it carries NOC_AT_WRAP(31)|NOC_AT_INS_INCR_GET = 0x107C..0x107F, "
                      "which sits inside NOC_MAX_PACKET_SIZE and was being added as 4,222 bytes for a "
                      "transfer that moves FOUR. The meter charged every program for its handshakes at a "
                      "thousand times their weight, and it is not a neutral error: it flatters whichever "
                      "stack synchronizes less.",
            "found_by": "predicting the 512x512x512 multicast plan's NOC bytes from its choreography and "
                        "missing by exactly one atomic per receiver per K block",
            "effect": "512^3 multicast-split read 3,095,224 bytes before the fix and 2,622,240 after, "
                      "against 2,621,440 predicted from the program — a 0.03% agreement where there had "
                      "been 18%. Every row was re-measured afterwards.",
        },
    ],
}

STACK_DOC = {
    "logos_emitted": "the LOGOS compiler's own emitted tt-metal program; `generated` says which parts",
    "tt_metal_example": "a stock tt-metal programming example under tt_metal/programming_examples/matmul/, unmodified",
    "ttnn_production": "the vendor's production op ttnn.matmul called from Python under ttsim",
}


def refuse(msg):
    sys.stderr.write("REFUSING: " + msg + "\n")
    sys.exit(3)


def drift(msg):
    sys.stderr.write("VERIFY FAILED: " + msg + "\n")
    sys.exit(1)


def now_iso():
    return _dt.datetime.now(_dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def sha256_sources(paths):
    """sha256 over (relative name, content) pairs of every file named or found under `paths`, sorted."""
    files = []
    for p in paths:
        if os.path.isdir(p):
            for root, _dirs, names in os.walk(p):
                for n in names:
                    full = os.path.join(root, n)
                    files.append((os.path.relpath(full, p).replace(os.sep, "/"), full))
        elif os.path.isfile(p):
            files.append((os.path.basename(p), p))
        else:
            refuse("program source not found: %s" % p)
    h = hashlib.sha256()
    for rel, full in sorted(files):
        h.update(rel.encode() + b"\0")
        with open(full, "rb") as f:
            h.update(f.read())
        h.update(b"\0")
    return h.hexdigest()


def knobs_str(k):
    return "%s-fp32_%d-l1acc_%d" % (k["fidelity"], int(k["fp32_dest_acc"]), int(k["packer_l1_acc"]))


def shape_str(s):
    return "%dx%dx%d" % (s["m"], s["n"], s["k"])


def strategy_of(cfg_parsed):
    """The ledger's strategy name for a parsed LOGOS_MM_CFG: ttnn's two origins are different PROGRAMS and get
    distinct names (so they never share a key); everything else keeps the strategy the program printed."""
    if cfg_parsed is None:
        return None
    if cfg_parsed.get("origin") == ORIGIN_TTNN_DEFAULT:
        return "vendor_default"
    if cfg_parsed.get("origin") == ORIGIN_TTNN_OURS:
        return "vendor_our_config"
    return cfg_parsed["strategy"]


def knobs_of(fid, fp32, l1acc, approx=False):
    return {"fidelity": fid, "fp32_dest_acc": bool(fp32), "packer_l1_acc": bool(l1acc), "math_approx_mode": bool(approx)}


# ─────────────────────────────────────────────────────────────── log parsing
def parse_log(text):
    """Pure: everything the ledger needs from a ttsim run log."""
    out = {"env": None, "cycles": None, "device_cycles": None, "sim_seconds": None, "heartbeat_line": None, "errors": [], "cfg": None, "cfg_line": None,
           "pass": None, "pass_line": None, "pass_bare": False, "fail_line": None, "pcc_line": None,
           "ex_pcc": None, "ex_pass": False, "jit": None, "jit_line": None,
           "work": None, "work_line": None, "retired_line": None, "stalls_line": None}
    for raw in text.splitlines():
        line = raw.rstrip("\n")
        m = RE_ENV.match(line)
        if m:
            out["env"] = {"chip": m.group(1), "so": m.group(2), "so_sha256": m.group(3),
                          "ttsim_version": m.group(4), "tt_metal_sha": m.group(5)}
            continue
        m = RE_HEARTBEAT.match(line)
        if m:
            out["cycles"] = int(m.group(1))
            out["heartbeat_line"] = line
            out["sim_seconds"] = float(m.group(2))
            continue
        if RE_ERROR.match(line):
            out["errors"].append(line)
            continue
        m = RE_CFG.match(line)
        if m:
            cfg = None
            if m.group(2) is not None:
                cfg = {"grid": "%sx%s" % (m.group(2), m.group(3)), "per_core": "%sx%s" % (m.group(4), m.group(5)),
                       "out_block": "%sx%s" % (m.group(6), m.group(7)), "in0_block_w": int(m.group(8)),
                       "subblock": "%sx%s" % (m.group(9), m.group(10))}
                if m.group(12) is not None:
                    cfg["mesh"] = "%sx%s" % (m.group(12), m.group(13))
                    cfg["sites"] = int(m.group(14))
            out["cfg"] = {"strategy": m.group(1), "cfg": cfg, "origin": m.group(11)}
            out["cfg_line"] = line
            continue
        m = RE_PASS.match(line)
        if m:
            out["pass"] = {"knobs": knobs_of(m.group(1), m.group(2) == "1", m.group(3) == "1"),
                           "pcc": float(m.group(4)), "floor": float(m.group(5))}
            out["pass_line"] = line
            continue
        m = RE_DEVICE_CYCLES.match(line)
        if m:
            out["device_cycles"] = int(m.group(1))
            continue
        m = RE_WORK.match(line)
        if m:
            out["work"] = dict(out["work"] or {})
            out["work"].update({"matmul_issued": int(m.group(1)), "elementwise": int(m.group(2)),
                                "tensix_inst": int(m.group(3)), "noc_txns": int(m.group(4)),
                                "noc_bytes": int(m.group(5))})
            out["work_line"] = line
            continue
        m = RE_RETIRED.match(line)
        if m:
            out["work"] = dict(out["work"] or {})
            out["work"]["matmul_retired"] = int(m.group(1))
            out["retired_line"] = line
            continue
        m = RE_STALLS.match(line)
        if m:
            out["work"] = dict(out["work"] or {})
            out["work"].update({"stall_srca": int(m.group(1)), "stall_srcb": int(m.group(2)), "stall_both": int(m.group(3))})
            out["stalls_line"] = line
            continue
        if RE_PASS_BARE.match(line):
            out["pass_bare"] = True
            out["pass_line"] = line
            continue
        if RE_FAIL.match(line):
            out["fail_line"] = line
            continue
        m = RE_JIT.search(line)
        if m:
            out["jit"] = {"hits": int(m.group(1)), "total": int(m.group(2))}
            out["jit_line"] = line
            continue
        m = RE_EX_PCC.match(line)
        if m:
            out["ex_pcc"] = float(m.group(1))
            continue
        if RE_EX_PASS.match(line):
            out["ex_pass"] = True
            out["pass_line"] = line
            continue
        m = RE_PCC_LINE.search(line)
        if m:
            out["pcc_line"] = float(m.group(1))
    return out


def knobs_from_source(path):
    try:
        with open(path) as f:
            src = f.read()
    except OSError as e:
        refuse("--knobs-from-source %s: %s" % (path, e))
    m = RE_SRC_FID.search(src)
    if not m:
        refuse("no MathFidelity::<X> in %s — cannot derive the fidelity knob" % path)
    fid = m.group(1)
    line_no = src[: m.start()].count("\n") + 1
    fp = RE_SRC_FP32.search(src)
    l1 = RE_SRC_L1.search(src)
    fp32 = bool(fp and fp.group(1) == "true")
    l1acc = bool(l1 and l1.group(1) == "true")
    base = os.path.basename(path)
    source = ("grepped from %s: MathFidelity::%s at line %d; fp32_dest_acc_en %s; packer_l1_acc %s"
              % (base, fid, line_no, ("= " + fp.group(1)) if fp else "absent -> false",
                 ("= " + l1.group(1)) if l1 else "absent -> false"))
    floor_m = RE_SRC_PCC_FLOOR.search(src) or RE_SRC_LOGOS_FLOOR.search(src)
    floor = float(floor_m.group(1)) if floor_m else None
    return knobs_of(fid, fp32, l1acc), source, floor


def shapes_from_source(src):
    """All (M,N,K) triples a source can run, in order of appearance (every #ifdef branch)."""
    shapes, cur, first_line = [], {}, None
    for m in RE_SRC_SHAPE.finditer(src):
        name, val = m.group(1), int(m.group(2))
        if name == "M":
            cur = {}
            if first_line is None:
                first_line = src[: m.start()].count("\n") + 1
        cur[name] = val
        if all(k in cur for k in "MNK"):
            shapes.append({"m": cur["M"], "n": cur["N"], "k": cur["K"]})
            cur = {}
    return shapes, first_line


# ─────────────────────────────────────────────────────────────── ledger core
def empty_ledger():
    return {
        "_comment": COMMENT,
        "schema_version": 1,
        "generated_by": "scripts/tt-gemm-ledger.py",
        "provenance": PROVENANCE,
        "clock_ghz": 1.0,
        "cycle_definition": "last `[N] … seconds (… MHz|KHz)` heartbeat line of the ttsim run (ttsim src/sim.cpp ttsim_exit)",
        "clock_validity": CLOCK_VALIDITY,
        "stacks": STACK_DOC,
        "headline_key": None,
        "rows": [],
        "comparisons": [],
        "frontier": [],
        "unmatched_comparisons": [],
        "spatial": [],
        "headline": None,
        "vendor_example_shapes": {},
    }


def cfg_str(cfg):
    """The BLOCKING a row ran, canonically. Two runs of one program at different tilings are DIFFERENT
    measurements, not a re-measurement of the same one, so the blocking belongs in the identity: without it a
    tiling sweep silently supersedes its own best point and the frontier under-reports."""
    if not cfg:
        return "-"
    return ",".join("%s=%s" % (k, cfg[k]) for k in sorted(cfg) if k != "origin")


# Harness flags that PIN part of the plan. A row measured with any of them is evidence about the MACHINE —
# what the hardware can do at a chosen blocking. A row with none of them is evidence about the COMPILER —
# what it picks unaided. Only the second kind may back a sentence that says the compiler chooses something.
PIN_FLAGS = ("--block", "--per-core", "--packing", "--tiling", "--strategy", "--config ours")


def instrument(row):
    """The build a row was measured on. ★ THE INSTRUMENT IS PART OF THE MEASUREMENT: the patched build
    carries the work meter (`matmul_retired`, `noc_txns`, `noc_bytes`, `tensix_inst`) and the stock release
    carries none of it, so two runs under two builds are two experiments and neither may erase the other.

    Bringing Blackhole up, the same vendor example was run instrumented and then stock to prove the patch
    moves no clock (it does not — 309,062 cycles under both). The stock run, being newer, superseded the
    instrumented one and the chip's only vendor row lost its work meter. `rederive` already retires rows
    measured on a build the ledger has replaced; superseding has to agree with it or the two fight."""
    return (row["ttsim"]["so_sha256"], row["tt_metal_sha"])


def plan_source(row):
    """`compiler` when nothing about the plan was pinned on the command line, else `pinned`. Vendor rows are
    `vendor` — their plan is ttnn's own, whether or not we handed it a program config.

    ★ `--candidate` PINS THE ARRANGEMENT unless it is `best`. `--candidate fused` forces a multicast plan
    and `--candidate streamed` forces a streaming one, so a row measured that way shows what the machine
    can do, not what the compiler decided. Counting them as the compiler's own choice put four `streamed`
    and three blocked arrangements into `compiler_chosen` that the compiler never picked."""
    if row.get("stack") != "logos_emitted":
        return "vendor"
    repro = row.get("reproduce") or ""
    forced_candidate = any(("--candidate " + k) in repro for k in ("streamed", "fused", "unfused"))
    return "pinned" if forced_candidate or any(f in repro for f in PIN_FLAGS) else "compiler"


def key_of(chip, shape, stack, program, strategy, knobs, _cfg=None):
    """A row's identity, as `tt_sim_lock` and `tt_gemm_ledger_tool_lock` pin it: chip, shape, stack, program,
    strategy and knobs. The BLOCKING is deliberately not part of it, so re-running a program at a new tiling
    supersedes the old row — which means the ledger's CURRENT rows are the latest of each program, not the
    best. `frontier` therefore ranges over every row ever recorded, superseded ones included (each still
    carries its PASS, its PCC and its committed ttsim excerpt), so a tiling sweep cannot lose its own best
    point. Making the blocking part of the key would be the deeper fix and would re-specify those locks."""
    return "%s/%s/%s/%s/%s/%s" % (chip, shape_str(shape), stack, program, strategy, knobs_str(knobs))


def load_ledger(path):
    if not os.path.exists(path):
        return empty_ledger()
    with open(path) as f:
        return json.load(f)


def save_ledger(path, d):
    os.makedirs(os.path.dirname(os.path.abspath(path)) or ".", exist_ok=True)
    with open(path, "w") as f:
        json.dump(d, f, indent=2)
        f.write("\n")


def current_rows(d):
    return [r for r in d["rows"] if r.get("current")]


def is_single_core(r):
    cfg = r.get("cfg")
    return bool(cfg and cfg.get("grid") == "1x1") or "single_core" in r.get("program", "")


def knob_diff(a, b):
    out = []
    for k in KNOB_KEYS:
        if a[k] != b[k]:
            fa, fb = a[k], b[k]
            out.append("%s: %s vs %s" % (k, str(fa).lower() if isinstance(fa, bool) else fa,
                                         str(fb).lower() if isinstance(fb, bool) else fb))
    return out


def derive(d):
    """Recompute comparisons / unmatched_comparisons / spatial / headline from the CURRENT rows."""
    rows = current_rows(d)
    groups = {}
    for r in rows:
        key = (r["chip"], shape_str(r["shape"]), knobs_str(r["knobs"]))
        groups.setdefault(key, []).append(r)
    # The frontier ranges over EVERY recorded row, including rows a later run of the same program superseded:
    # a tiling sweep supersedes its own best point under this key, and the best cycle count ever measured is
    # not allowed to disappear because a slower tiling was measured afterwards.
    all_groups = {}
    for r in d["rows"]:
        key = (r["chip"], shape_str(r["shape"]), knobs_str(r["knobs"]))
        all_groups.setdefault(key, []).append(r)
    comparisons, unmatched, spatial, frontier = [], [], [], []
    # THE FRONTIER: per (chip, shape, knobs), the fewest cycles ANY program achieved, whoever achieved it,
    # and every stack's distance from it. This is the "minimal cycles" question stated so it cannot be argued:
    # a single monotone number per knob set, held by whichever program earned it, with the field measured
    # behind it. When our emitted program takes the frontier, it says so by data, not by editing.
    for (chip, shape, ks), grp in sorted(all_groups.items()):
        best = min(grp, key=lambda r: (r["cycles"], r["id"]))
        by_stack = {}
        for st in sorted({r["stack"] for r in grp}):
            b = min((r for r in grp if r["stack"] == st), key=lambda r: (r["cycles"], r["id"]))
            by_stack[st] = {"row": b["id"], "program": b["program"], "strategy": b["strategy"],
                            "cycles": b["cycles"], "over_frontier": b["cycles"] / best["cycles"]}
        # THE WHOLE FIELD, winners and losers alike. A choice the compiler REJECTED is the evidence that its
        # choices are measured rather than tuned, so a losing arrangement is never filtered out of the record —
        # it appears with its distance from the frontier, beside the arrangement that beat it.
        field = [
            {"row": r["id"], "stack": r["stack"], "program": r["program"], "strategy": r["strategy"],
             "cycles": r["cycles"], "over_frontier": r["cycles"] / best["cycles"],
             "cfg": r.get("cfg"), "current": bool(r.get("current")), "plan_source": plan_source(r),
             "fully_generated": bool(r.get("generated") and all(r["generated"].values()))}
            for r in sorted(grp, key=lambda r: (r["cycles"], r["id"]))
        ]
        frontier.append({
            "id": "%s/%s/%s" % (chip, shape, ks),
            "chip": chip, "shape": shape, "knobs": ks,
            "cycles": best["cycles"],
            "held_by": {"row": best["id"], "stack": best["stack"], "program": best["program"],
                        "strategy": best["strategy"], "program_origin": best["program_origin"],
                        "fully_generated": bool(best.get("generated") and all(best["generated"].values()))},
            "by_stack": by_stack,
            "field": field,
            # The row the COMPILER chose unaided — no blocking, packing or strategy pinned on the command
            # line. A pinned row shows what the machine can do; only this one shows what the compiler does.
            #
            # ★ THE CURRENT ONE, AND THE MOST RECENT OF THOSE — never the historically cheapest. The
            # frontier spans every row ever recorded, so ranking the candidates by `cycles` named a
            # SUPERSEDED row from a compiler that no longer exists: after the plan search moved to the
            # work model, 34 of 36 entries still pointed at 2026-09-03 blocked plans, and the lock that
            # checks "the ledger's plan is the plan the pipeline emits today" was comparing today's
            # compiler against last week's. It also ranked by the cycle counter, which the instrument
            # audit shows is not a function of the program — so a stale row could win on noise.
            "compiler_chosen": (lambda cands: {
                "row": cands[0]["id"], "program": cands[0]["program"], "strategy": cands[0]["strategy"],
                "cycles": cands[0]["cycles"], "over_frontier": cands[0]["cycles"] / best["cycles"],
                "fully_generated": bool(cands[0].get("generated") and all(cands[0]["generated"].values())),
            } if cands else None)(sorted((r for r in grp if plan_source(r) == "compiler" and r.get("current")),
                                         key=lambda r: (r["measured_at"], r["id"]), reverse=True)),
            "programs_measured": len(grp),
        })
    for (chip, shape, ks), grp in sorted(groups.items()):
        logos = [r for r in grp if r["stack"] == "logos_emitted"]
        if not logos:
            continue
        subject = min(logos, key=lambda r: (r["cycles"], r["id"]))
        vs = {}
        refs = [subject]
        ttnn_default = [r for r in grp if r["stack"] == "ttnn_production" and r["program_origin"] == ORIGIN_TTNN_DEFAULT]
        ttnn_ours = [r for r in grp if r["stack"] == "ttnn_production" and r["program_origin"] == ORIGIN_TTNN_OURS]
        # Two clocks coexist while the device counter is rolled out: the legacy whole-program g_clock, which
        # does not advance with work, and `device_cycles`, the core's own counter, which does. A ratio taken
        # across them is meaningless, so a mixed pair is REFUSED here rather than emitted and caught later —
        # the bad state should not be representable. The reason is recorded so the gap is visible.
        # Keyed on the metric the RATIO is computed from, not on which fields a row happens to carry. A row
        # may record `device_cycles` from an instrumentation experiment while the comparison still divides
        # `cycles` by `cycles` — same clock on both sides, nothing mixed. Reading field PRESENCE as field USE
        # refused four valid comparisons and left a published table rendering "—" and "0.00x".
        RATIO_METRIC = "cycles"

        def clock_of(r):
            return "device_cycles" if RATIO_METRIC == "device_cycles" and r.get("device_cycles") is not None else "g_clock"
        subj_clock = clock_of(subject)
        refused = {}
        for name, cands in (("ttnn_production_default", ttnn_default), ("ttnn_production_our_config", ttnn_ours)):
            if cands:
                o = min(cands, key=lambda r: (r["cycles"], r["id"]))
                if clock_of(o) != subj_clock:
                    # Kept OUT of `vs` entirely, not stored as a half-entry: every reader of `vs` is entitled
                    # to assume a comparison it can quote. The gap is recorded beside it so it stays visible.
                    refused[name] = {"row": o["id"],
                                     "why": "clock mismatch: subject on %s, reference on %s — re-measure the "
                                            "reference on the same clock, or drop the comparison"
                                            % (subj_clock, clock_of(o))}
                    continue
                ratio = subject["cycles"] / o["cycles"]
                # The verdict names an ORDERING, not a duration. `cycles` here comes from a simulator that
                # does not advance with work (see `clock_validity`), so calling one program "faster" would be
                # a claim about time that the measurement cannot support. Ratios stay — they identify and
                # order programs — but the words do not promise speed until the clock does.
                verdict = "fewer_cycles" if ratio < 1.0 else ("equal_cycles" if ratio == 1.0 else "more_cycles")
                vs[name] = {"row": o["id"], "cycles": o["cycles"], "ratio": ratio, "verdict": verdict,
                            "verdict_is_time_claim": False, "ratio_metric": RATIO_METRIC}
                refs.append(o)
        examples = {}
        for r in sorted((r for r in grp if r["stack"] == "tt_metal_example"), key=lambda r: r["program"]):
            ratio = subject["cycles"] / r["cycles"]
            examples[r["program"]] = {"row": r["id"], "cycles": r["cycles"], "ratio": ratio, "parity": ratio <= 1.0}
            refs.append(r)
        if examples:
            vs["tt_metal_example"] = examples
        if vs:
            same_sim = all(r["ttsim"]["so_sha256"] == subject["ttsim"]["so_sha256"] and r["tt_metal_sha"] == subject["tt_metal_sha"] for r in refs)
            comparisons.append({
                "id": "%s/%s/%s" % (chip, shape, ks), "shape": shape, "chip": chip, "knobs": subject["knobs"],
                "matched_numerics": True, "subject": subject["id"], "subject_cycles": subject["cycles"],
                "subject_generated": subject.get("generated"), "vs": vs, "refused": refused, "same_simulator": same_sim,
            })
        for o in rows:
            if o["stack"] == "logos_emitted" or o["chip"] != chip or shape_str(o["shape"]) != shape:
                continue
            if knobs_str(o["knobs"]) == ks and knob_diff(subject["knobs"], o["knobs"]) == []:
                continue
            unmatched.append({
                "subject": subject["id"], "other": o["id"], "other_stack": o["stack"], "other_origin": o["program_origin"],
                "matched_numerics": False, "why_unmatched": knob_diff(subject["knobs"], o["knobs"]),
                "ratio": subject["cycles"] / o["cycles"],
                "note": "NOT a head-to-head — numerics differ; may be quoted only with both fidelity labels",
            })
        singles = [r for r in grp if is_single_core(r) and r["id"] != subject["id"]]
        for sc in sorted(singles, key=lambda r: (r["stack"] != "logos_emitted", r["cycles"], r["id"]))[:1]:
            spatial.append({"chip": chip, "shape": shape, "knobs": ks, "single_core_row": sc["id"],
                            "single_core_stack": sc["stack"], "subject": subject["id"],
                            "speedup": sc["cycles"] / subject["cycles"], "cross_stack": sc["stack"] != "logos_emitted"})
    unmatched.sort(key=lambda u: (u["subject"], u["other"]))
    headline = None
    hk = d.get("headline_key")
    if hk:
        cid = "%s/%s/%s" % (hk["chip"], hk["shape"], hk["knobs"])
        c = next((c for c in comparisons if c["id"] == cid), None)
        if c:
            headline = {"comparison": cid, "subject": c["subject"]}
    frontier.sort(key=lambda f: f["id"])
    return {"comparisons": comparisons, "unmatched_comparisons": unmatched, "spatial": spatial,
            "frontier": frontier, "headline": headline,
            # Derived, not stored, so an existing ledger picks it up on `rederive` and cannot sit there
            # quietly asserting a validity it was written before anyone checked.
            "provenance": PROVENANCE,
            "clock_validity": CLOCK_VALIDITY, "work_meter": WORK_METER}


REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def tt_metal_home_here():
    """This machine's tt-metal tree: `$TT_METAL_HOME`, else `~/tt-metal` — the tree every measurement builds in."""
    return os.path.realpath(os.environ.get("TT_METAL_HOME") or os.path.expanduser("~/tt-metal"))


def portable(text):
    """`text` with this machine's paths spelled portably: its tt-metal tree as `$TT_METAL_HOME`, the repository as
    `$LOGOS_ROOT`, its home directory as `~`. Nothing the ledger parses reads a path, so an excerpt re-parses to the
    same facts either way — and a checked-in file names no one's home. A path under none of them (a fixture tree) is
    left as it is."""
    subs = [(tt_metal_home_here(), "$TT_METAL_HOME"), (os.path.realpath(REPO_ROOT), "$LOGOS_ROOT"), (os.path.realpath(os.path.expanduser("~")), "~")]
    for path, name in sorted(subs, key=lambda s: -len(s[0])):
        if len(path) > 1:
            text = text.replace(path, name)
    return text


def local(path):
    """A path recorded by [`portable`], resolved on this machine."""
    if path.startswith("$TT_METAL_HOME"):
        return tt_metal_home_here() + path[len("$TT_METAL_HOME"):]
    if path.startswith("$LOGOS_ROOT"):
        return REPO_ROOT + path[len("$LOGOS_ROOT"):]
    return os.path.expanduser(path)


def excerpt_of(text):
    lines = text.splitlines()
    if len(lines) <= HEAD_LINES + TAIL_LINES:
        return "\n".join(lines) + "\n"
    elided = len(lines) - HEAD_LINES - TAIL_LINES
    return "\n".join(lines[:HEAD_LINES] + ["... (%d lines elided by tt-gemm-ledger) ..." % elided] + lines[-TAIL_LINES:]) + "\n"


def check_row_against_excerpt(row, ledger_dir):
    """Re-parse the committed excerpt; return a list of drift messages (empty = consistent)."""
    path = os.path.join(ledger_dir, row["log"])
    if not os.path.exists(path):
        return ["row %s: excerpt %s missing" % (row["id"], row["log"])]
    with open(path) as f:
        p = parse_log(f.read())
    msgs = []
    if p["errors"]:
        msgs.append("row %s: excerpt carries a ttsim ERROR line: %s" % (row["id"], p["errors"][0]))
    if p["cycles"] != row["cycles"]:
        msgs.append("row %s: cycles %s in the ledger but the excerpt heartbeat says %s" % (row["id"], row["cycles"], p["cycles"]))
    if p.get("work") != row.get("work"):
        msgs.append("row %s: work meter %s in the ledger but the excerpt re-parses to %s" % (row["id"], row.get("work"), p.get("work")))
    if p["jit"] is None or p["jit"] != row.get("jit_cache"):
        msgs.append("row %s: the excerpt's JIT cache line (%s) disagrees with the row's jit_cache %s" % (row["id"], p["jit"], row.get("jit_cache")))
    if row.get("jit_cache") and row["jit_cache"]["hits"] < row["jit_cache"]["total"]:
        msgs.append("row %s: recorded with a cold JIT cache (%s) — inflated cycles" % (row["id"], row["jit_cache"]))
    env = p["env"]
    if not env:
        msgs.append("row %s: excerpt lacks the LOGOS_TT_ENV banner" % row["id"])
    else:
        if env["so_sha256"] != row["ttsim"]["so_sha256"] or env["tt_metal_sha"] != row["tt_metal_sha"] or env["chip"] != row["chip"]:
            msgs.append("row %s: excerpt banner (chip=%s so_sha256=%s tt_metal_sha=%s) disagrees with the row"
                        % (row["id"], env["chip"], env["so_sha256"], env["tt_metal_sha"]))
    if row["stack"] == "tt_metal_example":
        if not p["ex_pass"] or p["ex_pcc"] is None:
            msgs.append("row %s: excerpt lacks `Test Passed` + `Metalium vs Golden -- PCC`" % row["id"])
        elif p["ex_pcc"] != row["pcc"]:
            msgs.append("row %s: pcc %s in the ledger but the excerpt says %s" % (row["id"], row["pcc"], p["ex_pcc"]))
    elif p["pass"]:
        if p["pass"]["knobs"] != row["knobs"] or p["pass"]["pcc"] != row["pcc"] or p["pass"]["floor"] != row["pcc_floor"]:
            msgs.append("row %s: the excerpt PASS line (%s) disagrees with the row's knobs/pcc/floor" % (row["id"], p["pass_line"]))
    elif p["pass_bare"]:
        if row["knobs_source"] == "LOGOS_MM_PASS line printed by the program":
            msgs.append("row %s: a bare PASS cannot have sourced its knobs from the PASS line" % row["id"])
        if p["pcc_line"] != row["pcc"]:
            msgs.append("row %s: pcc %s in the ledger but the excerpt's `-- PCC =` line says %s" % (row["id"], row["pcc"], p["pcc_line"]))
    else:
        msgs.append("row %s: excerpt carries no PASS line" % row["id"])
    if row["stack"] != "tt_metal_example":
        if not p["cfg"]:
            msgs.append("row %s: excerpt lacks the LOGOS_MM_CFG line" % row["id"])
        elif strategy_of(p["cfg"]) != row["strategy"] or p["cfg"]["cfg"] != row["cfg"]:
            msgs.append("row %s: excerpt CFG line disagrees with the row's strategy/cfg" % row["id"])
    return msgs


def verify_ledger(d, ledger_dir):
    msgs = []
    if d.get("provenance") != PROVENANCE:
        msgs.append("top-level provenance must be %s" % PROVENANCE)
    seen_current = {}
    for r in d["rows"]:
        for k in ("id", "key", "current", "shape", "chip", "stack", "program", "program_origin", "strategy", "cfg",
                  "knobs", "knobs_source", "cycles", "pcc", "pcc_floor", "pass", "pass_line", "reproduce", "ttsim",
                  "tt_metal_sha", "measured_at", "program_sha256", "log", "log_full_sha256", "provenance"):
            if k not in r:
                msgs.append("row %s lacks `%s`" % (r.get("id"), k))
        if r.get("stack") not in STACKS:
            msgs.append("row %s: unknown stack %s" % (r.get("id"), r.get("stack")))
        if r.get("provenance") != PROVENANCE:
            msgs.append("row %s: provenance must be %s" % (r.get("id"), PROVENANCE))
        if not r.get("pass"):
            msgs.append("row %s: pass must be true" % r.get("id"))
        if r.get("pcc", 0) < r.get("pcc_floor", 1):
            msgs.append("row %s: pcc %s below its floor %s" % (r.get("id"), r.get("pcc"), r.get("pcc_floor")))
        if r.get("knobs", {}).get("fidelity") not in FIDELITIES:
            msgs.append("row %s: bad fidelity" % r.get("id"))
        exp_key = key_of(r["chip"], r["shape"], r["stack"], r["program"], r["strategy"], r["knobs"], r.get("cfg"))
        if r.get("key") != exp_key:
            msgs.append("row %s: key %s != expected %s" % (r.get("id"), r.get("key"), exp_key))
        if r.get("id") != "%s/%s" % (r.get("key"), r.get("measured_at")):
            msgs.append("row %s: id must be key/measured_at" % r.get("id"))
        if r.get("current"):
            ck = (r["key"], plan_source(r), instrument(r))
            if ck in seen_current:
                msgs.append("two current rows share key %s as %s on simulator %s"
                            % (r["key"], plan_source(r), instrument(r)[0][:12]))
            seen_current[ck] = r["id"]
        if r.get("cfg") is None and not (r["stack"] == "tt_metal_example" or r.get("program_origin") == ORIGIN_TTNN_DEFAULT):
            msgs.append("row %s: cfg may be null only for tt-metal examples and the ttnn default heuristic" % r.get("id"))
        if r["stack"] == "logos_emitted" and not isinstance(r.get("generated"), dict):
            msgs.append("row %s: logos_emitted rows must carry `generated`" % r.get("id"))
        if r["stack"] == "tt_metal_example":
            ves = d.get("vendor_example_shapes", {}).get(r["program"])
            if not ves or shape_str(r["shape"]) not in ves["shapes"]:
                msgs.append("row %s: example %s cannot run %s" % (r.get("id"), r["program"], shape_str(r["shape"])))
        msgs.extend(check_row_against_excerpt(r, ledger_dir))
    exp = derive(d)
    for k in ("comparisons", "unmatched_comparisons", "spatial", "frontier", "headline"):
        if d.get(k) != exp[k]:
            msgs.append("`%s` is not what the rows derive: stored %s vs derived %s"
                        % (k, json.dumps(d.get(k), sort_keys=True)[:400], json.dumps(exp[k], sort_keys=True)[:400]))
    return msgs


# ─────────────────────────────────────────────────────────────── commands
def cmd_scan_examples(args):
    d = load_ledger(args.ledger)
    home = os.path.abspath(args.tt_metal_home)
    base = os.path.join(home, "tt_metal", "programming_examples", "matmul")
    if not os.path.isdir(base):
        refuse("no tt-metal matmul examples under %s" % base)
    found = {}
    for name in sorted(os.listdir(base)):
        cpp = os.path.join(base, name, name + ".cpp")
        if not os.path.isfile(cpp):
            continue
        with open(cpp) as f:
            src = f.read()
        shapes, first_line = shapes_from_source(src)
        if not shapes:
            continue
        m = RE_SRC_FID.search(src)
        if not m:
            refuse("example %s declares no MathFidelity" % name)
        fp = RE_SRC_FP32.search(src)
        l1 = RE_SRC_L1.search(src)
        floor_m = RE_SRC_PCC_FLOOR.search(src)
        rel = os.path.relpath(cpp, home).replace(os.sep, "/")
        sources = [cpp]
        kdir = os.path.join(base, name, "kernels")
        if os.path.isdir(kdir):
            sources.append(kdir)
        found[name] = {
            "shapes": [shape_str(s) for s in shapes],
            "source": "%s:%d" % (rel, first_line),
            "knobs": knobs_of(m.group(1), bool(fp and fp.group(1) == "true"), bool(l1 and l1.group(1) == "true")),
            "knobs_source": "grepped from %s: MathFidelity::%s; fp32_dest_acc_en %s; packer_l1_acc %s"
                            % (rel, m.group(1), ("= " + fp.group(1)) if fp else "absent -> false", ("= " + l1.group(1)) if l1 else "absent -> false"),
            "pcc_floor": float(floor_m.group(1)) if floor_m else None,
            "sources": [portable(s) for s in sources],
            "tt_metal_home": portable(home),
        }
    if not found:
        refuse("no matmul example with a constexpr M/N/K shape found under %s" % base)
    d["vendor_example_shapes"] = found
    d.update(derive(d))
    msgs = verify_ledger(d, os.path.dirname(os.path.abspath(args.ledger)))
    if msgs:
        drift("\n".join(msgs))
    save_ledger(args.ledger, d)
    print("scanned %d tt-metal matmul examples into %s" % (len(found), args.ledger))


def cmd_add(args):
    if args.exit_code != 0:
        refuse("the run exited with code %d — only a clean, passing run can be recorded" % args.exit_code)
    if args.stack not in STACKS:
        refuse("unknown stack %s (one of %s)" % (args.stack, ", ".join(STACKS)))
    if args.chip not in CHIPS:
        refuse("unknown chip %s (one of %s)" % (args.chip, ", ".join(CHIPS)))
    try:
        with open(args.log) as f:
            text = f.read()
    except OSError as e:
        refuse("cannot read log %s: %s" % (args.log, e))
    p = parse_log(text)
    if p["errors"]:
        refuse("ttsim reported ERROR: %s" % p["errors"][0])
    if p["cycles"] is None:
        refuse("no ttsim exit heartbeat (`[N] X.X seconds (Y.Y MHz)`) in %s — no cycle count to record" % args.log)
    if not p["env"]:
        refuse("no LOGOS_TT_ENV banner in the log — run through scripts/tt-sim.sh so the simulator + tt-metal identities are recorded")
    if p["env"]["chip"] != args.chip:
        refuse("--chip %s but the run banner says chip=%s" % (args.chip, p["env"]["chip"]))
    if p["fail_line"]:
        refuse("the program reported %s" % p["fail_line"])
    if p["jit"] is None:
        refuse("no `JIT cache stats: N/N hits` line in the log — the kernel-cache state is part of the measurement (a cold "
               "compile inflates whole-program cycles); run through scripts/tt-sim.sh so tt-metal's BuildKernels line is captured")
    if p["jit"]["hits"] < p["jit"]["total"]:
        refuse("JIT cache %d/%d hits: kernels were compiled DURING the run, which inflates the device cycle count "
               "(measured: 9,735 warm vs 13,543 cold for the same program) — re-run to warm the cache and record the warm run"
               % (p["jit"]["hits"], p["jit"]["total"]))
    d = load_ledger(args.ledger)
    ledger_dir = os.path.dirname(os.path.abspath(args.ledger))
    shape = {"m": args.shape[0], "n": args.shape[1], "k": args.shape[2]}
    measured_at = args.now or now_iso()
    generated = None
    knobs = knobs_source = pcc = floor = None
    pass_line = None
    origin = args.program_origin
    program_srcs = list(args.program_src or [])
    strategy, cfg = None, None

    if args.stack == "tt_metal_example":
        ves = d.get("vendor_example_shapes", {}).get(args.program)
        if not ves:
            refuse("no scan of tt-metal example `%s` — run `scan-examples --tt-metal-home DIR` first so the shape it can run is known" % args.program)
        if shape_str(shape) not in ves["shapes"]:
            refuse("tt-metal example %s cannot run %s — its source runs only %s (%s)"
                   % (args.program, shape_str(shape), ", ".join(ves["shapes"]), ves["source"]))
        if not p["ex_pass"] or p["ex_pcc"] is None:
            refuse("example run lacks `Test Passed` + `Metalium vs Golden -- PCC = …`")
        knobs, knobs_source, floor = dict(ves["knobs"]), ves["knobs_source"], ves["pcc_floor"]
        if floor is None:
            refuse("example %s has no TT_FATAL(pearson > X) floor in its source" % args.program)
        pcc, pass_line = p["ex_pcc"], p["pass_line"]
        origin = origin or ("tt-metal programming example " + args.program)
        strategy, cfg = "example", None
        if not program_srcs:
            program_srcs = [local(s) for s in ves["sources"]]
    else:
        if not p["cfg"]:
            refuse("no LOGOS_MM_CFG line in the log — the program must print its strategy/config")
        strategy, cfg = p["cfg"]["strategy"], p["cfg"]["cfg"]
        if args.stack == "logos_emitted":
            if cfg is None:
                refuse("a logos_emitted row needs the full LOGOS_MM_CFG (grid/per_core/out_block/in0_block_w/subblock)")
            if not origin:
                refuse("--program-origin is required for logos_emitted rows")
            if not args.generated:
                refuse("--generated compute,dataflow,host (the parts the compiler generated) is required for logos_emitted rows")
            parts = {x.strip() for x in args.generated.split(",") if x.strip()}
            bad = parts - {"compute", "dataflow", "host"}
            if bad:
                refuse("unknown --generated part(s): %s" % ", ".join(sorted(bad)))
            generated = {"compute": "compute" in parts, "dataflow": "dataflow" in parts, "host": "host" in parts}
        else:
            cfg_origin = p["cfg"]["origin"]
            if not cfg_origin:
                refuse("a ttnn_production run must print origin=\"…\" on its LOGOS_MM_CFG line")
            if origin and origin != cfg_origin:
                refuse("--program-origin %r disagrees with the run's origin %r" % (origin, cfg_origin))
            origin = cfg_origin
            if origin not in (ORIGIN_TTNN_DEFAULT, ORIGIN_TTNN_OURS):
                refuse("ttnn origin must be %r or %r, got %r" % (ORIGIN_TTNN_DEFAULT, ORIGIN_TTNN_OURS, origin))
            if origin == ORIGIN_TTNN_OURS and cfg is None:
                refuse("a ttnn run with the LOGOS program config must print the full config")
            strategy = strategy_of(p["cfg"])
        if not program_srcs:
            refuse("--program-src PATH... (the program sources to hash) is required")
        if p["pass"]:
            knobs, pcc, floor = p["pass"]["knobs"], p["pass"]["pcc"], p["pass"]["floor"]
            knobs_source, pass_line = "LOGOS_MM_PASS line printed by the program", p["pass_line"]
        elif p["pass_bare"]:
            if not args.knobs_from_source:
                refuse("a bare LOGOS_MM_PASS carries no fidelity/fp32/l1acc — pass --knobs-from-source FILE (the host that hardcodes them)")
            if p["pcc_line"] is None:
                refuse("a bare LOGOS_MM_PASS needs a `-- PCC = X` line to record the PCC")
            knobs, knobs_source, src_floor = knobs_from_source(args.knobs_from_source)
            pcc, floor, pass_line = p["pcc_line"], (src_floor if src_floor is not None else 0.99), p["pass_line"]
        else:
            refuse("no LOGOS_MM_PASS line in the log — the run did not pass its own golden check")
    if args.requested_knobs:
        parts = [x.strip() for x in args.requested_knobs.split(",")]
        if len(parts) != 3 or parts[0] not in FIDELITIES or parts[1] not in ("0", "1") or parts[2] not in ("0", "1"):
            refuse("--requested-knobs must be FIDELITY,fp32(0|1),l1acc(0|1)")
        req = knobs_of(parts[0], parts[1] == "1", parts[2] == "1")
        diff = knob_diff(req, knobs)
        if diff:
            refuse("the harness requested %s but the program reported %s (%s) — the numerics confound is exactly what this ledger refuses to record silently"
                   % (knobs_str(req), knobs_str(knobs), "; ".join(diff)))
    if args.shape_from_source:
        with open(args.shape_from_source) as f:
            shapes, _ = shapes_from_source(f.read())
        if shape_str(shape) not in [shape_str(s) for s in shapes]:
            refuse("--shape %s is not a shape %s declares (%s)" % (shape_str(shape), args.shape_from_source, ", ".join(shape_str(s) for s in shapes)))
    if pcc < floor:
        refuse("pcc %s is below the floor %s" % (pcc, floor))
    scaffold = None
    if args.scaffold_log:
        with open(args.scaffold_log) as f:
            sp = parse_log(f.read())
        if sp["cycles"] is None or sp["errors"]:
            refuse("scaffold log has no clean heartbeat")
        if sp["jit"] is None or sp["jit"]["hits"] < sp["jit"]["total"]:
            refuse("scaffold run must be JIT-warm too")
        scaffold = sp["cycles"]

    key = key_of(args.chip, shape, args.stack, args.program, strategy, knobs, cfg)
    row_id = "%s/%s" % (key, measured_at)
    excerpt_dir = args.log_excerpt_dir or os.path.join(ledger_dir, "ttsim_logs")
    slug = key.replace("/", "__") + "__" + measured_at.replace("-", "").replace(":", "")
    excerpt_rel = os.path.relpath(os.path.join(excerpt_dir, slug + ".log"), ledger_dir).replace(os.sep, "/")
    excerpt = portable(excerpt_of(text))
    for needed in [p["heartbeat_line"], pass_line, p["jit_line"], p["cfg_line"] if args.stack != "tt_metal_example" else None,
                   p["work_line"], p["retired_line"], p["stalls_line"]]:
        if needed and portable(needed) not in excerpt:
            refuse("the log excerpt (first %d + last %d lines) would not contain `%s`" % (HEAD_LINES, TAIL_LINES, needed))
    if "LOGOS_TT_ENV chip=" not in excerpt:
        refuse("the log excerpt would not contain the LOGOS_TT_ENV banner")

    row = {
        "id": row_id, "key": key, "current": True, "superseded_by": None,
        "shape": shape, "dtype": "bf16", "chip": args.chip, "stack": args.stack, "program": args.program,
        "program_origin": origin, "generated": generated, "strategy": strategy, "cfg": cfg,
        "knobs": knobs, "knobs_source": knobs_source,
        "cycles": p["cycles"], "device_cycles": p.get("device_cycles"), "work": p.get("work"),
        "sim_seconds": p.get("sim_seconds"), "jit_cache": p["jit"], "scaffold_cycles": scaffold,
        "program_cycles": (p["cycles"] - scaffold) if scaffold is not None else None,
        "pcc": pcc, "pcc_floor": floor, "pass": True, "pass_line": pass_line,
        "input_fill": args.input_fill,
        "reproduce": args.reproduce,
        "ttsim": {"so": p["env"]["so"], "so_sha256": p["env"]["so_sha256"], "version": p["env"]["ttsim_version"],
                  "version_source": "LOGOS_TT_ENV banner printed by scripts/tt-sim.sh"},
        "tt_metal_sha": p["env"]["tt_metal_sha"],
        "measured_at": measured_at,
        "program_sha256": sha256_sources(program_srcs),
        "log": excerpt_rel, "log_full_sha256": hashlib.sha256(text.encode()).hexdigest(),
        "provenance": PROVENANCE,
    }
    # ★ A PINNED ROW NEVER SUPERSEDES A COMPILER-CHOSEN ONE, even when they print the same plan.
    #
    # They are different EXPERIMENTS: one shows what the machine can do when told exactly what to do, the
    # other shows what the compiler decides unaided. Once the planner got good enough that `--candidate
    # best` and `--candidate fused` reach the same arrangement, they started landing on the same key — and
    # whichever ran last erased the other. The record that the compiler CHOSE the plan is the thing every
    # "beats the vendor" claim rests on, and it was being overwritten by a calibration run.
    src, inst = plan_source(row), instrument(row)
    for r in d["rows"]:
        if r["key"] == key and r.get("current") and plan_source(r) == src and instrument(r) == inst:
            r["current"] = False
            r["superseded_by"] = row_id
    d["rows"].append(row)
    if args.headline:
        d["headline_key"] = {"shape": shape_str(shape), "chip": args.chip, "knobs": knobs_str(knobs)}
    os.makedirs(os.path.dirname(os.path.join(ledger_dir, excerpt_rel)), exist_ok=True)
    with open(os.path.join(ledger_dir, excerpt_rel), "w") as f:
        f.write(excerpt)
    d.update(derive(d))
    msgs = verify_ledger(d, ledger_dir)
    if msgs:
        os.remove(os.path.join(ledger_dir, excerpt_rel))
        drift("the ledger would not verify after this add:\n" + "\n".join(msgs))
    save_ledger(args.ledger, d)
    print("recorded %s: %d cycles, pcc %s, %s" % (row_id, row["cycles"], row["pcc"], knobs_str(knobs)))


def cmd_rederive(args):
    """Recompute the derived blocks from the CURRENT rows and save. A row's own fields are only ever
    RE-PARSED from its committed log excerpt, never typed — this exists so a schema addition (the work
    meter) can be filled from evidence already in the repo without re-measuring anything."""
    d = load_ledger(args.ledger)
    ledger_dir = os.path.dirname(os.path.abspath(args.ledger))
    # ★ A ROW FROM A RETIRED INSTRUMENT IS NOT A CURRENT MEASUREMENT.
    #
    # `current` normally turns off when a NEWER ROW takes the same key. That misses a whole class: when the
    # simulator itself is replaced, a row measured on the old build keeps its key (nothing re-measured that
    # exact arrangement) and stays current forever — and then it enters comparisons beside rows from the new
    # build, which `tt_sim_lock` refuses outright ("every row must come from the same simulator build and
    # tt-metal commit"). Five 2026-09-02 rows sat like that after the instrumented simulator landed, and
    # re-running them did not help: the new planner emits a DIFFERENT arrangement, so the fresh rows took
    # different keys and the stale ones were never superseded.
    #
    # The instrument is part of the measurement. A row taken on a build the ledger no longer uses is history,
    # not evidence about today, so it is retired here — kept, readable, and out of every comparison.
    # Re-derive `current` per (key, plan_source, instrument): the newest row of each is the live one. This heals
    # a ledger where a pinned run overwrote the compiler's own choice before the rule above existed.
    groups = {}
    for r in d["rows"]:
        groups.setdefault((r["key"], plan_source(r), instrument(r)), []).append(r)
    restored = 0
    for _, g in groups.items():
        g.sort(key=lambda r: (r["measured_at"], r["id"]))
        for r in g[:-1]:
            r["current"] = False
        live = g[-1]
        if not live.get("current"):
            live["current"] = True
            live["superseded_by"] = None
            restored += 1
    if restored:
        print("restored %d row(s) a same-key run of a DIFFERENT plan source had superseded" % restored)
    # ★ THE LIVE BUILD IS PER CHIP. A chip's simulator is its own object — `libttsim_wh.so` and
    # `libttsim_bh.so` have different contents and therefore different shas BY CONSTRUCTION — so a single
    # global "newest build" means the newest chip measured retires every row of every other chip. Bringing
    # Blackhole up retired all three of its rows on the first rederive, including the one that had just
    # proven the instrument moves no clock, because the newest row in the ledger was a Wormhole one.
    #
    # What retirement is actually for is a build REPLACED for that chip: the same simulator rebuilt, or
    # tt-metal moved. Compared per chip it says that and nothing else.
    live_build = {}
    for r in d["rows"]:
        if not r.get("current"):
            continue
        c = r["chip"]
        if c not in live_build or r.get("measured_at", "") > live_build[c][0]:
            live_build[c] = (r.get("measured_at", ""), instrument(r))
    retired = 0
    for r in d["rows"]:
        # A row that is live again must not still carry the reason it was once retired — a stale field
        # asserting something untrue is the same defect as a stale comment.
        if r.get("current") and r.get("retired_reason"):
            r.pop("retired_reason", None)
        live = live_build.get(r["chip"])
        if r.get("current") and live and instrument(r) != live[1]:
            r["current"] = False
            r["retired_reason"] = "measured on simulator %s / tt-metal %s, which the ledger has replaced for %s" % (
                r["ttsim"]["so_sha256"][:12], r["tt_metal_sha"][:12], r["chip"])
            retired += 1
    if retired:
        print("retired %d row(s) measured on a simulator build the ledger no longer uses for their chip" % retired)
    filled = 0
    for r in d["rows"]:
        if r.get("work") is not None:
            continue
        path = os.path.join(ledger_dir, r["log"])
        if not os.path.exists(path):
            continue
        with open(path) as f:
            w = parse_log(f.read()).get("work")
        if w:
            r["work"] = w
            filled += 1
    if filled:
        print("backfilled the work meter onto %d rows by re-parsing their committed excerpts" % filled)
    d.update(derive(d))
    save_ledger(args.ledger, d)
    print("rederived %s: %d rows (%d current), %d comparisons, %d frontier entries over every row ever recorded"
          % (args.ledger, len(d["rows"]), len(current_rows(d)), len(d["comparisons"]), len(d.get("frontier", []))))


def cmd_verify(args):
    if not os.path.exists(args.ledger):
        drift("ledger %s does not exist" % args.ledger)
    d = load_ledger(args.ledger)
    msgs = verify_ledger(d, os.path.dirname(os.path.abspath(args.ledger)))
    if msgs:
        drift("\n".join(msgs))
    print("verified %s: %d rows, %d matched comparisons, %d unmatched pairs"
          % (args.ledger, len(d["rows"]), len(d["comparisons"]), len(d["unmatched_comparisons"])))


def cmd_show(args):
    d = load_ledger(args.ledger)
    rows = {r["id"]: r for r in d["rows"]}
    if not d["comparisons"]:
        print("no matched-numerics comparisons yet (%d rows)" % len(d["rows"]))
    for c in d["comparisons"]:
        s = rows[c["subject"]]
        print("%s  [%s]" % (c["id"], "HEADLINE" if d.get("headline") and d["headline"]["comparison"] == c["id"] else "matched"))
        print("  subject  %-9d %-10s %s (%s)" % (s["cycles"], knobs_str(s["knobs"]), s["program_origin"], s["strategy"]))
        for name, v in c["vs"].items():
            if name == "tt_metal_example":
                for prog, e in v.items():
                    print("  vs %-24s %-9d ratio %.3f %s" % (prog, e["cycles"], e["ratio"], "parity" if e["parity"] else "example faster"))
            else:
                print("  vs %-24s %-9d ratio %.3f %s" % (name, v["cycles"], v["ratio"], v["verdict"]))
    for u in d["unmatched_comparisons"]:
        o = rows[u["other"]]
        print("  UNMATCHED vs %s (%s): %d cycles, ratio %.3f — %s" % (o["program_origin"], knobs_str(o["knobs"]), o["cycles"], u["ratio"], "; ".join(u["why_unmatched"])))


def main(argv=None):
    ap = argparse.ArgumentParser(prog="tt-gemm-ledger.py", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    a = sub.add_parser("add", help="record one ttsim run")
    a.add_argument("--ledger", required=True)
    a.add_argument("--stack", required=True, choices=STACKS)
    a.add_argument("--log", required=True, help="the FULL run log (stdout+stderr of scripts/tt-sim.sh)")
    a.add_argument("--exit-code", type=int, required=True)
    a.add_argument("--program", required=True)
    a.add_argument("--program-origin", default=None)
    a.add_argument("--generated", default=None, help="logos_emitted only: comma list of compute,dataflow,host")
    a.add_argument("--shape", type=int, nargs=3, required=True, metavar=("M", "N", "K"))
    a.add_argument("--chip", required=True, choices=CHIPS)
    a.add_argument("--reproduce", required=True)
    a.add_argument("--program-src", nargs="+", default=None)
    a.add_argument("--knobs-from-source", default=None)
    a.add_argument("--shape-from-source", default=None)
    a.add_argument("--requested-knobs", default=None, help="FIDELITY,fp32(0|1),l1acc(0|1) the harness asked for")
    a.add_argument("--scaffold-log", default=None)
    a.add_argument("--headline", action="store_true")
    a.add_argument("--now", default=None)
    a.add_argument("--log-excerpt-dir", default=None)
    a.add_argument("--input-fill", default="deterministic bf16 fill, golden = fp32 host matmul")
    a.set_defaults(fn=cmd_add)
    sc = sub.add_parser("scan-examples", help="record the shapes/knobs the tt-metal matmul examples can run")
    sc.add_argument("--ledger", required=True)
    sc.add_argument("--tt-metal-home", required=True)
    sc.set_defaults(fn=cmd_scan_examples)
    rd = sub.add_parser("rederive", help="recompute every derived block from the rows and save (for a schema addition)")
    rd.add_argument("--ledger", required=True)
    rd.set_defaults(fn=cmd_rederive)
    v = sub.add_parser("verify", help="re-derive + re-parse everything; exit 1 on drift")
    v.add_argument("--ledger", required=True)
    v.set_defaults(fn=cmd_verify)
    sh = sub.add_parser("show", help="print the matched comparisons")
    sh.add_argument("--ledger", required=True)
    sh.set_defaults(fn=cmd_show)
    args = ap.parse_args(argv)
    args.fn(args)


if __name__ == "__main__":
    main()
