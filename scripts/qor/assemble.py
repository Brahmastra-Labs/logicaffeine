#!/usr/bin/env python3
"""Join the comparative-synthesis bench's raw ledgers into benchmarks/silicon/comparative_synthesis.json — the
only writer of that file.

    assemble.py            write the joined ledger
    assemble.py verify     re-derive every joined number from the raw ledgers and compare

One row per kernel and candidate: the candidate's functional gate (benchmarks/silicon/qor_function.json: both
stimulus modes, the signature against the golden, cycles per result), its Artix-7 open-flow results per mapping
(benchmarks/silicon/qor_artix7.json: every placement seed, the median, minimum and maximum post-route frequency
of the seeds that routed, the candidate's own cells, the seeds that were refused and why, and whether each
critical path lies inside the candidate), the throughput those imply, its Sky130 attempts with its own minimum period and its kernel's iso period
(benchmarks/silicon/qor_sky130.json, staged by qor_sky130_stage.json), and its provenance (the tool, the source it
read, the command that generated it, its interface). A candidate a tool refused to build (the manifest's
`refusals`) has a row that says so. Nothing here is measured: every number is copied or derived from a raw
ledger row that carries its own tool excerpt.
"""
import json
import os
import statistics
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import qorlib  # noqa: E402

OUT = os.path.join(qorlib.SIL, "comparative_synthesis.json")
MANIFEST = os.path.join(qorlib.SIL, "qor", "candidates.json")
TIERS = ["SIM", "SIMULATED_TUNGSTEN", "MODELED", "SYN", "PNR-open", "GDS", "GDS-VCD", "GDS-default-activity", "BOARD"]
KERNELS = {
    "K1": ("decide", "LOGOS compiler (compiled from .lg)"),
    "K2": ("horner", "LOGOS compiler (compiled from .lg)"),
    "K3": ("gemm8", "LOGOS RTL generator"),
    "K4": ("keccak_f1600", "LOGOS RTL generator"),
    "K5": ("ntt_bfly", "LOGOS RTL generator"),
    "K6": ("simt_axpb_sum", "LOGOS compiler (compiled from .lg)"),
    "K7": ("ddr2_controller", "LOGOS RTL generator"),
}
HEADER = {
    "_generated_by": "scripts/qor/assemble.py (never hand-edited)",
    "schema": "logos.qor.comparative/1",
    "evidence_tiers": TIERS,
    "honesty": {
        "function": "SIM: an Icarus Verilog simulation of each candidate inside the shared harness; PASS only when the "
                    "MISR signature of its first 1000 results equals the golden on both stimulus modes.",
        "artix7": "PNR-open: yosys synth_xilinx and nextpnr-xilinx on the Nexys A7 part (xc7a100tcsg324-1), each "
                  "candidate out of context in a wrapper that only registers its inputs and folds its outputs into a "
                  "register; cells are yosys's for the candidate's own module, the frequency nextpnr's post-route "
                  "estimate, not vendor signoff. The fabric map times every cell; the native map's DSP, block-RAM, LUT-RAM and SRL cells are "
                  "not timed by nextpnr-xilinx, so its frequency is reported only where the design has none. A run "
                  "nextpnr's default router (router2) cannot finish routing within its deadline is run once more with the "
                  "older router (router1), the row naming its router and keeping router2's verdict.",
        "dram_derived": "A K7 controller's megabytes a second per thousand LUTs is its traffic-gate throughput over "
                        "the LUTs of its Artix-7 fabric map: what its area buys at DDR2-400, whatever clock it closes at.",
        "derived": "Throughput is the median routed fabric frequency divided by the candidate's steady cycles per "
                   "result: an open-flow estimate of results per second, never a silicon measurement.",
        "k7": "K7 has no harness contract. Its functional gate is the traffic gate (SIM): every controller carries "
              "the same stream of 64-bit words, written then read back, to a behavioural model of the Nexys A7's "
              "DDR2 (MT47H64M16HR-25) at DDR2-400 through an ideal PHY; its bus is held to the JEDEC DDR2 rules and "
              "every word read back is checked. Spans and latencies are DRAM clocks; throughput counts the words "
              "written and read over the stream's span. On hardware, a controller's gate is a board memtest.",
        "sky130": "GDS: OpenLane v1 (yosys, OpenROAD, Magic, Netgen, KLayout) on sky130A with the repo's overlay (its "
                  "SHA3-256 in flows.sky130, one per kernel), each candidate out of context in its fold wrapper at a "
                  "clock the bench sets; a control runs a candidate on OpenLane as released at the same commit, the "
                  "overlay's contribution the difference. Attempts and controls alike run OpenLane's image at that "
                  "commit with its OpenROAD (41a51eaf) rebuilt with one fix (scripts/openroad, the image in "
                  "flows.sky130): repair_timing walks a net's global route into a buffered tree, and on a route that "
                  "closes a cycle the released walk never ends and overflows the stack; the fixed walk takes a route "
                  "without a cycle exactly as before and one with a cycle once over a spanning tree. A candidate "
                  "closes at a period when its run is signoff-clean (every corner meets setup and hold; DRC, LVS, "
                  "antenna and routing clean). Its own minimum is the shortest period it closed at; a kernel's iso "
                  "period is the longest own minimum among "
                  "its candidates. Areas are the candidate's own module as synthesis reports it and the whole wrapped "
                  "design's post-route standard-cell area with physical-only cells left out; the die and core areas "
                  "are set by the floorplan and are not compared. Every run starts in the first of flows."
                  "sky130_floorplans; one the global router refuses for congestion, or one that meets timing and fails "
                  "signoff only on antenna pins no repair could fit, is run again in the next, its core utilization and "
                  "placement density lowered together, and its row names the floorplan it ran in and each one left "
                  "before it, with why. Every candidate runs the one comparative flow whose full config is flows."
                  "sky130_settings: every candidate is synthesized for delay, a run that misses setup at its typical "
                  "corner completes and records the slack it "
                  "missed by, synthesis check findings are recorded rather than stopping the flow, hold is repaired "
                  "with a margin that covers every corner, and a pin still violating antenna where the jog router "
                  "stops gets one more diode.",
        "power": "GDS-VCD: each candidate inside its functional harness, hardened whole on sky130A at its kernel's iso "
                 "period (or, after a setup miss there, at the period its run needs), simulated at gate level over a "
                 "window of whole operations once its pipeline is full; OpenSTA reads that window's activity on the "
                 "routed netlist with its extracted parasitics, at every corner, with the upstream fix that lets it use "
                 "every annotated activity. Energy per operation is power times the window's length over its "
                 "operations. The candidate's is its own cells', the buffers the flow inserted inside it, and the "
                 "clock tree's in the share of clock sinks that are its own; the whole block's is beside it, never "
                 "subtracted.",
    },
}
SKY130 = os.path.join(qorlib.SIL, "qor_sky130.json")
SKY130_CONTROL = os.path.join(qorlib.SIL, "qor_sky130_control.json")
SKY130_POWER = os.path.join(qorlib.SIL, "qor_sky130_power.json")
# The ledger each role's runs are recorded in (scripts/qor/run-qor-sky130.py ROLES).
SKY130_OF = {"attempt": SKY130, "control": SKY130_CONTROL, "power": SKY130_POWER}


def artix7_flow(runs):
    """The Artix-7 flow the runs went through, as exactly as they record it: the parts, the yosys and nextpnr version
    strings, the nextpnr patch sets and binaries (a binary rebuilt in place keeps its version string), how many runs
    predate the record of the binary, and how many predate the record of the RTL they measured (a run records it file by
    file, `rtl_files`, or, a fold design's, as one SHA3-256)."""
    return {"part": sorted({r["part"] for r in runs}), "yosys": sorted({r["yosys"] for r in runs}),
            "nextpnr": sorted({r["nextpnr"] for r in runs}),
            "nextpnr_patches_sha3": sorted({r["nextpnr_patches_sha3"] for r in runs}),
            "nextpnr_sha3": sorted({r["nextpnr_sha3"] for r in runs if r.get("nextpnr_sha3")}),
            "nextpnr_sha3_unrecorded": sum(1 for r in runs if not r.get("nextpnr_sha3")),
            "rtl_unrecorded": sum(1 for r in runs if "rtl_files" not in r and "rtl_sha3_256" not in r)}


def dram_summary(dram_rows):
    """A K7 candidate's traffic-gate runs (its rows of qor_dram.json) by stream, the upstream-module disclosure runs
    beside under `<stream>+upstream`: the operating point (the part, its clock, burst and CAS latency, the probe's read
    spacing), and the refusal, or what the run measured and what follows from it — the stream's span in microseconds,
    the payload's share of it, megabytes moved a second, the share of column commands that found their row open."""
    if not dram_rows:
        return None
    out = {}
    for r in sorted(dram_rows, key=lambda r: r["key"]):
        key = r["pattern"] + ("+upstream" if r["module"] == "upstream" else "")
        base = {"pass": r["pass"], "words": r["words"], "part": r["part"], "tck_ps": r["tck_ps"], "bl": r["bl"],
                "cl": r["cl"], "every": r["every"], "evidence_tier": "SIM", "reproduce": r["reproduce"]}
        if not r["pass"]:
            out[key] = {**base, "refusal": r["refusal"]}
            continue
        ns = r["stream_cycles"] * r["tck_ps"] / 1000.0
        cas = r["commands"]["read"] + r["commands"]["write"]
        out[key] = {**base, "stream_cycles": r["stream_cycles"], "stream_us": round(ns / 1000.0, 3),
                    "payload_share": round(r["payload_cycles"] / r["stream_cycles"], 4),
                    "mbytes_per_s": round(2 * r["words"] * 8 / ns * 1000.0, 2),
                    "row_hit_share": round(r["row_hits"] / cas, 4) if cas else None,
                    "commands": r["commands"], "write_latency": r["write_latency"],
                    "read_latency": r["read_latency"], "read_service": r["read_service"], "read_bus": r["read_bus"]}
    return out


def tool_of(cand, kernel, source_kind, entry):
    """The tool behind a candidate: the manifest's for a built third-party candidate, the K7 design table's for
    a controller, LOGOS for everything else."""
    if entry is not None:
        return entry["tool"]
    if kernel == "K7":
        return qorlib.fold_designs()[cand]["source"]
    if source_kind == "third-party":
        sys.exit("%s: a third-party candidate missing from the manifest" % cand)
    return "logos"


def function_summary(rows):
    by_mode = {r["mode"]: r for r in rows}
    if not by_mode:
        return None
    modes = {m: {"pass": r["pass"], "verdict": r["verdict"], "signature": r["signature"],
                 "golden_signature": r["golden_signature"], "ops": r["ops"], "cycles": r["cycles"],
                 "first_result_cycle": r["first_result_cycle"], "steady_cycles_per_op": r["steady_cycles_per_op"]}
             for m, r in sorted(by_mode.items())}
    passed = set(by_mode) == {"shaped", "edge"} and all(r["pass"] for r in by_mode.values())
    steady = by_mode["shaped"]["steady_cycles_per_op"] if "shaped" in by_mode else None
    return {"tier": "SIM", "pass": passed, "modes": modes, "steady_cycles_per_op": steady if passed else None,
            "simulator": next(iter(by_mode.values()))["simulator"]}


def gated_record(fn, cand):
    """The candidate record the functional gate committed with its shaped run (its protocol among it)."""
    row = next((r for r in fn if r["candidate"] == cand and r["mode"] == "shaped"), None)
    if row is None:
        return None
    texts, bad = qorlib.read_excerpts(row)
    if bad:
        sys.exit("; ".join(bad))
    return json.loads(texts["candidate.json"]) if "candidate.json" in texts else None


def artix7_summary(rows):
    out = {}
    for mapping in ("fabric", "native"):
        rs = sorted((r for r in rows if r["mapping"] == mapping), key=lambda r: r["seed"])
        if not rs:
            continue
        routed = [r for r in rs if r["verdict"] == "complete" and r.get("fmax_mhz") is not None]
        fmax = [r["fmax_mhz"] for r in routed]
        cells = [r["candidate_cells"] for r in rs if r.get("candidate_cells")]
        timing_complete = all(r.get("timing_complete") for r in rs if "timing_complete" in r)
        out[mapping] = {
            "tier": "PNR-open",
            "seeds": [r["seed"] for r in rs],
            "routed": len(routed),
            "refused": [{"seed": r["seed"], "verdict": r["verdict"]} for r in rs if r["verdict"] != "complete"],
            "fmax_mhz": {"median": statistics.median(fmax), "min": min(fmax), "max": max(fmax)} if fmax else None,
            "timing_complete": timing_complete,
            "frequency_reported": bool(fmax) and timing_complete,
            "cells": cells[0] if cells and all(c == cells[0] for c in cells) else None,
            "cells_vary": bool(cells) and any(c != cells[0] for c in cells),
            "critical_path_in_candidate": sum(1 for r in routed if (r.get("critical_path") or {}).get("in_candidate")),
            "critical_path_kinds": sorted({(r.get("critical_path") or {}).get("kind") or "unknown" for r in routed}),
            "untimed_cells": rs[0].get("untimed_cells"),
            "target_mhz": rs[0]["target_mhz"],
            "flow": rs[0]["flow"],
        }
    return out


def sky130_attempts(cand, role="attempt"):
    """The candidate's Sky130 runs of `role` (its attempts, its controls on stock OpenLane, or its power blocks),
    shortest period first: what each staged (qor_sky130_stage.json) beside what its run recorded (SKY130_OF) — or why
    it recorded nothing, whatever an older run of its block left in the ledger — the flow it ran, the floorplan it ran
    in, with each floorplan left before it and why, and what the run cost: its wall time and the most memory its
    container held. Only runs through the flow a run of `role` goes through now
    (qorlib.flow_identity): an attempt run under other flow knowledge is superseded and compares with nothing."""
    stage = qorlib.load("sky130_stage", {"rows": []})["rows"]
    ledger = SKY130_OF[role]
    blocks = (json.load(open(ledger)).get("blocks") or {}) if os.path.exists(ledger) else {}
    flow = qorlib.flow_identity(role == "control")
    out = []
    # A row only staged (run-qor-sky130.py stage) has run no flow, and is no attempt yet.
    mine = (s for s in stage if s["candidate"] == cand and s["role"] == role and s.get("flow") == flow
            and "flow_rc" in s)
    for s in sorted(mine, key=lambda s: s["period_ns"]):
        rec = None if s.get("refused") else blocks.get(s["block"])
        a = {"block": s["block"], "period_ns": s["period_ns"], "flow": s.get("flow"), "floorplan": s.get("floorplan"),
             "loosened": s.get("loosened"), "flow_wall_s": s.get("flow_wall_s"),
             "flow_peak_mem_gb": s.get("flow_peak_mem_gb"), "refused": s.get("refused") if rec is None else None}
        if rec is not None:
            a.update({k: rec[k] for k in ("signoff_clean", "setup_worst_slack_ns", "hold_worst_slack_ns", "std_cells",
                                          "std_cell_area_um2", "synth_area_um2", "die_area_mm2", "synth_findings")})
            a["candidate_area_um2"] = (rec.get("synth_module_areas_um2") or {}).get(s["candidate_top"])
        out.append(a)
    return out


def sky130_summary(attempts, controls, iso_period, steady):
    """Own minimum and iso period, each with its attempt and the results per second per mm² of standard cells it
    implies (`steady` the candidate's cycles per result); and every control on stock OpenLane beside the attempt at
    its period."""
    if not attempts and not controls:
        return None
    closed = [a for a in attempts if a.get("signoff_clean")]
    own = min(closed, key=lambda a: a["period_ns"]) if closed else None
    iso = next((a for a in attempts if a["period_ns"] == iso_period), None) if iso_period else None

    def at(a):
        if a is None:
            return None
        per_mm2 = None
        if a.get("signoff_clean") and steady and a.get("std_cell_area_um2", -1) > 0:
            per_mm2 = round(1000.0 / a["period_ns"] / steady / (a["std_cell_area_um2"] / 1e6), 4)
        return dict(a, mresults_per_s_per_mm2=per_mm2)
    beside = [{"control": at(c), "overlay": at(next((a for a in attempts if a["period_ns"] == c["period_ns"]), None))}
              for c in controls]
    return {"attempts": attempts, "own_minimum_ns": own["period_ns"] if own else None, "iso_period_ns": iso_period,
            "at_own_minimum": at(own), "at_iso": at(iso), "controls": beside}


def power_summary(cand, power_rows, blocks):
    """The candidate's energy per operation (`power_rows` its rows of qor_power.json, `blocks` its power blocks as
    sky130_attempts gives them): the measured block at the shortest period, each corner's energy for the candidate
    and for the whole block, and what it was read over; or None, with every power block it staged beside."""
    if not power_rows and not blocks:
        return None
    r = min(power_rows, key=lambda r: r["period_ns"]) if power_rows else None
    out = {"blocks": blocks, "measured": None}
    if r is not None:
        out["measured"] = {
            "block": r["block"], "period_ns": r["period_ns"], "evidence_tier": "GDS-VCD",
            "corners": {lib: {"dut_pj": c["energy"]["dut_pj"], "block_pj": c["energy"]["block_pj"]}
                        for lib, c in sorted(r["corners"].items())},
            "clock_share": r["corners"]["tt_025C_1v80"]["energy"]["clock_share"],
            "window": {k: r["window"][k] for k in ("ops", "cycles", "warmup_ops", "span_ns")},
            "reproduce": r["reproduce"],
        }
    return out


def assemble():
    fn = qorlib.load("function", {"rows": []})["rows"]
    ax = qorlib.load("artix7", {"rows": []})["rows"]
    manifest = json.load(open(MANIFEST))
    entries = {e["candidate"]["name"]: e for e in manifest["candidates"]}
    dram = qorlib.load("dram", {"rows": []})["rows"]
    designs = qorlib.fold_designs()
    names = {}
    for r in fn + ax:
        names.setdefault((r["kernel"], r["candidate"]), r["source_kind"])
    for d in dram:
        names.setdefault(("K7", d["candidate"]),
                         "LOGOS RTL generator" if designs[d["candidate"]]["source"] == "logos" else "third-party")
    rows = []
    for (kernel, cand), source_kind in sorted(names.items()):
        entry = entries.get(cand)
        f = function_summary([r for r in fn if r["candidate"] == cand])
        a = artix7_summary([r for r in ax if r["candidate"] == cand])
        k7 = next((r for r in ax if r["candidate"] == cand and r["kernel"] == "K7"), None)
        tool = tool_of(cand, kernel, source_kind, entry)
        design = designs.get(cand) if kernel == "K7" else None
        record = gated_record(fn, cand)
        row = {
            "key": "%s/%s" % (kernel, cand), "kernel": kernel, "candidate": cand, "source_kind": source_kind,
            "tool": tool, "function": f, "artix7": a, "refusal": None,
            "protocol": record.get("protocol") if record else None,
            "interface_note": entry["interface_note"] if entry else (design["note"] if design else None),
            "source": entry["source"] if entry else (None if design else "the LOGOS %s of %s (codegen_sva::qor_harness)" % (
                "program" if source_kind.startswith("compiled") else "RTL generator", kernel)),
            "generate": entry["generate"] if entry else (qorlib.fold_generate(cand) if design else
                        "cargo run -p logicaffeine-compile --example emit_qor_harness -- <dir> %s %s" % (kernel, cand)),
            "functional_gate": "SIM: the K7 traffic gate" if any(d["candidate"] == cand for d in dram)
                               else "none: no traffic testbench drives it" if kernel == "K7"
                               else ("simulation" if f else None),
        }
        fab = a.get("fabric")
        if f and f["pass"] and fab and fab["fmax_mhz"] and f["steady_cycles_per_op"]:
            mops = fab["fmax_mhz"]["median"] / f["steady_cycles_per_op"]
            lut = (fab["cells"] or {}).get("lut")
            row["derived"] = {"fabric_mresults_per_s": round(mops, 4),
                              "fabric_mresults_per_s_per_klut": round(mops / (lut / 1000.0), 4) if lut else None}
        else:
            row["derived"] = None
        rows.append(row)
    for r in manifest.get("refusals", []):
        rows.append({"key": "%s/%s" % (r["kernel"], r["name"]), "kernel": r["kernel"], "candidate": r["name"],
                     "source_kind": "third-party", "tool": r["tool"], "function": None, "artix7": {}, "derived": None,
                     "refusal": {"stage": "build", "reason": r["reason"]}, "interface_note": None, "source": None,
                     "generate": r["generate"], "functional_gate": None})
    # A kernel's iso period is the longest own minimum among its candidates; each row then reads its attempt there.
    attempts = {r["candidate"]: sky130_attempts(r["candidate"]) for r in rows}
    own = {c: min((a["period_ns"] for a in at if a.get("signoff_clean")), default=None) for c, at in attempts.items()}
    iso = {}
    for r in rows:
        if own[r["candidate"]] is not None:
            iso[r["kernel"]] = max(iso.get(r["kernel"], 0), own[r["candidate"]])
    power = qorlib.load("power", {"rows": []})["rows"]
    for r in rows:
        steady = (r["function"] or {}).get("steady_cycles_per_op") if (r["function"] or {}).get("pass") else None
        r["sky130"] = sky130_summary(attempts[r["candidate"]], sky130_attempts(r["candidate"], "control"),
                                     iso.get(r["kernel"]), steady)
        r["power"] = power_summary(r["candidate"], [p for p in power if p["candidate"] == r["candidate"]],
                                   sky130_attempts(r["candidate"], "power"))
        r["dram"] = dram_summary([d for d in dram if d["candidate"] == r["candidate"]])
        lut = (((r.get("artix7") or {}).get("fabric") or {}).get("cells") or {}).get("lut")
        for s in (r["dram"] or {}).values():
            if s.get("mbytes_per_s") is not None and lut:
                s["mbytes_per_s_per_klut"] = round(s["mbytes_per_s"] / (lut / 1000.0), 2)
    doc = dict(HEADER)
    doc["kernels"] = {k: {"name": n, "logos_source": s} for k, (n, s) in KERNELS.items()}
    doc["flows"] = {
        "function": sorted({r["simulator"] for r in fn}),
        "artix7": artix7_flow(ax),
        "sky130": {k: sorted({json.dumps(a["flow"], sort_keys=True) for r in rows if r["kernel"] == k
                              for a in attempts[r["candidate"]] if a.get("flow")})
                   for k in sorted({r["kernel"] for r in rows})},
        "sky130_floorplans": list(qorlib.SKY130_FLOORPLANS),
        "sky130_settings": qorlib.flow_settings_config(),
        "power": {"opensta": sorted({p["opensta"] for p in power})},
        "dram": {"simulator": sorted({d["simulator"] for d in dram}), "part": sorted({d["part"] for d in dram}),
                 "tck_ps": sorted({d["tck_ps"] for d in dram})},
    }
    doc["rows"] = sorted(rows, key=lambda r: r["key"])
    return doc


def main():
    if sys.argv[1:] == ["verify"]:
        want = assemble()
        have = json.load(open(OUT))
        if have != json.loads(json.dumps(want)):
            bad = [r["key"] for r, s in zip(want["rows"], have.get("rows", [])) if json.loads(json.dumps(r)) != s]
            sys.exit("comparative_synthesis.json is stale or edited: %s" % (bad[:10] or "header"))
        print("verified %d rows" % len(have["rows"]))
        return
    doc = assemble()
    with open(OUT, "w") as f:
        json.dump(doc, f, indent=1, sort_keys=True)
        f.write("\n")
    print("%d rows -> %s" % (len(doc["rows"]), qorlib.rel(OUT)))


if __name__ == "__main__":
    main()
